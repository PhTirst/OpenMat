import { describe, expect, it } from "vitest";

import {
  constrainOpenMatWindowBounds,
  initialOpenMatWindowManagerState,
  openMatWindowManagerReducer,
  type OpenMatWindowBounds,
  type OpenMatWindowInstance,
  type OpenMatWindowManagerState,
} from "./window-state";

function instance(
  id: string,
  bounds: OpenMatWindowBounds = { x: 20, y: 30, width: 500, height: 360 },
): OpenMatWindowInstance {
  return {
    id,
    appId: "openmat.test",
    title: id,
    closeButtonLabel: `Close ${id}`,
    payload: null,
    bounds,
    minimumSize: { width: 240, height: 160 },
    restoreBounds: null,
    mode: "normal",
    minimizedFrom: null,
    closeState: "open",
    closeOnEscape: false,
    initialFocus: "window",
  };
}

function open(
  state: OpenMatWindowManagerState,
  window: OpenMatWindowInstance,
  focusExisting = false,
): OpenMatWindowManagerState {
  return openMatWindowManagerReducer(state, {
    type: "opened",
    window,
    focusExisting,
  });
}

describe("openMatWindowManagerReducer", () => {
  it("maintains deterministic focus and stacking for overlapping windows", () => {
    let state = open(initialOpenMatWindowManagerState, instance("first"));
    state = open(state, instance("second"));

    expect(state.stackingOrder).toEqual(["first", "second"]);
    expect(state.activeWindowId).toBe("second");

    state = openMatWindowManagerReducer(state, { type: "focused", id: "first" });
    expect(state.stackingOrder).toEqual(["second", "first"]);
    expect(state.activeWindowId).toBe("first");

    state = openMatWindowManagerReducer(state, { type: "minimized", id: "first" });
    expect(state.windows.get("first")?.mode).toBe("minimized");
    expect(state.activeWindowId).toBe("second");

    state = openMatWindowManagerReducer(state, { type: "restored", id: "first" });
    expect(state.windows.get("first")?.mode).toBe("normal");
    expect(state.stackingOrder).toEqual(["second", "first"]);
    expect(state.activeWindowId).toBe("first");
  });

  it("updates an existing window without destroying user bounds or focus", () => {
    let state = open(initialOpenMatWindowManagerState, instance("first"));
    state = open(state, instance("second"));
    state = open(
      state,
      {
        ...instance("first", { x: 0, y: 0, width: 200, height: 160 }),
        title: "Updated",
        payload: { revision: 2 },
      },
      false,
    );

    expect(state.windows.get("first")).toMatchObject({
      title: "Updated",
      payload: { revision: 2 },
      bounds: { x: 20, y: 30, width: 500, height: 360 },
    });
    expect(state.activeWindowId).toBe("second");
    expect(state.stackingOrder).toEqual(["first", "second"]);
  });

  it("restores and focuses a minimized existing window when requested", () => {
    let state = open(initialOpenMatWindowManagerState, instance("first"));
    state = open(state, instance("second"));
    state = openMatWindowManagerReducer(state, {
      type: "minimized",
      id: "first",
    });
    state = open(state, { ...instance("first"), title: "Reopened" }, true);

    expect(state.windows.get("first")).toMatchObject({
      title: "Reopened",
      mode: "normal",
      minimizedFrom: null,
    });
    expect(state.stackingOrder).toEqual(["second", "first"]);
    expect(state.activeWindowId).toBe("first");
  });

  it("round-trips maximize and restore while ignoring normal bounds changes", () => {
    const original = instance("figure");
    let state = open(initialOpenMatWindowManagerState, original);
    state = openMatWindowManagerReducer(state, {
      type: "maximized",
      id: original.id,
    });
    expect(state.windows.get(original.id)).toMatchObject({
      mode: "maximized",
      restoreBounds: original.bounds,
    });

    state = openMatWindowManagerReducer(state, {
      type: "boundsChanged",
      id: original.id,
      bounds: { x: 90, y: 90, width: 700, height: 500 },
    });
    expect(state.windows.get(original.id)?.bounds).toEqual(original.bounds);

    state = openMatWindowManagerReducer(state, {
      type: "restored",
      id: original.id,
    });
    expect(state.windows.get(original.id)).toMatchObject({
      mode: "normal",
      bounds: original.bounds,
      restoreBounds: null,
    });
  });

  it("restores a minimized maximized window to its maximized state", () => {
    let state = open(initialOpenMatWindowManagerState, instance("figure"));
    state = openMatWindowManagerReducer(state, {
      type: "maximized",
      id: "figure",
    });
    state = openMatWindowManagerReducer(state, {
      type: "minimized",
      id: "figure",
    });
    expect(state.windows.get("figure")).toMatchObject({
      mode: "minimized",
      minimizedFrom: "maximized",
    });

    state = openMatWindowManagerReducer(state, {
      type: "restored",
      id: "figure",
    });
    expect(state.windows.get("figure")).toMatchObject({
      mode: "maximized",
      minimizedFrom: null,
    });
  });

  it("preserves an asynchronous close state through minimize and restore", () => {
    let state = open(initialOpenMatWindowManagerState, instance("figure"));
    state = openMatWindowManagerReducer(state, {
      type: "closeStateChanged",
      id: "figure",
      closing: true,
    });
    state = openMatWindowManagerReducer(state, {
      type: "minimized",
      id: "figure",
    });
    state = openMatWindowManagerReducer(state, {
      type: "restored",
      id: "figure",
    });

    expect(state.windows.get("figure")?.closeState).toBe("closing");
  });

  it("reconstrains normal and restore bounds when the desktop shrinks", () => {
    let state = open(
      initialOpenMatWindowManagerState,
      instance("figure", { x: 900, y: 700, width: 600, height: 500 }),
    );
    state = openMatWindowManagerReducer(state, {
      type: "maximized",
      id: "figure",
    });
    state = openMatWindowManagerReducer(state, {
      type: "desktopResized",
      size: { width: 800, height: 500 },
    });

    expect(state.windows.get("figure")).toMatchObject({
      bounds: { x: 200, y: 0, width: 600, height: 500 },
      restoreBounds: { x: 200, y: 0, width: 600, height: 500 },
    });
  });

  it("selects the next visible window when the active one closes", () => {
    let state = open(initialOpenMatWindowManagerState, instance("first"));
    state = open(state, instance("second"));
    state = open(state, instance("third"));
    state = openMatWindowManagerReducer(state, { type: "minimized", id: "second" });
    state = openMatWindowManagerReducer(state, { type: "closed", id: "third" });

    expect(state.stackingOrder).toEqual(["first", "second"]);
    expect(state.activeWindowId).toBe("first");
  });
});

describe("constrainOpenMatWindowBounds", () => {
  it("keeps a window visible and respects the effective minimum size", () => {
    expect(
      constrainOpenMatWindowBounds(
        { x: -100, y: 999, width: 40, height: 900 },
        { width: 640, height: 480 },
        { width: 320, height: 240 },
      ),
    ).toEqual({ x: 0, y: 0, width: 320, height: 480 });
  });

  it("normalizes non-finite geometry at the extension boundary", () => {
    expect(
      constrainOpenMatWindowBounds(
        { x: Number.NaN, y: Infinity, width: Number.NaN, height: Infinity },
        { width: 640, height: 480 },
        { width: 320, height: 240 },
      ),
    ).toEqual({ x: 0, y: 0, width: 320, height: 240 });
  });
});
