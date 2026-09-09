import { useRef, type KeyboardEvent } from "react";
import type { DocumentRecoveryStatus } from "../documents/document-session";

export interface OpenEditorItem {
  readonly id: string;
  readonly path: string;
  readonly dirty: boolean;
  readonly saving: boolean;
  readonly recoveryStatus: DocumentRecoveryStatus;
}

interface OpenEditorsListProps {
  readonly editors: readonly OpenEditorItem[];
  readonly activeDocumentId: string | null;
  readonly onActivate: (documentId: string) => void;
  readonly onClose: (documentId: string) => void;
}

export function OpenEditorsList({
  editors,
  activeDocumentId,
  onActivate,
  onClose,
}: OpenEditorsListProps) {
  const listRef = useRef<HTMLDivElement>(null);

  const navigate = (event: KeyboardEvent<HTMLButtonElement>): void => {
    const currentIndex = editors.findIndex(
      (editor) => editor.id === event.currentTarget.dataset.documentId,
    );
    if (currentIndex < 0) {
      return;
    }
    let nextIndex: number | null = null;
    if (event.key === "ArrowUp") {
      nextIndex = (currentIndex - 1 + editors.length) % editors.length;
    } else if (event.key === "ArrowDown") {
      nextIndex = (currentIndex + 1) % editors.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = editors.length - 1;
    }
    if (nextIndex === null) {
      return;
    }
    event.preventDefault();
    const next = editors[nextIndex];
    if (next === undefined) {
      return;
    }
    onActivate(next.id);
    listRef.current
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      .item(nextIndex)
      .focus();
  };

  return (
    <section className="open-editors-section" aria-labelledby="open-editors-title">
      <header className="open-editors-header">
        <span id="open-editors-title">Open Editors</span>
        <span
          className="open-editors-count"
          aria-label={`${editors.length.toString()} open files`}
        >
          {editors.length.toString()}
        </span>
      </header>
      {editors.length === 0 ? (
        <div className="open-editors-empty">No open files</div>
      ) : null}
      <div
        ref={listRef}
        className="open-editors-list"
        hidden={editors.length === 0}
        role="tablist"
        aria-label="Open files"
        aria-orientation="vertical"
      >
        {editors.map((editor, index) => {
          const active = editor.id === activeDocumentId;
          return (
            <div
              className={`open-editor-row${active ? " active" : ""}`}
              role="presentation"
              key={editor.id}
            >
              <button
                id={`open-editor-tab-${index.toString()}`}
                className="open-editor-select"
                type="button"
                role="tab"
                aria-controls="editor-document-panel"
                aria-selected={active}
                data-document-id={editor.id}
                tabIndex={active ? 0 : -1}
                title={editor.path}
                onClick={() => onActivate(editor.id)}
                onKeyDown={navigate}
              >
                <span className="open-editor-label">{fileName(editor.path)}</span>
                {editor.dirty ? <span aria-label="Unsaved changes">●</span> : null}
                {editor.recoveryStatus === "none" ? null : (
                  <span className="open-editor-recovery" aria-label="Recovered draft">
                    R
                  </span>
                )}
              </button>
              <button
                className="open-editor-close"
                type="button"
                aria-label={`Close ${editor.path}`}
                disabled={editor.saving}
                onClick={() => onClose(editor.id)}
              >
                ×
              </button>
            </div>
          );
        })}
      </div>
    </section>
  );
}

function fileName(path: string): string {
  const separator = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return separator < 0 ? path : path.slice(separator + 1);
}
