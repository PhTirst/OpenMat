import { useEffect, useId, useRef, useState } from "react";
import type { DirectoryBrowserSnapshot } from "../workspace/workspace-client";
import { FolderIcon, ParentFolderIcon } from "./Icons";

interface DirectoryPickerDialogProps {
  readonly initialPath: string;
  readonly snapshot: DirectoryBrowserSnapshot | null;
  readonly loading: boolean;
  readonly selecting: boolean;
  readonly error: string | null;
  readonly onBrowse: (path: string) => void;
  readonly onConfirm: () => void;
  readonly onCancel: () => void;
}

export function DirectoryPickerDialog({
  initialPath,
  snapshot,
  loading,
  selecting,
  error,
  onBrowse,
  onConfirm,
  onCancel,
}: DirectoryPickerDialogProps) {
  const titleId = useId();
  const descriptionId = useId();
  const pathInputRef = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState(snapshot?.path ?? initialPath);
  const busy = loading || selecting;

  useEffect(() => {
    pathInputRef.current?.focus();
    pathInputRef.current?.select();
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key === "Escape" && !selecting) {
        event.preventDefault();
        onCancel();
      }
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onCancel, selecting]);

  useEffect(() => {
    if (snapshot !== null) {
      setDraft(snapshot.path);
    }
  }, [snapshot]);

  return (
    <div
      className="directory-picker-backdrop"
      onMouseDown={(event) => {
        if (event.currentTarget === event.target && !selecting) {
          onCancel();
        }
      }}
    >
      <section
        className="directory-picker-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
      >
        <header>
          <div>
            <strong id={titleId}>Choose Current Folder</strong>
            <span id={descriptionId}>
              Browse folders on this computer. The Current Folder changes only after
              you confirm.
            </span>
          </div>
          <button
            className="icon-button"
            type="button"
            aria-label="Close folder chooser"
            title="Close"
            disabled={selecting}
            onClick={onCancel}
          >
            ×
          </button>
        </header>

        <form
          className="directory-picker-address"
          onSubmit={(event) => {
            event.preventDefault();
            const requested = draft.trim();
            if (requested.length > 0 && !busy) {
              onBrowse(requested);
            }
          }}
        >
          <button
            className="icon-button"
            type="button"
            aria-label="Browse parent folder"
            title="Parent Folder"
            disabled={busy || snapshot?.parentPath == null}
            onClick={() => {
              if (snapshot?.parentPath != null) {
                onBrowse(snapshot.parentPath);
              }
            }}
          >
            <ParentFolderIcon />
          </button>
          <input
            ref={pathInputRef}
            aria-label="Folder chooser path"
            value={draft}
            disabled={busy}
            spellCheck={false}
            onChange={(event) => setDraft(event.target.value)}
          />
          <button
            className="button button-secondary"
            type="submit"
            disabled={busy || draft.trim().length === 0}
          >
            Go
          </button>
        </form>

        <div className="directory-picker-content">
          <nav className="directory-picker-roots" aria-label="Filesystem roots">
            <span>Locations</span>
            {snapshot?.roots.map((root) => (
              <button
                key={root.path}
                className={root.path === snapshot.path ? "active" : undefined}
                type="button"
                disabled={busy}
                aria-current={root.path === snapshot.path ? "location" : undefined}
                onClick={() => onBrowse(root.path)}
              >
                <FolderIcon />
                <span>{root.name}</span>
              </button>
            ))}
          </nav>

          <div className="directory-picker-browser">
            <div className="directory-picker-browser-heading">
              <span>{snapshot?.path ?? initialPath}</span>
              {loading ? <span role="status">Loading…</span> : null}
            </div>
            {error === null ? null : (
              <div className="directory-picker-error" role="alert">
                {error}
              </div>
            )}
            <div className="directory-picker-list" role="list">
              {snapshot !== null && snapshot.entries.length === 0 && !loading ? (
                <div className="directory-picker-empty">This folder has no subfolders.</div>
              ) : null}
              {snapshot?.entries.map((entry) => (
                <div key={entry.path} role="listitem">
                  <button
                    type="button"
                    disabled={busy}
                    aria-label={`Open folder ${entry.name}`}
                    onClick={() => onBrowse(entry.path)}
                  >
                    <FolderIcon />
                    <span>{entry.name}</span>
                  </button>
                </div>
              ))}
            </div>
          </div>
        </div>

        <footer>
          <span className="directory-picker-selection" title={snapshot?.path ?? initialPath}>
            {snapshot?.path ?? initialPath}
          </span>
          <div>
            <button
              className="button button-secondary"
              type="button"
              disabled={selecting}
              onClick={onCancel}
            >
              Cancel
            </button>
            <button
              className="button button-primary"
              type="button"
              disabled={busy || snapshot === null}
              onClick={onConfirm}
            >
              {selecting ? "Selecting…" : "Select Folder"}
            </button>
          </div>
        </footer>
      </section>
    </div>
  );
}
