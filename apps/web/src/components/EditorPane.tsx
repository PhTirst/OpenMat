import { lazy, Suspense } from "react";
import type {
  DocumentRecoveryStatus,
  DocumentViewState,
  OpenDocument,
} from "../documents/document-session";
import type { IdeTheme } from "../theme";
import { usePlatformServices } from "../platform/platform-services";
import type { CodeEditorProps } from "./CodeEditor";

const CodeEditor = lazy(() => import("./CodeEditor"));

interface EditorPaneProps {
  readonly activeDocumentId: string | null;
  readonly documentPath: string | null;
  readonly documentUri: string | null;
  readonly documentVersion: number;
  readonly documentViewState: DocumentViewState | null;
  readonly code: string;
  readonly loading: boolean;
  readonly saving: boolean;
  readonly error: string | null;
  readonly recoveryStatus: DocumentRecoveryStatus;
  readonly recoveryMessage: string | null;
  readonly canRun: boolean;
  readonly canSave: boolean;
  readonly canCancel: boolean;
  readonly theme: IdeTheme;
  readonly onCodeChange: (code: string) => void;
  readonly onViewStateChange: (
    documentId: string,
    viewState: DocumentViewState,
  ) => void;
  readonly onRun: () => void;
  readonly onSave: () => void;
  readonly onSaveAs?: (() => void) | undefined;
  readonly canSaveAs?: boolean;
  readonly onCancel: () => void;
  readonly onResolveRecoveryConflict: () => void;
  readonly lspUrl: string | null;
  readonly editorSession?: CodeEditorProps["editorSession"];
  readonly pendingPath?: string | null;
  readonly workspaceDocuments: readonly OpenDocument[];
  readonly onWorkspaceDocumentChange: NonNullable<
    CodeEditorProps["onWorkspaceDocumentChange"]
  >;
  readonly onOpenDocument: NonNullable<CodeEditorProps["onOpenDocument"]>;
  readonly reveal: NonNullable<CodeEditorProps["reveal"]> | null;
}

export function EditorPane({
  activeDocumentId,
  documentPath,
  documentUri,
  documentVersion,
  documentViewState,
  code,
  loading,
  saving,
  error,
  recoveryStatus,
  recoveryMessage,
  canRun,
  canSave,
  canCancel,
  theme,
  onCodeChange,
  onViewStateChange,
  onRun,
  onSave,
  onSaveAs,
  canSaveAs = false,
  onCancel,
  onResolveRecoveryConflict,
  lspUrl,
  editorSession,
  pendingPath,
  workspaceDocuments,
  onWorkspaceDocumentChange,
  onOpenDocument,
  reveal,
}: EditorPaneProps) {
  const desktop = usePlatformServices().kind === "desktop";
  const hasRecoveryNotice = recoveryStatus !== "none" && recoveryMessage !== null;
  return (
    <section className="pane editor-pane" aria-labelledby="editor-title">
      <header className="pane-header editor-header">
        <div className="editor-title-area">
          <span id="editor-title" className="pane-title">
            Editor
          </span>
        </div>
        <div className="editor-actions">
          {error === null ? null : (
            <span className="editor-operation-error" role="alert" title={error}>
              {error}
            </span>
          )}
          <span className="shortcut-hint">
            Save: Ctrl/⌘ + S · Run: {desktop ? "F5" : "Ctrl/⌘ + Enter"}
          </span>
          <button
            className="button button-secondary"
            type="button"
            onClick={onSave}
            disabled={!canSave || saving}
          >
            {saving ? "Saving…" : "Save"}
          </button>
          {onSaveAs ? (
            <button className="button button-secondary" type="button" onClick={onSaveAs} disabled={!canSaveAs || saving}>
              Save As…
            </button>
          ) : null}
          <button
            className="button button-secondary"
            type="button"
            onClick={onCancel}
            disabled={!canCancel}
          >
            Stop
          </button>
          <button
            className="button button-primary"
            type="button"
            title={desktop
              ? "Run current script (F5, or Ctrl/⌘ + Enter in the editor)"
              : "Run current script (Ctrl/⌘ + Enter in the editor)"}
            aria-keyshortcuts={desktop ? "F5 Control+Enter Meta+Enter" : "Control+Enter Meta+Enter"}
            onClick={onRun}
            disabled={!canRun}
          >
            <span aria-hidden="true">▶</span> Run
          </button>
        </div>
      </header>
      <div
        id="editor-document-panel"
        className={`editor-host${hasRecoveryNotice ? " has-recovery-notice" : ""}`}
        role="tabpanel"
        aria-label={
          documentPath === null ? "Source editor" : `Editor for ${documentPath}`
        }
      >
        {pendingPath ? (
          <div className="document-recovery-notice" role="status">
            Saving will rename this source file to {pendingPath}.
          </div>
        ) : null}
        {hasRecoveryNotice ? (
          <div className={`document-recovery-notice ${recoveryStatus}`} role="alert">
            <span>{recoveryMessage}</span>
            {recoveryStatus === "conflict" ? (
              <span className="document-recovery-actions">
                <button
                  className="button button-secondary"
                  type="button"
                  onClick={onResolveRecoveryConflict}
                >
                  Resolve Conflict…
                </button>
              </span>
            ) : null}
          </div>
        ) : null}
        <div className="editor-document-surface">
          {loading && activeDocumentId === null ? (
            <div className="editor-loading" role="status">
              Opening workspace file…
            </div>
          ) : documentPath === null ||
            documentUri === null ||
            activeDocumentId === null ? (
            <div className="editor-empty-state">
              Select a workspace file to open it in the editor.
            </div>
          ) : (
            <Suspense
              fallback={<div className="editor-loading">Loading Monaco editor…</div>}
            >
              <CodeEditor
                value={code}
                theme={theme}
                documentId={activeDocumentId}
                documentPath={documentPath}
                documentUri={documentUri}
                documentVersion={documentVersion}
                viewState={documentViewState}
                onChange={onCodeChange}
                onViewStateChange={onViewStateChange}
                onRun={onRun}
                onSave={onSave}
                lspUrl={lspUrl}
                {...(editorSession === undefined ? {} : { editorSession })}
                workspaceDocuments={workspaceDocuments}
                onWorkspaceDocumentChange={onWorkspaceDocumentChange}
                onOpenDocument={onOpenDocument}
                reveal={reveal}
              />
            </Suspense>
          )}
        </div>
      </div>
    </section>
  );
}
