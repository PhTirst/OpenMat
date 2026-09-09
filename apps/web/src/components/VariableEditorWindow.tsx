import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { MatrixRange, VariableSummary } from "../protocol/kernel-v0";
import type {
  AggregatePreview,
  CellExactValue,
  ExactValue,
  MatrixPreview,
  PreviewValue,
  StructRecord,
  TableAggregatePreview,
  TableExactValue,
  NumericScalar,
} from "../protocol/kernel-v2";
import type { OpenMatAppDefinition } from "../windowing/app-registry";
import { useOpenMatWindowManagerActions } from "../windowing/WindowManager";

export type VariableEditorPreview = MatrixPreview | AggregatePreview;

export interface VariableEditorCellEdit {
  readonly indices: readonly number[];
  readonly value: NumericScalar;
}

export type VariableEditorCommitResult =
  | { readonly ok: true }
  | {
      readonly ok: false;
      readonly conflict: boolean;
      readonly message: string;
    };

const MAX_PAGE_ROWS = 16;
const MAX_PAGE_COLUMNS = 16;
const MIN_VIEWPORT_ROWS = 12;
const MIN_VIEWPORT_COLUMNS = 8;

interface VariableEditorWindowProps {
  readonly variable: VariableSummary;
  readonly range: MatrixRange;
  readonly preview: VariableEditorPreview | null;
  readonly loading: boolean;
  readonly error: string | null;
  readonly revision?: number | null;
  readonly maxElements: number;
  readonly canNavigate: boolean;
  readonly canEdit?: boolean;
  readonly onRangeChange: (range: MatrixRange) => void;
  readonly onCellCommit?: (
    edit: VariableEditorCellEdit,
  ) => Promise<VariableEditorCommitResult>;
  readonly onReload?: () => void;
  readonly onClose: () => void;
}

interface ActiveCellEdit {
  readonly key: string;
  readonly indices: readonly number[];
  readonly originalText: string;
}

const INTEGER_CLASS_BOUNDS: Readonly<
  Record<string, readonly [minimum: bigint, maximum: bigint]>
> = {
  int8: [-128n, 127n],
  uint8: [0n, 255n],
  int16: [-32_768n, 32_767n],
  uint16: [0n, 65_535n],
  int32: [-2_147_483_648n, 2_147_483_647n],
  uint32: [0n, 4_294_967_295n],
  int64: [-9_223_372_036_854_775_808n, 9_223_372_036_854_775_807n],
  uint64: [0n, 18_446_744_073_709_551_615n],
};
const EDITABLE_NUMERIC_CLASSES = new Set([
  "double",
  "single",
  "logical",
  ...Object.keys(INTEGER_CLASS_BOUNDS),
]);

function canonicalNumberComponent(input: string): string | null {
  const text = input.trim();
  if (text === "NaN") {
    return "NaN";
  }
  if (text === "Inf" || text === "+Inf" || text === "Infinity" || text === "+Infinity") {
    return "+Inf";
  }
  if (text === "-Inf" || text === "-Infinity") {
    return "-Inf";
  }
  if (
    !/^-?(?:(?:0|[1-9]\d*)(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(
      text,
    )
  ) {
    return null;
  }
  const value = Number(text);
  if (!Number.isFinite(value)) {
    return value === Number.POSITIVE_INFINITY
      ? "+Inf"
      : value === Number.NEGATIVE_INFINITY
        ? "-Inf"
        : null;
  }
  return Object.is(value, -0) ? "-0" : value.toString();
}

function complexSeparatorIndex(text: string): number {
  for (let index = text.length - 1; index > 0; index -= 1) {
    const character = text[index];
    if (
      (character === "+" || character === "-") &&
      text[index - 1]?.toLowerCase() !== "e"
    ) {
      return index;
    }
  }
  return -1;
}

function splitNumericInput(
  compact: string,
): readonly [real: string, imaginary: string] {
  if (!/[ij]$/i.test(compact)) {
    return [compact, "0"];
  }
  const body = compact.slice(0, -1);
  const separator = complexSeparatorIndex(body);
  const realText = separator < 0 ? "0" : body.slice(0, separator);
  let imaginaryText = separator < 0 ? body : body.slice(separator);
  if (imaginaryText === "" || imaginaryText === "+") {
    imaginaryText = "1";
  } else if (imaginaryText === "-") {
    imaginaryText = "-1";
  }
  return [realText, imaginaryText];
}

function canonicalIntegerComponent(
  input: string,
  bounds: readonly [minimum: bigint, maximum: bigint],
): string | null {
  if (!/^(?:0|-[1-9]\d*|[1-9]\d*)$/.test(input)) {
    return null;
  }
  const value = BigInt(input);
  return value < bounds[0] || value > bounds[1] ? null : value.toString();
}

export function parseVariableEditorNumericInput(
  input: string,
  valueClass: string,
  allowComplex = true,
): NumericScalar | null {
  const compact = input.trim().replaceAll(" ", "");
  if (compact.length === 0) {
    return null;
  }
  if (valueClass === "logical") {
    if (compact.toLowerCase() === "true") {
      return { real: "1", imaginary: "0" };
    }
    if (compact.toLowerCase() === "false") {
      return { real: "0", imaginary: "0" };
    }
  }

  const [realText, imaginaryText] = splitNumericInput(compact);
  const integerBounds = INTEGER_CLASS_BOUNDS[valueClass];
  const real =
    integerBounds === undefined
      ? canonicalNumberComponent(realText)
      : canonicalIntegerComponent(realText, integerBounds);
  const imaginary =
    integerBounds === undefined
      ? canonicalNumberComponent(imaginaryText)
      : canonicalIntegerComponent(imaginaryText, integerBounds);
  if (
    real === null ||
    imaginary === null ||
    (!allowComplex && imaginary !== "0" && imaginary !== "-0")
  ) {
    return null;
  }
  return { real, imaginary };
}

function formatDimensions(dimensions: readonly number[]): string {
  return dimensions.join(" × ");
}

function pageShape(
  dimensions: readonly number[],
  maxElements: number,
): readonly [number, number] {
  const elementBudget = Math.max(1, Math.min(256, Math.floor(maxElements)));
  const rowExtent = dimensions[0] ?? 0;
  const columnExtent = dimensions[1] ?? 0;
  const rows = Math.min(rowExtent, MAX_PAGE_ROWS, elementBudget);
  const columnBudget = Math.max(1, Math.floor(elementBudget / Math.max(rows, 1)));
  const columns = Math.min(columnExtent, MAX_PAGE_COLUMNS, columnBudget);
  return [rows, columns];
}

function clampStart(value: number | undefined, extent: number): number {
  if (extent === 0) {
    return 1;
  }
  return Math.min(Math.max(value ?? 1, 1), extent);
}

export function createVariableEditorRange(
  dimensions: readonly number[],
  maxElements: number,
  requestedStart: readonly number[] = dimensions.map(() => 1),
): MatrixRange {
  const [pageRows, pageColumns] = pageShape(dimensions, maxElements);
  const start = dimensions.map((extent, index) =>
    clampStart(requestedStart[index], extent),
  );
  const size = dimensions.map((extent, index) => {
    if (extent === 0) {
      return 0;
    }
    const remaining = extent - (start[index] ?? 1) + 1;
    if (index === 0) {
      return Math.min(pageRows, remaining);
    }
    if (index === 1) {
      return Math.min(pageColumns, remaining);
    }
    return 1;
  });
  return { start, size };
}

function formatFiniteNumber(value: number): string {
  return Object.is(value, -0) ? "-0" : value.toString();
}

function formatComplex(real: number, imaginary: number): string {
  const imaginaryText = formatFiniteNumber(imaginary);
  const sign = imaginary < 0 || Object.is(imaginary, -0) ? "" : "+";
  return `${formatFiniteNumber(real)}${sign}${imaginaryText}i`;
}

function codeUnitHex(codeUnit: number): string {
  return `U+${codeUnit.toString(16).toUpperCase().padStart(4, "0")}`;
}

function isHighSurrogate(codeUnit: number): boolean {
  return codeUnit >= 0xd800 && codeUnit <= 0xdbff;
}

function isLowSurrogate(codeUnit: number): boolean {
  return codeUnit >= 0xdc00 && codeUnit <= 0xdfff;
}

function escapedCodeUnit(codeUnit: number): string {
  switch (codeUnit) {
    case 0:
      return "\\0";
    case 9:
      return "\\t";
    case 10:
      return "\\n";
    case 13:
      return "\\r";
    case 34:
      return '\\"';
    case 92:
      return "\\\\";
    case 0xfffd:
      return "\\uFFFD";
    default:
      if (
        codeUnit < 0x20 ||
        codeUnit === 0x7f ||
        isHighSurrogate(codeUnit) ||
        isLowSurrogate(codeUnit)
      ) {
        return `\\u${codeUnit.toString(16).toUpperCase().padStart(4, "0")}`;
      }
      return String.fromCharCode(codeUnit);
  }
}

function safeCodeUnitString(codeUnits: readonly number[]): {
  readonly display: string;
  readonly isolatedSurrogates: readonly string[];
} {
  let display = "";
  const isolatedSurrogates: string[] = [];
  for (let index = 0; index < codeUnits.length; index += 1) {
    const codeUnit = codeUnits[index] ?? 0;
    const next = codeUnits[index + 1];
    if (isHighSurrogate(codeUnit) && next !== undefined && isLowSurrogate(next)) {
      display += String.fromCharCode(codeUnit, next);
      index += 1;
      continue;
    }
    if (isHighSurrogate(codeUnit) || isLowSurrogate(codeUnit)) {
      isolatedSurrogates.push(codeUnitHex(codeUnit));
    }
    display += escapedCodeUnit(codeUnit);
  }
  return { display, isolatedSurrogates };
}

function stringCodeUnits(value: string): readonly number[] {
  const codeUnits: number[] = [];
  for (let index = 0; index < value.length; index += 1) {
    codeUnits.push(value.charCodeAt(index));
  }
  return codeUnits;
}

function formatStringCodeUnits(codeUnits: readonly number[]): string {
  if (codeUnits.length === 0) {
    return '\"\" · empty string';
  }
  const { display, isolatedSurrogates } = safeCodeUnitString(codeUnits);
  const exact = codeUnits
    .map((codeUnit) => `${codeUnitHex(codeUnit)} (${codeUnit.toString()})`)
    .join(", ");
  const isolated =
    isolatedSurrogates.length === 0
      ? ""
      : ` · isolated surrogate ${isolatedSurrogates.join(", ")}`;
  return `\"${display}\" · UTF-16 [${exact}]${isolated}`;
}

function formatCharCodeUnit(codeUnit: number): string {
  const exact = `${codeUnitHex(codeUnit)} (${codeUnit.toString()})`;
  if (isHighSurrogate(codeUnit)) {
    return `${exact} · high surrogate code unit`;
  }
  if (isLowSurrogate(codeUnit)) {
    return `${exact} · low surrogate code unit`;
  }
  return `'${escapedCodeUnit(codeUnit)}' · ${exact}`;
}

function formatPreviewValue(value: PreviewValue, complex: boolean): string {
  switch (value.kind) {
    case "number":
      return formatFiniteNumber(value.value);
    case "complex":
      return formatComplex(value.real, value.imaginary);
    case "logical":
      return value.value ? "true" : "false";
    case "text":
      return formatStringCodeUnits(stringCodeUnits(value.value));
    case "special":
      switch (value.value) {
        case "nan":
          return "NaN";
        case "infinity":
          return "Inf";
        case "negativeInfinity":
          return "-Inf";
      }
    case "missing":
      return "<missing>";
    case "charCodeUnit":
      return formatCharCodeUnit(value.value);
    case "integer": {
      if (!complex) {
        return value.real;
      }
      const sign = value.imaginary.startsWith("-") ? "" : "+";
      return `${value.real}${sign}${value.imaginary}i`;
    }
    case "string":
      return value.missing
        ? "<missing string>"
        : formatStringCodeUnits(value.codeUnits);
  }
}

function isAggregatePreview(
  preview: VariableEditorPreview,
): preview is AggregatePreview {
  return (
    "kind" in preview &&
    (preview.kind === "cell" ||
      preview.kind === "struct" ||
      preview.kind === "table")
  );
}

function formatExactComplex(
  real: string,
  imaginary: string,
  complex: boolean,
): string {
  if (!complex) {
    return real;
  }
  const sign = imaginary.startsWith("-") ? "" : "+";
  return `${real}${sign}${imaginary}i`;
}

function linearSubscript(
  linearIndex: number,
  size: readonly number[],
): readonly number[] {
  let remaining = linearIndex;
  return size.map((extent) => {
    const subscript = (remaining % extent) + 1;
    remaining = Math.floor(remaining / extent);
    return subscript;
  });
}

function formatSubscript(linearIndex: number, size: readonly number[]): string {
  return `(${linearSubscript(linearIndex, size).join(", ")})`;
}

function exactLeafText(value: ExactValue, index: number): string {
  switch (value.kind) {
    case "numeric":
      return formatExactComplex(
        value.real[index] ?? "",
        value.imag[index] ?? "",
        value.complex,
      );
    case "integer": {
      const item = value.integer[index];
      return item === undefined
        ? ""
        : formatExactComplex(item.real, item.imaginary, value.complex);
    }
    case "logical":
      return value.logical[index] ? "true" : "false";
    case "char":
      return formatCharCodeUnit(value.code_units[index] ?? 0);
    case "string":
      return value.missing[index]
        ? "<missing string>"
        : formatStringCodeUnits(value.string_code_units[index] ?? []);
    case "cell":
    case "struct":
    case "table":
      return "";
  }
}

function selectExactRowEntries<T>(
  entries: readonly T[],
  rowCount: number,
  rowOffset: number,
): readonly T[] {
  if (rowCount === 0) {
    return [];
  }
  const entriesPerRow = Math.floor(entries.length / rowCount);
  return Array.from(
    { length: entriesPerRow },
    (_entry, trailingIndex) => entries[rowOffset + trailingIndex * rowCount]!,
  );
}

function exactRowSlice(value: ExactValue, rowOffset: number): ExactValue {
  const rowCount = value.size[0] ?? 0;
  const size = [1, ...value.size.slice(1)];
  const numel = rowCount === 0 ? 0 : value.numel / rowCount;
  const metadata = { size, ndims: size.length, numel };
  switch (value.kind) {
    case "numeric":
      return {
        ...value,
        ...metadata,
        real: selectExactRowEntries(value.real, rowCount, rowOffset),
        imag: selectExactRowEntries(value.imag, rowCount, rowOffset),
      };
    case "integer":
      return {
        ...value,
        ...metadata,
        integer: selectExactRowEntries(value.integer, rowCount, rowOffset),
      };
    case "logical":
      return {
        ...value,
        ...metadata,
        logical: selectExactRowEntries(value.logical, rowCount, rowOffset),
      };
    case "char":
      return {
        ...value,
        ...metadata,
        code_units: selectExactRowEntries(value.code_units, rowCount, rowOffset),
      };
    case "string":
      return {
        ...value,
        ...metadata,
        string_code_units: selectExactRowEntries(
          value.string_code_units,
          rowCount,
          rowOffset,
        ),
        missing: selectExactRowEntries(value.missing, rowCount, rowOffset),
      };
    case "cell":
      return {
        ...value,
        ...metadata,
        items: selectExactRowEntries(value.items, rowCount, rowOffset),
      };
    case "struct":
      return {
        ...value,
        ...metadata,
        records: selectExactRowEntries(value.records, rowCount, rowOffset),
      };
    case "table":
      return {
        ...value,
        size: [1, value.size[1]],
        ndims: 2,
        numel: value.size[1],
        variables: value.variables.map((variable) =>
          exactRowSlice(variable, rowOffset),
        ),
      };
  }
}

function isDirectTableColumn(value: ExactValue, rowCount: number): boolean {
  return (
    compactTableText(value, rowCount, 0) !== null ||
    (value.kind !== "cell" &&
    value.kind !== "struct" &&
    value.kind !== "table" &&
    value.numel === rowCount)
  );
}

function friendlyCodeUnitText(codeUnits: readonly number[]): string {
  return safeCodeUnitString(codeUnits).display;
}

function isCellstrColumn(
  value: ExactValue,
  rowCount: number,
): value is CellExactValue {
  return (
    value.kind === "cell" &&
    value.size[0] === rowCount &&
    value.numel === rowCount &&
    value.items.every(
      (item) =>
        item.kind === "char" &&
        item.size.length === 2 &&
        (item.size[0] === 0 || item.size[0] === 1),
    )
  );
}

function compactTableText(
  value: ExactValue,
  rowCount: number,
  rowOffset: number,
): string | null {
  if (
    value.kind === "char" &&
    value.size.length === 2 &&
    value.size[0] === rowCount
  ) {
    return friendlyCodeUnitText(
      selectExactRowEntries(value.code_units, rowCount, rowOffset),
    );
  }
  if (
    value.kind === "string" &&
    value.size[0] === rowCount &&
    value.numel === rowCount
  ) {
    return value.missing[rowOffset]
      ? "<missing string>"
      : friendlyCodeUnitText(value.string_code_units[rowOffset] ?? []);
  }
  if (isCellstrColumn(value, rowCount)) {
    const item = value.items[rowOffset];
    return item?.kind === "char" ? friendlyCodeUnitText(item.code_units) : "";
  }
  return null;
}

function TableVariableCellValue({
  value,
  rowCount,
  rowOffset,
}: {
  readonly value: ExactValue;
  readonly rowCount: number;
  readonly rowOffset: number;
}) {
  const compactText = compactTableText(value, rowCount, rowOffset);
  if (compactText !== null || isDirectTableColumn(value, rowCount)) {
    const text = compactText ?? exactLeafText(value, rowOffset);
    return (
      <span
        className="variable-editor-cell-value"
        title={text}
        aria-label={text}
      >
        {text}
      </span>
    );
  }
  return (
    <div className="table-variable-exact-value">
      <ExactValueView value={exactRowSlice(value, rowOffset)} />
    </div>
  );
}

function ExactLeafTable({ value }: { readonly value: ExactValue }) {
  if (value.numel === 0) {
    return (
      <p className="exact-value-empty">
        Shaped empty {value.class} {formatDimensions(value.size)} · 0 elements
      </p>
    );
  }
  return (
    <div className="exact-table-scroller">
      <table className="exact-value-table">
        <caption>
          Complete column-major contents of {value.class} {formatDimensions(value.size)}
        </caption>
        <thead>
          <tr>
            <th scope="col">Linear</th>
            <th scope="col">Subscript</th>
            <th scope="col">Exact value</th>
          </tr>
        </thead>
        <tbody>
          {Array.from({ length: value.numel }, (_entry, index) => {
            const text = exactLeafText(value, index);
            return (
              <tr key={index}>
                <th scope="row">{(index + 1).toString()}</th>
                <td>{formatSubscript(index, value.size)}</td>
                <td>
                  <span
                    className="exact-value-cell-content"
                    title={text}
                    aria-label={text}
                  >
                    {text}
                  </span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function FillerCell({ header = false }: { readonly header?: boolean }) {
  const sharedProps = {
    "aria-hidden": true,
    "data-variable-editor-filler": "true",
    role: "presentation",
  } as const;
  return header ? (
    <th className="variable-editor-filler-header" {...sharedProps} />
  ) : (
    <td className="variable-editor-filler-cell" {...sharedProps} />
  );
}

function VariableEditorGrid({
  caption,
  rowStart,
  columnStart,
  rowCount,
  columnCount,
  aggregate = false,
  table = false,
  columnHeaders,
  renderCell,
}: {
  readonly caption: string;
  readonly rowStart: number;
  readonly columnStart: number;
  readonly rowCount: number;
  readonly columnCount: number;
  readonly aggregate?: boolean;
  readonly table?: boolean;
  readonly columnHeaders?: readonly (string | undefined)[];
  readonly renderCell: (rowOffset: number, columnOffset: number) => ReactNode;
}) {
  const viewportRows = Math.max(rowCount, MIN_VIEWPORT_ROWS);
  const viewportColumns = Math.max(columnCount, MIN_VIEWPORT_COLUMNS);

  return (
    <table
      className={`variable-editor-table${aggregate ? " aggregate-root-table" : ""}${table ? " table-root-table" : ""}`}
      data-array-rows={rowCount.toString()}
      data-array-columns={columnCount.toString()}
    >
      <caption>{caption}</caption>
      <colgroup>
        <col className="variable-editor-row-header-column" />
        {Array.from({ length: viewportColumns }, (_entry, columnOffset) => (
          <col
            className={
              table && columnOffset < columnCount
                ? "variable-editor-table-column"
                : aggregate && columnOffset < columnCount
                ? "variable-editor-aggregate-column"
                : "variable-editor-grid-column"
            }
            key={columnOffset}
          />
        ))}
      </colgroup>
      <thead>
        <tr>
          <th
            className="variable-editor-corner-cell"
            scope="col"
            aria-label={`Row and ${table ? "variable" : "column"} header intersection`}
            title={`Row and ${table ? "variable" : "column"} header intersection`}
          >
            <span aria-hidden="true">↘</span>
          </th>
          {Array.from({ length: viewportColumns }, (_entry, columnOffset) =>
            columnOffset < columnCount ? (
              <th
                className={`variable-editor-column-header${table ? " variable-editor-table-column-header" : ""}`}
                scope="col"
                title={
                  table
                    ? columnHeaders?.[columnOffset] === undefined
                      ? `Variable ${(columnStart + columnOffset).toString()} was not returned`
                      : `Variable ${(columnStart + columnOffset).toString()}: ${columnHeaders[columnOffset]}`
                    : `Column ${(columnStart + columnOffset).toString()}`
                }
                key={columnOffset}
              >
                {table
                  ? (columnHeaders?.[columnOffset] ?? "not returned")
                  : (columnStart + columnOffset).toString()}
              </th>
            ) : (
              <FillerCell header key={columnOffset} />
            ),
          )}
        </tr>
      </thead>
      <tbody>
        {Array.from({ length: viewportRows }, (_entry, rowOffset) => {
          const dataRow = rowOffset < rowCount;
          return (
            <tr
              className={dataRow ? undefined : "variable-editor-filler-row"}
              aria-hidden={dataRow ? undefined : true}
              data-variable-editor-filler-row={dataRow ? undefined : "true"}
              role={dataRow ? undefined : "presentation"}
              key={rowOffset}
            >
              {dataRow ? (
                <th
                  className="variable-editor-row-header"
                  scope="row"
                  title={`Row ${(rowStart + rowOffset).toString()}`}
                >
                  {(rowStart + rowOffset).toString()}
                </th>
              ) : (
                <FillerCell header />
              )}
              {Array.from({ length: viewportColumns }, (_cell, columnOffset) =>
                dataRow && columnOffset < columnCount ? (
                  renderCell(rowOffset, columnOffset)
                ) : (
                  <FillerCell key={columnOffset} />
                ),
              )}
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function StructFields({
  fields,
  record,
}: {
  readonly fields: readonly string[];
  readonly record: StructRecord;
}) {
  if (fields.length === 0) {
    return <p className="exact-value-empty">Unfielded struct record</p>;
  }
  return (
    <table className="struct-fields-table">
      <caption>Struct fields in declared order</caption>
      <tbody>
        {fields.map((field) => (
          <tr key={field}>
            <th scope="row">.{field}</th>
            <td>
              <ExactValueView value={record[field]!} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ExactTableValueView({ value }: { readonly value: TableExactValue }) {
  const rowCount = value.size[0];
  const variableCount = value.size[1];
  if (variableCount === 0) {
    return (
      <p className="exact-value-empty">
        Table has {rowCount.toString()} row{rowCount === 1 ? "" : "s"} and no
        variables
      </p>
    );
  }
  return (
    <div className="exact-table-scroller exact-table-value-scroller">
      {rowCount === 0 ? (
        <p className="exact-table-empty-rows">No rows in this table slice</p>
      ) : null}
      <table className="exact-table-value-table">
        <caption>Exact table contents with named variables</caption>
        <thead>
          <tr>
            <th scope="col" aria-label="Row number">
              #
            </th>
            {value.variableNames.map((name) => (
              <th scope="col" title={name} key={name}>
                {name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {Array.from({ length: rowCount }, (_entry, rowOffset) => (
            <tr key={rowOffset}>
              <th scope="row">{(rowOffset + 1).toString()}</th>
              {value.variables.map((variable, variableOffset) => (
                <td key={value.variableNames[variableOffset]}>
                  <div className="exact-table-variable-cell-content">
                    <TableVariableCellValue
                      value={variable}
                      rowCount={rowCount}
                      rowOffset={rowOffset}
                    />
                  </div>
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ExactValueView({ value }: { readonly value: ExactValue }) {
  const shape = formatDimensions(value.size);
  return (
    <section
      className={`exact-value exact-value-${value.kind}`}
      aria-label={`${value.class} ${shape} exact value`}
    >
      <header className="exact-value-header">
        <strong>{value.class}</strong>
        <span>
          {shape} · {value.numel.toString()} element
          {value.numel === 1 ? "" : "s"}
          {value.complex ? " · complex" : ""}
        </span>
      </header>
      {value.kind === "cell" ? (
        value.items.length === 0 ? (
          <p className="exact-value-empty">Shaped empty cell {shape}</p>
        ) : (
          <ol className="aggregate-sequence">
            {value.items.map((item, index) => (
              <li key={index}>
                <span className="aggregate-index">
                  {`{${formatSubscript(index, value.size)}}`}
                </span>
                <ExactValueView value={item} />
              </li>
            ))}
          </ol>
        )
      ) : value.kind === "struct" ? (
        <div className="struct-value">
          <p className="struct-field-order">
            Field order: {value.fields.length === 0 ? "(none)" : value.fields.join(", ")}
          </p>
          {value.records.length === 0 ? (
            <p className="exact-value-empty">
              Shaped empty struct {shape} · schema retained
            </p>
          ) : (
            value.records.map((record, index) => (
              <details className="struct-record" open key={index}>
                <summary>
                  Record {formatSubscript(index, value.size)}
                </summary>
                <StructFields fields={value.fields} record={record} />
              </details>
            ))
          )}
        </div>
      ) : value.kind === "table" ? (
        <ExactTableValueView value={value} />
      ) : (
        <ExactLeafTable value={value} />
      )}
    </section>
  );
}

function TableAggregatePreviewGrid({
  variableName,
  preview,
}: {
  readonly variableName: string;
  readonly preview: TableAggregatePreview;
}) {
  const rowStart = preview.selectedRange.start[0];
  const variableStart = preview.selectedRange.start[1];
  const rowCount = preview.selectedRange.size[0];
  const variableCount = preview.selectedRange.size[1];
  const emptyDescription =
    rowCount === 0
      ? `${variableName} has no rows in this selection.`
      : variableCount === 0
        ? `${variableName} has ${rowCount.toString()} selected row${rowCount === 1 ? "" : "s"} and no variables.`
        : null;
  const columnHeaders = Array.from(
    { length: variableCount },
    (_entry, variableOffset) => preview.variableNames[variableOffset],
  );

  return (
    <div className="aggregate-preview table-aggregate-preview">
      <div
        className={`variable-editor-table-scroller${emptyDescription === null ? "" : " variable-editor-empty-grid"}`}
      >
        {emptyDescription === null ? null : (
          <div className="variable-editor-empty-note" role="status">
            {emptyDescription}
          </div>
        )}
        <VariableEditorGrid
          table
          caption={`Read-only table values for ${variableName}, rows ${rowStart.toString()} through ${rangeEnd(rowStart, rowCount).toString()}, variables ${variableStart.toString()} through ${rangeEnd(variableStart, variableCount).toString()}`}
          rowStart={rowStart}
          columnStart={variableStart}
          rowCount={rowCount}
          columnCount={variableCount}
          columnHeaders={columnHeaders}
          renderCell={(rowOffset, variableOffset) => {
            const tableVariable = preview.variables[variableOffset];
            if (tableVariable === undefined) {
              return (
                <td
                  className="variable-editor-table-variable-cell"
                  key={variableOffset}
                >
                  <span className="variable-editor-omitted">not returned</span>
                </td>
              );
            }
            const direct = isDirectTableColumn(tableVariable, rowCount);
            return (
              <td
                className={`variable-editor-table-variable-cell${direct ? "" : " has-exact-value"}`}
                key={variableOffset}
              >
                <div className="variable-editor-table-variable-cell-content">
                  <TableVariableCellValue
                    value={tableVariable}
                    rowCount={rowCount}
                    rowOffset={rowOffset}
                  />
                </div>
              </td>
            );
          }}
        />
      </div>
    </div>
  );
}

function AggregatePreviewTable({
  variableName,
  preview,
}: {
  readonly variableName: string;
  readonly preview: AggregatePreview;
}) {
  if (preview.kind === "table") {
    return (
      <TableAggregatePreviewGrid
        variableName={variableName}
        preview={preview}
      />
    );
  }
  const rowStart = preview.selectedRange.start[0] ?? 1;
  const columnStart = preview.selectedRange.start[1] ?? 1;
  const rowCount = preview.selectedRange.size[0] ?? 0;
  const columnCount = preview.selectedRange.size[1] ?? 0;
  const selectedCount = preview.selectedRange.size.reduce(
    (product, extent) => product * extent,
    1,
  );
  const returnedCount =
    preview.kind === "cell" ? preview.items.length : preview.records.length;

  return (
    <div className="aggregate-preview">
      {preview.kind === "struct" ? (
        <p className="struct-field-order root-schema">
          Field order: {preview.fields.length === 0 ? "(none)" : preview.fields.join(", ")}
        </p>
      ) : null}
      {selectedCount === 0 ? (
        <div className="variable-editor-message aggregate-empty" role="status">
          <strong>
            Shaped empty {preview.kind} {formatDimensions(preview.dimensions)}
          </strong>
          {preview.kind === "struct" ? <span>Complete field schema retained.</span> : null}
        </div>
      ) : (
        <div className="variable-editor-table-scroller">
          <VariableEditorGrid
            aggregate
            caption={`Read-only aggregate values for ${variableName}, rows ${rowStart.toString()} through ${rangeEnd(rowStart, rowCount).toString()}, columns ${columnStart.toString()} through ${rangeEnd(columnStart, columnCount).toString()}`}
            rowStart={rowStart}
            columnStart={columnStart}
            rowCount={rowCount}
            columnCount={columnCount}
            renderCell={(rowOffset, columnOffset) => {
              const valueIndex = rowOffset + rowCount * columnOffset;
              return (
                <td className="variable-editor-aggregate-cell" key={columnOffset}>
                  <div className="variable-editor-aggregate-cell-content">
                    {valueIndex >= returnedCount ? (
                      <span className="variable-editor-omitted">not returned</span>
                    ) : preview.kind === "cell" ? (
                      <ExactValueView value={preview.items[valueIndex]!} />
                    ) : (
                      <StructFields
                        fields={preview.fields}
                        record={preview.records[valueIndex]!}
                      />
                    )}
                  </div>
                </td>
              );
            }}
          />
        </div>
      )}
    </div>
  );
}

function rangeEnd(start: number, size: number): number {
  return size === 0 ? start - 1 : start + size - 1;
}

function rangesMatch(left: MatrixRange, right: MatrixRange): boolean {
  return (
    left.start.length === right.start.length &&
    left.start.every((value, index) => value === right.start[index]) &&
    left.size.every((value, index) => value === right.size[index])
  );
}

type VariableEditorContentProps = Omit<VariableEditorWindowProps, "onClose">;

const VariableEditorContent = memo(function VariableEditorContent({
  variable,
  range,
  preview,
  loading,
  error,
  revision = null,
  maxElements,
  canNavigate,
  canEdit = false,
  onRangeChange,
  onCellCommit,
  onReload,
}: VariableEditorContentProps) {
  const [activeEdit, setActiveEdit] = useState<ActiveCellEdit | null>(null);
  const [editText, setEditText] = useState("");
  const [saving, setSaving] = useState(false);
  const [editFailure, setEditFailure] = useState<{
    readonly conflict: boolean;
    readonly message: string;
  } | null>(null);
  const emptyArray = variable.dimensions.some((extent) => extent === 0);
  const displayedPreview =
    preview !== null && rangesMatch(preview.selectedRange, range)
      ? preview
      : null;
  const displayedAggregate =
    displayedPreview !== null && isAggregatePreview(displayedPreview)
      ? displayedPreview
      : null;
  const displayedTable =
    displayedAggregate !== null && displayedAggregate.kind === "table"
      ? displayedAggregate
      : null;
  const displayedMatrix: MatrixPreview | null =
    displayedPreview !== null && !isAggregatePreview(displayedPreview)
      ? displayedPreview
      : null;
  const selectedRange = displayedPreview?.selectedRange ?? range;
  const rowCount = selectedRange.size[0] ?? 0;
  const columnCount = selectedRange.size[1] ?? 0;
  const complex =
    displayedMatrix !== null && "complex" in displayedMatrix
      ? displayedMatrix.complex
      : variable.complex;
  const expectedValues =
    displayedTable === null
      ? selectedRange.size.reduce((product, extent) => product * extent, 1)
      : (selectedRange.size[1] ?? 0);
  const loadedValues =
    displayedPreview === null
      ? 0
      : displayedAggregate === null
        ? (displayedMatrix?.values.length ?? 0)
        : displayedAggregate.kind === "cell"
          ? displayedAggregate.items.length
          : displayedAggregate.kind === "struct"
            ? displayedAggregate.records.length
            : displayedAggregate.variables.length;

  const movePage = (axis: 0 | 1, direction: -1 | 1) => {
    const pageSpan = pageShape(variable.dimensions, maxElements)[axis];
    const nextStart = [...range.start];
    nextStart[axis] = (range.start[axis] ?? 1) + direction * pageSpan;
    onRangeChange(
      createVariableEditorRange(variable.dimensions, maxElements, nextStart),
    );
  };

  const moveSlice = (axis: number, direction: -1 | 1) => {
    const nextStart = [...range.start];
    nextStart[axis] = (range.start[axis] ?? 1) + direction;
    onRangeChange(
      createVariableEditorRange(variable.dimensions, maxElements, nextStart),
    );
  };

  const navigationDisabled = loading || !canNavigate;
  const rowStart = range.start[0] ?? 1;
  const columnStart = range.start[1] ?? 1;
  const rowSize = range.size[0] ?? 0;
  const columnSize = range.size[1] ?? 0;
  const rowExtent = variable.dimensions[0] ?? 0;
  const columnExtent = variable.dimensions[1] ?? 0;
  const variableAxis = variable.class === "table";
  const cellsEditable =
    canEdit &&
    revision !== null &&
    onCellCommit !== undefined &&
    EDITABLE_NUMERIC_CLASSES.has(variable.class);

  useEffect(() => {
    setActiveEdit(null);
    setEditText("");
    setSaving(false);
    setEditFailure(null);
  }, [range, variable.name]);

  const beginCellEdit = (
    rowOffset: number,
    columnOffset: number,
    text: string,
  ): void => {
    if (!cellsEditable || saving) {
      return;
    }
    const indices = range.start.map((start, axis) =>
      axis === 0
        ? start + rowOffset
        : axis === 1
          ? start + columnOffset
          : start,
    );
    setActiveEdit({
      key: indices.join(":"),
      indices,
      originalText: text,
    });
    setEditText(text);
    setEditFailure(null);
  };

  const commitCellEdit = async (): Promise<void> => {
    if (activeEdit === null || onCellCommit === undefined || saving) {
      return;
    }
    const value = parseVariableEditorNumericInput(
      editText,
      variable.class,
      variable.complex || variable.class === "double" || variable.class === "single",
    );
    if (value === null) {
      setEditFailure({
        conflict: false,
        message: `Enter a numeric value such as 12, -3.5, Inf, or 2+4i.`,
      });
      return;
    }
    setSaving(true);
    setEditFailure(null);
    let result: VariableEditorCommitResult;
    try {
      result = await onCellCommit({ indices: activeEdit.indices, value });
    } catch (error: unknown) {
      result = {
        ok: false,
        conflict: false,
        message:
          error instanceof Error ? error.message : "The variable edit failed.",
      };
    } finally {
      setSaving(false);
    }
    if (result.ok) {
      setActiveEdit(null);
      setEditText("");
      return;
    }
    setEditFailure({ conflict: result.conflict, message: result.message });
  };

  const reloadLatest = (): void => {
    setActiveEdit(null);
    setEditText("");
    setEditFailure(null);
    onReload?.();
  };

  return (
    <div className="variable-editor-app">
      <div className="variable-editor-toolbar" aria-label="Matrix navigation">
        <div className="variable-editor-page-control">
          <span>
            Rows {rowSize === 0 ? "0" : `${rowStart.toString()}–${rangeEnd(rowStart, rowSize).toString()}`} of{" "}
            {rowExtent.toString()}
          </span>
          <button
            className="button button-secondary"
            type="button"
            aria-label="Previous row page"
            disabled={navigationDisabled || rowStart <= 1 || rowSize === 0}
            onClick={() => movePage(0, -1)}
          >
            ‹
          </button>
          <button
            className="button button-secondary"
            type="button"
            aria-label="Next row page"
            disabled={
              navigationDisabled ||
              rowSize === 0 ||
              rowStart + rowSize > rowExtent
            }
            onClick={() => movePage(0, 1)}
          >
            ›
          </button>
        </div>
        <div className="variable-editor-page-control">
          <span>
            {variableAxis ? "Variables" : "Columns"} {columnSize === 0 ? "0" : `${columnStart.toString()}–${rangeEnd(columnStart, columnSize).toString()}`} of{" "}
            {columnExtent.toString()}
          </span>
          <button
            className="button button-secondary"
            type="button"
            aria-label={`Previous ${variableAxis ? "variable" : "column"} page`}
            disabled={navigationDisabled || columnStart <= 1 || columnSize === 0}
            onClick={() => movePage(1, -1)}
          >
            ‹
          </button>
          <button
            className="button button-secondary"
            type="button"
            aria-label={`Next ${variableAxis ? "variable" : "column"} page`}
            disabled={
              navigationDisabled ||
              columnSize === 0 ||
              columnStart + columnSize > columnExtent
            }
            onClick={() => movePage(1, 1)}
          >
            ›
          </button>
        </div>
        {variable.dimensions.slice(2).map((extent, relativeAxis) => {
          const axis = relativeAxis + 2;
          const position = range.start[axis] ?? 1;
          return (
            <div className="variable-editor-page-control" key={axis}>
              <span>
                Dim {axis + 1}: {extent === 0 ? "0" : position.toString()} of{" "}
                {extent.toString()}
              </span>
              <button
                className="button button-secondary"
                type="button"
                aria-label={`Previous slice in dimension ${(axis + 1).toString()}`}
                disabled={navigationDisabled || extent === 0 || position <= 1}
                onClick={() => moveSlice(axis, -1)}
              >
                ‹
              </button>
              <button
                className="button button-secondary"
                type="button"
                aria-label={`Next slice in dimension ${(axis + 1).toString()}`}
                disabled={navigationDisabled || extent === 0 || position >= extent}
                onClick={() => moveSlice(axis, 1)}
              >
                ›
              </button>
            </div>
          );
        })}
        {editFailure !== null ? (
          <div className="variable-editor-edit-failure" role="alert">
            <span>{editFailure.message}</span>
            {editFailure.conflict && onReload !== undefined ? (
              <button
                className="button button-secondary"
                type="button"
                onClick={reloadLatest}
              >
                Reload latest
              </button>
            ) : null}
          </div>
        ) : null}
      </div>

      <div className="variable-editor-body">
        {error !== null ? (
          <div className="variable-editor-message error" role="alert">
            <strong>Could not load {variable.name}</strong>
            <span>{error}</span>
          </div>
        ) : loading ? (
          <div className="variable-editor-message" role="status" aria-live="polite">
            Loading {variable.name}…
          </div>
        ) : displayedAggregate !== null ? (
          <AggregatePreviewTable
            variableName={variable.name}
            preview={displayedAggregate}
          />
        ) : emptyArray ? (
          <div className="variable-editor-table-scroller variable-editor-empty-grid">
            <div className="variable-editor-empty-note" role="status">
              {variable.name} is an empty {formatDimensions(variable.dimensions)} array.
            </div>
            <VariableEditorGrid
              caption={`Read-only empty grid for ${variable.name}`}
              rowStart={rowStart}
              columnStart={columnStart}
              rowCount={rowCount}
              columnCount={columnCount}
              renderCell={(rowOffset, columnOffset) => (
                <FillerCell key={`${rowOffset.toString()}-${columnOffset.toString()}`} />
              )}
            />
          </div>
        ) : displayedMatrix === null ? (
          <div className="variable-editor-message" role="status">
            No values are available for this range.
          </div>
        ) : (
          <div className="variable-editor-table-scroller">
            <VariableEditorGrid
              caption={`${cellsEditable ? "Editable" : "Read-only"} values for ${variable.name}, rows ${rowStart.toString()} through ${rangeEnd(rowStart, rowCount).toString()}, columns ${columnStart.toString()} through ${rangeEnd(columnStart, columnCount).toString()}`}
              rowStart={rowStart}
              columnStart={columnStart}
              rowCount={rowCount}
              columnCount={columnCount}
              renderCell={(rowOffset, columnOffset) => {
                const valueIndex = rowOffset + rowCount * columnOffset;
                const value = displayedMatrix.values[valueIndex];
                if (value === undefined) {
                  return (
                    <td key={columnOffset}>
                      <span className="variable-editor-omitted">not returned</span>
                    </td>
                  );
                }
                const text = formatPreviewValue(value, complex);
                const indices = range.start.map((start, axis) =>
                  axis === 0
                    ? start + rowOffset
                    : axis === 1
                      ? start + columnOffset
                      : start,
                );
                const editKey = indices.join(":");
                return (
                  <td
                    className={cellsEditable ? "variable-editor-editable-cell" : undefined}
                    key={columnOffset}
                    onDoubleClick={() => beginCellEdit(rowOffset, columnOffset, text)}
                  >
                    {activeEdit?.key === editKey ? (
                      <input
                        className="variable-editor-cell-input"
                        aria-label={`Edit ${variable.name}(${indices.join(", ")})`}
                        autoFocus
                        disabled={saving}
                        value={editText}
                        onChange={(event) => setEditText(event.target.value)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter") {
                            event.preventDefault();
                            void commitCellEdit();
                          } else if (event.key === "Escape") {
                            event.preventDefault();
                            event.stopPropagation();
                            setActiveEdit(null);
                            setEditText(activeEdit.originalText);
                            setEditFailure(null);
                          }
                        }}
                      />
                    ) : (
                      <span
                        className="variable-editor-cell-value"
                        title={cellsEditable ? `${text} · double-click to edit` : text}
                        aria-label={text}
                      >
                        {text}
                      </span>
                    )}
                  </td>
                );
              }}
            />
          </div>
        )}
      </div>

      <footer className="variable-editor-status" aria-live="polite">
        {displayedPreview === null ? (
          <span>{loading ? "Requesting bounded range…" : "No range loaded"}</span>
        ) : displayedPreview.truncation.truncated ? (
          <span className="truncated">
            Loaded {loadedValues.toString()} of {expectedValues.toString()}{" "}
            {displayedTable === null ? "elements" : "variables"} · truncated ·{" "}
            {displayedPreview.truncation.omittedElements.toString()}
            {displayedTable === null
              ? " omitted by kernel limits"
              : ` variable${displayedPreview.truncation.omittedElements === 1 ? "" : "s"} omitted by kernel limits`}
          </span>
        ) : (
          <span>
            Loaded {loadedValues.toString()} of {expectedValues.toString()}{" "}
            {displayedTable === null ? "elements" : "variables"} · complete
            {isAggregatePreview(displayedPreview)
              ? ` · ${displayedPreview.usage.nodes.toString()} exact nodes`
              : ""}
          </span>
        )}
        <span>
          {revision === null ? "unversioned" : `revision ${revision.toString()}`} · Esc closes
        </span>
      </footer>
    </div>
  );
});

export const VARIABLE_EDITOR_APP_ID = "openmat.variable-editor";

type VariableEditorAppPayload = VariableEditorContentProps;

export const variableEditorAppDefinition: OpenMatAppDefinition<VariableEditorAppPayload> = {
  id: VARIABLE_EDITOR_APP_ID,
  displayName: "Variable Editor",
  defaultSize: { width: 920, height: 620 },
  minimumSize: { width: 480, height: 320 },
  component: ({ payload }) => <VariableEditorContent {...payload} />,
};

export function variableEditorWindowId(variableName: string): string {
  return `variable-editor:${encodeURIComponent(variableName)}`;
}

export function VariableEditorWindow(props: VariableEditorWindowProps) {
  const windowManager = useOpenMatWindowManagerActions();
  const id = variableEditorWindowId(props.variable.name);
  const editable =
    props.canEdit === true &&
    EDITABLE_NUMERIC_CLASSES.has(props.variable.class);
  const description = `${props.variable.class}${props.variable.complex ? " complex" : ""} · ${formatDimensions(props.variable.dimensions)} · ${editable ? "editable numeric cells" : "read-only"}`;
  const onRangeChangeRef = useRef(props.onRangeChange);
  const onCellCommitRef = useRef(props.onCellCommit);
  const onReloadRef = useRef(props.onReload);
  const onCloseRef = useRef(props.onClose);
  onRangeChangeRef.current = props.onRangeChange;
  onCellCommitRef.current = props.onCellCommit;
  onReloadRef.current = props.onReload;
  onCloseRef.current = props.onClose;
  const handleRangeChange = useCallback(
    (range: MatrixRange) => onRangeChangeRef.current(range),
    [],
  );
  const handleCellCommit = useCallback(
    (edit: VariableEditorCellEdit) =>
      onCellCommitRef.current?.(edit) ??
      Promise.resolve({
        ok: false,
        conflict: false,
        message: "Variable editing is unavailable.",
      } as const),
    [],
  );
  const handleReload = useCallback(() => onReloadRef.current?.(), []);
  const payload = useMemo<VariableEditorAppPayload>(
    () => ({
      variable: props.variable,
      range: props.range,
      preview: props.preview,
      loading: props.loading,
      error: props.error,
      revision: props.revision ?? null,
      maxElements: props.maxElements,
      canNavigate: props.canNavigate,
      canEdit: props.canEdit ?? false,
      onRangeChange: handleRangeChange,
      onCellCommit: handleCellCommit,
      onReload: handleReload,
    }),
    [
      props.canNavigate,
      props.error,
      props.loading,
      props.maxElements,
      props.preview,
      props.range,
      props.revision,
      props.variable,
      props.canEdit,
      handleCellCommit,
      handleRangeChange,
      handleReload,
    ],
  );

  useEffect(() => {
    windowManager.openWindow({
      id,
      appId: VARIABLE_EDITOR_APP_ID,
      title: `Variable Editor: ${props.variable.name}`,
      ariaDescription: description,
      closeButtonLabel: `Close Variable Editor for ${props.variable.name}`,
      closeOnEscape: true,
      initialFocus: "close",
      payload,
      onCloseRequested: () => {
        onCloseRef.current();
        return "close";
      },
    });
  }, [description, id, payload, props.variable.name, windowManager]);

  useEffect(
    () => () => windowManager.closeWindow(id),
    [id, windowManager],
  );

  return null;
}
