import { describe, expect, it } from "vitest";
import styles from "./styles.css?raw";
import windowStyles from "./windowing/window-manager.css?raw";

describe("responsive editor layout contract", () => {
  it("allows Monaco's intrinsic width to shrink inside the source pane", () => {
    expect(styles).toMatch(
      /\.editor-pane\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/s,
    );
    expect(styles).toMatch(
      /\.editor-host,\s*\.editor-loading\s*\{[^}]*min-width:\s*0/s,
    );
  });

  it("stacks open editors above the file explorer without narrowing the editor", () => {
    expect(styles).toMatch(
      /\.editor-actions\s*\{[^}]*flex:\s*0\s+0\s+auto/s,
    );
    expect(styles).toMatch(
      /\.open-editors-section\s*\{[^}]*max-height:\s*min\(240px,\s*45%\)[^}]*flex:\s*0\s+1\s+auto/s,
    );
    expect(styles).toMatch(
      /\.open-editors-list\s*\{[^}]*flex-direction:\s*column[^}]*min-height:\s*0[^}]*overflow-y:\s*auto/s,
    );
    expect(styles).toMatch(
      /\.open-editor-label\s*\{[^}]*min-width:\s*0[^}]*text-overflow:\s*ellipsis/s,
    );
    expect(styles).not.toContain(".editor-workbench");
  });

  it("keeps the MATLAB-style three-column desktop above tablet widths", () => {
    expect(styles).toMatch(
      /\.ide-grid\s*\{[^}]*grid-column:\s*1[^}]*grid-template-columns:\s*minmax\(180px,[^;]+minmax\(390px,[^;]+minmax\(240px,/s,
    );
    expect(styles).toContain("@media (max-width: 860px)");
  });

  it("defines complete fixed Modern Light and Modern Dark token contracts", () => {
    expect(styles).toContain(':root[data-openmat-theme="modern-light"]');
    for (const token of [
      "--bg-shell",
      "--bg-pane",
      "--pane-header-bg",
      "--editor-surface",
      "--console-text",
      "--dialog-title-bg",
      "--dialog-body-bg",
      "--grid-cell-bg",
      "--exact-cell-bg",
      "--input-bg",
      "--statusbar-bg",
    ]) {
      expect(styles.match(new RegExp(token, "g"))?.length).toBeGreaterThanOrEqual(2);
    }
    expect(styles).toMatch(
      /\.app-shell\[data-theme="modern-dark"\]\s*\{[^}]*color-scheme:\s*dark/s,
    );
    expect(styles).toMatch(
      /\.app-shell\[data-theme="modern-light"\]\s*\{[^}]*color-scheme:\s*light/s,
    );
  });

  it("uses theme tokens across shell, command, variable editor, and settings surfaces", () => {
    expect(styles).toMatch(/\.command-transcript\s*\{[^}]*var\(--editor-surface\)/s);
    expect(styles).toMatch(/\.variable-editor-body\s*\{[^}]*var\(--dialog-body-bg\)/s);
    expect(styles).toMatch(/\.settings-panel\s*\{[^}]*var\(--bg-pane\)/s);
    expect(styles).toMatch(/\.create-entry-form\s*\{[^}]*var\(--bg-raised\)/s);
    expect(styles).toMatch(/\.icon-button:disabled\s*\{/);
  });

  it("preserves command-window matrix columns without breaking numeric cells", () => {
    expect(styles).toMatch(
      /\.command-line\s*\{[^}]*white-space:\s*pre;[^}]*overflow-wrap:\s*normal;[^}]*word-break:\s*normal;/s,
    );
    expect(styles).toMatch(/\.command-transcript\s*\{[^}]*overflow:\s*auto;/s);
  });

  it("layers the graphics-v1 canvas below non-interactive SVG and DOM overlays", () => {
    expect(styles).toMatch(
      /\.figure-canvas,\s*\.figure-svg-overlay,\s*\.figure-dom-overlay\s*\{[^}]*position:\s*absolute[^}]*inset:\s*0/s,
    );
    expect(styles).toMatch(
      /\.figure-svg-overlay,\s*\.figure-dom-overlay\s*\{[^}]*pointer-events:\s*none/s,
    );
    expect(windowStyles).toMatch(
      /\.openmat-window-layer\s*\{[^}]*overflow:\s*hidden[^}]*pointer-events:\s*none/s,
    );
    expect(windowStyles).toMatch(
      /\.openmat-window-layer\s*\{[^}]*grid-row:\s*3[^}]*grid-column:\s*1/s,
    );
  });
});
