export const WORKSPACE_PROTOCOL_V1 = "openmat-workspace-v1" as const;
export const WORKSPACE_PROTOCOL_V2 = "openmat-workspace-v2" as const;
export const WORKSPACE_PROTOCOL_V3 = "openmat-workspace-v3" as const;

export type WorkspaceEntryKind = "file" | "directory" | "other";
export type CreatableWorkspaceEntryKind = Exclude<WorkspaceEntryKind, "other">;

export interface WorkspaceEntry {
  readonly name: string;
  readonly path: string;
  readonly kind: WorkspaceEntryKind;
  readonly size: number | null;
  readonly revision: string | null;
}

export interface WorkspaceSnapshot {
  readonly rootName: string;
  readonly rootPath: string;
  readonly rootGeneration: number;
  readonly path: string;
  readonly recursive: boolean;
  readonly entries: readonly WorkspaceEntry[];
}

export interface WorkspaceFile {
  readonly path: string;
  readonly content: string;
  readonly revision: string;
  readonly size: number;
  readonly rootGeneration: number;
  readonly rootPath: string;
}

export interface WorkspaceDownload {
  readonly url: string;
  readonly name: string;
  readonly size: number;
  readonly expiresInSeconds: number;
}

export interface WorkspaceUpload {
  readonly url: string;
  readonly name: string;
  readonly size: number;
  readonly expiresInSeconds: number;
}

export interface CurrentDirectory {
  readonly path: string;
  readonly rootName: string;
  readonly generation: number;
}

export interface WorkspaceChange {
  readonly rootGeneration: number;
}

export interface SearchPathDirectory {
  readonly path: string;
  readonly workspacePath: string | null;
}

export interface SearchPathSnapshot {
  readonly generation: number;
  readonly directories: readonly SearchPathDirectory[];
}

export type SearchPathPosition = "begin" | "end";

export interface DirectoryBrowserEntry {
  readonly name: string;
  readonly path: string;
}

export interface DirectoryBrowserSnapshot {
  readonly path: string;
  readonly parentPath: string | null;
  readonly roots: readonly DirectoryBrowserEntry[];
  readonly entries: readonly DirectoryBrowserEntry[];
}

export interface WorkspaceMoveResult {
  readonly previousPath: string;
  readonly entry: WorkspaceEntry;
}

export interface WorkspaceDeleteResult {
  readonly path: string;
  readonly kind: WorkspaceEntryKind;
  readonly recursive: boolean;
}

export type WorkspaceClientKind = "mock" | "websocket";
export type WorkspaceConnectionLossListener = (error: Error) => void;

export class WorkspaceClientError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly field?: string,
    readonly details?: Readonly<Record<string, unknown>>,
  ) {
    super(message);
    this.name = "WorkspaceClientError";
  }
}

export interface WorkspaceClient {
  readonly kind: WorkspaceClientKind;
  readonly label: string;
  connect(): Promise<void>;
  disconnect(): Promise<void>;
  subscribeConnectionLoss(listener: WorkspaceConnectionLossListener): () => void;
  currentDirectory(): Promise<CurrentDirectory>;
  changeDirectory(path: string): Promise<CurrentDirectory>;
  browseDirectories(path: string): Promise<DirectoryBrowserSnapshot>;
  onCurrentDirectoryChanged(
    listener: (directory: CurrentDirectory) => void,
  ): () => void;
  onWorkspaceChanged(listener: (change: WorkspaceChange) => void): () => void;
  searchPath?(): Promise<SearchPathSnapshot>;
  addSearchPath?(
    path: string,
    recursive: boolean,
    position?: SearchPathPosition,
  ): Promise<SearchPathSnapshot>;
  removeSearchPath?(path: string, recursive: boolean): Promise<SearchPathSnapshot>;
  onSearchPathChanged?(
    listener: (snapshot: SearchPathSnapshot) => void,
  ): () => void;
  list(path: string, recursive: boolean): Promise<WorkspaceSnapshot>;
  read(path: string): Promise<WorkspaceFile>;
  prepareDownload(path: string, rootGeneration: number): Promise<WorkspaceDownload>;
  upload(
    path: string,
    file: File,
    rootGeneration: number,
    overwrite?: boolean,
  ): Promise<WorkspaceEntry>;
  write(
    path: string,
    content: string,
    expectedRevision: string,
    rootGeneration: number,
  ): Promise<WorkspaceEntry>;
  create(
    path: string,
    kind: CreatableWorkspaceEntryKind,
  ): Promise<WorkspaceEntry>;
  rename(path: string, newName: string, rootGeneration?: number): Promise<WorkspaceMoveResult>;
  move(path: string, targetPath: string): Promise<WorkspaceMoveResult>;
  delete(path: string, recursive: boolean): Promise<WorkspaceDeleteResult>;
}

export function workspaceDocumentUri(
  path: string,
  rootGeneration = 0,
  rootPath?: string,
): string {
  // A server restart can reuse a generation for a different workspace root.
  // Escape twice so Monaco's URI parser cannot treat root separators or a Windows
  // drive letter as part of the document path when it decodes the URI once.
  const rootSegment = rootPath === undefined
    ? ""
    : `${encodeURIComponent(encodeURIComponent(rootPath))}/`;
  return `openmat-workspace://root-${rootGeneration.toString()}/${rootSegment}${path
    .split("/")
    .map((segment) => encodeURIComponent(segment))
    .join("/")}`;
}

export interface WorkspaceDocumentLocation {
  readonly path: string;
  readonly rootPath: string;
  readonly rootGeneration: number;
}

export function parseWorkspaceDocumentUri(uri: string): WorkspaceDocumentLocation | null {
  // Do not use URL here: it normalizes dot segments before we can reject them.
  // The root is escaped twice by workspaceDocumentUri; relative segments once.
  const match = /^openmat-workspace:\/\/root-(0|[1-9]\d*)\/([^/?#]+)\/([^?#]+)$/.exec(uri);
  if (match === null || uri.length > 262_144) {
    return null;
  }
  const rootGeneration = Number(match[1]);
  if (!Number.isSafeInteger(rootGeneration)) {
    return null;
  }
  try {
    const rootPath = decodeURIComponent(decodeURIComponent(match[2]!));
    const segments = match[3]!.split("/").map((segment) => decodeURIComponent(segment));
    if (
      rootPath.length === 0 ||
      rootPath.length > 32_768 ||
      !/^(?:[A-Za-z]:[\\/]|\/|\\\\)/.test(rootPath) ||
      /[\u0000-\u001f\u007f]/.test(rootPath) ||
      /^[A-Za-z]:/.test(segments[0] ?? "") ||
      segments.some((segment) =>
        segment.length === 0 ||
        segment === "." ||
        segment === ".." ||
        /[\\/\u0000-\u001f\u007f]/.test(segment),
      )
    ) {
      return null;
    }
    const path = segments.join("/");
    return path.length > 32_768 ? null : { path, rootPath, rootGeneration };
  } catch {
    return null;
  }
}

export function workspaceParentPath(path: string): string {
  const separator = path.lastIndexOf("/");
  return separator < 0 ? "" : path.slice(0, separator);
}

export function joinWorkspacePath(parent: string, name: string): string {
  return parent.length === 0 ? name : `${parent}/${name}`;
}
