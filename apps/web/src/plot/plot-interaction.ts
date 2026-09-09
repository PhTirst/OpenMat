import type {
  PlotInteractionResult,
  PlotOverlay,
  PlotPickResult,
  PlotWasmFigure,
} from "./plot-wasm";

const DEFAULT_WHEEL_END_DELAY_MS = 120;
const DEFAULT_PICK_TOLERANCE_CSS_PX = 6;
const MIN_BOX_SIZE_CSS_PX = 2;

export type PlotInteractionMode = "pan" | "box" | "dataCursor";

export interface PlotSelectionBox {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface PlotPickSelection {
  readonly hit: PlotPickResult | null;
  readonly xCssPx: number;
  readonly yCssPx: number;
}

export interface PlotInteractionHandlers {
  /** Called once at gesture end; the caller may commit these semantic limits to Kernel. */
  readonly onCommit: (result: PlotInteractionResult) => void;
  readonly onOverlay?: (overlay: PlotOverlay) => void;
  readonly onBoxPreview?: (box: PlotSelectionBox | null) => void;
  readonly onPick?: (selection: PlotPickSelection) => void;
  readonly onError?: (error: unknown) => void;
}

export interface PlotInteractionOptions {
  readonly wheelEndDelayMs?: number;
  readonly pickToleranceCssPx?: number;
}

export interface PlotInteractionController {
  setMode(mode: PlotInteractionMode): void;
  home(): void;
  pickAt(clientX: number, clientY: number): PlotPickResult | null;
  dispose(): void;
}

type Point = { readonly x: number; readonly y: number };

type PointerGesture =
  | {
      readonly kind: "pan" | "orbit";
      readonly pointerId: number;
      last: Point;
      changed: boolean;
    }
  | {
      readonly kind: "box";
      readonly pointerId: number;
      readonly start: Point;
      last: Point;
    };

/**
 * Owns high-frequency pointer state outside React. The WASM Figure mutates only
 * its retained renderer-local view; `onCommit` fires once when a gesture ends.
 */
export function createPlotInteractionController(
  canvas: HTMLCanvasElement,
  figure: PlotWasmFigure,
  handlers: PlotInteractionHandlers,
  options: PlotInteractionOptions = {},
): PlotInteractionController {
  const wheelEndDelayMs =
    options.wheelEndDelayMs ?? DEFAULT_WHEEL_END_DELAY_MS;
  const pickToleranceCssPx =
    options.pickToleranceCssPx ?? DEFAULT_PICK_TOLERANCE_CSS_PX;
  let pointerGesture: PointerGesture | null = null;
  let wheelTimer: ReturnType<typeof setTimeout> | null = null;
  let wheelActive = false;
  let disposed = false;
  let mode: PlotInteractionMode = "pan";

  const requireInteractionApi = (): Required<
    Pick<
      PlotWasmFigure,
      | "beginInteraction"
      | "homeView"
      | "panBy"
      | "wheelZoom"
      | "boxZoom"
      | "endInteraction"
      | "pick"
    >
  > => {
    if (
      figure.beginInteraction === undefined ||
      figure.homeView === undefined ||
      figure.panBy === undefined ||
      figure.wheelZoom === undefined ||
      figure.boxZoom === undefined ||
      figure.endInteraction === undefined ||
      figure.pick === undefined
    ) {
      throw new Error("OpenMat Plot WASM interaction API is unavailable.");
    }
    if (
      figure.interactionDimension?.() === "3d" &&
      figure.orbitBy === undefined
    ) {
      throw new Error("OpenMat Plot WASM 3D interaction API is unavailable.");
    }
    return figure as Required<
      Pick<
        PlotWasmFigure,
        | "beginInteraction"
        | "homeView"
        | "panBy"
        | "wheelZoom"
        | "boxZoom"
        | "endInteraction"
        | "pick"
      >
    >;
  };

  const reportError = (error: unknown): void => {
    pointerGesture = null;
    handlers.onBoxPreview?.(null);
    wheelActive = false;
    if (wheelTimer !== null) {
      clearTimeout(wheelTimer);
      wheelTimer = null;
    }
    handlers.onError?.(error);
  };

  const localPoint = (clientX: number, clientY: number): Point => {
    const bounds = canvas.getBoundingClientRect();
    return { x: clientX - bounds.left, y: clientY - bounds.top };
  };

  const commitGesture = (): void => {
    try {
      const result = requireInteractionApi().endInteraction();
      if (result.changed) {
        handlers.onCommit(result);
      }
    } catch (error: unknown) {
      reportError(error);
    }
  };

  const refreshProjectedOverlay = (): void => {
    const overlay = figure.currentOverlay?.();
    if (overlay !== undefined && overlay !== null) {
      handlers.onOverlay?.(overlay);
    }
  };

  const selectionBox = (first: Point, second: Point): PlotSelectionBox => ({
    x: Math.min(first.x, second.x),
    y: Math.min(first.y, second.y),
    width: Math.abs(second.x - first.x),
    height: Math.abs(second.y - first.y),
  });

  const beginInteractionAt = (
    api: Required<Pick<PlotWasmFigure, "beginInteraction">>,
    point?: Point,
  ): void => {
    if (point !== undefined && figure.beginInteractionAt !== undefined) {
      figure.beginInteractionAt(point.x, point.y);
    } else {
      api.beginInteraction();
    }
  };

  const interactionDimensionAt = (point?: Point): "2d" | "polar" | "3d" =>
    point !== undefined && figure.interactionDimensionAt !== undefined
      ? figure.interactionDimensionAt(point.x, point.y)
      : (figure.interactionDimension?.() ?? "2d");

  const home = (): void => {
    if (disposed) {
      return;
    }
    try {
      const api = requireInteractionApi();
      beginInteractionAt(api);
      api.homeView();
      refreshProjectedOverlay();
      commitGesture();
    } catch (error: unknown) {
      reportError(error);
    }
  };

  const onPointerDown = (event: PointerEvent): void => {
    if (disposed || event.button !== 0 || pointerGesture !== null) {
      return;
    }
    const point = localPoint(event.clientX, event.clientY);
    if (mode === "dataCursor" && !event.shiftKey) {
      handlers.onPick?.({
        hit: (() => {
          try {
            return requireInteractionApi().pick(
              point.x,
              point.y,
              pickToleranceCssPx,
            );
          } catch (error: unknown) {
            reportError(error);
            return null;
          }
        })(),
        xCssPx: point.x,
        yCssPx: point.y,
      });
      event.preventDefault();
      return;
    }
    try {
      const api = requireInteractionApi();
      const dimension = interactionDimensionAt(point);
      if (dimension === "polar") {
        event.preventDefault();
        return;
      }
      beginInteractionAt(api, point);
      const isThreeDimensional = dimension === "3d";
      pointerGesture = isThreeDimensional
        ? {
            kind: "orbit",
            pointerId: event.pointerId,
            last: point,
            changed: false,
          }
        : event.shiftKey || mode === "box"
        ? {
            kind: "box",
            pointerId: event.pointerId,
            start: point,
            last: point,
          }
        : {
            kind: "pan",
            pointerId: event.pointerId,
            last: point,
            changed: false,
          };
      if (pointerGesture.kind === "box") {
        handlers.onBoxPreview?.(selectionBox(point, point));
      }
      canvas.setPointerCapture?.(event.pointerId);
      event.preventDefault();
    } catch (error: unknown) {
      reportError(error);
    }
  };

  const onPointerMove = (event: PointerEvent): void => {
    const gesture = pointerGesture;
    if (disposed || gesture === null || gesture.pointerId !== event.pointerId) {
      return;
    }
    const point = localPoint(event.clientX, event.clientY);
    if (gesture.kind === "box") {
      gesture.last = point;
      handlers.onBoxPreview?.(selectionBox(gesture.start, point));
      return;
    }
    const deltaX = point.x - gesture.last.x;
    const deltaY = point.y - gesture.last.y;
    gesture.last = point;
    if (deltaX === 0 && deltaY === 0) {
      return;
    }
    try {
      const api = requireInteractionApi();
      if (gesture.kind === "orbit") {
        figure.orbitBy?.(deltaX, deltaY);
        refreshProjectedOverlay();
      } else {
        api.panBy(deltaX, deltaY);
      }
      gesture.changed = true;
    } catch (error: unknown) {
      reportError(error);
    }
  };

  const finishPointer = (event: PointerEvent): void => {
    const gesture = pointerGesture;
    if (disposed || gesture === null || gesture.pointerId !== event.pointerId) {
      return;
    }
    pointerGesture = null;
    handlers.onBoxPreview?.(null);
    canvas.releasePointerCapture?.(event.pointerId);
    try {
      if (gesture.kind === "box") {
        const width = Math.abs(gesture.last.x - gesture.start.x);
        const height = Math.abs(gesture.last.y - gesture.start.y);
        if (width < MIN_BOX_SIZE_CSS_PX || height < MIN_BOX_SIZE_CSS_PX) {
          requireInteractionApi().endInteraction();
          return;
        }
        requireInteractionApi().boxZoom(
          gesture.start.x,
          gesture.start.y,
          gesture.last.x,
          gesture.last.y,
        );
      } else if (!gesture.changed) {
        requireInteractionApi().endInteraction();
        return;
      }
      commitGesture();
    } catch (error: unknown) {
      reportError(error);
    }
  };

  const onWheel = (event: WheelEvent): void => {
    if (disposed) {
      return;
    }
    event.preventDefault();
    try {
      const api = requireInteractionApi();
      const point = localPoint(event.clientX, event.clientY);
      if (interactionDimensionAt(point) === "polar") {
        return;
      }
      if (!wheelActive) {
        beginInteractionAt(api, point);
        wheelActive = true;
      }
      api.wheelZoom(point.x, point.y, event.deltaY);
      if (interactionDimensionAt(point) === "3d") {
        refreshProjectedOverlay();
      }
      if (wheelTimer !== null) {
        clearTimeout(wheelTimer);
      }
      wheelTimer = setTimeout(() => {
        wheelTimer = null;
        wheelActive = false;
        if (!disposed) {
          commitGesture();
        }
      }, wheelEndDelayMs);
    } catch (error: unknown) {
      reportError(error);
    }
  };

  const onDoubleClick = (event: MouseEvent): void => {
    if (disposed || event.button !== 0) {
      return;
    }
    try {
      const point = localPoint(event.clientX, event.clientY);
      if (interactionDimensionAt(point) !== "polar") {
        home();
      }
    } catch (error: unknown) {
      reportError(error);
    }
  };

  canvas.addEventListener("pointerdown", onPointerDown);
  canvas.addEventListener("pointermove", onPointerMove);
  canvas.addEventListener("pointerup", finishPointer);
  canvas.addEventListener("pointercancel", finishPointer);
  canvas.addEventListener("wheel", onWheel, { passive: false });
  canvas.addEventListener("dblclick", onDoubleClick);

  return {
    setMode(nextMode: PlotInteractionMode): void {
      mode = nextMode;
    },
    home,
    pickAt(clientX: number, clientY: number): PlotPickResult | null {
      if (disposed) {
        return null;
      }
      try {
        const point = localPoint(clientX, clientY);
        return requireInteractionApi().pick(
          point.x,
          point.y,
          pickToleranceCssPx,
        );
      } catch (error: unknown) {
        reportError(error);
        return null;
      }
    },
    dispose(): void {
      if (disposed) {
        return;
      }
      disposed = true;
      pointerGesture = null;
      handlers.onBoxPreview?.(null);
      if (wheelTimer !== null) {
        clearTimeout(wheelTimer);
        wheelTimer = null;
      }
      canvas.removeEventListener("pointerdown", onPointerDown);
      canvas.removeEventListener("pointermove", onPointerMove);
      canvas.removeEventListener("pointerup", finishPointer);
      canvas.removeEventListener("pointercancel", finishPointer);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("dblclick", onDoubleClick);
    },
  };
}
