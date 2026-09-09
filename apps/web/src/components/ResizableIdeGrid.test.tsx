import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_IDE_LAYOUT,
  IDE_LAYOUT_STORAGE_KEY,
} from "../layout/ide-layout";
import { ResizableIdeGrid } from "./ResizableIdeGrid";

type MediaListener = (event: MediaQueryListEvent) => void;

let containerWidth = 1200;
let containerHeight = 700;
let resizeCallback: ResizeObserverCallback | undefined;
let mediaMatches = true;
const mediaListeners = new Set<MediaListener>();

const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalSetPointerCapture = HTMLElement.prototype.setPointerCapture;
const originalReleasePointerCapture = HTMLElement.prototype.releasePointerCapture;
const originalHasPointerCapture = HTMLElement.prototype.hasPointerCapture;

class TestPointerEvent extends MouseEvent {
  readonly pointerId: number;

  constructor(type: string, init: PointerEventInit = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 0;
  }
}

class TestResizeObserver implements ResizeObserver {
  constructor(callback: ResizeObserverCallback) {
    resizeCallback = callback;
  }

  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

function rect(width: number, height: number): DOMRect {
  return {
    x: 0,
    y: 0,
    top: 0,
    right: width,
    bottom: height,
    left: 0,
    width,
    height,
    toJSON: () => ({}),
  };
}

function setDesktop(matches: boolean): void {
  mediaMatches = matches;
  const event = { matches, media: "(min-width: 861px)" } as MediaQueryListEvent;
  for (const listener of mediaListeners) {
    listener(event);
  }
}

function renderGrid() {
  return render(
    <ResizableIdeGrid>
      <section className="pane current-folder-pane">Current Folder</section>
      <section className="pane editor-pane">Editor</section>
      <section className="pane command-window-pane">Command Window</section>
      <section className="pane workspace-pane">Workspace</section>
      <section className="pane command-history-pane">Command History</section>
    </ResizableIdeGrid>,
  );
}

function splitter(name: string): HTMLElement {
  return screen.getByRole("separator", { name });
}

beforeEach(() => {
  localStorage.clear();
  containerWidth = 1200;
  containerHeight = 700;
  resizeCallback = undefined;
  mediaMatches = true;
  mediaListeners.clear();

  vi.stubGlobal("PointerEvent", TestPointerEvent);
  vi.stubGlobal("ResizeObserver", TestResizeObserver);
  vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: mediaMatches,
      media: query,
      onchange: null,
      addEventListener: (_type: string, listener: MediaListener) =>
        mediaListeners.add(listener),
      removeEventListener: (_type: string, listener: MediaListener) =>
        mediaListeners.delete(listener),
      addListener: (listener: MediaListener) => mediaListeners.add(listener),
      removeListener: (listener: MediaListener) => mediaListeners.delete(listener),
      dispatchEvent: () => true,
    })),
  });
  HTMLElement.prototype.getBoundingClientRect = function getBoundingClientRect() {
    return this.classList.contains("resizable-ide-grid")
      ? rect(containerWidth, containerHeight)
      : rect(0, 0);
  };

  const captured = new WeakMap<HTMLElement, Set<number>>();
  HTMLElement.prototype.setPointerCapture = vi.fn(function setPointerCapture(
    this: HTMLElement,
    pointerId: number,
  ) {
    const pointers = captured.get(this) ?? new Set<number>();
    pointers.add(pointerId);
    captured.set(this, pointers);
  });
  HTMLElement.prototype.hasPointerCapture = vi.fn(function hasPointerCapture(
    this: HTMLElement,
    pointerId: number,
  ) {
    return captured.get(this)?.has(pointerId) === true;
  });
  HTMLElement.prototype.releasePointerCapture = vi.fn(function releasePointerCapture(
    this: HTMLElement,
    pointerId: number,
  ) {
    captured.get(this)?.delete(pointerId);
  });
});

afterEach(() => {
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  HTMLElement.prototype.setPointerCapture = originalSetPointerCapture;
  HTMLElement.prototype.releasePointerCapture = originalReleasePointerCapture;
  HTMLElement.prototype.hasPointerCapture = originalHasPointerCapture;
  vi.unstubAllGlobals();
});

describe("ResizableIdeGrid", () => {
  it("drags with pointer capture, batches pointer moves, and restores the persisted layout", () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    const first = renderGrid();
    const left = splitter("Resize Current Folder and Editor");
    const initialValue = Number(left.getAttribute("aria-valuenow"));
    const right = splitter("Resize Editor and Workspace columns");
    const initialRightValue = right.getAttribute("aria-valuenow");

    expect(screen.getAllByRole("separator")).toHaveLength(3);
    expect(left).toHaveAttribute("aria-orientation", "vertical");
    expect(splitter("Resize upper and lower IDE panels")).toHaveAttribute(
      "aria-orientation",
      "horizontal",
    );

    fireEvent.pointerDown(left, { pointerId: 7, button: 0, clientX: 216 });
    expect(left.setPointerCapture).toHaveBeenCalledWith(7);
    expect(document.body).toHaveClass("ide-layout-resizing");

    fireEvent.pointerMove(left, { pointerId: 7, clientX: 250 });
    fireEvent.pointerMove(left, { pointerId: 7, clientX: 275 });
    fireEvent.pointerMove(left, { pointerId: 7, clientX: 296 });
    expect(requestAnimationFrame).toHaveBeenCalledTimes(1);
    expect(setItem).not.toHaveBeenCalled();

    fireEvent.pointerUp(left, { pointerId: 7, clientX: 296 });
    expect(left.releasePointerCapture).toHaveBeenCalledWith(7);
    expect(document.body).not.toHaveClass("ide-layout-resizing");
    expect(setItem).toHaveBeenCalledTimes(1);
    expect(setItem).toHaveBeenCalledWith(
      IDE_LAYOUT_STORAGE_KEY,
      expect.any(String),
    );
    expect(Number(left.getAttribute("aria-valuenow"))).toBeGreaterThan(
      initialValue,
    );
    expect(right).toHaveAttribute("aria-valuenow", initialRightValue);

    const persistedValue = left.getAttribute("aria-valuenow");
    first.unmount();
    renderGrid();
    expect(splitter("Resize Current Folder and Editor")).toHaveAttribute(
      "aria-valuenow",
      persistedValue,
    );
  });

  it("supports arrow keys plus Home and End with current ARIA values", () => {
    renderGrid();
    const right = splitter("Resize Editor and Workspace columns");
    const initialRight = Number(right.getAttribute("aria-valuenow"));

    fireEvent.keyDown(right, { key: "ArrowLeft" });
    expect(Number(right.getAttribute("aria-valuenow"))).toBeLessThan(
      initialRight,
    );
    fireEvent.keyDown(right, { key: "Home" });
    expect(right).toHaveAttribute(
      "aria-valuenow",
      right.getAttribute("aria-valuemin"),
    );
    fireEvent.keyDown(right, { key: "End" });
    expect(right).toHaveAttribute(
      "aria-valuenow",
      right.getAttribute("aria-valuemax"),
    );

    const horizontal = splitter("Resize upper and lower IDE panels");
    const initialTop = Number(horizontal.getAttribute("aria-valuenow"));
    fireEvent.keyDown(horizontal, { key: "ArrowDown", shiftKey: true });
    expect(Number(horizontal.getAttribute("aria-valuenow"))).toBeGreaterThan(
      initialTop,
    );
    expect(JSON.parse(localStorage.getItem(IDE_LAYOUT_STORAGE_KEY) ?? "null")).toMatchObject({
      version: 1,
    });
  });

  it("falls back safely for malformed, incompatible, and invalid stored values", () => {
    localStorage.setItem(IDE_LAYOUT_STORAGE_KEY, "not-json");
    const malformed = renderGrid();
    const defaultValue = Number(
      splitter("Resize Current Folder and Editor").getAttribute(
        "aria-valuenow",
      ),
    );
    expect(defaultValue).toBe(Math.round(DEFAULT_IDE_LAYOUT.leftFraction * 100));

    malformed.unmount();
    localStorage.setItem(
      IDE_LAYOUT_STORAGE_KEY,
      JSON.stringify({
        version: 0,
        leftFraction: 0.3,
        rightFraction: 0.3,
        topFraction: 0.5,
      }),
    );
    const oldVersion = renderGrid();
    expect(splitter("Resize Current Folder and Editor")).toHaveAttribute(
      "aria-valuenow",
      defaultValue.toString(),
    );

    oldVersion.unmount();
    localStorage.setItem(
      IDE_LAYOUT_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        leftFraction: -1,
        rightFraction: 2,
        topFraction: null,
      }),
    );
    renderGrid();
    expect(splitter("Resize Current Folder and Editor")).toHaveAttribute(
      "aria-valuenow",
      defaultValue.toString(),
    );
  });

  it("clamps every pane when ResizeObserver reports a smaller desktop", () => {
    localStorage.setItem(
      IDE_LAYOUT_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        leftFraction: 0.45,
        rightFraction: 0.4,
        topFraction: 0.8,
      }),
    );
    const { container } = renderGrid();
    const grid = container.querySelector<HTMLElement>(".resizable-ide-grid");
    expect(grid).not.toBeNull();

    act(() => {
      containerWidth = 861;
      containerHeight = 400;
      resizeCallback?.(
        [
          {
            contentRect: rect(containerWidth, containerHeight),
          } as ResizeObserverEntry,
        ],
        {} as ResizeObserver,
      );
    });

    const columns = grid?.style.gridTemplateColumns
      .split(" ")
      .filter((value) => value.endsWith("px"))
      .map((value) => Number.parseFloat(value)) ?? [];
    const rows = grid?.style.gridTemplateRows
      .split(" ")
      .filter((value) => value.endsWith("px"))
      .map((value) => Number.parseFloat(value)) ?? [];
    expect(columns[0]).toBeGreaterThanOrEqual(160);
    expect(columns[2]).toBeGreaterThanOrEqual(360);
    expect(columns[4]).toBeGreaterThanOrEqual(220);
    expect(rows[0]).toBeGreaterThanOrEqual(240);
    expect(rows[2]).toBeGreaterThanOrEqual(96);
  });

  it("removes meaningless splitters while retaining all panes in responsive mode", () => {
    const { container } = renderGrid();
    expect(screen.getAllByRole("separator")).toHaveLength(3);

    act(() => setDesktop(false));

    expect(screen.queryByRole("separator")).toBeNull();
    expect(container.querySelector(".resizable-ide-grid")).toHaveAttribute(
      "data-layout-mode",
      "responsive",
    );
    for (const pane of [
      "Current Folder",
      "Editor",
      "Command Window",
      "Workspace",
      "Command History",
    ]) {
      expect(screen.getByText(pane)).toBeVisible();
    }
  });
});
