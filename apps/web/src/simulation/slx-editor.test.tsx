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
import { example, PENDULUM_SOURCE } from "./examples";
import { parseDocument, serializeDocument } from "./model";
import { encodeSlx } from "./slx-authoring";
import { MockWorkspaceClient } from "../workspace/mock-workspace-client";
import { PendingOperations } from "../platform/pending-operations";
import type { ModelEditorSession } from "./session";

const native = vi.hoisted(() => ({
    run: vi.fn(),
    check: vi.fn(),
    importSlx: vi.fn(),
}));
vi.mock("./use-simulation-run", async (original) => {
    const state = {
        client: { current: { importSlx: native.importSlx } },
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
});

function authored() {
    const doc = example("pendulum");
    doc.schemaVersion = 4;
    doc.model.schemaVersion = 4;
    doc.sources = { "pendulum.m": PENDULUM_SOURCE };
    doc.slx = {
        name: "authored.slx",
        package: encodeSlx(new Uint8Array([80, 75, 1, 2])),
        parameters: "K=2;",
        appliedParameters: "K=2;",
        runnable: true,
        issues: [],
        document: {
            name: "authored",
            systems: [
                {
                    parentBlock: null,
                    blocks: [
                        {
                            sid: "1",
                            name: "Plant",
                            blockType: "SubSystem",
                            properties: {},
                            source: { part: "top.xml" },
                        },
                    ],
                    lines: [],
                },
                {
                    parentBlock: "1",
                    blocks: [
                        {
                            sid: "2",
                            name: "Gain",
                            blockType: "Gain",
                            properties: { Gain: "K" },
                            source: { part: "child.xml" },
                        },
                    ],
                    lines: [],
                },
            ],
        },
    };
    return doc;
}

it("navigates original subsystems and applies parameters before running a saved, movable snapshot", async () => {
    const doc = authored(),
        workspace = new MockWorkspaceClient();
    await workspace.connect();
    const root = await workspace.currentDirectory();
    await workspace.create("control.omsim", "file");
    const file = await workspace.read("control.omsim");
    const written = await workspace.write(
        file.path,
        serializeDocument(doc),
        file.revision,
        root.generation,
    );
    const content = serializeDocument(doc);
    localStorage.setItem(
        `openmat.simulation.draft.v1:${root.path}`,
        JSON.stringify({
            content,
            saved: content,
            file: { path: file.path, revision: written.revision },
        }),
    );
    const read = vi.spyOn(workspace, "read"),
        session = createRef<ModelEditorSession>();
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
    fireEvent.doubleClick(
        await screen.findByRole("button", { name: "Plant (SubSystem)" }),
    );
    expect(
        screen.getByRole("navigation", { name: "SLX 当前位置" }),
    ).toHaveTextContent("authored / Plant");
    fireEvent.click(screen.getByRole("button", { name: "Gain (Gain)" }));
    expect(screen.getByText("K", { selector: "dd" })).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "SLX 参数代码" }), {
        target: { value: "K=3;" },
    });
    expect(screen.getByRole("button", { name: /▶ 运行/ })).toBeDisabled();
    fireEvent.keyDown(window, { key: "F5" });
    expect(native.run).not.toHaveBeenCalled();
    native.importSlx.mockResolvedValueOnce({
        runnable: false,
        document: doc.slx!.document,
        issues: [
            {
                code: "unsupported",
                message: "Parameter check failed",
                block: "2",
            },
        ],
    });
    fireEvent.click(screen.getByRole("button", { name: "应用参数并检查" }));
    await waitFor(() => expect(session.current?.busy).toBe(false));
    expect(screen.getByRole("button", { name: /▶ 运行/ })).toBeDisabled();
    const updated = PENDULUM_SOURCE + "\n% frozen K=3\n";
    native.importSlx.mockResolvedValueOnce({
        runnable: true,
        document: doc.slx!.document,
        model: doc.model,
        sources: { "pendulum.m": updated },
        issues: [],
    });
    fireEvent.click(screen.getByRole("button", { name: "应用参数并检查" }));
    await waitFor(() =>
        expect(screen.getByRole("button", { name: /▶ 运行/ })).toBeEnabled(),
    );
    expect(native.importSlx.mock.lastCall?.[2]).toBe("K=3;");
    await act(async () => {
        expect(await session.current!.save()).toBe(true);
    });
    const saved = parseDocument(
        (await workspace.read("control.omsim")).content,
    );
    expect(saved.slx?.appliedParameters).toBe("K=3;");
    expect(saved.sources?.["pendulum.m"]).toBe(updated);
    view.unmount();
    view = mount();
    await screen.findByRole("textbox", { name: "SLX 参数代码" });
    fireEvent.click(screen.getByRole("button", { name: /▶ 运行/ }));
    await waitFor(() => expect(native.run).toHaveBeenCalledOnce());
    expect(native.run.mock.calls[0]?.[1].sources).toEqual({
        "pendulum.m": updated,
    });
    expect(read.mock.calls.some(([path]) => path.endsWith(".m"))).toBe(false);
    view.unmount();
});
