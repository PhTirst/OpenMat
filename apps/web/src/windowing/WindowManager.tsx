import {
  Component,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
  type ErrorInfo,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import * as ReactRuntime from "react";

import {
  type OpenMatAppRegistry,
  type OpenMatDynamicAppHost,
  type OpenMatDynamicAppModule,
  type OpenMatDynamicWindowRequest,
  type OpenMatWindowHandle,
  activateOpenMatAppModule,
  loadOpenMatAppModule,
} from "./app-registry";
import {
  constrainOpenMatWindowBounds,
  initialOpenMatWindowManagerState,
  openMatWindowManagerReducer,
  type OpenMatWindowBounds,
  type OpenMatWindowInstance,
  type OpenMatWindowManagerState,
  type OpenMatWindowSize,
} from "./window-state";
import "./window-manager.css";

const FALLBACK_DESKTOP_SIZE: OpenMatWindowSize = { width: 1280, height: 720 };
const CASCADE_START = 28;
const CASCADE_STEP = 28;
const CASCADE_COUNT = 10;

export interface OpenMatWindowManagerActions {
  openWindow<Payload>(request: OpenMatDynamicWindowRequest<Payload>): string;
  updateWindow(
    id: string,
    patch: Partial<
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
    >,
  ): void;
  focusWindow(id: string): void;
  closeWindow(id: string): void;
  requestCloseWindow(id: string): void;
  setWindowBounds(id: string, bounds: OpenMatWindowBounds): void;
  minimizeWindow(id: string): void;
  maximizeWindow(id: string): void;
  restoreWindow(id: string): void;
  setWindowTitle(id: string, title: string): void;
  getDesktopSize(): OpenMatWindowSize;
  resizeDesktop(size: OpenMatWindowSize): void;
  openDialog<Result>(request: OpenMatDialogRequest<Result>): Promise<Result>;
}

export interface OpenMatDialogControls<Result> {
  close(result: Result): void;
}

export interface OpenMatDialogRequest<Result> {
  readonly label: string;
  readonly dismissResult: Result;
  readonly render: (controls: OpenMatDialogControls<Result>) => ReactNode;
  readonly closeOnEscape?: boolean;
  readonly closeOnBackdrop?: boolean;
}

interface WindowManagerProviderProps {
  readonly registry: OpenMatAppRegistry;
  readonly children: ReactNode;
}

const WindowStateContext = createContext<OpenMatWindowManagerState | null>(null);
const WindowActionsContext = createContext<OpenMatWindowManagerActions | null>(null);
const WindowRegistryContext = createContext<OpenMatAppRegistry | null>(null);
const DesktopElementContext = createContext<
  ((element: HTMLElement | null) => void) | null
>(null);

interface ErasedOpenMatDialog {
  readonly id: string;
  readonly label: string;
  readonly dismissResult: unknown;
  readonly render: (close: (result: unknown) => void) => ReactNode;
  readonly closeOnEscape: boolean;
  readonly closeOnBackdrop: boolean;
  readonly resolve: (result: unknown) => void;
  readonly returnFocus: HTMLElement | null;
}

interface OpenMatDialogLayerState {
  readonly dialogs: readonly ErasedOpenMatDialog[];
  closeDialog(id: string, result: unknown): void;
}

const DialogLayerContext = createContext<OpenMatDialogLayerState | null>(null);

function requireContext<T>(value: T | null, name: string): T {
  if (value === null) {
    throw new Error(`${name} must be used inside OpenMatWindowManagerProvider.`);
  }
  return value;
}

export function useOpenMatWindowManager(): OpenMatWindowManagerState {
  return requireContext(useContext(WindowStateContext), "useOpenMatWindowManager");
}

export function useOpenMatWindowManagerActions(): OpenMatWindowManagerActions {
  return requireContext(
    useContext(WindowActionsContext),
    "useOpenMatWindowManagerActions",
  );
}

export function useOpenMatAppRegistry(): OpenMatAppRegistry {
  return requireContext(useContext(WindowRegistryContext), "useOpenMatAppRegistry");
}

function desktopSize(element: HTMLElement | null): OpenMatWindowSize {
  if (element !== null) {
    const bounds = element.getBoundingClientRect();
    if (bounds.width > 0 && bounds.height > 0) {
      return { width: bounds.width, height: bounds.height };
    }
  }
  if (typeof window !== "undefined") {
    return {
      width: Math.max(window.innerWidth, 1),
      height: Math.max(window.innerHeight - 77, 1),
    };
  }
  return FALLBACK_DESKTOP_SIZE;
}

export function OpenMatWindowManagerProvider({
  registry,
  children,
}: WindowManagerProviderProps) {
  const [state, dispatch] = useReducer(
    openMatWindowManagerReducer,
    initialOpenMatWindowManagerState,
  );
  const stateRef = useRef(state);
  const desktopRef = useRef<HTMLElement | null>(null);
  const sequenceRef = useRef(0);
  const dialogSequenceRef = useRef(0);
  const dialogQueueRef = useRef<readonly ErasedOpenMatDialog[]>([]);
  const closeRequestTokensRef = useRef(new Map<string, symbol>());
  const [dialogQueue, setDialogQueue] = useState<
    readonly ErasedOpenMatDialog[]
  >([]);
  stateRef.current = state;

  const getDesktopSize = useCallback(
    () => desktopSize(desktopRef.current),
    [],
  );

  const registerDesktopElement = useCallback((element: HTMLElement | null) => {
    desktopRef.current = element;
  }, []);

  const openWindow = useCallback(
    <Payload,>(request: OpenMatDynamicWindowRequest<Payload>): string => {
      const definition = registry.get(request.appId);
      if (definition === undefined) {
        throw new Error(`OpenMat app is not registered: ${request.appId}`);
      }
      sequenceRef.current += 1;
      const sequence = sequenceRef.current;
      const id =
        request.id ??
        `${request.appId}:${Date.now().toString(36)}:${sequence.toString(36)}`;
      if (id.length === 0 || /[\u0000-\u0020\u007f]/u.test(id)) {
        throw new Error(`Invalid OpenMat window id: ${id}`);
      }
      const existing = stateRef.current.windows.get(id);
      const size = getDesktopSize();
      const cascadeIndex = (sequence - 1) % CASCADE_COUNT;
      const proposedBounds =
        existing?.bounds ??
        request.bounds ?? {
          x: CASCADE_START + cascadeIndex * CASCADE_STEP,
          y: CASCADE_START + cascadeIndex * CASCADE_STEP,
          width: definition.defaultSize.width,
          height: definition.defaultSize.height,
        };
      const bounds = constrainOpenMatWindowBounds(
        proposedBounds,
        size,
        definition.minimumSize,
      );
      const instance: OpenMatWindowInstance = {
        id,
        appId: request.appId,
        title: request.title ?? definition.displayName,
        ...(request.ariaDescription === undefined
          ? {}
          : { ariaDescription: request.ariaDescription }),
        closeButtonLabel:
          request.closeButtonLabel ?? `Close ${request.title ?? definition.displayName}`,
        payload: request.payload,
        bounds,
        minimumSize: definition.minimumSize,
        restoreBounds:
          existing?.restoreBounds === null || existing?.restoreBounds === undefined
            ? null
            : constrainOpenMatWindowBounds(
                existing.restoreBounds,
                size,
                definition.minimumSize,
              ),
        mode: existing?.mode ?? "normal",
        minimizedFrom: existing?.minimizedFrom ?? null,
        closeState: existing?.closeState ?? "open",
        closeOnEscape: request.closeOnEscape ?? false,
        initialFocus: request.initialFocus ?? "window",
        ...(request.onCloseRequested === undefined
          ? {}
          : { onCloseRequested: request.onCloseRequested }),
      };
      dispatch({
        type: "opened",
        window: instance,
        focusExisting: request.focusExisting ?? false,
      });
      return id;
    },
    [getDesktopSize, registry],
  );

  const updateWindow = useCallback<OpenMatWindowManagerActions["updateWindow"]>(
    (id, patch) => dispatch({ type: "updated", id, patch }),
    [],
  );
  const focusWindow = useCallback((id: string) => {
    dispatch({ type: "focused", id });
  }, []);
  const closeWindow = useCallback((id: string) => {
    closeRequestTokensRef.current.delete(id);
    dispatch({ type: "closed", id });
  }, []);
  const setWindowBounds = useCallback(
    (id: string, bounds: OpenMatWindowBounds) => {
      const instance = stateRef.current.windows.get(id);
      if (instance === undefined) {
        return;
      }
      dispatch({
        type: "boundsChanged",
        id,
        bounds: constrainOpenMatWindowBounds(
          bounds,
          getDesktopSize(),
          instance.minimumSize,
        ),
      });
    },
    [getDesktopSize],
  );
  const minimizeWindow = useCallback((id: string) => {
    dispatch({ type: "minimized", id });
  }, []);
  const maximizeWindow = useCallback((id: string) => {
    dispatch({ type: "maximized", id });
  }, []);
  const restoreWindow = useCallback((id: string) => {
    dispatch({ type: "restored", id });
  }, []);
  const setWindowTitle = useCallback((id: string, title: string) => {
    dispatch({ type: "updated", id, patch: { title } });
  }, []);
  const resizeDesktop = useCallback((size: OpenMatWindowSize) => {
    dispatch({ type: "desktopResized", size });
  }, []);

  const closeDialog = useCallback((id: string, result: unknown): void => {
    const active = dialogQueueRef.current[0];
    if (active?.id !== id) {
      return;
    }
    const remaining = dialogQueueRef.current.slice(1);
    dialogQueueRef.current = remaining;
    setDialogQueue(remaining);
    active.resolve(result);
    window.setTimeout(() => {
      if (
        dialogQueueRef.current.length === 0 &&
        active.returnFocus?.isConnected === true
      ) {
        active.returnFocus.focus({ preventScroll: true });
      }
    }, 0);
  }, []);

  const openDialog = useCallback(
    <Result,>(request: OpenMatDialogRequest<Result>): Promise<Result> =>
      new Promise<Result>((resolve) => {
        dialogSequenceRef.current += 1;
        const id = `dialog:${dialogSequenceRef.current.toString(36)}`;
        const dialog: ErasedOpenMatDialog = {
          id,
          label: request.label,
          dismissResult: request.dismissResult,
          closeOnEscape: request.closeOnEscape ?? true,
          closeOnBackdrop: request.closeOnBackdrop ?? false,
          render: (close) =>
            request.render({ close: (result) => close(result) }),
          resolve: (result) => resolve(result as Result),
          returnFocus:
            document.activeElement instanceof HTMLElement
              ? document.activeElement
              : null,
        };
        const next = [...dialogQueueRef.current, dialog];
        dialogQueueRef.current = next;
        setDialogQueue(next);
      }),
    [],
  );

  const requestCloseWindow = useCallback((id: string) => {
    const instance = stateRef.current.windows.get(id);
    if (
      instance === undefined ||
      instance.closeState === "closing" ||
      closeRequestTokensRef.current.has(id)
    ) {
      return;
    }
    const closeHandler = instance.onCloseRequested;
    if (closeHandler === undefined) {
      dispatch({ type: "closed", id });
      return;
    }
    const token = Symbol(id);
    closeRequestTokensRef.current.set(id, token);
    dispatch({ type: "closeStateChanged", id, closing: true });
    const resolveDecision = async (): Promise<void> => {
      try {
        const decision = await closeHandler();
        if (closeRequestTokensRef.current.get(id) !== token) {
          return;
        }
        if (decision === "defer") {
          return;
        }
        closeRequestTokensRef.current.delete(id);
        if (decision === "cancel") {
          dispatch({ type: "closeStateChanged", id, closing: false });
          return;
        }
        dispatch({ type: "closed", id });
      } catch (error: unknown) {
        if (closeRequestTokensRef.current.get(id) !== token) {
          return;
        }
        closeRequestTokensRef.current.delete(id);
        dispatch({ type: "closeStateChanged", id, closing: false });
        console.error(`OpenMat window ${id} close handler failed.`, error);
      }
    };
    void resolveDecision();
  }, []);

  const actions = useMemo<OpenMatWindowManagerActions>(
    () => ({
      openWindow,
      updateWindow,
      focusWindow,
      closeWindow,
      requestCloseWindow,
      setWindowBounds,
      minimizeWindow,
      maximizeWindow,
      restoreWindow,
      setWindowTitle,
      getDesktopSize,
      resizeDesktop,
      openDialog,
    }),
    [
      closeWindow,
      focusWindow,
      getDesktopSize,
      maximizeWindow,
      minimizeWindow,
      openWindow,
      requestCloseWindow,
      restoreWindow,
      setWindowBounds,
      setWindowTitle,
      updateWindow,
      resizeDesktop,
      openDialog,
    ],
  );

  useEffect(
    () => () => {
      const pending = dialogQueueRef.current;
      dialogQueueRef.current = [];
      for (const dialog of pending) {
        dialog.resolve(dialog.dismissResult);
      }
    },
    [],
  );

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.key !== "Escape") {
        return;
      }
      const active = stateRef.current.activeWindowId;
      if (active !== null && stateRef.current.windows.get(active)?.closeOnEscape) {
        event.preventDefault();
        requestCloseWindow(active);
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [requestCloseWindow]);

  return (
    <WindowRegistryContext.Provider value={registry}>
      <WindowActionsContext.Provider value={actions}>
        <WindowStateContext.Provider value={state}>
          <DialogLayerContext.Provider
            value={{ dialogs: dialogQueue, closeDialog }}
          >
            <DesktopElementContext.Provider value={registerDesktopElement}>
              {children}
              <OpenMatDialogLayer />
            </DesktopElementContext.Provider>
          </DialogLayerContext.Provider>
        </WindowStateContext.Provider>
      </WindowActionsContext.Provider>
    </WindowRegistryContext.Provider>
  );
}

const FOCUSABLE_SELECTOR = [
  "button:not(:disabled)",
  "[href]",
  "input:not(:disabled)",
  "select:not(:disabled)",
  "textarea:not(:disabled)",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

function focusableDialogElements(container: HTMLElement): readonly HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)].filter(
    (element) => element.getAttribute("aria-hidden") !== "true",
  );
}

export function OpenMatDialogLayer() {
  const { dialogs, closeDialog } = requireContext(
    useContext(DialogLayerContext),
    "OpenMatDialogLayer",
  );
  const layerRef = useRef<HTMLDivElement>(null);
  const active = dialogs[0];

  useEffect(() => {
    const layer = layerRef.current;
    if (layer === null || active === undefined) {
      return;
    }
    const dialog = layer.querySelector<HTMLElement>('[role="dialog"]');
    if (dialog === null) {
      return;
    }
    const focusInitial = (): void => {
      const initial =
        dialog.querySelector<HTMLElement>("[data-dialog-initial-focus]") ??
        focusableDialogElements(dialog)[0] ??
        dialog;
      initial.focus({ preventScroll: true });
    };
    focusInitial();
    const retainFocus = (event: FocusEvent): void => {
      if (event.target instanceof Node && !dialog.contains(event.target)) {
        focusInitial();
      }
    };
    document.addEventListener("focusin", retainFocus);
    return () => document.removeEventListener("focusin", retainFocus);
  }, [active]);

  if (active === undefined) {
    return null;
  }

  const dismiss = (): void => closeDialog(active.id, active.dismissResult);

  return (
    <div
      ref={layerRef}
      className="openmat-dialog-layer"
      data-dialog-id={active.id}
      data-dialog-queue-length={dialogs.length}
      onMouseDown={(event) => {
        if (event.currentTarget === event.target && active.closeOnBackdrop) {
          dismiss();
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && active.closeOnEscape) {
          event.preventDefault();
          event.stopPropagation();
          dismiss();
          return;
        }
        if (event.key !== "Tab") {
          return;
        }
        const dialog = event.currentTarget.querySelector<HTMLElement>(
          '[role="dialog"]',
        );
        if (dialog === null) {
          return;
        }
        const focusable = focusableDialogElements(dialog);
        if (focusable.length === 0) {
          event.preventDefault();
          dialog.focus({ preventScroll: true });
          return;
        }
        const first = focusable[0];
        const last = focusable.at(-1);
        if (
          (!event.shiftKey && document.activeElement === last) ||
          (event.shiftKey && document.activeElement === first)
        ) {
          event.preventDefault();
          (event.shiftKey ? last : first)?.focus({ preventScroll: true });
        }
      }}
    >
      {active.render((result) => closeDialog(active.id, result))}
      <span className="sr-only" aria-live="polite">
        {dialogs.length > 1
          ? `${(dialogs.length - 1).toString()} more dialog queued after ${active.label}.`
          : ""}
      </span>
    </div>
  );
}

type InteractionKind =
  | "move"
  | "north"
  | "south"
  | "east"
  | "west"
  | "north-east"
  | "north-west"
  | "south-east"
  | "south-west";

interface WindowInteraction {
  readonly pointerId: number;
  readonly kind: InteractionKind;
  readonly startX: number;
  readonly startY: number;
  readonly startBounds: OpenMatWindowBounds;
  latestBounds: OpenMatWindowBounds;
}

function interactionBounds(
  interaction: WindowInteraction,
  clientX: number,
  clientY: number,
  desktop: OpenMatWindowSize,
  minimumSize: OpenMatWindowSize,
): OpenMatWindowBounds {
  const dx = clientX - interaction.startX;
  const dy = clientY - interaction.startY;
  const start = interaction.startBounds;
  if (interaction.kind === "move") {
    return constrainOpenMatWindowBounds(
      { ...start, x: start.x + dx, y: start.y + dy },
      desktop,
      minimumSize,
    );
  }
  const west = interaction.kind.includes("west");
  const east = interaction.kind.includes("east");
  const north = interaction.kind.includes("north");
  const south = interaction.kind.includes("south");
  const next = {
    x: west ? start.x + dx : start.x,
    y: north ? start.y + dy : start.y,
    width: start.width + (east ? dx : 0) - (west ? dx : 0),
    height: start.height + (south ? dy : 0) - (north ? dy : 0),
  };
  const constrained = constrainOpenMatWindowBounds(next, desktop, minimumSize);
  if (west && constrained.width === minimumSize.width) {
    return { ...constrained, x: start.x + start.width - constrained.width };
  }
  if (north && constrained.height === minimumSize.height) {
    return { ...constrained, y: start.y + start.height - constrained.height };
  }
  return constrained;
}

interface ManagedWindowProps {
  readonly instance: OpenMatWindowInstance;
  readonly active: boolean;
  readonly zIndex: number;
  readonly registry: OpenMatAppRegistry;
}

interface WindowAppErrorBoundaryProps {
  readonly appId: string;
  readonly children: ReactNode;
}

interface WindowAppErrorBoundaryState {
  readonly error: Error | null;
}

class WindowAppErrorBoundary extends Component<
  WindowAppErrorBoundaryProps,
  WindowAppErrorBoundaryState
> {
  state: WindowAppErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): WindowAppErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error(`OpenMat app ${this.props.appId} failed to render.`, error, info);
  }

  componentDidUpdate(previousProps: WindowAppErrorBoundaryProps): void {
    if (
      previousProps.appId !== this.props.appId &&
      this.state.error !== null
    ) {
      this.setState({ error: null });
    }
  }

  render(): ReactNode {
    return this.state.error === null ? (
      this.props.children
    ) : (
      <div className="openmat-window-app-error" role="alert">
        <strong>This application could not be rendered.</strong>
        <span>{this.state.error.message}</span>
      </div>
    );
  }
}

function ManagedWindow({ instance, active, zIndex, registry }: ManagedWindowProps) {
  const actions = useOpenMatWindowManagerActions();
  const elementRef = useRef<HTMLElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const interactionRef = useRef<WindowInteraction | null>(null);
  const initiallyFocusedRef = useRef(false);
  const definition = registry.get(instance.appId);
  const AppComponent = definition?.component;
  const maximized = instance.mode === "maximized";

  useEffect(
    () => () => {
      if (interactionRef.current !== null) {
        document.body.classList.remove("openmat-window-interacting");
        delete document.body.dataset.openmatWindowInteraction;
      }
    },
    [],
  );

  useEffect(() => {
    if (!active || initiallyFocusedRef.current) {
      return;
    }
    initiallyFocusedRef.current = true;
    if (instance.initialFocus === "close") {
      closeButtonRef.current?.focus();
    } else {
      elementRef.current?.focus({ preventScroll: true });
    }
  }, [active, instance.initialFocus]);

  const applyTransientBounds = (bounds: OpenMatWindowBounds): void => {
    const element = elementRef.current;
    if (element === null) {
      return;
    }
    element.style.left = `${bounds.x.toString()}px`;
    element.style.top = `${bounds.y.toString()}px`;
    element.style.width = `${bounds.width.toString()}px`;
    element.style.height = `${bounds.height.toString()}px`;
  };

  const beginInteraction = (
    kind: InteractionKind,
    event: ReactPointerEvent<HTMLElement>,
  ): void => {
    if (maximized || event.button !== 0) {
      return;
    }
    if (
      kind === "move" &&
      event.target instanceof Element &&
      event.target.closest("button") !== null
    ) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    if (kind === "move") {
      initiallyFocusedRef.current = true;
      event.currentTarget.focus({ preventScroll: true });
    }
    actions.focusWindow(instance.id);
    interactionRef.current = {
      pointerId: event.pointerId,
      kind,
      startX: event.clientX,
      startY: event.clientY,
      startBounds: instance.bounds,
      latestBounds: instance.bounds,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
    document.body.classList.add("openmat-window-interacting");
    document.body.dataset.openmatWindowInteraction = kind;
  };

  const continueInteraction = (event: ReactPointerEvent<HTMLElement>): void => {
    const interaction = interactionRef.current;
    if (interaction === null || interaction.pointerId !== event.pointerId) {
      return;
    }
    const bounds = interactionBounds(
      interaction,
      event.clientX,
      event.clientY,
      actions.getDesktopSize(),
      instance.minimumSize,
    );
    interaction.latestBounds = bounds;
    applyTransientBounds(bounds);
  };

  const finishInteraction = (
    pointerId: number,
    commit: boolean,
  ): void => {
    const interaction = interactionRef.current;
    if (interaction === null || interaction.pointerId !== pointerId) {
      return;
    }
    interactionRef.current = null;
    document.body.classList.remove("openmat-window-interacting");
    delete document.body.dataset.openmatWindowInteraction;
    if (commit) {
      actions.setWindowBounds(instance.id, interaction.latestBounds);
    } else {
      applyTransientBounds(instance.bounds);
    }
  };

  const keyboardTransform = (event: ReactKeyboardEvent<HTMLElement>): void => {
    if (
      !event.altKey ||
      maximized ||
      (event.target instanceof Element && event.target.closest("button") !== null)
    ) {
      return;
    }
    const amount = event.shiftKey ? 24 : 8;
    let dx = 0;
    let dy = 0;
    if (event.key === "ArrowLeft") dx = -amount;
    if (event.key === "ArrowRight") dx = amount;
    if (event.key === "ArrowUp") dy = -amount;
    if (event.key === "ArrowDown") dy = amount;
    if (dx === 0 && dy === 0) {
      return;
    }
    event.preventDefault();
    actions.setWindowBounds(
      instance.id,
      event.ctrlKey
        ? {
            ...instance.bounds,
            width: instance.bounds.width + dx,
            height: instance.bounds.height + dy,
          }
        : {
            ...instance.bounds,
            x: instance.bounds.x + dx,
            y: instance.bounds.y + dy,
          },
    );
  };

  const handle: OpenMatWindowHandle = useMemo(
    () => ({
      id: instance.id,
      active,
      close: () => actions.requestCloseWindow(instance.id),
      focus: () => actions.focusWindow(instance.id),
      minimize: () => actions.minimizeWindow(instance.id),
      maximize: () => actions.maximizeWindow(instance.id),
      restore: () => actions.restoreWindow(instance.id),
      setTitle: (title) => actions.setWindowTitle(instance.id, title),
    }),
    [actions, active, instance.id],
  );

  const style = maximized
    ? { zIndex }
    : {
        zIndex,
        left: instance.bounds.x,
        top: instance.bounds.y,
        width: instance.bounds.width,
        height: instance.bounds.height,
      };

  return (
    <section
      ref={elementRef}
      className={`openmat-window${active ? " active" : ""}${maximized ? " maximized" : ""}`}
      style={style}
      role="dialog"
      aria-labelledby={`${instance.id}-title`}
      aria-describedby={
        instance.ariaDescription === undefined
          ? undefined
          : `${instance.id}-description`
      }
      data-window-id={instance.id}
      data-app-id={instance.appId}
      data-window-mode={instance.mode}
      tabIndex={-1}
      onPointerDown={() => actions.focusWindow(instance.id)}
    >
      <header
        className="openmat-window-titlebar"
        aria-label={`${instance.title} window. Alt plus arrow keys move; Control plus Alt plus arrow keys resize.`}
        tabIndex={0}
        onDoubleClick={() =>
          maximized
            ? actions.restoreWindow(instance.id)
            : actions.maximizeWindow(instance.id)
        }
        onPointerDown={(event) => beginInteraction("move", event)}
        onPointerMove={continueInteraction}
        onPointerUp={(event) => finishInteraction(event.pointerId, true)}
        onPointerCancel={(event) => finishInteraction(event.pointerId, false)}
        onLostPointerCapture={(event) => finishInteraction(event.pointerId, true)}
        onKeyDown={keyboardTransform}
      >
        <strong id={`${instance.id}-title`}>{instance.title}</strong>
        {instance.ariaDescription === undefined ? null : (
          <span className="sr-only" id={`${instance.id}-description`}>
            {instance.ariaDescription}
          </span>
        )}
        <div className="openmat-window-controls">
          <button
            className="openmat-window-control minimize"
            type="button"
            aria-label={`Minimize ${instance.title}`}
            title="Minimize"
            onClick={() => actions.minimizeWindow(instance.id)}
          >
            <span aria-hidden="true" />
          </button>
          <button
            className={`openmat-window-control ${maximized ? "restore" : "maximize"}`}
            type="button"
            aria-label={`${maximized ? "Restore" : "Maximize"} ${instance.title}`}
            title={maximized ? "Restore" : "Maximize"}
            onClick={() =>
              maximized
                ? actions.restoreWindow(instance.id)
                : actions.maximizeWindow(instance.id)
            }
          >
            <span aria-hidden="true" />
          </button>
          <button
            ref={closeButtonRef}
            className="openmat-window-control close"
            type="button"
            aria-label={instance.closeButtonLabel}
            title="Close"
            disabled={instance.closeState === "closing"}
            onClick={() => actions.requestCloseWindow(instance.id)}
          >
            <span aria-hidden="true" />
          </button>
        </div>
      </header>
      <div className="openmat-window-content">
        {AppComponent === undefined ? (
          <div className="openmat-window-app-error" role="alert">
            <strong>Application unavailable</strong>
            <span>{instance.appId} is not registered.</span>
          </div>
        ) : (
          <WindowAppErrorBoundary appId={instance.appId}>
            <AppComponent payload={instance.payload} window={handle} />
          </WindowAppErrorBoundary>
        )}
      </div>
      {maximized
        ? null
        : ([
            "north",
            "south",
            "east",
            "west",
            "north-east",
            "north-west",
            "south-east",
            "south-west",
          ] as const).map((kind) => (
            <div
              className={`openmat-window-resize-handle ${kind}`}
              data-resize-handle={kind}
              key={kind}
              aria-hidden="true"
              onPointerDown={(event) => beginInteraction(kind, event)}
              onPointerMove={continueInteraction}
              onPointerUp={(event) => finishInteraction(event.pointerId, true)}
              onPointerCancel={(event) => finishInteraction(event.pointerId, false)}
              onLostPointerCapture={(event) =>
                finishInteraction(event.pointerId, true)
              }
            />
          ))}
    </section>
  );
}

export function OpenMatWindowLayer() {
  const state = useOpenMatWindowManager();
  const actions = useOpenMatWindowManagerActions();
  const registry = useOpenMatAppRegistry();
  const registerDesktopElement = requireContext(
    useContext(DesktopElementContext),
    "OpenMatWindowLayer",
  );
  const layerRef = useRef<HTMLDivElement | null>(null);
  useSyncExternalStore(registry.subscribe, registry.getSnapshot, registry.getSnapshot);

  const setLayerRef = useCallback(
    (element: HTMLDivElement | null) => {
      layerRef.current = element;
      registerDesktopElement(element);
    },
    [registerDesktopElement],
  );

  useEffect(() => {
    const layer = layerRef.current;
    if (layer === null) {
      return;
    }
    const reportSize = (): void => {
      const bounds = layer.getBoundingClientRect();
      if (bounds.width > 0 && bounds.height > 0) {
        actions.resizeDesktop({ width: bounds.width, height: bounds.height });
      }
    };
    reportSize();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", reportSize);
      return () => window.removeEventListener("resize", reportSize);
    }
    const observer = new ResizeObserver(reportSize);
    observer.observe(layer);
    return () => observer.disconnect();
  }, [actions]);

  const minimized = state.stackingOrder
    .map((id) => state.windows.get(id))
    .filter(
      (instance): instance is OpenMatWindowInstance =>
        instance?.mode === "minimized",
    );

  return (
    <div
      ref={setLayerRef}
      className="openmat-window-layer"
      aria-label="OpenMat application windows"
    >
      {state.stackingOrder.map((id, index) => {
        const instance = state.windows.get(id);
        return instance === undefined || instance.mode === "minimized" ? null : (
          <ManagedWindow
            key={id}
            instance={instance}
            active={state.activeWindowId === id}
            zIndex={100 + index}
            registry={registry}
          />
        );
      })}
      {minimized.length === 0 ? null : (
        <nav className="openmat-window-shelf" aria-label="Minimized windows">
          {minimized.map((instance) => (
            <button
              key={instance.id}
              type="button"
              onClick={() => actions.restoreWindow(instance.id)}
            >
              {instance.title}
            </button>
          ))}
        </nav>
      )}
    </div>
  );
}

export type OpenMatDynamicAppSource = string | OpenMatDynamicAppModule;

interface OpenMatDynamicAppLoaderProps {
  readonly sources: readonly OpenMatDynamicAppSource[];
  readonly onError?: (message: string) => void;
}

export function OpenMatDynamicAppLoader({
  sources,
  onError,
}: OpenMatDynamicAppLoaderProps) {
  const registry = useOpenMatAppRegistry();
  const actions = useOpenMatWindowManagerActions();

  const host = useMemo<OpenMatDynamicAppHost>(
    () => ({
      apiVersion: "openmat-app-v1",
      react: ReactRuntime,
      apps: {
        register: (definition) => registry.register(definition),
        list: () =>
          registry.list().map(({ id, displayName }) => ({ id, displayName })),
      },
      windows: {
        open: (request) => actions.openWindow(request),
        close: actions.closeWindow,
        focus: actions.focusWindow,
      },
    }),
    [actions, registry],
  );

  useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void> = [];
    const cleanupSafely = (cleanup: () => void): void => {
      try {
        cleanup();
      } catch (error: unknown) {
        onError?.(
          error instanceof Error
            ? error.message
            : "A dynamic OpenMat application failed to unload.",
        );
      }
    };
    const load = async (): Promise<void> => {
      const results = await Promise.allSettled(
        sources.map((source) =>
          typeof source === "string"
            ? loadOpenMatAppModule(source, host)
            : activateOpenMatAppModule(source, host),
        ),
      );
      for (const result of results) {
        if (result.status === "fulfilled") {
          if (disposed) {
            cleanupSafely(result.value);
          } else {
            cleanups.push(result.value);
          }
        } else if (!disposed) {
          onError?.(
            result.reason instanceof Error
              ? result.reason.message
              : "A dynamic OpenMat application failed to load.",
          );
        }
      }
    };
    void load();
    return () => {
      disposed = true;
      cleanups.reverse().forEach(cleanupSafely);
    };
  }, [host, onError, sources]);

  return null;
}
