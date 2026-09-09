import type { DisplayEventData } from "../protocol/kernel-v0";

export const GRAPHICS_PROTOCOL_V1 = "openmat-graphics-v1" as const;
export const GRAPHICS_PROTOCOL_V2 = "openmat-graphics-v2" as const;
export const GRAPHICS_PROTOCOL_V3 = "openmat-graphics-v3" as const;
export const GRAPHICS_PROTOCOL_V4 = "openmat-graphics-v4" as const;
/** Frozen v1 alias retained for existing imports and compatibility tests. */
export const GRAPHICS_PROTOCOL = GRAPHICS_PROTOCOL_V1;
export const FIGURE_MIME_TYPE =
  "application/vnd.openmat.figure+json" as const;
export const LEGACY_PLOT_MIME_TYPE =
  "application/vnd.openmat.plot+json" as const;
export const GRAPHICS_ENDPOINT_V1 = "/graphics/v1" as const;
export const GRAPHICS_ENDPOINT_V2 = "/graphics/v2" as const;
export const GRAPHICS_ENDPOINT_V3 = "/graphics/v3" as const;
export const GRAPHICS_ENDPOINT_V4 = "/graphics/v4" as const;
/** Frozen v1 alias retained for existing imports and compatibility tests. */
export const GRAPHICS_ENDPOINT = GRAPHICS_ENDPOINT_V1;

export const MAX_TEXT_FRAME_BYTES = 1_048_576;
export const MAX_BINARY_FRAME_BYTES = 8_388_608;
export const MAX_BUFFER_BYTES = 268_435_456;
export const MAX_RESIDENT_BYTES = 536_870_912;
export const MAX_OBJECTS = 100_000;
const MAX_BINARY_HEADER_BYTES = 65_536;
const BINARY_PREFIX_BYTES = 16;
const UINT64_MAX = 18_446_744_073_709_551_615n;

type JsonRecord = Record<string, unknown>;

export class GraphicsV1Error extends Error {
  readonly category:
    | "discovery"
    | "text"
    | "scene"
    | "binary"
    | "sequence";

  constructor(
    category: GraphicsV1Error["category"],
    message: string,
  ) {
    super(message);
    this.name = "GraphicsV1Error";
    this.category = category;
  }
}

export interface FigureDiscoveryV1 {
  readonly schemaVersion: 1;
  readonly graphicsProtocol: typeof GRAPHICS_PROTOCOL_V1;
  readonly endpoint: typeof GRAPHICS_ENDPOINT_V1;
  /** Secret capability: never render, log, or include in errors. */
  readonly attachToken: string;
  readonly figureId: string;
  readonly revision: number;
}

export interface FigureDiscoveryV2 {
  readonly schemaVersion: 2;
  readonly graphicsProtocol: typeof GRAPHICS_PROTOCOL_V2;
  readonly endpoint: typeof GRAPHICS_ENDPOINT_V2;
  /** Secret capability: never render, log, or include in errors. */
  readonly attachToken: string;
  readonly figureId: string;
  readonly revision: number;
}

export interface FigureDiscoveryV3 {
  readonly schemaVersion: 3;
  readonly graphicsProtocol: typeof GRAPHICS_PROTOCOL_V3;
  readonly endpoint: typeof GRAPHICS_ENDPOINT_V3;
  /** Secret capability: never render, log, or include in errors. */
  readonly attachToken: string;
  readonly figureId: string;
  readonly revision: number;
}

export interface FigureDiscoveryV4 {
  readonly schemaVersion: 4;
  readonly graphicsProtocol: typeof GRAPHICS_PROTOCOL_V4;
  readonly endpoint: typeof GRAPHICS_ENDPOINT_V4;
  /** Secret capability: never render, log, or include in errors. */
  readonly attachToken: string;
  readonly figureId: string;
  readonly revision: number;
}

export type FigureDiscovery =
  | FigureDiscoveryV1
  | FigureDiscoveryV2
  | FigureDiscoveryV3
  | FigureDiscoveryV4;
export type GraphicsProtocol = FigureDiscovery["graphicsProtocol"];
export type GraphicsEndpoint = FigureDiscovery["endpoint"];

export type DataType = "f32" | "f64";

export interface DataRef {
  readonly bufferId: string;
  readonly dtype: DataType;
  readonly shape: readonly [number, number];
  readonly order: "columnMajor";
  readonly endianness: "little";
  readonly byteOffset: 0;
  readonly byteLength: number;
}

export type GraphicsObjectKind =
  | "figure"
  | "axes2d"
  | "lineSeries"
  | "scatterSeries"
  | "surfaceSeries"
  | "patchSeries"
  | "chartGroup"
  | "text"
  | "legend"
  | "colorBar";

export interface GraphicsObject {
  readonly id: string;
  readonly generation: number;
  readonly objectRevision: number;
  readonly kind: GraphicsObjectKind;
  readonly parentId: string | null;
  readonly children: readonly string[];
  readonly properties: Readonly<JsonRecord>;
}

export interface FigureSnapshot {
  readonly type: "figureSnapshot";
  readonly figureId: string;
  readonly revision: number;
  readonly rootId: string;
  readonly objects: readonly GraphicsObject[];
  readonly referencedBuffers: readonly DataRef[];
}

export type DeltaOperation =
  | { readonly type: "upsertObject"; readonly object: GraphicsObject }
  | {
      readonly type: "deleteObject";
      readonly id: string;
      readonly generation: number;
    }
  | {
      readonly type: "reorderChildren";
      readonly parentId: string;
      readonly children: readonly string[];
    }
  | { readonly type: "setRoot"; readonly rootId: string };

export interface FigureDelta {
  readonly type: "figureDelta";
  readonly figureId: string;
  readonly baseRevision: number;
  readonly revision: number;
  readonly operations: readonly DeltaOperation[];
  readonly addedBuffers: readonly DataRef[];
  readonly releasedBufferIds: readonly string[];
}

export interface BufferChunk {
  readonly transferId: string;
  readonly bufferId: string;
  readonly offset: number;
  readonly totalBytes: number;
  readonly final: boolean;
  readonly payload: Uint8Array;
}

export function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function fail(
  category: GraphicsV1Error["category"],
  message: string,
): never {
  throw new GraphicsV1Error(category, message);
}

function requireRecord(
  value: unknown,
  field: string,
  category: GraphicsV1Error["category"],
): JsonRecord {
  if (!isRecord(value)) {
    fail(category, `${field} must be an object`);
  }
  return value;
}

function requireString(
  value: unknown,
  field: string,
  category: GraphicsV1Error["category"],
): string {
  if (typeof value !== "string" || value.length === 0) {
    fail(category, `${field} must be a non-empty string`);
  }
  return value;
}

function requireSafeInteger(
  value: unknown,
  field: string,
  category: GraphicsV1Error["category"],
  positive = false,
): number {
  if (
    !Number.isSafeInteger(value) ||
    (value as number) < (positive ? 1 : 0)
  ) {
    fail(
      category,
      `${field} must be a ${positive ? "positive " : ""}safe integer`,
    );
  }
  return value as number;
}

function requireFinite(
  value: unknown,
  field: string,
  predicate: (number: number) => boolean = () => true,
): number {
  if (
    typeof value !== "number" ||
    !Number.isFinite(value) ||
    !predicate(value)
  ) {
    fail("scene", `${field} is outside the graphics-v1 numeric domain`);
  }
  return value;
}

function requireBoolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") {
    fail("scene", `${field} must be Boolean`);
  }
  return value;
}

function requireStringArray(
  value: unknown,
  field: string,
): readonly string[] {
  if (
    !Array.isArray(value) ||
    !value.every((item) => typeof item === "string" && item.length > 0) ||
    new Set(value).size !== value.length
  ) {
    fail("scene", `${field} must contain unique non-empty identifiers`);
  }
  return value;
}

function requireCodeUnits(value: unknown, field: string): readonly number[] {
  if (
    !Array.isArray(value) ||
    !value.every(
      (item) => Number.isInteger(item) && item >= 0 && item <= 65_535,
    )
  ) {
    fail("scene", `${field} must be an array of UTF-16 code units`);
  }
  return value as number[];
}

function requireFiniteTuple(
  value: unknown,
  length: number,
  field: string,
  predicate?: (number: number) => boolean,
): readonly number[] {
  if (!Array.isArray(value) || value.length !== length) {
    fail("scene", `${field} must contain ${length.toString()} numbers`);
  }
  return value.map((item, index) =>
    requireFinite(item, `${field}[${index.toString()}]`, predicate),
  );
}

function requireEnum<T extends string>(
  value: unknown,
  allowed: ReadonlySet<T>,
  field: string,
): T {
  if (typeof value !== "string" || !allowed.has(value as T)) {
    fail("scene", `${field} is unsupported by graphics-v1`);
  }
  return value as T;
}

function parseJson(encoded: string, category: "discovery" | "text"): unknown {
  if (new TextEncoder().encode(encoded).byteLength > MAX_TEXT_FRAME_BYTES) {
    fail(category, "graphics-v1 text frame exceeds the maximum size");
  }
  try {
    return JSON.parse(encoded) as unknown;
  } catch {
    fail(category, "graphics-v1 text is not valid JSON");
  }
}

export function parseFigureDiscovery(encoded: string): FigureDiscovery {
  const value = requireRecord(
    parseJson(encoded, "discovery"),
    "Figure discovery",
    "discovery",
  );
  const v1 =
    value.schemaVersion === 1 &&
    value.graphicsProtocol === GRAPHICS_PROTOCOL_V1 &&
    value.endpoint === GRAPHICS_ENDPOINT_V1;
  const v2 =
    value.schemaVersion === 2 &&
    value.graphicsProtocol === GRAPHICS_PROTOCOL_V2 &&
    value.endpoint === GRAPHICS_ENDPOINT_V2;
  const v3 =
    value.schemaVersion === 3 &&
    value.graphicsProtocol === GRAPHICS_PROTOCOL_V3 &&
    value.endpoint === GRAPHICS_ENDPOINT_V3;
  const v4 =
    value.schemaVersion === 4 &&
    value.graphicsProtocol === GRAPHICS_PROTOCOL_V4 &&
    value.endpoint === GRAPHICS_ENDPOINT_V4;
  if (!v1 && !v2 && !v3 && !v4) {
    fail(
      "discovery",
      "Figure discovery version, protocol, and endpoint are inconsistent or unsupported",
    );
  }
  const common = {
    attachToken: requireString(
      value.attachToken,
      "Figure discovery attachToken",
      "discovery",
    ),
    figureId: requireString(
      value.figureId,
      "Figure discovery figureId",
      "discovery",
    ),
    revision: requireSafeInteger(
      value.revision,
      "Figure discovery revision",
      "discovery",
      true,
    ),
  };
  return v4
    ? {
        schemaVersion: 4,
        graphicsProtocol: GRAPHICS_PROTOCOL_V4,
        endpoint: GRAPHICS_ENDPOINT_V4,
        ...common,
      }
    : v3
    ? {
        schemaVersion: 3,
        graphicsProtocol: GRAPHICS_PROTOCOL_V3,
        endpoint: GRAPHICS_ENDPOINT_V3,
        ...common,
      }
    : v2
    ? {
        schemaVersion: 2,
        graphicsProtocol: GRAPHICS_PROTOCOL_V2,
        endpoint: GRAPHICS_ENDPOINT_V2,
        ...common,
      }
    : {
        schemaVersion: 1,
        graphicsProtocol: GRAPHICS_PROTOCOL_V1,
        endpoint: GRAPHICS_ENDPOINT_V1,
        ...common,
      };
}

export function discoverFigure(
  display: DisplayEventData,
): FigureDiscovery | null {
  const encoded = display.representations[FIGURE_MIME_TYPE];
  return encoded === undefined ? null : parseFigureDiscovery(encoded);
}

export function graphicsWebSocketUrl(
  location: Pick<Location, "protocol" | "host"> = window.location,
  endpoint: GraphicsEndpoint = GRAPHICS_ENDPOINT_V1,
): string {
  if (location.protocol !== "http:" && location.protocol !== "https:") {
    fail("discovery", "graphics-v1 requires an HTTP(S) application origin");
  }
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${location.host}${endpoint}`;
}

export function resolveGraphicsWebSocketUrl(
  configuredKernelUrl: string | undefined,
  location: Pick<Location, "protocol" | "host"> = window.location,
  endpoint: GraphicsEndpoint = GRAPHICS_ENDPOINT_V1,
): string {
  const configured = configuredKernelUrl?.trim();
  if (configured === undefined || configured.length === 0) {
    return graphicsWebSocketUrl(location, endpoint);
  }
  let url: URL;
  try {
    url = new URL(configured);
  } catch {
    fail("discovery", "The configured Kernel WebSocket URL is invalid");
  }
  if (
    (url.protocol !== "ws:" && url.protocol !== "wss:") ||
    url.username.length > 0 ||
    url.password.length > 0 ||
    url.search.length > 0 ||
    url.hash.length > 0 ||
    url.pathname !== "/kernel"
  ) {
    fail("discovery", "graphics-v1 requires a clean Kernel /kernel WebSocket URL");
  }
  url.pathname = endpoint;
  return url.href;
}

export function parseTextEnvelope(
  encoded: string,
  protocol: GraphicsProtocol = GRAPHICS_PROTOCOL_V1,
): JsonRecord {
  const value = requireRecord(
    parseJson(encoded, "text"),
    "graphics-v1 envelope",
    "text",
  );
  if (value.protocol !== protocol) {
    fail("text", "graphics envelope protocol does not match discovery");
  }
  requireString(value.sessionId, "sessionId", "text");
  requireString(value.messageId, "messageId", "text");
  if (value.kind !== "response" && value.kind !== "event") {
    fail("text", "graphics-v1 server envelope kind is invalid");
  }
  return value;
}

export function parseDataRef(value: unknown, field = "DataRef"): DataRef {
  const data = requireRecord(value, field, "scene");
  const dtype = requireEnum(
    data.dtype,
    new Set<DataType>(["f32", "f64"]),
    `${field}.dtype`,
  );
  if (!Array.isArray(data.shape) || data.shape.length !== 2) {
    fail("scene", `${field}.shape must contain exactly two extents`);
  }
  const rows = requireSafeInteger(data.shape[0], `${field}.shape[0]`, "scene");
  const columns = requireSafeInteger(data.shape[1], `${field}.shape[1]`, "scene");
  const byteLength = requireSafeInteger(
    data.byteLength,
    `${field}.byteLength`,
    "scene",
  );
  const expectedLength = rows * columns * (dtype === "f32" ? 4 : 8);
  if (
    !Number.isSafeInteger(expectedLength) ||
    byteLength !== expectedLength ||
    byteLength > MAX_BUFFER_BYTES
  ) {
    fail("scene", `${field}.byteLength does not match shape and dtype`);
  }
  if (
    data.order !== "columnMajor" ||
    data.endianness !== "little" ||
    data.byteOffset !== 0
  ) {
    fail("scene", `${field} storage layout is unsupported by graphics-v1`);
  }
  return {
    bufferId: requireString(data.bufferId, `${field}.bufferId`, "scene"),
    dtype,
    shape: [rows, columns],
    order: "columnMajor",
    endianness: "little",
    byteOffset: 0,
    byteLength,
  };
}

const objectKinds = new Set<GraphicsObjectKind>([
  "figure",
  "axes2d",
  "lineSeries",
  "scatterSeries",
  "surfaceSeries",
  "patchSeries",
  "chartGroup",
  "text",
  "legend",
  "colorBar",
]);
const lineStyles = new Set(["none", "solid", "dash", "dot", "dashDot"]);
const markers = new Set([
  "none",
  "circle",
  "plus",
  "star",
  "point",
  "cross",
  "square",
  "diamond",
  "triangleUp",
  "triangleDown",
  "triangleLeft",
  "triangleRight",
  "pentagram",
  "hexagram",
]);

function requireRgba(value: unknown, field: string): void {
  requireFiniteTuple(value, 4, field, (channel) => channel >= 0 && channel <= 1);
}

function validateFigureProperties(properties: JsonRecord): void {
  requireSafeInteger(properties.number, "figure.number", "scene", true);
  requireCodeUnits(properties.nameCodeUnits, "figure.nameCodeUnits");
  requireBoolean(properties.numberTitle, "figure.numberTitle");
  requireBoolean(properties.visible, "figure.visible");
  requireRgba(properties.backgroundRgba, "figure.backgroundRgba");
  const initialSize = requireFiniteTuple(
    properties.initialLogicalSizeCssPixels,
    2,
    "figure.initialLogicalSizeCssPixels",
    (number) => number > 0,
  );
  const position = requireFiniteTuple(
    properties.positionCssPixels,
    4,
    "figure.positionCssPixels",
  );
  const width = position[2] ?? Number.NaN;
  const height = position[3] ?? Number.NaN;
  if (width <= 0 || height <= 0) {
    fail("scene", "figure.positionCssPixels width and height must be positive");
  }
  if (
    width !== initialSize[0] ||
    height !== initialSize[1]
  ) {
    fail(
      "scene",
      "figure.initialLogicalSizeCssPixels must match Position width and height",
    );
  }
  requireEnum(properties.nextPlot, new Set(["add", "new"]), "figure.nextPlot");
}

function validateAxesProperties(properties: JsonRecord): void {
  if ("coordinateSystem" in properties) {
    requireEnum(
      properties.coordinateSystem,
      new Set(["cartesian", "polar"]),
      "axes2d.coordinateSystem",
    );
  }
  if ("thetaAxisUnits" in properties) {
    requireEnum(
      properties.thetaAxisUnits,
      new Set(["degrees", "radians"]),
      "axes2d.thetaAxisUnits",
    );
  }
  if ("thetaDirection" in properties) {
    requireEnum(
      properties.thetaDirection,
      new Set(["counterclockwise", "clockwise"]),
      "axes2d.thetaDirection",
    );
  }
  if ("thetaZeroLocation" in properties) {
    requireEnum(
      properties.thetaZeroLocation,
      new Set(["right", "top", "left", "bottom"]),
      "axes2d.thetaZeroLocation",
    );
  }
  if ("rAxisLocation" in properties) {
    requireFinite(properties.rAxisLocation, "axes2d.rAxisLocation");
  }
  const position = requireFiniteTuple(
    properties.positionNormalized,
    4,
    "axes2d.positionNormalized",
    (number) => number >= 0,
  );
  if (
    (position[2] ?? 0) <= 0 ||
    (position[3] ?? 0) <= 0 ||
    (position[0] ?? 0) + (position[2] ?? 0) > 1 ||
    (position[1] ?? 0) + (position[3] ?? 0) > 1
  ) {
    fail("scene", "axes2d.positionNormalized is outside [0,1]^2");
  }
  if ("backgroundRgba" in properties && properties.backgroundRgba !== null) {
    requireRgba(properties.backgroundRgba, "axes2d.backgroundRgba");
  }
  for (const field of ["xScale", "yScale"] as const) {
    requireEnum(properties[field], new Set(["linear", "log"]), `axes2d.${field}`);
  }
  if (
    properties.coordinateSystem === "polar" &&
    (properties.xScale !== "linear" || properties.yScale !== "linear")
  ) {
    fail("scene", "polar axes require linear theta and radial scales");
  }
  if ("zScale" in properties) {
    requireEnum(properties.zScale, new Set(["linear", "log"]), "axes2d.zScale");
  }
  for (const field of ["xDirection", "yDirection", "zDirection"] as const) {
    if (field in properties) {
      requireEnum(properties[field], new Set(["normal", "reverse"]), `axes2d.${field}`);
    }
  }
  if ("visible" in properties) {
    requireBoolean(properties.visible, "axes2d.visible");
  }
  for (const [field, value] of [
    ["xLimits", properties.xLimits],
    ["yLimits", properties.yLimits],
  ] as const) {
    const limits = requireFiniteTuple(value, 2, `axes2d.${field}`);
    if ((limits[0] ?? 0) >= (limits[1] ?? 0)) {
      fail("scene", `axes2d.${field} must be strictly increasing`);
    }
    const scale = properties[field === "xLimits" ? "xScale" : "yScale"];
    if (scale === "log" && (limits[0] ?? 0) <= 0) {
      fail("scene", `axes2d.${field} must be positive for a logarithmic axis`);
    }
  }
  requireEnum(
    properties.xLimitsMode,
    new Set(["auto", "manual"]),
    "axes2d.xLimitsMode",
  );
  requireEnum(
    properties.yLimitsMode,
    new Set(["auto", "manual"]),
    "axes2d.yLimitsMode",
  );
  if ("zLimits" in properties) {
    const limits = requireFiniteTuple(properties.zLimits, 2, "axes2d.zLimits");
    if ((limits[0] ?? 0) >= (limits[1] ?? 0)) {
      fail("scene", "axes2d.zLimits must be strictly increasing");
    }
    if (properties.zScale === "log" && (limits[0] ?? 0) <= 0) {
      fail("scene", "axes2d.zLimits must be positive for a logarithmic axis");
    }
  }
  if ("cLimits" in properties) {
    const limits = requireFiniteTuple(properties.cLimits, 2, "axes2d.cLimits");
    if ((limits[0] ?? 0) >= (limits[1] ?? 0)) {
      fail("scene", "axes2d.cLimits must be strictly increasing");
    }
  }
  for (const modeField of ["zLimitsMode", "cLimitsMode"] as const) {
    if (modeField in properties) {
      requireEnum(
        properties[modeField],
        new Set(["auto", "manual"]),
        `axes2d.${modeField}`,
      );
    }
  }
  requireEnum(properties.nextPlot, new Set(["replace", "add"]), "axes2d.nextPlot");
  requireBoolean(properties.gridX, "axes2d.gridX");
  requireBoolean(properties.gridY, "axes2d.gridY");
  if ("gridZ" in properties) {
    requireBoolean(properties.gridZ, "axes2d.gridZ");
  }
  requireSafeInteger(
    properties.colorOrderIndex,
    "axes2d.colorOrderIndex",
    "scene",
    true,
  );
  for (const dimension of ["x", "y", "z"] as const) {
    const tickField = `${dimension}Tick` as const;
    if (tickField in properties) {
      validateTickValues(properties[tickField], `axes2d.${tickField}`);
    }
    const labelField = `${dimension}TickLabelCodeUnits` as const;
    if (labelField in properties) {
      if (!Array.isArray(properties[labelField])) {
        fail("scene", `axes2d.${labelField} must be an array`);
      }
      properties[labelField].forEach((label, index) =>
        requireCodeUnits(label, `axes2d.${labelField}[${index.toString()}]`),
      );
    }
    for (const modeField of [
      `${dimension}TickMode`,
      `${dimension}TickLabelMode`,
    ] as const) {
      if (modeField in properties) {
        requireEnum(
          properties[modeField],
          new Set(["auto", "manual"]),
          `axes2d.${modeField}`,
        );
      }
    }
  }
  if ("box" in properties) {
    requireBoolean(properties.box, "axes2d.box");
  }
  for (const field of ["fontSizeCssPx", "lineWidthCssPx"] as const) {
    if (field in properties) {
      requireFinite(properties[field], `axes2d.${field}`, (value) => value > 0);
    }
  }
  if ("tickLabelInterpreter" in properties) {
    validateInterpreter(properties.tickLabelInterpreter, "axes2d.tickLabelInterpreter");
  }
  if ("colorbarVisible" in properties) {
    requireBoolean(properties.colorbarVisible, "axes2d.colorbarVisible");
  }
  if ("view" in properties) {
    requireFiniteTuple(properties.view, 2, "axes2d.view");
  }
  if ("projection" in properties) {
    requireEnum(
      properties.projection,
      new Set(["orthographic", "perspective"]),
      "axes2d.projection",
    );
  }
  if ("cameraScale" in properties) {
    requireFinite(
      properties.cameraScale,
      "axes2d.cameraScale",
      (value) => value >= 1 / 45 && value <= 170 / 45,
    );
  }
  for (const field of ["dataAspectRatio", "plotBoxAspectRatio"] as const) {
    if (field in properties) {
      requireFiniteTuple(properties[field], 3, `axes2d.${field}`, (value) => value > 0);
    }
  }
  for (const field of ["dataAspectRatioMode", "plotBoxAspectRatioMode"] as const) {
    if (field in properties) {
      requireEnum(properties[field], new Set(["auto", "manual"]), `axes2d.${field}`);
    }
  }
  for (const field of ["titleId", "xLabelId", "yLabelId"] as const) {
    if (!(field in properties)) {
      fail("scene", `axes2d.${field} is required`);
    }
    if (properties[field] !== null) {
      requireString(properties[field], `axes2d.${field}`, "scene");
    }
  }
  if ("zLabelId" in properties && properties.zLabelId !== null) {
    requireString(properties.zLabelId, "axes2d.zLabelId", "scene");
  }
}

function validateTickValues(value: unknown, field: string): void {
  if (!Array.isArray(value)) {
    fail("scene", `${field} must be an array`);
  }
  const decoded = value.map((item, index) => {
    if (item === "Infinity") {
      return Number.POSITIVE_INFINITY;
    }
    if (item === "-Infinity") {
      return Number.NEGATIVE_INFINITY;
    }
    return requireFinite(item, `${field}[${index.toString()}]`);
  });
  if (decoded.some((item, index) => index > 0 && (decoded[index - 1] ?? 0) >= item)) {
    fail("scene", `${field} must be strictly increasing without duplicates`);
  }
}

function validateInterpreter(value: unknown, field: string): void {
  requireEnum(value, new Set(["tex", "latex", "none"]), field);
}

function validateLineProperties(properties: JsonRecord): void {
  const x = parseDataRef(properties.xData, "lineSeries.xData");
  const y = parseDataRef(properties.yData, "lineSeries.yData");
  if (x.shape[0] !== 1 || y.shape[0] !== 1 || x.shape[1] !== y.shape[1]) {
    fail("scene", "lineSeries X/Y lengths differ");
  }
  if ("zData" in properties && properties.zData !== null) {
    const z = parseDataRef(properties.zData, "lineSeries.zData");
    if (z.shape[0] !== 1 || z.shape[1] !== x.shape[1]) {
      fail("scene", "lineSeries X/Z lengths differ");
    }
  }
  requireRgba(properties.colorRgba, "lineSeries.colorRgba");
  requireFinite(properties.lineWidthCssPx, "lineSeries.lineWidthCssPx", (n) => n > 0);
  requireEnum(properties.lineStyle, lineStyles, "lineSeries.lineStyle");
  requireEnum(properties.marker, markers, "lineSeries.marker");
  requireFinite(properties.markerSizeCssPx, "lineSeries.markerSizeCssPx", (n) => n > 0);
  if ("markerIndices" in properties && properties.markerIndices !== null) {
    if (!Array.isArray(properties.markerIndices)) {
      fail("scene", "lineSeries.markerIndices must be null or an array");
    }
    properties.markerIndices.forEach((index, position) => {
      if (
        typeof index !== "string" ||
        !/^[1-9][0-9]*$/u.test(index) ||
        BigInt(index) > UINT64_MAX
      ) {
        fail(
          "scene",
          `lineSeries.markerIndices[${position.toString()}] must be a positive uint64 decimal string`,
        );
      }
    });
  }
  requireCodeUnits(properties.displayNameCodeUnits, "lineSeries.displayNameCodeUnits");
  requireBoolean(properties.visible, "lineSeries.visible");
  requireBoolean(properties.clipping, "lineSeries.clipping");
}

function optionalDataRef(
  value: unknown,
  field: string,
  expectedElements: number,
): void {
  if (value === null) {
    return;
  }
  const ref = parseDataRef(value, field);
  const elements = ref.shape[0] * ref.shape[1];
  if (
    (ref.shape[0] !== 1 && ref.shape[1] !== 1) ||
    (elements !== 1 && elements !== expectedElements)
  ) {
    fail("scene", `${field} must be scalar or match the series length`);
  }
}

function validateScatterProperties(properties: JsonRecord): void {
  const x = parseDataRef(properties.xData, "scatterSeries.xData");
  const y = parseDataRef(properties.yData, "scatterSeries.yData");
  if (x.shape[0] !== 1 || y.shape[0] !== 1 || x.shape[1] !== y.shape[1]) {
    fail("scene", "scatterSeries X/Y lengths differ");
  }
  if ("zData" in properties && properties.zData !== null) {
    const z = parseDataRef(properties.zData, "scatterSeries.zData");
    if (z.shape[0] !== 1 || z.shape[1] !== x.shape[1]) {
      fail("scene", "scatterSeries X/Z lengths differ");
    }
  }
  if (!("sizeData" in properties) || !("colorData" in properties)) {
    fail("scene", "scatterSeries nullable data members are required");
  }
  optionalDataRef(properties.sizeData, "scatterSeries.sizeData", x.shape[1]);
  optionalDataRef(properties.colorData, "scatterSeries.colorData", x.shape[1]);
  if ("colorDataTarget" in properties) {
    requireEnum(
      properties.colorDataTarget,
      new Set(["none", "face", "edge"]),
      "scatterSeries.colorDataTarget",
    );
  }
  if (requireEnum(properties.marker, markers, "scatterSeries.marker") === "none") {
    fail("scene", "scatterSeries marker cannot be none");
  }
  requireFinite(properties.markerSizeCssPx, "scatterSeries.markerSizeCssPx", (n) => n > 0);
  requireRgba(properties.markerFaceRgba, "scatterSeries.markerFaceRgba");
  requireRgba(properties.markerEdgeRgba, "scatterSeries.markerEdgeRgba");
  requireCodeUnits(properties.displayNameCodeUnits, "scatterSeries.displayNameCodeUnits");
  requireBoolean(properties.visible, "scatterSeries.visible");
  requireBoolean(properties.clipping, "scatterSeries.clipping");
}

function validateSurfaceColor(
  modeValue: unknown,
  rgbaValue: unknown,
  field: string,
): void {
  const mode = requireEnum(
    modeValue,
    new Set(["flat", "interp", "none", "uniform"]),
    `${field}Color`,
  );
  if (mode === "uniform") {
    requireRgba(rgbaValue, `${field}Rgba`);
  } else if (rgbaValue !== null) {
    fail("scene", `${field}Rgba must be null unless ${field}Color is uniform`);
  }
}

function validateSurfaceProperties(properties: JsonRecord): void {
  const x = parseDataRef(properties.xData, "surfaceSeries.xData");
  const y = parseDataRef(properties.yData, "surfaceSeries.yData");
  const z = parseDataRef(properties.zData, "surfaceSeries.zData");
  const c = parseDataRef(properties.cData, "surfaceSeries.cData");
  const [rows, columns] = z.shape;
  const vectorCoordinates =
    x.shape[0] === 1 &&
    x.shape[1] === columns &&
    y.shape[0] === rows &&
    y.shape[1] === 1;
  const matrixCoordinates =
    x.shape[0] === rows &&
    x.shape[1] === columns &&
    y.shape[0] === rows &&
    y.shape[1] === columns;
  if (
    rows < 2 ||
    columns < 2 ||
    (!vectorCoordinates && !matrixCoordinates) ||
    c.shape[0] !== rows ||
    c.shape[1] !== columns
  ) {
    fail("scene", "surfaceSeries X/Y/Z/CData shapes are inconsistent");
  }
  validateSurfaceColor(properties.faceColor, properties.faceRgba, "surfaceSeries.face");
  validateSurfaceColor(properties.edgeColor, properties.edgeRgba, "surfaceSeries.edge");
  requireFinite(properties.lineWidthCssPx, "surfaceSeries.lineWidthCssPx", (n) => n > 0);
  if ("lineStyle" in properties) {
    requireEnum(properties.lineStyle, lineStyles, "surfaceSeries.lineStyle");
  }
  requireEnum(
    properties.cDataMapping,
    new Set(["scaled", "direct"]),
    "surfaceSeries.cDataMapping",
  );
  if (requireFinite(properties.faceAlpha, "surfaceSeries.faceAlpha") !== 1) {
    fail("scene", "the first surfaceSeries pipeline requires opaque faceAlpha");
  }
  if ("lightingEnabled" in properties) {
    requireBoolean(properties.lightingEnabled, "surfaceSeries.lightingEnabled");
  }
  requireBoolean(properties.visible, "surfaceSeries.visible");
  requireBoolean(properties.clipping, "surfaceSeries.clipping");
}

function validatePatchProperties(properties: JsonRecord): void {
  const faces = parseDataRef(properties.faces, "patchSeries.faces");
  const vertices = parseDataRef(properties.vertices, "patchSeries.vertices");
  const cdata = parseDataRef(
    properties.faceVertexCdata,
    "patchSeries.faceVertexCdata",
  );
  if (properties.vertexNormals !== undefined && properties.vertexNormals !== null) {
    const normals = parseDataRef(properties.vertexNormals, "patchSeries.vertexNormals");
    if (normals.shape[0] !== vertices.shape[0] || normals.shape[1] !== 3) {
      fail("scene", "patchSeries VertexNormals must be N-by-3 and match Vertices");
    }
  }
  if (faces.shape[0] < 1 || faces.shape[1] < 3) {
    fail("scene", "patchSeries Faces must be an M-by-N matrix with N >= 3");
  }
  if (
    vertices.shape[0] < 3 ||
    (vertices.shape[1] !== 2 && vertices.shape[1] !== 3)
  ) {
    fail("scene", "patchSeries Vertices must be an N-by-2 or N-by-3 matrix");
  }
  const cdataElements = cdata.shape[0] * cdata.shape[1];
  if (
    cdataElements !== 0 &&
    cdataElements !== faces.shape[0] &&
    cdataElements !== vertices.shape[0]
  ) {
    fail("scene", "patchSeries FaceVertexCData must have one value per face or vertex");
  }
  validateSurfaceColor(properties.faceColor, properties.faceRgba, "patchSeries.face");
  validateSurfaceColor(properties.edgeColor, properties.edgeRgba, "patchSeries.edge");
  const usesMappedColor =
    properties.faceColor === "flat" ||
    properties.faceColor === "interp" ||
    properties.edgeColor === "flat" ||
    properties.edgeColor === "interp";
  if (usesMappedColor && cdataElements === 0) {
    fail("scene", "mapped patchSeries colors require FaceVertexCData");
  }
  if (
    (properties.faceColor === "interp" || properties.edgeColor === "interp") &&
    cdataElements !== vertices.shape[0]
  ) {
    fail("scene", "interpolated patchSeries colors require one value per vertex");
  }
  requireFinite(properties.lineWidthCssPx, "patchSeries.lineWidthCssPx", (n) => n > 0);
  requireEnum(properties.lineStyle, lineStyles, "patchSeries.lineStyle");
  requireEnum(
    properties.cDataMapping,
    new Set(["scaled", "direct"]),
    "patchSeries.cDataMapping",
  );
  const faceAlpha = requireFinite(
    properties.faceAlpha,
    "patchSeries.faceAlpha",
    (value) => value >= 0 && value <= 1,
  );
  const edgeAlpha = requireFinite(
    properties.edgeAlpha,
    "patchSeries.edgeAlpha",
    (value) => value >= 0 && value <= 1,
  );
  if (vertices.shape[1] === 3 && (faceAlpha !== 1 || edgeAlpha !== 1)) {
    fail("scene", "the current 3D patchSeries pipeline requires opaque alpha");
  }
  if ("lightingEnabled" in properties) {
    requireBoolean(properties.lightingEnabled, "patchSeries.lightingEnabled");
  }
  if ("smoothNormals" in properties) {
    requireBoolean(properties.smoothNormals, "patchSeries.smoothNormals");
  }
  requireBoolean(properties.visible, "patchSeries.visible");
  requireBoolean(properties.clipping, "patchSeries.clipping");
}

function validateTextProperties(properties: JsonRecord): void {
  requireCodeUnits(properties.codeUnits, "text.codeUnits");
  requireEnum(
    properties.role,
    new Set(["title", "xLabel", "yLabel", "zLabel", "annotation"]),
    "text.role",
  );
  requireFiniteTuple(properties.anchorNormalized, 2, "text.anchorNormalized");
  requireEnum(
    properties.horizontalAlignment,
    new Set(["left", "center", "right"]),
    "text.horizontalAlignment",
  );
  requireEnum(
    properties.verticalAlignment,
    new Set(["top", "middle", "baseline", "bottom"]),
    "text.verticalAlignment",
  );
  requireRgba(properties.colorRgba, "text.colorRgba");
  requireCodeUnits(properties.fontFamilyCodeUnits, "text.fontFamilyCodeUnits");
  requireFinite(properties.fontSizeCssPx, "text.fontSizeCssPx", (n) => n > 0);
  const weight = requireSafeInteger(properties.fontWeight, "text.fontWeight", "scene", true);
  if (weight > 1_000) {
    fail("scene", "text.fontWeight must be in 1..1000");
  }
  requireEnum(properties.fontStyle, new Set(["normal", "italic"]), "text.fontStyle");
  if ("interpreter" in properties) {
    validateInterpreter(properties.interpreter, "text.interpreter");
  }
  requireBoolean(properties.visible, "text.visible");
}

function validateLegendProperties(properties: JsonRecord): void {
  const seriesIds = requireStringArray(properties.seriesIds, "legend.seriesIds");
  if (
    !Array.isArray(properties.labelCodeUnits) ||
    properties.labelCodeUnits.length !== seriesIds.length
  ) {
    fail("scene", "legend labels must match seriesIds");
  }
  properties.labelCodeUnits.forEach((label, index) =>
    requireCodeUnits(label, `legend.labelCodeUnits[${index.toString()}]`),
  );
  requireEnum(
    properties.location,
    new Set([
      "best",
      "north",
      "south",
      "east",
      "west",
      "northEast",
      "northWest",
      "southEast",
      "southWest",
      "southOutside",
    ]),
    "legend.location",
  );
  requireBoolean(properties.visible, "legend.visible");
  requireRgba(properties.backgroundRgba, "legend.backgroundRgba");
  requireRgba(properties.borderRgba, "legend.borderRgba");
  requireCodeUnits(properties.fontFamilyCodeUnits, "legend.fontFamilyCodeUnits");
  requireFinite(properties.fontSizeCssPx, "legend.fontSizeCssPx", (n) => n > 0);
  const weight = requireSafeInteger(properties.fontWeight, "legend.fontWeight", "scene", true);
  if (weight > 1_000) {
    fail("scene", "legend.fontWeight must be in 1..1000");
  }
  if ("interpreter" in properties) {
    validateInterpreter(properties.interpreter, "legend.interpreter");
  }
}

function validateColorBarProperties(properties: JsonRecord): void {
  requireBoolean(properties.visible, "colorBar.visible");
  if ("ticks" in properties) {
    validateTickValues(properties.ticks, "colorBar.ticks");
  }
  for (const modeField of ["ticksMode", "tickLabelsMode"] as const) {
    if (modeField in properties) {
      requireEnum(
        properties[modeField],
        new Set(["auto", "manual"]),
        `colorBar.${modeField}`,
      );
    }
  }
  if ("tickLabelCodeUnits" in properties) {
    if (!Array.isArray(properties.tickLabelCodeUnits)) {
      fail("scene", "colorBar.tickLabelCodeUnits must be an array");
    }
    properties.tickLabelCodeUnits.forEach((label, index) =>
      requireCodeUnits(label, `colorBar.tickLabelCodeUnits[${index.toString()}]`),
    );
  }
}

function validateChartGroupProperties(properties: JsonRecord): void {
  requireEnum(
    properties.chartType,
    new Set([
      "stair",
      "stem",
      "errorBar",
      "area",
      "bar",
      "histogram",
      "contour",
      "image",
    ]),
    "chartGroup.chartType",
  );
  requireBoolean(properties.visible, "chartGroup.visible");
  validateChartColor(properties.color, "chartGroup.color");
  requireFinite(properties.lineWidthCssPx, "chartGroup.lineWidthCssPx", (n) => n > 0);
  requireEnum(properties.lineStyle, lineStyles, "chartGroup.lineStyle");
  if (properties.marker !== null) {
    requireEnum(properties.marker, markers, "chartGroup.marker");
  }
  requireNullableFinitePositive(
    properties.markerSizeCssPx,
    "chartGroup.markerSizeCssPx",
  );
  validateChartColor(properties.markerFaceColor, "chartGroup.markerFaceColor");
  validateChartColor(properties.markerEdgeColor, "chartGroup.markerEdgeColor");
  validateChartColor(properties.faceColor, "chartGroup.faceColor");
  validateChartColor(properties.edgeColor, "chartGroup.edgeColor");
  for (const field of ["faceAlpha", "edgeAlpha"] as const) {
    if (properties[field] !== null) {
      requireFinite(properties[field], `chartGroup.${field}`, (n) => n >= 0 && n <= 1);
    }
  }
  if (properties.baseValue !== null) {
    requireFinite(properties.baseValue, "chartGroup.baseValue");
  }
  if (properties.barWidth !== null) {
    requireFinite(
      properties.barWidth,
      "chartGroup.barWidth",
      (n) => n > 0 && n <= 1,
    );
  }
  if (properties.capSizeCssPx !== null) {
    requireFinite(properties.capSizeCssPx, "chartGroup.capSizeCssPx", (n) => n >= 0);
  }
}

function validateChartColor(value: unknown, field: string): void {
  if (value === null) {
    return;
  }
  const color = requireRecord(value, field, "scene");
  const mode = requireEnum(
    color.mode,
    new Set(["auto", "none", "uniform"]),
    `${field}.mode`,
  );
  if (mode === "uniform") {
    requireRgba(color.rgba, `${field}.rgba`);
  } else if (color.rgba !== null) {
    fail("scene", `${field}.rgba must be null unless mode is uniform`);
  }
}

function requireNullableFinitePositive(value: unknown, field: string): void {
  if (value !== null) {
    requireFinite(value, field, (number) => number > 0);
  }
}

export function parseGraphicsObject(
  value: unknown,
  field = "object",
): GraphicsObject {
  const object = requireRecord(value, field, "scene");
  const kind = requireEnum(object.kind, objectKinds, `${field}.kind`);
  const parentId =
    object.parentId === null
      ? null
      : requireString(object.parentId, `${field}.parentId`, "scene");
  const properties = requireRecord(object.properties, `${field}.properties`, "scene");
  const parsed: GraphicsObject = {
    id: requireString(object.id, `${field}.id`, "scene"),
    generation: requireSafeInteger(
      object.generation,
      `${field}.generation`,
      "scene",
      true,
    ),
    objectRevision: requireSafeInteger(
      object.objectRevision,
      `${field}.objectRevision`,
      "scene",
      true,
    ),
    kind,
    parentId,
    children: requireStringArray(object.children, `${field}.children`),
    properties,
  };
  switch (kind) {
    case "figure":
      validateFigureProperties(properties);
      break;
    case "axes2d":
      validateAxesProperties(properties);
      break;
    case "lineSeries":
      validateLineProperties(properties);
      break;
    case "scatterSeries":
      validateScatterProperties(properties);
      break;
    case "surfaceSeries":
      validateSurfaceProperties(properties);
      break;
    case "patchSeries":
      validatePatchProperties(properties);
      break;
    case "chartGroup":
      validateChartGroupProperties(properties);
      break;
    case "text":
      validateTextProperties(properties);
      break;
    case "legend":
      validateLegendProperties(properties);
      break;
    case "colorBar":
      validateColorBarProperties(properties);
      break;
  }
  return parsed;
}

function dataRefsForObject(object: GraphicsObject): readonly DataRef[] {
  const properties = object.properties;
  if (object.kind === "lineSeries") {
    const refs = [
      parseDataRef(properties.xData, `${object.id}.xData`),
      parseDataRef(properties.yData, `${object.id}.yData`),
    ];
    if (properties.zData !== undefined && properties.zData !== null) {
      refs.push(parseDataRef(properties.zData, `${object.id}.zData`));
    }
    return refs;
  }
  if (object.kind === "scatterSeries") {
    const refs = [
      parseDataRef(properties.xData, `${object.id}.xData`),
      parseDataRef(properties.yData, `${object.id}.yData`),
    ];
    if (properties.zData !== undefined && properties.zData !== null) {
      refs.push(parseDataRef(properties.zData, `${object.id}.zData`));
    }
    if (properties.sizeData !== null) {
      refs.push(parseDataRef(properties.sizeData, `${object.id}.sizeData`));
    }
    if (properties.colorData !== null) {
      refs.push(parseDataRef(properties.colorData, `${object.id}.colorData`));
    }
    return refs;
  }
  if (object.kind === "surfaceSeries") {
    return [
      parseDataRef(properties.xData, `${object.id}.xData`),
      parseDataRef(properties.yData, `${object.id}.yData`),
      parseDataRef(properties.zData, `${object.id}.zData`),
      parseDataRef(properties.cData, `${object.id}.cData`),
    ];
  }
  if (object.kind === "patchSeries") {
    const refs = [
      parseDataRef(properties.faces, `${object.id}.faces`),
      parseDataRef(properties.vertices, `${object.id}.vertices`),
      parseDataRef(properties.faceVertexCdata, `${object.id}.faceVertexCdata`),
    ];
    if (properties.vertexNormals !== undefined && properties.vertexNormals !== null) {
      refs.push(parseDataRef(properties.vertexNormals, `${object.id}.vertexNormals`));
    }
    return refs;
  }
  return [];
}

function sameDataRef(left: DataRef, right: DataRef): boolean {
  return (
    left.bufferId === right.bufferId &&
    left.dtype === right.dtype &&
    left.shape[0] === right.shape[0] &&
    left.shape[1] === right.shape[1] &&
    left.order === right.order &&
    left.endianness === right.endianness &&
    left.byteOffset === right.byteOffset &&
    left.byteLength === right.byteLength
  );
}

function validateScene(snapshot: FigureSnapshot): void {
  if (snapshot.objects.length > MAX_OBJECTS) {
    fail("scene", "Figure exceeds the graphics-v1 object limit");
  }
  const objects = new Map<string, GraphicsObject>();
  for (const object of snapshot.objects) {
    if (objects.has(object.id)) {
      fail("scene", "Figure contains a duplicate object identifier");
    }
    objects.set(object.id, object);
  }
  const root = objects.get(snapshot.rootId);
  if (root?.kind !== "figure" || root.parentId !== null) {
    fail("scene", "Figure root must be a parentless figure object");
  }
  const allowedChildren: Record<GraphicsObjectKind, ReadonlySet<GraphicsObjectKind>> = {
    figure: new Set(["axes2d"]),
    axes2d: new Set([
      "lineSeries",
      "scatterSeries",
      "surfaceSeries",
      "patchSeries",
      "chartGroup",
      "text",
      "legend",
      "colorBar",
    ]),
    lineSeries: new Set(),
    scatterSeries: new Set(),
    surfaceSeries: new Set(),
    patchSeries: new Set(),
    chartGroup: new Set(["lineSeries", "patchSeries"]),
    text: new Set(),
    legend: new Set(),
    colorBar: new Set(),
  };
  for (const object of snapshot.objects) {
    if (object.id !== snapshot.rootId && object.parentId === null) {
      fail("scene", "Every non-root graphics object must have one parent");
    }
    if (
      (object.kind === "lineSeries" ||
        object.kind === "scatterSeries" ||
        object.kind === "surfaceSeries" ||
        object.kind === "patchSeries" ||
        object.kind === "text" ||
        object.kind === "legend" ||
        object.kind === "colorBar") &&
      object.children.length !== 0
    ) {
      fail("scene", "Leaf graphics objects cannot have children");
    }
    for (const childId of object.children) {
      const child = objects.get(childId);
      if (
        child === undefined ||
        child.parentId !== object.id ||
        !allowedChildren[object.kind].has(child.kind)
      ) {
        fail("scene", "Figure hierarchy contains an invalid parent/child edge");
      }
    }
    if (object.parentId !== null) {
      const parent = objects.get(object.parentId);
      if (parent === undefined || !parent.children.includes(object.id)) {
        fail("scene", "Figure hierarchy parent links are inconsistent");
      }
    }
  }
  const visited = new Set<string>();
  const visit = (id: string): void => {
    if (visited.has(id)) {
      fail("scene", "Figure hierarchy contains a cycle or duplicate edge");
    }
    visited.add(id);
    objects.get(id)?.children.forEach(visit);
  };
  visit(snapshot.rootId);
  if (visited.size !== objects.size) {
    fail("scene", "Figure hierarchy contains an unreachable object");
  }

  const listed = new Map<string, DataRef>();
  let residentBytes = 0;
  for (const descriptor of snapshot.referencedBuffers) {
    if (listed.has(descriptor.bufferId)) {
      fail("scene", "referencedBuffers contains a duplicate bufferId");
    }
    residentBytes += descriptor.byteLength;
    if (!Number.isSafeInteger(residentBytes) || residentBytes > MAX_RESIDENT_BYTES) {
      fail("scene", "Figure exceeds the graphics-v1 resident byte limit");
    }
    listed.set(descriptor.bufferId, descriptor);
  }
  const used = new Map<string, DataRef>();
  for (const object of snapshot.objects) {
    for (const descriptor of dataRefsForObject(object)) {
      const previous = used.get(descriptor.bufferId);
      if (previous !== undefined && !sameDataRef(previous, descriptor)) {
        fail("scene", "One bufferId has conflicting DataRef descriptors");
      }
      used.set(descriptor.bufferId, descriptor);
    }
  }
  if (used.size !== listed.size) {
    fail("scene", "referencedBuffers must exactly match used DataRefs");
  }
  for (const [bufferId, descriptor] of used) {
    const listedDescriptor = listed.get(bufferId);
    if (listedDescriptor === undefined || !sameDataRef(descriptor, listedDescriptor)) {
      fail("scene", "A used DataRef is absent or conflicts with referencedBuffers");
    }
  }
}

export function parseFigureSnapshot(value: unknown): FigureSnapshot {
  const snapshot = requireRecord(value, "FigureSnapshot", "scene");
  if (snapshot.type !== "figureSnapshot") {
    fail("scene", "FigureSnapshot.type must be figureSnapshot");
  }
  if (!Array.isArray(snapshot.objects) || !Array.isArray(snapshot.referencedBuffers)) {
    fail("scene", "FigureSnapshot object and buffer lists are required");
  }
  const parsed: FigureSnapshot = {
    type: "figureSnapshot",
    figureId: requireString(snapshot.figureId, "FigureSnapshot.figureId", "scene"),
    revision: requireSafeInteger(
      snapshot.revision,
      "FigureSnapshot.revision",
      "scene",
      true,
    ),
    rootId: requireString(snapshot.rootId, "FigureSnapshot.rootId", "scene"),
    objects: snapshot.objects.map((object, index) =>
      parseGraphicsObject(object, `FigureSnapshot.objects[${index.toString()}]`),
    ),
    referencedBuffers: snapshot.referencedBuffers.map((descriptor, index) =>
      parseDataRef(
        descriptor,
        `FigureSnapshot.referencedBuffers[${index.toString()}]`,
      ),
    ),
  };
  validateScene(parsed);
  return parsed;
}

function parseDeltaOperation(value: unknown, index: number): DeltaOperation {
  const operation = requireRecord(
    value,
    `figureDelta.operations[${index.toString()}]`,
    "scene",
  );
  switch (operation.type) {
    case "upsertObject":
      return {
        type: "upsertObject",
        object: parseGraphicsObject(operation.object, "upsertObject.object"),
      };
    case "deleteObject":
      return {
        type: "deleteObject",
        id: requireString(operation.id, "deleteObject.id", "scene"),
        generation: requireSafeInteger(
          operation.generation,
          "deleteObject.generation",
          "scene",
          true,
        ),
      };
    case "reorderChildren":
      return {
        type: "reorderChildren",
        parentId: requireString(
          operation.parentId,
          "reorderChildren.parentId",
          "scene",
        ),
        children: requireStringArray(
          operation.children,
          "reorderChildren.children",
        ),
      };
    case "setRoot":
      return {
        type: "setRoot",
        rootId: requireString(operation.rootId, "setRoot.rootId", "scene"),
      };
    default:
      fail("scene", "Figure delta operation is unsupported by graphics-v1");
  }
}

export function parseFigureDelta(value: unknown): FigureDelta {
  const delta = requireRecord(value, "figureDelta", "scene");
  if (delta.type !== "figureDelta") {
    fail("scene", "Figure delta type is invalid");
  }
  if (
    !Array.isArray(delta.operations) ||
    !Array.isArray(delta.addedBuffers) ||
    !Array.isArray(delta.releasedBufferIds)
  ) {
    fail("scene", "Figure delta lists are required");
  }
  const baseRevision = requireSafeInteger(
    delta.baseRevision,
    "figureDelta.baseRevision",
    "scene",
    true,
  );
  const revision = requireSafeInteger(
    delta.revision,
    "figureDelta.revision",
    "scene",
    true,
  );
  if (revision !== baseRevision + 1) {
    fail("sequence", "Figure delta revision is not exactly baseRevision + 1");
  }
  const addedBuffers = delta.addedBuffers.map((descriptor, index) =>
    parseDataRef(descriptor, `figureDelta.addedBuffers[${index.toString()}]`),
  );
  const addedIds = new Set(addedBuffers.map((descriptor) => descriptor.bufferId));
  if (addedIds.size !== addedBuffers.length) {
    fail("scene", "figureDelta.addedBuffers contains duplicates");
  }
  const releasedBufferIds = requireStringArray(
    delta.releasedBufferIds,
    "figureDelta.releasedBufferIds",
  );
  if (releasedBufferIds.some((id) => addedIds.has(id))) {
    fail("scene", "Figure delta added and released buffer lists overlap");
  }
  return {
    type: "figureDelta",
    figureId: requireString(delta.figureId, "figureDelta.figureId", "scene"),
    baseRevision,
    revision,
    operations: delta.operations.map(parseDeltaOperation),
    addedBuffers,
    releasedBufferIds,
  };
}

export function applyFigureDelta(
  previous: FigureSnapshot,
  delta: FigureDelta,
): FigureSnapshot {
  if (
    previous.figureId !== delta.figureId ||
    previous.revision !== delta.baseRevision
  ) {
    fail("sequence", "Figure delta does not continue the current revision");
  }
  const objects = new Map(previous.objects.map((object) => [object.id, object]));
  const descriptors = new Map(
    previous.referencedBuffers.map((descriptor) => [descriptor.bufferId, descriptor]),
  );
  for (const descriptor of delta.addedBuffers) {
    const existing = descriptors.get(descriptor.bufferId);
    if (existing !== undefined && !sameDataRef(existing, descriptor)) {
      fail("scene", "Figure delta changes an immutable buffer descriptor");
    }
    descriptors.set(descriptor.bufferId, descriptor);
  }
  let rootId = previous.rootId;
  for (const operation of delta.operations) {
    switch (operation.type) {
      case "upsertObject":
        objects.set(operation.object.id, operation.object);
        break;
      case "deleteObject": {
        const target = objects.get(operation.id);
        if (target === undefined || target.generation !== operation.generation) {
          fail("scene", "Figure delta deletes an unknown object generation");
        }
        const remove = (id: string): void => {
          const object = objects.get(id);
          object?.children.forEach(remove);
          objects.delete(id);
        };
        remove(operation.id);
        break;
      }
      case "reorderChildren": {
        const parent = objects.get(operation.parentId);
        if (parent === undefined) {
          fail("scene", "Figure delta reorders an unknown parent");
        }
        objects.set(parent.id, { ...parent, children: operation.children });
        break;
      }
      case "setRoot":
        rootId = operation.rootId;
        break;
    }
  }
  const usedIds = new Set<string>();
  for (const object of objects.values()) {
    dataRefsForObject(object).forEach((descriptor) => usedIds.add(descriptor.bufferId));
  }
  const referencedBuffers = [...usedIds].map((bufferId) => {
    const descriptor = descriptors.get(bufferId);
    if (descriptor === undefined) {
      fail("scene", "Figure delta references an unavailable buffer descriptor");
    }
    return descriptor;
  });
  const candidate: FigureSnapshot = {
    type: "figureSnapshot",
    figureId: previous.figureId,
    revision: delta.revision,
    rootId,
    objects: [...objects.values()],
    referencedBuffers,
  };
  validateScene(candidate);
  return candidate;
}

export function decodeOmgpFrame(
  input: ArrayBuffer | ArrayBufferView,
  maximumBytes = MAX_BINARY_FRAME_BYTES,
): BufferChunk {
  const bytes =
    input instanceof ArrayBuffer
      ? new Uint8Array(input)
      : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
  if (bytes.byteLength < BINARY_PREFIX_BYTES || bytes.byteLength > maximumBytes) {
    fail("binary", "OMGP frame length is outside the negotiated limit");
  }
  if (
    bytes[0] !== 0x4f ||
    bytes[1] !== 0x4d ||
    bytes[2] !== 0x47 ||
    bytes[3] !== 0x50
  ) {
    fail("binary", "OMGP frame magic is invalid");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint16(4, true) !== 1 || view.getUint16(6, true) !== 0) {
    fail("binary", "OMGP frame protocol version is unsupported");
  }
  const headerLength = view.getUint32(8, true);
  const payloadLength = view.getUint32(12, true);
  if (headerLength > MAX_BINARY_HEADER_BYTES) {
    fail("binary", "OMGP frame header exceeds the maximum size");
  }
  const expectedLength = BINARY_PREFIX_BYTES + headerLength + payloadLength;
  if (!Number.isSafeInteger(expectedLength) || expectedLength !== bytes.byteLength) {
    fail("binary", "OMGP frame lengths do not match the message");
  }
  let headerValue: unknown;
  try {
    const headerText = new TextDecoder("utf-8", { fatal: true }).decode(
      bytes.subarray(BINARY_PREFIX_BYTES, BINARY_PREFIX_BYTES + headerLength),
    );
    headerValue = JSON.parse(headerText) as unknown;
  } catch {
    fail("binary", "OMGP frame header is not valid UTF-8 JSON");
  }
  const header = requireRecord(headerValue, "OMGP header", "binary");
  if (header.type !== "bufferChunk") {
    fail("binary", "OMGP frame type is unsupported");
  }
  const offset = requireSafeInteger(header.offset, "OMGP offset", "binary");
  const totalBytes = requireSafeInteger(
    header.totalBytes,
    "OMGP totalBytes",
    "binary",
  );
  if (totalBytes > MAX_BUFFER_BYTES || offset + payloadLength > totalBytes) {
    fail("binary", "OMGP payload range is outside the transfer");
  }
  if (typeof header.final !== "boolean") {
    fail("binary", "OMGP final must be Boolean");
  }
  if (header.final !== (offset + payloadLength === totalBytes)) {
    fail("binary", "OMGP final flag does not match the payload range");
  }
  return {
    transferId: requireString(header.transferId, "OMGP transferId", "binary"),
    bufferId: requireString(header.bufferId, "OMGP bufferId", "binary"),
    offset,
    totalBytes,
    final: header.final,
    payload: bytes.slice(BINARY_PREFIX_BYTES + headerLength),
  };
}

export function utf16CodeUnitsToString(value: unknown): string {
  const codeUnits = requireCodeUnits(value, "text codeUnits");
  const chunks: string[] = [];
  for (let index = 0; index < codeUnits.length; index += 4_096) {
    chunks.push(String.fromCharCode(...codeUnits.slice(index, index + 4_096)));
  }
  return chunks.join("");
}

export function figureAccessibleName(snapshot: FigureSnapshot): string {
  const root = snapshot.objects.find((object) => object.id === snapshot.rootId);
  if (root?.kind === "figure") {
    const name = utf16CodeUnitsToString(root.properties.nameCodeUnits);
    const number = root.properties.number;
    if (root.properties.numberTitle && name.length > 0) {
      return `Figure ${String(number)}: ${name}`;
    }
    if (root.properties.numberTitle) {
      return `Figure ${String(number)}`;
    }
    return name.length > 0 ? name : "Figure";
  }
  return "OpenMat Figure";
}

export function figurePositionCssPixels(
  snapshot: FigureSnapshot,
): readonly [number, number, number, number] | null {
  const root = snapshot.objects.find((object) => object.id === snapshot.rootId);
  const position = root?.kind === "figure" ? root.properties.positionCssPixels : null;
  if (
    !Array.isArray(position) ||
    position.length !== 4 ||
    !position.every((value) => typeof value === "number" && Number.isFinite(value))
  ) {
    return null;
  }
  const [left, bottom, width, height] = position;
  if (
    left === undefined ||
    bottom === undefined ||
    width === undefined ||
    height === undefined ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }
  return [left, bottom, width, height];
}
