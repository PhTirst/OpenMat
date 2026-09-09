import { useMemo, useState, type KeyboardEvent } from "react";
import type { VariableSummary } from "../protocol/kernel-v0";

type WorkspaceSortKey = "name" | "size" | "type";

interface WorkspacePaneProps {
  readonly variables: readonly VariableSummary[];
  readonly selectedName: string | null;
  readonly canOpen: boolean;
  readonly canClear: boolean;
  readonly onSelect: (variable: VariableSummary) => void;
  readonly onOpen: (variable: VariableSummary) => void;
  readonly onClear: (variable: VariableSummary) => void;
  readonly onClearAll: () => void;
}

function formatDimensions(dimensions: readonly number[]): string {
  return dimensions.join(" × ");
}

function formatBytes(bytes: number | undefined): string {
  if (bytes === undefined) {
    return "size unavailable";
  }
  return `${bytes.toLocaleString()} bytes`;
}

function elementCount(dimensions: readonly number[]): number {
  return dimensions.reduce((product, extent) => product * extent, 1);
}

function sizeOf(variable: VariableSummary): number {
  return variable.bytes ?? elementCount(variable.dimensions);
}

function compareVariables(
  left: VariableSummary,
  right: VariableSummary,
  sortKey: WorkspaceSortKey,
): number {
  if (sortKey === "size") {
    return sizeOf(left) - sizeOf(right) || left.name.localeCompare(right.name);
  }
  if (sortKey === "type") {
    return (
      left.class.localeCompare(right.class) || left.name.localeCompare(right.name)
    );
  }
  return left.name.localeCompare(right.name);
}

export function WorkspacePane({
  variables,
  selectedName,
  canOpen,
  canClear,
  onSelect,
  onOpen,
  onClear,
  onClearAll,
}: WorkspacePaneProps) {
  const [filter, setFilter] = useState("");
  const [sortKey, setSortKey] = useState<WorkspaceSortKey>("name");
  const [descending, setDescending] = useState(false);
  const selectedVariable = variables.find(
    (variable) => variable.name === selectedName,
  );
  const visibleVariables = useMemo(() => {
    const query = filter.trim().toLocaleLowerCase();
    const filtered =
      query.length === 0
        ? variables
        : variables.filter((variable) =>
            [
              variable.name,
              variable.class,
              formatDimensions(variable.dimensions),
              formatBytes(variable.bytes),
            ].some((value) => value.toLocaleLowerCase().includes(query)),
          );
    return [...filtered].sort((left, right) => {
      const compared = compareVariables(left, right, sortKey);
      return descending ? -compared : compared;
    });
  }, [descending, filter, sortKey, variables]);

  const handleRowKeyDown = (
    event: KeyboardEvent<HTMLTableRowElement>,
    variable: VariableSummary,
  ) => {
    if (event.key === " ") {
      event.preventDefault();
      onSelect(variable);
    } else if (event.key === "Enter" && canOpen) {
      event.preventDefault();
      onOpen(variable);
    }
  };

  return (
    <section className="pane workspace-pane" aria-labelledby="workspace-title">
      <header className="pane-header workspace-header">
        <div>
          <span id="workspace-title" className="pane-title">
            Workspace
          </span>
          <span className="pane-meta">kernel summaries</span>
        </div>
        <div className="workspace-actions">
          <button
            className="button button-secondary workspace-open-button"
            type="button"
            aria-label={
              selectedVariable === undefined
                ? "Open selected variable"
                : `Open ${selectedVariable.name} in Variable Editor`
            }
            disabled={selectedVariable === undefined || !canOpen}
            onClick={() => {
              if (selectedVariable !== undefined) {
                onOpen(selectedVariable);
              }
            }}
          >
            Open
          </button>
          <button
            className="button button-secondary workspace-clear-button"
            type="button"
            aria-label={
              selectedVariable === undefined
                ? "Clear selected variable"
                : `Clear ${selectedVariable.name}`
            }
            disabled={selectedVariable === undefined || !canClear}
            onClick={() => {
              if (selectedVariable !== undefined) {
                onClear(selectedVariable);
              }
            }}
          >
            Clear
          </button>
          <button
            className="button button-secondary workspace-clear-all-button"
            type="button"
            disabled={variables.length === 0 || !canClear}
            onClick={onClearAll}
          >
            Clear all
          </button>
        </div>
      </header>
      <div className="workspace-tools">
        <label className="workspace-filter">
          <span className="sr-only">Filter workspace variables</span>
          <input
            type="search"
            value={filter}
            placeholder="Filter variables"
            aria-label="Filter workspace variables"
            onChange={(event) => setFilter(event.target.value)}
          />
        </label>
        <label className="workspace-sort">
          <span className="sr-only">Sort workspace variables</span>
          <select
            value={sortKey}
            aria-label="Sort workspace variables"
            onChange={(event) =>
              setSortKey(event.target.value as WorkspaceSortKey)
            }
          >
            <option value="name">Name</option>
            <option value="size">Size</option>
            <option value="type">Type</option>
          </select>
        </label>
        <button
          className="button button-secondary workspace-sort-direction"
          type="button"
          aria-label={descending ? "Sort ascending" : "Sort descending"}
          title={descending ? "Sort ascending" : "Sort descending"}
          onClick={() => setDescending((current) => !current)}
        >
          {descending ? "↓" : "↑"}
        </button>
      </div>
      {variables.length === 0 ? (
        <div className="empty-state">
          <span className="empty-symbol" aria-hidden="true">
            ∅
          </span>
          <strong>No variables yet</strong>
          <span>Kernel workspace summaries appear after execution.</span>
        </div>
      ) : visibleVariables.length === 0 ? (
        <div className="empty-state workspace-filter-empty" role="status">
          <span className="empty-symbol" aria-hidden="true">
            ∅
          </span>
          <strong>No matching variables</strong>
          <span>Try a different name, type, shape, or storage size.</span>
        </div>
      ) : (
        <div className="table-scroller">
          <table
            className="workspace-table"
            role="grid"
            aria-label="Kernel workspace summaries"
          >
            <caption className="sr-only">
              Select a variable, then use Open or double-click a row to view its
              bounded values in the Variable Editor.
            </caption>
            <thead>
              <tr>
                <th scope="col">Name</th>
                <th scope="col">Shape</th>
                <th scope="col">Storage</th>
              </tr>
            </thead>
            <tbody>
              {visibleVariables.map((variable) => {
                const selected = variable.name === selectedName;
                return (
                  <tr
                    className={selected ? "selected" : undefined}
                    key={variable.name}
                    tabIndex={0}
                    aria-selected={selected}
                    onClick={() => onSelect(variable)}
                    onDoubleClick={() => {
                      if (canOpen) {
                        onOpen(variable);
                      }
                    }}
                    onKeyDown={(event) => handleRowKeyDown(event, variable)}
                  >
                    <th scope="row">
                      <span className="variable-name">{variable.name}</span>
                      <span className="variable-class">{variable.class}</span>
                    </th>
                    <td>{formatDimensions(variable.dimensions)}</td>
                    <td>
                      <span className="variable-summary">
                        {variable.complex ? "complex · " : ""}
                        {formatBytes(variable.bytes)}
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
