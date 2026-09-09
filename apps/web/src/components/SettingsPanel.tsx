import { useEffect, useId, useRef } from "react";
import type { IdeTheme } from "../theme";

interface SettingsPanelProps {
  readonly theme: IdeTheme;
  readonly onThemeChange: (theme: IdeTheme) => void;
  readonly onClose: () => void;
}

export function SettingsPanel({
  theme,
  onThemeChange,
  onClose,
}: SettingsPanelProps) {
  const titleId = useId();
  const selectRef = useRef<HTMLSelectElement>(null);

  useEffect(() => {
    selectRef.current?.focus();
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  return (
    <aside className="settings-panel" role="dialog" aria-labelledby={titleId}>
      <header>
        <strong id={titleId}>Settings</strong>
        <button
          className="icon-button"
          type="button"
          aria-label="Close Settings"
          title="Close Settings"
          onClick={onClose}
        >
          ×
        </button>
      </header>
      <div className="settings-field">
        <label htmlFor="ide-theme-select">Theme</label>
        <select
          ref={selectRef}
          id="ide-theme-select"
          value={theme}
          onChange={(event) =>
            onThemeChange(event.target.value as IdeTheme)
          }
        >
          <option value="modern-light">Modern Light</option>
          <option value="modern-dark">Modern Dark</option>
        </select>
        <span>Changes the complete IDE and editor color palette.</span>
      </div>
    </aside>
  );
}
