import {
  KERNEL_PROTOCOL as KERNEL_PROTOCOL_V0,
  MAX_PREVIEW_ELEMENTS,
  parseKernelServerMessage as parseV0KernelServerMessage,
  type Capabilities as V0Capabilities,
  type EventDataByType,
  type ImplementationInfo,
  type InitializeParams as V0InitializeParams,
  type KernelRequestType,
  type MatrixPreview as V0MatrixPreview,
  type PreviewValue as V0PreviewValue,
  type ProtocolError,
  type RequestParamsByType as V0RequestParamsByType,
  type ResponseDataByType as V0ResponseDataByType,
} from "./kernel-v0";

export { KERNEL_PROTOCOL_V0, MAX_PREVIEW_ELEMENTS };

export const KERNEL_PROTOCOL_V1 = "openmat-kernel-v1" as const;
export const SUPPORTED_KERNEL_PROTOCOLS = [
  KERNEL_PROTOCOL_V1,
  KERNEL_PROTOCOL_V0,
] as const;
export const MAX_STRING_ELEMENT_CODE_UNITS = 16_384;
export const MAX_PREVIEW_CODE_UNITS = 65_536;

export type KernelProtocol =
  | typeof KERNEL_PROTOCOL_V0
  | typeof KERNEL_PROTOCOL_V1;

export interface V1Capabilities extends V0Capabilities {
  readonly maxStringElementCodeUnits: number;
  readonly maxPreviewCodeUnits: number;
}

export type Capabilities = V0Capabilities | V1Capabilities;

export interface BootstrapInitializeParams
  extends Omit<V0InitializeParams, "supportedProtocols" | "capabilities"> {
  readonly supportedProtocols: readonly KernelProtocol[];
  readonly capabilities: V1Capabilities;
}

export type IntegerClass =
  | "int8"
  | "uint8"
  | "int16"
  | "uint16"
  | "int32"
  | "uint32"
  | "int64"
  | "uint64";
export type V1PreviewClass =
  | "char"
  | "string"
  | "logical"
  | IntegerClass
  | "double"
  | "single";

export type V1PreviewValue =
  | { readonly kind: "number"; readonly value: number }
  | {
      readonly kind: "complex";
      readonly real: number;
      readonly imaginary: number;
    }
  | { readonly kind: "logical"; readonly value: boolean }
  | {
      readonly kind: "special";
      readonly value: "nan" | "infinity" | "negativeInfinity";
    }
  | { readonly kind: "charCodeUnit"; readonly value: number }
  | {
      readonly kind: "integer";
      readonly real: string;
      readonly imaginary: string;
    }
  | {
      readonly kind: "string";
      readonly codeUnits: readonly number[];
      readonly missing: boolean;
    };

export interface V1MatrixPreview {
  readonly class: V1PreviewClass;
  readonly dimensions: readonly number[];
  readonly complex: boolean;
  readonly selectedRange: {
    readonly start: readonly number[];
    readonly size: readonly number[];
  };
  readonly values: readonly V1PreviewValue[];
  readonly truncation: {
    readonly truncated: boolean;
    readonly omittedElements: number;
  };
}

export type MatrixPreview = V0MatrixPreview | V1MatrixPreview;
export type PreviewValue = V0PreviewValue | V1PreviewValue;

type RequestParamsFor<T extends KernelRequestType> = T extends "initialize"
  ? V0InitializeParams | BootstrapInitializeParams
  : V0RequestParamsByType[T];

export type KernelRequest<T extends KernelRequestType = KernelRequestType> = {
  readonly [K in T]: {
    readonly protocol: KernelProtocol;
    readonly sessionId: string;
    readonly messageId: string;
    readonly kind: "request";
    readonly request: {
      readonly type: K;
      readonly params: RequestParamsFor<K>;
    };
  };
}[T];

type ResponseDataFor<T extends KernelRequestType> = T extends "initialize"
  ? {
      readonly negotiatedProtocol: KernelProtocol;
      readonly implementation: ImplementationInfo;
      readonly capabilities: Capabilities;
    }
  : T extends "inspect"
    ? MatrixPreview
    : V0ResponseDataByType[T];

export type KernelSuccessResponse<
  T extends KernelRequestType = KernelRequestType,
> = {
  readonly [K in T]: {
    readonly protocol: KernelProtocol;
    readonly sessionId: string;
    readonly messageId: string;
    readonly kind: "response";
    readonly replyTo: string;
    readonly ok: true;
    readonly result: {
      readonly type: K;
      readonly data: ResponseDataFor<K>;
    };
  };
}[T];

export type KernelFailureResponse = {
  readonly protocol: KernelProtocol;
  readonly sessionId: string;
  readonly messageId: string;
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

export type KernelEvent<E extends keyof EventDataByType = keyof EventDataByType> = {
  readonly [K in E]: {
    readonly protocol: KernelProtocol;
    readonly sessionId: string;
    readonly messageId: string;
    readonly kind: "event";
    readonly event: {
      readonly type: K;
      readonly data: EventDataByType[K];
    };
  };
}[E];

export type UnknownKernelEvent = {
  readonly protocol: KernelProtocol;
  readonly sessionId: string;
  readonly messageId: string;
  readonly kind: "event";
  readonly event: {
    readonly type: string;
    readonly data: import("./kernel-v0").JsonValue;
  };
};

export type ParsedKernelServerMessage =
  | { readonly messageType: "response"; readonly response: KernelResponse }
  | { readonly messageType: "event"; readonly event: KernelEvent }
  | {
      readonly messageType: "unknownEvent";
      readonly event: UnknownKernelEvent;
    };

export interface PreviewDecodeLimits {
  readonly maxPreviewElements: number;
  readonly maxStringElementCodeUnits: number;
  readonly maxPreviewCodeUnits: number;
}

const HARD_PREVIEW_LIMITS: PreviewDecodeLimits = {
  maxPreviewElements: MAX_PREVIEW_ELEMENTS,
  maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
  maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
};

const integerClasses = new Set<IntegerClass>([
  "int8",
  "uint8",
  "int16",
  "uint16",
  "int32",
  "uint32",
  "int64",
  "uint64",
]);
const v1Classes = new Set<V1PreviewClass>([
  "char",
  "string",
  "logical",
  ...integerClasses,
  "double",
  "single",
]);
const decimalPattern = /^(?:0|-[1-9][0-9]*|[1-9][0-9]*)$/;
const integerBounds: Readonly<
  Record<IntegerClass, readonly [minimum: bigint, maximum: bigint]>
> = {
  int8: [-128n, 127n],
  uint8: [0n, 255n],
  int16: [-32_768n, 32_767n],
  uint16: [0n, 65_535n],
  int32: [-2_147_483_648n, 2_147_483_647n],
  uint32: [0n, 4_294_967_295n],
  int64: [-9_223_372_036_854_775_808n, 9_223_372_036_854_775_807n],
  uint64: [0n, 18_446_744_073_709_551_615n],
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isSafeUnsignedInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function isPositiveSafeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) > 0;
}

function isSafeUnsignedIntegerArray(value: unknown): value is number[] {
  return Array.isArray(value) && value.every(isSafeUnsignedInteger);
}

function checkedProduct(extents: readonly number[]): number | null {
  let product = 1;
  for (const extent of extents) {
    if (extent === 0) {
      return 0;
    }
    if (product > Number.MAX_SAFE_INTEGER / extent) {
      return null;
    }
    product *= extent;
  }
  return product;
}

function isCodeUnit(value: unknown): value is number {
  return isSafeUnsignedInteger(value) && value <= 65_535;
}

function isCanonicalIntegerComponent(
  value: unknown,
  integerClass: IntegerClass,
): value is string {
  if (typeof value !== "string" || !decimalPattern.test(value)) {
    return false;
  }
  const parsed = BigInt(value);
  const [minimum, maximum] = integerBounds[integerClass];
  return parsed >= minimum && parsed <= maximum;
}

function isValidV1Capabilities(value: unknown): value is V1Capabilities {
  if (!isRecord(value)) {
    return false;
  }
  return (
    Array.isArray(value.executionModes) &&
    value.executionModes.every(
      (mode) => mode === "file" || mode === "cell" || mode === "repl",
    ) &&
    Array.isArray(value.displayMimeTypes) &&
    value.displayMimeTypes.every((mime) => typeof mime === "string") &&
    isPositiveSafeInteger(value.maxPreviewElements) &&
    value.maxPreviewElements <= MAX_PREVIEW_ELEMENTS &&
    isPositiveSafeInteger(value.maxStringElementCodeUnits) &&
    value.maxStringElementCodeUnits <= MAX_STRING_ELEMENT_CODE_UNITS &&
    isPositiveSafeInteger(value.maxPreviewCodeUnits) &&
    value.maxPreviewCodeUnits <= MAX_PREVIEW_CODE_UNITS &&
    value.maxPreviewCodeUnits >= value.maxStringElementCodeUnits &&
    typeof value.interrupt === "boolean" &&
    typeof value.workspaceDelta === "boolean"
  );
}

export function hasValidV1Capabilities(
  capabilities: Capabilities,
): capabilities is V1Capabilities {
  return isValidV1Capabilities(capabilities);
}

export function validateBootstrapInitializeParams(
  params: V0InitializeParams | BootstrapInitializeParams,
): string | null {
  if (
    !Array.isArray(params.supportedProtocols) ||
    params.supportedProtocols.length === 0 ||
    !params.supportedProtocols.every(
      (protocol) => typeof protocol === "string" && protocol.length > 0,
    ) ||
    new Set(params.supportedProtocols).size !== params.supportedProtocols.length
  ) {
    return "supportedProtocols must contain unique, non-empty identifiers";
  }
  if (
    params.supportedProtocols.includes(KERNEL_PROTOCOL_V1) &&
    !isValidV1Capabilities(params.capabilities)
  ) {
    return "A v1 offer requires valid code-unit capabilities";
  }
  return null;
}

function negotiatedCapabilitiesFitOffer(
  negotiated: V1Capabilities,
  offered: V1Capabilities | undefined,
): boolean {
  return (
    offered === undefined ||
    (negotiated.maxPreviewElements <= offered.maxPreviewElements &&
      negotiated.maxStringElementCodeUnits <=
        offered.maxStringElementCodeUnits &&
      negotiated.maxPreviewCodeUnits <= offered.maxPreviewCodeUnits)
  );
}

function hasV1OnlyCapabilities(value: V0Capabilities): boolean {
  const capabilities = value as unknown as Record<string, unknown>;
  return (
    Object.hasOwn(capabilities, "maxStringElementCodeUnits") ||
    Object.hasOwn(capabilities, "maxPreviewCodeUnits")
  );
}

function validatePreviewValueKinds(
  previewClass: V1PreviewClass,
  complex: boolean,
  values: readonly unknown[],
  limits: PreviewDecodeLimits,
): values is V1PreviewValue[] {
  let hasNonzeroImaginary = false;
  let totalCodeUnits = 0;

  for (const value of values) {
    if (!isRecord(value) || typeof value.kind !== "string") {
      return false;
    }

    if (previewClass === "char") {
      if (complex || value.kind !== "charCodeUnit" || !isCodeUnit(value.value)) {
        return false;
      }
      continue;
    }

    if (previewClass === "string") {
      if (
        complex ||
        value.kind !== "string" ||
        !Array.isArray(value.codeUnits) ||
        !value.codeUnits.every(isCodeUnit) ||
        value.codeUnits.length > limits.maxStringElementCodeUnits ||
        typeof value.missing !== "boolean" ||
        (value.missing && value.codeUnits.length !== 0)
      ) {
        return false;
      }
      totalCodeUnits += value.codeUnits.length;
      if (totalCodeUnits > limits.maxPreviewCodeUnits) {
        return false;
      }
      continue;
    }

    if (previewClass === "logical") {
      if (complex || value.kind !== "logical" || typeof value.value !== "boolean") {
        return false;
      }
      continue;
    }

    if (integerClasses.has(previewClass as IntegerClass)) {
      const integerClass = previewClass as IntegerClass;
      if (
        value.kind !== "integer" ||
        !isCanonicalIntegerComponent(value.real, integerClass) ||
        !isCanonicalIntegerComponent(value.imaginary, integerClass)
      ) {
        return false;
      }
      if (value.imaginary !== "0") {
        hasNonzeroImaginary = true;
      }
      continue;
    }

    if (complex) {
      if (
        value.kind !== "complex" ||
        typeof value.real !== "number" ||
        !Number.isFinite(value.real) ||
        typeof value.imaginary !== "number" ||
        !Number.isFinite(value.imaginary)
      ) {
        return false;
      }
      continue;
    }

    if (value.kind === "number") {
      if (typeof value.value !== "number" || !Number.isFinite(value.value)) {
        return false;
      }
      continue;
    }
    if (
      value.kind !== "special" ||
      (value.value !== "nan" &&
        value.value !== "infinity" &&
        value.value !== "negativeInfinity")
    ) {
      return false;
    }
  }

  if (integerClasses.has(previewClass as IntegerClass)) {
    return complex ? hasNonzeroImaginary : !hasNonzeroImaginary;
  }
  return true;
}

function isV1MatrixPreview(
  value: unknown,
  requestedLimits: PreviewDecodeLimits,
): value is V1MatrixPreview {
  if (
    !isPositiveSafeInteger(requestedLimits.maxPreviewElements) ||
    requestedLimits.maxPreviewElements > MAX_PREVIEW_ELEMENTS ||
    !isPositiveSafeInteger(requestedLimits.maxStringElementCodeUnits) ||
    requestedLimits.maxStringElementCodeUnits >
      MAX_STRING_ELEMENT_CODE_UNITS ||
    !isPositiveSafeInteger(requestedLimits.maxPreviewCodeUnits) ||
    requestedLimits.maxPreviewCodeUnits > MAX_PREVIEW_CODE_UNITS ||
    requestedLimits.maxPreviewCodeUnits <
      requestedLimits.maxStringElementCodeUnits
  ) {
    return false;
  }
  const limits: PreviewDecodeLimits = {
    maxPreviewElements: Math.min(
      requestedLimits.maxPreviewElements,
      MAX_PREVIEW_ELEMENTS,
    ),
    maxStringElementCodeUnits: Math.min(
      requestedLimits.maxStringElementCodeUnits,
      MAX_STRING_ELEMENT_CODE_UNITS,
    ),
    maxPreviewCodeUnits: Math.min(
      requestedLimits.maxPreviewCodeUnits,
      MAX_PREVIEW_CODE_UNITS,
    ),
  };
  if (
    !isRecord(value) ||
    typeof value.class !== "string" ||
    !v1Classes.has(value.class as V1PreviewClass) ||
    !isSafeUnsignedIntegerArray(value.dimensions) ||
    value.dimensions.length < 2 ||
    checkedProduct(value.dimensions) === null ||
    typeof value.complex !== "boolean" ||
    !isRecord(value.selectedRange) ||
    !isSafeUnsignedIntegerArray(value.selectedRange.start) ||
    !isSafeUnsignedIntegerArray(value.selectedRange.size) ||
    value.selectedRange.start.length !== value.dimensions.length ||
    value.selectedRange.size.length !== value.dimensions.length ||
    !value.selectedRange.start.every((start) => start > 0) ||
    !Array.isArray(value.values) ||
    value.values.length > limits.maxPreviewElements ||
    !isRecord(value.truncation) ||
    typeof value.truncation.truncated !== "boolean" ||
    !isSafeUnsignedInteger(value.truncation.omittedElements)
  ) {
    return false;
  }

  const selectedCount = checkedProduct(value.selectedRange.size);
  if (selectedCount === null) {
    return false;
  }
  for (let index = 0; index < value.dimensions.length; index += 1) {
    const dimension = value.dimensions[index];
    const start = value.selectedRange.start[index];
    const size = value.selectedRange.size[index];
    if (
      dimension === undefined ||
      start === undefined ||
      size === undefined ||
      (size > 0 && (start > dimension || size > dimension - start + 1))
    ) {
      return false;
    }
  }

  if (
    value.values.length > selectedCount ||
    value.truncation.omittedElements !== selectedCount - value.values.length ||
    value.truncation.truncated !== (value.truncation.omittedElements !== 0) ||
    !validatePreviewValueKinds(
      value.class as V1PreviewClass,
      value.complex,
      value.values,
      limits,
    )
  ) {
    return false;
  }
  return true;
}

function normalizeV0ParsedMessage(
  value: unknown,
  protocol: KernelProtocol,
): ParsedKernelServerMessage | null {
  if (!isRecord(value) || value.protocol !== protocol) {
    return null;
  }
  const parsed = parseV0KernelServerMessage({
    ...value,
    protocol: KERNEL_PROTOCOL_V0,
  });
  if (parsed === null) {
    return null;
  }
  if (parsed.messageType === "response") {
    return {
      messageType: "response",
      response: { ...parsed.response, protocol } as KernelResponse,
    };
  }
  if (parsed.messageType === "event") {
    return {
      messageType: "event",
      event: { ...parsed.event, protocol } as KernelEvent,
    };
  }
  return {
    messageType: "unknownEvent",
    event: { ...parsed.event, protocol } as UnknownKernelEvent,
  };
}

export function parseKernelServerMessage(
  value: unknown,
  protocol: KernelProtocol,
  limits: PreviewDecodeLimits = HARD_PREVIEW_LIMITS,
): ParsedKernelServerMessage | null {
  if (!isRecord(value) || value.protocol !== protocol) {
    return null;
  }

  if (
    protocol === KERNEL_PROTOCOL_V1 &&
    value.kind === "response" &&
    value.ok === true &&
    isRecord(value.result) &&
    value.result.type === "inspect"
  ) {
    if (
      typeof value.sessionId !== "string" ||
      value.sessionId.length === 0 ||
      typeof value.messageId !== "string" ||
      value.messageId.length === 0 ||
      typeof value.replyTo !== "string" ||
      value.replyTo.length === 0 ||
      value.error !== undefined ||
      !isV1MatrixPreview(value.result.data, limits)
    ) {
      return null;
    }
    return {
      messageType: "response",
      response: value as unknown as KernelResponse,
    };
  }

  return normalizeV0ParsedMessage(value, protocol);
}

export function parseBootstrapKernelServerMessage(
  value: unknown,
  offeredCapabilities?: V1Capabilities,
  offeredProtocols?: readonly string[],
): ParsedKernelServerMessage | null {
  const parsed = normalizeV0ParsedMessage(value, KERNEL_PROTOCOL_V0);
  if (
    parsed?.messageType !== "response" ||
    !parsed.response.ok ||
    parsed.response.result.type !== "initialize"
  ) {
    return parsed;
  }

  const { negotiatedProtocol, capabilities } = parsed.response.result.data;
  if (
    offeredProtocols !== undefined &&
    !offeredProtocols.includes(negotiatedProtocol)
  ) {
    return null;
  }
  if (negotiatedProtocol === KERNEL_PROTOCOL_V1) {
    if (
      !isValidV1Capabilities(capabilities) ||
      !negotiatedCapabilitiesFitOffer(capabilities, offeredCapabilities)
    ) {
      return null;
    }
  } else if (
    negotiatedProtocol !== KERNEL_PROTOCOL_V0 ||
    hasV1OnlyCapabilities(capabilities)
  ) {
    return null;
  }
  return parsed;
}

export function createKernelRequest<T extends KernelRequestType>(
  protocol: KernelProtocol,
  sessionId: string,
  messageId: string,
  type: T,
  params: RequestParamsFor<T>,
): Extract<KernelRequest, { readonly request: { readonly type: T } }> {
  return {
    protocol,
    sessionId,
    messageId,
    kind: "request",
    request: { type, params },
  } as Extract<KernelRequest, { readonly request: { readonly type: T } }>;
}

export function createBootstrapInitializeRequest(
  sessionId: string,
  messageId: string,
  params: BootstrapInitializeParams,
): Extract<KernelRequest, { readonly request: { readonly type: "initialize" } }> {
  return createKernelRequest(
    KERNEL_PROTOCOL_V0,
    sessionId,
    messageId,
    "initialize",
    params,
  );
}
