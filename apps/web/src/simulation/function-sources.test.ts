import { describe, expect, it, vi } from "vitest";
import { MockWorkspaceClient } from "../workspace/mock-workspace-client";
import {
    openDocumentFromWorkspaceFile,
    type OpenDocument,
} from "../documents/document-session";
import type { DesignerSourceWorkspace } from "../documents/designer-source-workspace";
import { example, PENDULUM_SOURCE } from "./examples";
import {
    DEFAULT_EXECUTION,
    parseDocument,
    serializeDocument,
    validateFunctionKind,
} from "./model";
import {
    readFunctionSources,
    saveFunctionSources,
    sourceBundle,
    sourcePath,
} from "./function-sources";
import { numericalSource } from "./use-simulation-run";

async function setup() {
    const workspace = new MockWorkspaceClient();
    await workspace.connect();
    const root = await workspace.currentDirectory();
    const documents = new Map<string, OpenDocument>();
    const shared: DesignerSourceWorkspace = {
        documents: [],
        editorSession: {} as DesignerSourceWorkspace["editorSession"],
        lspUrl: null,
        getSource: (path) => documents.get(path),
        ensureSource: (source) => {
            const doc =
                documents.get(source.path) ??
                openDocumentFromWorkspaceFile(source.file!);
            documents.set(source.path, doc);
            return doc;
        },
        updateSource: (id, content) => {
            const doc = [...documents.values()].find((d) => d.id === id)!;
            documents.set(doc.path, {
                ...doc,
                content,
                version: doc.version + 1,
            });
        },
        acceptSaved: (file) => {
            const doc = documents.get(file.path);
            documents.set(
                file.path,
                doc
                    ? {
                          ...doc,
                          savedContent: file.content,
                          revision: file.revision,
                      }
                    : openDocumentFromWorkspaceFile(file),
            );
        },
        openDocument: () => false,
    };
    await workspace.create("pendulum.m", "file");
    const file = await workspace.read("pendulum.m");
    await workspace.write(
        file.path,
        PENDULUM_SOURCE,
        file.revision,
        root.generation,
    );
    return { workspace, shared, root };
}

describe("m function source snapshots", () => {
    it("uses unsaved shared drafts and freezes source independently of subsequent edits", async () => {
        const { workspace, shared } = await setup();
        const model = example("pendulum").model;
        await readFunctionSources(model, "example.omsim", workspace, shared);
        const doc = shared.getSource("pendulum.m")!;
        shared.updateSource(doc.id, PENDULUM_SOURCE.replace("0", "1"));
        const snapshot = await readFunctionSources(
            model,
            "example.omsim",
            workspace,
            shared,
        );
        shared.updateSource(doc.id, "changed again");
        expect(snapshot[0]!.content).not.toBe("changed again");
        expect((await workspace.read("pendulum.m")).content).toBe(
            PENDULUM_SOURCE,
        );
        expect(
            numericalSource(model, {
                sources: sourceBundle(snapshot),
                execution: DEFAULT_EXECUTION,
            }),
        ).not.toBe(
            numericalSource(model, {
                sources: { "pendulum.m": "changed again" },
                execution: DEFAULT_EXECUTION,
            }),
        );
    });
    it("preserves edits made while a source write is in flight and rejects revision conflicts", async () => {
        const { workspace, shared, root } = await setup();
        const files = await readFunctionSources(
            example("pendulum").model,
            "test.omsim",
            workspace,
            shared,
        );
        const write = workspace.write.bind(workspace);
        vi.spyOn(workspace, "write").mockImplementationOnce(async (...args) => {
            shared.updateSource(
                shared.getSource("pendulum.m")!.id,
                "newer draft",
            );
            return write(...args);
        });
        await saveFunctionSources(
            files,
            "test.omsim",
            workspace,
            root.generation,
            shared,
        );
        expect(shared.getSource("pendulum.m")!.content).toBe("newer draft");
        expect(shared.getSource("pendulum.m")!.savedContent).toBe(
            PENDULUM_SOURCE,
        );
        await expect(
            saveFunctionSources(
                files,
                "test.omsim",
                workspace,
                root.generation,
                shared,
            ),
        ).rejects.toThrow();
    });
    it("retries an interrupted first source save without losing the draft", async () => {
        const { workspace, shared, root } = await setup();
        const file = {
            ...(await workspace.read("pendulum.m")),
            path: "new.m",
            revision: "",
        };
        shared.ensureSource({
            path: file.path,
            content: file.content,
            savedContent: "",
            file,
        });
        const snapshot = {
            reference: file.path,
            path: file.path,
            content: file.content,
            revision: "",
        };
        vi.spyOn(workspace, "write").mockRejectedValueOnce(
            new Error("interrupted"),
        );
        await expect(
            saveFunctionSources(
                [snapshot],
                "new.omsim",
                workspace,
                root.generation,
                shared,
            ),
        ).rejects.toThrow("interrupted");
        expect(shared.getSource(file.path)!.content).toBe(file.content);
        expect(shared.getSource(file.path)!.savedContent).toBe("");
        expect(shared.getSource(file.path)!.revision).not.toBe("");
        await saveFunctionSources(
            [{ ...snapshot, revision: shared.getSource(file.path)!.revision }],
            "new.omsim",
            workspace,
            root.generation,
            shared,
        );
        expect((await workspace.read(file.path)).content).toBe(file.content);
    });
    it("copies relative sources for Save As and never overwrites a conflicting target", async () => {
        const { workspace, shared, root } = await setup();
        const files = await readFunctionSources(
            example("pendulum").model,
            "test.omsim",
            workspace,
            shared,
        );
        await workspace.create("models", "directory");
        await saveFunctionSources(
            files,
            "models/copy.omsim",
            workspace,
            root.generation,
            shared,
        );
        expect((await workspace.read("models/pendulum.m")).content).toBe(
            PENDULUM_SOURCE,
        );
        await expect(
            saveFunctionSources(
                [{ ...files[0]!, content: "unrelated" }],
                "models/copy2.omsim",
                workspace,
                root.generation,
                shared,
            ),
        ).rejects.toThrow();
        expect((await workspace.read("models/pendulum.m")).content).toBe(
            PENDULUM_SOURCE,
        );
        expect(sourcePath("models/copy.omsim", "pendulum.m")).toBe(
            "models/pendulum.m",
        );
        expect(() => sourcePath(null, "../private.m")).toThrow();
    });
    it("round trips v2 source references and solver options while rejecting invalid signatures", () => {
        const doc = example("pendulum");
        doc.execution = {
            backend: "llvm",
            solver: {
                type: "cvode",
                method: "bdf",
                relativeTolerance: 1e-6,
                absoluteTolerance: 1e-9,
            },
        };
        expect(parseDocument(serializeDocument(doc))).toEqual(doc);
        const block = doc.model.blocks.find(
            (b) => b.kind.type === "mFunction",
        )!;
        expect(() =>
            validateFunctionKind({
                ...block.kind,
                inputs: [
                    { name: "x", width: 2 },
                    { name: "x", width: 1 },
                ],
            }),
        ).toThrow();
        expect(() =>
            parseDocument(serializeDocument({ ...doc, schemaVersion: 1 })),
        ).toThrow();
    });
});
