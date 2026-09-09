export const IDE_LAYOUT_STORAGE_KEY = "openmat.ide-layout.v1";
export const IDE_LAYOUT_VERSION = 1;

export const IDE_DESKTOP_BREAKPOINT = 860;
export const IDE_DESKTOP_MEDIA_QUERY = `(min-width: ${IDE_DESKTOP_BREAKPOINT + 1}px)`;

export const IDE_SPLITTER_SIZE = 8;
export const IDE_GRID_PADDING = 2;

const LEFT_MIN = 160;
const LEFT_MAX = 360;
const CENTER_MIN = 360;
const RIGHT_MIN = 220;
const RIGHT_MAX = 480;
const TOP_MIN = 240;
const BOTTOM_MIN = 160;
const EMERGENCY_ROW_MIN = 96;

export interface IdeLayout {
  readonly leftFraction: number;
  readonly rightFraction: number;
  readonly topFraction: number;
}

export interface IdeLayoutPixels {
  readonly left: number;
  readonly center: number;
  readonly right: number;
  readonly top: number;
  readonly bottom: number;
  readonly availableWidth: number;
  readonly availableHeight: number;
}

interface PersistedIdeLayout extends IdeLayout {
  readonly version: typeof IDE_LAYOUT_VERSION;
}

export const DEFAULT_IDE_LAYOUT: IdeLayout = Object.freeze({
  leftFraction: 0.18,
  rightFraction: 0.25,
  topFraction: 0.64,
});

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isValidFraction(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 && value < 1;
}

export function isIdeLayout(value: unknown): value is IdeLayout {
  if (!isRecord(value)) {
    return false;
  }
  const { leftFraction, rightFraction, topFraction } = value;
  return (
    isValidFraction(leftFraction) &&
    isValidFraction(rightFraction) &&
    isValidFraction(topFraction) &&
    leftFraction + rightFraction < 0.9
  );
}

function getDefaultStorage(): Pick<Storage, "getItem" | "setItem"> | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

export function restoreIdeLayout(
  storage: Pick<Storage, "getItem"> | undefined = getDefaultStorage(),
): IdeLayout {
  if (storage === undefined) {
    return DEFAULT_IDE_LAYOUT;
  }
  try {
    const raw = storage.getItem(IDE_LAYOUT_STORAGE_KEY);
    if (raw === null) {
      return DEFAULT_IDE_LAYOUT;
    }
    const parsed: unknown = JSON.parse(raw);
    if (
      !isRecord(parsed) ||
      parsed.version !== IDE_LAYOUT_VERSION ||
      !isIdeLayout(parsed)
    ) {
      return DEFAULT_IDE_LAYOUT;
    }
    return {
      leftFraction: parsed.leftFraction,
      rightFraction: parsed.rightFraction,
      topFraction: parsed.topFraction,
    };
  } catch {
    return DEFAULT_IDE_LAYOUT;
  }
}

export function persistIdeLayout(
  layout: IdeLayout,
  storage: Pick<Storage, "setItem"> | undefined = getDefaultStorage(),
): void {
  if (storage === undefined || !isIdeLayout(layout)) {
    return;
  }
  const persisted: PersistedIdeLayout = {
    version: IDE_LAYOUT_VERSION,
    ...layout,
  };
  try {
    storage.setItem(IDE_LAYOUT_STORAGE_KEY, JSON.stringify(persisted));
  } catch {
    // Storage can be unavailable or full. Layout changes remain usable in-memory.
  }
}

function getAvailableWidth(containerWidth: number): number {
  return Math.max(
    LEFT_MIN + CENTER_MIN + RIGHT_MIN,
    containerWidth - IDE_GRID_PADDING - IDE_SPLITTER_SIZE * 2,
  );
}

function getAvailableHeight(containerHeight: number): number {
  return Math.max(
    EMERGENCY_ROW_MIN * 2,
    containerHeight - IDE_GRID_PADDING - IDE_SPLITTER_SIZE,
  );
}

export function resolveIdeLayout(
  layout: IdeLayout,
  containerWidth: number,
  containerHeight: number,
): IdeLayoutPixels {
  const availableWidth = getAvailableWidth(containerWidth);
  const desiredLeft = availableWidth * layout.leftFraction;
  const desiredRight = availableWidth * layout.rightFraction;
  const leftMaximum = Math.min(
    LEFT_MAX,
    availableWidth - CENTER_MIN - RIGHT_MIN,
  );
  const left = clamp(desiredLeft, LEFT_MIN, leftMaximum);
  const rightMaximum = Math.min(
    RIGHT_MAX,
    availableWidth - left - CENTER_MIN,
  );
  const right = clamp(desiredRight, RIGHT_MIN, rightMaximum);

  const availableHeight = getAvailableHeight(containerHeight);
  const emergencyMinimum = Math.min(
    EMERGENCY_ROW_MIN,
    availableHeight / 2,
  );
  const topMinimum = Math.min(TOP_MIN, availableHeight - emergencyMinimum);
  const bottomMinimum = Math.min(BOTTOM_MIN, availableHeight - topMinimum);
  const top = clamp(
    availableHeight * layout.topFraction,
    topMinimum,
    availableHeight - bottomMinimum,
  );

  return {
    left,
    center: availableWidth - left - right,
    right,
    top,
    bottom: availableHeight - top,
    availableWidth,
    availableHeight,
  };
}

export function layoutFromPixels(layout: IdeLayoutPixels): IdeLayout {
  return {
    leftFraction: layout.left / layout.availableWidth,
    rightFraction: layout.right / layout.availableWidth,
    topFraction: layout.top / layout.availableHeight,
  };
}

export function moveIdeSplitter(
  layout: IdeLayoutPixels,
  splitter: "left" | "right" | "horizontal",
  delta: number,
): IdeLayoutPixels {
  const range = getSplitterRange(layout, splitter);
  const value = clamp(range.value + delta, range.minimum, range.maximum);
  if (splitter === "left") {
    return {
      ...layout,
      left: value,
      center: layout.availableWidth - value - layout.right,
    };
  }
  if (splitter === "right") {
    return {
      ...layout,
      center: value - layout.left,
      right: layout.availableWidth - value,
    };
  }
  return {
    ...layout,
    top: value,
    bottom: layout.availableHeight - value,
  };
}

export function getSplitterRange(
  layout: IdeLayoutPixels,
  splitter: "left" | "right" | "horizontal",
): { readonly minimum: number; readonly maximum: number; readonly value: number } {
  if (splitter === "left") {
    return {
      minimum: LEFT_MIN,
      maximum: Math.min(
        LEFT_MAX,
        layout.availableWidth - layout.right - CENTER_MIN,
      ),
      value: layout.left,
    };
  }
  if (splitter === "right") {
    return {
      minimum: Math.max(
        layout.left + CENTER_MIN,
        layout.availableWidth - RIGHT_MAX,
      ),
      maximum: layout.availableWidth - RIGHT_MIN,
      value: layout.left + layout.center,
    };
  }
  const emergencyMinimum = Math.min(
    EMERGENCY_ROW_MIN,
    layout.availableHeight / 2,
  );
  const minimum = Math.min(
    TOP_MIN,
    layout.availableHeight - emergencyMinimum,
  );
  const bottomMinimum = Math.min(
    BOTTOM_MIN,
    layout.availableHeight - minimum,
  );
  return {
    minimum,
    maximum: layout.availableHeight - bottomMinimum,
    value: layout.top,
  };
}
