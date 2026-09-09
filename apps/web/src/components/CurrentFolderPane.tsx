import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import type {
  CreatableWorkspaceEntryKind,
  WorkspaceEntry,
} from "../workspace/workspace-client";
import { workspaceParentPath } from "../workspace/workspace-client";
import {
  FileIcon,
  OpenFileIcon,
  FileExplorerIcon,
  FolderIcon,
  NewFileIcon,
  NewFolderIcon,
  OtherEntryIcon,
  RefreshIcon,
  UploadIcon,
} from "./Icons";
import { OpenEditorsList, type OpenEditorItem } from "./OpenEditorsList";
import "./workspace-tree.css";

interface CurrentFolderPaneProps {
  readonly openEditors: readonly OpenEditorItem[];
  readonly activeDocumentId: string | null;
  readonly entries: readonly WorkspaceEntry[];
  readonly rootName: string;
  readonly rootPath: string;
  readonly serviceLabel: string;
  readonly selectedPath: string | null;
  readonly expandedPaths: ReadonlySet<string>;
  readonly searchPathWorkspacePaths?: ReadonlySet<string>;
  readonly loading: boolean;
  readonly refreshing: boolean;
  readonly canMutate: boolean;
  readonly creatingKind: CreatableWorkspaceEntryKind | null;
  readonly creatingParentPath: string;
  readonly operationBusy: boolean;
  readonly uploadStatus: string | null;
  readonly error: { readonly code: string; readonly message: string } | null;
  readonly onActivateDocument: (documentId: string) => void;
  readonly onCloseDocument: (documentId: string) => void;
  readonly onToggleDirectory: (path: string) => void;
  readonly onRefresh: () => void;
  readonly onStartCreate: (
    kind: CreatableWorkspaceEntryKind,
    parentPath: string,
  ) => void;
  readonly onCancelCreate: () => void;
  readonly onCreate: (name: string) => void;
  readonly onFocus: (entry: WorkspaceEntry) => void;
  readonly onSelect: (entry: WorkspaceEntry) => void;
  readonly onEnterDirectory: (path: string) => void;
  readonly onAddSearchPath?: (entry: WorkspaceEntry, recursive: boolean) => void;
  readonly onRemoveSearchPath?: (entry: WorkspaceEntry, recursive: boolean) => void;
  readonly fileTransfers?: boolean;
  readonly onOpenFile?: (() => void) | undefined;
  readonly onRevealPath?: ((path: string) => void) | undefined;
  readonly onDownload: (entry: WorkspaceEntry) => void;
  readonly onUpload: (files: readonly File[], parentPath: string) => void;
  readonly onCopyRelativePath: (entry: WorkspaceEntry) => Promise<void>;
  readonly onRename: (entry: WorkspaceEntry) => void;
  readonly onMove: (entry: WorkspaceEntry) => void;
  readonly onDelete: (entry: WorkspaceEntry) => void;
}

interface ContextMenuState {
  readonly entry: WorkspaceEntry;
  readonly x: number;
  readonly y: number;
}

interface ViewportRect {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

export function fitContextMenuToViewport(
  x: number,
  y: number,
  menuWidth: number,
  menuHeight: number,
  viewport: ViewportRect,
  margin = 8,
): { readonly x: number; readonly y: number } {
  const right = viewport.left + viewport.width;
  const bottom = viewport.top + viewport.height;
  const preferredX = x + menuWidth > right - margin ? x - menuWidth : x;
  const preferredY = y + menuHeight > bottom - margin ? y - menuHeight : y;
  return {
    x: Math.min(
      Math.max(preferredX, viewport.left + margin),
      Math.max(viewport.left + margin, right - menuWidth - margin),
    ),
    y: Math.min(
      Math.max(preferredY, viewport.top + margin),
      Math.max(viewport.top + margin, bottom - menuHeight - margin),
    ),
  };
}

function containsFiles(event: DragEvent<HTMLElement>): boolean {
  return [...event.dataTransfer.types].includes("Files");
}

const EMPTY_SEARCH_PATHS: ReadonlySet<string> = new Set();
const NOOP_SEARCH_PATH_ACTION = (): void => {};

function EntryIcon({ kind }: { readonly kind: WorkspaceEntry["kind"] }) {
  switch (kind) {
    case "file":
      return <FileIcon />;
    case "directory":
      return <FolderIcon />;
    case "other":
      return <OtherEntryIcon />;
  }
}

export function CurrentFolderPane({
  openEditors,
  activeDocumentId,
  entries,
  rootName,
  rootPath,
  serviceLabel,
  selectedPath,
  expandedPaths,
  searchPathWorkspacePaths = EMPTY_SEARCH_PATHS,
  loading,
  refreshing,
  canMutate,
  creatingKind,
  creatingParentPath,
  operationBusy,
  uploadStatus,
  error,
  onActivateDocument,
  onCloseDocument,
  onToggleDirectory,
  onRefresh,
  onStartCreate,
  onCancelCreate,
  onCreate,
  onFocus,
  onSelect,
  onEnterDirectory,
  onAddSearchPath = NOOP_SEARCH_PATH_ACTION,
  onRemoveSearchPath = NOOP_SEARCH_PATH_ACTION,
  fileTransfers = true,
  onOpenFile,
  onRevealPath,
  onDownload,
  onUpload,
  onCopyRelativePath,
  onRename,
  onMove,
  onDelete,
}: CurrentFolderPaneProps) {
  const [name, setName] = useState("");
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [dropTargetPath, setDropTargetPath] = useState<string | undefined>();
  const [announcement, setAnnouncement] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const uploadInputRef = useRef<HTMLInputElement>(null);
  const uploadParentRef = useRef("");
  const treeRef = useRef<HTMLUListElement>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const entriesByPath = useMemo(
    () => new Map(entries.map((entry) => [entry.path, entry])),
    [entries],
  );
  const childrenByParent = useMemo(() => {
    const children = new Map<string, WorkspaceEntry[]>();
    for (const entry of entries) {
      const parent = workspaceParentPath(entry.path);
      const bucket = children.get(parent) ?? [];
      bucket.push(entry);
      children.set(parent, bucket);
    }
    return children;
  }, [entries]);

  useEffect(() => {
    if (creatingKind !== null) {
      setName("");
      inputRef.current?.focus();
    }
  }, [creatingKind]);

  useEffect(() => {
    if (contextMenu === null) {
      return;
    }
    contextMenuRef.current
      ?.querySelector<HTMLButtonElement>('button:not(:disabled)')
      ?.focus();
    const close = (): void => setContextMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("blur", close);
    };
  }, [contextMenu]);

  useLayoutEffect(() => {
    if (contextMenu === null || contextMenuRef.current === null) {
      return;
    }
    const bounds = contextMenuRef.current.getBoundingClientRect();
    const visualViewport = window.visualViewport;
    const viewport = visualViewport
      ? {
          left: visualViewport.offsetLeft,
          top: visualViewport.offsetTop,
          width: visualViewport.width,
          height: visualViewport.height,
        }
      : { left: 0, top: 0, width: window.innerWidth, height: window.innerHeight };
    const fitted = fitContextMenuToViewport(
      contextMenu.x,
      contextMenu.y,
      bounds.width,
      bounds.height,
      viewport,
    );
    if (fitted.x !== contextMenu.x || fitted.y !== contextMenu.y) {
      setContextMenu((current) =>
        current === null ? null : { ...current, x: fitted.x, y: fitted.y },
      );
    }
  }, [contextMenu]);

  useEffect(() => {
    if (announcement === null) {
      return;
    }
    const timeout = window.setTimeout(() => setAnnouncement(null), 2500);
    return () => window.clearTimeout(timeout);
  }, [announcement]);

  const selectedEntry =
    selectedPath === null ? undefined : entriesByPath.get(selectedPath);
  const defaultCreateParent =
    selectedEntry?.kind === "directory"
      ? selectedEntry.path
      : selectedEntry === undefined
        ? ""
        : workspaceParentPath(selectedEntry.path);

  const submit = (event: FormEvent): void => {
    event.preventDefault();
    if (!operationBusy && name.trim().length > 0) {
      onCreate(name);
    }
  };

  const focusRelative = (current: HTMLButtonElement, offset: number): void => {
    const items = [...(treeRef.current?.querySelectorAll<HTMLButtonElement>(
      '[role="treeitem"]',
    ) ?? [])];
    const index = items.indexOf(current);
    items[index + offset]?.focus();
  };

  const handleTreeKey = (
    event: KeyboardEvent<HTMLButtonElement>,
    entry: WorkspaceEntry,
  ): void => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      focusRelative(event.currentTarget, event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "ArrowRight" && entry.kind === "directory") {
      event.preventDefault();
      if (!expandedPaths.has(entry.path)) {
        onToggleDirectory(entry.path);
      }
    } else if (event.key === "ArrowLeft" && entry.kind === "directory") {
      event.preventDefault();
      if (expandedPaths.has(entry.path)) {
        onToggleDirectory(entry.path);
      }
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(entry);
    } else if (event.key === "F2" && canMutate) {
      event.preventDefault();
      onRename(entry);
    } else if (event.key === "Delete" && canMutate) {
      event.preventDefault();
      onDelete(entry);
    } else if (event.key === "F10" && event.shiftKey) {
      event.preventDefault();
      const bounds = event.currentTarget.getBoundingClientRect();
      setContextMenu({ entry, x: bounds.left + 24, y: bounds.bottom });
    }
  };

  const renderChildren = (parentPath: string, level: number) =>
    (childrenByParent.get(parentPath) ?? []).map((entry) => {
      const selected = entry.path === selectedPath;
      const directory = entry.kind === "directory";
      const expanded = directory && expandedPaths.has(entry.path);
      const onSearchPath = directory && searchPathWorkspacePaths.has(entry.path);
      return (
        <li key={entry.path} role="none">
          <button
            className={[
              "file-item",
              selected ? "active" : "",
              directory && dropTargetPath === entry.path ? "is-drop-target" : "",
            ]
              .filter(Boolean)
              .join(" ")}
            type="button"
            role="treeitem"
            aria-level={level}
            aria-selected={selected}
            aria-expanded={directory ? expanded : undefined}
            data-tree-path={entry.path}
            title={`${entry.path} · ${entry.kind}${onSearchPath ? " · On MATLAB path" : ""}`}
            style={{ paddingInlineStart: `${(level - 1) * 16 + 7}px` }}
            onClick={(event) => {
              // A double click expands or collapses a folder only once.
              if (directory && event.detail > 1) return;
              onSelect(entry);
            }}
            onKeyDown={(event) => handleTreeKey(event, entry)}
            onContextMenu={(event) => {
              event.preventDefault();
              onFocus(entry);
              setContextMenu({ entry, x: event.clientX, y: event.clientY });
            }}
            onDragEnter={(event) => {
              if (fileTransfers && containsFiles(event) && canMutate && !operationBusy) {
                event.preventDefault();
                event.stopPropagation();
                setDropTargetPath(
                  entry.kind === "directory"
                    ? entry.path
                    : workspaceParentPath(entry.path),
                );
              }
            }}
            onDragOver={(event) => {
              if (fileTransfers && containsFiles(event) && canMutate && !operationBusy) {
                event.preventDefault();
                event.stopPropagation();
                event.dataTransfer.dropEffect = "copy";
              }
            }}
            onDrop={(event) =>
              uploadDroppedFiles(
                event,
                entry.kind === "directory"
                  ? entry.path
                  : workspaceParentPath(entry.path),
              )
            }
          >
            <span className="tree-disclosure" aria-hidden="true">
              {directory ? (expanded ? "▾" : "▸") : ""}
            </span>
            <span className={`file-icon ${entry.kind}`} aria-hidden="true">
              <EntryIcon kind={entry.kind} />
            </span>
            <span className="tree-entry-name">{entry.name}</span>
            {onSearchPath ? (
              <span className="search-path-badge" title="On MATLAB path">
                PATH
              </span>
            ) : null}
          </button>
          {expanded ? (
            <ul role="group">{renderChildren(entry.path, level + 1)}</ul>
          ) : null}
        </li>
      );
    });

  const openCreateFromEntry = (
    kind: CreatableWorkspaceEntryKind,
    entry: WorkspaceEntry,
  ): void => {
    onStartCreate(
      kind,
      entry.kind === "directory" ? entry.path : workspaceParentPath(entry.path),
    );
    setContextMenu(null);
  };

  const closeContextMenuAndRestoreFocus = (): void => {
    const path = contextMenu?.entry.path;
    setContextMenu(null);
    if (path !== undefined) {
      [...(treeRef.current?.querySelectorAll<HTMLButtonElement>(
        '[role="treeitem"]',
      ) ?? [])]
        .find((item) => item.dataset.treePath === path)
        ?.focus();
    }
  };

  const requestUpload = (parentPath: string): void => {
    if (!fileTransfers) return;
    uploadParentRef.current = parentPath;
    uploadInputRef.current?.click();
  };

  const uploadDroppedFiles = (
    event: DragEvent<HTMLElement>,
    parentPath: string,
  ): void => {
    if (!fileTransfers && containsFiles(event)) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    if (!containsFiles(event) || !canMutate || operationBusy) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    setDropTargetPath(undefined);
    const files = [...event.dataTransfer.files];
    if (files.length > 0) {
      onUpload(files, parentPath);
    }
  };

  return (
    <aside
      className={
        dropTargetPath === undefined
          ? "pane current-folder-pane"
          : "pane current-folder-pane is-file-dragging"
      }
      aria-label="Workspace explorer"
      onDragOver={(event) => {
        if (!fileTransfers && containsFiles(event)) {
          event.preventDefault();
          event.dataTransfer.dropEffect = "none";
          return;
        }
        if (fileTransfers && containsFiles(event) && canMutate && !operationBusy) {
          event.preventDefault();
          event.dataTransfer.dropEffect = "copy";
          setDropTargetPath("");
        }
      }}
      onDragLeave={(event) => {
        const related = event.relatedTarget;
        if (!(related instanceof Node) || !event.currentTarget.contains(related)) {
          setDropTargetPath(undefined);
        }
      }}
      onDrop={(event) => uploadDroppedFiles(event, "")}
    >
      <OpenEditorsList
        editors={openEditors}
        activeDocumentId={activeDocumentId}
        onActivate={onActivateDocument}
        onClose={onCloseDocument}
      />
      <header className="pane-header current-folder-header">
        <span id="current-folder-title" className="pane-title">
          Current Folder
        </span>
        <div className="pane-icon-actions" aria-label="Current Folder actions">
          <button
            className={refreshing ? "icon-button is-refreshing" : "icon-button"}
            type="button"
            title="Refresh Current Folder"
            aria-label="Refresh Current Folder"
            aria-busy={refreshing}
            disabled={!canMutate || operationBusy || loading || refreshing}
            onClick={onRefresh}
          >
            <span className="refresh-icon" aria-hidden="true">
              <RefreshIcon />
            </span>
          </button>
          {onOpenFile ? (
            <button className="icon-button" type="button" title="Open File…" aria-label="Open File…" disabled={!canMutate || operationBusy} onClick={onOpenFile}>
              <OpenFileIcon />
            </button>
          ) : null}
          {onRevealPath ? (
            <button className="icon-button" type="button" title="Open Current Folder in File Explorer" aria-label="Open Current Folder in File Explorer" disabled={!canMutate || operationBusy} onClick={() => onRevealPath("")}>
              <FileExplorerIcon />
            </button>
          ) : null}
          {fileTransfers ? <>
          <button
            className="icon-button"
            type="button"
            title="Upload Files"
            aria-label="Upload Files"
            disabled={!canMutate || operationBusy}
            onClick={() => requestUpload(defaultCreateParent)}
          >
            <UploadIcon />
          </button>
          <input
            ref={uploadInputRef}
            hidden
            type="file"
            multiple
            tabIndex={-1}
            aria-hidden="true"
            onChange={(event) => {
              const files = [...(event.currentTarget.files ?? [])];
              event.currentTarget.value = "";
              if (files.length > 0) {
                onUpload(files, uploadParentRef.current);
              }
            }}
          />
          </> : null}
          <button
            className="icon-button"
            type="button"
            title="New File"
            aria-label="New File"
            disabled={!canMutate || operationBusy}
            onClick={() => onStartCreate("file", defaultCreateParent)}
          >
            <NewFileIcon />
          </button>
          <button
            className="icon-button"
            type="button"
            title="New Folder"
            aria-label="New Folder"
            disabled={!canMutate || operationBusy}
            onClick={() => onStartCreate("directory", defaultCreateParent)}
          >
            <NewFolderIcon />
          </button>
        </div>
      </header>
      <div className="current-folder-path" title={`${rootName}: ${rootPath}`}>
        <FolderIcon />
        <span>{rootPath}</span>
        {searchPathWorkspacePaths.has("") ? (
          <span className="search-path-badge" title="Current Folder is on MATLAB path">
            PATH
          </span>
        ) : null}
      </div>
      {creatingKind === null ? null : (
        <form className="create-entry-form" onSubmit={submit}>
          <div className="create-entry-heading">
            <strong>New {creatingKind === "file" ? "file" : "folder"}</strong>
            <span>
              In {creatingParentPath.length === 0 ? "workspace root" : creatingParentPath}
            </span>
          </div>
          <label htmlFor="new-workspace-entry-name">Name</label>
          <input
            ref={inputRef}
            id="new-workspace-entry-name"
            value={name}
            disabled={operationBusy}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={error !== null}
            aria-describedby={error === null ? undefined : "workspace-operation-error"}
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                onCancelCreate();
              }
            }}
          />
          {error === null ? null : (
            <div id="workspace-operation-error" className="inline-error" role="alert">
              <span>{error.message}</span>
              <code>{error.code}</code>
            </div>
          )}
          <div className="create-entry-actions">
            <button
              className="button button-secondary"
              type="button"
              disabled={operationBusy}
              onClick={onCancelCreate}
            >
              Cancel
            </button>
            <button
              className="button button-primary"
              type="submit"
              disabled={operationBusy || name.trim().length === 0}
            >
              {operationBusy ? "Creating…" : "Create"}
            </button>
          </div>
        </form>
      )}
      {creatingKind === null && error !== null ? (
        <div className="workspace-tree-error" role="alert">
          <span>{error.message}</span>
          <code>{error.code}</code>
        </div>
      ) : null}
      <nav className="file-tree-nav" aria-label="Current folder files">
        {dropTargetPath === undefined ? null : (
          <div className="workspace-drop-hint" role="status">
            Drop files into {dropTargetPath.length === 0 ? "Current Folder" : dropTargetPath}
          </div>
        )}
        {loading ? (
          <div className="folder-loading" role="status">
            Loading workspace…
          </div>
        ) : entries.length === 0 ? (
          <div className="folder-loading">This folder is empty.</div>
        ) : (
          <ul ref={treeRef} className="file-tree" role="tree" aria-label="Workspace files">
            {renderChildren("", 1)}
          </ul>
        )}
      </nav>
      <div className="folder-summary">
        <span>
          {entries.length.toString()} {entries.length === 1 ? "item" : "items"}
        </span>
        <span>{serviceLabel}</span>
      </div>
      {uploadStatus === null && announcement === null ? null : (
        <div className="workspace-operation-status" role="status" aria-live="polite">
          {uploadStatus ?? announcement}
        </div>
      )}
      {contextMenu === null ? null : (
        <div
          ref={contextMenuRef}
          className="workspace-context-menu"
          role="menu"
          aria-label={`Actions for ${contextMenu.entry.path}`}
          aria-orientation="vertical"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onClick={(event) => event.stopPropagation()}
          onKeyDown={(event) => {
            const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>(
              'button:not(:disabled)',
            )];
            const current = items.indexOf(document.activeElement as HTMLButtonElement);
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              const offset = event.key === "ArrowDown" ? 1 : -1;
              items[(current + offset + items.length) % items.length]?.focus();
            } else if (event.key === "Home" || event.key === "End") {
              event.preventDefault();
              items[event.key === "Home" ? 0 : items.length - 1]?.focus();
            } else if (event.key === "Escape") {
              event.preventDefault();
              closeContextMenuAndRestoreFocus();
            }
          }}
        >
          {contextMenu.entry.kind === "directory" ? (
            <button
              type="button"
              role="menuitem"
              disabled={!canMutate || operationBusy}
              onClick={() => {
                onEnterDirectory(contextMenu.entry.path);
                setContextMenu(null);
              }}
            >
              Open
            </button>
          ) : null}
          <button
            type="button"
            role="menuitem"
            disabled={!canMutate || operationBusy}
            onClick={() => openCreateFromEntry("file", contextMenu.entry)}
          >
            New file here
          </button>
          <button
            type="button"
            role="menuitem"
            disabled={!canMutate || operationBusy}
            onClick={() => openCreateFromEntry("directory", contextMenu.entry)}
          >
            New folder here
          </button>
          {onRevealPath ? (
            <button type="button" role="menuitem" disabled={!canMutate || operationBusy} onClick={() => { onRevealPath(contextMenu.entry.path); setContextMenu(null); }}>
              {contextMenu.entry.kind === "directory" ? "Open in File Explorer" : "Show in File Explorer"}
            </button>
          ) : null}
          {fileTransfers ? (
          <button
            type="button"
            role="menuitem"
            disabled={!canMutate || operationBusy}
            onClick={() => {
              requestUpload(
                contextMenu.entry.kind === "directory"
                  ? contextMenu.entry.path
                  : workspaceParentPath(contextMenu.entry.path),
              );
              setContextMenu(null);
            }}
          >
            Upload files here…
          </button>
          ) : null}
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const entry = contextMenu.entry;
              setContextMenu(null);
              void onCopyRelativePath(entry)
                .then(() => setAnnouncement(`Copied ${entry.path}`))
                .catch(() => setAnnouncement(`Could not copy ${entry.path}`));
            }}
          >
            Copy Relative Path
          </button>
          {contextMenu.entry.kind !== "directory" ? null : searchPathWorkspacePaths.has(
              contextMenu.entry.path,
            ) ? (
            <>
              <button
                type="button"
                role="menuitem"
                disabled={!canMutate || operationBusy}
                onClick={() => {
                  onRemoveSearchPath(contextMenu.entry, false);
                  setContextMenu(null);
                }}
              >
                Remove from Path
              </button>
              <button
                type="button"
                role="menuitem"
                disabled={!canMutate || operationBusy}
                onClick={() => {
                  onRemoveSearchPath(contextMenu.entry, true);
                  setContextMenu(null);
                }}
              >
                Remove from Path with Subfolders
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                role="menuitem"
                disabled={!canMutate || operationBusy}
                onClick={() => {
                  onAddSearchPath(contextMenu.entry, false);
                  setContextMenu(null);
                }}
              >
                Add to Path
              </button>
              <button
                type="button"
                role="menuitem"
                disabled={!canMutate || operationBusy}
                onClick={() => {
                  onAddSearchPath(contextMenu.entry, true);
                  setContextMenu(null);
                }}
              >
                Add to Path with Subfolders
              </button>
            </>
          )}
          {fileTransfers && contextMenu.entry.kind === "file" ? (
            <button
              type="button"
              role="menuitem"
              disabled={!canMutate || operationBusy}
              onClick={() => {
                onDownload(contextMenu.entry);
                setContextMenu(null);
              }}
            >
              Download
            </button>
          ) : null}
          <button
            type="button"
            role="menuitem"
            disabled={!canMutate || operationBusy}
            onClick={() => {
              onRename(contextMenu.entry);
              setContextMenu(null);
            }}
          >
            Rename…
          </button>
          <button
            type="button"
            role="menuitem"
            disabled={!canMutate || operationBusy}
            onClick={() => {
              onMove(contextMenu.entry);
              setContextMenu(null);
            }}
          >
            Move…
          </button>
          <button
            className="danger"
            type="button"
            role="menuitem"
            disabled={!canMutate || operationBusy}
            onClick={() => {
              onDelete(contextMenu.entry);
              setContextMenu(null);
            }}
          >
            Delete…
          </button>
        </div>
      )}
    </aside>
  );
}
