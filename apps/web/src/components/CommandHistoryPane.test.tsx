import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CommandHistoryPane } from "./CommandHistoryPane";

describe("CommandHistoryPane", () => {
  it("recalls on click and executes on double click", () => {
    const onRecall = vi.fn();
    const onExecute = vi.fn();
    render(
      <CommandHistoryPane
        commands={["disp(answer)"]}
        onRecall={onRecall}
        onExecute={onExecute}
      />,
    );

    const command = screen.getByRole("button", { name: "disp(answer)" });
    fireEvent.click(command);
    expect(onRecall).toHaveBeenCalledWith("disp(answer)");

    fireEvent.doubleClick(command);
    expect(onExecute).toHaveBeenCalledWith("disp(answer)");
  });
});
