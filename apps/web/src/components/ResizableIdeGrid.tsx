import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
} from "react";
import {
  IDE_DESKTOP_BREAKPOINT,
  IDE_DESKTOP_MEDIA_QUERY,
  IDE_SPLITTER_SIZE,
  getSplitterRange,
  layoutFromPixels,
  moveIdeSplitter,
  persistIdeLayout,
  resolveIdeLayout,
  restoreIdeLayout,
  type IdeLayout,
  type IdeLayoutPixels,
} from "../layout/ide-layout";
import "./resizable-ide-grid.css";

type SplitterName = "left" | "right" | "horizontal";

interface ContainerSize {
  readonly width: number;
  readonly height: number;
}

interface ActiveDrag {
  readonly pointerId: number;
  readonly splitter: SplitterName;
  readonly startCoordinate: number;
  readonly initialLayout: IdeLayoutPixels;
  readonly target: HTMLDivElement;
}

interface ResizableIdeGridProps {
  readonly children: ReactNode;
}

const KEYBOARD_STEP = 12;
const KEYBOARD_LARGE_STEP = 40;

function isDesktopViewport(): boolean {
  if (typeof window === "undefined") {
    return true;
  }
  if (typeof window.matchMedia === "function") {
    return window.matchMedia(IDE_DESKTOP_MEDIA_QUERY).matches;
  }
  return window.innerWidth > IDE_DESKTOP_BREAKPOINT;
}

function initialContainerSize(): ContainerSize {
  if (typeof window === "undefined") {
    return { width: 1200, height: 700 };
  }
  return {
    width: Math.max(window.innerWidth, IDE_DESKTOP_BREAKPOINT + 1),
    height: Math.max(window.innerHeight - 77, 400),
  };
}

function roundedPixels(value: number): number {
  return Math.round(value * 10) / 10;
}

function percent(value: number, total: number): number {
  return Math.round((value / total) * 1000) / 10;
}

function releaseCapture(drag: ActiveDrag): void {
  try {
    if (
      typeof drag.target.releasePointerCapture === "function" &&
      (typeof drag.target.hasPointerCapture !== "function" ||
        drag.target.hasPointerCapture(drag.pointerId))
    ) {
      drag.target.releasePointerCapture(drag.pointerId);
    }
  } catch {
    // Capture may already have been released by the browser.
  }
}

function markDragging(active: boolean, splitter?: SplitterName): void {
  if (typeof document === "undefined") {
    return;
  }
  document.body.classList.toggle("ide-layout-resizing", active);
  if (active && splitter !== undefined) {
    document.body.dataset.ideResizeOrientation =
      splitter === "horizontal" ? "horizontal" : "vertical";
  } else {
    delete document.body.dataset.ideResizeOrientation;
  }
}

export function ResizableIdeGrid({ children }: ResizableIdeGridProps) {
  const gridRef = useRef<HTMLElement>(null);
  const [desktop, setDesktop] = useState(isDesktopViewport);
  const [containerSize, setContainerSize] =
    useState<ContainerSize>(initialContainerSize);
  const [layout, setLayout] = useState<IdeLayout>(restoreIdeLayout);
  const layoutRef = useRef(layout);
  const dragRef = useRef<ActiveDrag | null>(null);
  const pendingLayoutRef = useRef<IdeLayout | null>(null);
  const animationFrameRef = useRef<number | null>(null);

  const updateContainerSize = useCallback((width: number, height: number) => {
    if (width <= 0 || height <= 0) {
      return;
    }
    setContainerSize((current) =>
      current.width === width && current.height === height
        ? current
        : { width, height },
    );
  }, []);

  const measureGrid = useCallback(() => {
    const grid = gridRef.current;
    if (grid === null) {
      return;
    }
    const bounds = grid.getBoundingClientRect();
    updateContainerSize(
      bounds.width || grid.clientWidth,
      bounds.height || grid.clientHeight,
    );
  }, [updateContainerSize]);

  useEffect(() => {
    measureGrid();
    const grid = gridRef.current;
    if (grid === null || typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measureGrid);
      return () => window.removeEventListener("resize", measureGrid);
    }
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry !== undefined) {
        updateContainerSize(entry.contentRect.width, entry.contentRect.height);
      }
    });
    observer.observe(grid);
    return () => observer.disconnect();
  }, [measureGrid, updateContainerSize]);

  useEffect(() => {
    if (typeof window.matchMedia !== "function") {
      const handleResize = () => setDesktop(isDesktopViewport());
      window.addEventListener("resize", handleResize);
      return () => window.removeEventListener("resize", handleResize);
    }
    const media = window.matchMedia(IDE_DESKTOP_MEDIA_QUERY);
    const handleChange = (event: MediaQueryListEvent) => {
      setDesktop(event.matches);
    };
    setDesktop(media.matches);
    media.addEventListener("change", handleChange);
    return () => media.removeEventListener("change", handleChange);
  }, []);

  const flushScheduledLayout = useCallback(() => {
    if (animationFrameRef.current !== null) {
      cancelAnimationFrame(animationFrameRef.current);
      animationFrameRef.current = null;
    }
    const pending = pendingLayoutRef.current;
    pendingLayoutRef.current = null;
    if (pending !== null) {
      setLayout(pending);
    }
  }, []);

  const scheduleLayout = useCallback((next: IdeLayout) => {
    layoutRef.current = next;
    pendingLayoutRef.current = next;
    if (animationFrameRef.current !== null) {
      return;
    }
    animationFrameRef.current = requestAnimationFrame(() => {
      animationFrameRef.current = null;
      const pending = pendingLayoutRef.current;
      pendingLayoutRef.current = null;
      if (pending !== null) {
        setLayout(pending);
      }
    });
  }, []);

  const finishDrag = useCallback(
    (pointerId: number, release: boolean) => {
      const drag = dragRef.current;
      if (drag === null || drag.pointerId !== pointerId) {
        return;
      }
      dragRef.current = null;
      if (release) {
        releaseCapture(drag);
      }
      flushScheduledLayout();
      persistIdeLayout(layoutRef.current);
      markDragging(false);
    },
    [flushScheduledLayout],
  );

  useEffect(() => {
    const drag = dragRef.current;
    if (!desktop && drag !== null) {
      finishDrag(drag.pointerId, true);
    }
  }, [desktop, finishDrag]);

  useEffect(
    () => () => {
      if (animationFrameRef.current !== null) {
        cancelAnimationFrame(animationFrameRef.current);
      }
      const drag = dragRef.current;
      if (drag !== null) {
        releaseCapture(drag);
      }
      markDragging(false);
    },
    [],
  );

  const pixelLayout = resolveIdeLayout(
    layout,
    containerSize.width,
    containerSize.height,
  );

  const beginDrag = useCallback(
    (splitter: SplitterName, event: PointerEvent<HTMLDivElement>) => {
      if (!desktop || event.button !== 0) {
        return;
      }
      event.preventDefault();
      const target = event.currentTarget;
      try {
        target.setPointerCapture(event.pointerId);
      } catch {
        // Pointer capture is unavailable in a few embedded browser shells.
      }
      dragRef.current = {
        pointerId: event.pointerId,
        splitter,
        startCoordinate:
          splitter === "horizontal" ? event.clientY : event.clientX,
        initialLayout: resolveIdeLayout(
          layoutRef.current,
          containerSize.width,
          containerSize.height,
        ),
        target,
      };
      markDragging(true, splitter);
    },
    [containerSize.height, containerSize.width, desktop],
  );

  const continueDrag = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (drag === null || drag.pointerId !== event.pointerId) {
        return;
      }
      event.preventDefault();
      const coordinate =
        drag.splitter === "horizontal" ? event.clientY : event.clientX;
      const nextPixels = moveIdeSplitter(
        drag.initialLayout,
        drag.splitter,
        coordinate - drag.startCoordinate,
      );
      scheduleLayout(layoutFromPixels(nextPixels));
    },
    [scheduleLayout],
  );

  const adjustWithKeyboard = useCallback(
    (splitter: SplitterName, event: KeyboardEvent<HTMLDivElement>) => {
      const current = resolveIdeLayout(
        layoutRef.current,
        containerSize.width,
        containerSize.height,
      );
      const range = getSplitterRange(current, splitter);
      const step = event.shiftKey ? KEYBOARD_LARGE_STEP : KEYBOARD_STEP;
      let target = range.value;

      if (event.key === "Home") {
        target = range.minimum;
      } else if (event.key === "End") {
        target = range.maximum;
      } else if (
        splitter === "horizontal" &&
        (event.key === "ArrowUp" || event.key === "ArrowDown")
      ) {
        target += event.key === "ArrowUp" ? -step : step;
      } else if (
        splitter !== "horizontal" &&
        (event.key === "ArrowLeft" || event.key === "ArrowRight")
      ) {
        target += event.key === "ArrowLeft" ? -step : step;
      } else {
        return;
      }

      event.preventDefault();
      const nextPixels = moveIdeSplitter(
        current,
        splitter,
        target - range.value,
      );
      const next = layoutFromPixels(nextPixels);
      layoutRef.current = next;
      pendingLayoutRef.current = null;
      setLayout(next);
      persistIdeLayout(next);
    },
    [containerSize.height, containerSize.width],
  );

  const separatorValues = (splitter: SplitterName) => {
    const range = getSplitterRange(pixelLayout, splitter);
    const total =
      splitter === "horizontal"
        ? pixelLayout.availableHeight
        : pixelLayout.availableWidth;
    return {
      minimum: percent(range.minimum, total),
      maximum: percent(range.maximum, total),
      value: percent(range.value, total),
    };
  };

  const leftValues = separatorValues("left");
  const rightValues = separatorValues("right");
  const horizontalValues = separatorValues("horizontal");
  const gridStyle = desktop
    ? {
        gridTemplateColumns: [
          `${roundedPixels(pixelLayout.left)}px`,
          `${IDE_SPLITTER_SIZE}px`,
          `${roundedPixels(pixelLayout.center)}px`,
          `${IDE_SPLITTER_SIZE}px`,
          `${roundedPixels(pixelLayout.right)}px`,
        ].join(" "),
        gridTemplateRows: [
          `${roundedPixels(pixelLayout.top)}px`,
          `${IDE_SPLITTER_SIZE}px`,
          `${roundedPixels(pixelLayout.bottom)}px`,
        ].join(" "),
      }
    : undefined;

  return (
    <main
      ref={gridRef}
      className="ide-grid resizable-ide-grid"
      data-layout-mode={desktop ? "resizable" : "responsive"}
      style={gridStyle}
    >
      {children}
      {desktop ? (
        <>
          <div
            className="ide-splitter ide-splitter-left"
            data-splitter="left"
            role="separator"
            tabIndex={0}
            aria-label="Resize Current Folder and Editor"
            aria-orientation="vertical"
            aria-valuemin={leftValues.minimum}
            aria-valuemax={leftValues.maximum}
            aria-valuenow={leftValues.value}
            aria-valuetext={`Current Folder width ${Math.round(pixelLayout.left)} pixels`}
            onKeyDown={(event) => adjustWithKeyboard("left", event)}
            onPointerDown={(event) => beginDrag("left", event)}
            onPointerMove={continueDrag}
            onPointerUp={(event) => finishDrag(event.pointerId, true)}
            onPointerCancel={(event) => finishDrag(event.pointerId, true)}
            onLostPointerCapture={(event) => finishDrag(event.pointerId, false)}
          />
          <div
            className="ide-splitter ide-splitter-right"
            data-splitter="right"
            role="separator"
            tabIndex={0}
            aria-label="Resize Editor and Workspace columns"
            aria-orientation="vertical"
            aria-valuemin={rightValues.minimum}
            aria-valuemax={rightValues.maximum}
            aria-valuenow={rightValues.value}
            aria-valuetext={`Workspace column width ${Math.round(pixelLayout.right)} pixels`}
            onKeyDown={(event) => adjustWithKeyboard("right", event)}
            onPointerDown={(event) => beginDrag("right", event)}
            onPointerMove={continueDrag}
            onPointerUp={(event) => finishDrag(event.pointerId, true)}
            onPointerCancel={(event) => finishDrag(event.pointerId, true)}
            onLostPointerCapture={(event) => finishDrag(event.pointerId, false)}
          />
          <div
            className="ide-splitter ide-splitter-horizontal"
            data-splitter="horizontal"
            role="separator"
            tabIndex={0}
            aria-label="Resize upper and lower IDE panels"
            aria-orientation="horizontal"
            aria-valuemin={horizontalValues.minimum}
            aria-valuemax={horizontalValues.maximum}
            aria-valuenow={horizontalValues.value}
            aria-valuetext={`Upper panel height ${Math.round(pixelLayout.top)} pixels`}
            onKeyDown={(event) => adjustWithKeyboard("horizontal", event)}
            onPointerDown={(event) => beginDrag("horizontal", event)}
            onPointerMove={continueDrag}
            onPointerUp={(event) => finishDrag(event.pointerId, true)}
            onPointerCancel={(event) => finishDrag(event.pointerId, true)}
            onLostPointerCapture={(event) => finishDrag(event.pointerId, false)}
          />
        </>
      ) : null}
    </main>
  );
}
