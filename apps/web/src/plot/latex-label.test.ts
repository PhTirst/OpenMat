import { describe, expect, it } from "vitest";
import { renderLatexLabel } from "./latex-label";

function rendered(value: string): { element: HTMLDivElement; text: string } {
  const element = document.createElement("div");
  element.innerHTML = renderLatexLabel(value);
  expect(element.querySelector(".katex-error")).toBeNull();
  // Subscript layout contains invisible struts; keep all visible spaces.
  const text = element.querySelector(".katex-html")?.textContent
    ?.replaceAll("\u00a0", " ").replaceAll("\u200b", "") ?? "";
  return { element, text };
}

describe("LaTeX Figure labels", () => {
  it("keeps a prose title upright with spaces", () => {
    const { element, text } = rendered("2D Function Plot");
    expect(text).toBe("2D Function Plot");
    expect(element.querySelector(".mathnormal")).toBeNull();
  });

  it.each([
    ["$x^2$ and $y_1$", "x2 and y1"],
    ["Value \\(x^2\\) and \\[y_1\\]", "Value x2 and y1"],
    ["Cost \\$5 and $x^2$", "Cost $5 and x2"],
    ["\\textbf{Value $x^2$ today}", "Value x2 today"],
    ["$\\text{net gain} = x$", "net gain=x"],
  ])("typesets math within text: %s", (source, expected) => {
    const { element, text } = rendered(source);
    expect(text).toBe(expected);
    expect(element.querySelector(".mathnormal")).not.toBeNull();
  });

  it.each(["$$x^2$$", "\\[x^2\\]"])("retains standalone display math: %s", (source) => {
    const { element, text } = rendered(source);
    expect(text).toBe("x2");
    expect(element.querySelector(".katex-display")).not.toBeNull();
  });

  it("escapes HTML in text instead of creating elements", () => {
    const { element, text } = rendered("Title <img src=x> plot");
    expect(text).toBe("Title <img src=x> plot");
    expect(element.querySelector("img")).toBeNull();
  });
});
