import {
    act,
    fireEvent,
    render,
    screen,
    waitFor,
    within,
} from "@testing-library/react";
import { createRef } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ModelEditor from "./ModelEditor";
import type { ModelEditorSession } from "./session";
import { MockWorkspaceClient } from "../workspace/mock-workspace-client";
import {
    PlatformContext,
    type PlatformServices,
} from "../platform/platform-services";
import { PendingOperations } from "../platform/pending-operations";
import { parseDocument, serializeDocument } from "./model";

// Browser acceptance exercises React Flow itself; these tests cover file/lifecycle contracts.
vi.mock("./SimulationCanvas", () => ({
    SimulationCanvas: () => <div>Graph surface</div>,
}));
vi.mock("./ScopePanel", () => ({ ScopePanel: () => <div>Scope surface</div> }));
beforeEach(() => localStorage.clear());

async function setup(platform?: PlatformServices) {
    const workspace = new MockWorkspaceClient();
    await workspace.connect();
    const root = await workspace.currentDirectory();
    const session = createRef<ModelEditorSession>(),
        onClose = vi.fn(),
        onOpenNative = vi.fn();
    const component = (
        <ModelEditor
            workspace={workspace}
            rootPath={root.path}
            rootGeneration={root.generation}
            visible
            theme="modern-light"
            onClose={onClose}
            onSaved={vi.fn()}
            onOpenNative={onOpenNative}
            sessionRef={session}
            pendingSaves={new PendingOperations()}
        />
    );
    const view = render(
        platform ? (
            <PlatformContext value={platform}>{component}</PlatformContext>
        ) : (
            component
        ),
    );
    await waitFor(() => expect(session.current).not.toBeNull());
    return { workspace, session, root, onClose, onOpenNative, ...view };
}
function rename(value: string) {
    const name = screen.getByRole("textbox", { name: "模型名称" });
    fireEvent.change(name, { target: { value } });
    fireEvent.blur(name);
}
async function saveNew(path: string) {
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

describe("Model Editor files and lifecycle", () => {
    it("starts clean, preserves names through save/recovery and rejects external revision conflicts", async () => {
        const { workspace, session, root, unmount } = await setup();
        expect(session.current!.dirty).toBe(false);
        rename("Measured feedback");
        expect(session.current!.dirty).toBe(true);
        await saveNew("feedback.omsim");
        expect(session.current!.dirty).toBe(false);
        const saved = await workspace.read("feedback.omsim");
        expect(parseDocument(saved.content).model.name).toBe(
            "Measured feedback",
        );
        const external = parseDocument(saved.content);
        external.model.name = "Edited elsewhere";
        await workspace.write(
            saved.path,
            serializeDocument(external),
            saved.revision,
            root.generation,
        );
        rename("Local unsaved change");
        await act(async () => {
            expect(await session.current!.save()).toBe(false);
        });
        expect(session.current!.dirty).toBe(true);
        expect(
            parseDocument((await workspace.read(saved.path)).content).model
                .name,
        ).toBe("Edited elsewhere");
        unmount();
        const recovery = JSON.parse(
            localStorage.getItem(`openmat.simulation.draft.v1:${root.path}`)!,
        );
        expect(parseDocument(recovery.content).model.name).toBe(
            "Local unsaved change",
        );
        expect(parseDocument(recovery.saved).model.name).toBe(
            "Measured feedback",
        );
    });
    it("retries a failed first write using the revision of the file it already created", async () => {
        const { workspace, session } = await setup();
        rename("Retry");
        const create = vi.spyOn(workspace, "create"),
            write = vi.spyOn(workspace, "write");
        write.mockRejectedValueOnce(new Error("Temporary write failure"));
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        const dialog = await screen.findByRole("dialog", { name: "保存模型" });
        fireEvent.change(
            within(dialog).getByRole("textbox", { name: "模型文件路径" }),
            { target: { value: "retry.omsim" } },
        );
        fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
        await waitFor(() => expect(session.current!.busy).toBe(false));
        expect(session.current!.dirty).toBe(true);
        fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
        await waitFor(() => expect(session.current!.dirty).toBe(false));
        expect(create).toHaveBeenCalledOnce();
        expect(
            parseDocument((await workspace.read("retry.omsim")).content).model
                .name,
        ).toBe("Retry");
    });
    it("confirms directory changes and restores the saved snapshot on explicit discard", async () => {
        const { session } = await setup();
        rename("Saved name");
        await saveNew("saved.omsim");
        rename("Discard me");
        let decision: Promise<boolean>;
        act(() => {
            decision = session.current!.prepareSwitch();
        });
        const dialog = screen.getByRole("dialog", { name: "未保存的模型" });
        fireEvent.click(
            within(dialog).getByRole("button", { name: "放弃修改" }),
        );
        await expect(decision!).resolves.toBe(true);
        expect(screen.getByRole("textbox", { name: "模型名称" })).toHaveValue(
            "Saved name",
        );
        expect(session.current!.dirty).toBe(false);
    });
    it("chains Save before close, and canceling a save keeps the pending directory unchanged", async () => {
        const { session, onClose } = await setup();
        rename("Save before close");
        fireEvent.click(screen.getByRole("button", { name: "关闭模型编辑器" }));
        fireEvent.click(
            within(
                screen.getByRole("dialog", { name: "未保存的模型" }),
            ).getByRole("button", { name: "保存" }),
        );
        const saveDialog = await screen.findByRole("dialog", {
            name: "保存模型",
        });
        fireEvent.change(within(saveDialog).getByRole("textbox"), {
            target: { value: "closed.omsim" },
        });
        fireEvent.click(
            within(saveDialog).getByRole("button", { name: "保存" }),
        );
        await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
        rename("Next change");
        let decision: Promise<boolean>;
        act(() => {
            decision = session.current!.prepareSwitch();
        });
        fireEvent.click(
            within(
                screen.getByRole("dialog", { name: "未保存的模型" }),
            ).getByRole("button", { name: "取消" }),
        );
        await expect(decision!).resolves.toBe(false);
        expect(session.current!.dirty).toBe(true);
    });
    it("uses Desktop file services without an upload action and preserves dirty state when the OS dialog is canceled", async () => {
        const saveTextFile = vi.fn().mockResolvedValue(null);
        const platform: PlatformServices = {
            kind: "desktop",
            saveExport: vi.fn(),
            files: {
                saveTextFile,
                pickDirectory: vi.fn(),
                pickFile: vi.fn(),
                revealPath: vi.fn(),
            },
        };
        const { session, onOpenNative } = await setup(platform);
        expect(
            screen.queryByRole("button", { name: "导入文件" }),
        ).not.toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "打开" }));
        expect(onOpenNative).toHaveBeenCalledOnce();
        rename("Native name");
        await act(async () => {
            expect(await session.current!.save()).toBe(false);
        });
        expect(saveTextFile).toHaveBeenCalledWith(
            "Native name.omsim",
            expect.any(String),
            expect.stringContaining("Native name"),
        );
        expect(session.current!.dirty).toBe(true);
    });
    it("does not commit a property value when Escape cancels the edit", async () => {
        const { session } = await setup();
        const input = screen.getByRole("textbox", { name: "模型名称" });
        input.focus();
        fireEvent.change(input, { target: { value: "Not committed" } });
        fireEvent.keyDown(input, { key: "Escape" });
        expect(input).toHaveValue("First order feedback");
        expect(session.current!.dirty).toBe(false);
    });
});
