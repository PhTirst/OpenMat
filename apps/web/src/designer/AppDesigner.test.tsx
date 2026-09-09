import {
    act,
    fireEvent,
    render,
    screen,
    waitFor,
    within,
} from "@testing-library/react";
import { createRef, useState } from "react";
import type { DesignerSession } from "./designer-session";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AppDesigner from "./AppDesigner";
import { MockWorkspaceClient } from "../workspace/mock-workspace-client";
import { WorkspaceClientError } from "../workspace/workspace-client";
import { parseUi, serializeUi } from "./xml";
import { walk } from "./model";
import { SIGNAL_CODE, signalExample } from "./example";
import type { CodeEditorProps } from "../components/CodeEditor";
import type { DesignerSourceWorkspace } from "../documents/designer-source-workspace";
import { openDocumentFromWorkspaceFile, type OpenDocument } from "../documents/document-session";

const editorHarness = vi.hoisted(() => ({ props: null as CodeEditorProps | null }));

vi.mock("../components/FigureWindow", () => ({
    EmbeddedFigure: () => <div>Native plot surface</div>,
}));
vi.mock("../components/CodeEditor", () => ({
    default: (props: CodeEditorProps) => {
        editorHarness.props = props;
        const {
        value,
        onChange,
        documentPath,
        reveal,
        } = props;
        return (
        <textarea
            aria-label="回调代码"
            data-document-path={documentPath}
            data-reveal-line={reveal?.lineNumber}
            value={value}
            onChange={(e) => onChange(e.target.value)}
        />
        );
    },
}));
async function openDesigner(
    legacy = false,
    shared = false,
    options: { directOpen?: boolean; existingController?: string } = {},
) {
    const workspace = new MockWorkspaceClient();
    await workspace.connect();
    const directory = await workspace.currentDirectory();
    if (legacy) {
        const document = signalExample();
        document.version = 1;
        delete document.appClass;
        document.controller = "SignalApp";
        walk(document.root).forEach((node) => {
            node.events = {};
        });
        localStorage.setItem(
            `openmat.designer.draft.v1:${directory.path}`,
            JSON.stringify({
                xml: serializeUi(document),
                path: "SignalApp.omui",
                savedXml: "",
                savedCode: "",
                file: null,
                codeFile: null,
                code: "function SignalApp(app,event)\n% Legacy application\n% Native callback\nif strcmp(event.Source, 'RunButton')\n t = linspace(0,2,400);\nend\nend\n",
            }),
        );
    }
    const onSaved = vi.fn();
    if (options.existingController !== undefined) {
        const created = await workspace.create("SignalApp.m", "file");
        await workspace.write("SignalApp.m", options.existingController, created.revision!, directory.generation);
    }
    if (options.directOpen) {
        const created = await workspace.create("SignalApp.omui", "file");
        await workspace.write("SignalApp.omui", serializeUi(signalExample()), created.revision!, directory.generation);
    }
    const props = {
        workspace,
        rootPath: directory.path,
        rootGeneration: directory.generation,
        wsUrl: undefined,
        theme: "modern-light" as const,
        openRequest: options.directOpen ? { path: "SignalApp.omui", serial: 1 } : null,
        onClose: vi.fn(),
        onSaved,
    };
    let documents: readonly OpenDocument[] = [];
    let notify = (_next: readonly OpenDocument[]) => {};
    let setVisible = (_next: boolean) => {};
    const commit = (next: readonly OpenDocument[]) => {
        documents = next;
        notify(next);
    };
    const sharedSources: DesignerSourceWorkspace = {
        get documents() { return documents; },
        editorSession: {} as DesignerSourceWorkspace["editorSession"],
        lspUrl: "ws://localhost:8787/lsp",
        getSource: (path) => documents.find((document) => document.path === path),
        ensureSource: (source) => {
            const existing = sharedSources.getSource(source.path);
            if (existing) return existing;
            const document = {
                ...openDocumentFromWorkspaceFile(source.file ?? {
                    path: source.path, rootPath: directory.path,
                    rootGeneration: directory.generation, content: "",
                    revision: "", size: 0,
                }),
                content: source.content,
                savedContent: source.savedContent,
            };
            commit([...documents, document]);
            return document;
        },
        updateSource: (id, content) => commit(documents.map((document) =>
            document.id === id && document.content !== content
                ? { ...document, content, version: document.version + 1 }
                : document)),
        acceptSaved: (file) => commit(documents.map((document) =>
            document.path === file.path
                ? { ...document, savedContent: file.content, revision: file.revision }
                : document)),
        openDocument: vi.fn(async () => true),
    };
    function SharedDesigner() {
        const [current, setCurrent] = useState(documents);
        const [visible, changeVisible] = useState(true);
        notify = setCurrent;
        setVisible = changeVisible;
        return <AppDesigner {...props} visible={visible} sourceWorkspace={{ ...sharedSources, documents: current }} />;
    }
    const view = render(shared ? <SharedDesigner /> : <AppDesigner {...props} />);
    if (shared) await waitFor(() => expect(sharedSources.getSource("SignalApp.m")).toBeDefined());
    const recoverSource = (path: string, recoveryStatus: OpenDocument["recoveryStatus"]) =>
        commit(documents.map((document) => document.path === path
            ? { ...document, recoveryStatus, recoveryMessage: "Resolve the source recovery conflict first." }
            : document));
    const closeSource = (path: string) => commit(documents.filter((document) => document.path !== path));
    return { workspace, props, sharedSources, recoverSource, closeSource, setVisible, ...view };
}
describe("App Designer workbench", () => {
    it("recovers unapplied, invalid XML after a remount", async () => {
        const { props, unmount } = await openDesigner();
        fireEvent.click(screen.getByRole("button", { name: "XML" }));
        fireEvent.change(screen.getByLabelText("界面 XML"), { target: { value: "<unfinished-layout" } });
        unmount();
        render(<AppDesigner {...props} />);
        expect(screen.getByLabelText("界面 XML")).toHaveValue("<unfinished-layout");
    });

    it("saves a restored design with a new server root generation while retaining revision checks", async () => {
        const { workspace, props, unmount } = await openDesigner();
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        unmount();
        await workspace.changeDirectory(props.rootPath);
        const directory = await workspace.currentDirectory();
        const sessionRef = createRef<DesignerSession>();
        render(<AppDesigner {...props} rootGeneration={directory.generation} sessionRef={sessionRef} />);
        await act(async () => { expect(await sessionRef.current!.save()).toBe(true); });
        expect(props.onSaved).toHaveBeenCalledTimes(2);
    });
    beforeEach(() => localStorage.clear());
    it("shares current source content, model identity, versions and the language session with the workbench", async () => {
        const { sharedSources } = await openDesigner(false, true);
        const original = sharedSources.getSource("SignalApp.m")!;
        expect(original).toBeDefined();
        const fromWorkbench = `${original.content}\n% Edited in the workbench\n`;
        act(() => sharedSources.updateSource(original.id, fromWorkbench));
        fireEvent.click(screen.getByRole("button", { name: "代码" }));
        const editor = await screen.findByRole("textbox", { name: "回调代码" });
        expect(editor).toHaveValue(fromWorkbench);
        expect(editorHarness.props).toMatchObject({
            documentId: original.id,
            documentUri: original.uri,
            documentVersion: 2,
            lspUrl: "ws://localhost:8787/lsp",
            editorSession: sharedSources.editorSession,
        });
        expect(editorHarness.props?.workspaceDocuments).toEqual(sharedSources.documents);
        const fromDesigner = `${fromWorkbench}% Edited in the Designer\n`;
        fireEvent.change(editor, { target: { value: fromDesigner } });
        expect(sharedSources.getSource("SignalApp.m")?.content).toBe(fromDesigner);
        expect(editorHarness.props?.documentVersion).toBe(3);
        const range = { startLineNumber: 2, startColumn: 1, endLineNumber: 2, endColumn: 3 };
        await editorHarness.props?.onOpenDocument?.("file:///helper.m", range);
        expect(sharedSources.openDocument).toHaveBeenCalledWith("file:///helper.m", range);
    });
    it("preserves a newer shared edit during an in-flight Designer save and uses the current saved revision", async () => {
        const { workspace, props, sharedSources } = await openDesigner(true, true);
        fireEvent.click(screen.getByRole("button", { name: "代码" }));
        const editor = await screen.findByRole("textbox", { name: "回调代码" });
        const original = sharedSources.getSource("SignalApp.m")!;
        const first = `${original.content}% First saved revision\n`;
        fireEvent.change(editor, { target: { value: first } });
        const actualWrite = workspace.write.bind(workspace);
        let resume!: () => void;
        const pending = new Promise<void>((resolve) => { resume = resolve; });
        const write = vi.spyOn(workspace, "write").mockImplementationOnce(async (...args) => {
            await pending;
            return actualWrite(...args);
        });
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(write).toHaveBeenCalledOnce());
        const newer = `${first}% Typed while saving\n`;
        act(() => sharedSources.updateSource(original.id, newer));
        await act(async () => resume());
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        expect(sharedSources.getSource("SignalApp.m")).toMatchObject({
            content: newer,
            savedContent: first,
        });
        expect(editor).toHaveValue(newer);
        const savedByWorkbench = await actualWrite("SignalApp.m", newer,
            sharedSources.getSource("SignalApp.m")!.revision, props.rootGeneration);
        const savedFile = { ...await workspace.read("SignalApp.m"), revision: savedByWorkbench.revision! };
        act(() => sharedSources.acceptSaved(savedFile));
        const lastEdit = `${newer}% After workbench save\n`;
        fireEvent.change(editor, { target: { value: lastEdit } });
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledTimes(2));
        expect((await workspace.read("SignalApp.m")).content).toBe(lastEdit);
        expect(sharedSources.getSource("SignalApp.m")?.savedContent).toBe(lastEdit);
    });
    it("opens an independent callback from its existing workbench draft and keeps later edits when saving", async () => {
        const { workspace, props, sharedSources } = await openDesigner(true, true);
        const created = await workspace.create("onRun.m", "file");
        const disk = "function onRun(app, event)\nend\n";
        await workspace.write("onRun.m", disk, created.revision!, props.rootGeneration);
        const file = await workspace.read("onRun.m");
        const draft = disk.replace("end", "app.Status.Text = 'workbench';\nend");
        act(() => sharedSources.ensureSource({ path: file.path, content: draft, savedContent: disk, file }));
        const read = vi.spyOn(workspace, "read");
        fireEvent.click(screen.getByRole("button", { name: "RunButton Button" }));
        fireEvent.click(screen.getByRole("button", { name: "事件" }));
        const binding = screen.getByRole("textbox", { name: "Clicked" });
        fireEvent.change(binding, { target: { value: "onRun" } });
        fireEvent.blur(binding);
        fireEvent.click(screen.getByRole("button", { name: "编辑 Clicked 回调" }));
        const editor = await screen.findByRole("textbox", { name: "回调代码" });
        expect(editor).toHaveValue(draft);
        expect(read).not.toHaveBeenCalledWith("onRun.m");
        const latest = draft.replace("workbench", "latest shared draft");
        act(() => sharedSources.updateSource(sharedSources.getSource("onRun.m")!.id, latest));
        expect(editor).toHaveValue(latest);
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        expect((await workspace.read("onRun.m")).content).toBe(latest);
    });
    it("does not bypass shared source recovery conflicts when saving the design", async () => {
        const { workspace, props, recoverSource } = await openDesigner(false, true);
        act(() => recoverSource("SignalApp.m", "conflict"));
        const create = vi.spyOn(workspace, "create");
        const write = vi.spyOn(workspace, "write");
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("Resolve the source recovery conflict first.");
        expect(create).not.toHaveBeenCalled();
        expect(write).not.toHaveBeenCalled();
        expect(props.onSaved).not.toHaveBeenCalled();
    });
    it("loads the real controller when directly opening a design instead of registering the default example", async () => {
        const actual = `${SIGNAL_CODE}\n% Existing application on disk\n`;
        const { workspace, props, sharedSources } = await openDesigner(false, true, {
            directOpen: true,
            existingController: actual,
        });
        expect(sharedSources.getSource("SignalApp.m")).toMatchObject({ content: actual, savedContent: actual });
        expect(sharedSources.getSource("SignalApp.m")?.revision).not.toBe("");
        fireEvent.click(screen.getByRole("button", { name: "代码" }));
        expect(await screen.findByRole("textbox", { name: "回调代码" })).toHaveValue(actual);
        const create = vi.spyOn(workspace, "create");
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        expect(create).not.toHaveBeenCalled();
        expect((await workspace.read("SignalApp.m")).content).toContain("Existing application on disk");
    });
    it("keeps a discarded source closed while hidden and reloads its disk contents on explicit return", async () => {
        const disk = `${SIGNAL_CODE}\n% Saved disk content\n`;
        const { props, sharedSources, setVisible, closeSource } = await openDesigner(false, true, { existingController: disk });
        fireEvent.click(screen.getByRole("button", { name: "代码" }));
        const editor = await screen.findByRole("textbox", { name: "回调代码" });
        expect(editor).toHaveValue(disk);
        fireEvent.change(editor, { target: { value: `${disk}% Discarded draft\n` } });
        act(() => setVisible(false));
        act(() => closeSource("SignalApp.m"));
        await act(async () => {});
        expect(sharedSources.getSource("SignalApp.m")).toBeUndefined();
        expect(screen.queryByRole("textbox", { name: "回调代码" })).not.toBeInTheDocument();
        expect(JSON.parse(localStorage.getItem(`openmat.designer.draft.v1:${props.rootPath}`)!).code).toBe(disk);
        act(() => setVisible(true));
        expect(await screen.findByRole("textbox", { name: "回调代码" })).toHaveValue(disk);
        expect(sharedSources.getSource("SignalApp.m")?.content).toBe(disk);
    });
    it("opens the canvas context menu and operates on the clicked component with undo", async () => {
        const { container } = await openDesigner();
        fireEvent.contextMenu(
            container.querySelector(".designer-artboard .ui-Button")!,
            { clientX: 400, clientY: 300 },
        );
        const menu = screen.getByRole("menu", { name: "组件操作" });
        expect(menu).toHaveTextContent("RunButton");
        fireEvent.click(
            within(menu).getByRole("menuitem", { name: /复制组件/ }),
        );
        expect(
            screen.getByRole("button", { name: "RunButtonCopy Button" }),
        ).toBeInTheDocument();
        expect(screen.queryByRole("menu")).not.toBeInTheDocument();
        fireEvent.contextMenu(
            screen.getByRole("button", { name: "RunButton Button" }),
        );
        fireEvent.click(screen.getByRole("menuitem", { name: /删除组件/ }));
        expect(
            screen.queryByRole("button", { name: "RunButton Button" }),
        ).not.toBeInTheDocument();
        expect(
            screen.getByRole("button", { name: "RunButtonCopy Button" }),
        ).toBeInTheDocument();
        fireEvent.click(screen.getByTitle("撤销 Ctrl+Z"));
        expect(
            screen.getByRole("button", { name: "RunButton Button" }),
        ).toBeInTheDocument();
    });
    it("retains a right-clicked multi-selection, deletes it atomically, and replaces it for another target", async () => {
        await openDesigner();
        fireEvent.click(
            screen.getByRole("button", { name: "FrequencyLabel Label" }),
        );
        fireEvent.click(
            screen.getByRole("button", { name: "AmplitudeLabel Label" }),
            { shiftKey: true },
        );
        fireEvent.contextMenu(
            screen.getByRole("button", { name: "FrequencyLabel Label" }),
        );
        expect(screen.getByRole("menu")).toHaveTextContent("已选择 2 个组件");
        expect(screen.getByRole("menuitem", { name: /重命名/ })).toBeDisabled();
        fireEvent.click(screen.getByRole("menuitem", { name: /删除组件/ }));
        expect(
            screen.queryByRole("button", { name: "FrequencyLabel Label" }),
        ).not.toBeInTheDocument();
        expect(
            screen.queryByRole("button", { name: "AmplitudeLabel Label" }),
        ).not.toBeInTheDocument();
        fireEvent.click(screen.getByTitle("撤销 Ctrl+Z"));
        expect(
            screen.getByRole("button", { name: "FrequencyLabel Label" }),
        ).toBeInTheDocument();
        expect(
            screen.getByRole("button", { name: "AmplitudeLabel Label" }),
        ).toBeInTheDocument();
        fireEvent.contextMenu(
            screen.getByRole("button", { name: "RunButton Button" }),
        );
        expect(screen.getByRole("menuitem", { name: /重命名/ })).toBeEnabled();
        expect(screen.getByRole("textbox", { name: "组件名称" })).toHaveValue(
            "RunButton",
        );
    });
    it("protects the root and supports keyboard invocation, navigation, dismissal and rename", async () => {
        await openDesigner();
        const root = screen.getByRole("button", { name: "MainWindow Window" });
        fireEvent.keyDown(root, { key: "F10", shiftKey: true });
        expect(
            screen.getByRole("menuitem", { name: /复制组件/ }),
        ).toBeDisabled();
        expect(
            screen.getByRole("menuitem", { name: /删除组件/ }),
        ).toBeDisabled();
        fireEvent.keyDown(screen.getByRole("menu"), { key: "Delete" });
        expect(root).toBeInTheDocument();
        fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
        expect(root).toHaveFocus();
        const button = screen.getByRole("button", { name: "RunButton Button" });
        fireEvent.keyDown(button, { key: "ContextMenu" });
        expect(
            screen.getByRole("menuitem", { name: /复制组件/ }),
        ).toHaveFocus();
        fireEvent.keyDown(screen.getByRole("menuitem", { name: /复制组件/ }), {
            key: "ArrowDown",
        });
        expect(screen.getByRole("menuitem", { name: /重命名/ })).toHaveFocus();
        fireEvent.click(screen.getByRole("menuitem", { name: /重命名/ }));
        expect(screen.getByRole("textbox", { name: "组件名称" })).toHaveFocus();
        fireEvent.contextMenu(button);
        fireEvent.pointerDown(
            screen.getByRole("textbox", { name: "搜索组件" }),
        );
        expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    });
    it("bulk edits mixed properties, wraps a selection, and persists window sizing with one undo per operation", async () => {
        const { workspace, props } = await openDesigner();
        fireEvent.click(
            screen.getByRole("button", { name: "FrequencyLabel Label" }),
        );
        fireEvent.click(
            screen.getByRole("button", { name: "AmplitudeLabel Label" }),
            { shiftKey: true },
        );
        expect(screen.getByText("已选择 2 个组件")).toBeInTheDocument();
        const text = screen.getByRole("textbox", { name: "文本" });
        expect(text).toHaveAttribute("placeholder", "多个值");
        fireEvent.change(text, { target: { value: "共同标题" } });
        fireEvent.blur(text);
        expect(screen.getByRole("textbox", { name: "文本" })).toHaveValue(
            "共同标题",
        );
        fireEvent.click(screen.getByTitle("撤销 Ctrl+Z"));
        expect(screen.getByRole("textbox", { name: "文本" })).toHaveAttribute(
            "placeholder",
            "多个值",
        );
        fireEvent.click(screen.getByTitle("重做 Ctrl+Shift+Z"));
        fireEvent.change(
            screen.getByRole("combobox", { name: "包入布局容器" }),
            { target: { value: "ColumnLayout" } },
        );
        expect(
            screen.getByRole("button", { name: "ColumnLayout1 ColumnLayout" }),
        ).toBeInTheDocument();
        fireEvent.click(screen.getByTitle("撤销 Ctrl+Z"));
        expect(
            screen.queryByRole("button", {
                name: "ColumnLayout1 ColumnLayout",
            }),
        ).not.toBeInTheDocument();
        const width = screen.getByRole("spinbutton", { name: "预览宽度" });
        fireEvent.change(width, { target: { value: "1100" } });
        fireEvent.blur(width);
        fireEvent.click(
            screen.getByRole("button", { name: "FrequencyLabel Label" }),
        );
        fireEvent.click(screen.getByRole("button", { name: "布局" }));
        expect(screen.getByRole("spinbutton", { name: "X" })).toBeDisabled();
        fireEvent.change(screen.getByRole("combobox", { name: "宽度策略" }), {
            target: { value: "fill" },
        });
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        const saved = parseUi((await workspace.read("SignalApp.omui")).content);
        expect(saved.root.properties.Width).toBe(1100);
        expect(
            walk(saved.root).find((n) => n.name === "FrequencyLabel")!.layout
                .widthMode,
        ).toBe("fill");
        expect(
            walk(saved.root).filter((n) => n.properties.Text === "共同标题"),
        ).toHaveLength(2);
        fireEvent.click(screen.getByRole("button", { name: "打开" }));
        fireEvent.click(screen.getByRole("button", { name: "属性" }));
        await waitFor(() =>
            expect(
                screen.getByRole("textbox", { name: "组件名称" }),
            ).toHaveValue("MainWindow"),
        );
        expect(
            screen.getByRole("spinbutton", { name: "预览宽度" }),
        ).toHaveValue(1100);
    });
    it("creates a reusable component, renames its class, edits layout and reopens its definition", async () => {
        const { workspace, props } = await openDesigner();
        fireEvent.click(screen.getByRole("button", { name: "新建组件" }));
        fireEvent.click(screen.getByRole("button", { name: "放弃修改并继续" }));
        const className = screen.getByRole("textbox", { name: "组件类名称" });
        fireEvent.change(className, { target: { value: "CustomPanel" } });
        fireEvent.blur(className);
        fireEvent.click(screen.getByRole("button", { name: "布局" }));
        fireEvent.change(screen.getByRole("combobox", { name: "子组件布局" }), {
            target: { value: "grid" },
        });
        fireEvent.click(screen.getByRole("button", { name: "按钮" }));
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        const source = (await workspace.read("CustomPanel.m")).content;
        expect(source).toContain("classdef CustomPanel < openmat.ui.Panel");
        expect(source).toContain("function app = CustomPanel()");
        expect(source).toContain("components.Button1 = openmat.ui.Button();");
        const saved = parseUi(
            (await workspace.read("CustomPanel.omui")).content,
        );
        expect(saved.kind).toBe("component");
        expect(saved.root.layout.mode).toBe("grid");
        fireEvent.click(screen.getByRole("button", { name: "打开" }));
        await waitFor(() =>
            expect(
                screen.getByRole("button", { name: "Root Panel" }),
            ).toBeInTheDocument(),
        );
        expect(screen.getByRole("textbox", { name: "组件类名称" })).toHaveValue(
            "CustomPanel",
        );
    });
    it("shows all component icons and edits a component through the inspector with undo", async () => {
        const { container } = await openDesigner();
        expect(
            container.querySelectorAll(".designer-component-grid svg"),
        ).toHaveLength(24);
        fireEvent.click(screen.getByRole("button", { name: "标签" }));
        const input = screen.getByRole("textbox", {
            name: "文本",
        });
        fireEvent.change(input, { target: { value: "新增标签" } });
        fireEvent.blur(input);
        expect(
            container.querySelector(".designer-artboard")?.textContent,
        ).toContain("新增标签");
        fireEvent.click(screen.getByTitle("撤销 Ctrl+Z"));
        expect(
            container.querySelector(".designer-artboard")?.textContent,
        ).not.toContain("新增标签");
        fireEvent.click(screen.getByTitle("重做 Ctrl+Shift+Z"));
        expect(
            container.querySelector(".designer-artboard")?.textContent,
        ).toContain("新增标签");
    });
    it("saves XML and callback source with revisions and reloads stable component IDs", async () => {
        const { workspace, props } = await openDesigner();
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        const saved = await workspace.read("SignalApp.omui");
        expect((await workspace.read("SignalApp.m")).content).toContain(
            "classdef SignalApp",
        );
        const document = parseUi(saved.content);
        expect(walk(document.root)).toHaveLength(15);
        fireEvent.click(screen.getByRole("button", { name: "打开" }));
        await waitFor(() =>
            expect(
                screen.getByRole("textbox", { name: "组件名称" }),
            ).toHaveValue("MainWindow"),
        );
        fireEvent.click(
            within(
                screen.getByRole("navigation", { name: "设计器视图" }),
            ).getByRole("button", { name: "XML" }),
        );
        expect(screen.getByRole("textbox", { name: "界面 XML" })).toHaveValue(
            saved.content,
        );
    });
    it("accepts palette copy drops and can undo the inserted control", async () => {
        const { container } = await openDesigner();
        const transfer = {
            types: ["application/x-openmat-component"],
            effectAllowed: "copy",
            dropEffect: "none",
            getData: (type: string) =>
                type === "application/x-openmat-component" ? "Button" : "",
            setData: vi.fn(),
        };
        const root = container.querySelector(".designer-artboard .ui-Window")!;
        fireEvent.dragOver(root, { dataTransfer: transfer });
        expect(transfer.dropEffect).toBe(transfer.effectAllowed);
        const drop = new MouseEvent("drop", {
            bubbles: true,
            clientX: 300,
            clientY: 200,
        });
        Object.defineProperty(drop, "dataTransfer", { value: transfer });
        fireEvent(root, drop);
        expect(screen.getByRole("textbox", { name: "组件名称" })).toHaveValue(
            "Button1",
        );
        fireEvent.click(screen.getByTitle("撤销 Ctrl+Z"));
        expect(
            container.querySelectorAll(".designer-artboard .ui-Button"),
        ).toHaveLength(1);
    });
    it("applies edited XML with extended constraints and allows one-step undo", async () => {
        await openDesigner();
        fireEvent.click(
            within(
                screen.getByRole("navigation", { name: "设计器视图" }),
            ).getByRole("button", { name: "XML" }),
        );
        const source = screen.getByRole("textbox", {
            name: "界面 XML",
        }) as HTMLTextAreaElement;
        const original = source.value,
            document = parseUi(original);
        document.root.layout.columnTracks = "260 1fr";
        fireEvent.change(source, { target: { value: serializeUi(document) } });
        fireEvent.click(screen.getByRole("button", { name: "应用 XML" }));
        expect(screen.queryByRole("alert")).not.toBeInTheDocument();
        expect(source.value).toContain('layoutVersion="2"');
        fireEvent.click(screen.getByTitle("撤销 Ctrl+Z"));
        expect(source.value).toBe(original);
    });
    it("preserves the last valid design when XML cannot be applied", async () => {
        const { container } = await openDesigner();
        fireEvent.click(
            within(
                screen.getByRole("navigation", { name: "设计器视图" }),
            ).getByRole("button", { name: "XML" }),
        );
        fireEvent.change(screen.getByRole("textbox", { name: "界面 XML" }), {
            target: { value: "<bad>" },
        });
        fireEvent.click(screen.getByRole("button", { name: "应用 XML" }));
        expect(screen.getByRole("alert")).toHaveTextContent("XML 语法错误");
        fireEvent.click(
            within(
                screen.getByRole("navigation", { name: "设计器视图" }),
            ).getByRole("button", { name: "设计" }),
        );
        expect(container.querySelector(".designer-artboard")).toHaveTextContent(
            "信号分析",
        );
    });
    it("reports external revision conflicts and never overwrites newer file contents", async () => {
        const { workspace, props } = await openDesigner();
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        const saved = await workspace.read("SignalApp.omui");
        await workspace.write(
            saved.path,
            saved.content + "<!-- external -->",
            saved.revision,
            saved.rootGeneration,
        );
        const title = screen.getByRole("textbox", {
            name: "标题",
        });
        fireEvent.change(title, { target: { value: "My changed title" } });
        fireEvent.blur(title);
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() =>
            expect(screen.getByRole("alert")).toBeInTheDocument(),
        );
        expect((await workspace.read(saved.path)).content).toContain(
            "<!-- external -->",
        );
        expect(props.onSaved).toHaveBeenCalledOnce();
    });
    it("retries the first write after creation without recreating or overwriting an unrelated file", async () => {
        const { workspace, props } = await openDesigner();
        const write = vi
            .spyOn(workspace, "write")
            .mockRejectedValueOnce(new Error("Temporary write failure"));
        const create = vi.spyOn(workspace, "create");
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() =>
            expect(screen.getByRole("alert")).toHaveTextContent(
                "Temporary write failure",
            ),
        );
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        expect(
            create.mock.calls.filter(([path]) => path === "SignalApp.m"),
        ).toHaveLength(1);
        expect(write).toHaveBeenCalledTimes(3);
        expect((await workspace.read("SignalApp.m")).content).toContain(
            "classdef SignalApp",
        );
    });
    it("reveals the effective inherited callback and edits and saves its actual .m source", async () => {
        const { workspace, props } = await openDesigner(true);
        fireEvent.click(
            screen.getByRole("button", { name: "RunButton Button" }),
        );
        fireEvent.click(screen.getByRole("button", { name: "事件" }));
        expect(screen.getByRole("textbox", { name: "Clicked" })).toHaveValue(
            "SignalApp",
        );
        expect(screen.getByText("使用应用回调")).toBeInTheDocument();
        expect(screen.getByText("SignalApp.m")).toBeInTheDocument();
        fireEvent.click(
            screen.getByRole("button", { name: "编辑 Clicked 回调" }),
        );
        const editor = await screen.findByRole("textbox", { name: "回调代码" });
        expect(editor).toHaveAttribute("data-document-path", "SignalApp.m");
        expect(editor).toHaveAttribute("data-reveal-line", "4");
        expect((editor as HTMLTextAreaElement).value).toContain(
            "function SignalApp",
        );
        fireEvent.change(editor, {
            target: {
                value: "function SignalApp(app, event)\napp.Status.Text = 'edited';\nend\n",
            },
        });
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        expect((await workspace.read("SignalApp.m")).content).toContain(
            "'edited'",
        );
        expect(
            walk(
                parseUi((await workspace.read("SignalApp.omui")).content).root,
            ).find((n) => n.name === "RunButton")?.events,
        ).toEqual({});
    });
    it("edits and creates class member handlers and persists the generated instance wiring", async () => {
        const { workspace, props } = await openDesigner();
        fireEvent.click(
            screen.getByRole("button", { name: "RunButton Button" }),
        );
        fireEvent.click(screen.getByRole("button", { name: "事件" }));
        const binding = screen.getByRole("combobox", { name: "Clicked" });
        expect(binding).toHaveValue("app.onRun");
        expect(screen.getByText("实例成员方法")).toBeInTheDocument();
        fireEvent.click(
            screen.getByRole("button", { name: "编辑 Clicked 回调" }),
        );
        const editor = await screen.findByRole("textbox", { name: "回调代码" });
        expect(editor).toHaveAttribute("data-document-path", "SignalApp.m");
        const original = (editor as HTMLTextAreaElement).value;
        const line = Number(editor.getAttribute("data-reveal-line"));
        expect(original.split("\n")[line - 1]).toContain(
            "function onRun(app, source, event)",
        );
        fireEvent.change(binding, { target: { value: "" } });
        fireEvent.blur(binding);
        expect(screen.getByText("尚未绑定回调")).toBeInTheDocument();
        fireEvent.change(binding, { target: { value: "app.onAnalyze" } });
        // The path is already shown before blur, so a pointer click cannot lose
        // its target when committing the input causes the inspector to render.
        expect(screen.getByText("实例成员方法")).toBeInTheDocument();
        fireEvent.blur(binding);
        fireEvent.click(
            screen.getByRole("button", { name: "编辑 Clicked 回调" }),
        );
        expect((editor as HTMLTextAreaElement).value).toContain(
            "function onAnalyze(obj, source, event)",
        );
        expect((editor as HTMLTextAreaElement).value).toContain(
            "function onRun(app, source, event)",
        );
        fireEvent.change(editor, {
            target: {
                value: (editor as HTMLTextAreaElement).value.replace(
                    "% 在此处理事件。obj 是此类的当前实例。",
                    "obj.Status.Text = 'Member callback';",
                ),
            },
        });
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        const saved = (await workspace.read("SignalApp.m")).content;
        expect(saved).toContain("classdef SignalApp < openmat.ui.AppBase");
        expect(saved).toContain("obj.Status.Text = 'Member callback';");
        expect(saved).toContain(
            "app.listen(app.RunButton, 'Clicked', @(source, event) app.onAnalyze(source, event));",
        );
        expect(
            parseUi((await workspace.read("SignalApp.omui")).content).version,
        ).toBe(2);
        await expect(workspace.read("onAnalyze.m")).rejects.toMatchObject({
            code: "workspace.notFound",
        });
    });
    it("opens independent callbacks, preserves drafts across file switches and saves the right files", async () => {
        const { workspace, props } = await openDesigner(true);
        const created = await workspace.create("onRun.m", "file");
        await workspace.write(
            "onRun.m",
            "function onRun(app, event)\nend\n",
            created.revision!,
            props.rootGeneration,
        );
        fireEvent.click(
            screen.getByRole("button", { name: "RunButton Button" }),
        );
        fireEvent.click(screen.getByRole("button", { name: "事件" }));
        const binding = screen.getByRole("textbox", { name: "Clicked" });
        fireEvent.change(binding, { target: { value: "onRun" } });
        fireEvent.blur(binding);
        fireEvent.click(
            screen.getByRole("button", { name: "编辑 Clicked 回调" }),
        );
        const editor = await screen.findByRole("textbox", { name: "回调代码" });
        await waitFor(() =>
            expect(editor).toHaveAttribute("data-document-path", "onRun.m"),
        );
        const changed =
            "function onRun(app, event)\napp.Status.Text = 'separate';\nend\n";
        fireEvent.change(editor, { target: { value: changed } });
        fireEvent.click(screen.getByRole("button", { name: "编辑应用回调" }));
        expect(editor).toHaveAttribute("data-document-path", "SignalApp.m");
        fireEvent.change(screen.getByRole("combobox", { name: "回调文件" }), {
            target: { value: "onRun.m" },
        });
        expect(editor).toHaveValue(changed);
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        expect((await workspace.read("onRun.m")).content).toBe(changed);
        expect((await workspace.read("SignalApp.m")).content).toContain(
            "linspace",
        );
        expect(
            walk(
                parseUi((await workspace.read("SignalApp.omui")).content).root,
            ).find((n) => n.name === "RunButton")?.events.Clicked,
        ).toBe("onRun");
    });
    it("creates missing callback templates but never treats permission errors as a missing file", async () => {
        const { workspace, props } = await openDesigner(true);
        fireEvent.click(
            screen.getByRole("button", { name: "RunButton Button" }),
        );
        fireEvent.click(screen.getByRole("button", { name: "事件" }));
        let binding = screen.getByRole("textbox", { name: "Clicked" });
        fireEvent.change(binding, { target: { value: "newClick" } });
        fireEvent.blur(binding);
        fireEvent.click(
            screen.getByRole("button", { name: "编辑 Clicked 回调" }),
        );
        const editor = await screen.findByRole("textbox", { name: "回调代码" });
        await waitFor(() =>
            expect(editor).toHaveAttribute("data-document-path", "newClick.m"),
        );
        expect((editor as HTMLTextAreaElement).value).toContain(
            "function newClick(app, event)",
        );
        await expect(workspace.read("newClick.m")).rejects.toMatchObject({
            code: "workspace.notFound",
        });
        fireEvent.click(screen.getByRole("button", { name: "保存" }));
        await waitFor(() => expect(props.onSaved).toHaveBeenCalledOnce());
        expect((await workspace.read("newClick.m")).content).toContain(
            "function newClick",
        );
        binding = screen.getByRole("textbox", { name: "Clicked" });
        fireEvent.change(binding, { target: { value: "unreadable" } });
        fireEvent.blur(binding);
        vi.spyOn(workspace, "read").mockRejectedValueOnce(
            new WorkspaceClientError(
                "workspace.permissionDenied",
                "Read denied",
            ),
        );
        fireEvent.click(
            screen.getByRole("button", { name: "编辑 Clicked 回调" }),
        );
        await waitFor(() =>
            expect(screen.getByRole("alert")).toHaveTextContent("Read denied"),
        );
        expect(editor).toHaveAttribute("data-document-path", "newClick.m");
    });
});
