import {
  KERNEL_PROTOCOL_V0,
  KERNEL_PROTOCOL_V1,
  KERNEL_PROTOCOL_V2,
  KERNEL_PROTOCOL_V3,
  MAX_KERNEL_FRAME_BYTES,
  V2_HARD_LIMITS,
  hasValidV2Capabilities,
  parseBootstrapKernelServerMessage,
  parseKernelServerMessage,
  validateBootstrapInitializeParams,
  validateInspectRequestParams,
  validateSetVariableElementParams,
  type Capabilities,
  type KernelProtocol,
  type KernelRequestType,
  type KernelRequest,
  type KernelResponse,
  type PreviewDecodeLimits,
  type ResponseFor,
  type V2Capabilities,
} from "../protocol/kernel-v2";
import {
  hasValidV1Capabilities,
  type V1Capabilities,
} from "../protocol/kernel-v1";
import type {
  KernelConnectionLossListener,
  KernelEventListener,
  KernelTransport,
} from "./kernel-transport";

export const DEFAULT_MAX_MESSAGE_BYTES = MAX_KERNEL_FRAME_BYTES;
const SOCKET_CONNECTING = 0;
const SOCKET_OPEN = 1;

export type KernelTransportErrorCode =
  | "connection_failed"
  | "connection_closed"
  | "disconnected"
  | "duplicate_message_id"
  | "invalid_message"
  | "message_too_large"
  | "request_mismatch"
  | "send_failed"
  | "unexpected_frame_type"
  | "unexpected_response";

export class KernelTransportError extends Error {
  constructor(
    readonly code: KernelTransportErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "KernelTransportError";
  }
}

export interface WebSocketKernelTransportOptions {
  readonly maxMessageBytes?: number;
  readonly webSocketFactory?: (url: string) => WebSocket;
}

interface PendingRequest {
  readonly requestType: KernelRequestType;
  readonly offeredCapabilities?: V1Capabilities | V2Capabilities | undefined;
  readonly offeredProtocols?: readonly string[] | undefined;
  readonly previewLimits?: PreviewDecodeLimits | undefined;
  readonly requestMaxElements?: number | undefined;
  readonly resolve: (response: KernelResponse) => void;
  readonly reject: (error: KernelTransportError) => void;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function byteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function closeDescription(event: CloseEvent): string {
  const suffix = event.reason.length > 0 ? `: ${event.reason}` : "";
  return `Kernel WebSocket closed (${event.code.toString()})${suffix}`;
}

export class WebSocketKernelTransport implements KernelTransport {
  readonly kind = "websocket" as const;
  readonly label = "Rust kernel";
  readonly #url: string;
  readonly #maxMessageBytes: number;
  readonly #webSocketFactory: (url: string) => WebSocket;
  readonly #listeners = new Set<KernelEventListener>();
  readonly #connectionLossListeners = new Set<KernelConnectionLossListener>();
  readonly #pending = new Map<string, PendingRequest>();
  readonly #sentMessageIds = new Set<string>();
  readonly #receivedMessageIds = new Set<string>();
  #socket: WebSocket | null = null;
  #sessionId: string | null = null;
  #terminalError: KernelTransportError | null = null;
  #initializeRequestId: string | null = null;
  #negotiatedProtocol: KernelProtocol | null = null;
  #capabilities: Capabilities | null = null;

  constructor(url: string, options: WebSocketKernelTransportOptions = {}) {
    const parsedUrl = new URL(url);
    if (parsedUrl.protocol !== "ws:" && parsedUrl.protocol !== "wss:") {
      throw new Error("Kernel WebSocket URL must use ws:// or wss://");
    }

    const maxMessageBytes = options.maxMessageBytes ?? DEFAULT_MAX_MESSAGE_BYTES;
    if (
      !Number.isSafeInteger(maxMessageBytes) ||
      maxMessageBytes <= 0 ||
      maxMessageBytes > MAX_KERNEL_FRAME_BYTES
    ) {
      throw new Error(
        `maxMessageBytes must be a positive safe integer no greater than ${MAX_KERNEL_FRAME_BYTES.toString()}`,
      );
    }

    this.#url = parsedUrl.toString();
    this.#maxMessageBytes = maxMessageBytes;
    this.#webSocketFactory =
      options.webSocketFactory ?? ((socketUrl) => new WebSocket(socketUrl));
  }

  async connect(sessionId: string): Promise<void> {
    if (sessionId.length === 0) {
      throw new Error("Kernel session ID must not be empty");
    }
    if (this.#socket !== null) {
      throw new Error("Kernel WebSocket transport is already connected");
    }

    this.#terminalError = null;
    this.#initializeRequestId = null;
    this.#negotiatedProtocol = null;
    this.#capabilities = null;
    this.#sentMessageIds.clear();
    this.#receivedMessageIds.clear();
    const socket = this.#webSocketFactory(this.#url);
    this.#socket = socket;
    this.#sessionId = sessionId;
    socket.binaryType = "arraybuffer";

    await new Promise<void>((resolve, reject) => {
      let opened = false;
      let settled = false;

      const rejectConnection = (error: KernelTransportError): void => {
        if (!settled) {
          settled = true;
          reject(error);
        }
      };

      socket.addEventListener("open", () => {
        if (this.#socket !== socket) {
          return;
        }
        opened = true;
        settled = true;
        resolve();
      });
      socket.addEventListener("message", (event) => {
        if (this.#socket === socket) {
          this.#handleMessage(event);
        }
      });
      socket.addEventListener("error", () => {
        const error = new KernelTransportError(
          "connection_failed",
          "Kernel WebSocket connection failed",
        );
        if (!opened) {
          this.#clearSocket(socket);
          if (
            socket.readyState === SOCKET_CONNECTING ||
            socket.readyState === SOCKET_OPEN
          ) {
            socket.close(1011, error.code);
          }
          rejectConnection(error);
        } else {
          this.#failConnection(socket, error, 1011);
        }
      });
      socket.addEventListener("close", (event) => {
        if (this.#socket !== socket) {
          return;
        }
        const error = new KernelTransportError(
          opened ? "connection_closed" : "connection_failed",
          closeDescription(event),
        );
        this.#terminalError = error;
        this.#clearSocket(socket);
        this.#rejectPending(error);
        if (opened) {
          this.#notifyConnectionLoss(error);
        }
        rejectConnection(error);
      });
    });
  }

  async disconnect(): Promise<void> {
    const socket = this.#socket;
    if (socket === null) {
      return;
    }

    const error = new KernelTransportError(
      "disconnected",
      "Kernel WebSocket transport disconnected",
    );
    this.#clearSocket(socket);
    this.#rejectPending(error);
    if (
      socket.readyState === SOCKET_CONNECTING ||
      socket.readyState === SOCKET_OPEN
    ) {
      socket.close(1000, "client disconnect");
    }
  }

  subscribe(listener: KernelEventListener): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  subscribeConnectionLoss(listener: KernelConnectionLossListener): () => void {
    this.#connectionLossListeners.add(listener);
    return () => {
      this.#connectionLossListeners.delete(listener);
    };
  }

  request<TRequest extends KernelRequest>(
    request: TRequest,
  ): Promise<ResponseFor<TRequest>> {
    const socket = this.#socket;
    if (socket === null || socket.readyState !== SOCKET_OPEN) {
      return Promise.reject(
        this.#terminalError ??
          new KernelTransportError(
            "disconnected",
            "Kernel WebSocket transport is not connected",
          ),
      );
    }
    if (
      request.kind !== "request" ||
      request.sessionId !== this.#sessionId ||
      request.messageId.length === 0
    ) {
      return Promise.reject(
        new KernelTransportError(
          "request_mismatch",
          "Request envelope does not match the active kernel session",
        ),
      );
    }
    const requestType = request.request.type;
    const isInitialize = requestType === "initialize";
    if (isInitialize) {
      const validationError = validateBootstrapInitializeParams(
        request.request.params,
      );
      if (
        request.protocol !== KERNEL_PROTOCOL_V0 ||
        this.#initializeRequestId !== null ||
        this.#negotiatedProtocol !== null ||
        this.#pending.size !== 0 ||
        validationError !== null
      ) {
        return Promise.reject(
          new KernelTransportError(
            "request_mismatch",
            validationError ??
              "Initialize must be the sole v0 bootstrap request for the session",
          ),
        );
      }
    } else {
      const expectedProtocol =
        this.#negotiatedProtocol ?? KERNEL_PROTOCOL_V0;
      if (
        this.#initializeRequestId !== null ||
        request.protocol !== expectedProtocol
      ) {
        return Promise.reject(
          new KernelTransportError(
            "request_mismatch",
            "Request protocol does not match the active kernel session",
          ),
        );
      }
    }
    if (this.#sentMessageIds.has(request.messageId)) {
      return Promise.reject(
        new KernelTransportError(
          "duplicate_message_id",
          `Request messageId ${request.messageId} was already used in this session`,
        ),
      );
    }

    const encoded = JSON.stringify(request);
    if (byteLength(encoded) > this.#maxMessageBytes) {
      return Promise.reject(
        new KernelTransportError(
          "message_too_large",
          `Kernel request exceeds the ${this.#maxMessageBytes.toString()} byte limit`,
        ),
      );
    }

    let offeredCapabilities: V1Capabilities | V2Capabilities | undefined;
    if (
      isInitialize &&
      request.request.params.supportedProtocols.some(
        (protocol) =>
          protocol === KERNEL_PROTOCOL_V2 || protocol === KERNEL_PROTOCOL_V3,
      ) &&
      hasValidV2Capabilities(request.request.params.capabilities)
    ) {
      offeredCapabilities = request.request.params.capabilities;
    } else if (
      isInitialize &&
      request.request.params.supportedProtocols.some(
        (protocol) => protocol === KERNEL_PROTOCOL_V1,
      ) &&
      hasValidV1Capabilities(request.request.params.capabilities)
    ) {
      offeredCapabilities = request.request.params.capabilities;
    }
    const offeredProtocols = isInitialize
      ? request.request.params.supportedProtocols
      : undefined;
    let previewLimits: PreviewDecodeLimits | undefined;
    let requestMaxElements: number | undefined;
    if (
      requestType === "inspect" &&
      this.#negotiatedProtocol === KERNEL_PROTOCOL_V1 &&
      this.#capabilities !== null &&
      hasValidV1Capabilities(this.#capabilities)
    ) {
      const maxElements = request.request.params.maxElements;
      if (
        !Number.isSafeInteger(maxElements) ||
        maxElements <= 0 ||
        maxElements > this.#capabilities.maxPreviewElements
      ) {
        return Promise.reject(
          new KernelTransportError(
            "request_mismatch",
            "Inspect maxElements exceeds the negotiated v1 limit",
          ),
        );
      }
      previewLimits = {
        ...V2_HARD_LIMITS,
        maxPreviewElements: maxElements,
        maxStringElementCodeUnits:
          this.#capabilities.maxStringElementCodeUnits,
        maxPreviewCodeUnits: this.#capabilities.maxPreviewCodeUnits,
      };
      requestMaxElements = maxElements;
    } else if (
      requestType === "inspect" &&
      (this.#negotiatedProtocol === KERNEL_PROTOCOL_V2 ||
        this.#negotiatedProtocol === KERNEL_PROTOCOL_V3) &&
      this.#capabilities !== null &&
      hasValidV2Capabilities(this.#capabilities)
    ) {
      previewLimits = {
        maxPreviewElements: this.#capabilities.maxPreviewElements,
        maxStringElementCodeUnits:
          this.#capabilities.maxStringElementCodeUnits,
        maxPreviewCodeUnits: this.#capabilities.maxPreviewCodeUnits,
        maxAggregateNodes: this.#capabilities.maxAggregateNodes,
        maxAggregateElements: this.#capabilities.maxAggregateElements,
        maxAggregateDepth: this.#capabilities.maxAggregateDepth,
      };
      const validationError = validateInspectRequestParams(
        request.request.params,
        previewLimits,
      );
      if (validationError !== null) {
        return Promise.reject(
          new KernelTransportError("request_mismatch", validationError),
        );
      }
      requestMaxElements = request.request.params.maxElements;
    } else if (requestType === "setVariableElement") {
      const validationError = validateSetVariableElementParams(
        request.request.params,
      );
      if (
        this.#negotiatedProtocol !== KERNEL_PROTOCOL_V3 ||
        validationError !== null
      ) {
        return Promise.reject(
          new KernelTransportError(
            "request_mismatch",
            validationError ??
              "Variable element mutation requires openmat-kernel-v3",
          ),
        );
      }
    }

    return new Promise<ResponseFor<TRequest>>((resolve, reject) => {
      this.#sentMessageIds.add(request.messageId);
      if (isInitialize) {
        this.#initializeRequestId = request.messageId;
      }
      this.#pending.set(request.messageId, {
        requestType,
        offeredCapabilities,
        offeredProtocols,
        previewLimits,
        requestMaxElements,
        resolve: (response) => resolve(response as ResponseFor<TRequest>),
        reject,
      });
      try {
        socket.send(encoded);
      } catch {
        this.#sentMessageIds.delete(request.messageId);
        this.#pending.delete(request.messageId);
        if (this.#initializeRequestId === request.messageId) {
          this.#initializeRequestId = null;
        }
        reject(
          new KernelTransportError(
            "send_failed",
            "Failed to send the kernel request",
          ),
        );
      }
    });
  }

  #handleMessage(event: MessageEvent<unknown>): void {
    const socket = this.#socket;
    if (socket === null) {
      return;
    }
    if (typeof event.data !== "string") {
      this.#failConnection(
        socket,
        new KernelTransportError(
          "unexpected_frame_type",
          "Kernel WebSocket sent a non-text frame",
        ),
        1003,
      );
      return;
    }
    if (byteLength(event.data) > this.#maxMessageBytes) {
      this.#failConnection(
        socket,
        new KernelTransportError(
          "message_too_large",
          `Kernel message exceeds the ${this.#maxMessageBytes.toString()} byte limit`,
        ),
        1009,
      );
      return;
    }

    let decoded: unknown;
    try {
      decoded = JSON.parse(event.data) as unknown;
    } catch {
      this.#failConnection(
        socket,
        new KernelTransportError("invalid_message", "Kernel sent invalid JSON"),
        1002,
      );
      return;
    }

    const pendingForFrame =
      isRecord(decoded) &&
      decoded.kind === "response" &&
      typeof decoded.replyTo === "string"
        ? this.#pending.get(decoded.replyTo)
        : undefined;
    const message =
      pendingForFrame?.requestType === "initialize"
        ? parseBootstrapKernelServerMessage(
            decoded,
            pendingForFrame.offeredCapabilities,
            pendingForFrame.offeredProtocols,
          )
        : parseKernelServerMessage(
            decoded,
            this.#negotiatedProtocol ?? KERNEL_PROTOCOL_V0,
            pendingForFrame?.previewLimits,
            pendingForFrame?.requestMaxElements,
          );
    if (message === null) {
      this.#failConnection(
        socket,
        new KernelTransportError(
          "invalid_message",
          "Kernel sent an invalid protocol message",
        ),
        1002,
      );
      return;
    }

    const envelope =
      message.messageType === "response" ? message.response : message.event;
    if (envelope.sessionId !== this.#sessionId) {
      this.#failConnection(
        socket,
        new KernelTransportError(
          "invalid_message",
          "Kernel sent a cross-session protocol message",
        ),
        1002,
      );
      return;
    }
    if (this.#receivedMessageIds.has(envelope.messageId)) {
      this.#failConnection(
        socket,
        new KernelTransportError(
          "duplicate_message_id",
          `Kernel reused messageId ${envelope.messageId}`,
        ),
        1002,
      );
      return;
    }
    this.#receivedMessageIds.add(envelope.messageId);

    if (message.messageType === "unknownEvent") {
      return;
    }
    if (message.messageType === "event") {
      for (const listener of this.#listeners) {
        listener(message.event);
      }
      return;
    }

    const pending = this.#pending.get(message.response.replyTo);
    if (pending === undefined) {
      this.#failConnection(
        socket,
        new KernelTransportError(
          "unexpected_response",
          `Kernel response has unknown replyTo ${message.response.replyTo}`,
        ),
        1002,
      );
      return;
    }
    if (
      message.response.ok &&
      message.response.result.type !== pending.requestType
    ) {
      this.#failConnection(
        socket,
        new KernelTransportError(
          "request_mismatch",
          `Kernel response type ${message.response.result.type} does not match ${pending.requestType}`,
        ),
        1002,
      );
      return;
    }

    if (pending.requestType === "initialize") {
      this.#initializeRequestId = null;
      if (
        message.response.ok &&
        message.response.result.type === "initialize"
      ) {
        this.#negotiatedProtocol =
          message.response.result.data.negotiatedProtocol;
        this.#capabilities = message.response.result.data.capabilities;
      }
    }

    this.#pending.delete(message.response.replyTo);
    pending.resolve(message.response);
  }

  #failConnection(
    socket: WebSocket,
    error: KernelTransportError,
    closeCode: number,
  ): void {
    this.#terminalError = error;
    this.#clearSocket(socket);
    this.#rejectPending(error);
    this.#notifyConnectionLoss(error);
    if (
      socket.readyState === SOCKET_CONNECTING ||
      socket.readyState === SOCKET_OPEN
    ) {
      socket.close(closeCode, error.code);
    }
  }

  #notifyConnectionLoss(error: KernelTransportError): void {
    for (const listener of this.#connectionLossListeners) {
      listener(error);
    }
  }

  #clearSocket(socket: WebSocket): void {
    if (this.#socket === socket) {
      this.#socket = null;
      this.#sessionId = null;
      this.#initializeRequestId = null;
      this.#negotiatedProtocol = null;
      this.#capabilities = null;
    }
  }

  #rejectPending(error: KernelTransportError): void {
    for (const pending of this.#pending.values()) {
      pending.reject(error);
    }
    this.#pending.clear();
    this.#initializeRequestId = null;
  }
}
