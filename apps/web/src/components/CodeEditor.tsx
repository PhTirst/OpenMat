import Editor, {
  loader,
  type BeforeMount,
  type OnMount,
} from "@monaco-editor/react";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import "monaco-editor/esm/vs/editor/contrib/suggest/browser/suggestController";
import "monaco-editor/esm/vs/editor/contrib/gotoSymbol/browser/goToCommands";
import "monaco-editor/esm/vs/editor/standalone/browser/referenceSearch/standaloneReferenceSearch";
import "monaco-editor/esm/vs/editor/contrib/gotoSymbol/browser/link/goToDefinitionAtPosition";
import "monaco-editor/esm/vs/editor/contrib/rename/browser/rename";
import "monaco-editor/esm/vs/editor/contrib/hover/browser/hoverContribution";
import "monaco-editor/esm/vs/editor/contrib/contextmenu/browser/contextmenu";
import { useEffect, useMemo, useRef, useState } from "react";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import type { DocumentViewState } from "../documents/document-session";
import { OpenMatMonacoLspWorkspace } from "../lsp/monaco-lsp";
import type { SharedEditorSession } from "../lsp/shared-editor-session";
import {
  WorkspaceEditorModels,
  type WorkspaceEditorDocument,
} from "../lsp/workspace-editor-models";
import { resolveLspWebSocketUrl } from "../lsp/url";
import { kernelWebSocketUrl } from "../runtime-config";
import { DESKTOP_CLOSE_PREPARE_EVENT } from "../platform/desktop-lifecycle";
import { monacoTheme, type IdeTheme } from "../theme";

type MonacoHost = typeof globalThis & {
  MonacoEnvironment?: {
    getWorker: () => Worker;
  };
};

const monacoHost = globalThis as MonacoHost;
monacoHost.MonacoEnvironment = {
  getWorker: () => new editorWorker(),
};
loader.config({ monaco });

const configureOpenMat: BeforeMount = (monacoApi) => {
  monacoApi.editor.defineTheme("openmat-modern-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "comment", foreground: "7F9F8F" },
      { token: "keyword", foreground: "7AB8F5" },
      { token: "number", foreground: "D6B46A" },
      { token: "string", foreground: "93C47D" },
    ],
    colors: {
      "editor.background": "#20252B",
      "editor.foreground": "#DCE4EE",
      "editor.lineHighlightBackground": "#272D35",
      "editorCursor.foreground": "#70B9FF",
      "editor.selectionBackground": "#253D54",
    },
  });
  monacoApi.editor.defineTheme("openmat-modern-light", {
    base: "vs",
    inherit: true,
    rules: [
      { token: "comment", foreground: "537A65" },
      { token: "keyword", foreground: "075EA8" },
      { token: "number", foreground: "8A5A00" },
      { token: "string", foreground: "2E6B2E" },
    ],
    colors: {
      "editor.background": "#FFFFFF",
      "editor.foreground": "#283342",
      "editor.lineHighlightBackground": "#F5F7FA",
      "editorCursor.foreground": "#176BBA",
      "editor.selectionBackground": "#E7F1FC",
    },
  });

  if (monacoApi.languages.getLanguages().some(({ id }) => id === "openmat")) {
    return;
  }

  monacoApi.languages.register({ id: "openmat" });
  monacoApi.languages.setMonarchTokensProvider("openmat", {
    keywords: [
      "break",
      "case",
      "catch",
      "classdef",
      "continue",
      "else",
      "elseif",
      "end",
      "for",
      "function",
      "global",
      "if",
      "otherwise",
      "persistent",
      "return",
      "switch",
      "try",
      "while",
    ],
    tokenizer: {
      root: [
        [/%\{/, "comment", "@comment"],
        [/%.*$/, "comment"],
        [/[a-zA-Z_]\w*/, { cases: { "@keywords": "keyword", "@default": "identifier" } }],
        [/\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/, "number"],
        [/"(?:[^"]|"")*"/, "string"],
        [/'(?:[^']|'')*'/, "string"],
        [/[+\-*\/.^=<>~:&|]+/, "operator"],
      ],
      comment: [
        [/%\}/, "comment", "@pop"],
        [/./, "comment"],
      ],
    },
  });
  monacoApi.languages.setLanguageConfiguration("openmat", {
    comments: { lineComment: "%", blockComment: ["%{", "%}"] },
    brackets: [
      ["{", "}"],
      ["[", "]"],
      ["(", ")"],
    ],
    autoClosingPairs: [
      { open: "{", close: "}" },
      { open: "[", close: "]" },
      { open: "(", close: ")" },
      { open: "\"", close: "\"" },
      { open: "'", close: "'" },
    ],
  });
};

export interface CodeEditorProps {
  readonly value: string;
  readonly theme: IdeTheme;
  readonly documentId: string;
  readonly documentPath: string;
  readonly documentUri: string;
  readonly documentVersion: number;
  readonly viewState: DocumentViewState | null;
  readonly onChange: (value: string) => void;
  readonly onViewStateChange: (
    documentId: string,
    viewState: DocumentViewState,
  ) => void;
  readonly onRun: () => void;
  readonly onSave: () => void;
  /** WebSocket endpoint for the standard JSON-RPC/LSP session. `null` disables LSP. */
  readonly lspUrl?: string | null;
  readonly workspaceDocuments?: readonly WorkspaceEditorDocument[];
  readonly editorSession?: SharedEditorSession;
  readonly onWorkspaceDocumentChange?: (documentId: string, content: string) => void;
  readonly onOpenDocument?: (
    uri: string,
    selection?: monaco.IRange,
  ) => boolean | Promise<boolean>;
  readonly reveal?: {
    readonly lineNumber: number;
    readonly column?: number;
    readonly endLineNumber?: number;
    readonly endColumn?: number;
    readonly requestId: number;
    readonly documentUri?: string;
  } | null;
}

export default function CodeEditor({
  value,
  theme,
  documentId,
  documentPath,
  documentUri,
  documentVersion,
  viewState,
  onChange,
  onViewStateChange,
  onRun,
  onSave,
  lspUrl,
  workspaceDocuments,
  editorSession,
  onWorkspaceDocumentChange,
  onOpenDocument,
  reveal,
}: CodeEditorProps) {
  const onRunRef = useRef(onRun);
  const onSaveRef = useRef(onSave);
  const onViewStateChangeRef = useRef(onViewStateChange);
  const viewStateRef = useRef(viewState);
  const onChangeRef = useRef(onChange);
  const onWorkspaceDocumentChangeRef = useRef(onWorkspaceDocumentChange);
  const onOpenDocumentRef = useRef(onOpenDocument);
  const modelsRef = useRef<WorkspaceEditorModels | null>(null);
  const workspaceRef = useRef<OpenMatMonacoLspWorkspace | null>(null);
  const documents = useMemo<readonly WorkspaceEditorDocument[]>(
    () => workspaceDocuments ?? [{
      id: documentId, uri: documentUri, content: value, version: documentVersion,
    }],
    [workspaceDocuments, documentId, documentUri, value, documentVersion],
  );
  const documentsRef = useRef(documents);
  const [mounted, setMounted] = useState<{
    readonly editor: monaco.editor.IStandaloneCodeEditor;
    readonly monacoApi: typeof monaco;
  } | null>(null);
  const resolvedLspUrl = useMemo(
    () =>
      lspUrl === null
        ? undefined
        : resolveLspWebSocketUrl(
            lspUrl ?? import.meta.env.VITE_OPENMAT_LSP_URL,
            kernelWebSocketUrl(),
          ),
    [lspUrl],
  );

  useEffect(() => {
    onRunRef.current = onRun;
  }, [onRun]);

  useEffect(() => {
    onSaveRef.current = onSave;
  }, [onSave]);

  onViewStateChangeRef.current = onViewStateChange;
  viewStateRef.current = viewState;
  onChangeRef.current = onChange;
  onWorkspaceDocumentChangeRef.current = onWorkspaceDocumentChange;
  onOpenDocumentRef.current = onOpenDocument;
  documentsRef.current = documents;

  useEffect(() => {
    if (mounted === null) return;
    if (editorSession !== undefined) {
      editorSession.attach(mounted.monacoApi);
      return;
    }
    const models = new WorkspaceEditorModels(mounted.monacoApi, (id, content) => {
      if (onWorkspaceDocumentChangeRef.current !== undefined) {
        onWorkspaceDocumentChangeRef.current(id, content);
      } else {
        onChangeRef.current(content);
      }
    });
    models.sync(documentsRef.current);
    modelsRef.current = models;
    return () => {
      modelsRef.current = null;
      models.dispose();
    };
  }, [mounted, editorSession]);

  useEffect(() => {
    if (mounted === null || resolvedLspUrl === undefined || editorSession !== undefined) {
      return undefined;
    }
    const workspace = new OpenMatMonacoLspWorkspace(mounted.monacoApi, {
      url: resolvedLspUrl,
    }).start();
    workspace.syncModels(modelsRef.current?.models ?? []);
    workspaceRef.current = workspace;
    return () => {
      workspaceRef.current = null;
      workspace.dispose();
    };
  }, [mounted, resolvedLspUrl, editorSession]);

  useEffect(() => {
    const models = modelsRef.current;
    if (models === null) return;
    models.sync(documents);
    workspaceRef.current?.syncModels(models.models);
  }, [documents, mounted, editorSession]);

  useEffect(() => {
    if (mounted === null) return;
    const opener = mounted.monacoApi.editor.registerEditorOpener({
      openCodeEditor: (source, resource, selectionOrPosition) => {
        if (source !== mounted.editor) return false;
        const selection = selectionOrPosition === undefined ? undefined :
          "startLineNumber" in selectionOrPosition ? selectionOrPosition : {
            startLineNumber: selectionOrPosition.lineNumber,
            startColumn: selectionOrPosition.column,
            endLineNumber: selectionOrPosition.lineNumber,
            endColumn: selectionOrPosition.column,
          };
        return onOpenDocumentRef.current?.(resource.toString(), selection) ?? false;
      },
    });
    return () => opener.dispose();
  }, [mounted]);

  useEffect(() => {
    if (mounted === null) {
      return undefined;
    }
    const editor = mounted.editor;
    const restored = viewStateRef.current;
    if (restored !== null) {
      editor.setPosition({
        lineNumber: restored.lineNumber,
        column: restored.column,
      });
      editor.setScrollPosition({
        scrollTop: restored.scrollTop,
        scrollLeft: restored.scrollLeft,
      });
    }
    let publishTimer: number | null = null;
    const capture = (): void => {
      const position = editor.getPosition();
      if (position === null) {
        return;
      }
      onViewStateChangeRef.current(documentId, {
        lineNumber: position.lineNumber,
        column: position.column,
        scrollTop: editor.getScrollTop(),
        scrollLeft: editor.getScrollLeft(),
      });
    };
    const publishSoon = (): void => {
      if (publishTimer !== null) {
        window.clearTimeout(publishTimer);
      }
      publishTimer = window.setTimeout(() => {
        publishTimer = null;
        capture();
      }, 200);
    };
    const cursorSubscription = editor.onDidChangeCursorPosition(publishSoon);
    const scrollSubscription = editor.onDidScrollChange(publishSoon);
    window.addEventListener(DESKTOP_CLOSE_PREPARE_EVENT, capture);
    return () => {
      window.removeEventListener(DESKTOP_CLOSE_PREPARE_EVENT, capture);
      if (publishTimer !== null) {
        window.clearTimeout(publishTimer);
      }
      cursorSubscription.dispose();
      scrollSubscription.dispose();
      capture();
    };
  }, [documentId, mounted]);

  useEffect(() => {
    if (mounted === null || !reveal ||
      (reveal.documentUri !== undefined && reveal.documentUri !== documentUri)) return;
    const lineNumber = Math.max(1, Math.min(
      reveal.lineNumber,
      mounted.editor.getModel()?.getLineCount() ?? 1,
    ));
    const column = reveal.column ?? 1;
    mounted.editor.setPosition({ lineNumber, column });
    if (reveal.endLineNumber !== undefined && reveal.endColumn !== undefined) {
      mounted.editor.setSelection({
        startLineNumber: lineNumber, startColumn: column,
        endLineNumber: reveal.endLineNumber, endColumn: reveal.endColumn,
      });
    }
    mounted.editor.revealLineInCenter(lineNumber);
    mounted.editor.focus();
  }, [documentUri, mounted, reveal]);

  const handleMount: OnMount = (editor, monacoApi) => {
    const model = editor.getModel();
    if (model !== null && model.getLanguageId() !== "openmat") {
      // Models are cached by URI by @monaco-editor/react. A model first created
      // before the custom language was registered can otherwise remain
      // plaintext, which prevents every OpenMat language provider from running.
      monacoApi.editor.setModelLanguage(model, "openmat");
    }
    editor.addCommand(
      monacoApi.KeyMod.CtrlCmd | monacoApi.KeyCode.Enter,
      () => onRunRef.current(),
    );
    editor.addCommand(
      monacoApi.KeyMod.CtrlCmd | monacoApi.KeyCode.KeyS,
      () => onSaveRef.current(),
    );
    setMounted({ editor, monacoApi });
    editor.focus();
  };

  return (
    <div
      className="code-editor-document"
      data-document-path={documentPath}
      data-document-uri={documentUri}
      data-document-version={documentVersion}
    >
      <Editor
        height="100%"
        language="openmat"
        path={documentUri}
        theme={monacoTheme(theme)}
        defaultValue={value}
        keepCurrentModel
        beforeMount={configureOpenMat}
        onMount={handleMount}
        loading={<div className="editor-loading">Loading Monaco editor…</div>}
        options={{
          accessibilitySupport: "auto",
          automaticLayout: true,
          fontFamily: "'Cascadia Code', 'SFMono-Regular', Consolas, monospace",
          fontSize: 14,
          minimap: { enabled: false },
          padding: { top: 16 },
          quickSuggestions: {
            other: true,
            comments: false,
            strings: false,
          },
          renderLineHighlight: "all",
          scrollBeyondLastLine: false,
          suggestOnTriggerCharacters: true,
          tabSize: 4,
        }}
      />
    </div>
  );
}
