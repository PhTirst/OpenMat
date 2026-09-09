import { useEffect, useState, type FormEvent } from "react";
import {
  BackIcon,
  ForwardIcon,
  OpenFolderIcon,
  ParentFolderIcon,
} from "./Icons";

interface CurrentFolderAddressBarProps {
  readonly path: string;
  readonly busy: boolean;
  readonly disabled: boolean;
  readonly onNavigate: (path: string) => Promise<string | null>;
  readonly onChoose: () => void;
  readonly onParent: () => void;
}

interface FolderHistory {
  readonly entries: readonly string[];
  readonly index: number;
}

export function CurrentFolderAddressBar({
  path,
  busy,
  disabled,
  onNavigate,
  onChoose,
  onParent,
}: CurrentFolderAddressBarProps) {
  const [draft, setDraft] = useState(path);
  const [history, setHistory] = useState<FolderHistory>(() => ({
    entries: path.length === 0 ? [] : [path],
    index: path.length === 0 ? -1 : 0,
  }));

  useEffect(() => {
    setDraft(path);
    if (path.length === 0) {
      setHistory({ entries: [], index: -1 });
      return;
    }
    setHistory((current) => {
      if (current.entries[current.index] === path) {
        return current;
      }
      const entries = [...current.entries.slice(0, current.index + 1), path];
      return { entries, index: entries.length - 1 };
    });
  }, [path]);

  const submit = (event: FormEvent): void => {
    event.preventDefault();
    const requested = draft.trim();
    if (!busy && !disabled && requested.length > 0 && requested !== path) {
      void onNavigate(requested).then((resolvedPath) => {
        setDraft(resolvedPath ?? path);
      });
    }
  };

  const navigateHistory = (index: number): void => {
    const target = history.entries[index];
    if (target === undefined || busy || disabled) {
      return;
    }
    const previousIndex = history.index;
    setHistory((current) => ({ ...current, index }));
    setDraft(target);
    void onNavigate(target).then((resolvedPath) => {
      if (resolvedPath === null) {
        setHistory((current) =>
          current.index === index ? { ...current, index: previousIndex } : current,
        );
        setDraft(path);
      } else {
        setDraft(resolvedPath);
      }
    });
  };

  return (
    <div className="topbar-context current-folder-address" aria-label="Current Folder">
      <span className="muted-label">Current Folder</span>
      <div className="current-folder-navigation" aria-label="Folder navigation">
        <button
          className="icon-button"
          type="button"
          title="Back"
          aria-label="Back to previous folder"
          disabled={disabled || busy || history.index <= 0}
          onClick={() => navigateHistory(history.index - 1)}
        >
          <BackIcon />
        </button>
        <button
          className="icon-button"
          type="button"
          title="Forward"
          aria-label="Forward to next folder"
          disabled={
            disabled || busy || history.index + 1 >= history.entries.length
          }
          onClick={() => navigateHistory(history.index + 1)}
        >
          <ForwardIcon />
        </button>
        <button
          className="icon-button"
          type="button"
          title="Up One Level"
          aria-label="Open parent folder"
          disabled={disabled || busy}
          onClick={onParent}
        >
          <ParentFolderIcon />
        </button>
      </div>
      <form className="current-folder-address-form" onSubmit={submit}>
        <input
          value={draft}
          title={path}
          aria-label="Current Folder path"
          disabled={disabled || busy}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setDraft(path);
              event.currentTarget.blur();
            }
          }}
        />
      </form>
      <button
        className="icon-button"
        type="button"
        title="Choose Folder"
        aria-label="Choose Current Folder"
        disabled={disabled || busy}
        onClick={onChoose}
      >
        <OpenFolderIcon />
      </button>
    </div>
  );
}
