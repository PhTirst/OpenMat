import type { WorkspaceFile } from "../workspace/workspace-client";
import { workspaceDocumentUri } from "../workspace/workspace-client";

export const DOCUMENT_SESSION_VERSION = 1 as const;
export const MAX_SESSION_DOCUMENTS = 64;

export interface DocumentViewState {
  readonly lineNumber: number;
  readonly column: number;
  readonly scrollTop: number;
  readonly scrollLeft: number;
}

export type DocumentRecoveryStatus = "none" | "conflict" | "unavailable";

export interface OpenDocument {
  readonly id: string;
  readonly path: string;
  readonly pendingPath?: string;
  readonly rootPath: string;
  readonly rootGeneration: number;
  readonly uri: string;
  readonly content: string;
  readonly savedContent: string;
  readonly revision: string;
  readonly version: number;
  readonly recoveryStatus: DocumentRecoveryStatus;
  readonly recoveryMessage: string | null;
  readonly viewState: DocumentViewState | null;
}

export interface PersistedOpenDocument {
  readonly path: string;
  readonly pendingPath?: string;
  readonly rootPath: string;
  readonly rootGeneration: number;
  readonly content: string;
  readonly savedContent: string;
  readonly revision: string;
  readonly version: number;
  readonly recoveryStatus: DocumentRecoveryStatus;
  readonly viewState: DocumentViewState | null;
}

export interface PersistedDocumentSession {
  readonly version: typeof DOCUMENT_SESSION_VERSION;
  readonly activeDocumentId: string | null;
  readonly documents: readonly PersistedOpenDocument[];
  readonly desktopWorkspace?: DesktopWorkspaceSession;
}

export interface DesktopWorkspaceSession {
  readonly rootPath: string;
  readonly designerMounted: boolean;
  readonly designerVisible: boolean;
}

export function documentId(rootPath: string, path: string): string {
  return JSON.stringify([rootPath, path]);
}

export function isDocumentDirty(document: OpenDocument): boolean {
  return document.pendingPath !== undefined || document.revision === "" ||
    document.content !== document.savedContent;
}

export function openDocumentFromWorkspaceFile(file: WorkspaceFile): OpenDocument {
  return {
    id: documentId(file.rootPath, file.path),
    path: file.path,
    rootPath: file.rootPath,
    rootGeneration: file.rootGeneration,
    uri: workspaceDocumentUri(file.path, file.rootGeneration, file.rootPath),
    content: file.content,
    savedContent: file.content,
    revision: file.revision,
    version: 1,
    recoveryStatus: "none",
    recoveryMessage: null,
    viewState: null,
  };
}

export function snapshotDocumentSession(
  documents: readonly OpenDocument[],
  activeDocumentId: string | null,
  desktopWorkspace?: DesktopWorkspaceSession,
): PersistedDocumentSession {
  return {
    version: DOCUMENT_SESSION_VERSION,
    activeDocumentId,
    ...(desktopWorkspace === undefined ? {} : { desktopWorkspace }),
    documents: documents.slice(0, MAX_SESSION_DOCUMENTS).map((document) => ({
      path: document.path,
      ...(isValidPendingPath(document.path, document.pendingPath)
        ? { pendingPath: document.pendingPath }
        : {}),
      rootPath: document.rootPath,
      rootGeneration: document.rootGeneration,
      content: document.content,
      savedContent: document.savedContent,
      revision: document.revision,
      version: document.version,
      recoveryStatus: document.recoveryStatus,
      viewState: document.viewState,
    })),
  };
}

export function parseDocumentSession(value: unknown): PersistedDocumentSession | null {
  if (!isRecord(value) || value.version !== DOCUMENT_SESSION_VERSION) {
    return null;
  }
  if (!Array.isArray(value.documents)) {
    return null;
  }
  const documents: PersistedOpenDocument[] = [];
  const seenIds = new Set<string>();
  for (const candidate of value.documents) {
    const document = parsePersistedDocument(candidate);
    if (document === null) {
      continue;
    }
    const id = documentId(document.rootPath, document.path);
    if (seenIds.has(id)) {
      continue;
    }
    seenIds.add(id);
    documents.push(document);
    if (documents.length === MAX_SESSION_DOCUMENTS) {
      break;
    }
  }
  const ids = new Set(
    documents.map((document) => documentId(document.rootPath, document.path)),
  );
  const activeDocumentId =
    typeof value.activeDocumentId === "string" && ids.has(value.activeDocumentId)
      ? value.activeDocumentId
      : null;
  const workspace = value.desktopWorkspace;
  const desktopWorkspace = isRecord(workspace) &&
    isBoundedNonEmptyString(workspace.rootPath, 32_768) &&
    !/[\u0000-\u001f]/.test(workspace.rootPath) &&
    typeof workspace.designerMounted === "boolean" &&
    typeof workspace.designerVisible === "boolean"
    ? { rootPath: workspace.rootPath, designerMounted: workspace.designerMounted,
        designerVisible: workspace.designerVisible }
    : undefined;
  return { version: DOCUMENT_SESSION_VERSION, activeDocumentId, documents,
    ...(desktopWorkspace === undefined ? {} : { desktopWorkspace }) };
}

export function restoreDocument(
  persisted: PersistedOpenDocument,
  currentFile: WorkspaceFile | null,
  unavailableMessage: string,
): OpenDocument {
  const id = documentId(persisted.rootPath, persisted.path);
  if (currentFile === null) {
    return {
      ...persisted,
      id,
      uri: workspaceDocumentUri(persisted.path, persisted.rootGeneration, persisted.rootPath),
      recoveryStatus: "unavailable",
      recoveryMessage: unavailableMessage,
    };
  }

  const dirty = persisted.pendingPath !== undefined || persisted.revision === "" ||
    persisted.content !== persisted.savedContent;
  const changedOnDisk = currentFile.revision !== persisted.revision;
  const unresolvedConflict = persisted.recoveryStatus === "conflict";
  if (!dirty && !unresolvedConflict) {
    return {
      id,
      path: currentFile.path,
      rootPath: currentFile.rootPath,
      rootGeneration: currentFile.rootGeneration,
      uri: workspaceDocumentUri(currentFile.path, currentFile.rootGeneration, currentFile.rootPath),
      content: currentFile.content,
      savedContent: currentFile.content,
      revision: currentFile.revision,
      version: Math.max(1, persisted.version + 1),
      recoveryStatus: "none",
      recoveryMessage: null,
      viewState: persisted.viewState,
    };
  }

  return {
    id,
    path: currentFile.path,
    ...(persisted.pendingPath === undefined ? {} : { pendingPath: persisted.pendingPath }),
    rootPath: currentFile.rootPath,
    rootGeneration: currentFile.rootGeneration,
    uri: workspaceDocumentUri(currentFile.path, currentFile.rootGeneration, currentFile.rootPath),
    content: persisted.content,
    savedContent: currentFile.content,
    revision: currentFile.revision,
    version: Math.max(1, persisted.version + 1),
    recoveryStatus: changedOnDisk || unresolvedConflict ? "conflict" : "none",
    recoveryMessage: changedOnDisk || unresolvedConflict
      ? "This recovered draft was based on an older disk revision. Choose which version to keep before saving."
      : null,
    viewState: persisted.viewState,
  };
}

/** Call only after the current matching root confirmed that the file is absent. */
export function restoreNewDocument(
  persisted: PersistedOpenDocument,
  currentRootGeneration: number,
): OpenDocument | null {
  if (persisted.revision !== "") {
    return null;
  }
  return {
    ...persisted,
    id: documentId(persisted.rootPath, persisted.path),
    rootGeneration: currentRootGeneration,
    uri: workspaceDocumentUri(persisted.path, currentRootGeneration, persisted.rootPath),
    recoveryStatus: "none",
    recoveryMessage: null,
  };
}

function parsePersistedDocument(value: unknown): PersistedOpenDocument | null {
  if (
    !isRecord(value) ||
    !isRelativeWorkspacePath(value.path) ||
    (value.pendingPath !== undefined && !isValidPendingPath(value.path, value.pendingPath)) ||
    !isBoundedNonEmptyString(value.rootPath, 32_768) ||
    !isBoundedString(value.content, 512 * 1024) ||
    !isBoundedString(value.savedContent, 512 * 1024) ||
    !isBoundedString(value.revision, 4_096) ||
    !isPositiveInteger(value.version) ||
    !isRecoveryStatus(value.recoveryStatus)
  ) {
    return null;
  }
  const viewState = parseViewState(value.viewState);
  if (value.viewState !== null && viewState === null) {
    return null;
  }
  return {
    path: value.path,
    ...(typeof value.pendingPath === "string" ? { pendingPath: value.pendingPath } : {}),
    rootPath: value.rootPath,
    rootGeneration: isNonNegativeInteger(value.rootGeneration)
      ? value.rootGeneration
      : 0,
    content: value.content,
    savedContent: value.savedContent,
    revision: value.revision,
    version: value.version,
    recoveryStatus: value.recoveryStatus,
    viewState,
  };
}

function parseViewState(value: unknown): DocumentViewState | null {
  if (value === null) {
    return null;
  }
  if (
    !isRecord(value) ||
    !isPositiveInteger(value.lineNumber) ||
    !isPositiveInteger(value.column) ||
    !isNonNegativeNumber(value.scrollTop) ||
    !isNonNegativeNumber(value.scrollLeft)
  ) {
    return null;
  }
  return {
    lineNumber: value.lineNumber,
    column: value.column,
    scrollTop: value.scrollTop,
    scrollLeft: value.scrollLeft,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isBoundedString(value: unknown, maximumLength: number): value is string {
  return typeof value === "string" && value.length <= maximumLength;
}

function isBoundedNonEmptyString(
  value: unknown,
  maximumLength: number,
): value is string {
  return isBoundedString(value, maximumLength) && value.length > 0;
}

function isRelativeWorkspacePath(value: unknown): value is string {
  return isBoundedNonEmptyString(value, 32_768) &&
    !/[\\\u0000-\u001f\u007f]/.test(value) &&
    !/^[A-Za-z]:/.test(value) &&
    value.split("/").every((segment) => segment !== "" && segment !== "." && segment !== "..");
}

function isValidPendingPath(path: string, pendingPath: unknown): pendingPath is string {
  return isRelativeWorkspacePath(path) && isRelativeWorkspacePath(pendingPath) &&
    path !== pendingPath && /\.m$/i.test(path) && /\.m$/i.test(pendingPath) &&
    path.slice(0, path.lastIndexOf("/") + 1) ===
      pendingPath.slice(0, pendingPath.lastIndexOf("/") + 1);
}

function isPositiveInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) > 0;
}

function isNonNegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}

function isNonNegativeNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function isRecoveryStatus(value: unknown): value is DocumentRecoveryStatus {
  return value === "none" || value === "conflict" || value === "unavailable";
}
