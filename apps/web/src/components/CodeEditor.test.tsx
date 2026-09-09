import { act, render, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import type * as Monaco from "monaco-editor/esm/vs/editor/editor.api";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  workspace: vi.fn((_api: unknown, _options: unknown) => ({
    start: vi.fn().mockReturnThis(),
    syncModels: vi.fn((_models: readonly { model: Monaco.editor.ITextModel }[]) => undefined),
    dispose: vi.fn(),
  })),
  editorProps: null as Record<string, unknown> | null,
}));

vi.mock("../lsp/monaco-lsp", () => ({
  OpenMatMonacoLspWorkspace: class {
    constructor(api: unknown, options: unknown) { return mocks.workspace(api, options); }
  },
}));
vi.mock("monaco-editor/esm/vs/editor/editor.worker?worker", () => ({ default: class {} }));
vi.mock("monaco-editor/esm/vs/editor/editor.api", () => ({}));
vi.mock("@monaco-editor/react", () => ({
  loader: { config: vi.fn() },
  default: (props: Record<string, unknown>) => {
    mocks.editorProps = props;
    return <div data-testid="mock-monaco" />;
  },
}));

import CodeEditor from "./CodeEditor";
import { SharedEditorSession } from "../lsp/shared-editor-session";

function harness() {
  const models = new Map<string, Monaco.editor.ITextModel>();
  const createModel = (text: string, _language: string, uri: Monaco.Uri) => {
    let value = text;
    let disposed = false;
    const listeners = new Set<() => void>();
    const model = {
      uri, getValue: () => value, getLineCount: () => value.split("\n").length,
      getLanguageId: () => "openmat",
      setValue: (next: string) => { value = next; listeners.forEach((fn) => fn()); },
      onDidChangeContent: (fn: () => void) => {
        listeners.add(fn);
        return { dispose: () => listeners.delete(fn) };
      },
      isDisposed: () => disposed,
      dispose: () => { disposed = true; models.delete(uri.toString()); },
    } as unknown as Monaco.editor.ITextModel;
    models.set(uri.toString(), model);
    return model;
  };
  const editor = {
    addCommand: vi.fn(), focus: vi.fn(),
    getModel: () => models.get(String(mocks.editorProps?.path)) ?? null,
    getPosition: () => ({ lineNumber: 1, column: 1 }),
    getScrollTop: () => 0, getScrollLeft: () => 0,
    setPosition: vi.fn(), setSelection: vi.fn(),
    revealLineInCenter: vi.fn(), setScrollPosition: vi.fn(),
    onDidChangeCursorPosition: vi.fn(() => ({ dispose: vi.fn() })),
    onDidScrollChange: vi.fn(() => ({ dispose: vi.fn() })),
  };
  const openerDispose = vi.fn();
  const api = {
    editor: {
      getModel: (uri: Monaco.Uri) => models.get(uri.toString()) ?? null,
      createModel,
      setModelLanguage: vi.fn(),
      registerEditorOpener: vi.fn((_opener: Monaco.editor.ICodeEditorOpener) => ({ dispose: openerDispose })),
    },
    Uri: { parse: (uri: string) => ({ toString: () => uri }) },
    KeyMod: { CtrlCmd: 1 }, KeyCode: { Enter: 2, KeyS: 4 },
  };
  const mount = async (editorInstance = editor) => {
    const uri = String(mocks.editorProps?.path);
    if (!models.has(uri)) createModel(String(mocks.editorProps?.defaultValue), "openmat", api.Uri.parse(uri) as Monaco.Uri);
    await act(async () => {
      const onMount = mocks.editorProps?.onMount as (editor: unknown, api: unknown) => void;
      onMount(editorInstance, api);
    });
  };
  return { models, editor, api, mount, openerDispose };
}

const baseProps = (): ComponentProps<typeof CodeEditor> => ({
  value: "a = 1;\nb = 2;\nc = 3;\nd = 4;",
  theme: "modern-light", onChange: vi.fn(), onViewStateChange: vi.fn(),
  onRun: vi.fn(), onSave: vi.fn(), lspUrl: "ws://127.0.0.1:49152/lsp",
  documentId: "demo", documentPath: "demo.m", documentUri: "file:///demo.m",
  documentVersion: 7, viewState: null,
});

beforeEach(() => { mocks.workspace.mockClear(); });

describe("CodeEditor workspace lifecycle", () => {
  it("captures the latest cursor immediately on desktop exit and removes the listener on unmount", async () => {
    const props = baseProps();
    const { editor, mount } = harness();
    const view = render(<CodeEditor {...props} />);
    await mount();
    vi.spyOn(editor, "getPosition").mockReturnValue({ lineNumber: 7, column: 4 });
    vi.spyOn(editor, "getScrollTop").mockReturnValue(90);
    act(() => window.dispatchEvent(new Event("openmat:prepare-close")));
    expect(props.onViewStateChange).toHaveBeenLastCalledWith(props.documentId,
      { lineNumber: 7, column: 4, scrollTop: 90, scrollLeft: 0 });
    view.unmount();
    vi.mocked(props.onViewStateChange).mockClear();
    act(() => window.dispatchEvent(new Event("openmat:prepare-close")));
    expect(props.onViewStateChange).not.toHaveBeenCalled();
  });
  it("shares one model, change subscription and language session across workbench and Designer", async () => {
    const props = baseProps();
    const { models, editor, mount } = harness();
    const session = new SharedEditorSession();
    const changed = vi.fn();
    session.configure({ url: props.lspUrl ?? null, onChange: changed });
    session.syncDocuments([{ id: props.documentId, uri: props.documentUri, content: props.value, version: 7 }]);
    const workbench = render(<CodeEditor {...props} editorSession={session} />);
    await mount();
    const model = models.get(props.documentUri)!;
    const designer = render(<CodeEditor {...props} editorSession={session} />);
    await mount({ ...editor });
    expect(mocks.workspace).toHaveBeenCalledOnce();
    expect(models.get(props.documentUri)).toBe(model);
    act(() => model.setValue("edited in either surface"));
    expect(changed).toHaveBeenCalledExactlyOnceWith(props.documentId, "edited in either surface");
    expect(props.onChange).not.toHaveBeenCalled();
    workbench.unmount();
    designer.unmount();
    await Promise.resolve();
    expect(model.isDisposed()).toBe(false);
    expect(mocks.workspace.mock.results[0]!.value.dispose).not.toHaveBeenCalled();
    session.dispose();
    await Promise.resolve();
    expect(model.isDisposed()).toBe(true);
    expect(mocks.workspace.mock.results[0]!.value.dispose).toHaveBeenCalledOnce();
  });

  it("awaits navigation when the target has not been opened yet", async () => {
    const props = baseProps();
    const { api, editor, mount } = harness();
    const onOpenDocument = vi.fn(async () => true);
    render(<CodeEditor {...props} onOpenDocument={onOpenDocument} />);
    await mount();
    const opener = api.editor.registerEditorOpener.mock.calls[0]![0];
    await expect(opener.openCodeEditor(editor as unknown as Monaco.editor.ICodeEditor,
      api.Uri.parse("file:///unopened.m") as Monaco.Uri)).resolves.toBe(true);
    expect(onOpenDocument).toHaveBeenCalledWith("file:///unopened.m", undefined);
  });

  it("keeps one connection across edits and document switches and honors targeted reveals", async () => {
    const props = baseProps();
    const { api, editor, mount, openerDispose } = harness();
    const view = render(<CodeEditor {...props} />);
    await mount();
    expect(mocks.workspace).toHaveBeenCalledExactlyOnceWith(api, { url: props.lspUrl });
    expect(mocks.editorProps?.keepCurrentModel).toBe(true);
    expect(mocks.editorProps?.defaultValue).toBe(props.value);
    expect(mocks.editorProps).not.toHaveProperty("value");
    expect(editor.addCommand).toHaveBeenCalledWith(3, expect.any(Function));
    expect(editor.addCommand).toHaveBeenCalledWith(5, expect.any(Function));
    const workspace = mocks.workspace.mock.results[0]!.value;

    view.rerender(<CodeEditor {...props} value="updated" documentVersion={8} />);
    view.rerender(<CodeEditor {...props} documentId="second" documentUri="file:///second.m" documentPath="second.m" />);
    expect(mocks.workspace).toHaveBeenCalledOnce();
    expect(workspace.dispose).not.toHaveBeenCalled();
    expect(workspace.syncModels.mock.calls.at(-1)?.[0][0]?.model.uri.toString()).toBe("file:///second.m");

    const reveal = { documentUri: props.documentUri, lineNumber: 3, column: 2, requestId: 1 };
    view.rerender(<CodeEditor {...props} documentUri="file:///second.m" reveal={reveal} />);
    expect(editor.revealLineInCenter).not.toHaveBeenCalled();
    view.rerender(<CodeEditor {...props} reveal={reveal} />);
    expect(editor.setPosition).toHaveBeenLastCalledWith({ lineNumber: 3, column: 2 });
    expect(editor.revealLineInCenter).toHaveBeenLastCalledWith(3);
    const jumps = editor.revealLineInCenter.mock.calls.length;
    view.rerender(<CodeEditor {...props} value="changed" reveal={reveal} />);
    expect(editor.revealLineInCenter).toHaveBeenCalledTimes(jumps);

    view.unmount();
    expect(workspace.dispose).toHaveBeenCalledOnce();
    expect(openerDispose).toHaveBeenCalledOnce();
  });

  it("syncs inactive documents and routes their edits and navigation to the workbench", async () => {
    const props = baseProps();
    const { models, api, editor, mount } = harness();
    const documents = [
      { id: "demo", uri: props.documentUri, content: props.value, version: 7 },
      { id: "helper", uri: "file:///helper.m", content: "function y = helper(x)\ny = x;\nend", version: 1 },
    ];
    const onWorkspaceDocumentChange = vi.fn();
    const onOpenDocument = vi.fn(() => true);
    const view = render(<CodeEditor {...props} workspaceDocuments={documents}
      onWorkspaceDocumentChange={onWorkspaceDocumentChange} onOpenDocument={onOpenDocument} />);
    await mount();
    const workspace = mocks.workspace.mock.results[0]!.value;
    expect(workspace.syncModels.mock.calls.at(-1)?.[0]).toHaveLength(2);
    act(() => models.get("file:///helper.m")!.setValue("renamed helper"));
    expect(onWorkspaceDocumentChange).toHaveBeenCalledExactlyOnceWith("helper", "renamed helper");
    expect(props.onChange).not.toHaveBeenCalled();
    const opener = api.editor.registerEditorOpener.mock.calls[0]![0];
    expect(opener.openCodeEditor(editor as unknown as Monaco.editor.ICodeEditor,
      api.Uri.parse("file:///helper.m") as Monaco.Uri, { lineNumber: 2, column: 5 })).toBe(true);
    expect(onOpenDocument).toHaveBeenCalledWith("file:///helper.m", {
      startLineNumber: 2, startColumn: 5, endLineNumber: 2, endColumn: 5,
    });
    view.rerender(<CodeEditor {...props} workspaceDocuments={[documents[0]!]}
      onWorkspaceDocumentChange={onWorkspaceDocumentChange} onOpenDocument={onOpenDocument} />);
    await waitFor(() => expect(models.has("file:///helper.m")).toBe(false));
    expect(mocks.workspace).toHaveBeenCalledOnce();
  });
});
