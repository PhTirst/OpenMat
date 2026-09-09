import { isRecord } from "./graphics-v1";

const DEFAULT_PLOT_WASM_MODULE = "/wasm/openmat_plot_web.js";

export type PlotWasmState =
  | "created"
  | "initializing"
  | "ready"
  | "unsupportedWebGpu"
  | "rendererUnavailable"
  | "failed"
  | "disposed";

export interface PlotWasmError {
  readonly kind: string;
  readonly message: string;
}

export interface PlotOverlayLine {
  readonly x1: number;
  readonly y1: number;
  readonly x2: number;
  readonly y2: number;
  readonly color: readonly [number, number, number, number];
  readonly widthCssPx: number;
}

export interface PlotOverlayText {
  readonly key: string;
  readonly codeUnits: readonly number[];
  readonly x: number;
  readonly y: number;
  readonly horizontalAlignment: "start" | "center" | "end";
  readonly verticalAlignment: "top" | "middle" | "baseline" | "bottom";
  readonly rotationRadians: number;
  readonly color: readonly [number, number, number, number];
  readonly fontFamilyCodeUnits: readonly number[];
  readonly fontSizeCssPx: number;
  readonly fontWeight: number;
  readonly fontStyle: "normal" | "italic";
  /** Additive visual-fidelity fields; older WASM modules omit them. */
  readonly interpreter?: "tex" | "latex" | "none";
  readonly measuredWidthCssPx?: number;
  readonly measuredHeightCssPx?: number;
}

export interface PlotOverlay {
  readonly canvasWidthCssPx: number;
  readonly canvasHeightCssPx: number;
  readonly axisLines: readonly PlotOverlayLine[];
  readonly ticks: readonly PlotOverlayLine[];
  readonly text: readonly PlotOverlayText[];
}

export interface PlotInteractionResult {
  readonly axesId?: string;
  /** Omitted only by legacy WASM modules, which are treated as 2D. */
  readonly dimension?: "2d" | "polar" | "3d";
  readonly xLimits: readonly [number, number];
  readonly yLimits: readonly [number, number];
  readonly view?: readonly [number, number];
  readonly cameraScale?: number;
  readonly changed: boolean;
}

export interface PlotPickResult {
  readonly pickingId: number;
  readonly objectId: string;
  readonly kind?: "lineSegment" | "marker" | "line3" | "scatter3" | "surface";
  readonly primitiveIndex?: number;
  readonly sourceIndex?: number;
  readonly distanceCssPx?: number;
  readonly x?: number;
  readonly y?: number;
  readonly z?: number;
}

export interface PlotWasmFigure {
  readonly state: PlotWasmState | string;
  lastError(): unknown;
  resize(cssWidth: number, cssHeight: number, devicePixelRatio: number): void;
  applySnapshot(snapshot: unknown): void;
  applyDelta?(delta: unknown): void;
  ingestBuffer(bufferId: string, bytes: Uint8Array): void;
  render?(): PlotOverlay | null | void;
  completeTextLayout?(
    fontRevision: number,
    measurements: readonly unknown[],
  ): PlotOverlay | null | void;
  beginInteraction?(): void;
  beginInteractionAt?(xCssPx: number, yCssPx: number): void;
  interactionDimension?(): "2d" | "polar" | "3d";
  interactionDimensionAt?(xCssPx: number, yCssPx: number): "2d" | "polar" | "3d";
  currentOverlay?(): PlotOverlay | null;
  homeView?(): void;
  panBy?(deltaXCssPx: number, deltaYCssPx: number): void;
  orbitBy?(deltaXCssPx: number, deltaYCssPx: number): void;
  wheelZoom?(xCssPx: number, yCssPx: number, deltaY: number): void;
  boxZoom?(x1CssPx: number, y1CssPx: number, x2CssPx: number, y2CssPx: number): void;
  endInteraction?(): PlotInteractionResult;
  pick?(xCssPx: number, yCssPx: number, toleranceCssPx: number): PlotPickResult | null;
  dispose(): void;
}

export type PlotWasmLoader = (
  canvas: HTMLCanvasElement,
) => Promise<PlotWasmFigure>;

export interface PlotWasmModule {
  readonly default?: () => Promise<unknown>;
  readonly create_plot_figure?: (
    canvas: HTMLCanvasElement,
  ) => Promise<PlotWasmFigure>;
}

export type PlotWasmImporter = () => Promise<PlotWasmModule>;

export interface PlotWasmRuntime {
  readonly loadFigure: PlotWasmLoader;
  preload(): Promise<void>;
}

const defaultPlotWasmImporter: PlotWasmImporter = () => {
  const moduleUrl = new URL(DEFAULT_PLOT_WASM_MODULE, window.location.href).href;
  return import(
    /* @vite-ignore */ moduleUrl
  ) as Promise<PlotWasmModule>;
};

/**
 * Owns one browser-page Plot WASM lifecycle.
 *
 * Both the dynamic import and wasm-bindgen initialization are coalesced. Figure
 * creation is serialized so simultaneous Figure windows cannot race the first
 * shared WebGPU context initialization in Rust.
 */
export function createPlotWasmRuntime(
  importer: PlotWasmImporter,
): PlotWasmRuntime {
  let modulePromise: Promise<PlotWasmModule> | null = null;
  let initializedModulePromise: Promise<PlotWasmModule> | null = null;
  let figureCreationTail: Promise<void> = Promise.resolve();

  const importModule = (): Promise<PlotWasmModule> => {
    if (modulePromise === null) {
      const attempt = importer().catch((error: unknown) => {
        if (modulePromise === attempt) {
          modulePromise = null;
        }
        throw error;
      });
      modulePromise = attempt;
    }
    return modulePromise;
  };

  const initializeModule = (): Promise<PlotWasmModule> => {
    if (initializedModulePromise === null) {
      const attempt = importModule()
        .then(async (module) => {
          await module.default?.();
          if (module.create_plot_figure === undefined) {
            throw new Error(
              "OpenMat Plot WASM does not export create_plot_figure.",
            );
          }
          return module;
        })
        .catch((error: unknown) => {
          if (initializedModulePromise === attempt) {
            initializedModulePromise = null;
          }
          throw error;
        });
      initializedModulePromise = attempt;
    }
    return initializedModulePromise;
  };

  const loadFigure: PlotWasmLoader = (canvas) => {
    const creation = figureCreationTail.then(async () => {
      const module = await initializeModule();
      // initializeModule verifies this export before resolving.
      return module.create_plot_figure!(canvas);
    });
    figureCreationTail = creation.then(
      () => undefined,
      () => undefined,
    );
    return creation;
  };

  return {
    loadFigure,
    async preload(): Promise<void> {
      await initializeModule();
    },
  };
}

const defaultPlotWasmRuntime = createPlotWasmRuntime(defaultPlotWasmImporter);

export const loadPlotWasmFigure = defaultPlotWasmRuntime.loadFigure;
export const preloadPlotWasm = defaultPlotWasmRuntime.preload;

/** Initializes both wasm-bindgen and the shared WebGPU context ahead of the first visible Figure. */
export async function prewarmPlotWasmRenderer(): Promise<void> {
  const canvas = document.createElement("canvas");
  const figure = await loadPlotWasmFigure(canvas);
  figure.dispose();
}

export function parsePlotWasmError(value: unknown): PlotWasmError | null {
  if (
    !isRecord(value) ||
    typeof value.kind !== "string" ||
    typeof value.message !== "string"
  ) {
    return null;
  }
  return { kind: value.kind, message: value.message };
}

export function webGpuIsAvailable(navigatorValue: Navigator = navigator): boolean {
  return "gpu" in navigatorValue && navigatorValue.gpu !== undefined;
}
