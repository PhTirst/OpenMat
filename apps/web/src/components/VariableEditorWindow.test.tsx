import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { MatrixRange, VariableSummary } from "../protocol/kernel-v0";
import type { V1MatrixPreview } from "../protocol/kernel-v1";
import type {
  AggregatePreview,
  TableAggregatePreview,
} from "../protocol/kernel-v2";
import styles from "../styles.css?raw";
import { renderWithWindowManager } from "../test/render-with-window-manager";
import type { VariableEditorPreview } from "./VariableEditorWindow";
import {
  VariableEditorWindow,
  createVariableEditorRange,
  parseVariableEditorNumericInput,
  type VariableEditorCellEdit,
  type VariableEditorCommitResult,
} from "./VariableEditorWindow";

function variable(
  name: string,
  valueClass: string,
  dimensions: readonly number[],
  complex = false,
): VariableSummary {
  return { name, class: valueClass, dimensions, complex };
}

function charExact(text: string) {
  return {
    class: "char" as const,
    size: [1, text.length],
    ndims: 2,
    numel: text.length,
    complex: false as const,
    kind: "char" as const,
    code_units: Array.from(text, (character) => character.charCodeAt(0)),
  };
}

function renderEditor(options: {
  readonly variable?: VariableSummary;
  readonly range?: MatrixRange;
  readonly preview?: VariableEditorPreview | null;
  readonly loading?: boolean;
  readonly error?: string | null;
  readonly revision?: number | null;
  readonly canEdit?: boolean;
  readonly onCellCommit?: (
    edit: VariableEditorCellEdit,
  ) => Promise<VariableEditorCommitResult>;
  readonly onReload?: () => void;
  readonly onRangeChange?: (range: MatrixRange) => void;
  readonly onClose?: () => void;
}) {
  const editorVariable =
    options.variable ?? variable("matrix", "double", [3, 2]);
  const range =
    options.range ?? createVariableEditorRange(editorVariable.dimensions, 256);
  return renderWithWindowManager(
    <VariableEditorWindow
      variable={editorVariable}
      range={range}
      preview={options.preview ?? null}
      loading={options.loading ?? false}
      error={options.error ?? null}
      revision={options.revision ?? null}
      maxElements={256}
      canNavigate
      canEdit={options.canEdit ?? false}
      onRangeChange={options.onRangeChange ?? (() => undefined)}
      onCellCommit={
        options.onCellCommit ??
        (() =>
          Promise.resolve({
            ok: false,
            conflict: false,
            message: "Editing unavailable",
          }))
      }
      onReload={options.onReload ?? (() => undefined)}
      onClose={options.onClose ?? (() => undefined)}
    />,
  );
}

describe("VariableEditorWindow", () => {
  it("parses real, logical, special, and complex editor values", () => {
    expect(parseVariableEditorNumericInput(" 3.5 ", "double")).toEqual({
      real: "3.5",
      imaginary: "0",
    });
    expect(parseVariableEditorNumericInput(".5", "double")).toEqual({
      real: "0.5",
      imaginary: "0",
    });
    expect(parseVariableEditorNumericInput("true", "logical")).toEqual({
      real: "1",
      imaginary: "0",
    });
    expect(parseVariableEditorNumericInput("2 - 4i", "double")).toEqual({
      real: "2",
      imaginary: "-4",
    });
    expect(parseVariableEditorNumericInput("-i", "single")).toEqual({
      real: "0",
      imaginary: "-1",
    });
    expect(parseVariableEditorNumericInput("Inf", "double")).toEqual({
      real: "+Inf",
      imaginary: "0",
    });
    expect(
      parseVariableEditorNumericInput("18446744073709551615", "uint64", false),
    ).toEqual({
      real: "18446744073709551615",
      imaginary: "0",
    });
    expect(
      parseVariableEditorNumericInput("18446744073709551616", "uint64", false),
    ).toBeNull();
    expect(
      parseVariableEditorNumericInput("7-9i", "int64", true),
    ).toEqual({ real: "7", imaginary: "-9" });
    expect(
      parseVariableEditorNumericInput("7-9i", "int64", false),
    ).toBeNull();
    expect(parseVariableEditorNumericInput("not a value", "double")).toBeNull();
  });

  it("edits a numeric cell with one-based absolute indices", async () => {
    const onCellCommit = vi.fn(async () => ({ ok: true }) as const);
    const preview: V1MatrixPreview = {
      class: "double",
      dimensions: [3, 2],
      complex: false,
      selectedRange: { start: [2, 2], size: [2, 1] },
      values: [
        { kind: "number", value: 5 },
        { kind: "number", value: 6 },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    renderEditor({
      variable: variable("matrix", "double", [3, 2]),
      range: preview.selectedRange,
      preview,
      revision: 12,
      canEdit: true,
      onCellCommit,
    });

    const dialog = screen.getByRole("dialog", {
      name: "Variable Editor: matrix",
    });
    expect(dialog).toHaveAccessibleDescription(
      "double · 3 × 2 · editable numeric cells",
    );
    fireEvent.doubleClick(within(dialog).getByRole("cell", { name: "6" }));
    const input = within(dialog).getByRole("textbox", {
      name: "Edit matrix(3, 2)",
    });
    fireEvent.change(input, { target: { value: "7-2i" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() =>
      expect(onCellCommit).toHaveBeenCalledWith({
        indices: [3, 2],
        value: { real: "7", imaginary: "-2" },
      }),
    );
    await waitFor(() => expect(input).not.toBeInTheDocument());
  });

  it("keeps a conflicting edit visible until the user reloads", async () => {
    const onReload = vi.fn();
    const onCellCommit = vi.fn(async () => ({
      ok: false,
      conflict: true,
      message: "The workspace changed after this page loaded.",
    }) as const);
    const preview: V1MatrixPreview = {
      class: "double",
      dimensions: [1, 1],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 1] },
      values: [{ kind: "number", value: 5 }],
      truncation: { truncated: false, omittedElements: 0 },
    };
    renderEditor({
      variable: variable("matrix", "double", [1, 1]),
      range: preview.selectedRange,
      preview,
      revision: 4,
      canEdit: true,
      onCellCommit,
      onReload,
    });
    fireEvent.doubleClick(screen.getByRole("cell", { name: "5" }));
    const input = screen.getByRole("textbox", { name: "Edit matrix(1, 1)" });
    fireEvent.change(input, { target: { value: "9" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The workspace changed after this page loaded.",
    );
    expect(input).toHaveValue("9");
    fireEvent.click(screen.getByRole("button", { name: "Reload latest" }));
    expect(onReload).toHaveBeenCalledOnce();
    expect(input).not.toBeInTheDocument();
  });

  it("renders an accessible read-only table using column-major value order", () => {
    const preview: V1MatrixPreview = {
      class: "double",
      dimensions: [3, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [3, 2] },
      values: [
        { kind: "number", value: 1 },
        { kind: "number", value: 2 },
        { kind: "number", value: 3 },
        { kind: "number", value: 4 },
        { kind: "number", value: 5 },
        { kind: "number", value: 6 },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    renderEditor({ preview });

    expect(
      screen.getByRole("dialog", { name: "Variable Editor: matrix" }),
    ).toHaveAccessibleDescription("double · 3 × 2 · read-only");
    const table = screen.getByRole("table", {
      name: /Read-only values for matrix/,
    });
    expect(
      within(table)
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["1", "4", "2", "5", "3", "6"]);
    expect(within(table).getAllByRole("rowheader")).toHaveLength(3);
    expect(within(table).getAllByRole("columnheader")).toHaveLength(3);
    expect(screen.getByText("Loaded 6 of 6 elements · complete")).toBeVisible();
  });

  it("extends a 2x2 array with presentation-only filler cells", () => {
    const onRangeChange = vi.fn();
    const preview: V1MatrixPreview = {
      class: "double",
      dimensions: [2, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [2, 2] },
      values: [
        { kind: "number", value: 1 },
        { kind: "number", value: 2 },
        { kind: "number", value: 3 },
        { kind: "number", value: 4 },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    renderEditor({
      variable: variable("small", "double", [2, 2]),
      preview,
      onRangeChange,
    });

    const table = screen.getByRole("table", {
      name: /Read-only values for small/,
    });
    expect(table).toHaveAttribute("data-array-rows", "2");
    expect(table).toHaveAttribute("data-array-columns", "2");
    expect(
      within(table)
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["1", "3", "2", "4"]);
    expect(within(table).getAllByRole("rowheader")).toHaveLength(2);
    expect(within(table).getAllByRole("columnheader")).toHaveLength(3);
    expect(within(table).getAllByRole("row")).toHaveLength(3);
    expect(
      within(table).getByRole("columnheader", {
        name: "Row and column header intersection",
      }),
    ).toHaveAttribute("title", "Row and column header intersection");

    const fillers = Array.from(
      table.querySelectorAll<HTMLElement>("[data-variable-editor-filler='true']"),
    );
    expect(fillers.length).toBeGreaterThan(0);
    expect(fillers.every((cell) => cell.getAttribute("role") === "presentation")).toBe(
      true,
    );
    expect(fillers.every((cell) => cell.getAttribute("aria-hidden") === "true")).toBe(
      true,
    );
    expect(fillers.every((cell) => cell.textContent === "")).toBe(true);

    fireEvent.click(fillers[0]!);
    expect(onRangeChange).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Next row page" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Next column page" })).toBeDisabled();
  });

  it("keeps a 1x1 value as the only accessible data cell", () => {
    const preview: V1MatrixPreview = {
      class: "single",
      dimensions: [1, 1],
      complex: true,
      selectedRange: { start: [1, 1], size: [1, 1] },
      values: [{ kind: "complex", real: 1.25, imaginary: -2.5 }],
      truncation: { truncated: false, omittedElements: 0 },
    };
    renderEditor({
      variable: variable("scalar", "single", [1, 1], true),
      preview,
    });

    const table = screen.getByRole("table", {
      name: /Read-only values for scalar/,
    });
    expect(within(table).getAllByRole("cell")).toHaveLength(1);
    expect(within(table).getAllByRole("rowheader")).toHaveLength(1);
    expect(within(table).getAllByRole("columnheader")).toHaveLength(2);
    expect(within(table).getAllByRole("row")).toHaveLength(2);
    expect(within(table).getByRole("cell", { name: "1.25-2.5i" })).toHaveTextContent(
      "1.25-2.5i",
    );
    expect(within(table).getByTitle("1.25-2.5i")).toHaveClass(
      "variable-editor-cell-value",
    );
    expect(table.querySelectorAll("tbody tr")).toHaveLength(12);
  });

  it("formats numeric, logical, complex, and exact integer values", () => {
    const numericPreview: V1MatrixPreview = {
      class: "double",
      dimensions: [1, 3],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 3] },
      values: [
        { kind: "number", value: -0 },
        { kind: "special", value: "nan" },
        { kind: "special", value: "negativeInfinity" },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    const view = renderEditor({
      variable: variable("numbers", "double", [1, 3]),
      preview: numericPreview,
    });
    expect(screen.getByRole("cell", { name: "-0" })).toBeVisible();
    expect(screen.getByRole("cell", { name: "NaN" })).toBeVisible();
    expect(screen.getByRole("cell", { name: "-Inf" })).toBeVisible();

    const complexPreview: V1MatrixPreview = {
      class: "single",
      dimensions: [1, 1],
      complex: true,
      selectedRange: { start: [1, 1], size: [1, 1] },
      values: [{ kind: "complex", real: 3.5, imaginary: -4 }],
      truncation: { truncated: false, omittedElements: 0 },
    };
    view.rerender(
      <VariableEditorWindow
        variable={variable("z", "single", [1, 1], true)}
        range={complexPreview.selectedRange}
        preview={complexPreview}
        loading={false}
        error={null}
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );
    expect(screen.getByRole("cell", { name: "3.5-4i" })).toBeVisible();

    const integerPreview: V1MatrixPreview = {
      class: "int64",
      dimensions: [1, 1],
      complex: true,
      selectedRange: { start: [1, 1], size: [1, 1] },
      values: [
        {
          kind: "integer",
          real: "-9223372036854775808",
          imaginary: "9",
        },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    view.rerender(
      <VariableEditorWindow
        variable={variable("wide", "int64", [1, 1], true)}
        range={integerPreview.selectedRange}
        preview={integerPreview}
        loading={false}
        error={null}
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );
    expect(
      screen.getByRole("cell", { name: "-9223372036854775808+9i" }),
    ).toBeVisible();

    const logicalPreview: V1MatrixPreview = {
      class: "logical",
      dimensions: [1, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 2] },
      values: [
        { kind: "logical", value: true },
        { kind: "logical", value: false },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    view.rerender(
      <VariableEditorWindow
        variable={variable("flags", "logical", [1, 2])}
        range={logicalPreview.selectedRange}
        preview={logicalPreview}
        loading={false}
        error={null}
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );
    expect(screen.getByRole("cell", { name: "true" })).toBeVisible();
    expect(screen.getByRole("cell", { name: "false" })).toBeVisible();
  });

  it("renders UTF-16 char and string truth without replacement-character loss", () => {
    const charPreview: V1MatrixPreview = {
      class: "char",
      dimensions: [1, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 2] },
      values: [
        { kind: "charCodeUnit", value: 65 },
        { kind: "charCodeUnit", value: 55_357 },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    const view = renderEditor({
      variable: variable("chars", "char", [1, 2]),
      preview: charPreview,
    });
    expect(screen.getByText("'A' · U+0041 (65)")).toBeVisible();
    expect(
      screen.getByText("U+D83D (55357) · high surrogate code unit"),
    ).toBeVisible();
    expect(screen.getByRole("dialog").textContent).not.toContain("�");

    const stringPreview: V1MatrixPreview = {
      class: "string",
      dimensions: [1, 4],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 4] },
      values: [
        { kind: "string", codeUnits: [], missing: true },
        { kind: "string", codeUnits: [], missing: false },
        { kind: "string", codeUnits: [55_357], missing: false },
        { kind: "string", codeUnits: [55_357, 56_898], missing: false },
      ],
      truncation: { truncated: false, omittedElements: 0 },
    };
    view.rerender(
      <VariableEditorWindow
        variable={variable("labels", "string", [1, 4])}
        range={stringPreview.selectedRange}
        preview={stringPreview}
        loading={false}
        error={null}
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );
    expect(screen.getByText("<missing string>")).toBeVisible();
    expect(screen.getByText('\"\" · empty string')).toBeVisible();
    expect(screen.getByText(/\"\\uD83D\" · UTF-16/)).toHaveTextContent(
      "isolated surrogate U+D83D",
    );
    expect(screen.getByText(/\"🙂\" · UTF-16/)).toHaveTextContent(
      "U+D83D (55357), U+DE42 (56898)",
    );
    expect(screen.getByRole("dialog").textContent).not.toContain("�");
  });

  it("renders complete nested v2 cell and struct values without object coercion", () => {
    const preview = {
      class: "cell",
      dimensions: [1, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 2] },
      kind: "cell",
      items: [
        {
          class: "struct",
          size: [1, 1],
          ndims: 2,
          numel: 1,
          complex: false,
          kind: "struct",
          fields: ["zeta", "alpha", "glyph"],
          records: [
            {
              zeta: {
                class: "uint64",
                size: [1, 1],
                ndims: 2,
                numel: 1,
                complex: false,
                kind: "integer",
                integer: [
                  { real: "18446744073709551615", imaginary: "0" },
                ],
              },
              alpha: {
                class: "string",
                size: [1, 2],
                ndims: 2,
                numel: 2,
                complex: false,
                kind: "string",
                string_code_units: [[], []],
                missing: [true, false],
              },
              glyph: {
                class: "char",
                size: [1, 1],
                ndims: 2,
                numel: 1,
                complex: false,
                kind: "char",
                code_units: [55_357],
              },
            },
          ],
        },
        {
          class: "struct",
          size: [0, 3],
          ndims: 2,
          numel: 0,
          complex: false,
          kind: "struct",
          fields: ["beta", "alpha"],
          records: [],
        },
      ],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 6, elements: 7, codeUnits: 24, depth: 2 },
    } satisfies AggregatePreview;

    renderEditor({
      variable: variable("mixed", "cell", [1, 2]),
      preview,
    });

    expect(
      screen.getByRole("table", { name: /Read-only aggregate values for mixed/ }),
    ).toBeVisible();
    expect(screen.getByText("Field order: zeta, alpha, glyph")).toBeVisible();
    const orderedFields = screen
      .getAllByRole("rowheader")
      .map((header) => header.textContent)
      .filter((text) => text?.startsWith("."));
    expect(orderedFields).toEqual([".zeta", ".alpha", ".glyph"]);
    expect(screen.getByText("18446744073709551615")).toBeVisible();
    expect(screen.getByText("<missing string>")).toBeVisible();
    expect(screen.getByText('\"\" · empty string')).toBeVisible();
    expect(
      screen.getByText("U+D83D (55357) · high surrogate code unit"),
    ).toBeVisible();
    expect(
      screen.getByText("Shaped empty struct 0 × 3 · schema retained"),
    ).toBeVisible();
    expect(screen.getByTitle("18446744073709551615")).toHaveClass(
      "exact-value-cell-content",
    );
    expect(screen.getByRole("dialog").textContent).not.toContain(
      "[object Object]",
    );
    expect(
      screen.getByText("Loaded 2 of 2 elements · complete · 6 exact nodes"),
    ).toBeVisible();
  });

  it("keeps the ordered schema of a top-level shaped-empty v2 struct", () => {
    const preview = {
      class: "struct",
      dimensions: [0, 3],
      complex: false,
      selectedRange: { start: [1, 1], size: [0, 3] },
      kind: "struct",
      fields: ["beta", "alpha"],
      records: [],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 1, elements: 0, codeUnits: 9, depth: 0 },
    } satisfies AggregatePreview;

    renderEditor({
      variable: variable("empty_struct", "struct", [0, 3]),
      preview,
    });

    expect(screen.getByText("Field order: beta, alpha")).toBeVisible();
    expect(screen.getByText("Shaped empty struct 0 × 3")).toBeVisible();
    expect(screen.getByText("Complete field schema retained.")).toBeVisible();
    expect(
      screen.getByText("Loaded 0 of 0 elements · complete · 1 exact nodes"),
    ).toBeVisible();
  });

  it("renders named table columns and keeps cellstr, char rows, and strings compact", () => {
    const preview = {
      class: "table",
      dimensions: [2, 5],
      complex: false,
      selectedRange: { start: [1, 1], size: [2, 5] },
      kind: "table",
      variableNames: ["Labels", "Chars", "Names", "Count", "Pairs"],
      variables: [
        {
          class: "cell",
          size: [2, 1],
          ndims: 2,
          numel: 2,
          complex: false,
          kind: "cell",
          items: [charExact("alpha"), charExact("omega")],
        },
        {
          class: "char",
          size: [2, 3],
          ndims: 2,
          numel: 6,
          complex: false,
          kind: "char",
          code_units: Array.from("cdaotg", (character) =>
            character.charCodeAt(0),
          ),
        },
        {
          class: "string",
          size: [2, 1],
          ndims: 2,
          numel: 2,
          complex: false,
          kind: "string",
          string_code_units: [
            Array.from("beta", (character) => character.charCodeAt(0)),
            Array.from("gamma", (character) => character.charCodeAt(0)),
          ],
          missing: [false, false],
        },
        {
          class: "double",
          size: [2, 1],
          ndims: 2,
          numel: 2,
          complex: false,
          kind: "numeric",
          real: ["1", "2"],
          imag: ["0", "0"],
        },
        {
          class: "double",
          size: [2, 2],
          ndims: 2,
          numel: 4,
          complex: false,
          kind: "numeric",
          real: ["10", "20", "30", "40"],
          imag: ["0", "0", "0", "0"],
        },
      ],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 8, elements: 31, codeUnits: 51, depth: 2 },
    } satisfies TableAggregatePreview;

    renderEditor({
      variable: variable("T", "table", [2, 5]),
      preview,
    });

    const table = screen.getByRole("table", {
      name: /Read-only table values for T/,
    });
    expect(
      Array.from(
        table.querySelectorAll(
          ":scope > thead > tr > th:not([data-variable-editor-filler])",
        ),
      ).map((header) => header.textContent),
    ).toEqual(["↘", "Labels", "Chars", "Names", "Count", "Pairs"]);
    const rows = table.querySelectorAll(":scope > tbody > tr");
    const firstRowCells = rows[0]!.querySelectorAll(":scope > td");
    const secondRowCells = rows[1]!.querySelectorAll(":scope > td");
    expect(firstRowCells[0]).toHaveTextContent("alpha");
    expect(secondRowCells[0]).toHaveTextContent("omega");
    expect(firstRowCells[1]).toHaveTextContent("cat");
    expect(secondRowCells[1]).toHaveTextContent("dog");
    expect(firstRowCells[2]).toHaveTextContent("beta");
    expect(secondRowCells[2]).toHaveTextContent("gamma");
    expect(firstRowCells[3]).toHaveTextContent("1");
    expect(firstRowCells[0]!.querySelector(".exact-value")).toBeNull();
    expect(firstRowCells[1]!.querySelector(".exact-value")).toBeNull();
    expect(firstRowCells[2]!.querySelector(".exact-value")).toBeNull();
    expect(firstRowCells[4]!.querySelector(".exact-value-numeric")).not.toBeNull();
    expect(firstRowCells[4]).toHaveTextContent("10");
    expect(firstRowCells[4]).toHaveTextContent("30");
    expect(screen.getByText("Loaded 5 of 5 variables · complete · 8 exact nodes")).toBeVisible();
  });

  it("renders nested exact tables recursively with their variable names", () => {
    const preview = {
      class: "table",
      dimensions: [1, 1],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 1] },
      kind: "table",
      variableNames: ["Outer"],
      variables: [
        {
          class: "table",
          size: [1, 1],
          ndims: 2,
          numel: 1,
          complex: false,
          kind: "table",
          variableNames: ["Inner"],
          variables: [
            {
              class: "double",
              size: [1, 1],
              ndims: 2,
              numel: 1,
              complex: false,
              kind: "numeric",
              real: ["7"],
              imag: ["0"],
            },
          ],
        },
      ],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 4, elements: 4, codeUnits: 10, depth: 3 },
    } satisfies TableAggregatePreview;

    renderEditor({
      variable: variable("nested", "table", [1, 1]),
      preview,
    });

    const nestedTable = screen.getByRole("table", {
      name: "Exact table contents with named variables",
    });
    expect(within(nestedTable).getByRole("columnheader", { name: "Inner" })).toBeVisible();
    expect(within(nestedTable).getByText("7")).toBeVisible();
  });

  it("shows table prefix truncation and paginates rows and variables", () => {
    const onRangeChange = vi.fn();
    const dimensions = [40, 40] as const;
    const range = createVariableEditorRange(dimensions, 256);
    const pagedView = renderEditor({
      variable: variable("paged", "table", dimensions),
      range,
      onRangeChange,
    });

    fireEvent.click(screen.getByRole("button", { name: "Next row page" }));
    expect(onRangeChange).toHaveBeenLastCalledWith({
      start: [17, 1],
      size: [16, 16],
    });
    fireEvent.click(screen.getByRole("button", { name: "Next variable page" }));
    expect(onRangeChange).toHaveBeenLastCalledWith({
      start: [1, 17],
      size: [16, 16],
    });
    pagedView.unmount();

    const truncated = {
      class: "table",
      dimensions: [2, 3],
      complex: false,
      selectedRange: { start: [1, 1], size: [2, 3] },
      kind: "table",
      variableNames: ["A", "B"],
      variables: [
        {
          class: "double",
          size: [2, 1],
          ndims: 2,
          numel: 2,
          complex: false,
          kind: "numeric",
          real: ["1", "2"],
          imag: ["0", "0"],
        },
        {
          class: "logical",
          size: [2, 1],
          ndims: 2,
          numel: 2,
          complex: false,
          kind: "logical",
          logical: [true, false],
        },
      ],
      truncation: { truncated: true, omittedElements: 1 },
      usage: { nodes: 3, elements: 6, codeUnits: 2, depth: 1 },
    } satisfies TableAggregatePreview;
    renderEditor({
      variable: variable("short_table", "table", [2, 3]),
      preview: truncated,
    });

    const table = screen.getByRole("table", {
      name: /Read-only table values for short_table/,
    });
    expect(
      Array.from(
        table.querySelectorAll(
          ":scope > thead > tr > th:not([data-variable-editor-filler])",
        ),
      ).map((header) => header.textContent),
    ).toEqual(["↘", "A", "B", "not returned"]);
    expect(within(table).getAllByText("not returned")).toHaveLength(3);
    expect(
      screen.getByText(
        "Loaded 2 of 3 variables · truncated · 1 variable omitted by kernel limits",
      ),
    ).toBeVisible();
  });

  it("keeps 0xN table headers and Nx0 table row headers", () => {
    const zeroRows = {
      class: "table",
      dimensions: [0, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [0, 2] },
      kind: "table",
      variableNames: ["A", "B"],
      variables: [
        {
          class: "double",
          size: [0, 1],
          ndims: 2,
          numel: 0,
          complex: false,
          kind: "numeric",
          real: [],
          imag: [],
        },
        {
          class: "logical",
          size: [0, 1],
          ndims: 2,
          numel: 0,
          complex: false,
          kind: "logical",
          logical: [],
        },
      ],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 3, elements: 2, codeUnits: 2, depth: 1 },
    } satisfies TableAggregatePreview;
    const view = renderEditor({
      variable: variable("zero_rows", "table", [0, 2]),
      preview: zeroRows,
    });

    const zeroRowsTable = screen.getByRole("table", {
      name: /Read-only table values for zero_rows/,
    });
    expect(within(zeroRowsTable).getByRole("columnheader", { name: "A" })).toBeVisible();
    expect(within(zeroRowsTable).getByRole("columnheader", { name: "B" })).toBeVisible();
    expect(within(zeroRowsTable).queryAllByRole("rowheader")).toHaveLength(0);
    expect(screen.getByText("zero_rows has no rows in this selection.")).toBeVisible();

    const zeroVariables = {
      class: "table",
      dimensions: [3, 0],
      complex: false,
      selectedRange: { start: [1, 1], size: [3, 0] },
      kind: "table",
      variableNames: [],
      variables: [],
      truncation: { truncated: false, omittedElements: 0 },
      usage: { nodes: 1, elements: 0, codeUnits: 0, depth: 0 },
    } satisfies TableAggregatePreview;
    view.rerender(
      <VariableEditorWindow
        variable={variable("zero_variables", "table", [3, 0])}
        range={zeroVariables.selectedRange}
        preview={zeroVariables}
        loading={false}
        error={null}
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );

    const zeroVariablesTable = screen.getByRole("table", {
      name: /Read-only table values for zero_variables/,
    });
    expect(within(zeroVariablesTable).getAllByRole("rowheader")).toHaveLength(3);
    expect(within(zeroVariablesTable).queryAllByRole("cell")).toHaveLength(0);
    expect(
      screen.getByText("zero_variables has 3 selected rows and no variables."),
    ).toBeVisible();
  });

  it("requests bounded row, column, and higher-dimensional pages", () => {
    const onRangeChange = vi.fn();
    const dimensions = [40, 40, 2, 3];
    const range = createVariableEditorRange(dimensions, 256);
    expect(range).toEqual({
      start: [1, 1, 1, 1],
      size: [16, 16, 1, 1],
    });
    renderEditor({
      variable: variable("volume", "single", dimensions),
      range,
      onRangeChange,
    });

    fireEvent.click(screen.getByRole("button", { name: "Next row page" }));
    expect(onRangeChange).toHaveBeenLastCalledWith({
      start: [17, 1, 1, 1],
      size: [16, 16, 1, 1],
    });
    fireEvent.click(screen.getByRole("button", { name: "Next column page" }));
    expect(onRangeChange).toHaveBeenLastCalledWith({
      start: [1, 17, 1, 1],
      size: [16, 16, 1, 1],
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Next slice in dimension 3" }),
    );
    expect(onRangeChange).toHaveBeenLastCalledWith({
      start: [1, 1, 2, 1],
      size: [16, 16, 1, 1],
    });
  });

  it("shows empty, loading, error, and truncation states clearly", () => {
    const emptyVariable = variable("empty", "double", [0, 4]);
    const emptyRange = createVariableEditorRange(emptyVariable.dimensions, 256);
    const view = renderEditor({
      variable: emptyVariable,
      range: emptyRange,
      preview: {
        class: "double",
        dimensions: [0, 4],
        complex: false,
        selectedRange: emptyRange,
        values: [],
        truncation: { truncated: false, omittedElements: 0 },
      },
    });
    expect(screen.getByText("empty is an empty 0 × 4 array.")).toBeVisible();
    const emptyTable = screen.getByRole("table", {
      name: "Read-only empty grid for empty",
    });
    expect(emptyTable).toHaveAttribute("data-array-rows", "0");
    expect(emptyTable).toHaveAttribute("data-array-columns", "4");
    expect(within(emptyTable).queryAllByRole("cell")).toHaveLength(0);
    expect(within(emptyTable).queryAllByRole("rowheader")).toHaveLength(0);
    expect(within(emptyTable).getAllByRole("columnheader")).toHaveLength(5);
    expect(within(emptyTable).getAllByRole("row")).toHaveLength(1);
    expect(
      emptyTable.querySelectorAll("[data-variable-editor-filler='true']").length,
    ).toBeGreaterThan(0);

    const ndEmptyVariable = variable("empty_volume", "double", [2, 2, 0]);
    const ndEmptyRange = createVariableEditorRange(
      ndEmptyVariable.dimensions,
      256,
    );
    view.rerender(
      <VariableEditorWindow
        variable={ndEmptyVariable}
        range={ndEmptyRange}
        preview={{
          class: "double",
          dimensions: [2, 2, 0],
          complex: false,
          selectedRange: ndEmptyRange,
          values: [],
          truncation: { truncated: false, omittedElements: 0 },
        }}
        loading={false}
        error={null}
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );
    const ndEmptyTable = screen.getByRole("table", {
      name: "Read-only empty grid for empty_volume",
    });
    expect(within(ndEmptyTable).queryAllByRole("cell")).toHaveLength(0);
    expect(within(ndEmptyTable).getAllByRole("rowheader")).toHaveLength(2);
    expect(within(ndEmptyTable).getAllByRole("columnheader")).toHaveLength(3);

    view.rerender(
      <VariableEditorWindow
        variable={variable("slow", "double", [2, 2])}
        range={{ start: [1, 1], size: [2, 2] }}
        preview={null}
        loading
        error={null}
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );
    expect(screen.getByRole("status", { name: "" })).toHaveTextContent(
      "Loading slow",
    );

    view.rerender(
      <VariableEditorWindow
        variable={variable("broken", "double", [2, 2])}
        range={{ start: [1, 1], size: [2, 2] }}
        preview={null}
        loading={false}
        error="Kernel refused the range."
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Kernel refused the range.",
    );

    const truncatedPreview: V1MatrixPreview = {
      class: "double",
      dimensions: [2, 2],
      complex: false,
      selectedRange: { start: [1, 1], size: [2, 2] },
      values: [
        { kind: "number", value: 1 },
        { kind: "number", value: 2 },
        { kind: "number", value: 3 },
      ],
      truncation: { truncated: true, omittedElements: 1 },
    };
    view.rerender(
      <VariableEditorWindow
        variable={variable("short", "double", [2, 2])}
        range={truncatedPreview.selectedRange}
        preview={truncatedPreview}
        loading={false}
        error={null}
        maxElements={256}
        canNavigate
        onRangeChange={() => undefined}
        onClose={() => undefined}
      />,
    );
    expect(screen.getByText("not returned")).toBeVisible();
    expect(screen.getByText(/Loaded 3 of 4 elements/)).toHaveTextContent(
      "truncated · 1 omitted",
    );
  });

  it("focuses the close button and closes on Escape", () => {
    const onClose = vi.fn();
    renderEditor({ onClose });
    expect(
      screen.getByRole("button", {
        name: "Close Variable Editor for matrix",
      }),
    ).toHaveFocus();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("keeps long ordinary values single-line while exposing their complete text", () => {
    const codeUnits = Array.from({ length: 80 }, () => 65);
    const preview: V1MatrixPreview = {
      class: "string",
      dimensions: [1, 1],
      complex: false,
      selectedRange: { start: [1, 1], size: [1, 1] },
      values: [{ kind: "string", codeUnits, missing: false }],
      truncation: { truncated: false, omittedElements: 0 },
    };
    renderEditor({
      variable: variable("long_text", "string", [1, 1]),
      preview,
    });

    const value = document.querySelector<HTMLElement>(
      ".variable-editor-cell-value",
    );
    expect(value).not.toBeNull();
    expect(value!.textContent!.length).toBeGreaterThan(500);
    expect(value).toHaveAttribute("title", value!.textContent!);
    expect(value).toHaveAttribute("aria-label", value!.textContent!);

    expect(styles).toMatch(
      /\.variable-editor-table\s*\{[^}]*width:\s*max-content;[^}]*table-layout:\s*fixed;/s,
    );
    expect(styles).toMatch(
      /\.variable-editor-table col\.variable-editor-grid-column\s*\{[^}]*width:\s*var\(--variable-editor-column-width\);/s,
    );
    expect(styles).toMatch(
      /\.variable-editor-cell-value\s*\{[^}]*overflow:\s*hidden;[^}]*text-overflow:\s*ellipsis;[^}]*white-space:\s*nowrap;/s,
    );
    expect(styles).toMatch(
      /\.variable-editor-table th,\s*\.variable-editor-table td\s*\{[^}]*height:\s*var\(--variable-editor-row-height\);/s,
    );
  });

  it("pins both header rails and bounds aggregate expansion inside the grid", () => {
    expect(styles).toMatch(
      /\.variable-editor-table thead th\s*\{[^}]*position:\s*sticky;[^}]*top:\s*0;/s,
    );
    expect(styles).toMatch(
      /\.variable-editor-table tbody th\s*\{[^}]*position:\s*sticky;[^}]*left:\s*0;/s,
    );
    expect(styles).toMatch(
      /\.variable-editor-table \.variable-editor-corner-cell\s*\{[^}]*z-index:\s*4;[^}]*top:\s*0;[^}]*left:\s*0;/s,
    );
    expect(styles).toMatch(
      /\.aggregate-root-table \.variable-editor-aggregate-cell\s*\{[^}]*max-width:\s*320px;[^}]*max-height:\s*204px;/s,
    );
    expect(styles).toMatch(
      /\.variable-editor-aggregate-cell-content\s*\{[^}]*max-height:\s*188px;[^}]*overflow:\s*auto;/s,
    );
    expect(styles).toMatch(
      /\.exact-value-cell-content\s*\{[^}]*overflow:\s*hidden;[^}]*text-overflow:\s*ellipsis;[^}]*white-space:\s*nowrap;/s,
    );
    expect(styles).toMatch(
      /\.variable-editor-table col\.variable-editor-table-column\s*\{[^}]*width:\s*196px;/s,
    );
    expect(styles).toMatch(
      /\.table-root-table \.variable-editor-table-variable-cell\.has-exact-value\s*\{[^}]*max-height:\s*180px;/s,
    );
    expect(styles).toMatch(
      /\.variable-editor-table-variable-cell\.has-exact-value\s+\.variable-editor-table-variable-cell-content\s*\{[^}]*max-height:\s*166px;[^}]*overflow:\s*auto;/s,
    );
  });
});
