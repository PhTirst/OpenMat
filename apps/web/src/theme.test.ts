import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_THEME,
  THEME_STORAGE_KEY,
  monacoTheme,
  persistTheme,
  restoreTheme,
} from "./theme";

describe("IDE theme persistence", () => {
  it("uses a fixed light default independent of the host color scheme", () => {
    expect(restoreTheme({ getItem: () => null })).toBe(DEFAULT_THEME);
    expect(DEFAULT_THEME).toBe("modern-light");
    expect(monacoTheme("modern-dark")).toBe("openmat-modern-dark");
    expect(monacoTheme("modern-light")).toBe("openmat-modern-light");
  });

  it("restores only known values and persists selections", () => {
    expect(
      restoreTheme({ getItem: (key) => (key === THEME_STORAGE_KEY ? "modern-light" : null) }),
    ).toBe("modern-light");
    expect(restoreTheme({ getItem: () => "system" })).toBe(DEFAULT_THEME);
    const setItem = vi.fn();
    persistTheme("modern-light", { setItem });
    expect(setItem).toHaveBeenCalledWith(THEME_STORAGE_KEY, "modern-light");
  });

  it("falls back cleanly when storage is unavailable", () => {
    expect(
      restoreTheme({
        getItem: () => {
          throw new DOMException("blocked");
        },
      }),
    ).toBe(DEFAULT_THEME);
    expect(() =>
      persistTheme("modern-dark", {
        setItem: () => {
          throw new DOMException("blocked");
        },
      }),
    ).not.toThrow();
  });
});
