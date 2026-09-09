import { downloadBlob, usePlatformServices } from "../platform/platform-services";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import katex from "katex";
import "katex/dist/katex.min.css";
import { renderLatexLabel } from "../plot/latex-label";
import type { DisplayEventData } from "../protocol/kernel-v0";
import {
  GraphicsV1Client,
  type GraphicsClientEvent,
  type GraphicsConnectionStatus,
  type RenderableFigure,
} from "../plot/graphics-client";
import {
  FIGURE_MIME_TYPE,
  GRAPHICS_PROTOCOL_V2,
  GRAPHICS_PROTOCOL_V3,
  GRAPHICS_PROTOCOL_V4,
  LEGACY_PLOT_MIME_TYPE,
  GraphicsV1Error,
  figureAccessibleName,
  figurePositionCssPixels,
  parseFigureDiscovery,
  resolveGraphicsWebSocketUrl,
  type FigureDiscovery,
} from "../plot/graphics-v1";
import {
  loadPlotWasmFigure,
  parsePlotWasmError,
  prewarmPlotWasmRenderer,
  webGpuIsAvailable,
  type PlotOverlay,
  type PlotOverlayLine,
  type PlotOverlayText,
  type PlotWasmFigure,
  type PlotWasmLoader,
} from "../plot/plot-wasm";
import {
  createPlotInteractionController,
  type PlotInteractionController,
  type PlotInteractionMode,
  type PlotPickSelection,
  type PlotSelectionBox,
} from "../plot/plot-interaction";
import { kernelWebSocketUrl } from "../runtime-config";
import type { OpenMatAppDefinition } from "../windowing/app-registry";
import { useOpenMatWindowManagerActions } from "../windowing/WindowManager";
import {
  constrainOpenMatWindowBounds,
  type OpenMatWindowBounds,
  type OpenMatWindowCloseDecision,
  type OpenMatWindowSize,
} from "../windowing/window-state";
import {
  PlotBoxZoomIcon,
  PlotDataCursorIcon,
  PlotHomeIcon,
  PlotPanIcon,
} from "./Icons";

type OverlayTextRole =
  | "tickLabel"
  | "title"
  | "xLabel"
  | "yLabel"
  | "legendLabel"
  | "annotation";

type OverlayTextInterpreter = "tex" | "latex" | "none";

interface PlotOverlayTextWithRole extends PlotOverlayText {
  readonly role?: OverlayTextRole;
}

interface PlotOverlayLegend {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly background: readonly [number, number, number, number];
  readonly border: readonly [number, number, number, number];
  readonly entries: readonly PlotOverlayLine[];
}

interface PlotVisualOverlay extends Omit<PlotOverlay, "text"> {
  readonly text: readonly PlotOverlayTextWithRole[];
  readonly legend?: PlotOverlayLegend | null;
  readonly legends?: readonly PlotOverlayLegend[];
  readonly fontCacheRevision?: number;
}

export interface BrowserTextMeasurement {
  readonly key: string;
  readonly widthCssPx: number;
  readonly heightCssPx: number;
  readonly ascentCssPx: number;
  readonly descentCssPx: number;
  readonly advanceCssPx: number;
}

export interface BrowserTextMeasurementBatch {
  readonly fontRevision: number;
  readonly measurements: readonly BrowserTextMeasurement[];
}

export interface PlotTextLayoutBridge {
  completeTextLayout(
    fontRevision: number,
    measurements: readonly BrowserTextMeasurement[],
  ): PlotOverlay | null | void;
}

export interface SvgTextMeasurementController {
  measure(): void;
  dispose(): void;
}

interface LegacyPlotData {
  readonly title: string;
  readonly x: readonly number[];
  readonly y: readonly number[];
}

interface V1Display {
  readonly sourceIndex: number;
  readonly discovery: FigureDiscovery;
}

interface LegacyDisplay {
  readonly sourceIndex: number;
  readonly plot: LegacyPlotData;
}

interface InvalidDisplay {
  readonly sourceIndex: number;
  readonly message: string;
}

export interface FigureGraphicsClient {
  readonly status: GraphicsConnectionStatus;
  matchesAttachment(discovery: FigureDiscovery): boolean;
  setDiscoveries(discoveries: readonly FigureDiscovery[]): void;
  subscribe(listener: (event: GraphicsClientEvent) => void): () => void;
  connect(): void;
  closeFigure(figureId: string): boolean;
  setAxesLimits(
    figureId: string,
    expectedRevision: number,
    xLimits: readonly [number, number],
    yLimits: readonly [number, number],
    axesId?: string,
  ): void;
  setAxesCamera(
    figureId: string,
    expectedRevision: number,
    view: readonly [number, number],
    cameraScale: number,
    axesId?: string,
  ): void;
  releaseBuffer(bufferId: string): void;
  dispose(): void;
}

export type FigureGraphicsClientFactory = (
  sessionId: string,
  discovery: FigureDiscovery,
) => FigureGraphicsClient;

interface FigureWindowProps {
  readonly figures: readonly DisplayEventData[];
  readonly onClose: (index: number) => void;
  readonly graphicsSessionId?: string;
  readonly clientFactory?: FigureGraphicsClientFactory;
  readonly wasmLoader?: PlotWasmLoader;
  readonly webGpuAvailable?: boolean;
}

const defaultConnectionStatus: GraphicsConnectionStatus = {
  phase: "idle",
  reconnectAttempt: 0,
  message: null,
};

const defaultClientFactory: FigureGraphicsClientFactory = (
  sessionId,
  discovery,
) =>
  new GraphicsV1Client(sessionId, discovery, {
    url: resolveGraphicsWebSocketUrl(
      kernelWebSocketUrl(),
      window.location,
      discovery.endpoint,
    ),
  });

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseLegacyPlot(display: DisplayEventData): LegacyPlotData | null {
  const encoded = display.representations[LEGACY_PLOT_MIME_TYPE];
  if (encoded === undefined) {
    return null;
  }
  let value: unknown;
  try {
    value = JSON.parse(encoded) as unknown;
  } catch {
    return null;
  }
  if (!isRecord(value) || value.kind !== "line") {
    return null;
  }
  const { title, x, y } = value;
  if (
    typeof title !== "string" ||
    !Array.isArray(x) ||
    !x.every((item) => typeof item === "number") ||
    !Array.isArray(y) ||
    !y.every((item) => typeof item === "number") ||
    x.length !== y.length
  ) {
    return null;
  }
  return { title, x, y };
}

function parseDisplays(figures: readonly DisplayEventData[]): {
  readonly v1: readonly V1Display[];
  readonly legacy: readonly LegacyDisplay[];
  readonly invalid: readonly InvalidDisplay[];
} {
  const v1ByFigure = new Map<string, V1Display>();
  const legacy: LegacyDisplay[] = [];
  const invalid: InvalidDisplay[] = [];
  figures.forEach((display, sourceIndex) => {
    const encoded = display.representations[FIGURE_MIME_TYPE];
    if (encoded !== undefined) {
      try {
        const discovery = parseFigureDiscovery(encoded);
        v1ByFigure.set(discovery.figureId, { sourceIndex, discovery });
      } catch (error: unknown) {
        invalid.push({
          sourceIndex,
          message:
            error instanceof GraphicsV1Error
              ? error.message
              : "Figure discovery is invalid.",
        });
      }
      return;
    }
    const plot = parseLegacyPlot(display);
    if (plot !== null) {
      legacy.push({ sourceIndex, plot });
    }
  });
  return { v1: [...v1ByFigure.values()], legacy, invalid };
}

function applyRenderableFigure(
  wasm: PlotWasmFigure,
  figure: RenderableFigure,
  client: FigureGraphicsClient,
): PlotVisualOverlay | null {
  try {
    wasm.applySnapshot(figure.snapshot);
    for (const [bufferId, bytes] of figure.buffers) {
      wasm.ingestBuffer(bufferId, bytes);
    }
    return renderPlotOverlay(wasm);
  } finally {
    for (const bufferId of figure.buffers.keys()) {
      client.releaseBuffer(bufferId);
    }
  }
}

function renderPlotOverlay(wasm: PlotWasmFigure): PlotVisualOverlay | null {
  return (wasm.render?.() ?? null) as PlotVisualOverlay | null;
}

export function prepareFigurePngCapture(wasm: PlotWasmFigure): void {
  if (wasm.state !== "ready" || typeof wasm.render !== "function") {
    throw new Error("The Figure renderer is not ready for PNG export.");
  }
  // WebGPU presentation textures may be discarded after compositing. Render
  // immediately before the caller synchronously copies the canvas pixels.
  wasm.render();
}

function utf16Text(codeUnits: readonly number[]): string {
  return codeUnits.map((codeUnit) => String.fromCharCode(codeUnit)).join("");
}

function svgColor(color: readonly [number, number, number, number]): string {
  const [red, green, blue, alpha] = color;
  return `rgba(${Math.round(red * 255).toString()}, ${Math.round(
    green * 255,
  ).toString()}, ${Math.round(blue * 255).toString()}, ${alpha.toString()})`;
}

const SVG_NAMESPACE = "http://www.w3.org/2000/svg";

function exportBaseName(displayNumber: number): string {
  return `openmat-figure-${displayNumber.toString()}`;
}

function inlineForeignObjectStyles(
  source: SVGSVGElement,
  clone: SVGSVGElement,
): void {
  const sourceElements = source.querySelectorAll("foreignObject, foreignObject *");
  const clonedElements = clone.querySelectorAll("foreignObject, foreignObject *");
  sourceElements.forEach((element, index) => {
    const cloned = clonedElements[index];
    if (!(cloned instanceof Element)) {
      return;
    }
    const computed = window.getComputedStyle(element);
    const declarations: string[] = [];
    for (const property of computed) {
      declarations.push(
        `${property}:${computed.getPropertyValue(property)}${computed.getPropertyPriority(property) === "important" ? " !important" : ""}`,
      );
    }
    cloned.setAttribute("style", declarations.join(";"));
  });
}

function serializedFigureSvg(
  canvas: HTMLCanvasElement,
  svg: SVGSVGElement,
  includeCanvas: boolean,
): string {
  const clone = svg.cloneNode(true) as SVGSVGElement;
  const bounds = canvas.getBoundingClientRect();
  const viewBox = svg.getAttribute("viewBox");
  const width = bounds.width > 0 ? bounds.width : Math.max(canvas.width, 1);
  const height = bounds.height > 0 ? bounds.height : Math.max(canvas.height, 1);
  clone.setAttribute("xmlns", SVG_NAMESPACE);
  clone.setAttribute("width", width.toString());
  clone.setAttribute("height", height.toString());
  clone.setAttribute("viewBox", viewBox ?? `0 0 ${width.toString()} ${height.toString()}`);
  inlineForeignObjectStyles(svg, clone);
  if (includeCanvas) {
    const raster = document.createElementNS(SVG_NAMESPACE, "image");
    raster.setAttribute("x", "0");
    raster.setAttribute("y", "0");
    raster.setAttribute("width", width.toString());
    raster.setAttribute("height", height.toString());
    raster.setAttribute("preserveAspectRatio", "none");
    raster.setAttribute("href", canvas.toDataURL("image/png"));
    clone.insertBefore(raster, clone.firstChild);
  }
  return new XMLSerializer().serializeToString(clone);
}

export function createFigureSvgBlob(
  canvas: HTMLCanvasElement,
  svg: SVGSVGElement,
): Blob {
  return new Blob([serializedFigureSvg(canvas, svg, true)], {
    type: "image/svg+xml;charset=utf-8",
  });
}

function loadSvgImage(markup: string): Promise<HTMLImageElement> {
  // Chromium taints canvases when a blob URL contains SVG foreignObject nodes
  // (our TeX/LaTeX labels). An embedded SVG image keeps this local composition
  // origin-clean. URI encoding also preserves Unicode labels and literal '#'.
  const url = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(markup)}`;
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.addEventListener(
      "load",
      () => {
        resolve(image);
      },
      { once: true },
    );
    image.addEventListener(
      "error",
      () => {
        reject(new Error("The Figure SVG overlay could not be rasterized."));
      },
      { once: true },
    );
    image.src = url;
  });
}

function canvasBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob === null) {
        reject(new Error("The Figure canvas could not be encoded as PNG."));
      } else {
        resolve(blob);
      }
    }, "image/png");
  });
}

export async function createFigurePngBlob(
  canvas: HTMLCanvasElement,
  svg: SVGSVGElement,
): Promise<Blob> {
  if (canvas.width <= 0 || canvas.height <= 0) {
    throw new Error("The Figure canvas has no rendered pixels to export.");
  }
  const output = document.createElement("canvas");
  output.width = canvas.width;
  output.height = canvas.height;
  const context = output.getContext("2d");
  if (context === null) {
    throw new Error("The browser could not create a Figure export surface.");
  }
  context.drawImage(canvas, 0, 0, output.width, output.height);
  const overlay = await loadSvgImage(serializedFigureSvg(canvas, svg, false));
  context.drawImage(overlay, 0, 0, output.width, output.height);
  return canvasBlob(output);
}

export function downloadFigureBlob(blob: Blob, fileName: string): void {
  downloadBlob(blob, fileName);
}

function finiteMetric(value: number, fallback = 0): number {
  return Number.isFinite(value) && value >= 0 ? value : fallback;
}

function htmlTextMeasurement(element: HTMLElement): BrowserTextMeasurement {
  const bounds = element.getBoundingClientRect();
  const style = window.getComputedStyle(element);
  const fontSize = finiteMetric(Number.parseFloat(style.fontSize), 12);
  const width = Math.max(
    finiteMetric(element.offsetWidth),
    finiteMetric(element.scrollWidth),
    finiteMetric(bounds.width),
  );
  const height = Math.max(
    finiteMetric(element.offsetHeight),
    finiteMetric(element.scrollHeight),
    finiteMetric(bounds.height),
    fontSize,
  );
  const ascent = height * 0.8;
  return {
    key: element.dataset.overlayTextKey ?? "",
    widthCssPx: width,
    heightCssPx: height,
    ascentCssPx: ascent,
    descentCssPx: height - ascent,
    advanceCssPx: width,
  };
}

function overlayTextMeasurement(
  element: Element,
  context: CanvasRenderingContext2D | null,
): BrowserTextMeasurement | null {
  if (element.tagName.toLowerCase() === "text") {
    return measureSvgTextElement(element as SVGTextElement, context);
  }
  if (element instanceof HTMLElement) {
    return htmlTextMeasurement(element);
  }
  return null;
}

function measureSvgTextElement(
  element: SVGTextElement,
  context: CanvasRenderingContext2D | null,
): BrowserTextMeasurement {
  let boxWidth = 0;
  let boxHeight = finiteMetric(Number.parseFloat(element.getAttribute("font-size") ?? ""), 12);
  try {
    const box = element.getBBox();
    boxWidth = finiteMetric(box.width);
    boxHeight = finiteMetric(box.height, boxHeight);
  } catch {
    // Detached SVG trees and test DOMs may not expose geometry yet. Canvas
    // metrics below remain authoritative for horizontal advance.
  }
  const style = window.getComputedStyle(element);
  let measured: TextMetrics | null = null;
  if (context !== null) {
    const fontSize = style.fontSize || `${boxHeight.toString()}px`;
    const fontFamily = style.fontFamily || "Arial, Helvetica, sans-serif";
    context.font = `${style.fontStyle || "normal"} ${style.fontWeight || "400"} ${fontSize} ${fontFamily}`;
    measured = context.measureText(element.textContent ?? "");
  }
  const ascent = finiteMetric(measured?.actualBoundingBoxAscent ?? Number.NaN, boxHeight * 0.8);
  const descent = finiteMetric(measured?.actualBoundingBoxDescent ?? Number.NaN, boxHeight - ascent);
  const advance = finiteMetric(measured?.width ?? Number.NaN, boxWidth);
  const measuredWidth = finiteMetric(
    (measured?.actualBoundingBoxLeft ?? 0) +
      (measured?.actualBoundingBoxRight ?? 0),
    boxWidth,
  );
  return {
    key: element.dataset.overlayTextKey ?? "",
    widthCssPx: Math.max(boxWidth, measuredWidth),
    heightCssPx: Math.max(boxHeight, ascent + descent),
    ascentCssPx: ascent,
    descentCssPx: descent,
    advanceCssPx: advance,
  };
}

/**
 * Measures the exact SVG text currently selected by the browser. The optional
 * PlotTextLayoutBridge is deliberately separate from this DOM controller so
 * renderer ownership remains in WASM rather than React.
 */
export function createSvgTextMeasurementController(
  svg: SVGSVGElement,
  fontRevision: number,
  onMeasurements: (batch: BrowserTextMeasurementBatch) => void,
): SvgTextMeasurementController {
  let disposed = false;
  let context: CanvasRenderingContext2D | null = null;
  if (typeof CanvasRenderingContext2D !== "undefined") {
    try {
      context = document.createElement("canvas").getContext("2d");
    } catch {
      context = null;
    }
  }
  const measure = (): void => {
    if (disposed) {
      return;
    }
    const measurements = Array.from(
      svg.querySelectorAll<Element>("[data-overlay-text-key]"),
      (element) => overlayTextMeasurement(element, context),
    ).filter(
      (measurement): measurement is BrowserTextMeasurement =>
        measurement !== null && measurement.key.length > 0,
    );
    if (measurements.length > 0) {
      onMeasurements({ fontRevision, measurements });
    }
  };
  const onFontsLoaded = (): void => measure();
  document.fonts?.addEventListener("loadingdone", onFontsLoaded);
  void document.fonts?.ready.then(onFontsLoaded);
  return {
    measure,
    dispose(): void {
      disposed = true;
      document.fonts?.removeEventListener("loadingdone", onFontsLoaded);
    },
  };
}

function overlayLine(line: PlotOverlayLine, key: string) {
  return (
    <line
      key={key}
      x1={line.x1}
      y1={line.y1}
      x2={line.x2}
      y2={line.y2}
      style={{
        stroke: svgColor(line.color),
        strokeWidth: line.widthCssPx,
      }}
      vectorEffect="non-scaling-stroke"
    />
  );
}

function interpretedAsMath(
  value: string,
  interpreter: OverlayTextInterpreter,
): boolean {
  if (interpreter === "none") {
    return false;
  }
  return interpreter === "latex" || /(?:\\[A-Za-z]+|[_^{}$])/u.test(value);
}

function normalizedMathSource(value: string): {
  readonly source: string;
  readonly displayMode: boolean;
} {
  const trimmed = value.trim();
  if (trimmed.startsWith("$$") && trimmed.endsWith("$$") && trimmed.length >= 4) {
    return { source: trimmed.slice(2, -2), displayMode: true };
  }
  if (trimmed.startsWith("\\[") && trimmed.endsWith("\\]") && trimmed.length >= 4) {
    return { source: trimmed.slice(2, -2), displayMode: true };
  }
  if (trimmed.startsWith("$") && trimmed.endsWith("$") && trimmed.length >= 2) {
    return { source: trimmed.slice(1, -1), displayMode: false };
  }
  if (trimmed.startsWith("\\(") && trimmed.endsWith("\\)") && trimmed.length >= 4) {
    return { source: trimmed.slice(2, -2), displayMode: false };
  }
  return { source: value, displayMode: false };
}

function textBoxOrigin(
  text: PlotOverlayTextWithRole,
  width: number,
  height: number,
): readonly [number, number] {
  const x =
    text.horizontalAlignment === "center"
      ? text.x - width / 2
      : text.horizontalAlignment === "end"
        ? text.x - width
        : text.x;
  const y =
    text.verticalAlignment === "middle"
      ? text.y - height / 2
      : text.verticalAlignment === "bottom"
        ? text.y - height
        : text.verticalAlignment === "baseline"
          ? text.y - height * 0.8
          : text.y;
  return [x, y];
}

function overlayTextNode(text: PlotOverlayTextWithRole) {
  const value = utf16Text(text.codeUnits);
  const interpreter = text.interpreter ?? "tex";
  const className = `figure-overlay-text figure-overlay-${text.role ?? "annotation"}`;
  const rotation =
    text.rotationRadians === 0
      ? undefined
      : `rotate(${((text.rotationRadians * 180) / Math.PI).toString()} ${text.x.toString()} ${text.y.toString()})`;
  if (!interpretedAsMath(value, interpreter)) {
    return (
      <text
        key={text.key}
        className={className}
        data-overlay-text-key={text.key}
        x={text.x}
        y={text.y}
        fill={svgColor(text.color)}
        fontFamily={
          text.fontFamilyCodeUnits.length === 0
            ? "sans-serif"
            : utf16Text(text.fontFamilyCodeUnits)
        }
        fontSize={text.fontSizeCssPx}
        fontStyle={text.fontStyle}
        fontWeight={text.fontWeight}
        textAnchor={
          text.horizontalAlignment === "center"
            ? "middle"
            : text.horizontalAlignment
        }
        dominantBaseline={
          text.verticalAlignment === "top"
            ? "hanging"
            : text.verticalAlignment === "middle"
              ? "middle"
              : text.verticalAlignment === "bottom"
                ? "text-after-edge"
                : "alphabetic"
        }
        transform={rotation}
      >
        {value}
      </text>
    );
  }

  const fallbackWidth = Math.max(
    text.fontSizeCssPx,
    value.length * text.fontSizeCssPx * 0.68,
  );
  const width = finiteMetric(text.measuredWidthCssPx ?? Number.NaN, fallbackWidth);
  const height = finiteMetric(
    text.measuredHeightCssPx ?? Number.NaN,
    text.fontSizeCssPx * 1.35,
  );
  const [x, y] = textBoxOrigin(text, width, height);
  const { source, displayMode } = normalizedMathSource(value);
  const markup = interpreter === "latex"
    ? renderLatexLabel(value)
    : katex.renderToString(source, {
        displayMode,
        output: "htmlAndMathml",
        strict: "ignore",
        throwOnError: false,
        trust: false,
      });
  const padding = 2;
  return (
    <foreignObject
      key={text.key}
      className={`figure-overlay-math-host figure-overlay-${text.role ?? "annotation"}`}
      x={x - padding}
      y={y - padding}
      width={width + padding * 2}
      height={height + padding * 2}
      transform={rotation}
      style={{ overflow: "visible" }}
    >
      <div
        className="figure-overlay-math"
        data-overlay-text-key={text.key}
        style={{
          color: svgColor(text.color),
          fontSize: `${text.fontSizeCssPx.toString()}px`,
          fontStyle: text.fontStyle,
          fontWeight: text.fontWeight,
        }}
        dangerouslySetInnerHTML={{ __html: markup }}
      />
    </foreignObject>
  );
}

function rendererMessage(wasm: PlotWasmFigure): string | null {
  const error = parsePlotWasmError(wasm.lastError());
  if (error !== null) {
    return `${error.kind}: ${error.message}`;
  }
  if (wasm.state === "unsupportedWebGpu") {
    return "WebGPU is unavailable. OpenMat Plot Engine requires WebGPU and does not fall back to WebGL.";
  }
  if (wasm.state === "rendererUnavailable") {
    return "The WebGPU renderer is not available in this OpenMat build.";
  }
  if (wasm.state === "failed") {
    return "The OpenMat WebGPU renderer failed to initialize.";
  }
  return null;
}

function surfacedFailure(value: unknown, fallback: string): string {
  const typed = parsePlotWasmError(value);
  if (typed !== null) {
    return `${typed.kind}: ${typed.message}`;
  }
  if (value instanceof Error && value.message.length > 0) {
    return `${value.name}: ${value.message}`;
  }
  if (typeof value === "string" && value.length > 0) {
    return value;
  }
  return fallback;
}

interface GraphicsFigureSurfaceProps {
  readonly displayNumber: number;
  readonly discovery: FigureDiscovery;
  readonly client: FigureGraphicsClient | null;
  readonly connection: GraphicsConnectionStatus;
  readonly closing: boolean;
  readonly onTitleChange: (title: string) => void;
  readonly wasmLoader: PlotWasmLoader;
  readonly hasWebGpu: boolean;
}

const ignoreEmbeddedFigureChange = () => undefined;

/** Embeds the production Rust/WebGPU figure surface without creating an IDE window.
 * Each mounted preview owns its attachment and releases it when the app stops. */
export function EmbeddedFigure({
  figures,
  figureNumber,
  label,
  sessionId,
}: {
  figures: readonly DisplayEventData[];
  figureNumber: number;
  label: string;
  sessionId: string | undefined;
}) {
  const discoveries = useMemo(() => {
    const latest = new Map<string, FigureDiscovery>();
    for (const display of figures) {
      const encoded = display.representations[FIGURE_MIME_TYPE];
      if (encoded) {
        try {
          const discovery = parseFigureDiscovery(encoded);
          latest.set(discovery.figureId, discovery);
        } catch {
          /* Other display payloads are not plot attachments. */
        }
      }
    }
    return [...latest.values()];
  }, [figures]);
  const discovery = discoveries[figureNumber - 1];
  const [attachment, setAttachment] = useState<{
    client: FigureGraphicsClient;
    connection: GraphicsConnectionStatus;
  } | null>(null);
  const token = discovery?.attachToken;
  const endpoint = discovery?.endpoint;
  useEffect(() => {
    if (!discovery || !sessionId) return;
    const client = defaultClientFactory(sessionId, discovery);
    const unsubscribe = client.subscribe((event) => {
      if (event.type === "connection")
        setAttachment({ client, connection: event.status });
    });
    setAttachment({ client, connection: client.status });
    client.connect();
    return () => {
      unsubscribe();
      client.dispose();
    };
    // Only a new kernel attachment requires a new client; revisions update below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, endpoint, sessionId]);
  useEffect(() => {
    if (discovery) attachment?.client.setDiscoveries([discovery]);
  }, [discovery, attachment?.client]);
  if (!discovery)
    return (
      <div className="ui-placeholder">
        <PlotHomeIcon />
        <span>{label}</span>
        <small>等待运行代码创建绘图</small>
      </div>
    );
  return (
    <GraphicsFigureSurface
      key={discovery.figureId}
      displayNumber={figureNumber}
      discovery={discovery}
      client={attachment?.client ?? null}
      connection={attachment?.connection ?? defaultConnectionStatus}
      closing={false}
      onTitleChange={ignoreEmbeddedFigureChange}
      wasmLoader={loadPlotWasmFigure}
      hasWebGpu={webGpuIsAvailable()}
    />
  );
}

function GraphicsFigureSurface({
  displayNumber,
  discovery,
  client,
  connection,
  closing,
  onTitleChange,
  wasmLoader,
  hasWebGpu,
}: GraphicsFigureSurfaceProps) {
  const platform = usePlatformServices();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const selectionBoxRef = useRef<HTMLDivElement>(null);
  const wasmRef = useRef<PlotWasmFigure | null>(null);
  const interactionRef = useRef<PlotInteractionController | null>(null);
  const clientRef = useRef(client);
  const latestFigureRef = useRef<RenderableFigure | null>(null);
  const latestSizeRef = useRef({ width: 0, height: 0, dpr: 1 });
  const lastTextMeasurementRef = useRef("");
  const revisionRef = useRef(discovery.revision);
  const interactionModeRef = useRef<PlotInteractionMode>("pan");
  const [surfaceState, setSurfaceState] = useState<
    "loading" | "ready" | "unsupported" | "error"
  >("loading");
  const [surfaceMessage, setSurfaceMessage] = useState<string | null>(null);
  const [accessibleName, setAccessibleName] = useState(
    `Figure ${displayNumber.toString()}`,
  );
  const [revision, setRevision] = useState(discovery.revision);
  const [overlay, setOverlay] = useState<PlotVisualOverlay | null>(null);
  const [interactionMode, setInteractionMode] =
    useState<PlotInteractionMode>("pan");
  const [interactionDimension, setInteractionDimension] = useState<
    "2d" | "polar" | "3d"
  >("2d");
  const [pickSelection, setPickSelection] =
    useState<PlotPickSelection | null>(null);
  const [interactionMessage, setInteractionMessage] = useState<string | null>(
    null,
  );
  const [exporting, setExporting] = useState<"png" | "svg" | null>(null);
  clientRef.current = client;

  const updateSelectionBox = useCallback((box: PlotSelectionBox | null): void => {
    const element = selectionBoxRef.current;
    if (element === null) {
      return;
    }
    if (box === null) {
      element.hidden = true;
      return;
    }
    element.hidden = false;
    element.style.transform = `translate(${box.x.toString()}px, ${box.y.toString()}px)`;
    element.style.width = `${box.width.toString()}px`;
    element.style.height = `${box.height.toString()}px`;
  }, []);

  const syncInteractionDimension = useCallback((wasm: PlotWasmFigure): void => {
    const reportedDimension = wasm.interactionDimension?.();
    const nextDimension =
      reportedDimension === "3d" || reportedDimension === "polar"
        ? reportedDimension
        : "2d";
    setInteractionDimension(nextDimension);
    if (nextDimension === "polar" && interactionModeRef.current !== "dataCursor") {
      interactionModeRef.current = "dataCursor";
      interactionRef.current?.setMode("dataCursor");
      setInteractionMode("dataCursor");
      setPickSelection(null);
      updateSelectionBox(null);
    } else if (nextDimension === "3d" && interactionModeRef.current === "box") {
      interactionModeRef.current = "pan";
      interactionRef.current?.setMode("pan");
      setInteractionMode("pan");
      setPickSelection(null);
      updateSelectionBox(null);
    }
  }, [updateSelectionBox]);

  const installInteraction = useCallback(
    (canvas: HTMLCanvasElement, wasm: PlotWasmFigure): void => {
      interactionRef.current?.dispose();
      const controller = createPlotInteractionController(canvas, wasm, {
        onCommit: (result) => {
          setInteractionMessage(null);
          if (result.dimension === "3d") {
            const currentClient = clientRef.current;
            if (
              currentClient !== null &&
              (discovery.graphicsProtocol === GRAPHICS_PROTOCOL_V2 ||
                discovery.graphicsProtocol === GRAPHICS_PROTOCOL_V3 ||
                discovery.graphicsProtocol === GRAPHICS_PROTOCOL_V4) &&
              result.view !== undefined &&
              result.cameraScale !== undefined
            ) {
              try {
                currentClient.setAxesCamera(
                  discovery.figureId,
                  revisionRef.current,
                  result.view,
                  result.cameraScale,
                  discovery.graphicsProtocol === GRAPHICS_PROTOCOL_V3 ||
                  discovery.graphicsProtocol === GRAPHICS_PROTOCOL_V4
                    ? result.axesId
                    : undefined,
                );
              } catch {
                setInteractionMessage(
                  "The camera changed locally, but its view could not be committed.",
                );
              }
            }
            return;
          }
          const currentClient = clientRef.current;
          if (
            currentClient === null ||
            (discovery.graphicsProtocol !== GRAPHICS_PROTOCOL_V2 &&
              discovery.graphicsProtocol !== GRAPHICS_PROTOCOL_V3 &&
              discovery.graphicsProtocol !== GRAPHICS_PROTOCOL_V4)
          ) {
            return;
          }
          try {
            currentClient.setAxesLimits(
              discovery.figureId,
              revisionRef.current,
              result.xLimits,
              result.yLimits,
              discovery.graphicsProtocol === GRAPHICS_PROTOCOL_V3 ||
              discovery.graphicsProtocol === GRAPHICS_PROTOCOL_V4
                ? result.axesId
                : undefined,
            );
          } catch (error: unknown) {
            setInteractionMessage(
              surfacedFailure(
                error,
                "The view changed locally, but its axes limits could not be committed.",
              ),
            );
          }
        },
        onOverlay: setOverlay,
        onBoxPreview: updateSelectionBox,
        onPick: (selection) => {
          setPickSelection(selection.hit === null ? null : selection);
        },
        onError: (error) => {
          setInteractionMessage(
            surfacedFailure(error, "The Figure interaction could not be completed."),
          );
        },
      });
      controller.setMode(interactionModeRef.current);
      interactionRef.current = controller;
    },
    [discovery.figureId, updateSelectionBox],
  );

  const selectInteractionMode = (mode: PlotInteractionMode): void => {
    interactionModeRef.current = mode;
    interactionRef.current?.setMode(mode);
    setInteractionMode(mode);
    setPickSelection(null);
  };

  const exportFigure = async (format: "png" | "svg"): Promise<void> => {
    const canvas = canvasRef.current;
    const svg = svgRef.current;
    if (canvas === null || svg === null || exporting !== null) {
      return;
    }
    setExporting(format);
    setInteractionMessage(null);
    try {
      if (format === "png") {
        const wasm = wasmRef.current;
        if (wasm === null) {
          throw new Error("The Figure renderer is not ready for PNG export.");
        }
        prepareFigurePngCapture(wasm);
      }
      const blob =
        format === "png"
          ? await createFigurePngBlob(canvas, svg)
          : createFigureSvgBlob(canvas, svg);
      const saved = await platform.saveExport(blob, `${exportBaseName(displayNumber)}.${format}`);
      setInteractionMessage(saved ? `Exported ${format.toUpperCase()} image.` : "Export canceled.");
    } catch (error: unknown) {
      setInteractionMessage(
        surfacedFailure(error, `The Figure could not be exported as ${format.toUpperCase()}.`),
      );
    } finally {
      setExporting(null);
    }
  };

  useEffect(() => {
    onTitleChange(accessibleName);
  }, [accessibleName, onTitleChange]);

  useEffect(() => {
    if (client === null) {
      return;
    }
    return client.subscribe((event) => {
      if (
        event.type !== "figureReady" ||
        event.figure.figureId !== discovery.figureId
      ) {
        return;
      }
      latestFigureRef.current = event.figure;
      setAccessibleName(figureAccessibleName(event.figure.snapshot));
      setRevision(event.figure.revision);
      revisionRef.current = event.figure.revision;
      const wasm = wasmRef.current;
      if (wasm !== null && wasm.state === "ready") {
        try {
          setOverlay(applyRenderableFigure(wasm, event.figure, client));
          syncInteractionDimension(wasm);
          setSurfaceState("ready");
          setSurfaceMessage(null);
        } catch (error: unknown) {
          console.error("OpenMat WebGPU renderer rejected a Figure", error);
          setOverlay(null);
          setSurfaceState("error");
          setSurfaceMessage(
            surfacedFailure(
              error,
              "The OpenMat WebGPU renderer rejected this Figure.",
            ),
          );
        }
      }
    });
  }, [client, discovery.figureId, syncInteractionDimension]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null) {
      return;
    }
    let disposed = false;
    if (!hasWebGpu) {
      setSurfaceState("unsupported");
      setSurfaceMessage(
        "WebGPU is unavailable. OpenMat Plot Engine requires WebGPU; there is no SVG, WebGL, or WebGL2 plot fallback.",
      );
      return;
    }
    const initialize = async (): Promise<void> => {
      try {
        const wasm = await wasmLoader(canvas);
        if (disposed) {
          wasm.dispose();
          return;
        }
        wasmRef.current = wasm;
        installInteraction(canvas, wasm);
        const size = latestSizeRef.current;
        if (size.width > 0 && size.height > 0) {
          wasm.resize(size.width, size.height, size.dpr);
        }
        const message = rendererMessage(wasm);
        if (message !== null) {
          setSurfaceState(
            wasm.state === "unsupportedWebGpu" ? "unsupported" : "error",
          );
          setSurfaceMessage(message);
          return;
        }
        const latest = latestFigureRef.current;
        const currentClient = clientRef.current;
        if (latest !== null && currentClient !== null) {
          setOverlay(applyRenderableFigure(wasm, latest, currentClient));
          syncInteractionDimension(wasm);
          setSurfaceState("ready");
        }
        setSurfaceMessage(null);
      } catch (error: unknown) {
        if (!disposed) {
          setSurfaceState("error");
          setSurfaceMessage(
            surfacedFailure(
              error,
              "The OpenMat Plot WASM module could not initialize the WebGPU renderer.",
            ),
          );
        }
      }
    };
    void initialize();
    return () => {
      disposed = true;
      interactionRef.current?.dispose();
      interactionRef.current = null;
      wasmRef.current?.dispose();
      wasmRef.current = null;
      setOverlay(null);
    };
  }, [hasWebGpu, installInteraction, syncInteractionDimension, wasmLoader]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null) {
      return;
    }
    const updateSize = (width: number, height: number): void => {
      if (width <= 0 || height <= 0) {
        return;
      }
      const dpr = Math.max(window.devicePixelRatio || 1, 0.1);
      const previous = latestSizeRef.current;
      if (
        previous.width === width &&
        previous.height === height &&
        previous.dpr === dpr
      ) {
        return;
      }
      latestSizeRef.current = { width, height, dpr };
      try {
        const wasm = wasmRef.current;
        if (wasm !== null) {
          wasm.resize(width, height, dpr);
          if (latestFigureRef.current !== null) {
            const resizedOverlay = renderPlotOverlay(wasm);
            if (resizedOverlay !== null) {
              setOverlay(resizedOverlay);
            }
          }
        }
      } catch (error: unknown) {
        setSurfaceState("error");
        setSurfaceMessage(
          surfacedFailure(error, "The WebGPU canvas could not be resized."),
        );
      }
    };
    const initial = canvas.getBoundingClientRect();
    updateSize(initial.width, initial.height);
    const onWindowResize = (): void => {
      const bounds = canvas.getBoundingClientRect();
      updateSize(bounds.width, bounds.height);
    };
    window.addEventListener("resize", onWindowResize);
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver((entries) => {
            const entry = entries[0];
            if (entry !== undefined) {
              updateSize(entry.contentRect.width, entry.contentRect.height);
            }
          });
    observer?.observe(canvas);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", onWindowResize);
    };
  }, []);

  useEffect(() => {
    const svg = svgRef.current;
    if (svg === null || overlay === null || overlay.text.length === 0) {
      return;
    }
    const controller = createSvgTextMeasurementController(
      svg,
      overlay.fontCacheRevision ?? 1,
      (batch) => {
        const signature = `${revision.toString()}:${batch.fontRevision.toString()}:${batch.measurements
          .map(
            (measurement) =>
              `${measurement.key}:${measurement.widthCssPx.toFixed(3)}:${measurement.heightCssPx.toFixed(3)}:${measurement.advanceCssPx.toFixed(3)}`,
          )
          .join("|")}`;
        if (lastTextMeasurementRef.current === signature) {
          return;
        }
        lastTextMeasurementRef.current = signature;
        const wasm = wasmRef.current as
          | (PlotWasmFigure & Partial<PlotTextLayoutBridge>)
          | null;
        if (typeof wasm?.completeTextLayout !== "function") {
          return;
        }
        try {
          const measuredOverlay = wasm.completeTextLayout(
            batch.fontRevision,
            batch.measurements,
          );
          if (measuredOverlay !== null && measuredOverlay !== undefined) {
            setOverlay(measuredOverlay as PlotVisualOverlay);
          }
        } catch {
          // A rapid camera update can supersede a browser measurement batch.
          // Keep the provisional overlay; the next projected overlay owns a
          // matching retained text pass and will request fresh measurements.
        }
      },
    );
    controller.measure();
    return () => controller.dispose();
  }, [overlay, revision]);

  const statusText =
    surfaceMessage ??
    (connection.phase === "error" || connection.phase === "closed"
      ? (connection.message ?? "Graphics channel unavailable.")
      : connection.phase === "reconnecting"
        ? "Graphics channel reconnecting…"
        : surfaceState === "loading"
          ? "Loading the OpenMat Figure…"
          : null);

  const overlayLegends =
    overlay?.legends ??
    (overlay?.legend === undefined || overlay.legend === null
      ? []
      : [overlay.legend]);

  return (
    <div className="figure-app graphics-figure">
      <span className="figure-protocol-badge">
        {discovery.graphicsProtocol.replace("openmat-", "")}
      </span>
      <div className="figure-toolbar" role="toolbar" aria-label="Figure tools">
        <button
          type="button"
          className="icon-button"
          aria-label="Restore home view"
          title="Restore home view"
          onClick={() => interactionRef.current?.home()}
          disabled={surfaceState !== "ready" || interactionDimension === "polar"}
        >
          <PlotHomeIcon />
        </button>
        <button
          type="button"
          className="icon-button"
          aria-label={
            interactionDimension === "3d"
              ? "Rotate 3D view"
              : interactionDimension === "polar"
                ? "Pan unavailable for PolarAxes"
                : "Pan"
          }
          title={
            interactionDimension === "3d"
              ? "Rotate 3D view"
              : interactionDimension === "polar"
                ? "Pan unavailable for PolarAxes"
                : "Pan"
          }
          aria-pressed={interactionMode === "pan"}
          onClick={() => selectInteractionMode("pan")}
          disabled={surfaceState !== "ready" || interactionDimension === "polar"}
        >
          <PlotPanIcon />
        </button>
        <button
          type="button"
          className="icon-button"
          aria-label="Box zoom"
          title="Box zoom"
          aria-pressed={interactionMode === "box"}
          onClick={() => selectInteractionMode("box")}
          disabled={surfaceState !== "ready" || interactionDimension !== "2d"}
        >
          <PlotBoxZoomIcon />
        </button>
        <button
          type="button"
          className="icon-button"
          aria-label="Data cursor"
          title="Data cursor"
          aria-pressed={interactionMode === "dataCursor"}
          onClick={() => selectInteractionMode("dataCursor")}
          disabled={surfaceState !== "ready"}
        >
          <PlotDataCursorIcon />
        </button>
        <span className="figure-toolbar-separator" aria-hidden="true" />
        <button
          type="button"
          className="figure-export-button"
          aria-label="Export Figure as PNG"
          title="Export Figure as PNG"
          disabled={surfaceState !== "ready" || exporting !== null}
          onClick={() => void exportFigure("png")}
        >
          PNG
        </button>
        <button
          type="button"
          className="figure-export-button"
          aria-label="Export Figure as SVG"
          title="Export Figure as SVG"
          disabled={surfaceState !== "ready" || exporting !== null}
          onClick={() => void exportFigure("svg")}
        >
          SVG
        </button>
      </div>
      <figure className="figure-render-stack">
        <figcaption className="sr-only">
          {accessibleName}, graphics revision {revision.toString()}
          {overlayLegends.length === 0
            ? null
            : `; legend: ${(overlay?.text ?? [])
                .filter((text) => text.role === "legendLabel")
                .map((text) => utf16Text(text.codeUnits))
                .join(", ")}`}
        </figcaption>
        <canvas
          ref={canvasRef}
          className={`figure-canvas figure-canvas-${interactionMode}`}
          role="img"
          aria-label={`${accessibleName} WebGPU rendering surface`}
        />
        <svg
          ref={svgRef}
          className="figure-svg-overlay"
          aria-hidden="true"
          focusable="false"
          viewBox={
            overlay === null
              ? undefined
              : `0 0 ${overlay.canvasWidthCssPx.toString()} ${overlay.canvasHeightCssPx.toString()}`
          }
        >
          {overlay?.axisLines.map((line, index) =>
            overlayLine(line, `axis-${index.toString()}`),
          )}
          {overlay?.ticks.map((line, index) =>
            overlayLine(line, `tick-${index.toString()}`),
          )}
          {overlayLegends.map((legend, legendIndex) => (
            <g
              className="figure-legend"
              key={`legend-${legendIndex.toString()}`}
            >
              <rect
                x={legend.x}
                y={legend.y}
                width={legend.width}
                height={legend.height}
                fill={svgColor(legend.background)}
                stroke={svgColor(legend.border)}
                vectorEffect="non-scaling-stroke"
              />
              {legend.entries.map((entry, entryIndex) =>
                overlayLine(
                  entry,
                  `legend-${legendIndex.toString()}-entry-${entryIndex.toString()}`,
                ),
              )}
            </g>
          ))}
          {overlay?.text.map(overlayTextNode)}
        </svg>
        <div className="figure-dom-overlay">
          <div
            ref={selectionBoxRef}
            className="figure-selection-box"
            aria-hidden="true"
            hidden
          />
          {pickSelection?.hit === null || pickSelection === null ? null : (
            <output
              className="figure-data-tip"
              style={{
                transform: `translate(${pickSelection.xCssPx.toString()}px, ${pickSelection.yCssPx.toString()}px)`,
              }}
              aria-live="polite"
            >
              <strong>{pickSelection.hit.objectId}</strong>
              {pickSelection.hit.sourceIndex === undefined ? null : (
                <span>
                  Point {(pickSelection.hit.sourceIndex + 1).toString()}
                </span>
              )}
              {pickSelection.hit.x === undefined ||
              pickSelection.hit.y === undefined ? (
                <span>
                  {pickSelection.hit.kind ?? "object"} {"#"}
                  {(pickSelection.hit.primitiveIndex ?? 0).toString()}
                </span>
              ) : (
                <span>
                  X {pickSelection.hit.x.toPrecision(6)} · Y{" "}
                  {pickSelection.hit.y.toPrecision(6)}
                  {pickSelection.hit.z === undefined ? null : (
                    <> · Z {pickSelection.hit.z.toPrecision(6)}</>
                  )}
                </span>
              )}
            </output>
          )}
        </div>
        {interactionMessage === null ? null : (
          <div className="figure-interaction-message" role="status">
            {interactionMessage}
          </div>
        )}
        {closing ? (
          <div className="figure-renderer-message loading" role="status">
            Closing Figure…
          </div>
        ) : statusText === null ? null : (
          <div
            className={`figure-renderer-message ${surfaceState}`}
            role={
              surfaceState === "unsupported" || surfaceState === "error"
                ? "alert"
                : "status"
            }
          >
            {statusText}
          </div>
        )}
      </figure>
    </div>
  );
}

interface LegacyFigureProps {
  readonly displayNumber: number;
  readonly display: LegacyDisplay;
}

function LegacyFigure({ displayNumber, display }: LegacyFigureProps) {
  const { plot } = display;
  const width = 520;
  const height = 300;
  const padding = 36;
  let minX = 0;
  let maxX = 1;
  let minY = 0;
  let maxY = 1;
  if (plot.x.length > 0) {
    minX = plot.x[0] ?? 0;
    maxX = minX;
    minY = plot.y[0] ?? 0;
    maxY = minY;
    for (let pointIndex = 1; pointIndex < plot.x.length; pointIndex += 1) {
      const x = plot.x[pointIndex] ?? 0;
      const y = plot.y[pointIndex] ?? 0;
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y);
    }
  }
  const xSpan = Math.max(maxX - minX, 1);
  const ySpan = Math.max(maxY - minY, 1);
  const points = plot.x
    .map((x, pointIndex) => {
      const y = plot.y[pointIndex] ?? 0;
      const plotX = padding + ((x - minX) / xSpan) * (width - padding * 2);
      const plotY =
        height - padding - ((y - minY) / ySpan) * (height - padding * 2);
      return `${plotX.toString()},${plotY.toString()}`;
    })
    .join(" ");

  return (
    <div className="figure-app legacy-figure-window">
      <span className="figure-protocol-badge legacy">Legacy preview</span>
      <figure>
        <figcaption className="sr-only">
          {plot.title}, legacy line preview with {plot.x.length.toString()} points
        </figcaption>
        <svg
          viewBox={`0 0 ${width.toString()} ${height.toString()}`}
          role="img"
          aria-label={`${plot.title}, a legacy line preview with ${plot.x.length.toString()} points`}
        >
          <line x1={padding} y1={padding} x2={padding} y2={height - padding} />
          <line
            x1={padding}
            y1={height - padding}
            x2={width - padding}
            y2={height - padding}
          />
          <polyline points={points} />
        </svg>
      </figure>
    </div>
  );
}

export const GRAPHICS_FIGURE_APP_ID = "openmat.figure.graphics-v1";
export const LEGACY_FIGURE_APP_ID = "openmat.figure.legacy";
export const INVALID_FIGURE_APP_ID = "openmat.figure.invalid";

interface GraphicsFigureAppPayload {
  readonly displayNumber: number;
  readonly discovery: FigureDiscovery;
  readonly client: FigureGraphicsClient | null;
  readonly connection: GraphicsConnectionStatus;
  readonly closing: boolean;
  readonly wasmLoader: PlotWasmLoader;
  readonly hasWebGpu: boolean;
}

interface LegacyFigureAppPayload {
  readonly displayNumber: number;
  readonly display: LegacyDisplay;
}

interface InvalidFigureAppPayload {
  readonly message: string;
}

export const graphicsFigureAppDefinition: OpenMatAppDefinition<GraphicsFigureAppPayload> = {
  id: GRAPHICS_FIGURE_APP_ID,
  displayName: "Figure",
  defaultSize: { width: 660, height: 470 },
  minimumSize: { width: 360, height: 280 },
  component: ({ payload, window }) => (
    <GraphicsFigureSurface
      displayNumber={payload.displayNumber}
      discovery={payload.discovery}
      client={payload.client}
      connection={payload.connection}
      closing={payload.closing}
      onTitleChange={window.setTitle}
      wasmLoader={payload.wasmLoader}
      hasWebGpu={payload.hasWebGpu}
    />
  ),
};

export const legacyFigureAppDefinition: OpenMatAppDefinition<LegacyFigureAppPayload> = {
  id: LEGACY_FIGURE_APP_ID,
  displayName: "Legacy Figure",
  defaultSize: { width: 620, height: 430 },
  minimumSize: { width: 340, height: 260 },
  component: ({ payload }) => (
    <LegacyFigure
      displayNumber={payload.displayNumber}
      display={payload.display}
    />
  ),
};

export const invalidFigureAppDefinition: OpenMatAppDefinition<InvalidFigureAppPayload> = {
  id: INVALID_FIGURE_APP_ID,
  displayName: "Invalid OpenMat Figure",
  defaultSize: { width: 520, height: 220 },
  minimumSize: { width: 320, height: 180 },
  component: ({ payload }) => (
    <div className="invalid-figure-app">
      <div className="figure-renderer-message error" role="alert">
        {payload.message}
      </div>
    </div>
  ),
};

function graphicsFigureWindowId(graphicsSessionId: string, figureId: string): string {
  return `figure:${graphicsSessionId}:${figureId}`;
}

function legacyFigureWindowId(graphicsSessionId: string, sourceIndex: number): string {
  return `figure:${graphicsSessionId}:legacy:${sourceIndex.toString()}`;
}

function invalidFigureWindowId(graphicsSessionId: string, sourceIndex: number): string {
  return `figure:${graphicsSessionId}:invalid:${sourceIndex.toString()}`;
}

interface FigurePresentation {
  readonly title: string;
  readonly position: ReturnType<typeof figurePositionCssPixels>;
}

// FigureCreationProperties::default in the native graphics model. The current
// wire format carries a rectangle, with no auto/manual placement flag. Treat
// this exact rectangle as a default-placement hint only on the first snapshot;
// later Position changes (including a return to this rectangle) stay absolute.
const DEFAULT_FIGURE_POSITION = [100, 100, 560, 420] as const;

function centeredFigureBounds(
  size: OpenMatWindowSize,
  desktop: OpenMatWindowSize,
  minimumSize: OpenMatWindowSize,
): OpenMatWindowBounds {
  const { width, height } = constrainOpenMatWindowBounds(
    { x: 0, y: 0, ...size },
    desktop,
    minimumSize,
  );
  return {
    x: (desktop.width - width) / 2,
    y: (desktop.height - height) / 2,
    width,
    height,
  };
}

function sameFigurePosition(
  left: FigurePresentation["position"],
  right: FigurePresentation["position"],
): boolean {
  return (
    left === right ||
    (left !== null &&
      right !== null &&
      left.every((value, index) => value === right[index]))
  );
}

export function FigureWindow({
  figures,
  onClose,
  graphicsSessionId = "openmat-web-alpha",
  clientFactory = defaultClientFactory,
  wasmLoader = loadPlotWasmFigure,
  webGpuAvailable: suppliedWebGpuAvailable,
}: FigureWindowProps) {
  const windowManager = useOpenMatWindowManagerActions();
  const parsed = useMemo(() => parseDisplays(figures), [figures]);
  const activeAttachment = parsed.v1.at(-1)?.discovery ?? null;
  const activeToken = activeAttachment?.attachToken ?? null;
  const activeDiscoveries = useMemo(
    () =>
      activeAttachment === null
        ? []
        : parsed.v1
            .map((display) => display.discovery)
            .filter(
              (discovery) =>
                discovery.endpoint === activeAttachment.endpoint &&
                discovery.attachToken === activeAttachment.attachToken,
            ),
    [activeAttachment, parsed.v1],
  );
  const [client, setClient] = useState<FigureGraphicsClient | null>(null);
  const attachedClient = useRef<FigureGraphicsClient | null>(null);
  const [connection, setConnection] = useState(defaultConnectionStatus);
  const [presentations, setPresentations] = useState<{
    readonly client: FigureGraphicsClient | null;
    readonly sessionId: string;
    readonly figures: ReadonlyMap<string, FigurePresentation>;
  }>({ client: null, sessionId: graphicsSessionId, figures: new Map() });
  const [closingFigures, setClosingFigures] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const closeResolvers = useRef(
    new Map<string, (decision: OpenMatWindowCloseDecision) => void>(),
  );
  const ownedWindowIds = useRef<ReadonlySet<string>>(new Set());
  const appliedPositions = useRef(
    new Map<string, FigurePresentation["position"]>(),
  );
  const hasWebGpu =
    suppliedWebGpuAvailable ?? webGpuIsAvailable();

  useEffect(() => {
    if (!hasWebGpu || wasmLoader !== loadPlotWasmFigure) {
      return;
    }
    const timeout = window.setTimeout(() => {
      void prewarmPlotWasmRenderer().catch(() => {
        // A visible Figure retries through the same loader and owns user-facing
        // error reporting. Idle prewarming must not disrupt the workbench.
      });
    }, 250);
    return () => window.clearTimeout(timeout);
  }, [hasWebGpu, wasmLoader]);

  useEffect(() => {
    if (activeAttachment === null || activeToken === null) {
      attachedClient.current = null;
      setClient(null);
      setConnection(defaultConnectionStatus);
      return;
    }
    const nextClient = clientFactory(graphicsSessionId, activeAttachment);
    attachedClient.current = nextClient;
    setClient(nextClient);
    setConnection(nextClient.status);
    nextClient.connect();
    return () => {
      attachedClient.current = null;
      nextClient.dispose();
    };
  }, [activeAttachment?.endpoint, activeToken, clientFactory, graphicsSessionId]);

  useEffect(() => {
    if (client !== null && activeDiscoveries.length > 0) {
      client.setDiscoveries(activeDiscoveries);
    }
  }, [activeDiscoveries, client]);

  useEffect(() => {
    if (client === null) {
      return;
    }
    return client.subscribe((event) => {
      if (event.type === "connection") {
        setConnection(event.status);
        return;
      }
      if (event.type === "figureReady") {
        const next: FigurePresentation = {
          title: figureAccessibleName(event.figure.snapshot),
          position: figurePositionCssPixels(event.figure.snapshot),
        };
        setPresentations((current) => {
          const figures =
            current.client === client && current.sessionId === graphicsSessionId
              ? current.figures
              : new Map<string, FigurePresentation>();
          const previous = figures.get(event.figure.figureId);
          if (
            previous?.title === next.title &&
            sameFigurePosition(previous.position, next.position)
          ) {
            return current;
          }
          return {
            client,
            sessionId: graphicsSessionId,
            figures: new Map(figures).set(event.figure.figureId, next),
          };
        });
        return;
      }
      if (event.type === "figureClosed") {
        setPresentations((current) => {
          if (current.client !== client || !current.figures.has(event.figureId)) {
            return current;
          }
          const figures = new Map(current.figures);
          figures.delete(event.figureId);
          return { ...current, figures };
        });
        setClosingFigures((current) => {
          const next = new Set(current);
          next.delete(event.figureId);
          return next;
        });
        const display = parsed.v1.find(
          (candidate) => candidate.discovery.figureId === event.figureId,
        );
        if (display !== undefined) {
          const resolver = closeResolvers.current.get(event.figureId);
          closeResolvers.current.delete(event.figureId);
          if (resolver === undefined) {
            windowManager.closeWindow(
              graphicsFigureWindowId(graphicsSessionId, event.figureId),
            );
          } else {
            resolver("close");
          }
          onClose(display.sourceIndex);
        }
        return;
      }
      if (event.type === "figureCloseFailed") {
        setClosingFigures((current) => {
          const next = new Set(current);
          next.delete(event.figureId);
          return next;
        });
        const resolver = closeResolvers.current.get(event.figureId);
        closeResolvers.current.delete(event.figureId);
        resolver?.("cancel");
      }
    });
  }, [client, graphicsSessionId, onClose, parsed.v1, windowManager]);

  useEffect(
    () => () => {
      closeResolvers.current.forEach((resolve) => resolve("cancel"));
      closeResolvers.current.clear();
    },
    [],
  );

  const closeGraphicsFigure = (
    display: V1Display,
  ): OpenMatWindowCloseDecision | Promise<OpenMatWindowCloseDecision> => {
    if (
      client === null ||
      client.status.phase === "error" ||
      client.status.phase === "closed" ||
      (client.status.phase === "reconnecting" &&
        (presentations.client !== client ||
          !presentations.figures.has(display.discovery.figureId)))
    ) {
      onClose(display.sourceIndex);
      return "close";
    }
    const figureId = display.discovery.figureId;
    if (!client.closeFigure(figureId)) {
      return "cancel";
    }
    setClosingFigures((current) => new Set(current).add(figureId));
    return new Promise((resolve) => {
      closeResolvers.current.set(figureId, resolve);
    });
  };

  useEffect(() => {
    const desiredWindowIds = new Set<string>();
    parsed.v1.forEach((display, index) => {
      // A replacement attachment is installed before this effect runs. Never
      // open a new session's window with the previous client's cached geometry.
      if (client === null || client !== attachedClient.current) {
        return;
      }
      const displayNumber = index + 1;
      const id = graphicsFigureWindowId(
        graphicsSessionId,
        display.discovery.figureId,
      );
      const presentation =
        presentations.client === client &&
        presentations.sessionId === graphicsSessionId
          ? presentations.figures.get(display.discovery.figureId)
          : undefined;
      // Discovery contains no geometry. Wait for the first complete snapshot so
      // a new window never paints at a temporary cascade position. Transport
      // failures still get a visible, closable window even without a snapshot.
      if (
        presentation === undefined &&
        connection.phase !== "error" &&
        connection.phase !== "closed" &&
        connection.phase !== "reconnecting" &&
        !ownedWindowIds.current.has(id)
      ) {
        return;
      }
      desiredWindowIds.add(id);
      const position = presentation?.position ?? null;
      const desktop = windowManager.getDesktopSize();
      const firstPosition = (appliedPositions.current.get(id) ?? null) === null;
      const bounds =
        position === null ||
        (firstPosition && sameFigurePosition(position, DEFAULT_FIGURE_POSITION))
          ? centeredFigureBounds(
              position === null
                ? graphicsFigureAppDefinition.defaultSize
                : { width: position[2], height: position[3] },
              desktop,
              graphicsFigureAppDefinition.minimumSize,
            )
          : {
              x: position[0],
              y: desktop.height - position[1] - position[3],
              width: position[2],
              height: position[3],
            };
      windowManager.openWindow({
        id,
        appId: GRAPHICS_FIGURE_APP_ID,
        title: presentation?.title ?? `Figure ${displayNumber.toString()}`,
        closeButtonLabel: `Close Figure ${displayNumber.toString()}`,
        bounds,
        payload: {
          displayNumber,
          discovery: display.discovery,
          client,
          connection,
          closing: closingFigures.has(display.discovery.figureId),
          wasmLoader,
          hasWebGpu,
        },
        onCloseRequested: () => closeGraphicsFigure(display),
      });
      // Plot/camera updates repeat Position. Only an actual change from M code
      // may replace a user's local drag or resize.
      if (
        appliedPositions.current.has(id) &&
        !sameFigurePosition(appliedPositions.current.get(id) ?? null, position)
      ) {
        windowManager.setWindowBounds(id, bounds);
      }
      appliedPositions.current.set(id, position);
    });
    parsed.legacy.forEach((display, index) => {
      const displayNumber = parsed.v1.length + index + 1;
      const id = legacyFigureWindowId(graphicsSessionId, display.sourceIndex);
      desiredWindowIds.add(id);
      windowManager.openWindow({
        id,
        appId: LEGACY_FIGURE_APP_ID,
        title: `Figure ${displayNumber.toString()}: ${display.plot.title}`,
        closeButtonLabel: `Close Figure ${displayNumber.toString()}`,
        bounds: centeredFigureBounds(
          legacyFigureAppDefinition.defaultSize,
          windowManager.getDesktopSize(),
          legacyFigureAppDefinition.minimumSize,
        ),
        payload: { displayNumber, display },
        onCloseRequested: () => {
          onClose(display.sourceIndex);
          return "close";
        },
      });
    });
    parsed.invalid.forEach((display, index) => {
      const displayNumber = parsed.v1.length + parsed.legacy.length + index + 1;
      const id = invalidFigureWindowId(graphicsSessionId, display.sourceIndex);
      desiredWindowIds.add(id);
      windowManager.openWindow({
        id,
        appId: INVALID_FIGURE_APP_ID,
        title: "Invalid OpenMat Figure",
        ariaDescription: `Invalid Figure ${displayNumber.toString()}`,
        closeButtonLabel: "Close invalid Figure",
        bounds: centeredFigureBounds(
          invalidFigureAppDefinition.defaultSize,
          windowManager.getDesktopSize(),
          invalidFigureAppDefinition.minimumSize,
        ),
        payload: { message: display.message },
        onCloseRequested: () => {
          onClose(display.sourceIndex);
          return "close";
        },
      });
    });
    for (const id of ownedWindowIds.current) {
      if (!desiredWindowIds.has(id)) {
        windowManager.closeWindow(id);
        appliedPositions.current.delete(id);
      }
    }
    ownedWindowIds.current = desiredWindowIds;
  }, [
    client,
    closingFigures,
    connection,
    graphicsSessionId,
    hasWebGpu,
    onClose,
    parsed,
    presentations,
    wasmLoader,
    windowManager,
  ]);

  useEffect(
    () => () => {
      for (const id of ownedWindowIds.current) {
        windowManager.closeWindow(id);
      }
    },
    [windowManager],
  );

  return null;
}
