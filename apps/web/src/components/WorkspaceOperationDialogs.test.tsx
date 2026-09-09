import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { OpenDocument } from "../documents/document-session";
import type { WorkspaceEntry } from "../workspace/workspace-client";
import {
  DeleteEntryDialog,
  MoveEntryDialog,
  RenameEntryDialog,
  SaveConflictDialog,
  SaveCopyDialog,
  UnsavedChangesDialog,
  validateWorkspaceEntryName,
} from "./WorkspaceOperationDialogs";

const FILE_ENTRY: WorkspaceEntry = {
  name: "seed.m",
  path: "seed.m",
  kind: "file",
  size: 10,
  revision: "revision-1",
};

const DOCUMENT: OpenDocument = {
  id: "seed",
  path: "seed.m",
  rootPath: "C:\\workspace",
  rootGeneration: 1,
  uri: "openmat-workspace://root-1/seed.m",
  content: "local = 1;\n",
  savedContent: "seed = 1;\n",
  revision: "revision-1",
  version: 2,
  recoveryStatus: "none",
  recoveryMessage: null,
  viewState: null,
};

describe("Workspace operation dialogs", () => {
  it("validates workspace names with the same Windows-compatible boundary as the server", () => {
    expect(validateWorkspaceEntryName("", null, [])).toMatch(/Enter/u);
    expect(validateWorkspaceEntryName("nested/name.m", null, [])).toMatch(
      /one name/u,
    );
    expect(validateWorkspaceEntryName("CON.txt", null, [])).toMatch(/reserved/u);
    expect(validateWorkspaceEntryName("bad?.m", null, [])).toMatch(/Windows/u);
    expect(validateWorkspaceEntryName("taken.m", null, ["Taken.m"])).toMatch(
      /already exists/u,
    );
    expect(validateWorkspaceEntryName("valid.m", null, ["other.m"])).toBeNull();
  });

  it("keeps rename open for immediate and server validation, then completes", async () => {
    const onRename = vi
      .fn<(name: string) => Promise<string | null>>()
      .mockResolvedValueOnce("The server rejected this name.")
      .mockResolvedValueOnce(null);
    const onComplete = vi.fn();
    render(
      <RenameEntryDialog
        entry={FILE_ENTRY}
        siblingNames={["taken.m"]}
        onRename={onRename}
        onComplete={onComplete}
        onCancel={() => undefined}
      />,
    );
    const input = screen.getByRole("textbox", { name: "New name" });
    fireEvent.change(input, { target: { value: "CON" } });
    expect(screen.getByRole("button", { name: "Rename" })).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("reserved");

    fireEvent.change(input, { target: { value: "renamed.m" } });
    fireEvent.submit(input.closest("form")!);
    expect(await screen.findByRole("alert")).toHaveTextContent("server rejected");
    expect(onComplete).not.toHaveBeenCalled();
    fireEvent.submit(input.closest("form")!);
    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    expect(onRename).toHaveBeenLastCalledWith("renamed.m");
  });

  it("moves by selecting a destination folder instead of typing a path", async () => {
    const onMove = vi.fn<(path: string) => Promise<string | null>>().mockResolvedValue(null);
    const onComplete = vi.fn();
    render(
      <MoveEntryDialog
        entry={FILE_ENTRY}
        rootName="workspace"
        loadDirectories={async (path) =>
          path === "" ? [{ name: "src", path: "src" }] : []
        }
        onMove={onMove}
        onComplete={onComplete}
        onCancel={() => undefined}
      />,
    );
    expect(screen.getByRole("button", { name: "Move" })).toBeDisabled();
    fireEvent.click(await screen.findByRole("button", { name: "src" }));
    expect(screen.getByText("src/seed.m")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Move" }));
    await waitFor(() => expect(onMove).toHaveBeenCalledWith("src/seed.m"));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it("counts delete impact and keeps the destructive action away from initial focus", async () => {
    const onDelete = vi.fn<() => Promise<string | null>>().mockResolvedValue(null);
    render(
      <DeleteEntryDialog
        entry={{ ...FILE_ENTRY, name: "src", path: "src", kind: "directory" }}
        dirtyDocumentCount={2}
        loadImpact={async () => ({ files: 18, directories: 4 })}
        onDelete={onDelete}
        onComplete={() => undefined}
        onCancel={() => undefined}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Counting");
    expect(await screen.findByText("18")).toBeVisible();
    expect(screen.getByText("4")).toBeVisible();
    expect(screen.getByText(/Unsaved changes inside this target/u)).toBeVisible();
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveAttribute(
      "data-dialog-initial-focus",
    );
    fireEvent.click(screen.getByRole("button", { name: "Permanently Delete" }));
    await waitFor(() => expect(onDelete).toHaveBeenCalledTimes(1));
  });

  it("exposes explicit unsaved and save-conflict decisions", () => {
    const unsaved = vi.fn();
    const first = render(
      <UnsavedChangesDialog document={DOCUMENT} onDecision={unsaved} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Don’t Save" }));
    expect(unsaved).toHaveBeenCalledWith("discard");
    first.unmount();

    const conflict = vi.fn();
    render(
      <SaveConflictDialog
        document={DOCUMENT}
        message="revision conflict"
        onDecision={conflict}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Reload from Disk" }));
    fireEvent.click(screen.getByRole("button", { name: "Save a Copy…" }));
    fireEvent.click(screen.getByRole("button", { name: "Keep Editing" }));
    expect(conflict.mock.calls.map(([decision]) => decision)).toEqual([
      "reload",
      "saveCopy",
      "keep",
    ]);
  });

  it("validates and saves a copy beside the original", async () => {
    const onSave = vi.fn<(path: string) => Promise<string | null>>().mockResolvedValue(null);
    const onComplete = vi.fn();
    render(
      <SaveCopyDialog
        document={DOCUMENT}
        siblingNames={["seed.m"]}
        onSave={onSave}
        onComplete={onComplete}
        onCancel={() => undefined}
      />,
    );
    const input = screen.getByRole("textbox", { name: "Copy name" });
    expect(input).toHaveValue("seed copy.m");
    fireEvent.submit(input.closest("form")!);
    await waitFor(() => expect(onSave).toHaveBeenCalledWith("seed copy.m"));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });
});
