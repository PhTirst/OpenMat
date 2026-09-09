import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useLayoutEffect } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createFigureSvgBlob,
  downloadFigureBlob,
  createSvgTextMeasurementController,
  FigureWindow,
  prepareFigurePngCapture,
  type FigureGraphicsClient,
} from "./FigureWindow";
import type {
  GraphicsClientEvent,
  GraphicsConnectionStatus,
  RenderableFigure,
} from "../plot/graphics-client";
import type {
  FigureDiscovery,
  FigureSnapshot,
} from "../plot/graphics-v1";
import type { PlotOverlay, PlotWasmFigure } from "../plot/plot-wasm";
import { renderWithWindowManager } from "../test/render-with-window-manager";
import { OpenMatWindowLayer, OpenMatWindowManagerProvider, useOpenMatWindowManager } from "../windowing/WindowManager";
import { createOpenMatAppRegistry } from "../windowing/built-in-apps";
import styles from "../styles.css?raw";
import { PlatformContext } from "../platform/platform-services";

const attachToken = "secret-token-that-must-not-render";

function discovery(figureId: string, version: 1 | 2 = 1): FigureDiscovery {
  return version === 1
    ? {
        schemaVersion: 1,
        graphicsProtocol: "openmat-graphics-v1",
        endpoint: "/graphics/v1",
        attachToken,
        figureId,
        revision: 1,
      }
    : {
        schemaVersion: 2,
        graphicsProtocol: "openmat-graphics-v2",
        endpoint: "/graphics/v2",
        attachToken,
        figureId,
        revision: 1,
      };
}

function display(figureId: string, version: 1 | 2 = 1) {
  return {
    representations: {
      "application/vnd.openmat.figure+json": JSON.stringify(
        discovery(figureId, version),
      ),
    },
  };
}

function readBlobText(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.addEventListener("load", () => resolve(String(reader.result)), {
      once: true,
    });
    reader.addEventListener("error", () => reject(reader.error), { once: true });
    reader.readAsText(blob);
  });
}

function dispatchPointer(
  target: Element,
  type: string,
  init: MouseEventInit & { readonly pointerId?: number },
): void {
  const event = new MouseEvent(type, init);
  Object.defineProperty(event, "pointerId", {
    value: init.pointerId ?? 1,
  });
  fireEvent(target, event);
}

function snapshot(figureId: string): FigureSnapshot {
  return {
    type: "figureSnapshot",
    figureId,
    revision: 1,
    rootId: `${figureId}-root`,
    objects: [
      {
        id: `${figureId}-root`,
        generation: 1,
        objectRevision: 1,
        kind: "figure",
        parentId: null,
        children: [],
        properties: {
          number: 1,
          nameCodeUnits: [],
          numberTitle: true,
          visible: true,
          backgroundRgba: [1, 1, 1, 1],
          initialLogicalSizeCssPixels: [560, 420],
          positionCssPixels: [100, 100, 560, 420],
          nextPlot: "add",
        },
      },
    ],
    referencedBuffers: [],
  };
}

class FakeClient implements FigureGraphicsClient {
  status: GraphicsConnectionStatus = {
    phase: "idle",
    reconnectAttempt: 0,
    message: null,
  };
  readonly listeners = new Set<(event: GraphicsClientEvent) => void>();
  readonly readyFigures = new Map<string, RenderableFigure>();
  readonly discoveries: FigureDiscovery[][] = [];
  readonly closed: string[] = [];
  readonly released: string[] = [];
  readonly limits: Array<{
    figureId: string;
    expectedRevision: number;
    xLimits: readonly [number, number];
    yLimits: readonly [number, number];
  }> = [];
  readonly cameras: Array<{
    figureId: string;
    expectedRevision: number;
    view: readonly [number, number];
    cameraScale: number;
  }> = [];
  connectCalls = 0;
  disposeCalls = 0;

  matchesAttachment(value: FigureDiscovery): boolean {
    return value.attachToken === attachToken;
  }

  setDiscoveries(values: readonly FigureDiscovery[]): void {
    this.discoveries.push([...values]);
  }

  subscribe(listener: (event: GraphicsClientEvent) => void): () => void {
    this.listeners.add(listener);
    listener({ type: "connection", status: this.status });
    this.readyFigures.forEach((figure) =>
      listener({ type: "figureReady", figure }),
    );
    return () => this.listeners.delete(listener);
  }

  connect(): void {
    this.connectCalls += 1;
  }

  closeFigure(figureId: string): boolean {
    this.closed.push(figureId);
    return true;
  }

  setAxesLimits(
    figureId: string,
    expectedRevision: number,
    xLimits: readonly [number, number],
    yLimits: readonly [number, number],
  ): void {
    this.limits.push({ figureId, expectedRevision, xLimits, yLimits });
  }

  setAxesCamera(
    figureId: string,
    expectedRevision: number,
    view: readonly [number, number],
    cameraScale: number,
  ): void {
    this.cameras.push({ figureId, expectedRevision, view, cameraScale });
  }

  releaseBuffer(bufferId: string): void {
    this.released.push(bufferId);
  }

  dispose(): void {
    this.disposeCalls += 1;
  }

  emit(event: GraphicsClientEvent): void {
    act(() => {
      if (event.type === "figureReady") {
        this.readyFigures.set(event.figure.figureId, event.figure);
      }
      if (event.type === "figureClosed") {
        this.readyFigures.delete(event.figureId);
      }
      this.listeners.forEach((listener) => listener(event));
    });
  }
}

function publishFigure(client: FakeClient, figureId = "figure-1"): void {
  client.emit({
    type: "figureReady",
    figure: {
      figureId,
      revision: 1,
      snapshot: snapshot(figureId),
      buffers: new Map(),
    },
  });
}

function fakeWasm(): PlotWasmFigure & {
  readonly resize: ReturnType<typeof vi.fn>;
  readonly applySnapshot: ReturnType<typeof vi.fn>;
  readonly ingestBuffer: ReturnType<typeof vi.fn>;
  readonly render: ReturnType<typeof vi.fn>;
  readonly dispose: ReturnType<typeof vi.fn>;
} {
  return {
    state: "ready",
    lastError: () => null,
    resize: vi.fn(),
    applySnapshot: vi.fn(),
    ingestBuffer: vi.fn(),
    render: vi.fn(),
    dispose: vi.fn(),
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("FigureWindow", () => {
  it.each([true, false])("uses the platform export service and reports save result %s", async (saved) => {
    const client = new FakeClient();
    const saveExport = vi.fn(async () => saved);
    const toDataURL = vi.spyOn(HTMLCanvasElement.prototype, "toDataURL").mockReturnValue("data:image/png;base64,rendered");
    try {
      render(
        <PlatformContext.Provider value={{ kind: "desktop", saveExport }}>
          <OpenMatWindowManagerProvider registry={createOpenMatAppRegistry()}>
            <FigureWindow figures={[display("figure-1")]} onClose={vi.fn()} clientFactory={() => client} wasmLoader={async () => fakeWasm()} webGpuAvailable={true} />
            <OpenMatWindowLayer />
          </OpenMatWindowManagerProvider>
        </PlatformContext.Provider>,
      );
      publishFigure(client);
      const button = await screen.findByRole("button", { name: "Export Figure as SVG" });
      await waitFor(() => expect(button).toBeEnabled());
      fireEvent.click(button);
      await waitFor(() => expect(saveExport).toHaveBeenCalledOnce());
      expect(await screen.findByText(saved ? "Exported SVG image." : "Export canceled.")).toBeVisible();
    } finally {
      toDataURL.mockRestore();
    }
  });

  it("serializes a standalone SVG with the WebGPU canvas beneath the overlay", async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 640;
    canvas.height = 480;
    vi.spyOn(canvas, "toDataURL").mockReturnValue("data:image/png;base64,rendered");
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 320 240");
    const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
    label.textContent = "Signal";
    svg.append(label);

    const blob = createFigureSvgBlob(canvas, svg);
    const markup = await readBlobText(blob);

    expect(blob.type).toBe("image/svg+xml;charset=utf-8");
    expect(markup).toContain('viewBox="0 0 320 240"');
    expect(markup).toContain('href="data:image/png;base64,rendered"');
    expect(markup.indexOf("<image")).toBeLessThan(markup.indexOf("<text"));
    expect(markup).toContain("Signal");
  });

  it("downloads an exported blob with a browser object URL", () => {
    const createObjectURL = vi.fn(() => "blob:figure");
    const revokeObjectURL = vi.fn();
    vi.stubGlobal("URL", { createObjectURL, revokeObjectURL });
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => undefined);

    downloadFigureBlob(new Blob(["figure"]), "openmat-figure-1.svg");

    expect(createObjectURL).toHaveBeenCalledOnce();
    expect(click).toHaveBeenCalledOnce();
  });

  it("renders a fresh WebGPU frame immediately before PNG capture", () => {
    const wasm = fakeWasm();

    prepareFigurePngCapture(wasm);

    expect(wasm.render).toHaveBeenCalledOnce();
  });

  it("keeps the old MIME only as a visibly marked legacy preview", () => {
    const onClose = vi.fn();
    renderWithWindowManager(
      <FigureWindow
        figures={[
          {
            representations: {
              "application/vnd.openmat.plot+json": JSON.stringify({
                kind: "line",
                title: "Signal",
                x: [0, 1, 2],
                y: [0, 1, 4],
              }),
            },
          },
        ]}
        onClose={onClose}
      />,
    );

    expect(
      screen.getByRole("dialog", { name: "Figure 1: Signal" }),
    ).toHaveStyle({
      left: "202px", top: "130.5px", width: "620px", height: "430px",
    });
    expect(screen.getByText("Legacy preview")).toBeVisible();
    expect(screen.getByRole("img", { name: /legacy line preview/ })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Close Figure 1" }));
    expect(onClose).toHaveBeenCalledWith(0);
  });

  it("shows an explicit unsupported WebGPU state with no SVG line fallback", async () => {
    const client = new FakeClient();
    const loader = vi.fn();
    const { container } = renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={loader}
        webGpuAvailable={false}
      />,
    );

    publishFigure(client);

    expect(
      await screen.findByRole("alert", { name: "" }),
    ).toHaveTextContent("WebGPU is unavailable");
    expect(screen.getByLabelText(/WebGPU rendering surface/)).toBeVisible();
    expect(container.querySelector(".figure-svg-overlay")).not.toBeNull();
    expect(container.querySelector("polyline")).toBeNull();
    expect(loader).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain(attachToken);
  });

  it("shows the exact typed renderer failure instead of a generic rejection", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    wasm.render.mockImplementation(() => {
      throw {
        kind: "resourceLimit",
        message:
          "Plot MIR compilation failed: MeshVertex buffer requires 499.21 MiB, exceeding the configured 256.00 MiB limit.",
      };
    });
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );

    await waitFor(() => expect(client.listeners.size).toBeGreaterThan(0));
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 1,
        snapshot: snapshot("figure-1"),
        buffers: new Map(),
      },
    });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "resourceLimit: Plot MIR compilation failed: MeshVertex buffer requires 499.21 MiB, exceeding the configured 256.00 MiB limit.",
    );
  });

  it("uses one graphics client and distinct canvas windows for multiple Figures", async () => {
    const client = new FakeClient();
    const factory = vi.fn(() => client);
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1"), display("figure-2")]}
        onClose={vi.fn()}
        clientFactory={factory}
        wasmLoader={async () => fakeWasm()}
        webGpuAvailable={true}
      />,
    );

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    publishFigure(client, "figure-1");
    publishFigure(client, "figure-2");
    const dialogs = screen.getAllByRole("dialog");
    expect(dialogs).toHaveLength(2);
    expect(dialogs[0]).toHaveAttribute(
      "data-window-id",
      "figure:openmat-web-alpha:figure-1",
    );
    expect(dialogs[1]).toHaveAttribute(
      "data-window-id",
      "figure:openmat-web-alpha:figure-2",
    );
    expect(dialogs[0]).toHaveStyle({
      left: "232px", top: "135.5px", width: "560px", height: "420px",
    });
    expect(dialogs[1]).toHaveStyle({
      left: "232px", top: "135.5px", width: "560px", height: "420px",
    });
    expect(screen.getAllByLabelText(/WebGPU rendering surface/)).toHaveLength(2);
    await waitFor(() => expect(factory).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(client.discoveries.at(-1)?.map((item) => item.figureId)).toEqual([
        "figure-1",
        "figure-2",
      ]),
    );
    expect(client.connectCalls).toBe(1);
  });

  it("waits for snapshot geometry and keeps the same window while its renderer loads", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    let finishLoading!: (value: PlotWasmFigure) => void;
    const loader = vi.fn(() =>
      new Promise<PlotWasmFigure>((resolve) => {
        finishLoading = resolve;
      }),
    );
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={loader}
        webGpuAvailable={true}
      />,
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(loader).not.toHaveBeenCalled();

    // An intentionally empty figure is a valid complete snapshot, too.
    publishFigure(client);
    const dialog = screen.getByRole("dialog", { name: "Figure 1" });
    const canvas = screen.getByLabelText(/WebGPU rendering surface/);
    expect(dialog).toHaveStyle({
      left: "232px", top: "135.5px", width: "560px", height: "420px",
    });
    expect(screen.getByText("Loading the OpenMat Figure…")).toBeVisible();
    expect(screen.getByRole("button", { name: "Pan" })).toBeDisabled();

    await act(async () => finishLoading(wasm));
    expect(screen.getByRole("dialog", { name: "Figure 1" })).toBe(dialog);
    expect(screen.getByLabelText(/WebGPU rendering surface/)).toBe(canvas);
    expect(dialog).toHaveStyle({
      left: "232px", top: "135.5px", width: "560px", height: "420px",
    });
    expect(wasm.applySnapshot).toHaveBeenCalledOnce();
    expect(screen.queryByText("Loading the OpenMat Figure…")).not.toBeInTheDocument();
  });

  it("preserves local move and resize until M code changes Position", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );
    publishFigure(client);
    const dialog = screen.getByRole("dialog", { name: "Figure 1" });
    const titlebar = within(dialog).getByLabelText(/Alt plus arrow keys move/);
    fireEvent.keyDown(titlebar, { key: "ArrowRight", altKey: true });
    fireEvent.keyDown(titlebar, { key: "ArrowDown", altKey: true, ctrlKey: true });
    expect(dialog).toHaveStyle({ left: "240px", top: "135.5px", height: "428px" });
    const nextSnapshot = { ...snapshot("figure-1"), revision: 2 };
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 2,
        snapshot: nextSnapshot,
        buffers: new Map(),
      },
    });
    await waitFor(() =>
      expect(wasm.applySnapshot).toHaveBeenCalledWith(nextSnapshot),
    );
    expect(dialog).toHaveStyle({ left: "240px", top: "135.5px", height: "428px" });

    const movedSnapshot: FigureSnapshot = {
      ...nextSnapshot,
      revision: 3,
      objects: nextSnapshot.objects.map((object) =>
        object.kind === "figure"
          ? {
              ...object,
              properties: {
                ...object.properties,
                positionCssPixels: [160, 80, 580, 440],
              },
            }
          : object,
      ),
    };
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 3,
        snapshot: movedSnapshot,
        buffers: new Map(),
      },
    });
    expect(screen.getByRole("dialog", { name: "Figure 1" })).toBe(dialog);
    expect(dialog).toHaveStyle({
      left: "160px", top: "171px", width: "580px", height: "440px",
    });

    // Returning to the native default rectangle is a subsequent M Position
    // change, so it must use absolute coordinates instead of recentering.
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 4,
        snapshot: { ...snapshot("figure-1"), revision: 4 },
        buffers: new Map(),
      },
    });
    expect(dialog).toHaveStyle({
      left: "100px", top: "171px", width: "560px", height: "420px",
    });
  });

  it.each([
    { width: 1440, height: 900, left: 440, top: 240, figureWidth: 560, figureHeight: 420 },
    { width: 500, height: 380, left: 0, top: 0, figureWidth: 500, figureHeight: 380 },
    { width: 320, height: 240, left: 0, top: 0, figureWidth: 320, figureHeight: 240 },
  ])("centers a default Figure inside a $width × $height desktop", (size) => {
    const client = new FakeClient();
    const { container } = renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => fakeWasm()}
        webGpuAvailable={true}
      />,
    );
    const desktop = container.querySelector<HTMLElement>(".openmat-window-layer")!;
    // The desktop may be offset from the viewport. Placement uses its own
    // dimensions and coordinates, after fitting the window on smaller screens.
    vi.spyOn(desktop, "getBoundingClientRect").mockReturnValue(
      new DOMRect(160, 90, size.width, size.height),
    );

    publishFigure(client);

    expect(screen.getByRole("dialog", { name: "Figure 1" })).toHaveStyle({
      left: `${size.left}px`,
      top: `${size.top}px`,
      width: `${size.figureWidth}px`,
      height: `${size.figureHeight}px`,
    });
  });

  it.each(["error", "closed", "reconnecting"] as const)(
    "surfaces and dismisses a %s graphics attachment before its first snapshot",
    async (phase) => {
      const client = new FakeClient();
      const closeFigure = vi.spyOn(client, "closeFigure").mockReturnValue(false);
      const onClose = vi.fn();
      renderWithWindowManager(
        <FigureWindow
          figures={[display("figure-1")]}
          onClose={onClose}
          clientFactory={() => client}
          wasmLoader={async () => fakeWasm()}
          webGpuAvailable={true}
        />,
      );
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      client.status = {
        phase,
        reconnectAttempt: 0,
        message: "Graphics attachment failed",
      };
      client.emit({ type: "connection", status: client.status });
      expect(await screen.findByText(
        phase === "reconnecting"
          ? "Graphics channel reconnecting…"
          : "Graphics attachment failed",
      )).toBeVisible();
      expect(screen.getByRole("button", { name: "Close Figure 1" })).toBeEnabled();
      expect(screen.getByRole("button", { name: "Pan" })).toBeDisabled();
      fireEvent.click(screen.getByRole("button", { name: "Close Figure 1" }));
      await waitFor(() =>
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
      );
      expect(onClose).toHaveBeenCalledWith(0);
      expect(closeFigure).not.toHaveBeenCalled();
    },
  );

  it("never inserts a replacement session's figure using the old client's geometry", async () => {
    const firstClient = new FakeClient();
    const secondClient = new FakeClient();
    const factory = vi.fn((sessionId: string) =>
      sessionId === "session-1" ? firstClient : secondClient,
    );
    const loader = vi.fn(async () => fakeWasm());
    const figures = [display("figure-1")];
    const onClose = vi.fn();
    const insertedWindows = new Map<HTMLElement, {
      readonly id: string;
      readonly left: string;
      readonly top: string;
      readonly width: string;
      readonly height: string;
    }>();

    function ObserveCommittedWindows() {
      const state = useOpenMatWindowManager();
      useLayoutEffect(() => {
        // Capture each actual DOM insertion before passive effects can remove
        // a transient window; checking only the settled screen would miss it.
        for (const id of state.windows.keys()) {
          const element = document.querySelector<HTMLElement>(
            `[data-window-id="${id}"]`,
          );
          if (element !== null && !insertedWindows.has(element)) {
            insertedWindows.set(element, {
              id,
              left: element.style.left,
              top: element.style.top,
              width: element.style.width,
              height: element.style.height,
            });
          }
        }
      }, [state.windows]);
      return null;
    }

    const view = (sessionId: string) => (
      <>
        <ObserveCommittedWindows />
        <FigureWindow
          figures={figures}
          onClose={onClose}
          graphicsSessionId={sessionId}
          clientFactory={factory}
          wasmLoader={loader}
          webGpuAvailable={true}
        />
      </>
    );
    const result = renderWithWindowManager(view("session-1"));
    publishFigure(firstClient);
    await waitFor(() => expect(loader).toHaveBeenCalledTimes(1));
    const originalDialog = screen.getByRole("dialog", { name: "Figure 1" });
    expect(insertedWindows.get(originalDialog)).toEqual({
      id: "figure:session-1:figure-1",
      left: "232px",
      top: "135.5px",
      width: "560px",
      height: "420px",
    });

    result.rerender(view("session-2"));
    expect(firstClient.disposeCalls).toBe(1);
    expect(secondClient.connectCalls).toBe(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect([...insertedWindows.values()].map((entry) => entry.id)).toEqual([
      "figure:session-1:figure-1",
    ]);
    expect(loader).toHaveBeenCalledTimes(1);

    // A late frame from the detached client cannot open the replacement.
    publishFigure(firstClient);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(insertedWindows.size).toBe(1);

    const nextSnapshot: FigureSnapshot = {
      ...snapshot("figure-1"),
      objects: snapshot("figure-1").objects.map((object) =>
        object.kind === "figure"
          ? {
              ...object,
              properties: {
                ...object.properties,
                positionCssPixels: [240, 60, 600, 450],
              },
            }
          : object,
      ),
    };
    secondClient.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 1,
        snapshot: nextSnapshot,
        buffers: new Map(),
      },
    });
    const replacementDialog = screen.getByRole("dialog", { name: "Figure 1" });
    expect(replacementDialog).not.toBe(originalDialog);
    expect(insertedWindows.get(replacementDialog)).toEqual({
      id: "figure:session-2:figure-1",
      left: "240px",
      top: "181px",
      width: "600px",
      height: "450px",
    });
    expect(insertedWindows.size).toBe(2);
    await waitFor(() => expect(loader).toHaveBeenCalledTimes(2));
  });

  it("keeps its reconnecting window and canvas through recovery and the first snapshot", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    const loader = vi.fn(async () => wasm);
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={loader}
        webGpuAvailable={true}
      />,
    );
    client.status = { phase: "reconnecting", reconnectAttempt: 1, message: null };
    client.emit({ type: "connection", status: client.status });
    const dialog = screen.getByRole("dialog", { name: "Figure 1" });
    const canvas = screen.getByLabelText(/WebGPU rendering surface/);
    expect(screen.getByText("Graphics channel reconnecting…")).toBeVisible();
    await waitFor(() => expect(loader).toHaveBeenCalledOnce());

    for (const phase of ["connecting", "initializing", "attached"] as const) {
      client.status = { phase, reconnectAttempt: 0, message: null };
      client.emit({ type: "connection", status: client.status });
      expect(screen.getByRole("dialog", { name: "Figure 1" })).toBe(dialog);
      expect(screen.getByLabelText(/WebGPU rendering surface/)).toBe(canvas);
    }
    expect(screen.getByText("Loading the OpenMat Figure…")).toBeVisible();
    expect(screen.getByRole("button", { name: "Pan" })).toBeDisabled();

    publishFigure(client);
    expect(screen.getByRole("dialog", { name: "Figure 1" })).toBe(dialog);
    expect(screen.getByLabelText(/WebGPU rendering surface/)).toBe(canvas);
    expect(dialog).toHaveStyle({
      left: "232px", top: "135.5px", width: "560px", height: "420px",
    });
    await waitFor(() => expect(wasm.applySnapshot).toHaveBeenCalledOnce());
    expect(screen.queryByText("Loading the OpenMat Figure…")).not.toBeInTheDocument();
    expect(loader).toHaveBeenCalledOnce();
  });

  it("forwards ResizeObserver CSS size and DPR to WASM and disposes both lifecycles", async () => {
    const resizeCallbacks: ResizeObserverCallback[] = [];
    const disconnect = vi.fn();
    class FakeResizeObserver {
      constructor(callback: ResizeObserverCallback) {
        resizeCallbacks.push(callback);
      }
      observe(): void {}
      unobserve(): void {}
      disconnect(): void {
        disconnect();
      }
    }
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(2);
    const client = new FakeClient();
    const wasm = fakeWasm();
    const result = renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );
    await waitFor(() => expect(wasm.lastError).toBeDefined());
    publishFigure(client);
    await waitFor(() => expect(resizeCallbacks).toHaveLength(2));
    const callback = resizeCallbacks.at(-1);
    if (callback === undefined) {
      throw new Error("ResizeObserver callback missing");
    }
    await waitFor(() => expect(client.listeners.size).toBeGreaterThan(0));
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 1,
        snapshot: snapshot("figure-1"),
        buffers: new Map(),
      },
    });
    await waitFor(() => expect(wasm.applySnapshot).toHaveBeenCalled());
    wasm.resize.mockClear();
    wasm.render.mockClear();
    callback(
      [
        {
          contentRect: {
            width: 400,
            height: 250,
          },
        } as ResizeObserverEntry,
      ],
      {} as ResizeObserver,
    );
    expect(wasm.resize).toHaveBeenCalledWith(400, 250, 2);
    expect(wasm.render).toHaveBeenCalled();

    callback(
      [
        {
          contentRect: { width: 400, height: 250 },
        } as ResizeObserverEntry,
      ],
      {} as ResizeObserver,
    );
    expect(wasm.resize).toHaveBeenCalledTimes(1);

    vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(1.5);
    callback(
      [
        {
          contentRect: { width: 400, height: 250 },
        } as ResizeObserverEntry,
      ],
      {} as ResizeObserver,
    );
    expect(wasm.resize).toHaveBeenLastCalledWith(400, 250, 1.5);

    result.unmount();
    expect(wasm.dispose).toHaveBeenCalledTimes(1);
    expect(client.disposeCalls).toBe(1);
    expect(disconnect).toHaveBeenCalled();
  });

  it("imperatively ingests a ready Figure and releases its completed buffer lease", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    wasm.render.mockReturnValue({
      canvasWidthCssPx: 400,
      canvasHeightCssPx: 300,
      axisLines: [
        {
          x1: 40,
          y1: 260,
          x2: 360,
          y2: 260,
          color: [0, 0, 0, 1],
          widthCssPx: 1,
        },
      ],
      ticks: [],
      text: [
        {
          key: "x-tick-0-label",
          codeUnits: [48],
          x: 40,
          y: 278,
          horizontalAlignment: "center",
          verticalAlignment: "top",
          rotationRadians: 0,
          color: [0, 0, 0, 1],
          fontFamilyCodeUnits: [],
          fontSizeCssPx: 11,
          fontWeight: 400,
          fontStyle: "normal",
        },
      ],
    });
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );
    await waitFor(() => expect(client.listeners.size).toBeGreaterThan(0));
    const figure: RenderableFigure = {
      figureId: "figure-1",
      revision: 1,
      snapshot: snapshot("figure-1"),
      buffers: new Map([["buffer-1", new Uint8Array([1, 2, 3, 4])]]),
    };
    client.emit({ type: "figureReady", figure });

    await waitFor(() => expect(wasm.applySnapshot).toHaveBeenCalledWith(figure.snapshot));
    expect(wasm.ingestBuffer).toHaveBeenCalledWith(
      "buffer-1",
      figure.buffers.get("buffer-1"),
    );
    expect(wasm.render).toHaveBeenCalled();
    expect(client.released).toEqual(["buffer-1"]);
    expect(document.querySelector(".figure-svg-overlay line")).not.toBeNull();
    expect(screen.getByText("0")).toBeInTheDocument();
  });

  it("offers icon Figure tools and commits one semantic limit update per gesture", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    wasm.beginInteraction = vi.fn();
    wasm.homeView = vi.fn();
    wasm.panBy = vi.fn();
    wasm.wheelZoom = vi.fn();
    wasm.boxZoom = vi.fn();
    wasm.endInteraction = vi.fn(() => ({
      xLimits: [2, 8] as const,
      yLimits: [-1, 4] as const,
      changed: true,
    }));
    wasm.pick = vi.fn(() => ({
      pickingId: 2,
      objectId: "line-2",
      kind: "lineSegment" as const,
      primitiveIndex: 7,
      sourceIndex: 12,
      distanceCssPx: 0.25,
      x: 3.5,
      y: 1.25,
    }));
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1", 2)]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );

    publishFigure(client);
    const boxButton = await screen.findByRole("button", { name: "Box zoom" });
    await waitFor(() => expect(boxButton).toBeEnabled());
    fireEvent.click(boxButton);
    expect(boxButton).toHaveAttribute("aria-pressed", "true");
    const canvas = screen.getByLabelText(/WebGPU rendering surface/);
    dispatchPointer(canvas, "pointerdown", {
      button: 0,
      pointerId: 3,
      clientX: 10,
      clientY: 20,
    });
    dispatchPointer(canvas, "pointermove", {
      pointerId: 3,
      clientX: 90,
      clientY: 70,
    });
    dispatchPointer(canvas, "pointerup", {
      pointerId: 3,
      clientX: 90,
      clientY: 70,
    });
    expect(wasm.boxZoom).toHaveBeenCalledWith(10, 20, 90, 70);
    expect(client.limits).toEqual([
      {
        figureId: "figure-1",
        expectedRevision: 1,
        xLimits: [2, 8],
        yLimits: [-1, 4],
      },
    ]);

    fireEvent.click(screen.getByRole("button", { name: "Data cursor" }));
    dispatchPointer(canvas, "pointerdown", {
      button: 0,
      pointerId: 4,
      clientX: 30,
      clientY: 40,
    });
    expect(await screen.findByText("line-2")).toBeVisible();
    expect(screen.getByText("Point 13")).toBeVisible();
    expect(screen.getByText(/X 3\.50000/)).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Restore home view" }));
    expect(wasm.homeView).toHaveBeenCalledTimes(1);
    expect(client.limits).toHaveLength(2);
  });

  it("switches a 3D Figure to orbit controls and commits one camera transaction", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    wasm.beginInteraction = vi.fn();
    wasm.interactionDimension = vi.fn(() => "3d" as const);
    wasm.homeView = vi.fn();
    wasm.panBy = vi.fn();
    wasm.orbitBy = vi.fn();
    wasm.wheelZoom = vi.fn();
    wasm.boxZoom = vi.fn();
    wasm.endInteraction = vi.fn(() => ({
      dimension: "3d" as const,
      xLimits: [0, 1] as const,
      yLimits: [0, 1] as const,
      view: [55, 24] as const,
      cameraScale: 0.8,
      changed: true,
    }));
    wasm.pick = vi.fn(() => ({
      pickingId: 3,
      objectId: "surface-3",
      kind: "surface" as const,
      primitiveIndex: 9,
      sourceIndex: 17,
      distanceCssPx: 0.4,
      x: 1.5,
      y: -2.25,
      z: 4.75,
    }));
    const projectedOverlay: PlotOverlay = {
      canvasWidthCssPx: 360,
      canvasHeightCssPx: 240,
      axisLines: [],
      ticks: [],
      text: [
        {
          key: "z-tick-0-label",
          codeUnits: [48],
          x: 24,
          y: 80,
          horizontalAlignment: "end",
          verticalAlignment: "middle",
          rotationRadians: 0,
          color: [0.1, 0.1, 0.1, 1],
          fontFamilyCodeUnits: [],
          fontSizeCssPx: 10,
          fontWeight: 400,
          fontStyle: "normal",
          interpreter: "none",
          measuredWidthCssPx: 8,
          measuredHeightCssPx: 12,
        },
      ],
    };
    wasm.currentOverlay = vi.fn(() => projectedOverlay);
    wasm.completeTextLayout = vi.fn(() => {
      throw new Error("stale browser text measurements");
    });
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1", 2)]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );
    await waitFor(() => expect(client.listeners.size).toBeGreaterThan(0));
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 1,
        snapshot: snapshot("figure-1"),
        buffers: new Map(),
      },
    });

    const rotateButton = await screen.findByRole("button", {
      name: "Rotate 3D view",
    });
    await waitFor(() => expect(rotateButton).toBeEnabled());
    expect(screen.getByRole("button", { name: "Box zoom" })).toBeDisabled();
    const dataCursorButton = screen.getByRole("button", {
      name: "Data cursor",
    });
    expect(dataCursorButton).toBeEnabled();

    const canvas = screen.getByLabelText(/WebGPU rendering surface/);
    fireEvent.click(dataCursorButton);
    dispatchPointer(canvas, "pointerdown", {
      button: 0,
      pointerId: 7,
      clientX: 46,
      clientY: 62,
    });
    expect(await screen.findByText("surface-3")).toBeVisible();
    expect(screen.getByText("Point 18")).toBeVisible();
    expect(screen.getByText(/Z 4\.75000/)).toBeVisible();

    fireEvent.click(rotateButton);
    dispatchPointer(canvas, "pointerdown", {
      button: 0,
      pointerId: 8,
      clientX: 20,
      clientY: 40,
    });
    dispatchPointer(canvas, "pointermove", {
      pointerId: 8,
      clientX: 70,
      clientY: 25,
    });
    dispatchPointer(canvas, "pointerup", {
      pointerId: 8,
      clientX: 70,
      clientY: 25,
    });

    expect(wasm.orbitBy).toHaveBeenCalledWith(50, -15);
    expect(wasm.panBy).not.toHaveBeenCalled();
    expect(client.limits).toEqual([]);
    expect(client.cameras).toEqual([
      {
        figureId: "figure-1",
        expectedRevision: 1,
        view: [55, 24],
        cameraScale: 0.8,
      },
    ]);
    await waitFor(() => expect(wasm.completeTextLayout).toHaveBeenCalled());
    expect(
      screen.queryByText("This application could not be rendered."),
    ).not.toBeInTheDocument();
    expect(screen.getByText("0")).toBeVisible();
  });

  it("exposes R2022b PolarAxes as data-cursor only without limit commits", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    wasm.beginInteraction = vi.fn();
    wasm.interactionDimension = vi.fn(() => "polar" as const);
    wasm.interactionDimensionAt = vi.fn(() => "polar" as const);
    wasm.homeView = vi.fn();
    wasm.panBy = vi.fn();
    wasm.wheelZoom = vi.fn();
    wasm.boxZoom = vi.fn();
    wasm.endInteraction = vi.fn(() => ({
      dimension: "polar" as const,
      axesId: "polar-axes-1",
      xLimits: [0, 360] as const,
      yLimits: [0, 1.2] as const,
      changed: false,
    }));
    wasm.pick = vi.fn(() => null);

    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1", 2)]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );
    await waitFor(() => expect(client.listeners.size).toBeGreaterThan(0));
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 1,
        snapshot: snapshot("figure-1"),
        buffers: new Map(),
      },
    });

    const dataCursor = await screen.findByRole("button", { name: "Data cursor" });
    await waitFor(() => expect(dataCursor).toHaveAttribute("aria-pressed", "true"));
    expect(screen.getByRole("button", { name: "Restore home view" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Pan unavailable for PolarAxes" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Box zoom" })).toBeDisabled();

    const canvas = screen.getByLabelText(/WebGPU rendering surface/);
    fireEvent.wheel(canvas, { clientX: 30, clientY: 40, deltaY: -120 });
    expect(wasm.beginInteraction).not.toHaveBeenCalled();
    expect(wasm.wheelZoom).not.toHaveBeenCalled();
    expect(client.limits).toEqual([]);
  });

  it("renders a measured Unicode legend with a semantic Figure description", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    wasm.render.mockReturnValue({
      canvasWidthCssPx: 360,
      canvasHeightCssPx: 240,
      fontCacheRevision: 4,
      axisLines: [],
      ticks: [],
      legend: {
        x: 210,
        y: 24,
        width: 130,
        height: 42,
        background: [1, 1, 1, 0.9],
        border: [0.2, 0.2, 0.2, 1],
        entries: [
          {
            x1: 220,
            y1: 45,
            x2: 244,
            y2: 45,
            color: [0, 0.45, 0.74, 1],
            widthCssPx: 2,
          },
        ],
      },
      text: [
        {
          key: "legend-main-label-0",
          role: "legendLabel",
          codeUnits: Array.from("温度 🌡️").join("").split("").map((value) => value.charCodeAt(0)),
          x: 252,
          y: 45,
          horizontalAlignment: "start",
          verticalAlignment: "middle",
          rotationRadians: 0,
          color: [0.2, 0.2, 0.2, 1],
          fontFamilyCodeUnits: Array.from("Arial", (value) => value.charCodeAt(0)),
          fontSizeCssPx: 10,
          fontWeight: 400,
          fontStyle: "normal",
        },
      ],
    });
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );
    await waitFor(() => expect(client.listeners.size).toBeGreaterThan(0));
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 1,
        snapshot: snapshot("figure-1"),
        buffers: new Map(),
      },
    });

    expect(await screen.findByText("温度 🌡️")).toBeInTheDocument();
    expect(document.querySelector(".figure-legend rect")).not.toBeNull();
    expect(document.querySelector("figcaption")).toHaveTextContent(
      "legend: 温度 🌡️",
    );
  });

  it("typesets LaTeX with KaTeX while retaining the SVG text fast path", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    wasm.render.mockReturnValue({
      canvasWidthCssPx: 360,
      canvasHeightCssPx: 240,
      fontCacheRevision: 6,
      axisLines: [],
      ticks: [],
      text: [
        {
          key: "title-prose",
          role: "title",
          codeUnits: Array.from("2D Function Plot").map((value) => value.charCodeAt(0)),
          x: 180,
          y: 45,
          horizontalAlignment: "center",
          verticalAlignment: "top",
          rotationRadians: 0,
          color: [0.1, 0.2, 0.3, 1],
          fontFamilyCodeUnits: [],
          fontSizeCssPx: 14,
          fontWeight: 400,
          fontStyle: "normal",
          interpreter: "latex",
        },
        {
          key: "title-math",
          role: "title",
          codeUnits: Array.from("$x^2$").map((value) => value.charCodeAt(0)),
          x: 180,
          y: 20,
          horizontalAlignment: "center",
          verticalAlignment: "top",
          rotationRadians: 0,
          color: [0.1, 0.2, 0.3, 1],
          fontFamilyCodeUnits: [],
          fontSizeCssPx: 14,
          fontWeight: 400,
          fontStyle: "normal",
          interpreter: "latex",
          measuredWidthCssPx: 34,
          measuredHeightCssPx: 18,
        },
        {
          key: "tick-plain",
          role: "tickLabel",
          codeUnits: [49, 48],
          x: 40,
          y: 210,
          horizontalAlignment: "center",
          verticalAlignment: "top",
          rotationRadians: 0,
          color: [0.1, 0.2, 0.3, 1],
          fontFamilyCodeUnits: [],
          fontSizeCssPx: 10,
          fontWeight: 400,
          fontStyle: "normal",
          interpreter: "tex",
          measuredWidthCssPx: 12,
          measuredHeightCssPx: 12,
        },
      ],
    });
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => wasm}
        webGpuAvailable={true}
      />,
    );
    await waitFor(() => expect(client.listeners.size).toBeGreaterThan(0));
    client.emit({
      type: "figureReady",
      figure: {
        figureId: "figure-1",
        revision: 1,
        snapshot: snapshot("figure-1"),
        buffers: new Map(),
      },
    });

    await waitFor(() =>
      expect(document.querySelector(".figure-overlay-math .katex")).not.toBeNull(),
    );
    expect(document.querySelector("foreignObject[data-overlay-text-key]")).toBeNull();
    expect(document.querySelector("text[data-overlay-text-key='tick-plain']")).not.toBeNull();
    expect(screen.queryByText("$x^2$")).not.toBeInTheDocument();
    expect(document.querySelector('[data-overlay-text-key="title-math"]')?.textContent).toContain("x2");
    expect(
      document.querySelector('[data-overlay-text-key="title-prose"] .katex-html')
        ?.textContent?.replaceAll("\u00a0", " "),
    ).toBe("2D Function Plot");
  });

  it("measures actual browser text and stops after controller disposal", () => {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
    text.dataset.overlayTextKey = "title";
    text.setAttribute("font-size", "12");
    text.textContent = "Δ温度";
    Object.defineProperty(text, "getBBox", {
      configurable: true,
      value: () => ({ x: 0, y: 0, width: 31, height: 13 }),
    });
    svg.append(text);
    const callback = vi.fn();
    const controller = createSvgTextMeasurementController(svg, 9, callback);
    controller.measure();
    expect(callback).toHaveBeenCalledWith({
      fontRevision: 9,
      measurements: [
        expect.objectContaining({
          key: "title",
          widthCssPx: 31,
          heightCssPx: 13,
        }),
      ],
    });
    controller.dispose();
    controller.measure();
    expect(callback).toHaveBeenCalledTimes(1);
  });

  it("measures KaTeX HTML overlay boxes for the second layout pass", () => {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const html = document.createElement("div");
    html.dataset.overlayTextKey = "math-title";
    html.style.fontSize = "14px";
    Object.defineProperties(html, {
      offsetWidth: { configurable: true, value: 44 },
      offsetHeight: { configurable: true, value: 18 },
      scrollWidth: { configurable: true, value: 44 },
      scrollHeight: { configurable: true, value: 18 },
    });
    svg.append(html);
    const callback = vi.fn();
    const controller = createSvgTextMeasurementController(svg, 11, callback);
    controller.measure();
    expect(callback).toHaveBeenCalledWith({
      fontRevision: 11,
      measurements: [
        expect.objectContaining({
          key: "math-title",
          widthCssPx: 44,
          heightCssPx: 18,
        }),
      ],
    });
    controller.dispose();
  });

  it("disposes a late WASM load after the Figure unmounts", async () => {
    const client = new FakeClient();
    const wasm = fakeWasm();
    let resolveLoader!: (value: PlotWasmFigure) => void;
    const loader = vi.fn(
      () =>
        new Promise<PlotWasmFigure>((resolve) => {
          resolveLoader = resolve;
        }),
    );
    const result = renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={loader}
        webGpuAvailable={true}
      />,
    );
    publishFigure(client);
    await waitFor(() => expect(loader).toHaveBeenCalled());
    result.unmount();
    resolveLoader(wasm);
    await waitFor(() => expect(wasm.dispose).toHaveBeenCalledTimes(1));
  });

  it("keeps Modern Light scientific surfaces and Dark chrome contrast explicit", () => {
    expect(styles).toMatch(/--figure-canvas:\s*#fff/);
    expect(styles).toMatch(/--figure-frame-bg:\s*#20252b/);
    expect(styles).toMatch(
      /:root\[data-openmat-theme="modern-light"\][\s\S]*--figure-frame-bg:\s*#f7f9fa/,
    );
    expect(styles).toMatch(/\.figure-legend rect[\s\S]*stroke-width:\s*0\.75/);
  });

  it("requests protocol close and removes the matching discovery after figureClosed", async () => {
    const client = new FakeClient();
    const onClose = vi.fn();
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1"), display("figure-2")]}
        onClose={onClose}
        clientFactory={() => client}
        wasmLoader={async () => fakeWasm()}
        webGpuAvailable={true}
      />,
    );
    publishFigure(client, "figure-1");
    publishFigure(client, "figure-2");
    fireEvent.click(screen.getByRole("button", { name: "Close Figure 2" }));
    expect(client.closed).toEqual(["figure-2"]);
    client.emit({
      type: "figureClosed",
      figureId: "figure-2",
      closedRevision: 2,
    });
    await waitFor(() => expect(onClose).toHaveBeenCalledWith(1));
  });

  it("restores the close button when a protocol close fails", async () => {
    const client = new FakeClient();
    renderWithWindowManager(
      <FigureWindow
        figures={[display("figure-1")]}
        onClose={vi.fn()}
        clientFactory={() => client}
        wasmLoader={async () => fakeWasm()}
        webGpuAvailable={true}
      />,
    );
    publishFigure(client);
    const close = screen.getByRole("button", { name: "Close Figure 1" });
    fireEvent.click(close);
    expect(close).toBeDisabled();
    client.emit({
      type: "figureCloseFailed",
      figureId: "figure-1",
      category: "graphics.revisionConflict",
      message: "Figure changed while closing",
    });
    await waitFor(() => expect(close).not.toBeDisabled());
    fireEvent.click(close);
    expect(client.closed).toEqual(["figure-1", "figure-1"]);
  });
});
