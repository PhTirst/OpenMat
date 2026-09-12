import {
    fireEvent,
    render,
    screen,
    waitFor,
    within,
} from "@testing-library/react";
import { createRef, useState } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import ModelEditor from "./ModelEditor";
import type { ModelEditorSession } from "./session";
import { PendingOperations } from "../platform/pending-operations";
import { MockWorkspaceClient } from "../workspace/mock-workspace-client";
import type { DesignerSourceWorkspace } from "../documents/designer-source-workspace";
import {
    openDocumentFromWorkspaceFile,
    type OpenDocument,
} from "../documents/document-session";
import { modelSources, parseComponentFile } from "./components";
import { parseDocument } from "./model";

vi.mock("./SimulationCanvas", () => ({
    SimulationCanvas: () => <div>Graph surface</div>,
}));
vi.mock("./ScopePanel", () => ({ ScopePanel: () => <div>Scope surface</div> }));
vi.mock("./FunctionCodePanel", () => ({
    FunctionCodePanel: ({
        path,
        shared,
    }: {
        path: string;
        shared: DesignerSourceWorkspace;
    }) => {
        const doc = shared.getSource(path);
        return doc ? (
            <label>
                Test source
                <textarea
                    aria-label="Test source"
                    value={doc.content}
                    onChange={(e) =>
                        shared.updateSource(doc.id, e.target.value)
                    }
                />
            </label>
        ) : null;
    },
}));
beforeEach(() => localStorage.clear());
async function setup() {
    const workspace = new MockWorkspaceClient();
    await workspace.connect();
    const root = await workspace.currentDirectory();
    const documents = new Map<string, OpenDocument>();
    const session = createRef<ModelEditorSession>();
    function Harness() {
        const [, refresh] = useState(0);
        const shared: DesignerSourceWorkspace = {
            documents: [...documents.values()],
            editorSession: {} as DesignerSourceWorkspace["editorSession"],
            lspUrl: null,
            getSource: (path) => documents.get(path),
            ensureSource: (seed) => {
                const doc = documents.get(seed.path) ?? {
                    ...openDocumentFromWorkspaceFile(
                        seed.file ?? {
                            path: seed.path,
                            content: seed.content,
                            revision: "",
                            size: 0,
                            rootPath: root.path,
                            rootGeneration: root.generation,
                        },
                    ),
                    savedContent: seed.savedContent,
                };
                documents.set(seed.path, doc);
                return doc;
            },
            updateSource: (id, content) => {
                const doc = [...documents.values()].find((d) => d.id === id)!;
                documents.set(doc.path, {
                    ...doc,
                    content,
                    version: doc.version + 1,
                });
                refresh((v) => v + 1);
            },
            acceptSaved: (file) => {
                const doc = documents.get(file.path);
                if (doc)
                    documents.set(file.path, {
                        ...doc,
                        revision: file.revision,
                        savedContent: file.content,
                    });
            },
            openDocument: () => false,
        };
        return (
            <ModelEditor
                workspace={workspace}
                rootPath={root.path}
                rootGeneration={root.generation}
                visible
                theme="modern-light"
                onClose={vi.fn()}
                onSaved={vi.fn()}
                pendingSaves={new PendingOperations()}
                sessionRef={session}
                sourceWorkspace={shared}
            />
        );
    }
    render(<Harness />);
    await waitFor(() => expect(session.current).not.toBeNull());
    return { workspace, documents, session };
}
async function save(path: string) {
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    const dialog = await screen.findByRole("dialog", { name: "保存模型" });
    fireEvent.change(
        within(dialog).getByRole("textbox", { name: "模型文件路径" }),
        { target: { value: path } },
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() =>
        expect(
            screen.queryByRole("dialog", { name: "保存模型" }),
        ).not.toBeInTheDocument(),
    );
}

it("saves component sources and instance parameters, exports a reusable definition, and reopens the model", async () => {
    const { workspace, documents, session } = await setup();
    fireEvent.change(screen.getByRole("combobox", { name: "打开示例" }), {
        target: { value: "customDelay" },
    });
    fireEvent.click(
        screen.getByRole("treeitem", {
            name: "Custom Unit Delay",
        }),
    );
    const initial = screen.getByRole("textbox", {
        name: "组件参数 Initial output",
    });
    fireEvent.change(initial, { target: { value: "7" } });
    fireEvent.blur(initial);
    fireEvent.click(screen.getByRole("button", { name: /离散更新.*\.m/ }));
    const editor = await screen.findByRole("textbox", { name: "Test source" });
    fireEvent.change(editor, {
        target: {
            value: (editor as HTMLTextAreaElement).value.replace(
                "z = u;",
                "z = u + 2;",
            ),
        },
    });
    expect(session.current!.dirty).toBe(true);
    await save("custom.omsim");
    expect(session.current!.dirty).toBe(false);
    const written = parseDocument(
        (await workspace.read("custom.omsim")).content,
    );
    expect(
        written.model.blocks.find((b) => b.id === "delay")!.kind,
    ).toMatchObject({ parameters: { initial: [7] } });
    for (const reference of modelSources(written.model))
        expect((await workspace.read(reference)).content).toBe(
            documents.get(reference)!.content,
        );
    fireEvent.click(screen.getByRole("button", { name: "保存到项目组件库" }));
    await waitFor(async () =>
        expect(
            (await workspace.list("", true)).entries.filter((e) =>
                e.path.endsWith(".omblock.json"),
            ),
        ).toHaveLength(1),
    );
    await waitFor(() => expect(session.current!.busy).toBe(false));
    const entry = (await workspace.list("", true)).entries.find((e) =>
        e.path.endsWith(".omblock.json"),
    )!;
    expect(
        parseComponentFile((await workspace.read(entry.path)).content),
    ).toEqual(written.model.components![0]);
    fireEvent.click(screen.getByRole("button", { name: "打开" }));
    const open = await screen.findByRole("dialog", { name: "打开模型" });
    fireEvent.change(
        within(open).getByRole("textbox", { name: "模型文件路径" }),
        { target: { value: "custom.omsim" } },
    );
    fireEvent.submit(open);
    await waitFor(() =>
        expect(
            screen.queryByRole("dialog", { name: "打开模型" }),
        ).not.toBeInTheDocument(),
    );
    fireEvent.click(
        screen.getByRole("treeitem", {
            name: "Custom Unit Delay",
        }),
    );
    expect(
        screen.getByRole("textbox", { name: "组件参数 Initial output" }),
    ).toHaveValue("7");
});

it("creates a component with public parameters and lifecycle source skeletons", async () => {
    const { workspace } = await setup();
    fireEvent.click(screen.getByRole("button", { name: "新建自定义组件…" }));
    const dialog = screen.getByRole("dialog", { name: "组件定义" });
    fireEvent.change(
        within(dialog).getByRole("textbox", { name: "组件名称" }),
        { target: { value: "My custom filter" } },
    );
    fireEvent.change(
        within(dialog).getByRole("spinbutton", { name: "离散状态数" }),
        { target: { value: "1" } },
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "添加参数" }));
    fireEvent.change(
        within(dialog).getByRole("textbox", { name: "参数 1 名称" }),
        { target: { value: "gain" } },
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "应用定义" }));
    await screen.findByRole("treeitem", {
        name: "My custom filter",
    });
    expect(screen.getByRole("textbox", { name: "组件参数 gain" })).toHaveValue(
        "1",
    );
    await save("new.omsim");
    const model = parseDocument(
        (await workspace.read("new.omsim")).content,
    ).model;
    const d = model.components![0]!;
    expect(d.discreteStates).toBe(1);
    expect(d.initialize).toBeDefined();
    expect(d.update).toBeDefined();
    expect(d.derivatives).toBeUndefined();
    expect((await workspace.read(d.update!.source)).content).toContain(
        "z = q;",
    );
});

it("removes stale parameter overrides on definition edits and reports library conflicts before adding an instance", async () => {
    const { workspace } = await setup();
    fireEvent.change(screen.getByRole("combobox", { name: "打开示例" }), {
        target: { value: "customDelay" },
    });
    fireEvent.click(
        screen.getByRole("treeitem", { name: "Custom Unit Delay" }),
    );
    const initial = screen.getByRole("textbox", {
        name: "组件参数 Initial output",
    });
    fireEvent.change(initial, { target: { value: "7" } });
    fireEvent.blur(initial);
    fireEvent.click(screen.getByRole("button", { name: "编辑组件定义…" }));
    const dialog = screen.getByRole("dialog", { name: "组件定义" });
    fireEvent.change(
        within(dialog).getByRole("textbox", { name: "参数 1 名称" }),
        { target: { value: "startValue" } },
    );
    fireEvent.change(
        within(dialog).getByRole("textbox", { name: "参数 1 默认值" }),
        { target: { value: "3" } },
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "应用定义" }));
    expect(
        screen.getByRole("textbox", { name: "组件参数 Initial output" }),
    ).toHaveValue("3");
    fireEvent.click(
        screen.getAllByRole("button", { name: /Custom Unit Delay/ })[0]!,
    );
    await screen.findByText(/组件库与模型内相同 ID 的定义不同/);
    expect(
        screen.getAllByRole("treeitem", { name: "Custom Unit Delay" }),
    ).toHaveLength(1);
    fireEvent.click(
        screen.getAllByRole("button", { name: /Custom Unit Delay/ })[1]!,
    );
    await waitFor(() =>
        expect(
            screen.getAllByRole("treeitem", { name: "Custom Unit Delay" }),
        ).toHaveLength(2),
    );
    await save("redefined.omsim");
    const model = parseDocument(
        (await workspace.read("redefined.omsim")).content,
    ).model;
    expect(model.blocks.find((b) => b.id === "delay")!.kind).toMatchObject({
        parameters: {},
    });
    expect(model.components![0]!.parameters[0]).toMatchObject({
        name: "startValue",
        value: [3],
    });
});
