import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { CurrentFolderAddressBar } from "./CurrentFolderAddressBar";

function AddressHarness({ reject = "" }: { readonly reject?: string }) {
  const [path, setPath] = useState("C:\\first");
  return (
    <CurrentFolderAddressBar
      path={path}
      busy={false}
      disabled={false}
      onNavigate={async (requested) => {
        if (requested === reject) {
          return null;
        }
        setPath(requested);
        return requested;
      }}
      onChoose={() => undefined}
      onParent={() => undefined}
    />
  );
}

describe("CurrentFolderAddressBar", () => {
  it("edits absolute paths and preserves back and forward history", async () => {
    render(<AddressHarness />);
    const input = screen.getByRole("textbox", { name: "Current Folder path" });
    fireEvent.change(input, { target: { value: "D:\\work" } });
    fireEvent.submit(input.closest("form")!);
    await waitFor(() => expect(input).toHaveValue("D:\\work"));

    fireEvent.click(screen.getByRole("button", { name: "Back to previous folder" }));
    await waitFor(() => expect(input).toHaveValue("C:\\first"));
    fireEvent.click(screen.getByRole("button", { name: "Forward to next folder" }));
    await waitFor(() => expect(input).toHaveValue("D:\\work"));
  });

  it("restores the authoritative path after a rejected navigation", async () => {
    render(<AddressHarness reject={"Z:\\missing"} />);
    const input = screen.getByRole("textbox", { name: "Current Folder path" });
    fireEvent.change(input, { target: { value: "Z:\\missing" } });
    fireEvent.submit(input.closest("form")!);
    await waitFor(() => expect(input).toHaveValue("C:\\first"));
  });

  it("offers parent and in-app folder chooser icon actions", () => {
    const onChoose = vi.fn();
    const onParent = vi.fn();
    render(
      <CurrentFolderAddressBar
        path="C:\\first"
        busy={false}
        disabled={false}
        onNavigate={async () => "C:\\first"}
        onChoose={onChoose}
        onParent={onParent}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open parent folder" }));
    fireEvent.click(screen.getByRole("button", { name: "Choose Current Folder" }));
    expect(onParent).toHaveBeenCalledOnce();
    expect(onChoose).toHaveBeenCalledOnce();
  });
});
