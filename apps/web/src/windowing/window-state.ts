export type OpenMatWindowMode = "normal" | "maximized" | "minimized";

export interface OpenMatWindowBounds {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface OpenMatWindowSize {
  readonly width: number;
  readonly height: number;
}

export type OpenMatWindowCloseDecision = "close" | "cancel" | "defer";

export type OpenMatWindowCloseHandler = () =>
  | OpenMatWindowCloseDecision
  | Promise<OpenMatWindowCloseDecision>;

export interface OpenMatWindowInstance {
  readonly id: string;
  readonly appId: string;
  readonly title: string;
  readonly ariaDescription?: string;
  readonly closeButtonLabel: string;
  readonly payload: unknown;
  readonly bounds: OpenMatWindowBounds;
  readonly minimumSize: OpenMatWindowSize;
  readonly restoreBounds: OpenMatWindowBounds | null;
  readonly mode: OpenMatWindowMode;
  readonly minimizedFrom: Exclude<OpenMatWindowMode, "minimized"> | null;
  readonly closeState: "open" | "closing";
  readonly closeOnEscape: boolean;
  readonly initialFocus: "window" | "close";
  readonly onCloseRequested?: OpenMatWindowCloseHandler;
}

export interface OpenMatWindowManagerState {
  readonly windows: ReadonlyMap<string, OpenMatWindowInstance>;
  readonly stackingOrder: readonly string[];
  readonly activeWindowId: string | null;
}

export const initialOpenMatWindowManagerState: OpenMatWindowManagerState = {
  windows: new Map(),
  stackingOrder: [],
  activeWindowId: null,
};

export type OpenMatWindowManagerAction =
  | {
      readonly type: "opened";
      readonly window: OpenMatWindowInstance;
      readonly focusExisting: boolean;
    }
  | {
      readonly type: "updated";
      readonly id: string;
      readonly patch: Partial<
        Pick<
          OpenMatWindowInstance,
          | "title"
          | "ariaDescription"
          | "closeButtonLabel"
          | "payload"
          | "minimumSize"
          | "closeOnEscape"
          | "initialFocus"
          | "onCloseRequested"
        >
      >;
    }
  | { readonly type: "focused"; readonly id: string }
  | {
      readonly type: "boundsChanged";
      readonly id: string;
      readonly bounds: OpenMatWindowBounds;
    }
  | { readonly type: "minimized"; readonly id: string }
  | { readonly type: "maximized"; readonly id: string }
  | { readonly type: "restored"; readonly id: string }
  | {
      readonly type: "closeStateChanged";
      readonly id: string;
      readonly closing: boolean;
    }
  | { readonly type: "closed"; readonly id: string }
  | { readonly type: "desktopResized"; readonly size: OpenMatWindowSize };

function activeVisibleWindow(
  windows: ReadonlyMap<string, OpenMatWindowInstance>,
  stackingOrder: readonly string[],
): string | null {
  for (let index = stackingOrder.length - 1; index >= 0; index -= 1) {
    const id = stackingOrder[index];
    if (id !== undefined && windows.get(id)?.mode !== "minimized") {
      return id;
    }
  }
  return null;
}

function moveToTop(
  stackingOrder: readonly string[],
  id: string,
): readonly string[] {
  return [...stackingOrder.filter((candidate) => candidate !== id), id];
}

function replaceWindow(
  state: OpenMatWindowManagerState,
  id: string,
  replace: (window: OpenMatWindowInstance) => OpenMatWindowInstance,
): OpenMatWindowManagerState {
  const current = state.windows.get(id);
  if (current === undefined) {
    return state;
  }
  const windows = new Map(state.windows);
  windows.set(id, replace(current));
  return { ...state, windows };
}

function boundsMatch(
  left: OpenMatWindowBounds | null,
  right: OpenMatWindowBounds | null,
): boolean {
  return (
    left === right ||
    (left !== null &&
      right !== null &&
      left.x === right.x &&
      left.y === right.y &&
      left.width === right.width &&
      left.height === right.height)
  );
}

export function constrainOpenMatWindowBounds(
  bounds: OpenMatWindowBounds,
  desktop: OpenMatWindowSize,
  minimumSize: OpenMatWindowSize,
): OpenMatWindowBounds {
  const desktopWidth =
    Number.isFinite(desktop.width) && desktop.width > 0 ? desktop.width : 1;
  const desktopHeight =
    Number.isFinite(desktop.height) && desktop.height > 0 ? desktop.height : 1;
  const requestedMinimumWidth =
    Number.isFinite(minimumSize.width) && minimumSize.width > 0
      ? minimumSize.width
      : 1;
  const requestedMinimumHeight =
    Number.isFinite(minimumSize.height) && minimumSize.height > 0
      ? minimumSize.height
      : 1;
  const minimumWidth = Math.min(requestedMinimumWidth, desktopWidth);
  const minimumHeight = Math.min(requestedMinimumHeight, desktopHeight);
  const requestedWidth = Number.isFinite(bounds.width)
    ? bounds.width
    : minimumWidth;
  const requestedHeight = Number.isFinite(bounds.height)
    ? bounds.height
    : minimumHeight;
  const width = Math.min(Math.max(minimumWidth, requestedWidth), desktopWidth);
  const height = Math.min(
    Math.max(minimumHeight, requestedHeight),
    desktopHeight,
  );
  const requestedX = Number.isFinite(bounds.x) ? bounds.x : 0;
  const requestedY = Number.isFinite(bounds.y) ? bounds.y : 0;
  return {
    x: Math.min(Math.max(0, requestedX), desktopWidth - width),
    y: Math.min(Math.max(0, requestedY), desktopHeight - height),
    width,
    height,
  };
}

export function openMatWindowManagerReducer(
  state: OpenMatWindowManagerState,
  action: OpenMatWindowManagerAction,
): OpenMatWindowManagerState {
  switch (action.type) {
    case "opened": {
      const current = state.windows.get(action.window.id);
      if (current !== undefined) {
        const windows = new Map(state.windows);
        const {
          ariaDescription: _currentAriaDescription,
          onCloseRequested: _currentOnCloseRequested,
          ...requiredCurrent
        } = current;
        windows.set(action.window.id, {
          ...requiredCurrent,
          appId: action.window.appId,
          title: action.window.title,
          ...(action.window.ariaDescription === undefined
            ? {}
            : { ariaDescription: action.window.ariaDescription }),
          closeButtonLabel: action.window.closeButtonLabel,
          payload: action.window.payload,
          minimumSize: action.window.minimumSize,
          closeOnEscape: action.window.closeOnEscape,
          initialFocus: action.window.initialFocus,
          closeState: action.window.closeState,
          mode:
            action.focusExisting && current.mode === "minimized"
              ? current.minimizedFrom ?? "normal"
              : current.mode,
          minimizedFrom: action.focusExisting ? null : current.minimizedFrom,
          ...(action.window.onCloseRequested === undefined
            ? {}
            : { onCloseRequested: action.window.onCloseRequested }),
        });
        if (!action.focusExisting) {
          return { ...state, windows };
        }
        const stackingOrder = moveToTop(state.stackingOrder, action.window.id);
        return {
          windows,
          stackingOrder,
          activeWindowId: action.window.id,
        };
      }
      const windows = new Map(state.windows);
      windows.set(action.window.id, action.window);
      const stackingOrder = moveToTop(state.stackingOrder, action.window.id);
      return {
        windows,
        stackingOrder,
        activeWindowId: action.window.id,
      };
    }
    case "updated":
      return replaceWindow(state, action.id, (window) => ({
        ...window,
        ...action.patch,
      }));
    case "focused": {
      const window = state.windows.get(action.id);
      if (window === undefined || window.mode === "minimized") {
        return state;
      }
      return {
        ...state,
        stackingOrder: moveToTop(state.stackingOrder, action.id),
        activeWindowId: action.id,
      };
    }
    case "boundsChanged":
      return replaceWindow(state, action.id, (window) =>
        window.mode === "normal" ? { ...window, bounds: action.bounds } : window,
      );
    case "minimized": {
      const next = replaceWindow(state, action.id, (window) => ({
        ...window,
        mode: "minimized",
        minimizedFrom:
          window.mode === "minimized" ? window.minimizedFrom : window.mode,
      }));
      return next === state
        ? state
        : {
            ...next,
            activeWindowId: activeVisibleWindow(next.windows, next.stackingOrder),
          };
    }
    case "maximized": {
      const next = replaceWindow(state, action.id, (window) => ({
        ...window,
        restoreBounds:
          window.mode === "normal" ? window.bounds : window.restoreBounds,
        mode: "maximized",
        minimizedFrom: null,
      }));
      return next === state
        ? state
        : {
            ...next,
            stackingOrder: moveToTop(next.stackingOrder, action.id),
            activeWindowId: action.id,
          };
    }
    case "restored": {
      const next = replaceWindow(state, action.id, (window) => ({
        ...window,
        bounds:
          window.mode === "maximized"
            ? window.restoreBounds ?? window.bounds
            : window.bounds,
        restoreBounds:
          window.mode === "minimized" && window.minimizedFrom === "maximized"
            ? window.restoreBounds
            : null,
        mode:
          window.mode === "minimized"
            ? window.minimizedFrom ?? "normal"
            : "normal",
        minimizedFrom: null,
      }));
      return next === state
        ? state
        : {
            ...next,
            stackingOrder: moveToTop(next.stackingOrder, action.id),
            activeWindowId: action.id,
          };
    }
    case "closeStateChanged":
      return replaceWindow(state, action.id, (window) => ({
        ...window,
        closeState: action.closing ? "closing" : "open",
      }));
    case "closed": {
      if (!state.windows.has(action.id)) {
        return state;
      }
      const windows = new Map(state.windows);
      windows.delete(action.id);
      const stackingOrder = state.stackingOrder.filter((id) => id !== action.id);
      return {
        windows,
        stackingOrder,
        activeWindowId:
          state.activeWindowId === action.id
            ? activeVisibleWindow(windows, stackingOrder)
            : state.activeWindowId,
      };
    }
    case "desktopResized": {
      let changed = false;
      const windows = new Map<string, OpenMatWindowInstance>();
      for (const [id, window] of state.windows) {
        const bounds = constrainOpenMatWindowBounds(
          window.bounds,
          action.size,
          window.minimumSize,
        );
        const restoreBounds =
          window.restoreBounds === null
            ? null
            : constrainOpenMatWindowBounds(
                window.restoreBounds,
                action.size,
                window.minimumSize,
              );
        changed ||=
          !boundsMatch(bounds, window.bounds) ||
          !boundsMatch(restoreBounds, window.restoreBounds);
        windows.set(id, { ...window, bounds, restoreBounds });
      }
      return changed ? { ...state, windows } : state;
    }
  }
}
