import {
    act,
    fireEvent,
    render,
    screen,
    waitFor,
} from "@testing-library/react";
import { createRef } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import ModelEditor from "./ModelEditor";
import { example } from "./examples";
import {
    fromModel,
    parseDocument,
    serializeDocument,
    type Model,
} from "./model";
import type { AuthoringParameters } from "./block-parameters";
import { applyLiteralParameters } from "./literal-parameters";
import { MockWorkspaceClient } from "../workspace/mock-workspace-client";
import { PendingOperations } from "../platform/pending-operations";
import type { ModelEditorSession } from "./session";

const native = vi.hoisted(() => ({
    run: vi.fn(),
    check: vi.fn(),
    resolve: vi.fn(),
}));
vi.mock("./use-simulation-run", async (original) => {
    const state = {
        client: { current: { resolveParameters: native.resolve } },
        connected: true,
        connectionError: null,
        reconnect: vi.fn(),
        status: "idle",
        info: null,
        diagnostics: [],
        setDiagnostics: vi.fn(),
        frames: { current: [] },
        version: 0,
        elapsed: 0,
        capabilities: undefined,
        solverStats: null,
        source: { current: "" },
        run: native.run,
        check: native.check,
        cancel: vi.fn(),
        reset: vi.fn(),
        busy: false,
    };
    return {
        ...(await original<typeof import("./use-simulation-run")>()),
        useSimulationRun: () => state,
    };
});
vi.mock("./SimulationCanvas", () => ({
    SimulationCanvas: () => <div>Numerical canvas</div>,
}));
vi.mock("./ScopePanel", () => ({ ScopePanel: () => <div>Scope</div> }));
beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    native.resolve.mockImplementation(
        async (model: Model, parameters: AuthoringParameters) => {
            let doc = fromModel(model);
            for (const [id, fields] of Object.entries(parameters.bindings))
                doc = applyLiteralParameters(doc, id, fields);
            return { model: doc.model, values: {} };
        },
    );
});

it("groups feedback, edits inside, undoes, saves, reopens and runs the nested document", async () => {
    const workspace = new MockWorkspaceClient();
    await workspace.connect();
    const root = await workspace.currentDirectory();
    await workspace.create("native.omsim", "file");
    const file = await workspace.read("native.omsim"),
        content = serializeDocument(example("feedback"));
    const write = await workspace.write(
        file.path,
        content,
        file.revision,
        root.generation,
    );
    localStorage.setItem(
        `openmat.simulation.draft.v1:${root.path}`,
        JSON.stringify({
            content,
            saved: content,
            file: { path: file.path, revision: write.revision },
        }),
    );
    const session = createRef<ModelEditorSession>();
    const mount = () =>
        render(
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
            />,
        );
    let view = mount();
    fireEvent.click(await screen.findByRole("treeitem", { name: /State/ }));
    fireEvent.click(screen.getByRole("button", { name: "创建子系统" }));
    expect(
        screen.queryByRole("treeitem", { name: /State/ }),
    ).not.toBeInTheDocument();
    const container = screen.getByRole("treeitem", { name: /Subsystem/ });
    fireEvent.doubleClick(container);
    expect(
        screen.getByRole("navigation", { name: "当前子系统" }),
    ).toHaveTextContent("Subsystem");
    fireEvent.click(screen.getByRole("treeitem", { name: /State/ }));
    const parameter = screen.getByRole("textbox", { name: "初始条件" });
    fireEvent.change(parameter, { target: { value: "0.5" } });
    // A separate name edit must not clear the unapplied parameter guard.
    fireEvent.change(screen.getByLabelText("方块名称"), {
        target: { value: "Renamed state" },
    });
    await act(async () => {
        expect(await session.current!.save()).toBe(false);
        fireEvent.click(screen.getByRole("button", { name: /▶ 运行/ }));
    });
    expect(native.run).not.toHaveBeenCalled();
    await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "应用参数" }));
    });
    fireEvent.click(screen.getByRole("button", { name: "返回上层" }));
    // Undo is available while browsing any level; it restores the state parameter.
    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    fireEvent.keyDown(window, { key: "y", ctrlKey: true });
    await act(async () => {
        expect(await session.current!.save()).toBe(true);
    });
    const saved = parseDocument((await workspace.read(file.path)).content);
    expect(saved.model.schemaVersion).toBe(7);
    expect(saved.model.blocks.find((b) => b.id === "state")?.kind).toEqual({
        type: "integrator",
        initial: [0.5],
    });
    expect(
        saved.model.blocks.find((b) => b.id === "state")?.parent,
    ).toBeTruthy();
    view.unmount();
    view = mount();
    await screen.findByRole("treeitem", { name: /Subsystem/ });
    fireEvent.click(screen.getByRole("button", { name: /▶ 运行/ }));
    await waitFor(() => expect(native.run).toHaveBeenCalledOnce());
    expect(native.run.mock.calls[0]?.[0]).toEqual(saved.model);
    view.unmount();
});

it("changes conditional execution, restores its control wire with undo, saves and reopens", async () => {
    const workspace = new MockWorkspaceClient();
    await workspace.connect();
    const root = await workspace.currentDirectory();
    await workspace.create("conditional.omsim", "file");
    const file = await workspace.read("conditional.omsim");
    const content = serializeDocument(example("triggeredCounter"));
    const saved = await workspace.write(
        file.path,
        content,
        file.revision,
        root.generation,
    );
    localStorage.setItem(
        `openmat.simulation.draft.v1:${root.path}`,
        JSON.stringify({
            content,
            saved: content,
            file: { path: file.path, revision: saved.revision },
        }),
    );
    const session = createRef<ModelEditorSession>();
    const mount = () =>
        render(
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
            />,
        );
    let view = mount();
    fireEvent.click(
        await screen.findByRole("treeitem", { name: /上升沿触发计数器/ }),
    );
    fireEvent.change(screen.getByLabelText("子系统执行方式"), {
        target: { value: "virtual" },
    });
    fireEvent.click(screen.getByText("应用执行设置"));
    expect(screen.queryByLabelText("触发边沿")).not.toBeInTheDocument();
    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    fireEvent.doubleClick(
        screen.getByRole("treeitem", { name: /上升沿触发计数器/ }),
    );
    fireEvent.click(await screen.findByRole("treeitem", { name: "Trigger" }));
    expect(await screen.findByLabelText("触发类型")).toHaveValue("rising");
    fireEvent.change(screen.getByLabelText("触发类型"), {
        target: { value: "either" },
    });
    await act(async () => {
        fireEvent.click(screen.getByText("应用参数"));
    });
    await act(async () => {
        expect(await session.current!.save()).toBe(true);
    });
    const document = parseDocument((await workspace.read(file.path)).content);
    expect(document.model.schemaVersion).toBe(8);
    expect(
        document.model.blocks.find((b) => b.id === "counter")!.kind,
    ).toMatchObject({ execution: { type: "triggered", edge: "either" } });
    expect(
        document.model.connections.some(
            (e) => e.to.block === "counter" && e.to.port === "trigger",
        ),
    ).toBe(true);
    view.unmount();
    view = mount();
    fireEvent.doubleClick(
        await screen.findByRole("treeitem", { name: /上升沿触发计数器/ }),
    );
    fireEvent.click(await screen.findByRole("treeitem", { name: "Trigger" }));
    expect(screen.getByLabelText("触发类型")).toHaveValue("either");
    fireEvent.click(screen.getByRole("button", { name: /▶ 运行/ }));
    await waitFor(() => expect(native.run).toHaveBeenCalledOnce());
    expect(native.run.mock.calls[0]?.[0]).toEqual(document.model);
    view.unmount();
});
