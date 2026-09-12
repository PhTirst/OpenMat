import { describe, expect, it, vi } from "vitest";
import { example, PENDULUM_SOURCE } from "./examples";
import {
    copySelection,
    emptyDocument,
    parseDocument,
    pasteFragment,
    serializeDocument,
} from "./model";
import {
    decodeSlx,
    encodeSlx,
    validateSlxAsset,
    type SlxAsset,
} from "./slx-authoring";
import { readFunctionSources } from "./function-sources";
import { MockWorkspaceClient } from "../workspace/mock-workspace-client";

const asset: SlxAsset = {
    name: "authored.slx",
    package: encodeSlx(new Uint8Array([80, 75, 1, 2])),
    parameters: "K=2;",
    appliedParameters: "K=2;",
    runnable: true,
    issues: [],
    document: {
        name: "authored",
        systems: [{ parentBlock: null, blocks: [], lines: [] }],
    },
};

describe("SLX authoring snapshots", () => {
    it("roundtrips hierarchy, original package and immutable sources without reading local files", async () => {
        const doc = example("pendulum");
        doc.schemaVersion = 4;
        doc.model.schemaVersion = 4;
        doc.slx = asset;
        doc.sources = { "pendulum.m": PENDULUM_SOURCE };
        const restored = parseDocument(serializeDocument(doc));
        expect(restored).toEqual(doc);
        const workspace = new MockWorkspaceClient();
        const read = vi.spyOn(workspace, "read");
        const files = await readFunctionSources(
            restored.model,
            "moved/model.omsim",
            workspace,
            undefined,
            restored.sources,
        );
        expect(files[0]?.content).toBe(PENDULUM_SOURCE);
        expect(read).not.toHaveBeenCalled();
        const copied = copySelection(restored, new Set(["rhs"]));
        expect(copied.slx).toBeUndefined();
        const pasted = pasteFragment(
            emptyDocument(),
            copied,
            () => "copy",
        ).document;
        expect(parseDocument(serializeDocument(pasted)).sources).toEqual(
            doc.sources,
        );
        copied.sources!["pendulum.m"] = "function y=pendulum(x,p)\ny=x;\nend";
        expect(() => pasteFragment(pasted, copied, () => "copy2")).toThrow(
            /源码冲突/,
        );
    });
    it("preserves Step schema when copying into an older model", () => {
        const doc = emptyDocument();
        doc.schemaVersion = 4;
        doc.model.schemaVersion = 4;
        doc.model.blocks = [
            {
                id: "s",
                kind: { type: "step", time: 0.2, before: [0], after: [1] },
            },
        ];
        const pasted = pasteFragment(
            emptyDocument(),
            doc,
            () => "new_step",
        ).document;
        expect(
            parseDocument(serializeDocument(pasted)).model.schemaVersion,
        ).toBe(4);
    });
    it("bounds source paths, hierarchy and base64 without a recursive regular expression", () => {
        const bytes = new Uint8Array(2 * 1024 * 1024).fill(197);
        const restored = decodeSlx(encodeSlx(bytes));
        expect(restored.length).toBe(bytes.length);
        expect(restored.every((v) => v === 197)).toBe(true);
        expect(() => decodeSlx("A".repeat(2796208))).toThrow();
        for (const text of ["", "====", "a%AA", "AAA"])
            expect(() => decodeSlx(text)).toThrow();
        const bad = structuredClone(asset);
        bad.document.systems[0]!.parentBlock = "missing";
        expect(() => validateSlxAsset(bad)).toThrow();
        const doc = example("pendulum");
        doc.schemaVersion = 4;
        doc.sources = { "../escape.m": "x=1;" };
        expect(() => parseDocument(serializeDocument(doc))).toThrow(/源码/);
    });
});
