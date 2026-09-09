import {
  LSP_JSON_RPC_VERSION,
  OPENMAT_LSP_MAX_MESSAGE_BYTES,
  type JsonRpcId,
  type JsonRpcNotification,
  type JsonRpcResponse,
  type LspInitializeResult,
} from "./protocol";

export type LspConnectionState =
  | "disconnected"
  | "connecting"
  | "ready"
  | "closing";

export class LspClientError extends Error {
  readonly code: number | undefined;

  constructor(message: string, code?: number) {
    super(message);
    this.name = "LspClientError";
    this.code = code;
  }
}

export interface LspClient {
  readonly state: LspConnectionState;
  readonly initializeResult: LspInitializeResult | null;
  connect(): Promise<void>;
  disconnect(): Promise<void>;
  request<T>(method: string, params: unknown): Promise<T>;
  notify(method: string, params: unknown): boolean;
  onNotification(listener: (notification: JsonRpcNotification) => void): () => void;
  onStateChange(listener: (state: LspConnectionState) => void): () => void;
}

type PendingRequest = {
  readonly resolve: (value: unknown) => void;
  readonly reject: (reason: unknown) => void;
};

type WebSocketFactory = (url: string) => WebSocket;

const encodedLength = (text: string): number => new TextEncoder().encode(text).byteLength;
const requestKey = (id: JsonRpcId): string => `${typeof id}:${String(id)}`;
const SOCKET_CONNECTING = 0;
const SOCKET_OPEN = 1;
const GRACEFUL_SHUTDOWN_TIMEOUT_MS = 1000;

async function withTimeout<T>(promise: Promise<T>, milliseconds: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(
          () => reject(new LspClientError("OpenMat LSP shutdown timed out")),
          milliseconds,
        );
      }),
    ]);
  } finally {
    if (timer !== undefined) {
      clearTimeout(timer);
    }
  }
}

export class WebSocketLspClient implements LspClient {
  #socket: WebSocket | null = null;
  #state: LspConnectionState = "disconnected";
  #connectPromise: Promise<void> | null = null;
  #nextRequestId = 1;
  #pending = new Map<string, PendingRequest>();
  #notificationListeners = new Set<(notification: JsonRpcNotification) => void>();
  #stateListeners = new Set<(state: LspConnectionState) => void>();
  #manualClose = false;
  #initializeResult: LspInitializeResult | null = null;

  constructor(
    readonly url: string,
    private readonly createSocket: WebSocketFactory = (socketUrl) => new WebSocket(socketUrl),
  ) {}

  get state(): LspConnectionState {
    return this.#state;
  }

  get initializeResult(): LspInitializeResult | null {
    return this.#initializeResult;
  }

  connect(): Promise<void> {
    if (this.#state === "ready") {
      return Promise.resolve();
    }
    if (this.#connectPromise !== null) {
      return this.#connectPromise;
    }
    this.#manualClose = false;
    this.setState("connecting");
    this.#connectPromise = this.openAndInitialize()
      .catch((error: unknown) => {
        const failure =
          error instanceof Error ? error : new LspClientError("OpenMat LSP connection failed");
        this.resetConnection(failure);
        throw failure;
      })
      .finally(() => {
        this.#connectPromise = null;
      });
    return this.#connectPromise;
  }

  async disconnect(): Promise<void> {
    this.#manualClose = true;
    const socket = this.#socket;
    if (socket === null) {
      this.setState("disconnected");
      return;
    }
    this.setState("closing");
    if (socket.readyState === SOCKET_OPEN && this.#initializeResult !== null) {
      try {
        await withTimeout(this.sendRequest("shutdown", null), GRACEFUL_SHUTDOWN_TIMEOUT_MS);
        this.sendNotification("exit", null);
      } catch {
        // The peer may already be gone. Closing the socket still completes disposal.
      }
    }
    if (socket.readyState === SOCKET_OPEN || socket.readyState === SOCKET_CONNECTING) {
      socket.close(1000, "OpenMat LSP client disposed");
    }
    this.resetConnection(new LspClientError("OpenMat LSP client disconnected"));
  }

  async request<T>(method: string, params: unknown): Promise<T> {
    await this.connect();
    return (await this.sendRequest(method, params)) as T;
  }

  notify(method: string, params: unknown): boolean {
    if (this.#state !== "ready") {
      return false;
    }
    return this.sendNotification(method, params);
  }

  onNotification(listener: (notification: JsonRpcNotification) => void): () => void {
    this.#notificationListeners.add(listener);
    return () => this.#notificationListeners.delete(listener);
  }

  onStateChange(listener: (state: LspConnectionState) => void): () => void {
    this.#stateListeners.add(listener);
    return () => this.#stateListeners.delete(listener);
  }

  private async openAndInitialize(): Promise<void> {
    const socket = this.createSocket(this.url);
    this.#socket = socket;
    socket.onmessage = (event) => this.handleMessage(event);
    socket.onclose = () => {
      const error = new LspClientError(
        this.#manualClose
          ? "OpenMat LSP connection closed"
          : "OpenMat LSP connection was lost",
      );
      this.resetConnection(error);
    };
    socket.onerror = () => {
      if (this.#state !== "disconnected") {
        this.rejectPending(new LspClientError("OpenMat LSP WebSocket failed"));
      }
    };

    await new Promise<void>((resolve, reject) => {
      socket.onopen = () => resolve();
      const originalClose = socket.onclose;
      socket.onclose = (event) => {
        originalClose?.call(socket, event);
        reject(new LspClientError("OpenMat LSP WebSocket closed during initialization"));
      };
      socket.onerror = () => reject(new LspClientError("OpenMat LSP WebSocket failed to open"));
    });

    socket.onclose = () => {
      this.resetConnection(
        new LspClientError(
          this.#manualClose
            ? "OpenMat LSP connection closed"
            : "OpenMat LSP connection was lost",
        ),
      );
    };
    socket.onerror = () => {
      if (this.#state !== "disconnected") {
        this.rejectPending(new LspClientError("OpenMat LSP WebSocket failed"));
      }
    };

    const initializeResult = (await this.sendRequest("initialize", {
      processId: null,
      clientInfo: { name: "openmat-web", version: "0.1.0-alpha.0" },
      rootUri: null,
      capabilities: {
        general: { positionEncodings: ["utf-16"] },
        workspace: { workspaceEdit: { documentChanges: true, resourceOperations: ["rename"] } },
        textDocument: {
          synchronization: { didSave: false, dynamicRegistration: false },
          completion: { completionItem: { documentationFormat: ["markdown", "plaintext"] } },
          hover: { contentFormat: ["markdown", "plaintext"] },
          definition: {},
          references: {},
          rename: { prepareSupport: true },
          formatting: {},
          semanticTokens: {
            requests: { full: true },
            tokenTypes: [],
            tokenModifiers: [],
            formats: ["relative"],
          },
          codeAction: { codeActionLiteralSupport: { codeActionKind: { valueSet: ["quickfix"] } } },
          documentSymbol: { hierarchicalDocumentSymbolSupport: true },
        },
      },
      initializationOptions: { transport: "openmat-lsp-websocket-v1" },
    })) as LspInitializeResult;
    this.#initializeResult = initializeResult;
    this.sendNotification("initialized", {});
    this.setState("ready");
  }

  private sendRequest(method: string, params: unknown): Promise<unknown> {
    const id = this.#nextRequestId;
    this.#nextRequestId += 1;
    return new Promise((resolve, reject) => {
      this.#pending.set(requestKey(id), { resolve, reject });
      try {
        this.send({ jsonrpc: LSP_JSON_RPC_VERSION, id, method, params });
      } catch (error) {
        this.#pending.delete(requestKey(id));
        reject(error);
      }
    });
  }

  private sendNotification(method: string, params: unknown): boolean {
    try {
      this.send({ jsonrpc: LSP_JSON_RPC_VERSION, method, params });
      return true;
    } catch {
      return false;
    }
  }

  private send(message: unknown): void {
    const socket = this.#socket;
    if (socket === null || socket.readyState !== SOCKET_OPEN) {
      throw new LspClientError("OpenMat LSP WebSocket is not open");
    }
    const text = JSON.stringify(message);
    if (encodedLength(text) > OPENMAT_LSP_MAX_MESSAGE_BYTES) {
      throw new LspClientError("OpenMat LSP JSON-RPC message exceeds 1 MiB");
    }
    socket.send(text);
  }

  private handleMessage(event: MessageEvent): void {
    if (typeof event.data !== "string") {
      this.#socket?.close(1003, "OpenMat LSP requires text frames");
      return;
    }
    if (encodedLength(event.data) > OPENMAT_LSP_MAX_MESSAGE_BYTES) {
      this.#socket?.close(1009, "OpenMat LSP message exceeds 1 MiB");
      return;
    }
    let value: unknown;
    try {
      value = JSON.parse(event.data) as unknown;
    } catch {
      this.#socket?.close(1007, "OpenMat LSP sent invalid JSON");
      return;
    }
    if (typeof value !== "object" || value === null) {
      return;
    }
    const message = value as Partial<JsonRpcResponse & JsonRpcNotification>;
    if (message.jsonrpc !== LSP_JSON_RPC_VERSION) {
      return;
    }
    if ((typeof message.id === "number" || typeof message.id === "string") && ("result" in message || "error" in message)) {
      const pending = this.#pending.get(requestKey(message.id));
      if (pending === undefined) {
        return;
      }
      this.#pending.delete(requestKey(message.id));
      if (message.error !== undefined) {
        pending.reject(new LspClientError(message.error.message, message.error.code));
      } else {
        pending.resolve(message.result);
      }
      return;
    }
    if (typeof message.method === "string") {
      const notification = message as JsonRpcNotification;
      for (const listener of this.#notificationListeners) {
        listener(notification);
      }
    }
  }

  private rejectPending(error: Error): void {
    for (const pending of this.#pending.values()) {
      pending.reject(error);
    }
    this.#pending.clear();
  }

  private resetConnection(error: Error): void {
    this.rejectPending(error);
    this.#socket = null;
    this.#initializeResult = null;
    this.setState("disconnected");
  }

  private setState(state: LspConnectionState): void {
    if (this.#state === state) {
      return;
    }
    this.#state = state;
    for (const listener of this.#stateListeners) {
      listener(state);
    }
  }
}
