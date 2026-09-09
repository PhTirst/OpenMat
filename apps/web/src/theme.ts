export const THEME_STORAGE_KEY = "openmat.ide.theme";
export const DEFAULT_THEME = "modern-light" as const;

export type IdeTheme = "modern-light" | "modern-dark";

export function isIdeTheme(value: unknown): value is IdeTheme {
  return value === "modern-light" || value === "modern-dark";
}

export function restoreTheme(
  storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage,
): IdeTheme {
  try {
    const stored = storage?.getItem(THEME_STORAGE_KEY);
    return isIdeTheme(stored) ? stored : DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME;
  }
}

export function persistTheme(
  theme: IdeTheme,
  storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage,
): void {
  try {
    storage?.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // Storage can be blocked in privacy modes. The in-memory selection remains valid.
  }
}

export function monacoTheme(theme: IdeTheme): string {
  return theme === "modern-dark"
    ? "openmat-modern-dark"
    : "openmat-modern-light";
}
