import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CommandWindowPane } from "./CommandWindowPane";

describe("CommandWindowPane", () => {
  it("submits commands and recalls history with arrow keys", () => {
    const onCommandChange = vi.fn();
    const onSubmit = vi.fn();
    render(
      <CommandWindowPane
        entries={[]}
        command="x = 1"
        history={["a = 10", "b = 20"]}
        enabled
        canSubmit
        busy={false}
        onCommandChange={onCommandChange}
        onSubmit={onSubmit}
        onClear={() => undefined}
      />,
    );

    const input = screen.getByRole("textbox", { name: "Command Window input" });
    expect(input.closest(".command-transcript")).not.toBeNull();
    expect(input).not.toHaveAttribute("placeholder");
    expect(
      screen.queryByText(/Enter a MATLAB command below/),
    ).not.toBeInTheDocument();
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(onCommandChange).toHaveBeenLastCalledWith("b = 20");

    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSubmit).toHaveBeenCalledWith("x = 1");
  });
});
