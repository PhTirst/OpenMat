import { describe, expect, it, vi } from "vitest";
import {
  createPlotWasmRuntime,
  type PlotWasmFigure,
  type PlotWasmModule,
} from "./plot-wasm";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function figure(): PlotWasmFigure {
  return {
    state: "ready",
    lastError: () => null,
    resize: vi.fn(),
    applySnapshot: vi.fn(),
    ingestBuffer: vi.fn(),
    dispose: vi.fn(),
  };
}

describe("Plot WASM runtime", () => {
  it("coalesces initialization and serializes concurrent Figure creation", async () => {
    const initialization = deferred<unknown>();
    const firstCreation = deferred<PlotWasmFigure>();
    const secondFigure = figure();
    const initialize = vi.fn(() => initialization.promise);
    let activeCreations = 0;
    let maximumActiveCreations = 0;
    const create = vi.fn(async () => {
      activeCreations += 1;
      maximumActiveCreations = Math.max(
        maximumActiveCreations,
        activeCreations,
      );
      try {
        return create.mock.calls.length === 1
          ? await firstCreation.promise
          : secondFigure;
      } finally {
        activeCreations -= 1;
      }
    });
    const module: PlotWasmModule = {
      default: initialize,
      create_plot_figure: create,
    };
    const importer = vi.fn(async () => module);
    const runtime = createPlotWasmRuntime(importer);
    const canvasA = document.createElement("canvas");
    const canvasB = document.createElement("canvas");

    const preload = runtime.preload();
    const first = runtime.loadFigure(canvasA);
    const second = runtime.loadFigure(canvasB);
    await Promise.resolve();
    await Promise.resolve();

    expect(importer).toHaveBeenCalledTimes(1);
    expect(initialize).toHaveBeenCalledTimes(1);
    expect(create).not.toHaveBeenCalled();

    initialization.resolve(undefined);
    await preload;
    await Promise.resolve();
    expect(create).toHaveBeenCalledTimes(1);
    expect(create).toHaveBeenNthCalledWith(1, canvasA);

    firstCreation.resolve(figure());
    await first;
    await second;
    expect(create).toHaveBeenCalledTimes(2);
    expect(create).toHaveBeenNthCalledWith(2, canvasB);
    expect(maximumActiveCreations).toBe(1);
  });

  it("allows wasm-bindgen initialization to be retried after failure", async () => {
    const create = vi.fn(async () => figure());
    const initialize = vi
      .fn<() => Promise<unknown>>()
      .mockRejectedValueOnce(new Error("first initialization failed"))
      .mockResolvedValue(undefined);
    const importer = vi.fn(async () => ({
      default: initialize,
      create_plot_figure: create,
    }));
    const runtime = createPlotWasmRuntime(importer);

    await expect(runtime.preload()).rejects.toThrow("first initialization failed");
    await expect(runtime.loadFigure(document.createElement("canvas"))).resolves.toBeDefined();

    expect(importer).toHaveBeenCalledTimes(1);
    expect(initialize).toHaveBeenCalledTimes(2);
    expect(create).toHaveBeenCalledTimes(1);
  });
});
