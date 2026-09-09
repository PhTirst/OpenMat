import katex from "katex";

const delimiters = [
  { open: "$$", close: "$$", display: true },
  { open: "\\[", close: "\\]", display: true },
  { open: "\\(", close: "\\)", display: false },
  { open: "$", close: "$", display: false },
] as const;

function mathEnd(value: string, start: number, closing: string): number {
  let depth = 0;
  for (let index = start; index < value.length; index += 1) {
    if (depth === 0 && value.startsWith(closing, index)) return index;
    if (value[index] === "\\") {
      index += 1;
    } else if (value[index] === "{") {
      depth += 1;
    } else if (value[index] === "}") {
      depth = Math.max(0, depth - 1);
    }
  }
  return -1;
}

function labelSource(value: string): { source: string; displayMode: boolean } {
  let normalized = "";
  for (let index = 0; index < value.length;) {
    const delimiter = delimiters.find(({ open }) => value.startsWith(open, index));
    if (delimiter !== undefined) {
      const start = index + delimiter.open.length;
      const end = mathEnd(value, start, delimiter.close);
      if (end >= 0) {
        const source = value.slice(start, end);
        const next = end + delimiter.close.length;
        if (delimiter.display && value.slice(0, index).trim() === "" && value.slice(next).trim() === "") {
          return { source, displayMode: true };
        }
        normalized += `$${delimiter.display ? "\\displaystyle " : ""}${source}$`;
        index = next;
        continue;
      }
      // Leave incomplete markup to KaTeX's normal, escaped error rendering.
      normalized += value.slice(index);
      break;
    }
    // An escaped dollar or backslash must not start a math section.
    const length = value[index] === "\\" ? 2 : 1;
    normalized += value.slice(index, index + length);
    index += length;
  }
  // LaTeX labels start in text mode. Keeping one text group also lets font
  // commands span inline math, e.g. \textbf{Value $x^2$ today}.
  return { source: `\\text{${normalized}}`, displayMode: false };
}

export function renderLatexLabel(value: string): string {
  const { source, displayMode } = labelSource(value);
  return katex.renderToString(source, {
    displayMode,
    output: "htmlAndMathml",
    strict: "ignore",
    throwOnError: false,
    trust: false,
  });
}
