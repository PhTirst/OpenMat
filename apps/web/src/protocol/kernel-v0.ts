export const KERNEL_PROTOCOL = "openmat-kernel-v0" as const;
export const MAX_PREVIEW_ELEMENTS = 4_096;

export type KernelMessageKind = "request" | "response" | "event";
export type KernelStatus =
  | "starting"
  | "idle"
  | "busy"
  | "interrupted"
  | "dead";
export type ExecuteMode = "file" | "cell" | "repl";
export type StreamKind = "stdout" | "stderr";
export type DiagnosticSeverity =
  | "error"
  | "warning"
  | "information"
  | "hint";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue =
  | JsonPrimitive
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

export interface KernelEnvelope {
  readonly protocol: typeof KERNEL_PROTOCOL;
  readonly sessionId: string;
  readonly messageId: string;
  readonly kind: KernelMessageKind;
}

export interface ImplementationInfo {
  readonly name: string;
  readonly version: string;
}

export interface Capabilities {
  readonly executionModes: readonly ExecuteMode[];
  readonly displayMimeTypes: readonly string[];
  readonly maxPreviewElements: number;
  readonly interrupt: boolean;
  readonly workspaceDelta: boolean;
}

export interface SourceRange {
  readonly sourceName: string;
  /** Inclusive UTF-8 byte offset. */
  readonly start: number;
  /** Exclusive UTF-8 byte offset. */
  readonly end: number;
}

export interface RelatedDiagnostic {
  readonly message: string;
  readonly range: SourceRange;
}

export interface Diagnostic {
  readonly code?: string;
  readonly severity: DiagnosticSeverity;
  readonly message: string;
  readonly range?: SourceRange;
  readonly related?: readonly RelatedDiagnostic[];
}

export interface ProtocolError {
  readonly category: string;
  readonly message: string;
  readonly diagnostics?: readonly Diagnostic[];
}

export interface VariableSummary {
  readonly name: string;
  readonly class: string;
  readonly dimensions: readonly number[];
  readonly complex: boolean;
  readonly bytes?: number;
}

export interface WorkspaceSummary {
  readonly variables: readonly VariableSummary[];
}

export interface MatrixRange {
  readonly start: readonly number[];
  readonly size: readonly number[];
}

export type PreviewValue =
  | { readonly kind: "number"; readonly value: number }
  | {
      readonly kind: "complex";
      readonly real: number;
      readonly imaginary: number;
    }
  | { readonly kind: "logical"; readonly value: boolean }
  | { readonly kind: "text"; readonly value: string }
  | {
      readonly kind: "special";
      readonly value: "nan" | "infinity" | "negativeInfinity";
    }
  | { readonly kind: "missing" };

export interface MatrixPreview {
  readonly class: string;
  readonly dimensions: readonly number[];
  readonly selectedRange: MatrixRange;
  readonly values: readonly PreviewValue[];
  readonly truncation: {
    readonly truncated: boolean;
    readonly omittedElements: number;
  };
}

export interface InitializeParams {
  readonly client: ImplementationInfo;
  readonly supportedProtocols: readonly string[];
  readonly capabilities: Capabilities;
}

export interface ExecuteParams {
  readonly code: string;
  readonly sourceName: string;
  readonly mode: ExecuteMode;
}

export interface InspectParams {
  readonly name: string;
  readonly range: MatrixRange;
  readonly maxElements: number;
}

export interface RequestParamsByType {
  readonly initialize: InitializeParams;
  readonly execute: ExecuteParams;
  readonly interrupt: Record<string, never>;
  readonly inspect: InspectParams;
  readonly listWorkspace: Record<string, never>;
  readonly shutdown: Record<string, never>;
}

export interface ResponseDataByType {
  readonly initialize: {
    readonly negotiatedProtocol: string;
    readonly implementation: ImplementationInfo;
    readonly capabilities: Capabilities;
  };
  readonly execute: {
    readonly interrupted: boolean;
  };
  readonly interrupt: {
    readonly accepted: boolean;
  };
  readonly inspect: MatrixPreview;
  readonly listWorkspace: WorkspaceSummary;
  readonly shutdown: Record<string, never>;
}

export type KernelRequestType = keyof RequestParamsByType;

export type KernelRequest<T extends KernelRequestType = KernelRequestType> = {
  readonly [K in T]: KernelEnvelope & {
    readonly kind: "request";
    readonly request: {
      readonly type: K;
      readonly params: RequestParamsByType[K];
    };
  };
}[T];

export type KernelSuccessResponse<
  T extends KernelRequestType = KernelRequestType,
> = {
  readonly [K in T]: KernelEnvelope & {
    readonly kind: "response";
    readonly replyTo: string;
    readonly ok: true;
    readonly result: {
      readonly type: K;
      readonly data: ResponseDataByType[K];
    };
  };
}[T];

export type KernelFailureResponse = KernelEnvelope & {
  readonly kind: "response";
  readonly replyTo: string;
  readonly ok: false;
  readonly error: ProtocolError;
};

export type KernelResponse<T extends KernelRequestType = KernelRequestType> =
  | KernelSuccessResponse<T>
  | KernelFailureResponse;

export type ResponseFor<TRequest extends KernelRequest> = KernelResponse<
  TRequest["request"]["type"]
>;

export interface DisplayEventData {
  readonly representations: Readonly<Record<string, string>>;
}

export interface EventDataByType {
  readonly status: {
    readonly status: KernelStatus;
  };
  readonly stream: {
    readonly stream: StreamKind;
    readonly text: string;
  };
  readonly display: DisplayEventData;
  readonly diagnostic: Diagnostic;
  readonly workspaceDelta: {
    readonly added: readonly VariableSummary[];
    readonly changed: readonly VariableSummary[];
    readonly removed: readonly string[];
  };
}

export type KernelEventType = keyof EventDataByType;

export type KernelEvent<E extends KernelEventType = KernelEventType> = {
  readonly [K in E]: KernelEnvelope & {
    readonly kind: "event";
    readonly event: {
      readonly type: K;
      readonly data: EventDataByType[K];
    };
  };
}[E];

export type UnknownKernelEvent = KernelEnvelope & {
  readonly kind: "event";
  readonly event: {
    readonly type: string;
    readonly data: JsonValue;
  };
};

export type ParsedKernelServerMessage =
  | { readonly messageType: "response"; readonly response: KernelResponse }
  | { readonly messageType: "event"; readonly event: KernelEvent }
  | {
      readonly messageType: "unknownEvent";
      readonly event: UnknownKernelEvent;
    };

export function createKernelRequest<T extends KernelRequestType>(
  sessionId: string,
  messageId: string,
  type: T,
  params: RequestParamsByType[T],
): Extract<KernelRequest, { readonly request: { readonly type: T } }> {
  return {
    protocol: KERNEL_PROTOCOL,
    sessionId,
    messageId,
    kind: "request",
    request: { type, params },
  } as Extract<KernelRequest, { readonly request: { readonly type: T } }>;
}

const requestTypes = new Set<KernelRequestType>([
  "initialize",
  "execute",
  "interrupt",
  "inspect",
  "listWorkspace",
  "shutdown",
]);
const statuses = new Set<KernelStatus>([
  "starting",
  "idle",
  "busy",
  "interrupted",
  "dead",
]);
const streams = new Set<StreamKind>(["stdout", "stderr"]);
const severities = new Set<DiagnosticSeverity>([
  "error",
  "warning",
  "information",
  "hint",
]);
const knownEventTypes = new Set<KernelEventType>([
  "status",
  "stream",
  "display",
  "diagnostic",
  "workspaceDelta",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isJsonValue(value: unknown): value is JsonValue {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "boolean"
  ) {
    return true;
  }
  if (typeof value === "number") {
    return Number.isFinite(value);
  }
  if (Array.isArray(value)) {
    return value.every(isJsonValue);
  }
  return isRecord(value) && Object.values(value).every(isJsonValue);
}

function isU64(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function isU64Array(value: unknown): value is number[] {
  return Array.isArray(value) && value.every(isU64);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function hasEnvelope(
  value: Record<string, unknown>,
  kind: "response" | "event",
): boolean {
  return (
    value.protocol === KERNEL_PROTOCOL &&
    typeof value.sessionId === "string" &&
    value.sessionId.length > 0 &&
    typeof value.messageId === "string" &&
    value.messageId.length > 0 &&
    value.kind === kind
  );
}

function isImplementationInfo(value: unknown): value is ImplementationInfo {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    typeof value.version === "string"
  );
}

function isCapabilities(value: unknown): value is Capabilities {
  return (
    isRecord(value) &&
    Array.isArray(value.executionModes) &&
    value.executionModes.every(
      (mode) => mode === "file" || mode === "cell" || mode === "repl",
    ) &&
    isStringArray(value.displayMimeTypes) &&
    isU64(value.maxPreviewElements) &&
    value.maxPreviewElements > 0 &&
    value.maxPreviewElements <= MAX_PREVIEW_ELEMENTS &&
    typeof value.interrupt === "boolean" &&
    typeof value.workspaceDelta === "boolean"
  );
}

function isSourceRange(value: unknown): value is SourceRange {
  return (
    isRecord(value) &&
    typeof value.sourceName === "string" &&
    isU64(value.start) &&
    isU64(value.end)
  );
}

function isDiagnostic(value: unknown): value is Diagnostic {
  if (
    !isRecord(value) ||
    (value.code !== undefined && typeof value.code !== "string") ||
    typeof value.severity !== "string" ||
    !severities.has(value.severity as DiagnosticSeverity) ||
    typeof value.message !== "string" ||
    (value.range !== undefined && !isSourceRange(value.range))
  ) {
    return false;
  }

  return (
    value.related === undefined ||
    (Array.isArray(value.related) &&
      value.related.every(
        (related) =>
          isRecord(related) &&
          typeof related.message === "string" &&
          isSourceRange(related.range),
      ))
  );
}

function isProtocolError(value: unknown): value is ProtocolError {
  return (
    isRecord(value) &&
    typeof value.category === "string" &&
    typeof value.message === "string" &&
    (value.diagnostics === undefined ||
      (Array.isArray(value.diagnostics) && value.diagnostics.every(isDiagnostic)))
  );
}

function isVariableSummary(value: unknown): value is VariableSummary {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    typeof value.class === "string" &&
    isU64Array(value.dimensions) &&
    typeof value.complex === "boolean" &&
    (value.bytes === undefined || isU64(value.bytes))
  );
}

function isMatrixRange(value: unknown): value is MatrixRange {
  return (
    isRecord(value) &&
    isU64Array(value.start) &&
    isU64Array(value.size) &&
    value.start.length > 0 &&
    value.start.length === value.size.length &&
    value.start.every((index) => index > 0)
  );
}

function isPreviewValue(value: unknown): value is PreviewValue {
  if (!isRecord(value)) {
    return false;
  }

  switch (value.kind) {
    case "number":
      return typeof value.value === "number" && Number.isFinite(value.value);
    case "complex":
      return (
        typeof value.real === "number" &&
        Number.isFinite(value.real) &&
        typeof value.imaginary === "number" &&
        Number.isFinite(value.imaginary)
      );
    case "logical":
      return typeof value.value === "boolean";
    case "text":
      return typeof value.value === "string";
    case "special":
      return (
        value.value === "nan" ||
        value.value === "infinity" ||
        value.value === "negativeInfinity"
      );
    case "missing":
      return true;
    default:
      return false;
  }
}

function selectedElementCount(size: readonly number[]): number | null {
  let count = 1;
  for (const extent of size) {
    count *= extent;
    if (!Number.isSafeInteger(count)) {
      return null;
    }
  }
  return count;
}

function isMatrixPreview(value: unknown): value is MatrixPreview {
  if (
    !isRecord(value) ||
    typeof value.class !== "string" ||
    value.class.length === 0 ||
    !isU64Array(value.dimensions) ||
    !isMatrixRange(value.selectedRange) ||
    value.dimensions.length !== value.selectedRange.start.length ||
    !Array.isArray(value.values) ||
    value.values.length > MAX_PREVIEW_ELEMENTS ||
    !value.values.every(isPreviewValue) ||
    !isRecord(value.truncation) ||
    typeof value.truncation.truncated !== "boolean" ||
    !isU64(value.truncation.omittedElements)
  ) {
    return false;
  }

  for (let index = 0; index < value.dimensions.length; index += 1) {
    const start = value.selectedRange.start[index];
    const size = value.selectedRange.size[index];
    const dimension = value.dimensions[index];
    if (start === undefined || size === undefined || dimension === undefined) {
      return false;
    }
    if (size > 0 && start + size - 1 > dimension) {
      return false;
    }
  }

  const selectedCount = selectedElementCount(value.selectedRange.size);
  if (selectedCount === null || value.values.length > selectedCount) {
    return false;
  }
  const omitted = selectedCount - value.values.length;
  return (
    value.truncation.truncated === (omitted > 0) &&
    value.truncation.omittedElements === omitted
  );
}

function isResponseData(type: KernelRequestType, data: unknown): boolean {
  if (!isRecord(data)) {
    return false;
  }

  switch (type) {
    case "initialize":
      return (
        typeof data.negotiatedProtocol === "string" &&
        isImplementationInfo(data.implementation) &&
        isCapabilities(data.capabilities)
      );
    case "execute":
      return typeof data.interrupted === "boolean";
    case "interrupt":
      return typeof data.accepted === "boolean";
    case "inspect":
      return isMatrixPreview(data);
    case "listWorkspace":
      return (
        Array.isArray(data.variables) && data.variables.every(isVariableSummary)
      );
    case "shutdown":
      return true;
  }
}

function isEventData(type: KernelEventType, data: unknown): boolean {
  if (!isRecord(data)) {
    return false;
  }

  switch (type) {
    case "status":
      return (
        typeof data.status === "string" &&
        statuses.has(data.status as KernelStatus)
      );
    case "stream":
      return (
        typeof data.stream === "string" &&
        streams.has(data.stream as StreamKind) &&
        typeof data.text === "string"
      );
    case "display":
      return (
        isRecord(data.representations) &&
        Object.values(data.representations).every(
          (representation) => typeof representation === "string",
        )
      );
    case "diagnostic":
      return isDiagnostic(data);
    case "workspaceDelta":
      return (
        Array.isArray(data.added) &&
        data.added.every(isVariableSummary) &&
        Array.isArray(data.changed) &&
        data.changed.every(isVariableSummary) &&
        isStringArray(data.removed)
      );
  }
}

export function parseKernelResponse(value: unknown): KernelResponse | null {
  if (
    !isRecord(value) ||
    !hasEnvelope(value, "response") ||
    typeof value.replyTo !== "string" ||
    value.replyTo.length === 0 ||
    typeof value.ok !== "boolean"
  ) {
    return null;
  }

  if (value.ok) {
    if (!isRecord(value.result) || value.error !== undefined) {
      return null;
    }
    const type = value.result.type;
    if (
      typeof type !== "string" ||
      !requestTypes.has(type as KernelRequestType) ||
      !isResponseData(type as KernelRequestType, value.result.data)
    ) {
      return null;
    }
  } else if (value.result !== undefined || !isProtocolError(value.error)) {
    return null;
  }

  return value as unknown as KernelResponse;
}

export function parseKernelServerMessage(
  value: unknown,
): ParsedKernelServerMessage | null {
  if (!isRecord(value)) {
    return null;
  }

  if (value.kind === "response") {
    const response = parseKernelResponse(value);
    return response === null
      ? null
      : { messageType: "response", response };
  }

  if (!hasEnvelope(value, "event") || !isRecord(value.event)) {
    return null;
  }
  const eventType = value.event.type;
  if (typeof eventType !== "string" || eventType.length === 0) {
    return null;
  }

  if (!knownEventTypes.has(eventType as KernelEventType)) {
    const data = value.event.data ?? null;
    if (!isJsonValue(data)) {
      return null;
    }
    const event =
      value.event.data === undefined
        ? { ...value, event: { ...value.event, data } }
        : value;
    return {
      messageType: "unknownEvent",
      event: event as unknown as UnknownKernelEvent,
    };
  }

  if (!isEventData(eventType as KernelEventType, value.event.data)) {
    return null;
  }
  return {
    messageType: "event",
    event: value as unknown as KernelEvent,
  };
}

export function parseKernelEvent(value: unknown): KernelEvent | null {
  const message = parseKernelServerMessage(value);
  return message?.messageType === "event" ? message.event : null;
}
