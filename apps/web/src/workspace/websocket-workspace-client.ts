import {
  WORKSPACE_PROTOCOL_V3,
  WorkspaceClientError,
  type CurrentDirectory,
  type CreatableWorkspaceEntryKind,
  type DirectoryBrowserEntry,
  type DirectoryBrowserSnapshot,
  type SearchPathPosition,
  type SearchPathSnapshot,
  type WorkspaceClient,
  type WorkspaceChange,
  type WorkspaceConnectionLossListener,
  type WorkspaceDeleteResult,
  type WorkspaceDownload,
  type WorkspaceEntry,
  type WorkspaceEntryKind,
  type WorkspaceFile,
  type WorkspaceMoveResult,
  type WorkspaceSnapshot,
  type WorkspaceUpload,
} from "./workspace-client";

const MAX_MESSAGE_BYTES = 1024 * 1024;
const SOCKET_CONNECTING = 0;
const SOCKET_OPEN = 1;

type WorkspaceRequestType =
  | "currentDirectory"
  | "changeDirectory"
  | "browseDirectories"
  | "searchPath"
  | "addSearchPath"
  | "removeSearchPath"
  | "list"
  | "read"
  | "prepareDownload"
  | "prepareUpload"
  | "write"
  | "create"
  | "rename"
  | "move"
  | "delete";

interface PendingRequest {
  readonly expectedType: WorkspaceRequestType;
  readonly resolve: (data: unknown) => void;
  readonly reject: (error: Error) => void;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function byteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

export function workspaceUrlFromKernelUrl(kernelUrl: string): string {
  const url = new URL(kernelUrl);
  if (url.protocol !== "ws:" && url.protocol !== "wss:") {
    throw new Error("Kernel URL must use ws:// or wss://");
  }
  url.pathname = "/workspace/v3";
  url.search = "";
  url.hash = "";
  return url.toString();
}

export function downloadUrlFromWorkspaceUrl(
  workspaceUrl: string,
  ticket: string,
): string {
  const url = new URL(workspaceUrl);
  url.protocol = url.protocol === "wss:" ? "https:" : "http:";
  url.pathname = `/workspace/download/${ticket}`;
  url.search = "";
  url.hash = "";
  return url.toString();
}

export function uploadUrlFromWorkspaceUrl(
  workspaceUrl: string,
  ticket: string,
): string {
  const url = new URL(workspaceUrl);
  url.protocol = url.protocol === "wss:" ? "https:" : "http:";
  url.pathname = `/workspace/upload/${ticket}`;
  url.search = "";
  url.hash = "";
  return url.toString();
}

export class WebSocketWorkspaceClient implements WorkspaceClient {
  readonly kind = "websocket" as const;
  readonly label = "Server workspace";
  readonly #url: string;
  readonly #webSocketFactory: (url: string) => WebSocket;
  readonly #fetch: typeof fetch;
  readonly #pending = new Map<string, PendingRequest>();
  readonly #directoryListeners = new Set<
    (directory: CurrentDirectory) => void
  >();
  readonly #workspaceListeners = new Set<(change: WorkspaceChange) => void>();
  readonly #searchPathListeners = new Set<
    (snapshot: SearchPathSnapshot) => void
  >();
  readonly #connectionLossListeners = new Set<WorkspaceConnectionLossListener>();
  #socket: WebSocket | null = null;
  #requestSequence = 0;

  constructor(
    kernelUrl: string,
    webSocketFactory: (url: string) => WebSocket = (url) => new WebSocket(url),
    fetcher: typeof fetch = (input, init) => fetch(input, init),
  ) {
    this.#url = workspaceUrlFromKernelUrl(kernelUrl);
    this.#webSocketFactory = webSocketFactory;
    this.#fetch = fetcher;
  }

  async connect(): Promise<void> {
    if (this.#socket !== null) {
      throw new WorkspaceClientError(
        "workspace.alreadyConnected",
        "Workspace client is already connected.",
      );
    }
    const socket = this.#webSocketFactory(this.#url);
    this.#socket = socket;
    socket.binaryType = "arraybuffer";
    await new Promise<void>((resolve, reject) => {
      let settled = false;
      socket.addEventListener("open", () => {
        if (this.#socket === socket && !settled) {
          settled = true;
          resolve();
        }
      });
      socket.addEventListener("message", (event) => {
        if (this.#socket === socket) {
          this.#handleMessage(event);
        }
      });
      socket.addEventListener("error", () => {
        const error = new WorkspaceClientError(
          "workspace.connectionFailed",
          "Could not connect to the server workspace.",
        );
        if (!settled) {
          settled = true;
          this.#socket = null;
          reject(error);
        } else {
          this.#fail(error);
        }
      });
      socket.addEventListener("close", (event) => {
        if (this.#socket !== socket) {
          return;
        }
        this.#socket = null;
        const error = new WorkspaceClientError(
          "workspace.connectionClosed",
          `Server workspace connection closed (${event.code.toString()}).`,
        );
        this.#rejectPending(error);
        if (!settled) {
          settled = true;
          reject(error);
        } else {
          this.#notifyConnectionLoss(error);
        }
      });
    });
  }

  async disconnect(): Promise<void> {
    const socket = this.#socket;
    this.#socket = null;
    this.#rejectPending(
      new WorkspaceClientError(
        "workspace.disconnected",
        "Workspace client disconnected.",
      ),
    );
    if (
      socket !== null &&
      (socket.readyState === SOCKET_CONNECTING || socket.readyState === SOCKET_OPEN)
    ) {
      socket.close(1000, "client disconnect");
    }
  }

  subscribeConnectionLoss(listener: WorkspaceConnectionLossListener): () => void {
    this.#connectionLossListeners.add(listener);
    return () => this.#connectionLossListeners.delete(listener);
  }

  async currentDirectory(): Promise<CurrentDirectory> {
    return this.#parseCurrentDirectory(
      await this.#request("currentDirectory", {}),
    );
  }

  async changeDirectory(path: string): Promise<CurrentDirectory> {
    return this.#parseCurrentDirectory(
      await this.#request("changeDirectory", { path }),
    );
  }

  async browseDirectories(path: string): Promise<DirectoryBrowserSnapshot> {
    const data = await this.#request("browseDirectories", { path });
    if (
      !isRecord(data) ||
      typeof data.path !== "string" ||
      data.path.length === 0 ||
      !(data.parentPath === null || typeof data.parentPath === "string") ||
      !Array.isArray(data.roots) ||
      !Array.isArray(data.entries)
    ) {
      throw this.#invalidResponse("Directory browser data is malformed.");
    }
    return {
      path: data.path,
      parentPath: data.parentPath,
      roots: data.roots.map((entry) => this.#parseDirectoryBrowserEntry(entry)),
      entries: data.entries.map((entry) => this.#parseDirectoryBrowserEntry(entry)),
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
    return this.#parseSearchPath(await this.#request("searchPath", {}));
  }

  async addSearchPath(
    path: string,
    recursive: boolean,
    position: SearchPathPosition = "begin",
  ): Promise<SearchPathSnapshot> {
    return this.#parseSearchPath(
      await this.#request("addSearchPath", { path, recursive, position }),
    );
  }

  async removeSearchPath(
    path: string,
    recursive: boolean,
  ): Promise<SearchPathSnapshot> {
    return this.#parseSearchPath(
      await this.#request("removeSearchPath", { path, recursive }),
    );
  }

  onSearchPathChanged(
    listener: (snapshot: SearchPathSnapshot) => void,
  ): () => void {
    this.#searchPathListeners.add(listener);
    return () => this.#searchPathListeners.delete(listener);
  }

  async list(path: string, recursive: boolean): Promise<WorkspaceSnapshot> {
    const data = await this.#request("list", { path, recursive });
    if (!isRecord(data)) {
      throw this.#invalidResponse("Workspace list data must be an object.");
    }
    const {
      rootName,
      rootPath,
      rootGeneration,
      entries,
      path: listedPath,
      recursive: listedRecursively,
    } = data;
    if (
      typeof rootName !== "string" ||
      rootName.length === 0 ||
      typeof rootPath !== "string" ||
      rootPath.length === 0 ||
      !isNonNegativeInteger(rootGeneration) ||
      typeof listedPath !== "string" ||
      typeof listedRecursively !== "boolean" ||
      listedPath !== path ||
      listedRecursively !== recursive ||
      !Array.isArray(entries)
    ) {
      throw this.#invalidResponse("Workspace list data is malformed.");
    }
    return {
      rootName,
      rootPath,
      rootGeneration,
      path: listedPath,
      recursive: listedRecursively,
      entries: entries.map((entry) => this.#parseEntry(entry)),
    };
  }

  async read(path: string): Promise<WorkspaceFile> {
    const data = await this.#request("read", { path });
    if (
      !isRecord(data) ||
      data.path !== path ||
      typeof data.content !== "string" ||
      typeof data.revision !== "string" ||
      data.revision.length === 0 ||
      !isNonNegativeInteger(data.size) ||
      !isNonNegativeInteger(data.rootGeneration) ||
      typeof data.rootPath !== "string" ||
      data.rootPath.length === 0
    ) {
      throw this.#invalidResponse("Workspace read data is malformed.");
    }
    return {
      path,
      content: data.content,
      revision: data.revision,
      size: data.size,
      rootGeneration: data.rootGeneration,
      rootPath: data.rootPath,
    };
  }

  async prepareDownload(
    path: string,
    rootGeneration: number,
  ): Promise<WorkspaceDownload> {
    const data = await this.#request("prepareDownload", { path, rootGeneration });
    if (
      !isRecord(data) ||
      typeof data.ticket !== "string" ||
      !/^[0-9a-f]{64}$/.test(data.ticket) ||
      typeof data.name !== "string" ||
      data.name.length === 0 ||
      !isNonNegativeInteger(data.size) ||
      !isNonNegativeInteger(data.expiresInSeconds) ||
      data.expiresInSeconds === 0
    ) {
      throw this.#invalidResponse("Workspace download data is malformed.");
    }
    return {
      url: downloadUrlFromWorkspaceUrl(this.#url, data.ticket),
      name: data.name,
      size: data.size,
      expiresInSeconds: data.expiresInSeconds,
    };
  }

  async upload(
    path: string,
    file: File,
    rootGeneration: number,
    overwrite = false,
  ): Promise<WorkspaceEntry> {
    const data = await this.#request("prepareUpload", {
      path,
      size: file.size,
      rootGeneration,
      overwrite,
    });
    const upload = this.#parseUpload(data);
    const response = await this.#fetch(upload.url, {
      method: "PUT",
      body: file,
      headers: { "Content-Type": "application/octet-stream" },
      referrerPolicy: "no-referrer",
    });
    const body: unknown = await response.json().catch(() => null);
    if (!response.ok) {
      const error = isRecord(body) && isRecord(body.error) ? body.error : null;
      throw new WorkspaceClientError(
        typeof error?.code === "string" ? error.code : "workspace.uploadFailed",
        typeof error?.message === "string"
          ? error.message
          : `Upload failed with HTTP ${response.status.toString()}.`,
        typeof error?.field === "string" ? error.field : undefined,
        isRecord(error?.details) ? error.details : undefined,
      );
    }
    if (!isRecord(body) || !("entry" in body)) {
      throw this.#invalidResponse("Workspace upload response is malformed.");
    }
    return this.#parseEntry(body.entry);
  }

  async write(
    path: string,
    content: string,
    expectedRevision: string,
    rootGeneration: number,
  ): Promise<WorkspaceEntry> {
    const data = await this.#request("write", {
      path,
      content,
      expectedRevision,
      rootGeneration,
    });
    return this.#parseEntryContainer(data, "write");
  }

  async create(
    path: string,
    kind: CreatableWorkspaceEntryKind,
  ): Promise<WorkspaceEntry> {
    const data = await this.#request("create", { path, kind });
    return this.#parseEntryContainer(data, "create");
  }

  async rename(path: string, newName: string, rootGeneration?: number): Promise<WorkspaceMoveResult> {
    const data = await this.#request("rename", { path, newName,
      ...(rootGeneration === undefined ? {} : { rootGeneration }),
    });
    return this.#parseMove(data, path, "rename");
  }

  async move(path: string, targetPath: string): Promise<WorkspaceMoveResult> {
    const data = await this.#request("move", { path, targetPath });
    return this.#parseMove(data, path, "move");
  }

  async delete(path: string, recursive: boolean): Promise<WorkspaceDeleteResult> {
    const data = await this.#request("delete", {
      path,
      recursive,
      confirmPath: recursive ? path : null,
    });
    if (
      !isRecord(data) ||
      data.path !== path ||
      data.recursive !== recursive ||
      !isEntryKind(data.kind)
    ) {
      throw this.#invalidResponse("Workspace delete data is malformed.");
    }
    return { path, recursive, kind: data.kind };
  }

  #request(
    type: WorkspaceRequestType,
    params: Record<string, unknown>,
  ): Promise<unknown> {
    const socket = this.#socket;
    if (socket === null || socket.readyState !== SOCKET_OPEN) {
      return Promise.reject(
        new WorkspaceClientError(
          "workspace.disconnected",
          "Server workspace is not connected.",
        ),
      );
    }
    this.#requestSequence += 1;
    const requestId = `workspace-${this.#requestSequence.toString().padStart(4, "0")}`;
    const frame = JSON.stringify({
      protocol: WORKSPACE_PROTOCOL_V3,
      requestId,
      request: { type, params },
    });
    if (byteLength(frame) > MAX_MESSAGE_BYTES) {
      return Promise.reject(
        new WorkspaceClientError(
          "workspace.messageTooLarge",
          "Workspace request exceeds the 1 MiB limit.",
        ),
      );
    }
    return new Promise((resolve, reject) => {
      this.#pending.set(requestId, { expectedType: type, resolve, reject });
      try {
        socket.send(frame);
      } catch {
        this.#pending.delete(requestId);
        reject(
          new WorkspaceClientError(
            "workspace.sendFailed",
            "Could not send the workspace request.",
          ),
        );
      }
    });
  }

  #handleMessage(event: MessageEvent<unknown>): void {
    if (typeof event.data !== "string" || byteLength(event.data) > MAX_MESSAGE_BYTES) {
      this.#fail(this.#invalidResponse("Workspace server sent an invalid frame."));
      return;
    }
    let value: unknown;
    try {
      value = JSON.parse(event.data) as unknown;
    } catch {
      this.#fail(this.#invalidResponse("Workspace server sent invalid JSON."));
      return;
    }
    if (
      !isRecord(value) ||
      value.protocol !== WORKSPACE_PROTOCOL_V3
    ) {
      this.#fail(this.#invalidResponse("Workspace response envelope is malformed."));
      return;
    }
    if ("event" in value) {
      const event = value.event;
      if (!isRecord(event) || !("data" in event)) {
        this.#fail(this.#invalidResponse("Workspace event is malformed."));
        return;
      }
      if (event.type === "workspaceChanged") {
        if (
          !isRecord(event.data) ||
          !isNonNegativeInteger(event.data.rootGeneration)
        ) {
          this.#fail(this.#invalidResponse("Workspace change event is malformed."));
          return;
        }
        for (const listener of this.#workspaceListeners) {
          listener({ rootGeneration: event.data.rootGeneration });
        }
        return;
      }
      if (event.type === "currentDirectoryChanged") {
        let directory: CurrentDirectory;
        try {
          directory = this.#parseCurrentDirectory(event.data);
        } catch (error: unknown) {
          this.#fail(
            error instanceof WorkspaceClientError
              ? error
              : this.#invalidResponse("Workspace directory event is malformed."),
          );
          return;
        }
        for (const listener of this.#directoryListeners) {
          listener(directory);
        }
        return;
      }
      if (event.type === "searchPathChanged") {
        let snapshot: SearchPathSnapshot;
        try {
          snapshot = this.#parseSearchPath(event.data);
        } catch (error: unknown) {
          this.#fail(
            error instanceof WorkspaceClientError
              ? error
              : this.#invalidResponse("Search path event is malformed."),
          );
          return;
        }
        for (const listener of this.#searchPathListeners) {
          listener(snapshot);
        }
        return;
      }
      this.#fail(this.#invalidResponse("Workspace event type is unsupported."));
      return;
    }
    if (
      typeof value.requestId !== "string" ||
      typeof value.ok !== "boolean"
    ) {
      this.#fail(this.#invalidResponse("Workspace response envelope is malformed."));
      return;
    }
    const pending = this.#pending.get(value.requestId);
    if (pending === undefined) {
      this.#fail(this.#invalidResponse("Workspace response has an unknown requestId."));
      return;
    }
    this.#pending.delete(value.requestId);
    if (!value.ok) {
      const error = value.error;
      if (
        !isRecord(error) ||
        typeof error.code !== "string" ||
        typeof error.message !== "string" ||
        (error.field !== undefined && typeof error.field !== "string") ||
        (error.details !== undefined && !isRecord(error.details))
      ) {
        pending.reject(this.#invalidResponse("Workspace error response is malformed."));
        return;
      }
      pending.reject(
        new WorkspaceClientError(
          error.code,
          error.message,
          error.field as string | undefined,
          error.details as Readonly<Record<string, unknown>> | undefined,
        ),
      );
      return;
    }
    const result = value.result;
    if (
      !isRecord(result) ||
      result.type !== pending.expectedType ||
      !("data" in result)
    ) {
      pending.reject(this.#invalidResponse("Workspace result does not match its request."));
      return;
    }
    pending.resolve(result.data);
  }

  #parseEntryContainer(value: unknown, operation: string): WorkspaceEntry {
    if (!isRecord(value) || !("entry" in value)) {
      throw this.#invalidResponse(`Workspace ${operation} data is malformed.`);
    }
    return this.#parseEntry(value.entry);
  }

  #parseUpload(value: unknown): WorkspaceUpload {
    if (
      !isRecord(value) ||
      typeof value.ticket !== "string" ||
      !/^[0-9a-f]{64}$/.test(value.ticket) ||
      typeof value.name !== "string" ||
      value.name.length === 0 ||
      !isNonNegativeInteger(value.size) ||
      !isNonNegativeInteger(value.expiresInSeconds) ||
      value.expiresInSeconds === 0
    ) {
      throw this.#invalidResponse("Workspace upload data is malformed.");
    }
    return {
      url: uploadUrlFromWorkspaceUrl(this.#url, value.ticket),
      name: value.name,
      size: value.size,
      expiresInSeconds: value.expiresInSeconds,
    };
  }

  #parseCurrentDirectory(value: unknown): CurrentDirectory {
    if (
      !isRecord(value) ||
      typeof value.path !== "string" ||
      value.path.length === 0 ||
      typeof value.rootName !== "string" ||
      value.rootName.length === 0 ||
      !isNonNegativeInteger(value.generation)
    ) {
      throw this.#invalidResponse("Current Folder data is malformed.");
    }
    return {
      path: value.path,
      rootName: value.rootName,
      generation: value.generation,
    };
  }

  #parseSearchPath(value: unknown): SearchPathSnapshot {
    if (
      !isRecord(value) ||
      !isNonNegativeInteger(value.generation) ||
      !Array.isArray(value.directories)
    ) {
      throw this.#invalidResponse("Search path data is malformed.");
    }
    return {
      generation: value.generation,
      directories: value.directories.map((directory) => {
        if (
          !isRecord(directory) ||
          typeof directory.path !== "string" ||
          directory.path.length === 0 ||
          !(
            directory.workspacePath === null ||
            typeof directory.workspacePath === "string"
          )
        ) {
          throw this.#invalidResponse("Search path directory is malformed.");
        }
        return {
          path: directory.path,
          workspacePath: directory.workspacePath,
        };
      }),
    };
  }

  #parseMove(
    value: unknown,
    previousPath: string,
    operation: string,
  ): WorkspaceMoveResult {
    if (
      !isRecord(value) ||
      value.previousPath !== previousPath ||
      !("entry" in value)
    ) {
      throw this.#invalidResponse(`Workspace ${operation} data is malformed.`);
    }
    return { previousPath, entry: this.#parseEntry(value.entry) };
  }

  #parseEntry(value: unknown): WorkspaceEntry {
    if (
      !isRecord(value) ||
      typeof value.name !== "string" ||
      typeof value.path !== "string" ||
      !isEntryKind(value.kind) ||
      !(value.size === null || isNonNegativeInteger(value.size)) ||
      !(value.revision === null || typeof value.revision === "string")
    ) {
      throw this.#invalidResponse("Workspace entry is malformed.");
    }
    return {
      name: value.name,
      path: value.path,
      kind: value.kind,
      size: value.size,
      revision: value.revision,
    };
  }

  #parseDirectoryBrowserEntry(value: unknown): DirectoryBrowserEntry {
    if (
      !isRecord(value) ||
      typeof value.name !== "string" ||
      value.name.length === 0 ||
      typeof value.path !== "string" ||
      value.path.length === 0
    ) {
      throw this.#invalidResponse("Directory browser entry is malformed.");
    }
    return { name: value.name, path: value.path };
  }

  #invalidResponse(message: string): WorkspaceClientError {
    return new WorkspaceClientError("workspace.invalidResponse", message);
  }

  #fail(error: WorkspaceClientError): void {
    const socket = this.#socket;
    this.#socket = null;
    this.#rejectPending(error);
    if (socket !== null) {
      this.#notifyConnectionLoss(error);
    }
    if (
      socket !== null &&
      (socket.readyState === SOCKET_CONNECTING || socket.readyState === SOCKET_OPEN)
    ) {
      socket.close(1002, "workspace.invalidResponse");
    }
  }

  #rejectPending(error: Error): void {
    for (const pending of this.#pending.values()) {
      pending.reject(error);
    }
    this.#pending.clear();
  }

  #notifyConnectionLoss(error: Error): void {
    for (const listener of this.#connectionLossListeners) {
      listener(error);
    }
  }
}

function isEntryKind(value: unknown): value is WorkspaceEntryKind {
  return value === "file" || value === "directory" || value === "other";
}

function isNonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
