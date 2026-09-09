import { afterEach, describe, expect, it, vi } from "vitest";
import { createPlotInteractionController } from "./plot-interaction";
import type { PlotInteractionResult, PlotWasmFigure } from "./plot-wasm";

function interactiveFigure(): PlotWasmFigure {
  const result: PlotInteractionResult = {
    xLimits: [2, 8],
    yLimits: [3, 7],
    changed: true,
  };
  return {
    state: "ready",
    lastError: () => null,
    resize: vi.fn(),
    applySnapshot: vi.fn(),
    ingestBuffer: vi.fn(),
    beginInteraction: vi.fn(),
    homeView: vi.fn(),
    panBy: vi.fn(),
    wheelZoom: vi.fn(),
    boxZoom: vi.fn(),
    endInteraction: vi.fn(() => result),
    pick: vi.fn(() => ({ pickingId: 4, objectId: "series-4" })),
    dispose: vi.fn(),
  };
}

function pointerEvent(
  type: string,
  init: MouseEventInit & { pointerId?: number } = {},
): Event {
  const event = new MouseEvent(type, init);
  Object.defineProperty(event, "pointerId", {
    value: init.pointerId ?? 1,
  });
  return event;
}

afterEach(() => {
  vi.useRealTimers();
});

describe("Plot interaction controller", () => {
  it("keeps pan updates imperative and commits semantic limits once on pointer up", () => {
    const canvas = document.createElement("canvas");
    const figure = interactiveFigure();
    const onCommit = vi.fn();
    const controller = createPlotInteractionController(canvas, figure, {
      onCommit,
    });

    canvas.dispatchEvent(
      pointerEvent("pointerdown", { button: 0, clientX: 10, clientY: 20 }),
    );
    canvas.dispatchEvent(
      pointerEvent("pointermove", { clientX: 25, clientY: 15 }),
    );
    canvas.dispatchEvent(
      pointerEvent("pointermove", { clientX: 30, clientY: 25 }),
    );

    expect(figure.beginInteraction).toHaveBeenCalledTimes(1);
    expect(figure.panBy).toHaveBeenNthCalledWith(1, 15, -5);
    expect(figure.panBy).toHaveBeenNthCalledWith(2, 5, 10);
    expect(onCommit).not.toHaveBeenCalled();

    canvas.dispatchEvent(pointerEvent("pointerup", { clientX: 30, clientY: 25 }));
    expect(figure.endInteraction).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith({
      xLimits: [2, 8],
      yLimits: [3, 7],
      changed: true,
    });
    controller.dispose();
  });

  it("coalesces wheel events into one gesture-end commit and preserves CSS anchors", () => {
    vi.useFakeTimers();
    const canvas = document.createElement("canvas");
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      x: 40,
      y: 20,
      left: 40,
      top: 20,
      right: 240,
      bottom: 120,
      width: 200,
      height: 100,
      toJSON: () => ({}),
    });
    const figure = interactiveFigure();
    const onCommit = vi.fn();
    const controller = createPlotInteractionController(
      canvas,
      figure,
      { onCommit },
      { wheelEndDelayMs: 50 },
    );

    canvas.dispatchEvent(
      new WheelEvent("wheel", { clientX: 140, clientY: 70, deltaY: -100 }),
    );
    canvas.dispatchEvent(
      new WheelEvent("wheel", { clientX: 160, clientY: 80, deltaY: -40 }),
    );
    expect(figure.beginInteraction).toHaveBeenCalledTimes(1);
    expect(figure.wheelZoom).toHaveBeenNthCalledWith(1, 100, 50, -100);
    expect(figure.wheelZoom).toHaveBeenNthCalledWith(2, 120, 60, -40);
    expect(onCommit).not.toHaveBeenCalled();

    vi.advanceTimersByTime(50);
    expect(figure.endInteraction).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledTimes(1);
    controller.dispose();
  });

  it("routes a 3D pointer drag to orbit and never invokes 2D pan or box zoom", () => {
    const canvas = document.createElement("canvas");
    const figure = interactiveFigure();
    figure.interactionDimension = vi.fn(() => "3d" as const);
    figure.orbitBy = vi.fn();
    const overlay = {
      canvasWidthCssPx: 300,
      canvasHeightCssPx: 200,
      axisLines: [],
      ticks: [],
      text: [],
    };
    figure.currentOverlay = vi.fn(() => overlay);
    vi.mocked(figure.endInteraction!).mockReturnValue({
      dimension: "3d",
      xLimits: [0, 1],
      yLimits: [0, 1],
      view: [52, 18],
      cameraScale: 1.25,
      changed: true,
    });
    const onCommit = vi.fn();
    const onOverlay = vi.fn();
    const controller = createPlotInteractionController(canvas, figure, {
      onCommit,
      onOverlay,
    });
    controller.setMode("box");

    canvas.dispatchEvent(
      pointerEvent("pointerdown", {
        button: 0,
        clientX: 12,
        clientY: 16,
        shiftKey: true,
      }),
    );
    canvas.dispatchEvent(
      pointerEvent("pointermove", { clientX: 42, clientY: 6 }),
    );
    canvas.dispatchEvent(pointerEvent("pointerup", { clientX: 42, clientY: 6 }));

    expect(figure.orbitBy).toHaveBeenCalledWith(30, -10);
    expect(onOverlay).toHaveBeenCalledWith(overlay);
    expect(figure.panBy).not.toHaveBeenCalled();
    expect(figure.boxZoom).not.toHaveBeenCalled();
    expect(onCommit).toHaveBeenCalledWith({
      dimension: "3d",
      xLimits: [0, 1],
      yLimits: [0, 1],
      view: [52, 18],
      cameraScale: 1.25,
      changed: true,
    });
    controller.dispose();
  });

  it("keeps R2022b PolarAxes data-tip only and ignores navigation gestures", () => {
    vi.useFakeTimers();
    const canvas = document.createElement("canvas");
    const figure = interactiveFigure();
    figure.interactionDimension = vi.fn(() => "polar" as const);
    figure.interactionDimensionAt = vi.fn(() => "polar" as const);
    const onCommit = vi.fn();
    const onPick = vi.fn();
    const onError = vi.fn();
    const controller = createPlotInteractionController(canvas, figure, {
      onCommit,
      onPick,
      onError,
    });

    controller.setMode("box");
    canvas.dispatchEvent(
      pointerEvent("pointerdown", { button: 0, clientX: 12, clientY: 16 }),
    );
    canvas.dispatchEvent(
      pointerEvent("pointermove", { clientX: 42, clientY: 36 }),
    );
    canvas.dispatchEvent(pointerEvent("pointerup", { clientX: 42, clientY: 36 }));
    canvas.dispatchEvent(
      new WheelEvent("wheel", { clientX: 20, clientY: 25, deltaY: -120 }),
    );
    canvas.dispatchEvent(
      new MouseEvent("dblclick", { button: 0, clientX: 20, clientY: 25 }),
    );
    vi.runAllTimers();

    expect(figure.beginInteraction).not.toHaveBeenCalled();
    expect(figure.panBy).not.toHaveBeenCalled();
    expect(figure.wheelZoom).not.toHaveBeenCalled();
    expect(figure.boxZoom).not.toHaveBeenCalled();
    expect(figure.homeView).not.toHaveBeenCalled();
    expect(figure.endInteraction).not.toHaveBeenCalled();
    expect(onCommit).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();

    controller.setMode("dataCursor");
    canvas.dispatchEvent(
      pointerEvent("pointerdown", { button: 0, clientX: 17, clientY: 19 }),
    );
    expect(onPick).toHaveBeenCalledWith({
      hit: { pickingId: 4, objectId: "series-4" },
      xCssPx: 17,
      yCssPx: 19,
    });
    controller.dispose();
  });

  it("applies box zoom only at gesture end and exposes deterministic picking", () => {
    const canvas = document.createElement("canvas");
    const figure = interactiveFigure();
    const onCommit = vi.fn();
    const onBoxPreview = vi.fn();
    const controller = createPlotInteractionController(canvas, figure, {
      onCommit,
      onBoxPreview,
    });

    canvas.dispatchEvent(
      pointerEvent("pointerdown", {
        button: 0,
        clientX: 5,
        clientY: 10,
        shiftKey: true,
      }),
    );
    canvas.dispatchEvent(
      pointerEvent("pointermove", { clientX: 80, clientY: 60 }),
    );
    expect(onBoxPreview).toHaveBeenLastCalledWith({
      x: 5,
      y: 10,
      width: 75,
      height: 50,
    });
    expect(figure.boxZoom).not.toHaveBeenCalled();
    canvas.dispatchEvent(pointerEvent("pointerup", { clientX: 80, clientY: 60 }));
    expect(onBoxPreview).toHaveBeenLastCalledWith(null);
    expect(figure.boxZoom).toHaveBeenCalledWith(5, 10, 80, 60);
    expect(onCommit).toHaveBeenCalledTimes(1);

    expect(controller.pickAt(50, 30)).toEqual({
      pickingId: 4,
      objectId: "series-4",
    });
    expect(figure.pick).toHaveBeenCalledWith(50, 30, 6);
    controller.dispose();
  });

  it("supports explicit modes, one-shot home, and data-cursor selection", () => {
    const canvas = document.createElement("canvas");
    const figure = interactiveFigure();
    const onCommit = vi.fn();
    const onPick = vi.fn();
    const controller = createPlotInteractionController(canvas, figure, {
      onCommit,
      onPick,
    });

    controller.setMode("box");
    canvas.dispatchEvent(
      pointerEvent("pointerdown", { button: 0, clientX: 4, clientY: 6 }),
    );
    canvas.dispatchEvent(pointerEvent("pointermove", { clientX: 40, clientY: 60 }));
    canvas.dispatchEvent(pointerEvent("pointerup", { clientX: 40, clientY: 60 }));
    expect(figure.boxZoom).toHaveBeenCalledWith(4, 6, 40, 60);

    controller.setMode("dataCursor");
    canvas.dispatchEvent(
      pointerEvent("pointerdown", { button: 0, clientX: 17, clientY: 19 }),
    );
    expect(onPick).toHaveBeenCalledWith({
      hit: { pickingId: 4, objectId: "series-4" },
      xCssPx: 17,
      yCssPx: 19,
    });

    vi.mocked(figure.endInteraction!).mockReturnValueOnce({
      xLimits: [0, 1],
      yLimits: [0, 1],
      changed: false,
    });
    controller.home();
    expect(figure.homeView).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledTimes(1);
    controller.dispose();
  });

  it("disposal cancels delayed commits and device-loss errors stay local", () => {
    vi.useFakeTimers();
    const canvas = document.createElement("canvas");
    const figure = interactiveFigure();
    const deviceLoss = { kind: "deviceLost", message: "adapter reset" };
    vi.mocked(figure.panBy!).mockImplementation(() => {
      throw deviceLoss;
    });
    const onCommit = vi.fn();
    const onError = vi.fn();
    const controller = createPlotInteractionController(canvas, figure, {
      onCommit,
      onError,
    });

    canvas.dispatchEvent(
      pointerEvent("pointerdown", { button: 0, clientX: 10, clientY: 10 }),
    );
    canvas.dispatchEvent(
      pointerEvent("pointermove", { clientX: 20, clientY: 20 }),
    );
    expect(onError).toHaveBeenCalledWith(deviceLoss);
    expect(onCommit).not.toHaveBeenCalled();

    canvas.dispatchEvent(
      new WheelEvent("wheel", { clientX: 10, clientY: 10, deltaY: 20 }),
    );
    controller.dispose();
    controller.dispose();
    vi.runAllTimers();
    expect(onCommit).not.toHaveBeenCalled();
  });
});
