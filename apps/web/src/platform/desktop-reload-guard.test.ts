import { describe, expect, it } from "vitest";
import preventReload from "../../../desktop/src-tauri/src/prevent-reload.js?raw";

function desktopPage(): EventTarget {
  const page = new EventTarget();
  // Exercise the actual Tauri initialization script without loading React.
  new Function("window", preventReload)(page);
  return page;
}

describe("Desktop reload guard", () => {
  it.each<KeyboardEventInit>([
    { key: "F5" },
    { key: "F5", repeat: true },
    { key: "F5", ctrlKey: true },
    { key: "F5", shiftKey: true },
    { key: "r", ctrlKey: true },
    { key: "R", ctrlKey: true, shiftKey: true },
    { key: "r", metaKey: true },
  ])("prevents reload before the workbench has mounted: %j", (key) => {
    const event = new KeyboardEvent("keydown", { ...key, cancelable: true });
    expect(desktopPage().dispatchEvent(event)).toBe(false);
    expect(event.defaultPrevented).toBe(true);
  });

  it.each<KeyboardEventInit>([
    { key: "r" },
    { key: "s", ctrlKey: true },
    { key: "c", ctrlKey: true },
    { key: "v", ctrlKey: true },
    { key: "z", ctrlKey: true },
    { key: "+", ctrlKey: true },
    { key: "Enter", ctrlKey: true },
    { key: "F12" },
    { key: "F5", altKey: true },
  ])("preserves editing and unrelated shortcuts: %j", (key) => {
    const event = new KeyboardEvent("keydown", { ...key, cancelable: true });
    expect(desktopPage().dispatchEvent(event)).toBe(true);
  });
});
