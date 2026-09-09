import {
  GRAPHICS_ENDPOINT_V1,
  GRAPHICS_ENDPOINT_V2,
  GRAPHICS_ENDPOINT_V3,
  GRAPHICS_ENDPOINT_V4,
  GRAPHICS_PROTOCOL_V2,
  GRAPHICS_PROTOCOL_V3,
  GRAPHICS_PROTOCOL_V4,
  MAX_BINARY_FRAME_BYTES,
  MAX_BUFFER_BYTES,
  MAX_OBJECTS,
  MAX_RESIDENT_BYTES,
  MAX_TEXT_FRAME_BYTES,
  GraphicsV1Error,
  applyFigureDelta,
  decodeOmgpFrame,
  graphicsWebSocketUrl,
  isRecord,
  parseDataRef,
  parseFigureDelta,
  parseFigureSnapshot,
  parseTextEnvelope,
  type DataRef,
  type FigureDiscovery,
  type FigureSnapshot,
  type GraphicsProtocol,
} from "./graphics-v1";

const SOCKET_OPEN = 1;
const CLIENT_NAME = "openmat-web";
const CLIENT_VERSION = "0.1.0";
// Browser WebSocket.close() only permits 1000 or application-defined
// 3000-4999 codes. RFC-reserved peer codes such as 1002/1008 throw before a
// close frame can be sent when used by JavaScript clients.
const CLIENT_CLOSE_PROTOCOL_ERROR = 4002;
const CLIENT_CLOSE_POLICY_ERROR = 4008;

export type GraphicsConnectionPhase =
  | "idle"
  | "connecting"
  | "initializing"
  | "attached"
  | "reconnecting"
  | "closed"
  | "error";

export interface GraphicsConnectionStatus {
  readonly phase: GraphicsConnectionPhase;
  readonly reconnectAttempt: number;
  readonly message: string | null;
}

export interface RenderableFigure {
  readonly figureId: string;
  readonly revision: number;
  readonly snapshot: FigureSnapshot;
  readonly buffers: ReadonlyMap<string, Uint8Array>;
}

export type GraphicsClientEvent =
  | { readonly type: "connection"; readonly status: GraphicsConnectionStatus }
  | { readonly type: "figureReady"; readonly figure: RenderableFigure }
  | {
      readonly type: "figureClosed";
      readonly figureId: string;
      readonly closedRevision: number;
    }
  | {
      readonly type: "figureCloseFailed";
      readonly figureId: string;
      readonly category: string;
      readonly message: string;
    };

export interface GraphicsSocket {
  binaryType: BinaryType;
  readonly readyState: number;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent<unknown>) => void) | null;
  onerror: ((event: Event) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  send(data: string | ArrayBufferLike | Blob | ArrayBufferView): void;
  close(code?: number, reason?: string): void;
}

export type GraphicsSocketFactory = (url: string) => GraphicsSocket;

interface GraphicsClientOptions {
  readonly socketFactory?: GraphicsSocketFactory;
  readonly url?: string;
  readonly reconnectDelayMs?: (attempt: number) => number;
  readonly setTimer?: (callback: () => void, delayMs: number) => number;
  readonly clearTimer?: (timer: number) => void;
}

type RequestType =
  | "initialize"
  | "getSnapshot"
  | "resyncFigure"
  | "getBuffer"
  | "releaseBuffer"
  | "closeFigure"
  | "setAxesLimits"
  | "setAxesCamera"
  | "shutdown";

interface PendingRequest {
  readonly type: RequestType;
  readonly figureId?: string;
  readonly bufferId?: string;
  readonly expectedRevision?: number;
  readonly closeRetry?: boolean;
}

interface FigureRecord {
  snapshot: FigureSnapshot | null;
  renderedRevision: number | null;
  snapshotPending: boolean;
  resyncPending: boolean;
  closeRetryPending: boolean;
}

interface TransferRecord {
  readonly transferId: string;
  readonly bufferId: string;
  readonly totalBytes: number;
  readonly chunkBytes: number;
  readonly chunkCount: number;
  readonly bytes: Uint8Array;
  nextOffset: number;
  receivedChunks: number;
}

interface NegotiatedLimits {
  readonly maxTextFrameBytes: number;
  readonly maxBinaryFrameBytes: number;
  readonly maxBufferBytes: number;
  readonly maxResidentBytes: number;
  readonly maxObjects: number;
}

const defaultLimits: NegotiatedLimits = {
  maxTextFrameBytes: MAX_TEXT_FRAME_BYTES,
  maxBinaryFrameBytes: MAX_BINARY_FRAME_BYTES,
  maxBufferBytes: MAX_BUFFER_BYTES,
  maxResidentBytes: MAX_RESIDENT_BYTES,
  maxObjects: MAX_OBJECTS,
};

function defaultSocketFactory(url: string): GraphicsSocket {
  return new WebSocket(url);
}

function defaultReconnectDelay(attempt: number): number {
  return Math.min(250 * 2 ** Math.max(0, attempt - 1), 4_000);
}

function requireNonEmpty(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new GraphicsV1Error("text", `${field} must be a non-empty string`);
  }
  return value;
}

function requireSafeInteger(
  value: unknown,
  field: string,
  positive = false,
): number {
  if (
    !Number.isSafeInteger(value) ||
    (value as number) < (positive ? 1 : 0)
  ) {
    throw new GraphicsV1Error(
      "text",
      `${field} must be a ${positive ? "positive " : ""}safe integer`,
    );
  }
  return value as number;
}

function requireBoundedLimit(
  value: unknown,
  field: string,
  maximum: number,
): number {
  const limit = requireSafeInteger(value, field, true);
  if (limit > maximum) {
    throw new GraphicsV1Error("text", `${field} exceeds the graphics-v1 limit`);
  }
  return limit;
}

export class GraphicsV1Client {
  readonly #sessionId: string;
  readonly #protocol: GraphicsProtocol;
  readonly #endpoint: FigureDiscovery["endpoint"];
  readonly #attachToken: string;
  readonly #socketFactory: GraphicsSocketFactory;
  readonly #url: string;
  readonly #reconnectDelayMs: (attempt: number) => number;
  readonly #setTimer: (callback: () => void, delayMs: number) => number;
  readonly #clearTimer: (timer: number) => void;
  readonly #listeners = new Set<(event: GraphicsClientEvent) => void>();
  readonly #discoveries = new Map<string, number>();
  readonly #figures = new Map<string, FigureRecord>();
  readonly #pending = new Map<string, PendingRequest>();
  readonly #descriptors = new Map<string, DataRef>();
  readonly #buffers = new Map<string, Uint8Array>();
  readonly #bufferRequests = new Set<string>();
  readonly #transfers = new Map<string, TransferRecord>();

  #socket: GraphicsSocket | null = null;
  #phase: GraphicsConnectionPhase = "idle";
  #statusMessage: string | null = null;
  #messageSequence = 0;
  #reconnectAttempt = 0;
  #reconnectTimer: number | null = null;
  #disposed = false;
  #terminal = false;
  #limits = defaultLimits;

  constructor(
    sessionId: string,
    discovery: FigureDiscovery,
    options: GraphicsClientOptions = {},
  ) {
    if (sessionId.length === 0) {
      throw new GraphicsV1Error("discovery", "graphics sessionId is required");
    }
    this.#sessionId = sessionId;
    this.#protocol = discovery.graphicsProtocol;
    this.#endpoint = discovery.endpoint;
    this.#attachToken = discovery.attachToken;
    this.#socketFactory = options.socketFactory ?? defaultSocketFactory;
    this.#url = selectDiscoveredEndpoint(
      options.url ?? graphicsWebSocketUrl(window.location, discovery.endpoint),
      discovery.endpoint,
    );
    this.#reconnectDelayMs =
      options.reconnectDelayMs ?? defaultReconnectDelay;
    this.#setTimer =
      options.setTimer ??
      ((callback, delayMs) => window.setTimeout(callback, delayMs));
    this.#clearTimer =
      options.clearTimer ?? ((timer) => window.clearTimeout(timer));
    this.setDiscoveries([discovery]);
  }

  get status(): GraphicsConnectionStatus {
    return {
      phase: this.#phase,
      reconnectAttempt: this.#reconnectAttempt,
      message: this.#statusMessage,
    };
  }

  matchesAttachment(discovery: FigureDiscovery): boolean {
    return (
      discovery.endpoint === this.#endpoint &&
      discovery.graphicsProtocol === this.#protocol &&
      discovery.attachToken === this.#attachToken
    );
  }

  setDiscoveries(discoveries: readonly FigureDiscovery[]): void {
    for (const discovery of discoveries) {
      if (!this.matchesAttachment(discovery)) {
        throw new GraphicsV1Error(
          "discovery",
          "Figure discovery belongs to a different graphics attachment",
        );
      }
      this.#discoveries.set(discovery.figureId, discovery.revision);
      if (!this.#figures.has(discovery.figureId)) {
        this.#figures.set(discovery.figureId, {
          snapshot: null,
          renderedRevision: null,
          snapshotPending: false,
          resyncPending: false,
          closeRetryPending: false,
        });
      }
      if (this.#phase === "attached") {
        this.#requestSnapshot(discovery.figureId);
      }
    }
  }

  subscribe(listener: (event: GraphicsClientEvent) => void): () => void {
    this.#listeners.add(listener);
    listener({ type: "connection", status: this.status });
    for (const [figureId, record] of this.#figures) {
      if (
        record.snapshot !== null &&
        record.renderedRevision === record.snapshot.revision
      ) {
        const figure = this.#readyFigure(figureId, record);
        if (figure !== null) {
          listener({ type: "figureReady", figure });
        }
      }
    }
    return () => this.#listeners.delete(listener);
  }

  connect(): void {
    if (
      this.#disposed ||
      this.#terminal ||
      this.#socket !== null ||
      this.#phase === "connecting" ||
      this.#phase === "initializing" ||
      this.#phase === "attached"
    ) {
      return;
    }
    this.#setStatus(
      this.#reconnectAttempt === 0 ? "connecting" : "reconnecting",
      null,
    );
    let socket: GraphicsSocket;
    try {
      socket = this.#socketFactory(this.#url);
    } catch {
      this.#scheduleReconnect("Unable to open the graphics channel.");
      return;
    }
    this.#socket = socket;
    socket.binaryType = "arraybuffer";
    socket.onopen = () => {
      if (this.#socket !== socket || this.#disposed) {
        return;
      }
      this.#setStatus("initializing", null);
      this.#sendRequest(
        "initialize",
        {
          type: "initialize",
          attachToken: this.#attachToken,
          client: { name: CLIENT_NAME, version: CLIENT_VERSION },
          capabilities: {
            renderBackend: "webgpu",
            maxTextFrameBytes: MAX_TEXT_FRAME_BYTES,
            maxBinaryFrameBytes: MAX_BINARY_FRAME_BYTES,
            maxBufferBytes: MAX_BUFFER_BYTES,
            maxResidentBytes: MAX_RESIDENT_BYTES,
          },
        },
        {},
      );
    };
    socket.onmessage = (event) => {
      if (this.#socket !== socket || this.#disposed) {
        return;
      }
      void this.#handleSocketMessage(event.data);
    };
    socket.onerror = () => {
      if (this.#socket === socket && !this.#disposed) {
        this.#statusMessage = "The graphics channel reported a transport error.";
      }
    };
    socket.onclose = () => {
      if (this.#socket !== socket) {
        return;
      }
      this.#socket = null;
      const interruptedCloses = new Set<string>();
      for (const pending of this.#pending.values()) {
        if (pending.type === "closeFigure" && pending.figureId !== undefined) {
          interruptedCloses.add(pending.figureId);
        }
      }
      for (const [figureId, figure] of this.#figures) {
        if (figure.closeRetryPending) {
          interruptedCloses.add(figureId);
        }
      }
      this.#pending.clear();
      this.#transfers.clear();
      this.#bufferRequests.clear();
      for (const figure of this.#figures.values()) {
        figure.snapshotPending = false;
        figure.resyncPending = false;
        figure.closeRetryPending = false;
      }
      interruptedCloses.forEach((figureId) =>
        this.#emitCloseFailed(
          figureId,
          "graphics.transport",
          "The graphics channel closed before the Figure could be closed.",
        ),
      );
      if (!this.#disposed && !this.#terminal) {
        this.#scheduleReconnect(
          this.#statusMessage ?? "The graphics channel closed; reconnecting.",
        );
      }
    };
  }

  closeFigure(figureId: string): boolean {
    const record = this.#figures.get(figureId);
    if (
      this.#phase !== "attached" ||
      record?.snapshot === null ||
      record?.snapshot === undefined
    ) {
      return false;
    }
    return this.#sendCloseFigure(figureId, record.snapshot.revision, false);
  }

  #sendCloseFigure(
    figureId: string,
    expectedRevision: number,
    closeRetry: boolean,
  ): boolean {
    return this.#sendRequest(
      "closeFigure",
      {
        type: "closeFigure",
        figureId,
        expectedRevision,
      },
      { figureId, expectedRevision, closeRetry },
    );
  }

  setAxesLimits(
    figureId: string,
    expectedRevision: number,
    xLimits: readonly [number, number],
    yLimits: readonly [number, number],
    axesId?: string,
  ): void {
    if (
      this.#protocol !== GRAPHICS_PROTOCOL_V2 &&
      this.#protocol !== GRAPHICS_PROTOCOL_V3 &&
      this.#protocol !== GRAPHICS_PROTOCOL_V4
    ) {
      throw new GraphicsV1Error(
        "discovery",
        "setAxesLimits requires openmat-graphics-v2 or newer",
      );
    }
    const target = validateInteractionAxesTarget(this.#protocol, axesId);
    requireSafeInteger(expectedRevision, "expectedRevision", true);
    validateSemanticLimits(xLimits, "xLimits");
    validateSemanticLimits(yLimits, "yLimits");
    if (this.#phase !== "attached" || !this.#figures.has(figureId)) {
      return;
    }
    this.#sendRequest(
      "setAxesLimits",
      {
        type: "setAxesLimits",
        figureId,
        ...(target === undefined ? {} : { axesId: target }),
        expectedRevision,
        xLimits,
        yLimits,
      },
      { figureId, expectedRevision },
    );
  }

  setAxesCamera(
    figureId: string,
    expectedRevision: number,
    view: readonly [number, number],
    cameraScale: number,
    axesId?: string,
  ): void {
    if (
      this.#protocol !== GRAPHICS_PROTOCOL_V2 &&
      this.#protocol !== GRAPHICS_PROTOCOL_V3 &&
      this.#protocol !== GRAPHICS_PROTOCOL_V4
    ) {
      throw new GraphicsV1Error(
        "discovery",
        "setAxesCamera requires openmat-graphics-v2 or newer",
      );
    }
    const target = validateInteractionAxesTarget(this.#protocol, axesId);
    requireSafeInteger(expectedRevision, "expectedRevision", true);
    if (
      view.length !== 2 ||
      !view.every(Number.isFinite) ||
      view[1] < -90 ||
      view[1] > 90
    ) {
      throw new GraphicsV1Error(
        "scene",
        "view must contain a finite azimuth and an elevation in -90..90",
      );
    }
    if (
      !Number.isFinite(cameraScale) ||
      cameraScale < 1 / 45 ||
      cameraScale > 170 / 45
    ) {
      throw new GraphicsV1Error("scene", "cameraScale is outside the supported range");
    }
    if (this.#phase !== "attached" || !this.#figures.has(figureId)) {
      return;
    }
    this.#sendRequest(
      "setAxesCamera",
      {
        type: "setAxesCamera",
        figureId,
        ...(target === undefined ? {} : { axesId: target }),
        expectedRevision,
        view,
        cameraScale,
      },
      { figureId, expectedRevision },
    );
  }

  releaseBuffer(bufferId: string): void {
    if (this.#phase !== "attached" || !this.#buffers.has(bufferId)) {
      return;
    }
    this.#sendRequest(
      "releaseBuffer",
      { type: "releaseBuffer", bufferId },
      { bufferId },
    );
  }

  dispose(): void {
    if (this.#disposed) {
      return;
    }
    this.#disposed = true;
    if (this.#reconnectTimer !== null) {
      this.#clearTimer(this.#reconnectTimer);
      this.#reconnectTimer = null;
    }
    const socket = this.#socket;
    if (socket !== null) {
      if (socket.readyState === SOCKET_OPEN && this.#phase === "attached") {
        this.#sendRequest("shutdown", { type: "shutdown" }, {});
      }
      socket.onclose = null;
      socket.onmessage = null;
      socket.onerror = null;
      socket.onopen = null;
      socket.close(1000, "graphics client disposed");
    }
    this.#socket = null;
    this.#pending.clear();
    this.#transfers.clear();
    this.#setStatus("closed", null);
    this.#listeners.clear();
  }

  #setStatus(
    phase: GraphicsConnectionPhase,
    message: string | null,
  ): void {
    this.#phase = phase;
    this.#statusMessage = message;
    const event: GraphicsClientEvent = {
      type: "connection",
      status: this.status,
    };
    this.#listeners.forEach((listener) => listener(event));
  }

  #sendRequest(
    type: RequestType,
    request: Readonly<Record<string, unknown>>,
    context: Omit<PendingRequest, "type">,
  ): boolean {
    const socket = this.#socket;
    if (socket === null || socket.readyState !== SOCKET_OPEN) {
      return false;
    }
    this.#messageSequence += 1;
    const messageId = `graphics-web-${this.#messageSequence.toString()}`;
    const encoded = JSON.stringify({
      protocol: this.#protocol,
      sessionId: this.#sessionId,
      messageId,
      kind: "request",
      request,
    });
    if (new TextEncoder().encode(encoded).byteLength > this.#limits.maxTextFrameBytes) {
      this.#protocolFault("A graphics-v1 request exceeded the text limit.");
      return false;
    }
    this.#pending.set(messageId, { type, ...context });
    try {
      socket.send(encoded);
      return true;
    } catch {
      this.#pending.delete(messageId);
      return false;
    }
  }

  async #handleSocketMessage(data: unknown): Promise<void> {
    try {
      if (typeof data === "string") {
        if (
          new TextEncoder().encode(data).byteLength >
          this.#limits.maxTextFrameBytes
        ) {
          throw new GraphicsV1Error(
            "text",
            "graphics-v1 text frame exceeds the negotiated limit",
          );
        }
        this.#handleText(data);
        return;
      }
      if (data instanceof ArrayBuffer) {
        this.#handleBinary(data);
        return;
      }
      if (ArrayBuffer.isView(data)) {
        this.#handleBinary(data);
        return;
      }
      if (data instanceof Blob) {
        this.#handleBinary(await data.arrayBuffer());
        return;
      }
      throw new GraphicsV1Error("binary", "graphics-v1 frame type is unsupported");
    } catch {
      this.#protocolFault("The graphics channel sent an invalid protocol frame.");
    }
  }

  #handleText(encoded: string): void {
    const envelope = parseTextEnvelope(encoded, this.#protocol);
    if (envelope.sessionId !== this.#sessionId) {
      throw new GraphicsV1Error("text", "graphics-v1 sessionId does not match");
    }
    if (envelope.kind === "response") {
      this.#handleResponse(envelope);
    } else {
      this.#handleEvent(envelope);
    }
  }

  #handleResponse(envelope: Readonly<Record<string, unknown>>): void {
    const replyTo = requireNonEmpty(envelope.replyTo, "replyTo");
    const pending = this.#pending.get(replyTo);
    if (pending === undefined) {
      throw new GraphicsV1Error("sequence", "graphics response has no request");
    }
    this.#pending.delete(replyTo);
    if (typeof envelope.ok !== "boolean") {
      throw new GraphicsV1Error("text", "graphics response ok must be Boolean");
    }
    if (!envelope.ok) {
      const error = isRecord(envelope.error) ? envelope.error : null;
      const category =
        error === null
          ? "graphics.invalidRequest"
          : requireNonEmpty(error.category, "error.category");
      const rawMessage =
        error !== null && typeof error.message === "string"
          ? error.message
          : "The graphics request failed.";
      const message = this.#redact(rawMessage);
      if (pending.type === "initialize") {
        this.#terminal =
          category === "graphics.unauthorized" ||
          category === "graphics.sessionClosed";
        this.#setStatus("error", message);
        this.#socket?.close(
          CLIENT_CLOSE_POLICY_ERROR,
          "graphics initialization rejected",
        );
      } else if (pending.type === "closeFigure" && pending.figureId !== undefined) {
        const record = this.#figures.get(pending.figureId);
        if (
          category === "graphics.revisionConflict" &&
          pending.closeRetry !== true &&
          record?.snapshot !== null &&
          record?.snapshot !== undefined
        ) {
          record.closeRetryPending = true;
          if (!record.resyncPending) {
            record.resyncPending = true;
            const sent = this.#sendRequest(
              "resyncFigure",
              {
                type: "resyncFigure",
                figureId: pending.figureId,
                knownRevision: record.snapshot.revision,
              },
              { figureId: pending.figureId },
            );
            if (!sent) {
              record.resyncPending = false;
              record.closeRetryPending = false;
              this.#emitCloseFailed(pending.figureId, category, message);
              this.#setStatus("error", message);
            }
          }
        } else {
          if (record !== undefined) {
            record.closeRetryPending = false;
          }
          this.#emitCloseFailed(pending.figureId, category, message);
          this.#setStatus("error", message);
        }
      } else if (pending.type === "resyncFigure" && pending.figureId !== undefined) {
        const record = this.#figures.get(pending.figureId);
        if (record !== undefined) {
          record.resyncPending = false;
          if (record.closeRetryPending) {
            record.closeRetryPending = false;
            this.#emitCloseFailed(pending.figureId, category, message);
          }
        }
        this.#setStatus("error", message);
      } else {
        this.#setStatus("error", message);
      }
      return;
    }
    if (envelope.error !== undefined || !isRecord(envelope.result)) {
      throw new GraphicsV1Error("text", "graphics success response is malformed");
    }
    const result = envelope.result;
    switch (pending.type) {
      case "initialize":
        this.#acceptInitialize(result);
        break;
      case "getSnapshot":
      case "resyncFigure":
        this.#acceptSnapshot(pending, result);
        break;
      case "getBuffer":
        this.#acceptBufferTransfer(pending, result);
        break;
      case "releaseBuffer":
        if (result.type !== "releaseBuffer" || typeof result.released !== "boolean") {
          throw new GraphicsV1Error("text", "releaseBuffer response is invalid");
        }
        break;
      case "closeFigure":
        if (result.type !== "closeFigure") {
          throw new GraphicsV1Error("text", "closeFigure response is invalid");
        }
        requireSafeInteger(result.closedRevision, "closedRevision", true);
        break;
      case "setAxesLimits": {
        if (result.type !== "setAxesLimits") {
          throw new GraphicsV1Error("text", "setAxesLimits response is invalid");
        }
        const committedRevision = requireSafeInteger(
          result.committedRevision,
          "committedRevision",
          true,
        );
        if (
          pending.expectedRevision === undefined ||
          committedRevision !== pending.expectedRevision + 1
        ) {
          throw new GraphicsV1Error(
            "sequence",
            "setAxesLimits committedRevision is not the next Figure revision",
          );
        }
        // The ordinary figureDelta remains authoritative. Do not edit the
        // local snapshot or its revision from this acknowledgement.
        break;
      }
      case "setAxesCamera": {
        if (result.type !== "setAxesCamera") {
          throw new GraphicsV1Error("text", "setAxesCamera response is invalid");
        }
        const committedRevision = requireSafeInteger(
          result.committedRevision,
          "committedRevision",
          true,
        );
        if (
          pending.expectedRevision === undefined ||
          committedRevision !== pending.expectedRevision + 1
        ) {
          throw new GraphicsV1Error(
            "sequence",
            "setAxesCamera committedRevision is not the next Figure revision",
          );
        }
        break;
      }
      case "shutdown":
        if (result.type !== "shutdown") {
          throw new GraphicsV1Error("text", "shutdown response is invalid");
        }
        break;
    }
  }

  #acceptInitialize(result: Readonly<Record<string, unknown>>): void {
    if (result.type !== "initialize" || !isRecord(result.implementation)) {
      throw new GraphicsV1Error("text", "initialize response is invalid");
    }
    requireNonEmpty(result.implementation.name, "implementation.name");
    requireNonEmpty(result.implementation.version, "implementation.version");
    const capabilities = isRecord(result.capabilities)
      ? result.capabilities
      : null;
    if (capabilities === null || capabilities.renderBackend !== "webgpu") {
      throw new GraphicsV1Error("text", "initialize capabilities are invalid");
    }
    this.#limits = {
      maxTextFrameBytes: requireBoundedLimit(
        capabilities.maxTextFrameBytes,
        "maxTextFrameBytes",
        MAX_TEXT_FRAME_BYTES,
      ),
      maxBinaryFrameBytes: requireBoundedLimit(
        capabilities.maxBinaryFrameBytes,
        "maxBinaryFrameBytes",
        MAX_BINARY_FRAME_BYTES,
      ),
      maxBufferBytes: requireBoundedLimit(
        capabilities.maxBufferBytes,
        "maxBufferBytes",
        MAX_BUFFER_BYTES,
      ),
      maxResidentBytes: requireBoundedLimit(
        capabilities.maxResidentBytes,
        "maxResidentBytes",
        MAX_RESIDENT_BYTES,
      ),
      maxObjects: requireBoundedLimit(
        capabilities.maxObjects,
        "maxObjects",
        MAX_OBJECTS,
      ),
    };
    if (!Array.isArray(result.figures)) {
      throw new GraphicsV1Error("text", "initialize figures must be an array");
    }
    const figureIds = new Set<string>();
    for (const summaryValue of result.figures) {
      if (!isRecord(summaryValue)) {
        throw new GraphicsV1Error("text", "initialize Figure summary is invalid");
      }
      const figureId = requireNonEmpty(summaryValue.figureId, "figureId");
      requireSafeInteger(summaryValue.revision, "figure revision", true);
      if (figureIds.has(figureId)) {
        throw new GraphicsV1Error("text", "initialize contains duplicate Figures");
      }
      figureIds.add(figureId);
      if (!this.#figures.has(figureId)) {
        this.#figures.set(figureId, {
          snapshot: null,
          renderedRevision: null,
          snapshotPending: false,
          resyncPending: false,
          closeRetryPending: false,
        });
      }
    }
    for (const figureId of this.#discoveries.keys()) {
      figureIds.add(figureId);
    }
    this.#reconnectAttempt = 0;
    this.#setStatus("attached", null);
    figureIds.forEach((figureId) => this.#requestSnapshot(figureId));
  }

  #acceptSnapshot(
    pending: PendingRequest,
    result: Readonly<Record<string, unknown>>,
  ): void {
    if (
      (pending.type === "getSnapshot" && result.type !== "getSnapshot") ||
      (pending.type === "resyncFigure" && result.type !== "resyncFigure")
    ) {
      throw new GraphicsV1Error("text", "snapshot response type is invalid");
    }
    const snapshot = parseFigureSnapshot(result.snapshot);
    if (pending.figureId === undefined || snapshot.figureId !== pending.figureId) {
      throw new GraphicsV1Error("sequence", "snapshot response Figure does not match");
    }
    const record = this.#figures.get(snapshot.figureId) ?? {
      snapshot: null,
      renderedRevision: null,
      snapshotPending: false,
      resyncPending: false,
      closeRetryPending: false,
    };
    record.snapshot = snapshot;
    record.snapshotPending = false;
    record.resyncPending = false;
    this.#figures.set(snapshot.figureId, record);
    this.#registerDescriptors(snapshot.referencedBuffers);
    this.#requestMissingBuffers(snapshot.referencedBuffers);
    this.#publishWhenReady(snapshot.figureId);
    if (record.closeRetryPending) {
      record.closeRetryPending = false;
      if (!this.#sendCloseFigure(snapshot.figureId, snapshot.revision, true)) {
        this.#emitCloseFailed(
          snapshot.figureId,
          "graphics.transport",
          "The graphics channel is unavailable; the Figure remains open.",
        );
      }
    }
  }

  #acceptBufferTransfer(
    pending: PendingRequest,
    result: Readonly<Record<string, unknown>>,
  ): void {
    if (result.type !== "getBuffer" || pending.bufferId === undefined) {
      throw new GraphicsV1Error("text", "getBuffer response is invalid");
    }
    const transferId = requireNonEmpty(result.transferId, "transferId");
    const bufferId = requireNonEmpty(result.bufferId, "bufferId");
    if (bufferId !== pending.bufferId || this.#transfers.has(transferId)) {
      throw new GraphicsV1Error("sequence", "getBuffer transfer identity is invalid");
    }
    const totalBytes = requireSafeInteger(result.totalBytes, "totalBytes");
    const chunkBytes = requireSafeInteger(result.chunkBytes, "chunkBytes");
    const chunkCount = requireSafeInteger(result.chunkCount, "chunkCount");
    const descriptor = this.#descriptors.get(bufferId);
    if (
      descriptor === undefined ||
      totalBytes !== descriptor.byteLength ||
      totalBytes > this.#limits.maxBufferBytes
    ) {
      throw new GraphicsV1Error("sequence", "getBuffer descriptor does not match");
    }
    if (totalBytes === 0) {
      if (chunkBytes !== 0 || chunkCount !== 0) {
        throw new GraphicsV1Error("sequence", "zero-byte transfer must have no chunks");
      }
      this.#buffers.set(bufferId, new Uint8Array());
      this.#bufferRequests.delete(bufferId);
      this.#publishFiguresWaitingFor(bufferId);
      return;
    }
    const expectedChunks = Math.ceil(totalBytes / chunkBytes);
    if (
      chunkBytes <= 0 ||
      chunkBytes > this.#limits.maxBinaryFrameBytes ||
      chunkCount !== expectedChunks
    ) {
      throw new GraphicsV1Error("sequence", "getBuffer chunk plan is invalid");
    }
    this.#transfers.set(transferId, {
      transferId,
      bufferId,
      totalBytes,
      chunkBytes,
      chunkCount,
      bytes: new Uint8Array(totalBytes),
      nextOffset: 0,
      receivedChunks: 0,
    });
  }

  #handleEvent(envelope: Readonly<Record<string, unknown>>): void {
    if (!isRecord(envelope.event)) {
      throw new GraphicsV1Error("text", "graphics event body is invalid");
    }
    const event = envelope.event;
    const type = requireNonEmpty(event.type, "event.type");
    switch (type) {
      case "figureDelta":
        this.#acceptDelta(parseFigureDelta(event));
        break;
      case "figureClosed": {
        const figureId = requireNonEmpty(event.figureId, "figureClosed.figureId");
        const closedRevision = requireSafeInteger(
          event.closedRevision,
          "figureClosed.closedRevision",
          true,
        );
        this.#figures.delete(figureId);
        const clientEvent: GraphicsClientEvent = {
          type: "figureClosed",
          figureId,
          closedRevision,
        };
        this.#listeners.forEach((listener) => listener(clientEvent));
        break;
      }
      case "bufferReleased":
        requireNonEmpty(event.bufferId, "bufferReleased.bufferId");
        break;
      case "sessionClosed":
        this.#terminal = true;
        this.#setStatus("closed", "The graphics session has ended.");
        this.#socket?.close(1000, "graphics session closed");
        break;
      default:
        // Unknown event types are intentionally ignored for forward compatibility.
        break;
    }
  }

  #acceptDelta(delta: ReturnType<typeof parseFigureDelta>): void {
    const record = this.#figures.get(delta.figureId);
    if (record?.snapshot === null || record?.snapshot === undefined) {
      this.#requestSnapshot(delta.figureId);
      return;
    }
    if (record.snapshot.revision !== delta.baseRevision) {
      if (!record.resyncPending) {
        record.resyncPending = true;
        this.#sendRequest(
          "resyncFigure",
          {
            type: "resyncFigure",
            figureId: delta.figureId,
            knownRevision: record.snapshot.revision,
          },
          { figureId: delta.figureId },
        );
      }
      return;
    }
    record.snapshot = applyFigureDelta(record.snapshot, delta);
    this.#registerDescriptors(delta.addedBuffers);
    this.#requestMissingBuffers(record.snapshot.referencedBuffers);
    this.#publishWhenReady(delta.figureId);
  }

  #handleBinary(data: ArrayBuffer | ArrayBufferView): void {
    const chunk = decodeOmgpFrame(data, this.#limits.maxBinaryFrameBytes);
    const transfer = this.#transfers.get(chunk.transferId);
    if (
      transfer === undefined ||
      transfer.bufferId !== chunk.bufferId ||
      transfer.totalBytes !== chunk.totalBytes
    ) {
      throw new GraphicsV1Error("sequence", "OMGP chunk has no matching transfer");
    }
    if (chunk.offset !== transfer.nextOffset) {
      throw new GraphicsV1Error("sequence", "OMGP chunk has a gap or overlap");
    }
    if (
      chunk.payload.byteLength === 0 ||
      chunk.payload.byteLength > transfer.chunkBytes ||
      transfer.receivedChunks >= transfer.chunkCount
    ) {
      throw new GraphicsV1Error("sequence", "OMGP chunk length or count is invalid");
    }
    transfer.bytes.set(chunk.payload, chunk.offset);
    transfer.nextOffset += chunk.payload.byteLength;
    transfer.receivedChunks += 1;
    const complete = transfer.nextOffset === transfer.totalBytes;
    if (
      chunk.final !== complete ||
      (complete && transfer.receivedChunks !== transfer.chunkCount) ||
      (!complete && transfer.receivedChunks === transfer.chunkCount)
    ) {
      throw new GraphicsV1Error("sequence", "OMGP transfer coverage is incomplete");
    }
    if (complete) {
      this.#transfers.delete(transfer.transferId);
      this.#bufferRequests.delete(transfer.bufferId);
      this.#buffers.set(transfer.bufferId, transfer.bytes);
      this.#publishFiguresWaitingFor(transfer.bufferId);
    }
  }

  #requestSnapshot(figureId: string): void {
    const record = this.#figures.get(figureId) ?? {
      snapshot: null,
      renderedRevision: null,
      snapshotPending: false,
      resyncPending: false,
      closeRetryPending: false,
    };
    this.#figures.set(figureId, record);
    if (record.snapshotPending || this.#phase !== "attached") {
      return;
    }
    record.snapshotPending = true;
    this.#sendRequest(
      "getSnapshot",
      { type: "getSnapshot", figureId },
      { figureId },
    );
  }

  #registerDescriptors(descriptors: readonly DataRef[]): void {
    for (const descriptorValue of descriptors) {
      const descriptor = parseDataRef(descriptorValue);
      const previous = this.#descriptors.get(descriptor.bufferId);
      if (
        previous !== undefined &&
        (previous.dtype !== descriptor.dtype ||
          previous.shape[1] !== descriptor.shape[1] ||
          previous.byteLength !== descriptor.byteLength)
      ) {
        throw new GraphicsV1Error(
          "scene",
          "An immutable graphics buffer changed descriptor",
        );
      }
      this.#descriptors.set(descriptor.bufferId, descriptor);
    }
  }

  #requestMissingBuffers(descriptors: readonly DataRef[]): void {
    for (const descriptor of descriptors) {
      if (
        this.#buffers.has(descriptor.bufferId) ||
        this.#bufferRequests.has(descriptor.bufferId)
      ) {
        continue;
      }
      this.#bufferRequests.add(descriptor.bufferId);
      this.#sendRequest(
        "getBuffer",
        { type: "getBuffer", bufferId: descriptor.bufferId },
        { bufferId: descriptor.bufferId },
      );
    }
  }

  #publishFiguresWaitingFor(bufferId: string): void {
    for (const [figureId, record] of this.#figures) {
      if (
        record.snapshot?.referencedBuffers.some(
          (descriptor) => descriptor.bufferId === bufferId,
        ) === true
      ) {
        this.#publishWhenReady(figureId);
      }
    }
  }

  #publishWhenReady(figureId: string): void {
    const record = this.#figures.get(figureId);
    const snapshot = record?.snapshot;
    if (
      record === undefined ||
      snapshot === null ||
      snapshot === undefined ||
      record.renderedRevision === snapshot.revision
    ) {
      return;
    }
    const figure = this.#readyFigure(figureId, record);
    if (figure === null) {
      return;
    }
    record.renderedRevision = snapshot.revision;
    const event: GraphicsClientEvent = { type: "figureReady", figure };
    this.#listeners.forEach((listener) => listener(event));
  }

  #readyFigure(figureId: string, record: FigureRecord): RenderableFigure | null {
    const snapshot = record.snapshot;
    if (snapshot === null) {
      return null;
    }
    const buffers = new Map<string, Uint8Array>();
    for (const descriptor of snapshot.referencedBuffers) {
      const bytes = this.#buffers.get(descriptor.bufferId);
      if (bytes === undefined) {
        return null;
      }
      buffers.set(descriptor.bufferId, bytes);
    }
    return {
      figureId,
      revision: snapshot.revision,
      snapshot,
      buffers,
    };
  }

  #emitCloseFailed(figureId: string, category: string, message: string): void {
    const event: GraphicsClientEvent = {
      type: "figureCloseFailed",
      figureId,
      category,
      message,
    };
    this.#listeners.forEach((listener) => listener(event));
  }

  #protocolFault(message: string): void {
    this.#transfers.clear();
    this.#bufferRequests.clear();
    this.#statusMessage = message;
    this.#socket?.close(CLIENT_CLOSE_PROTOCOL_ERROR, "invalid graphics-v1 frame");
    if (this.#socket === null) {
      this.#scheduleReconnect(message);
    }
  }

  #scheduleReconnect(message: string): void {
    if (this.#disposed || this.#terminal || this.#reconnectTimer !== null) {
      return;
    }
    this.#reconnectAttempt += 1;
    this.#setStatus("reconnecting", message);
    this.#reconnectTimer = this.#setTimer(() => {
      this.#reconnectTimer = null;
      this.connect();
    }, this.#reconnectDelayMs(this.#reconnectAttempt));
  }

  #redact(message: string): string {
    return message.includes(this.#attachToken)
      ? message.split(this.#attachToken).join("[REDACTED]")
      : message;
  }
}

function selectDiscoveredEndpoint(
  configuredUrl: string,
  endpoint: FigureDiscovery["endpoint"],
): string {
  try {
    const url = new URL(configuredUrl);
    if (
      url.pathname === GRAPHICS_ENDPOINT_V1 ||
      url.pathname === GRAPHICS_ENDPOINT_V2 ||
      url.pathname === GRAPHICS_ENDPOINT_V3 ||
      url.pathname === GRAPHICS_ENDPOINT_V4
    ) {
      url.pathname = endpoint;
    }
    return url.href;
  } catch {
    if (
      configuredUrl === GRAPHICS_ENDPOINT_V1 ||
      configuredUrl === GRAPHICS_ENDPOINT_V2 ||
      configuredUrl === GRAPHICS_ENDPOINT_V3 ||
      configuredUrl === GRAPHICS_ENDPOINT_V4
    ) {
      return endpoint;
    }
    return configuredUrl;
  }
}

function validateInteractionAxesTarget(
  protocol: GraphicsProtocol,
  axesId: string | undefined,
): string | undefined {
  if (protocol === GRAPHICS_PROTOCOL_V2) {
    if (axesId !== undefined) {
      throw new GraphicsV1Error(
        "discovery",
        "axesId is unavailable in openmat-graphics-v2",
      );
    }
    return undefined;
  }
  if (protocol === GRAPHICS_PROTOCOL_V3 || protocol === GRAPHICS_PROTOCOL_V4) {
    return requireNonEmpty(axesId, "axesId");
  }
  throw new GraphicsV1Error(
    "discovery",
    "explicit graphics interaction is unavailable in this protocol",
  );
}

function validateSemanticLimits(
  limits: readonly [number, number],
  field: string,
): void {
  if (
    limits.length !== 2 ||
    !limits.every(Number.isFinite) ||
    limits[0] >= limits[1]
  ) {
    throw new GraphicsV1Error(
      "scene",
      `${field} must contain two finite strictly increasing numbers`,
    );
  }
}
