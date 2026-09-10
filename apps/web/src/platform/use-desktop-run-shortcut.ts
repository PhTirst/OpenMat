import { useEffect } from "react";

export function useDesktopRunShortcut(
  desktop: boolean,
  canRun: boolean,
  onRun: () => void,
): void {
  useEffect(() => {
    if (!desktop) return;

    const handleKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== "F5" || event.ctrlKey || event.metaKey ||
        event.altKey || event.shiftKey) return;

      // The native initialization script already cancels browser reload. Do not
      // skip defaultPrevented events: the application still owns F5 execution.
      event.preventDefault();
      event.stopPropagation();
      if (!canRun || event.repeat || event.isComposing) return;
      const modalOpen = [...document.querySelectorAll('[aria-modal="true"]')]
        .some((element) => element.closest("[hidden]") === null);
      if (!modalOpen) onRun();
    };

    // Capture before Monaco and floating windows consume editor key events.
    window.addEventListener("keydown", handleKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", handleKeyDown, { capture: true });
  }, [desktop, canRun, onRun]);
}
