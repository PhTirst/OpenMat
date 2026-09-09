import { fireEvent, render, screen, within } from "@testing-library/react";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import type { VariableSummary } from "../protocol/kernel-v0";
import { WorkspacePane } from "./WorkspacePane";

const variables: readonly VariableSummary[] = [
  {
    name: "zeta",
    class: "single",
    dimensions: [2, 2],
    complex: false,
    bytes: 16,
  },
  {
    name: "alpha",
    class: "double",
    dimensions: [1, 10],
    complex: false,
    bytes: 80,
  },
  {
    name: "beta",
    class: "logical",
    dimensions: [1, 2],
    complex: false,
    bytes: 2,
  },
];

function renderPane(overrides: Partial<ComponentProps<typeof WorkspacePane>> = {}) {
  const props: ComponentProps<typeof WorkspacePane> = {
    variables,
    selectedName: "alpha",
    canOpen: true,
    canClear: true,
    onSelect: vi.fn(),
    onOpen: vi.fn(),
    onClear: vi.fn(),
    onClearAll: vi.fn(),
    ...overrides,
  };
  render(<WorkspacePane {...props} />);
  return props;
}

function visibleNames(): string[] {
  return within(screen.getByRole("grid"))
    .getAllByRole("row")
    .slice(1)
    .map((row) => within(row).getByRole("rowheader").textContent ?? "");
}

describe("WorkspacePane", () => {
  it("filters by name or type and reports an empty match", () => {
    renderPane();
    const filter = screen.getByRole("searchbox", {
      name: "Filter workspace variables",
    });

    fireEvent.change(filter, { target: { value: "logical" } });
    expect(visibleNames()).toEqual(["betalogical"]);

    fireEvent.change(filter, { target: { value: "missing" } });
    expect(screen.getByText("No matching variables")).toBeVisible();
  });

  it("sorts by size and reverses the active order", () => {
    renderPane();
    fireEvent.change(
      screen.getByRole("combobox", { name: "Sort workspace variables" }),
      { target: { value: "size" } },
    );
    expect(visibleNames()).toEqual(["betalogical", "zetasingle", "alphadouble"]);

    fireEvent.click(screen.getByRole("button", { name: "Sort descending" }));
    expect(visibleNames()).toEqual(["alphadouble", "zetasingle", "betalogical"]);
  });

  it("clears the selected variable or the complete workspace", () => {
    const onClear = vi.fn();
    const onClearAll = vi.fn();
    renderPane({ onClear, onClearAll });

    fireEvent.click(screen.getByRole("button", { name: "Clear alpha" }));
    expect(onClear).toHaveBeenCalledWith(variables[1]);

    fireEvent.click(screen.getByRole("button", { name: "Clear all" }));
    expect(onClearAll).toHaveBeenCalledOnce();
  });
});
