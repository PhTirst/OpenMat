import { useEffect, useId, useMemo, useState, type ReactNode } from "react";
import type { OpenDocument } from "../documents/document-session";
import {
  joinWorkspacePath,
  workspaceParentPath,
  type WorkspaceEntry,
} from "../workspace/workspace-client";
import { FolderIcon } from "./Icons";

const WINDOWS_RESERVED_STEMS = new Set([
  "CON",
  "PRN",
  "AUX",
  "NUL",
  "COM1",
  "COM2",
  "COM3",
  "COM4",
  "COM5",
  "COM6",
  "COM7",
  "COM8",
  "COM9",
  "LPT1",
  "LPT2",
  "LPT3",
  "LPT4",
  "LPT5",
  "LPT6",
  "LPT7",
  "LPT8",
  "LPT9",
  "CONIN$",
  "CONOUT$",
  "CLOCK$",
]);

interface OperationDialogProps {
  readonly title: string;
  readonly description: string;
  readonly size?: "compact" | "wide";
  readonly children: ReactNode;
  readonly footer: ReactNode;
  readonly onCancel: () => void;
}

function OperationDialog({
  title,
  description,
  size = "compact",
  children,
  footer,
  onCancel,
}: OperationDialogProps) {
  const titleId = useId();
  const descriptionId = useId();
  return (
    <section
      className={`operation-dialog ${size}`}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      tabIndex={-1}
    >
      <header>
        <div>
          <strong id={titleId}>{title}</strong>
          <span id={descriptionId}>{description}</span>
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label={`Close ${title}`}
          title="Close"
          onClick={onCancel}
        >
          ×
        </button>
      </header>
      <div className="operation-dialog-body">{children}</div>
      <footer>{footer}</footer>
    </section>
  );
}

function utf16Length(value: string): number {
  return [...value].reduce(
    (total, character) => total + (character.codePointAt(0)! > 0xffff ? 2 : 1),
    0,
  );
}

export function validateWorkspaceEntryName(
  name: string,
  currentName: string | null,
  siblingNames: readonly string[],
): string | null {
  if (name.length === 0 || name.trim().length === 0) {
    return "Enter a file or folder name.";
  }
  if (
    name === "." ||
    name === ".." ||
    name.includes("/") ||
    name.includes("\\") ||
    name.includes("\0")
  ) {
    return "Use one name only; path separators and dot segments are not allowed.";
  }
  if (
    name.endsWith(" ") ||
    name.endsWith(".") ||
    [...name].some(
      (character) =>
        character.codePointAt(0)! <= 0x1f || '<>:"|?*'.includes(character),
    )
  ) {
    return "The name contains characters or a trailing suffix Windows does not allow.";
  }
  if (utf16Length(name) > 255) {
    return "The name is longer than the 255 UTF-16 code-unit limit.";
  }
  const stem = name.split(".")[0]?.toUpperCase() ?? "";
  if (WINDOWS_RESERVED_STEMS.has(stem)) {
    return `“${name}” is a reserved Windows device name.`;
  }
  if (currentName !== null && name === currentName) {
    return "Enter a different name.";
  }
  const folded = name.toUpperCase();
  if (
    siblingNames.some(
      (candidate) =>
        candidate !== currentName && candidate.toUpperCase() === folded,
    )
  ) {
    return `“${name}” already exists in this folder.`;
  }
  return null;
}

interface RenameEntryDialogProps {
  readonly entry: WorkspaceEntry;
  readonly siblingNames: readonly string[];
  readonly onRename: (name: string) => Promise<string | null>;
  readonly onComplete: () => void;
  readonly onCancel: () => void;
}

export function RenameEntryDialog({
  entry,
  siblingNames,
  onRename,
  onComplete,
  onCancel,
}: RenameEntryDialogProps) {
  const [name, setName] = useState(entry.name);
  const [submitting, setSubmitting] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const validationError = useMemo(
    () => validateWorkspaceEntryName(name, entry.name, siblingNames),
    [entry.name, name, siblingNames],
  );

  const submit = async (): Promise<void> => {
    if (validationError !== null || submitting) {
      return;
    }
    setSubmitting(true);
    setOperationError(null);
    const error = await onRename(name);
    if (error === null) {
      onComplete();
      return;
    }
    setOperationError(error);
    setSubmitting(false);
  };

  return (
    <OperationDialog
      title={`Rename ${entry.kind}`}
      description="The new name is checked before OpenMat changes the workspace."
      onCancel={onCancel}
      footer={
        <div className="operation-dialog-actions">
          <button
            className="button button-secondary"
            type="button"
            disabled={submitting}
            onClick={onCancel}
          >
            Cancel
          </button>
          <button
            className="button button-primary"
            type="submit"
            form="rename-workspace-entry"
            disabled={validationError !== null || submitting}
          >
            {submitting ? "Renaming…" : "Rename"}
          </button>
        </div>
      }
    >
      <form
        id="rename-workspace-entry"
        className="operation-dialog-form"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <label htmlFor="rename-workspace-name">New name</label>
        <input
          id="rename-workspace-name"
          data-dialog-initial-focus
          value={name}
          disabled={submitting}
          aria-invalid={validationError !== null}
          aria-describedby="rename-workspace-feedback"
          spellCheck={false}
          onChange={(event) => {
            setName(event.target.value);
            setOperationError(null);
          }}
        />
        <code className="operation-dialog-path">{entry.path}</code>
        <div
          id="rename-workspace-feedback"
          className="operation-dialog-feedback"
          role={validationError === null && operationError === null ? undefined : "alert"}
        >
          {validationError ?? operationError ?? "Name is available."}
        </div>
      </form>
    </OperationDialog>
  );
}

export interface WorkspaceDirectoryOption {
  readonly name: string;
  readonly path: string;
}

interface DirectoryTreeProps {
  readonly rootName: string;
  readonly childrenByPath: ReadonlyMap<string, readonly WorkspaceDirectoryOption[]>;
  readonly expanded: ReadonlySet<string>;
  readonly loading: ReadonlySet<string>;
  readonly selectedPath: string;
  readonly disabledPath: (path: string) => boolean;
  readonly onToggle: (path: string) => void;
  readonly onSelect: (path: string) => void;
}

function DirectoryTree({
  rootName,
  childrenByPath,
  expanded,
  loading,
  selectedPath,
  disabledPath,
  onToggle,
  onSelect,
}: DirectoryTreeProps) {
  const renderChildren = (parentPath: string, depth: number): ReactNode =>
    childrenByPath.get(parentPath)?.map((entry) => {
      const open = expanded.has(entry.path);
      const disabled = disabledPath(entry.path);
      return (
        <li
          key={entry.path}
          role="treeitem"
          aria-expanded={open}
          aria-selected={selectedPath === entry.path}
          aria-disabled={disabled}
        >
          <div
            className={`move-directory-row${selectedPath === entry.path ? " selected" : ""}`}
            style={{ paddingLeft: `${(depth * 18).toString()}px` }}
          >
            <button
              className="move-directory-toggle"
              type="button"
              aria-label={`${open ? "Collapse" : "Expand"} ${entry.path}`}
              disabled={disabled}
              onClick={() => onToggle(entry.path)}
            >
              {loading.has(entry.path) ? "…" : open ? "⌄" : "›"}
            </button>
            <button
              className="move-directory-select"
              type="button"
              disabled={disabled}
              onClick={() => onSelect(entry.path)}
            >
              <FolderIcon />
              <span>{entry.name}</span>
            </button>
          </div>
          {open ? <ul role="group">{renderChildren(entry.path, depth + 1)}</ul> : null}
        </li>
      );
    }) ?? null;

  const rootOpen = expanded.has("");
  return (
    <ul className="move-directory-tree" role="tree" aria-label="Destination folders">
      <li
        role="treeitem"
        aria-expanded={rootOpen}
        aria-selected={selectedPath === ""}
      >
        <div className={`move-directory-row${selectedPath === "" ? " selected" : ""}`}>
          <button
            className="move-directory-toggle"
            type="button"
            aria-label={`${rootOpen ? "Collapse" : "Expand"} workspace root`}
            onClick={() => onToggle("")}
          >
            {loading.has("") ? "…" : rootOpen ? "⌄" : "›"}
          </button>
          <button
            className="move-directory-select"
            type="button"
            onClick={() => onSelect("")}
          >
            <FolderIcon />
            <span>{rootName}</span>
          </button>
        </div>
        {rootOpen ? <ul role="group">{renderChildren("", 1)}</ul> : null}
      </li>
    </ul>
  );
}

interface MoveEntryDialogProps {
  readonly entry: WorkspaceEntry;
  readonly rootName: string;
  readonly loadDirectories: (
    path: string,
  ) => Promise<readonly WorkspaceDirectoryOption[]>;
  readonly onMove: (targetPath: string) => Promise<string | null>;
  readonly onComplete: () => void;
  readonly onCancel: () => void;
}

export function MoveEntryDialog({
  entry,
  rootName,
  loadDirectories,
  onMove,
  onComplete,
  onCancel,
}: MoveEntryDialogProps) {
  const initialParent = workspaceParentPath(entry.path);
  const [selectedPath, setSelectedPath] = useState(initialParent);
  const [childrenByPath, setChildrenByPath] = useState<
    ReadonlyMap<string, readonly WorkspaceDirectoryOption[]>
  >(() => new Map());
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () => new Set([""]),
  );
  const [loading, setLoading] = useState<ReadonlySet<string>>(() => new Set());
  const [loadError, setLoadError] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const disabledPath = (path: string): boolean =>
    entry.kind === "directory" &&
    (path === entry.path || path.startsWith(`${entry.path}/`));

  const ensureLoaded = async (path: string): Promise<void> => {
    if (childrenByPath.has(path) || loading.has(path)) {
      return;
    }
    setLoading((current) => new Set(current).add(path));
    setLoadError(null);
    try {
      const entries = await loadDirectories(path);
      setChildrenByPath((current) => new Map(current).set(path, entries));
    } catch (error: unknown) {
      setLoadError(error instanceof Error ? error.message : "Could not load folders.");
    } finally {
      setLoading((current) => {
        const next = new Set(current);
        next.delete(path);
        return next;
      });
    }
  };

  useEffect(() => {
    let disposed = false;
    const loadInitialPath = async (): Promise<void> => {
      const ancestors = [""];
      if (initialParent.length > 0) {
        const parts = initialParent.split("/");
        for (let index = 1; index <= parts.length; index += 1) {
          ancestors.push(parts.slice(0, index).join("/"));
        }
      }
      for (const path of ancestors) {
        if (disposed) return;
        await ensureLoaded(path);
      }
      if (!disposed) {
        setExpanded(new Set(ancestors));
      }
    };
    void loadInitialPath();
    return () => {
      disposed = true;
    };
    // Initial hydration must run once; subsequent expansion is event-driven.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const targetPath = joinWorkspacePath(selectedPath, entry.name);
  const validationError =
    disabledPath(selectedPath)
      ? "A folder cannot be moved inside itself."
      : targetPath === entry.path
        ? "Choose a different destination folder."
        : null;

  const submit = async (): Promise<void> => {
    if (validationError !== null || submitting) return;
    setSubmitting(true);
    setOperationError(null);
    const error = await onMove(targetPath);
    if (error === null) {
      onComplete();
      return;
    }
    setOperationError(error);
    setSubmitting(false);
  };

  return (
    <OperationDialog
      title={`Move ${entry.kind}`}
      description="Choose a workspace folder. The item keeps its current name."
      size="wide"
      onCancel={onCancel}
      footer={
        <div className="operation-dialog-actions spread">
          <code title={targetPath}>{targetPath}</code>
          <span>
            <button
              className="button button-secondary"
              type="button"
              data-dialog-initial-focus
              disabled={submitting}
              onClick={onCancel}
            >
              Cancel
            </button>
            <button
              className="button button-primary"
              type="button"
              disabled={validationError !== null || submitting}
              onClick={() => void submit()}
            >
              {submitting ? "Moving…" : "Move"}
            </button>
          </span>
        </div>
      }
    >
      <div className="move-directory-browser">
        <DirectoryTree
          rootName={rootName}
          childrenByPath={childrenByPath}
          expanded={expanded}
          loading={loading}
          selectedPath={selectedPath}
          disabledPath={disabledPath}
          onToggle={(path) => {
            setExpanded((current) => {
              const next = new Set(current);
              if (next.has(path)) next.delete(path);
              else next.add(path);
              return next;
            });
            if (!expanded.has(path)) void ensureLoaded(path);
          }}
          onSelect={(path) => {
            setSelectedPath(path);
            setOperationError(null);
            void ensureLoaded(path);
          }}
        />
      </div>
      {loadError ?? validationError ?? operationError ? (
        <div className="operation-dialog-feedback error" role="alert">
          {loadError ?? validationError ?? operationError}
        </div>
      ) : null}
    </OperationDialog>
  );
}

export interface DeleteImpact {
  readonly files: number;
  readonly directories: number;
}

interface DeleteEntryDialogProps {
  readonly entry: WorkspaceEntry;
  readonly dirtyDocumentCount: number;
  readonly loadImpact: () => Promise<DeleteImpact>;
  readonly onDelete: () => Promise<string | null>;
  readonly onComplete: () => void;
  readonly onCancel: () => void;
}

export function DeleteEntryDialog({
  entry,
  dirtyDocumentCount,
  loadImpact,
  onDelete,
  onComplete,
  onCancel,
}: DeleteEntryDialogProps) {
  const [impact, setImpact] = useState<DeleteImpact | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    let disposed = false;
    void loadImpact()
      .then((loaded) => {
        if (!disposed) setImpact(loaded);
      })
      .catch((failure: unknown) => {
        if (!disposed) {
          setError(
            failure instanceof Error
              ? failure.message
              : "Could not inspect the delete target.",
          );
        }
      });
    return () => {
      disposed = true;
    };
  }, [loadImpact]);

  const submit = async (): Promise<void> => {
    if (impact === null || error !== null || submitting) return;
    setSubmitting(true);
    const failure = await onDelete();
    if (failure === null) {
      onComplete();
      return;
    }
    setError(failure);
    setSubmitting(false);
  };

  return (
    <OperationDialog
      title={`Permanently delete ${entry.kind}?`}
      description="This operation cannot be undone from OpenMat."
      onCancel={onCancel}
      footer={
        <div className="operation-dialog-actions">
          <button
            className="button button-secondary"
            type="button"
            data-dialog-initial-focus
            disabled={submitting}
            onClick={onCancel}
          >
            Cancel
          </button>
          <button
            className="button button-danger"
            type="button"
            disabled={impact === null || error !== null || submitting}
            onClick={() => void submit()}
          >
            {submitting ? "Deleting…" : "Permanently Delete"}
          </button>
        </div>
      }
    >
      <code className="operation-dialog-target">{entry.path}</code>
      {impact === null && error === null ? (
        <div className="operation-dialog-status" role="status">
          Counting affected files…
        </div>
      ) : impact === null ? null : (
        <dl className="delete-impact">
          <div>
            <dt>Files</dt>
            <dd>{impact.files}</dd>
          </div>
          <div>
            <dt>Folders</dt>
            <dd>{impact.directories}</dd>
          </div>
          <div className={dirtyDocumentCount > 0 ? "warning" : undefined}>
            <dt>Unsaved open files</dt>
            <dd>{dirtyDocumentCount}</dd>
          </div>
        </dl>
      )}
      {dirtyDocumentCount > 0 ? (
        <p className="operation-dialog-warning">
          Unsaved changes inside this target will also be discarded.
        </p>
      ) : null}
      {error === null ? null : (
        <div className="operation-dialog-feedback error" role="alert">
          {error}
        </div>
      )}
    </OperationDialog>
  );
}

export type UnsavedChangesDecision = "save" | "discard" | "cancel";

interface UnsavedChangesDialogProps {
  readonly document: OpenDocument;
  readonly onDecision: (decision: UnsavedChangesDecision) => void;
}

export function UnsavedChangesDialog({
  document,
  onDecision,
}: UnsavedChangesDialogProps) {
  return (
    <OperationDialog
      title="Save changes before closing?"
      description="Choose what OpenMat should do with the edited document."
      onCancel={() => onDecision("cancel")}
      footer={
        <div className="operation-dialog-actions spread">
          <button
            className="button button-danger-text"
            type="button"
            onClick={() => onDecision("discard")}
          >
            Don’t Save
          </button>
          <span>
            <button
              className="button button-secondary"
              type="button"
              data-dialog-initial-focus
              onClick={() => onDecision("cancel")}
            >
              Cancel
            </button>
            <button
              className="button button-primary"
              type="button"
              onClick={() => onDecision("save")}
            >
              Save
            </button>
          </span>
        </div>
      }
    >
      <code className="operation-dialog-target">{document.path}</code>
      <p>Your in-memory edits have not been written to the workspace.</p>
    </OperationDialog>
  );
}

export type SaveConflictDecision = "reload" | "saveCopy" | "keep";

interface SaveConflictDialogProps {
  readonly document: OpenDocument;
  readonly message: string;
  readonly onDecision: (decision: SaveConflictDecision) => void;
}

export function SaveConflictDialog({
  document,
  message,
  onDecision,
}: SaveConflictDialogProps) {
  return (
    <OperationDialog
      title="The file changed on disk"
      description="OpenMat stopped the save so your edits were not overwritten."
      onCancel={() => onDecision("keep")}
      footer={
        <div className="operation-dialog-actions spread">
          <button
            className="button button-danger-text"
            type="button"
            onClick={() => onDecision("reload")}
          >
            Reload from Disk
          </button>
          <span>
            <button
              className="button button-secondary"
              type="button"
              data-dialog-initial-focus
              onClick={() => onDecision("keep")}
            >
              Keep Editing
            </button>
            <button
              className="button button-primary"
              type="button"
              onClick={() => onDecision("saveCopy")}
            >
              Save a Copy…
            </button>
          </span>
        </div>
      }
    >
      <code className="operation-dialog-target">{document.path}</code>
      <p>{message}</p>
      <p className="operation-dialog-warning">
        Reloading discards the editor contents shown above. Keeping them does not
        overwrite the newer disk revision.
      </p>
    </OperationDialog>
  );
}

function suggestedCopyName(name: string): string {
  const separator = name.lastIndexOf(".");
  return separator <= 0
    ? `${name} copy`
    : `${name.slice(0, separator)} copy${name.slice(separator)}`;
}

interface SaveCopyDialogProps {
  readonly document: OpenDocument;
  readonly siblingNames: readonly string[];
  readonly onSave: (path: string) => Promise<string | null>;
  readonly onComplete: () => void;
  readonly onCancel: () => void;
}

export function SaveCopyDialog({
  document,
  siblingNames,
  onSave,
  onComplete,
  onCancel,
}: SaveCopyDialogProps) {
  const [name, setName] = useState(() => suggestedCopyName(document.path.split("/").at(-1) ?? "copy.m"));
  const [submitting, setSubmitting] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const validationError = validateWorkspaceEntryName(name, null, siblingNames);
  const path = joinWorkspacePath(workspaceParentPath(document.path), name);

  const submit = async (): Promise<void> => {
    if (validationError !== null || submitting) return;
    setSubmitting(true);
    setOperationError(null);
    const error = await onSave(path);
    if (error === null) {
      onComplete();
      return;
    }
    setOperationError(error);
    setSubmitting(false);
  };

  return (
    <OperationDialog
      title="Save an edited copy"
      description="The newer disk file remains unchanged. Your editor switches to the copy."
      onCancel={onCancel}
      footer={
        <div className="operation-dialog-actions">
          <button
            className="button button-secondary"
            type="button"
            disabled={submitting}
            onClick={onCancel}
          >
            Cancel
          </button>
          <button
            className="button button-primary"
            type="submit"
            form="save-workspace-copy"
            disabled={validationError !== null || submitting}
          >
            {submitting ? "Saving…" : "Save Copy"}
          </button>
        </div>
      }
    >
      <form
        id="save-workspace-copy"
        className="operation-dialog-form"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <label htmlFor="save-copy-name">Copy name</label>
        <input
          id="save-copy-name"
          data-dialog-initial-focus
          value={name}
          disabled={submitting}
          aria-invalid={validationError !== null}
          spellCheck={false}
          onChange={(event) => {
            setName(event.target.value);
            setOperationError(null);
          }}
        />
        <code className="operation-dialog-path">{path}</code>
        <div
          className="operation-dialog-feedback"
          role={validationError === null && operationError === null ? undefined : "alert"}
        >
          {validationError ?? operationError ?? "The copy name is available."}
        </div>
      </form>
    </OperationDialog>
  );
}

interface ClearWorkspaceDialogProps {
  readonly variableCount: number;
  readonly onConfirm: () => void;
  readonly onCancel: () => void;
}

export function ClearWorkspaceDialog({
  variableCount,
  onConfirm,
  onCancel,
}: ClearWorkspaceDialogProps) {
  return (
    <OperationDialog
      title="Clear the entire workspace?"
      description="This removes every variable from the active kernel session."
      onCancel={onCancel}
      footer={
        <div className="operation-dialog-actions">
          <button
            className="button button-secondary"
            type="button"
            data-dialog-initial-focus
            onClick={onCancel}
          >
            Cancel
          </button>
          <button
            className="button button-danger"
            type="button"
            onClick={onConfirm}
          >
            Clear Workspace
          </button>
        </div>
      }
    >
      <p>
        {variableCount === 1
          ? "1 variable will be removed."
          : `${variableCount.toString()} variables will be removed.`}
      </p>
      <p className="operation-dialog-warning">This action cannot be undone.</p>
    </OperationDialog>
  );
}
