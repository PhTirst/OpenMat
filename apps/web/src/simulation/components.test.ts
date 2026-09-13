import { describe, expect, it } from "vitest";
import { MockWorkspaceClient } from "../workspace/mock-workspace-client";
import {
    attachComponent,
    callbacks,
    componentFile,
    modelSources,
    parseComponentFile,
    sameComponentDefinition,
    validateComponent,
    validateComponentKind,
} from "./components";
import {
    COMPONENT_TEMPLATES,
    copyComponentSources,
    discoverComponents,
    librarySources,
} from "./component-library";
import { example, initializeComponentExample } from "./examples";
import {
    copySelection,
    emptyDocument,
    parseDocument,
    pasteFragment,
    ports,
    serializeDocument,
} from "./model";
import { readFunctionSources, saveFunctionSources } from "./function-sources";

describe("stateful component documents and libraries", () => {
    it("distinguishes interface conflicts from model-local source renaming and JSON property order", () => {
        const entry = COMPONENT_TEMPLATES[0]!;
        const copied = copyComponentSources(
            entry.definition,
            entry.sources!,
            "local",
        );
        expect(
            sameComponentDefinition(entry.definition, copied.definition),
        ).toBe(false);
        expect(
            sameComponentDefinition(entry.definition, copied.definition, false),
        ).toBe(true);
        copied.definition.parameters[0]!.value = [7];
        expect(
            sameComponentDefinition(entry.definition, copied.definition, false),
        ).toBe(false);
        const reversed = Object.fromEntries(
            Object.entries(entry.definition).reverse(),
        ) as unknown as typeof entry.definition;
        const doc = emptyDocument();
        attachComponent(doc, entry.definition);
        attachComponent(doc, reversed);
        expect(doc.model.components).toHaveLength(1);
    });
    it("round-trips multi-output metadata and parameter overrides, with version isolation", () => {
        const doc = example("piControl");
        expect(parseDocument(serializeDocument(doc))).toEqual(doc);
        const plant = doc.model.blocks.find((b) => b.id === "plant")!;
        expect(ports(plant, doc.model.components).outputs).toEqual([
            "position",
            "velocity",
        ]);
        for (const version of [1, 2]) {
            const broken = structuredClone(doc);
            broken.schemaVersion = version as 1 | 2;
            expect(() => parseDocument(serializeDocument(broken))).toThrow();
            broken.model.schemaVersion = version as 1 | 2;
            expect(() => parseDocument(serializeDocument(broken))).toThrow();
        }
    });
    it("copies definitions with instances and rejects conflicting shared IDs", () => {
        const doc = example("piControl");
        const fragment = copySelection(doc, new Set(["controller"]));
        const pasted = pasteFragment(
            emptyDocument(),
            fragment,
            () => "copy",
        ).document;
        expect(pasted.model.components).toHaveLength(1);
        expect(parseDocument(serializeDocument(pasted))).toEqual(pasted);
        const conflicting = structuredClone(pasted.model.components![0]!);
        conflicting.name = "Different";
        expect(() => attachComponent(pasted, conflicting)).toThrow();
    });
    it("bounds shapes, callbacks, parameter metadata and overrides", () => {
        const definition = COMPONENT_TEMPLATES[0]!.definition;
        for (const change of [
            { continuousStates: 5000 },
            { name: "示".repeat(100) },
            { sampleTime: 0 },
            { outputs: [] },
            { update: undefined },
            { initialize: { source: "../secret.m", entry: "init" } },
            { parameters: [{ name: "a", value: [1], minimum: 2 }] },
        ])
            expect(() =>
                validateComponent({ ...definition, ...change }),
            ).toThrow();
        expect(() =>
            validateComponentKind(
                {
                    type: "component",
                    component: definition.id,
                    parameters: { unknown: [0] },
                },
                [definition],
            ),
        ).toThrow();
        expect(() =>
            validateComponentKind(
                {
                    type: "component",
                    component: definition.id,
                    parameters: { initial: [0, 1] },
                },
                [definition],
            ),
        ).toThrow();
        expect(parseComponentFile(componentFile(definition))).toEqual(
            definition,
        );
        expect(() =>
            parseComponentFile(
                componentFile(definition).replace(
                    '"schemaVersion": 1',
                    '"schemaVersion": 3',
                ),
            ),
        ).toThrow();
    });
    it("prepares standalone example sources with distinct filenames and matching entry declarations", () => {
        for (const name of [
            "customDelay",
            "massSpring",
            "piControl",
        ] as const) {
            const doc = example(name);
            const sources = initializeComponentExample(doc, "first");
            expect(Object.keys(sources).sort()).toEqual(
                modelSources(doc.model).sort(),
            );
            for (const d of doc.model.components!)
                for (const c of callbacks(d))
                    expect(sources[c.source]).toContain(`= ${c.entry}(`);
            const other = example(name);
            const otherSources = initializeComponentExample(other, "second");
            expect(
                Object.keys(otherSources).some((path) =>
                    Object.hasOwn(sources, path),
                ),
            ).toBe(false);
        }
    });
    it("renames a bracketed single output without rewriting a declaration inside a comment", () => {
        const entry = COMPONENT_TEMPLATES[0]!;
        const source = structuredClone(entry.sources!);
        const callback = entry.definition.outputsFunction;
        source[callback.source] =
            `% function y = ${callback.entry}(t,x,q,u,p)\nfunction [y] = ${callback.entry}(t,x,q,u,p)\ny=q;\nend`;
        const copied = copyComponentSources(
            entry.definition,
            source,
            "brackets",
        );
        expect(
            copied.sources[copied.definition.outputsFunction.source],
        ).toContain(`% function y = ${callback.entry}`);
        expect(
            copied.sources[copied.definition.outputsFunction.source],
        ).toContain(
            `function [y] = ${copied.definition.outputsFunction.entry}(`,
        );
    });
    it("discovers project definitions and preserves portable source files through save/reopen", async () => {
        const workspace = new MockWorkspaceClient();
        await workspace.connect();
        const root = await workspace.currentDirectory();
        const entry = COMPONENT_TEMPLATES[1]!;
        const copied = copyComponentSources(
            entry.definition,
            entry.sources!,
            "test",
        );
        const target = "plant.omblock.json";
        const snapshots = Object.entries(copied.sources).map(
            ([reference, content]) => ({
                reference,
                path: reference,
                content,
                revision: "",
            }),
        );
        await saveFunctionSources(
            snapshots,
            target,
            workspace,
            root.generation,
        );
        await workspace.create(target, "file");
        const baseline = await workspace.read(target);
        await workspace.write(
            target,
            componentFile(copied.definition),
            baseline.revision,
            root.generation,
        );
        const discovery = await discoverComponents(workspace);
        expect(discovery.issues).toEqual([]);
        expect(discovery.entries).toHaveLength(1);
        expect(await librarySources(discovery.entries[0]!, workspace)).toEqual(
            copied.sources,
        );
        const doc = emptyDocument();
        attachComponent(doc, copied.definition);
        doc.model.blocks.push({
            id: "plant",
            kind: {
                type: "component",
                component: copied.definition.id,
                parameters: {},
            },
        });
        const reloaded = parseDocument(serializeDocument(doc));
        const read = await readFunctionSources(
            reloaded.model,
            "test.omsim",
            workspace,
        );
        expect(
            Object.fromEntries(read.map((f) => [f.reference, f.content])),
        ).toEqual(copied.sources);
    });
});
