import type {
  CreatableWorkspaceEntryKind,
  CurrentDirectory,
  DirectoryBrowserSnapshot,
  SearchPathPosition,
  SearchPathSnapshot,
  WorkspaceClient,
  WorkspaceDeleteResult,
  WorkspaceDownload,
  WorkspaceEntry,
  WorkspaceFile,
  WorkspaceMoveResult,
  WorkspaceSnapshot,
  WorkspaceChange,
} from "./workspace-client";
import {
  WorkspaceClientError,
  joinWorkspacePath,
  workspaceParentPath,
} from "./workspace-client";

interface MockNode {
  readonly kind: CreatableWorkspaceEntryKind;
  content?: string;
  revision?: string;
}

const welcomeSource = `%% OpenMat workspace sample
x = 0;
while x < 3
    x = x + 1;
end
y = x * 10;
disp(y);`;

export class MockWorkspaceClient implements WorkspaceClient {
  readonly kind = "mock" as const;
  readonly label = "In-memory demo workspace";
  readonly #nodes = new Map<string, MockNode>([
    [
      "welcome.m",
      { kind: "file", content: welcomeSource, revision: "mock-revision-1" },
    ],
  ]);
  #connected = false;
  #revisionSequence = 1;
  #directory: CurrentDirectory = {
    path: "C:\\OpenMat\\demo-workspace",
    rootName: "demo-workspace",
    generation: 1,
  };
  readonly #directoryListeners = new Set<(directory: CurrentDirectory) => void>();
  readonly #workspaceListeners = new Set<(change: WorkspaceChange) => void>();
  readonly #searchPathListeners = new Set<
    (snapshot: SearchPathSnapshot) => void
  >();
  #searchPathGeneration = 0;
  #searchDirectories: string[] = [];

  async connect(): Promise<void> {
    this.#connected = true;
  }

  async disconnect(): Promise<void> {
    this.#connected = false;
  }

  subscribeConnectionLoss(): () => void {
    return () => {};
  }

  async currentDirectory(): Promise<CurrentDirectory> {
    this.#assertConnected();
    return this.#directory;
  }

  async changeDirectory(path: string): Promise<CurrentDirectory> {
    this.#assertConnected();
    const normalized = path.trim();
    if (normalized.length === 0) {
      throw new WorkspaceClientError(
        "workspace.invalidDirectory",
        "Current Folder requires a non-empty path.",
        "path",
      );
    }
    this.#directory = {
      path: normalized,
      rootName: normalized.split(/[\\/]/).filter(Boolean).at(-1) ?? "workspace",
      generation: this.#directory.generation + 1,
    };
    for (const listener of this.#directoryListeners) {
      listener(this.#directory);
    }
    return this.#directory;
  }

  async browseDirectories(path: string): Promise<DirectoryBrowserSnapshot> {
    this.#assertConnected();
    const normalized = path.trim();
    if (normalized.length === 0) {
      throw new WorkspaceClientError(
        "workspace.invalidDirectory",
        "Directory browser requires a non-empty path.",
        "path",
      );
    }
    const drive = /^[A-Za-z]:/.exec(normalized)?.[0] ?? null;
    return {
      path: normalized,
      parentPath: absoluteParentPath(normalized),
      roots: drive === null ? [{ name: "/", path: "/" }] : [{ name: drive, path: `${drive}\\` }],
      entries: [],
    };
  }

  onCurrentDirectoryChanged(
    listener: (directory: CurrentDirectory) => void,
  ): () => void {
    this.#directoryListeners.add(listener);
    return () => this.#directoryListeners.delete(listener);
  }

  onWorkspaceChanged(listener: (change: WorkspaceChange) => void): () => void {
    this.#workspaceListeners.add(listener);
    return () => this.#workspaceListeners.delete(listener);
  }

  async searchPath(): Promise<SearchPathSnapshot> {
    this.#assertConnected();
    return this.#searchPathSnapshot();
  }

  async addSearchPath(
    path: string,
    recursive: boolean,
    position: SearchPathPosition = "begin",
  ): Promise<SearchPathSnapshot> {
    this.#assertConnected();
    this.#assertDirectory(path);
    const relativePaths = recursive
      ? [
          path,
          ...[...this.#nodes.entries()]
            .filter(
              ([entryPath, node]) =>
                node.kind === "directory" && entryPath.startsWith(`${path}/`),
            )
            .map(([entryPath]) => entryPath),
        ]
      : [path];
    const additions = relativePaths.map((entryPath) =>
      this.#absoluteWorkspacePath(entryPath),
    );
    this.#searchDirectories = this.#searchDirectories.filter(
      (directory) => !additions.includes(directory),
    );
    this.#searchDirectories =
      position === "begin"
        ? [...additions, ...this.#searchDirectories]
        : [...this.#searchDirectories, ...additions];
    return this.#publishSearchPath();
  }

  async removeSearchPath(
    path: string,
    recursive: boolean,
  ): Promise<SearchPathSnapshot> {
    this.#assertConnected();
    this.#assertDirectory(path);
    const absolute = this.#absoluteWorkspacePath(path);
    this.#searchDirectories = this.#searchDirectories.filter(
      (directory) =>
        directory !== absolute &&
        !(recursive && directory.startsWith(`${absolute}\\`)),
    );
    return this.#publishSearchPath();
  }

  onSearchPathChanged(
    listener: (snapshot: SearchPathSnapshot) => void,
  ): () => void {
    this.#searchPathListeners.add(listener);
    return () => this.#searchPathListeners.delete(listener);
  }

  async list(path: string, recursive: boolean): Promise<WorkspaceSnapshot> {
    this.#assertConnected();
    this.#assertDirectory(path);
    const entries = [...this.#nodes.entries()]
      .filter(([entryPath]) => {
        const parent = workspaceParentPath(entryPath);
        return recursive
          ? path.length === 0 || entryPath.startsWith(`${path}/`)
          : parent === path;
      })
      .map(([entryPath, node]) => this.#entry(entryPath, node))
      .sort(compareEntries);
    return {
      rootName: this.#directory.rootName,
      rootPath: this.#directory.path,
      rootGeneration: this.#directory.generation,
      path,
      recursive,
      entries,
    };
  }

  async read(path: string): Promise<WorkspaceFile> {
    this.#assertConnected();
    const node = this.#requiredNode(path);
    if (node.kind !== "file") {
      throw new WorkspaceClientError(
        "workspace.notTextFile",
        `'${path}' is not a regular text file.`,
        "path",
      );
    }
    const content = node.content ?? "";
    return {
      path,
      content,
      revision: node.revision ?? this.#nextRevision(),
      size: new TextEncoder().encode(content).byteLength,
      rootGeneration: this.#directory.generation,
      rootPath: this.#directory.path,
    };
  }

  async prepareDownload(
    path: string,
    rootGeneration: number,
  ): Promise<WorkspaceDownload> {
    this.#assertConnected();
    if (rootGeneration !== this.#directory.generation) {
      throw new WorkspaceClientError(
        "workspace.unknownRootGeneration",
        "The requested Current Folder is no longer available.",
        "rootGeneration",
      );
    }
    const node = this.#requiredNode(path);
    if (node.kind !== "file") {
      throw new WorkspaceClientError(
        "workspace.notTextFile",
        `'${path}' is not a regular file.`,
        "path",
      );
    }
    const content = node.content ?? "";
    return {
      url: `data:application/octet-stream;charset=utf-8,${encodeURIComponent(content)}`,
      name: path.slice(path.lastIndexOf("/") + 1),
      size: new TextEncoder().encode(content).byteLength,
      expiresInSeconds: 60,
    };
  }

  async upload(
    path: string,
    file: File,
    rootGeneration: number,
    overwrite = false,
  ): Promise<WorkspaceEntry> {
    this.#assertConnected();
    if (rootGeneration !== this.#directory.generation) {
      throw new WorkspaceClientError(
        "workspace.unknownRootGeneration",
        "The requested Current Folder is no longer available.",
        "rootGeneration",
      );
    }
    this.#assertDirectory(workspaceParentPath(path));
    if (this.#nodes.has(path) && !overwrite) {
      throw new WorkspaceClientError(
        "workspace.conflict",
        `Workspace path '${path}' already exists. Nothing was overwritten.`,
        "path",
      );
    }
    const content = await file.text();
    const revision = this.#nextRevision();
    this.#nodes.set(path, { kind: "file", content, revision });
    const entry = this.#entry(path, this.#requiredNode(path));
    for (const listener of this.#workspaceListeners) {
      listener({ rootGeneration });
    }
    return { ...entry, size: file.size };
  }

  async write(
    path: string,
    content: string,
    expectedRevision: string,
    _rootGeneration: number,
  ): Promise<WorkspaceEntry> {
    this.#assertConnected();
    const node = this.#requiredNode(path);
    if (node.kind !== "file") {
      throw new WorkspaceClientError(
        "workspace.notTextFile",
        `'${path}' is not a regular text file.`,
        "path",
      );
    }
    if (node.revision !== expectedRevision) {
      throw new WorkspaceClientError(
        "workspace.revisionConflict",
        `'${path}' changed outside OpenMat. The editor content was not overwritten.`,
        "expectedRevision",
        { currentRevision: node.revision },
      );
    }
    node.content = content;
    node.revision = this.#nextRevision();
    return this.#entry(path, node);
  }

  async create(
    path: string,
    kind: CreatableWorkspaceEntryKind,
  ): Promise<WorkspaceEntry> {
    this.#assertConnected();
    if (this.#nodes.has(path)) {
      throw new WorkspaceClientError(
        "workspace.conflict",
        `Workspace path '${path}' already exists. Nothing was overwritten.`,
        "path",
      );
    }
    this.#assertDirectory(workspaceParentPath(path));
    const node: MockNode =
      kind === "file"
        ? { kind, content: "", revision: this.#nextRevision() }
        : { kind };
    this.#nodes.set(path, node);
    return this.#entry(path, node);
  }

  async rename(path: string, newName: string): Promise<WorkspaceMoveResult> {
    return this.move(path, joinWorkspacePath(workspaceParentPath(path), newName));
  }

  async move(path: string, targetPath: string): Promise<WorkspaceMoveResult> {
    this.#assertConnected();
    const node = this.#requiredNode(path);
    if (this.#nodes.has(targetPath)) {
      throw new WorkspaceClientError(
        "workspace.conflict",
        `Workspace path '${targetPath}' already exists. Nothing was overwritten.`,
        "targetPath",
      );
    }
    this.#assertDirectory(workspaceParentPath(targetPath));
    if (node.kind === "directory" && targetPath.startsWith(`${path}/`)) {
      throw new WorkspaceClientError(
        "workspace.invalidMove",
        "A directory cannot be moved inside itself.",
        "targetPath",
      );
    }
    const moved = [...this.#nodes.entries()].filter(
      ([entryPath]) => entryPath === path || entryPath.startsWith(`${path}/`),
    );
    for (const [entryPath] of moved) {
      this.#nodes.delete(entryPath);
    }
    for (const [entryPath, movedNode] of moved) {
      this.#nodes.set(`${targetPath}${entryPath.slice(path.length)}`, movedNode);
    }
    return { previousPath: path, entry: this.#entry(targetPath, node) };
  }

  async delete(path: string, recursive: boolean): Promise<WorkspaceDeleteResult> {
    this.#assertConnected();
    const node = this.#requiredNode(path);
    const descendants = [...this.#nodes.keys()].filter((entryPath) =>
      entryPath.startsWith(`${path}/`),
    );
    if (node.kind === "directory" && descendants.length > 0 && !recursive) {
      throw new WorkspaceClientError(
        "workspace.directoryNotEmpty",
        `Directory '${path}' is not empty; recursive confirmation is required.`,
        "recursive",
      );
    }
    this.#nodes.delete(path);
    if (recursive) {
      for (const descendant of descendants) {
        this.#nodes.delete(descendant);
      }
    }
    return { path, kind: node.kind, recursive };
  }

  #assertConnected(): void {
    if (!this.#connected) {
      throw new WorkspaceClientError(
        "workspace.disconnected",
        "The demo workspace is disconnected.",
      );
    }
  }

  #assertDirectory(path: string): void {
    if (path.length === 0) {
      return;
    }
    const node = this.#nodes.get(path);
    if (node?.kind !== "directory") {
      throw new WorkspaceClientError(
        "workspace.notDirectory",
        `Workspace path '${path}' is not a directory.`,
        "path",
      );
    }
  }

  #requiredNode(path: string): MockNode {
    const node = this.#nodes.get(path);
    if (node === undefined) {
      throw new WorkspaceClientError(
        "workspace.notFound",
        `Workspace path '${path}' does not exist.`,
        "path",
      );
    }
    return node;
  }

  #entry(path: string, node: MockNode): WorkspaceEntry {
    const content = node.kind === "file" ? (node.content ?? "") : null;
    return {
      name: path.slice(path.lastIndexOf("/") + 1),
      path,
      kind: node.kind,
      size:
        content === null ? null : new TextEncoder().encode(content).byteLength,
      revision: node.kind === "file" ? (node.revision ?? null) : null,
    };
  }

  #nextRevision(): string {
    this.#revisionSequence += 1;
    return `mock-revision-${this.#revisionSequence.toString()}`;
  }

  #absoluteWorkspacePath(path: string): string {
    return path.length === 0
      ? this.#directory.path
      : `${this.#directory.path.replace(/[\\/]$/, "")}\\${path.replaceAll("/", "\\")}`;
  }

  #searchPathSnapshot(): SearchPathSnapshot {
    const root = `${this.#directory.path.replace(/[\\/]$/, "")}\\`;
    return {
      generation: this.#searchPathGeneration,
      directories: this.#searchDirectories.map((path) => ({
        path,
        workspacePath:
          path === this.#directory.path
            ? ""
            : path.startsWith(root)
              ? path.slice(root.length).replaceAll("\\", "/")
              : null,
      })),
    };
  }

  #publishSearchPath(): SearchPathSnapshot {
    this.#searchPathGeneration += 1;
    const snapshot = this.#searchPathSnapshot();
    for (const listener of this.#searchPathListeners) {
      listener(snapshot);
    }
    return snapshot;
  }
}

function compareEntries(left: WorkspaceEntry, right: WorkspaceEntry): number {
  const order = { directory: 0, file: 1, other: 2 } as const;
  return (
    workspaceParentPath(left.path).localeCompare(workspaceParentPath(right.path)) ||
    order[left.kind] - order[right.kind] ||
    left.name.localeCompare(right.name)
  );
}

function absoluteParentPath(path: string): string | null {
  const trimmed = path.replace(/[\\/]+$/, "");
  if (/^[A-Za-z]:$/.test(trimmed) || trimmed.length === 0) {
    return null;
  }
  const separator = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  if (separator < 0) {
    return null;
  }
  if (separator === 2 && /^[A-Za-z]:/.test(trimmed)) {
    return `${trimmed.slice(0, 2)}\\`;
  }
  return separator === 0 ? "/" : trimmed.slice(0, separator);
}
