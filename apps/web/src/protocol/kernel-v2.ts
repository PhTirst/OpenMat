import {
  KERNEL_PROTOCOL as KERNEL_PROTOCOL_V0,
  MAX_PREVIEW_ELEMENTS,
  parseKernelServerMessage as parseV0KernelServerMessage,
  type Capabilities as V0Capabilities,
  type EventDataByType,
  type ImplementationInfo,
  type InitializeParams as V0InitializeParams,
  type InspectParams,
  type JsonValue,
  type KernelRequestType as V0KernelRequestType,
  type ProtocolError,
  type RequestParamsByType as V0RequestParamsByType,
  type ResponseDataByType as V0ResponseDataByType,
} from "./kernel-v0";
import {
  KERNEL_PROTOCOL_V1,
  MAX_PREVIEW_CODE_UNITS,
  MAX_STRING_ELEMENT_CODE_UNITS,
  hasValidV1Capabilities,
  parseKernelServerMessage as parseV1KernelServerMessage,
  validateBootstrapInitializeParams as validateV1BootstrapInitializeParams,
  type BootstrapInitializeParams as V1BootstrapInitializeParams,
  type IntegerClass,
  type MatrixPreview as VersionedMatrixPreview,
  type PreviewDecodeLimits as V1PreviewDecodeLimits,
  type PreviewValue as VersionedPreviewValue,
  type V1Capabilities,
  type V1MatrixPreview,
  type V1PreviewValue,
} from "./kernel-v1";

export {
  KERNEL_PROTOCOL_V0,
  KERNEL_PROTOCOL_V1,
  MAX_PREVIEW_CODE_UNITS,
  MAX_PREVIEW_ELEMENTS,
  MAX_STRING_ELEMENT_CODE_UNITS,
};
export type { IntegerClass, V1PreviewValue };

export const KERNEL_PROTOCOL_V2 = "openmat-kernel-v2" as const;
export const KERNEL_PROTOCOL_V3 = "openmat-kernel-v3" as const;
export const SUPPORTED_KERNEL_PROTOCOLS = [
  KERNEL_PROTOCOL_V3,
] as const;
export const MAX_AGGREGATE_NODES = 16_384;
export const MAX_AGGREGATE_ELEMENTS = 65_536;
export const MAX_AGGREGATE_DEPTH = 32;
export const MAX_KERNEL_FRAME_BYTES = 1_048_576;

export type KernelProtocol =
  | typeof KERNEL_PROTOCOL_V0
  | typeof KERNEL_PROTOCOL_V1
  | typeof KERNEL_PROTOCOL_V2
  | typeof KERNEL_PROTOCOL_V3;

export interface V2Capabilities extends V1Capabilities {
  readonly maxAggregateNodes: number;
  readonly maxAggregateElements: number;
  readonly maxAggregateDepth: number;
}

export type Capabilities = V0Capabilities | V1Capabilities | V2Capabilities;

export interface BootstrapInitializeParams
  extends Omit<V0InitializeParams, "supportedProtocols" | "capabilities"> {
  readonly supportedProtocols: readonly KernelProtocol[];
  readonly capabilities: V2Capabilities;
}

export interface PreviewDecodeLimits extends V1PreviewDecodeLimits {
  readonly maxAggregateNodes: number;
  readonly maxAggregateElements: number;
  readonly maxAggregateDepth: number;
}

export const V2_HARD_LIMITS: Readonly<PreviewDecodeLimits> = {
  maxPreviewElements: MAX_PREVIEW_ELEMENTS,
  maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
  maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
  maxAggregateNodes: MAX_AGGREGATE_NODES,
  maxAggregateElements: MAX_AGGREGATE_ELEMENTS,
  maxAggregateDepth: MAX_AGGREGATE_DEPTH,
};

// The public transport/UI union must retain v0 and v1 downgrade payloads.
// V2 non-aggregate decoding below still narrows successful v2 inspection to
// the exact V1MatrixPreview wire shape.
export type MatrixPreview = VersionedMatrixPreview;
export type PreviewValue = VersionedPreviewValue;

export interface PreviewTruncation {
  readonly truncated: boolean;
  readonly omittedElements: number;
}

export interface PreviewUsage {
  readonly nodes: number;
  readonly elements: number;
  readonly codeUnits: number;
  readonly depth: number;
}

export interface ExactValueBase {
  readonly class: string;
  readonly size: readonly number[];
  readonly ndims: number;
  readonly numel: number;
  readonly complex: boolean;
}

export interface NumericExactValue extends ExactValueBase {
  readonly kind: "numeric";
  readonly class: "double" | "single";
  readonly real: readonly string[];
  readonly imag: readonly string[];
}

export interface IntegerValue {
  readonly real: string;
  readonly imaginary: string;
}

export interface IntegerExactValue extends ExactValueBase {
  readonly kind: "integer";
  readonly class: IntegerClass;
  readonly integer: readonly IntegerValue[];
}

export interface LogicalExactValue extends ExactValueBase {
  readonly kind: "logical";
  readonly class: "logical";
  readonly complex: false;
  readonly logical: readonly boolean[];
}

export interface CharExactValue extends ExactValueBase {
  readonly kind: "char";
  readonly class: "char";
  readonly complex: false;
  readonly code_units: readonly number[];
}

export interface StringExactValue extends ExactValueBase {
  readonly kind: "string";
  readonly class: "string";
  readonly complex: false;
  readonly string_code_units: readonly (readonly number[])[];
  readonly missing: readonly boolean[];
}

export interface CellExactValue extends ExactValueBase {
  readonly kind: "cell";
  readonly class: "cell";
  readonly complex: false;
  readonly items: readonly ExactValue[];
}

export type StructRecord = Readonly<Record<string, ExactValue>>;

export interface StructExactValue extends ExactValueBase {
  readonly kind: "struct";
  readonly class: "struct";
  readonly complex: false;
  readonly fields: readonly string[];
  readonly records: readonly StructRecord[];
}

export interface TableExactValue extends ExactValueBase {
  readonly kind: "table";
  readonly class: "table";
  readonly complex: false;
  readonly size: readonly [number, number];
  readonly ndims: 2;
  readonly variableNames: readonly string[];
  readonly variables: readonly ExactValue[];
}

export type ExactValue =
  | NumericExactValue
  | IntegerExactValue
  | LogicalExactValue
  | CharExactValue
  | StringExactValue
  | CellExactValue
  | StructExactValue
  | TableExactValue;

interface AggregatePreviewBase {
  readonly dimensions: readonly number[];
  readonly complex: false;
  readonly selectedRange: {
    readonly start: readonly number[];
    readonly size: readonly number[];
  };
  readonly truncation: PreviewTruncation;
  readonly usage: PreviewUsage;
}

export interface CellAggregatePreview extends AggregatePreviewBase {
  readonly class: "cell";
  readonly kind: "cell";
  readonly items: readonly ExactValue[];
}

export interface StructAggregatePreview extends AggregatePreviewBase {
  readonly class: "struct";
  readonly kind: "struct";
  readonly fields: readonly string[];
  readonly records: readonly StructRecord[];
}

export interface TableAggregatePreview extends AggregatePreviewBase {
  readonly class: "table";
  readonly kind: "table";
  readonly dimensions: readonly [number, number];
  readonly selectedRange: {
    readonly start: readonly [number, number];
    readonly size: readonly [number, number];
  };
  readonly variableNames: readonly string[];
  readonly variables: readonly ExactValue[];
}

export type AggregatePreview =
  | CellAggregatePreview
  | StructAggregatePreview
  | TableAggregatePreview;
export type InspectPreview = MatrixPreview | AggregatePreview;

export interface VersionedInspectPreview {
  readonly revision: number;
  readonly preview: InspectPreview;
}

export interface VersionedWorkspaceSummary {
  readonly revision: number;
  readonly variables: readonly import("./kernel-v0").VariableSummary[];
}

export interface NumericScalar {
  readonly real: string;
  readonly imaginary: string;
}

export interface SetVariableElementParams {
  readonly name: string;
  readonly indices: readonly number[];
  readonly value: NumericScalar;
  readonly expectedRevision: number;
}

export interface SetVariableElementResult {
  readonly revision: number;
  readonly variable: import("./kernel-v0").VariableSummary;
}

export function validateSetVariableElementParams(
  params: SetVariableElementParams,
): string | null {
  if (!fieldNamePattern.test(params.name)) {
    return "Variable name must be an ASCII identifier.";
  }
  if (
    params.indices.length < 2 ||
    params.indices.some(
      (index) => !Number.isSafeInteger(index) || index <= 0,
    )
  ) {
    return "Variable indices must contain one positive safe integer per dimension.";
  }
  if (
    !Number.isSafeInteger(params.expectedRevision) ||
    params.expectedRevision < 0
  ) {
    return "Expected revision must be a non-negative safe integer.";
  }
  if (
    !numberStringPattern.test(params.value.real) ||
    !numberStringPattern.test(params.value.imaginary)
  ) {
    return "Numeric components must use canonical number strings.";
  }
  return null;
}

export type KernelRequestType = V0KernelRequestType | "setVariableElement";

type InitializeParams =
  | V0InitializeParams
  | V1BootstrapInitializeParams
  | BootstrapInitializeParams;

type RequestParamsFor<T extends KernelRequestType> = T extends "initialize"
  ? InitializeParams
  : T extends "setVariableElement"
    ? SetVariableElementParams
    : T extends V0KernelRequestType
      ? V0RequestParamsByType[T]
      : never;

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
    ? InspectPreview | VersionedInspectPreview
    : T extends "listWorkspace"
      ? V0ResponseDataByType["listWorkspace"] | VersionedWorkspaceSummary
      : T extends "setVariableElement"
        ? SetVariableElementResult
        : T extends V0KernelRequestType
          ? V0ResponseDataByType[T]
          : never;

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

export interface KernelFailureResponse {
  readonly protocol: KernelProtocol;
  readonly sessionId: string;
  readonly messageId: string;
  readonly kind: "response";
  readonly replyTo: string;
  readonly ok: false;
  readonly error: ProtocolError;
}

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

export interface UnknownKernelEvent {
  readonly protocol: KernelProtocol;
  readonly sessionId: string;
  readonly messageId: string;
  readonly kind: "event";
  readonly event: {
    readonly type: string;
    readonly data: JsonValue;
  };
}

export type ParsedKernelServerMessage =
  | { readonly messageType: "response"; readonly response: KernelResponse }
  | { readonly messageType: "event"; readonly event: KernelEvent }
  | {
      readonly messageType: "unknownEvent";
      readonly event: UnknownKernelEvent;
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
const integerPattern = /^(?:0|-[1-9][0-9]*|[1-9][0-9]*)$/;
const numberStringPattern =
  /^(?:NaN|\+Inf|-Inf|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)$/;
const fieldNamePattern = /^[A-Za-z][A-Za-z0-9_]*$/;

type MutableUsage = {
  nodes: number;
  elements: number;
  codeUnits: number;
  depth: number;
};

type ExactTraversalFrame =
  | { readonly type: "enter"; readonly value: unknown; readonly depth: number }
  | { readonly type: "exit"; readonly value: object }
  | {
      readonly type: "record";
      readonly record: unknown;
      readonly fields: readonly string[];
      readonly childDepth: number;
    };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
): boolean {
  const keys = Object.keys(value);
  if (keys.length !== expected.length) {
    return false;
  }
  const expectedKeys = new Set(expected);
  return keys.every((key) => expectedKeys.has(key));
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

function checkedAdd(left: number, right: number): number | null {
  if (
    !isSafeUnsignedInteger(left) ||
    !isSafeUnsignedInteger(right) ||
    left > Number.MAX_SAFE_INTEGER - right
  ) {
    return null;
  }
  return left + right;
}

function isCodeUnit(value: unknown): value is number {
  return isSafeUnsignedInteger(value) && value <= 65_535;
}

function isValidV0Capabilities(value: unknown): value is V0Capabilities {
  return (
    isRecord(value) &&
    Array.isArray(value.executionModes) &&
    value.executionModes.every(
      (mode) => mode === "file" || mode === "cell" || mode === "repl",
    ) &&
    Array.isArray(value.displayMimeTypes) &&
    value.displayMimeTypes.every((mime) => typeof mime === "string") &&
    isPositiveSafeInteger(value.maxPreviewElements) &&
    value.maxPreviewElements <= MAX_PREVIEW_ELEMENTS &&
    typeof value.interrupt === "boolean" &&
    typeof value.workspaceDelta === "boolean"
  );
}

function isValidV1Capabilities(value: unknown): value is V1Capabilities {
  return hasValidV1Capabilities(value as V1Capabilities);
}

function isValidPreviewDecodeLimits(
  value: unknown,
): value is PreviewDecodeLimits {
  if (!isRecord(value)) {
    return false;
  }
  return (
    isPositiveSafeInteger(value.maxPreviewElements) &&
    value.maxPreviewElements <= MAX_PREVIEW_ELEMENTS &&
    isPositiveSafeInteger(value.maxStringElementCodeUnits) &&
    value.maxStringElementCodeUnits <= MAX_STRING_ELEMENT_CODE_UNITS &&
    isPositiveSafeInteger(value.maxPreviewCodeUnits) &&
    value.maxPreviewCodeUnits <= MAX_PREVIEW_CODE_UNITS &&
    value.maxPreviewCodeUnits >= value.maxStringElementCodeUnits &&
    isPositiveSafeInteger(value.maxAggregateNodes) &&
    value.maxAggregateNodes <= MAX_AGGREGATE_NODES &&
    isPositiveSafeInteger(value.maxAggregateElements) &&
    value.maxAggregateElements <= MAX_AGGREGATE_ELEMENTS &&
    isSafeUnsignedInteger(value.maxAggregateDepth) &&
    value.maxAggregateDepth <= MAX_AGGREGATE_DEPTH
  );
}

export function hasValidV2Capabilities(
  capabilities: Capabilities,
): capabilities is V2Capabilities {
  if (!isRecord(capabilities) || !isValidV1Capabilities(capabilities)) {
    return false;
  }
  return (
    isPositiveSafeInteger(capabilities.maxAggregateNodes) &&
    capabilities.maxAggregateNodes <= MAX_AGGREGATE_NODES &&
    isPositiveSafeInteger(capabilities.maxAggregateElements) &&
    capabilities.maxAggregateElements <= MAX_AGGREGATE_ELEMENTS &&
    isSafeUnsignedInteger(capabilities.maxAggregateDepth) &&
    capabilities.maxAggregateDepth <= MAX_AGGREGATE_DEPTH
  );
}

export function validateBootstrapInitializeParams(
  params: InitializeParams,
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
    (params.supportedProtocols.includes(KERNEL_PROTOCOL_V2) ||
      params.supportedProtocols.includes(KERNEL_PROTOCOL_V3)) &&
    !hasValidV2Capabilities(params.capabilities as Capabilities)
  ) {
    return "A v2 or v3 offer requires all six valid preview and aggregate limits";
  }
  return validateV1BootstrapInitializeParams(
    params as V0InitializeParams | V1BootstrapInitializeParams,
  );
}

export function validateInspectRequestParams(
  params: InspectParams,
  limits: PreviewDecodeLimits = V2_HARD_LIMITS,
): string | null {
  if (!isValidPreviewDecodeLimits(limits) || !isRecord(params)) {
    return "inspect params and negotiated limits must be valid";
  }
  if (typeof params.name !== "string" || params.name.length === 0) {
    return "inspect name must be non-empty";
  }
  if (
    !isRecord(params.range) ||
    !hasExactKeys(params.range, ["start", "size"]) ||
    !isSafeUnsignedIntegerArray(params.range.start) ||
    !isSafeUnsignedIntegerArray(params.range.size) ||
    params.range.start.length < 2 ||
    params.range.start.length !== params.range.size.length ||
    !params.range.start.every((start) => start > 0) ||
    checkedProduct(params.range.size) === null
  ) {
    return "inspect range must be one-based with a safe checked product";
  }
  if (
    !isPositiveSafeInteger(params.maxElements) ||
    params.maxElements > limits.maxPreviewElements
  ) {
    return "inspect maxElements exceeds the negotiated preview limit";
  }
  return null;
}

function hasV1OnlyCapabilities(value: unknown): boolean {
  return (
    isRecord(value) &&
    (Object.hasOwn(value, "maxStringElementCodeUnits") ||
      Object.hasOwn(value, "maxPreviewCodeUnits"))
  );
}

function hasV2OnlyCapabilities(value: unknown): boolean {
  return (
    isRecord(value) &&
    (Object.hasOwn(value, "maxAggregateNodes") ||
      Object.hasOwn(value, "maxAggregateElements") ||
      Object.hasOwn(value, "maxAggregateDepth"))
  );
}

function negotiatedCapabilitiesFitOffer(
  negotiated: V1Capabilities | V2Capabilities,
  offered: V1Capabilities | V2Capabilities | undefined,
): boolean {
  if (offered === undefined) {
    return true;
  }
  if (
    negotiated.maxPreviewElements > offered.maxPreviewElements ||
    negotiated.maxStringElementCodeUnits >
      offered.maxStringElementCodeUnits ||
    negotiated.maxPreviewCodeUnits > offered.maxPreviewCodeUnits
  ) {
    return false;
  }
  if (hasValidV2Capabilities(negotiated)) {
    return (
      hasValidV2Capabilities(offered) &&
      negotiated.maxAggregateNodes <= offered.maxAggregateNodes &&
      negotiated.maxAggregateElements <= offered.maxAggregateElements &&
      negotiated.maxAggregateDepth <= offered.maxAggregateDepth
    );
  }
  return true;
}

function charge(
  usage: MutableUsage,
  property: "nodes" | "elements" | "codeUnits",
  amount: number,
  maximum: number,
): boolean {
  const next = checkedAdd(usage[property], amount);
  if (next === null || next > maximum) {
    return false;
  }
  usage[property] = next;
  return true;
}

function validateFields(
  value: unknown,
  usage: MutableUsage,
  limits: PreviewDecodeLimits,
): value is string[] {
  if (!Array.isArray(value)) {
    return false;
  }
  const unique = new Set<string>();
  for (const field of value) {
    if (
      typeof field !== "string" ||
      !fieldNamePattern.test(field) ||
      unique.has(field) ||
      !charge(usage, "codeUnits", field.length, limits.maxPreviewCodeUnits)
    ) {
      return false;
    }
    unique.add(field);
  }
  return true;
}

function validateTableVariableNames(
  value: unknown,
  width: number,
  usage: MutableUsage,
  limits: PreviewDecodeLimits,
): value is string[] {
  if (!Array.isArray(value) || value.length !== width) {
    return false;
  }
  const unique = new Set<string>();
  for (const name of value) {
    if (
      typeof name !== "string" ||
      name.length === 0 ||
      unique.has(name) ||
      !charge(usage, "codeUnits", name.length, limits.maxPreviewCodeUnits)
    ) {
      return false;
    }
    unique.add(name);
  }
  return true;
}

function validateRecord(
  value: unknown,
  fields: readonly string[],
): value is Record<string, unknown> {
  if (!isRecord(value)) {
    return false;
  }
  const keys = Object.keys(value);
  if (keys.length !== fields.length) {
    return false;
  }
  const fieldSet = new Set(fields);
  return (
    keys.every((key) => fieldSet.has(key)) &&
    fields.every((field) => Object.hasOwn(value, field))
  );
}

function isCanonicalIntegerComponent(
  value: unknown,
  integerClass: IntegerClass,
): value is string {
  if (typeof value !== "string" || !integerPattern.test(value)) {
    return false;
  }
  const [minimum, maximum] = integerBounds[integerClass];
  const negative = value.startsWith("-");
  const magnitude = negative ? value.slice(1) : value;
  const maximumMagnitude = (negative ? -minimum : maximum).toString();
  if (
    magnitude.length > maximumMagnitude.length ||
    (magnitude.length === maximumMagnitude.length &&
      magnitude > maximumMagnitude)
  ) {
    return false;
  }
  const parsed = BigInt(value);
  return parsed >= minimum && parsed <= maximum;
}

function numberStringIsZero(value: string): boolean {
  if (!numberStringPattern.test(value) || value === "NaN" || value.endsWith("Inf")) {
    return false;
  }
  const significand = value.split(/[eE]/u, 1)[0] ?? value;
  return [...significand].every(
    (character) => character === "-" || character === "0" || character === ".",
  );
}

function validateExactTree(
  root: unknown,
  rootDepth: number,
  usage: MutableUsage,
  limits: PreviewDecodeLimits,
): boolean {
  const active = new WeakSet<object>();
  const work: ExactTraversalFrame[] = [
    { type: "enter", value: root, depth: rootDepth },
  ];

  while (work.length > 0) {
    const frame = work.pop();
    if (frame === undefined) {
      return false;
    }
    if (frame.type === "exit") {
      active.delete(frame.value);
      continue;
    }
    if (frame.type === "record") {
      if (!validateRecord(frame.record, frame.fields)) {
        return false;
      }
      for (let index = frame.fields.length - 1; index >= 0; index -= 1) {
        const field = frame.fields[index];
        if (field === undefined) {
          return false;
        }
        work.push({
          type: "enter",
          value: frame.record[field],
          depth: frame.childDepth,
        });
      }
      continue;
    }

    const value = frame.value;
    if (!isRecord(value) || active.has(value)) {
      return false;
    }
    active.add(value);
    work.push({ type: "exit", value });

    const commonKeys = ["class", "size", "ndims", "numel", "complex", "kind"];
    const kind = value.kind;
    let variantKeys: readonly string[];
    switch (kind) {
      case "numeric":
        variantKeys = [...commonKeys, "real", "imag"];
        break;
      case "integer":
        variantKeys = [...commonKeys, "integer"];
        break;
      case "logical":
        variantKeys = [...commonKeys, "logical"];
        break;
      case "char":
        variantKeys = [...commonKeys, "code_units"];
        break;
      case "string":
        variantKeys = [...commonKeys, "string_code_units", "missing"];
        break;
      case "cell":
        variantKeys = [...commonKeys, "items"];
        break;
      case "struct":
        variantKeys = [...commonKeys, "fields", "records"];
        break;
      case "table":
        variantKeys = [...commonKeys, "variableNames", "variables"];
        break;
      default:
        return false;
    }

    if (
      !hasExactKeys(value, variantKeys) ||
      typeof value.class !== "string" ||
      value.class.length === 0 ||
      !isSafeUnsignedIntegerArray(value.size) ||
      value.size.length < 2 ||
      !isSafeUnsignedInteger(value.ndims) ||
      value.ndims !== value.size.length ||
      !isSafeUnsignedInteger(value.numel) ||
      typeof value.complex !== "boolean" ||
      frame.depth > limits.maxAggregateDepth
    ) {
      return false;
    }
    const product = checkedProduct(value.size);
    if (
      product === null ||
      value.numel !== product ||
      value.numel > limits.maxPreviewElements ||
      !charge(usage, "nodes", 1, limits.maxAggregateNodes) ||
      !charge(
        usage,
        "elements",
        value.numel,
        limits.maxAggregateElements,
      )
    ) {
      return false;
    }
    usage.depth = Math.max(usage.depth, frame.depth);

    switch (kind) {
      case "numeric": {
        if (
          (value.class !== "double" && value.class !== "single") ||
          !Array.isArray(value.real) ||
          value.real.length !== value.numel ||
          !value.real.every(
            (component) =>
              typeof component === "string" &&
              numberStringPattern.test(component),
          ) ||
          !Array.isArray(value.imag) ||
          value.imag.length !== value.numel ||
          !value.imag.every(
            (component) =>
              typeof component === "string" &&
              numberStringPattern.test(component),
          )
        ) {
          return false;
        }
        if (
          value.complex
            ? !value.imag.some(
                (component) =>
                  typeof component === "string" &&
                  !numberStringIsZero(component),
              )
            : !value.imag.every((component) => component === "0")
        ) {
          return false;
        }
        break;
      }
      case "integer": {
        if (
          !integerClasses.has(value.class as IntegerClass) ||
          !Array.isArray(value.integer) ||
          value.integer.length !== value.numel
        ) {
          return false;
        }
        const integerClass = value.class as IntegerClass;
        let hasNonzeroImaginary = false;
        for (const component of value.integer) {
          if (
            !isRecord(component) ||
            !hasExactKeys(component, ["real", "imaginary"]) ||
            !isCanonicalIntegerComponent(component.real, integerClass) ||
            !isCanonicalIntegerComponent(component.imaginary, integerClass)
          ) {
            return false;
          }
          hasNonzeroImaginary ||= component.imaginary !== "0";
          if (!value.complex && component.imaginary !== "0") {
            return false;
          }
        }
        if (value.complex && !hasNonzeroImaginary) {
          return false;
        }
        break;
      }
      case "logical":
        if (
          value.class !== "logical" ||
          value.complex ||
          !Array.isArray(value.logical) ||
          value.logical.length !== value.numel ||
          !value.logical.every((item) => typeof item === "boolean")
        ) {
          return false;
        }
        break;
      case "char":
        if (
          value.class !== "char" ||
          value.complex ||
          !Array.isArray(value.code_units) ||
          value.code_units.length !== value.numel ||
          !value.code_units.every(isCodeUnit) ||
          !charge(
            usage,
            "codeUnits",
            value.code_units.length,
            limits.maxPreviewCodeUnits,
          )
        ) {
          return false;
        }
        break;
      case "string": {
        if (
          value.class !== "string" ||
          value.complex ||
          !Array.isArray(value.string_code_units) ||
          value.string_code_units.length !== value.numel ||
          !Array.isArray(value.missing) ||
          value.missing.length !== value.numel ||
          !value.missing.every((item) => typeof item === "boolean")
        ) {
          return false;
        }
        for (let index = 0; index < value.string_code_units.length; index += 1) {
          const codeUnits = value.string_code_units[index];
          const missing = value.missing[index];
          if (
            !Array.isArray(codeUnits) ||
            codeUnits.length > limits.maxStringElementCodeUnits ||
            !codeUnits.every(isCodeUnit) ||
            (missing === true && codeUnits.length !== 0) ||
            !charge(
              usage,
              "codeUnits",
              codeUnits.length,
              limits.maxPreviewCodeUnits,
            )
          ) {
            return false;
          }
        }
        break;
      }
      case "cell": {
        if (
          value.class !== "cell" ||
          value.complex ||
          !Array.isArray(value.items) ||
          value.items.length !== value.numel
        ) {
          return false;
        }
        const childDepth = frame.depth + 1;
        for (let index = value.items.length - 1; index >= 0; index -= 1) {
          work.push({
            type: "enter",
            value: value.items[index],
            depth: childDepth,
          });
        }
        break;
      }
      case "struct": {
        if (
          value.class !== "struct" ||
          value.complex ||
          !Array.isArray(value.records) ||
          value.records.length !== value.numel ||
          !validateFields(value.fields, usage, limits)
        ) {
          return false;
        }
        const childDepth = frame.depth + 1;
        for (let index = value.records.length - 1; index >= 0; index -= 1) {
          work.push({
            type: "record",
            record: value.records[index],
            fields: value.fields,
            childDepth,
          });
        }
        break;
      }
      case "table": {
        const height = value.size[0];
        const width = value.size[1];
        if (
          value.class !== "table" ||
          value.complex ||
          value.size.length !== 2 ||
          height === undefined ||
          width === undefined ||
          !validateTableVariableNames(
            value.variableNames,
            width,
            usage,
            limits,
          ) ||
          !Array.isArray(value.variables) ||
          value.variables.length !== width
        ) {
          return false;
        }
        const childDepth = frame.depth + 1;
        for (let index = value.variables.length - 1; index >= 0; index -= 1) {
          const variable = value.variables[index];
          if (
            !isRecord(variable) ||
            !Array.isArray(variable.size) ||
            variable.size[0] !== height
          ) {
            return false;
          }
          work.push({ type: "enter", value: variable, depth: childDepth });
        }
        break;
      }
    }
  }
  return true;
}

function isValidRange(
  dimensions: readonly number[],
  value: unknown,
): value is { readonly start: number[]; readonly size: number[] } {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["start", "size"]) ||
    !isSafeUnsignedIntegerArray(value.start) ||
    !isSafeUnsignedIntegerArray(value.size) ||
    value.start.length !== dimensions.length ||
    value.size.length !== dimensions.length ||
    !value.start.every((start) => start > 0) ||
    checkedProduct(value.size) === null
  ) {
    return false;
  }
  for (let index = 0; index < dimensions.length; index += 1) {
    const dimension = dimensions[index];
    const start = value.start[index];
    const size = value.size[index];
    if (
      dimension === undefined ||
      start === undefined ||
      size === undefined ||
      (size > 0 && (start > dimension || size > dimension - start + 1))
    ) {
      return false;
    }
  }
  return true;
}

function isValidTruncation(value: unknown): value is PreviewTruncation {
  return (
    isRecord(value) &&
    hasExactKeys(value, ["truncated", "omittedElements"]) &&
    typeof value.truncated === "boolean" &&
    isSafeUnsignedInteger(value.omittedElements)
  );
}

function isValidUsage(value: unknown): value is PreviewUsage {
  return (
    isRecord(value) &&
    hasExactKeys(value, ["nodes", "elements", "codeUnits", "depth"]) &&
    isSafeUnsignedInteger(value.nodes) &&
    isSafeUnsignedInteger(value.elements) &&
    isSafeUnsignedInteger(value.codeUnits) &&
    isSafeUnsignedInteger(value.depth)
  );
}

function isAggregatePreview(
  value: unknown,
  limits: PreviewDecodeLimits,
  requestMaxElements: number,
): value is AggregatePreview {
  if (
    !isRecord(value) ||
    (value.kind !== "cell" &&
      value.kind !== "struct" &&
      value.kind !== "table") ||
    value.class !== value.kind ||
    value.complex !== false ||
    !isSafeUnsignedIntegerArray(value.dimensions) ||
    value.dimensions.length < 2 ||
    (value.kind === "table" && value.dimensions.length !== 2) ||
    checkedProduct(value.dimensions) === null ||
    !isValidRange(value.dimensions, value.selectedRange) ||
    !isValidTruncation(value.truncation) ||
    !isValidUsage(value.usage)
  ) {
    return false;
  }

  const commonKeys = [
    "class",
    "dimensions",
    "complex",
    "selectedRange",
    "kind",
    "truncation",
    "usage",
  ];
  const variantKeys =
    value.kind === "cell"
      ? [...commonKeys, "items"]
      : value.kind === "struct"
        ? [...commonKeys, "fields", "records"]
        : [...commonKeys, "variableNames", "variables"];
  if (!hasExactKeys(value, variantKeys)) {
    return false;
  }

  const returnedValue =
    value.kind === "cell"
      ? value.items
      : value.kind === "struct"
        ? value.records
        : value.variables;
  if (!Array.isArray(returnedValue)) {
    return false;
  }
  const returned = returnedValue.length;
  const selected =
    value.kind === "table"
      ? value.selectedRange.size[1]
      : checkedProduct(value.selectedRange.size);
  const represented = checkedAdd(returned, value.truncation.omittedElements);
  if (
    selected === undefined ||
    selected === null ||
    represented === null ||
    represented !== selected ||
    value.truncation.truncated !==
      (value.truncation.omittedElements !== 0) ||
    returned > Math.min(requestMaxElements, limits.maxPreviewElements)
  ) {
    return false;
  }

  const usage: MutableUsage = {
    nodes: 1,
    elements: 0,
    codeUnits: 0,
    depth: 0,
  };
  if (usage.nodes > limits.maxAggregateNodes) {
    return false;
  }

  if (value.kind === "cell") {
    for (const item of returnedValue) {
      if (
        !charge(usage, "elements", 1, limits.maxAggregateElements) ||
        !validateExactTree(item, 1, usage, limits)
      ) {
        return false;
      }
    }
  } else if (value.kind === "struct") {
    if (!validateFields(value.fields, usage, limits)) {
      return false;
    }
    for (const record of returnedValue) {
      if (
        !validateRecord(record, value.fields) ||
        !charge(usage, "elements", 1, limits.maxAggregateElements)
      ) {
        return false;
      }
      for (const field of value.fields) {
        if (!validateExactTree(record[field], 1, usage, limits)) {
          return false;
        }
      }
    }
  } else {
    if (!validateTableVariableNames(value.variableNames, returned, usage, limits)) {
      return false;
    }
    const selectedRows = value.selectedRange.size[0];
    if (selectedRows === undefined) {
      return false;
    }
    for (const variable of returnedValue) {
      if (
        !isRecord(variable) ||
        !Array.isArray(variable.size) ||
        variable.size[0] !== selectedRows ||
        !charge(usage, "elements", 1, limits.maxAggregateElements) ||
        !validateExactTree(variable, 1, usage, limits)
      ) {
        return false;
      }
    }
  }

  return (
    value.usage.nodes === usage.nodes &&
    value.usage.elements === usage.elements &&
    value.usage.codeUnits === usage.codeUnits &&
    value.usage.depth === usage.depth
  );
}

function parseMatrixPreview(
  value: unknown,
  limits: PreviewDecodeLimits,
  requestMaxElements: number,
): MatrixPreview | null {
  if (isRecord(value) && Object.hasOwn(value, "kind")) {
    return null;
  }
  const response = {
    protocol: KERNEL_PROTOCOL_V1,
    sessionId: "kernel-v2-codec",
    messageId: "matrix-preview",
    kind: "response",
    replyTo: "inspect-request",
    ok: true,
    result: { type: "inspect", data: value },
  };
  const parsed = parseV1KernelServerMessage(
    response,
    KERNEL_PROTOCOL_V1,
    {
      maxPreviewElements: Math.min(
        requestMaxElements,
        limits.maxPreviewElements,
      ),
      maxStringElementCodeUnits: limits.maxStringElementCodeUnits,
      maxPreviewCodeUnits: limits.maxPreviewCodeUnits,
    },
  );
  if (
    parsed?.messageType !== "response" ||
    !parsed.response.ok ||
    parsed.response.result.type !== "inspect"
  ) {
    return null;
  }
  return parsed.response.result.data as V1MatrixPreview;
}

export function parseInspectPreview(
  value: unknown,
  limits: PreviewDecodeLimits = V2_HARD_LIMITS,
  requestMaxElements: number = limits.maxPreviewElements,
): InspectPreview | null {
  if (
    !isValidPreviewDecodeLimits(limits) ||
    !isPositiveSafeInteger(requestMaxElements) ||
    requestMaxElements > limits.maxPreviewElements
  ) {
    return null;
  }
  if (isRecord(value) && Object.hasOwn(value, "kind")) {
    return isAggregatePreview(value, limits, requestMaxElements) ? value : null;
  }
  return parseMatrixPreview(value, limits, requestMaxElements);
}

function normalizeV1ParsedMessage(
  value: unknown,
  protocol: KernelProtocol,
  limits: PreviewDecodeLimits,
): ParsedKernelServerMessage | null {
  if (!isRecord(value) || value.protocol !== protocol) {
    return null;
  }
  const normalizedProtocol =
    protocol === KERNEL_PROTOCOL_V0
      ? KERNEL_PROTOCOL_V0
      : KERNEL_PROTOCOL_V1;
  const parsed = parseV1KernelServerMessage(
    { ...value, protocol: normalizedProtocol },
    normalizedProtocol,
    limits,
  );
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

function isVariableSummary(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    value.name.length > 0 &&
    typeof value.class === "string" &&
    value.class.length > 0 &&
    isSafeUnsignedIntegerArray(value.dimensions) &&
    value.dimensions.length >= 2 &&
    typeof value.complex === "boolean" &&
    (value.bytes === undefined || isSafeUnsignedInteger(value.bytes))
  );
}

function parseV3KernelServerMessage(
  value: Record<string, unknown>,
  limits: PreviewDecodeLimits,
  requestMaxElements: number,
): ParsedKernelServerMessage | null {
  if (value.kind !== "response" || value.ok !== true) {
    return normalizeV1ParsedMessage(value, KERNEL_PROTOCOL_V3, limits);
  }
  if (
    typeof value.sessionId !== "string" ||
    value.sessionId.length === 0 ||
    typeof value.messageId !== "string" ||
    value.messageId.length === 0 ||
    typeof value.replyTo !== "string" ||
    value.replyTo.length === 0 ||
    value.error !== undefined ||
    !isRecord(value.result) ||
    typeof value.result.type !== "string" ||
    !isRecord(value.result.data)
  ) {
    return null;
  }
  const { type, data } = value.result;
  if (type === "inspect") {
    if (
      !hasExactKeys(data, ["revision", "preview"]) ||
      !isSafeUnsignedInteger(data.revision) ||
      parseInspectPreview(data.preview, limits, requestMaxElements) === null
    ) {
      return null;
    }
  } else if (type === "listWorkspace") {
    if (
      !hasExactKeys(data, ["revision", "variables"]) ||
      !isSafeUnsignedInteger(data.revision) ||
      !Array.isArray(data.variables) ||
      !data.variables.every(isVariableSummary)
    ) {
      return null;
    }
  } else if (type === "setVariableElement") {
    if (
      !hasExactKeys(data, ["revision", "variable"]) ||
      !isSafeUnsignedInteger(data.revision) ||
      !isVariableSummary(data.variable)
    ) {
      return null;
    }
  } else {
    return normalizeV1ParsedMessage(value, KERNEL_PROTOCOL_V3, limits);
  }
  return {
    messageType: "response",
    response: value as unknown as KernelResponse,
  };
}

export function parseKernelServerMessage(
  value: unknown,
  protocol: KernelProtocol,
  limits: PreviewDecodeLimits = V2_HARD_LIMITS,
  requestMaxElements: number = limits.maxPreviewElements,
): ParsedKernelServerMessage | null {
  if (
    !isValidPreviewDecodeLimits(limits) ||
    !isRecord(value) ||
    value.protocol !== protocol
  ) {
    return null;
  }
  if (protocol === KERNEL_PROTOCOL_V3) {
    return parseV3KernelServerMessage(value, limits, requestMaxElements);
  }
  if (
    protocol === KERNEL_PROTOCOL_V2 &&
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
      parseInspectPreview(value.result.data, limits, requestMaxElements) === null
    ) {
      return null;
    }
    return {
      messageType: "response",
      response: value as unknown as KernelResponse,
    };
  }
  return normalizeV1ParsedMessage(value, protocol, limits);
}

function normalizeV0ParsedMessage(
  value: unknown,
): ParsedKernelServerMessage | null {
  const parsed = parseV0KernelServerMessage(value);
  if (parsed === null) {
    return null;
  }
  return parsed as unknown as ParsedKernelServerMessage;
}

export function parseBootstrapKernelServerMessage(
  value: unknown,
  offeredCapabilities?: V1Capabilities | V2Capabilities,
  offeredProtocols?: readonly string[],
): ParsedKernelServerMessage | null {
  const parsed = normalizeV0ParsedMessage(value);
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
  if (
    negotiatedProtocol === KERNEL_PROTOCOL_V2 ||
    negotiatedProtocol === KERNEL_PROTOCOL_V3
  ) {
    if (
      !hasValidV2Capabilities(capabilities) ||
      !negotiatedCapabilitiesFitOffer(capabilities, offeredCapabilities)
    ) {
      return null;
    }
  } else if (negotiatedProtocol === KERNEL_PROTOCOL_V1) {
    if (
      !isValidV1Capabilities(capabilities) ||
      hasV2OnlyCapabilities(capabilities) ||
      !negotiatedCapabilitiesFitOffer(capabilities, offeredCapabilities)
    ) {
      return null;
    }
  } else if (
    negotiatedProtocol !== KERNEL_PROTOCOL_V0 ||
    !isValidV0Capabilities(capabilities) ||
    hasV1OnlyCapabilities(capabilities) ||
    hasV2OnlyCapabilities(capabilities)
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

export function isKernelFrameWithinLimit(frame: string | Uint8Array): boolean {
  const byteLength =
    typeof frame === "string"
      ? new TextEncoder().encode(frame).byteLength
      : frame.byteLength;
  return byteLength <= MAX_KERNEL_FRAME_BYTES;
}

export function parseKernelServerTextFrame(
  frame: string,
  protocol: KernelProtocol,
  limits: PreviewDecodeLimits = V2_HARD_LIMITS,
  requestMaxElements: number = limits.maxPreviewElements,
): ParsedKernelServerMessage | null {
  if (!isKernelFrameWithinLimit(frame)) {
    return null;
  }
  let value: unknown;
  try {
    value = JSON.parse(frame) as unknown;
  } catch {
    return null;
  }
  return parseKernelServerMessage(
    value,
    protocol,
    limits,
    requestMaxElements,
  );
}
