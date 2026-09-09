import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { WorkspaceEntry } from "../workspace/workspace-client";
import {
  CurrentFolderPane,
  fitContextMenuToViewport,
} from "./CurrentFolderPane";

const entries: readonly WorkspaceEntry[] = [
  {
    name: "src",
    path: "src",
    kind: "directory",
    size: null,
    revision: null,
  },
  {
    name: "nested.m",
    path: "src/nested.m",
    kind: "file",
    size: 10,
    revision: null,
  },
  {
    name: "root.m",
    path: "root.m",
    kind: "file",
    size: 8,
    revision: null,
  },
];

function TreeHarness({
  onRename = () => undefined,
  onMove = () => undefined,
  onDelete = () => undefined,
  onStartCreate = () => undefined,
  onEnterDirectory = () => undefined,
  onRefresh = () => undefined,
  searchPathWorkspacePaths = new Set(),
  onAddSearchPath = () => undefined,
  onRemoveSearchPath = () => undefined,
  onDownload = () => undefined,
  onUpload = () => undefined,
  onCopyRelativePath = async () => undefined,
}: {
  readonly onRename?: (entry: WorkspaceEntry) => void;
  readonly onMove?: (entry: WorkspaceEntry) => void;
  readonly onDelete?: (entry: WorkspaceEntry) => void;
  readonly onStartCreate?: (kind: "file" | "directory", parent: string) => void;
  readonly onEnterDirectory?: (path: string) => void;
  readonly onRefresh?: () => void;
  readonly searchPathWorkspacePaths?: ReadonlySet<string>;
  readonly onAddSearchPath?: (entry: WorkspaceEntry, recursive: boolean) => void;
  readonly onRemoveSearchPath?: (entry: WorkspaceEntry, recursive: boolean) => void;
  readonly onDownload?: (entry: WorkspaceEntry) => void;
  readonly onUpload?: (files: readonly File[], parentPath: string) => void;
  readonly onCopyRelativePath?: (entry: WorkspaceEntry) => Promise<void>;
}) {
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [expandedPaths, setExpandedPaths] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  return (
    <CurrentFolderPane
      openEditors={[]}
      activeDocumentId={null}
      entries={entries}
      rootName="project"
      rootPath="C:\\project"
      serviceLabel="test workspace"
      selectedPath={selectedPath}
      expandedPaths={expandedPaths}
      searchPathWorkspacePaths={searchPathWorkspacePaths}
      loading={false}
      refreshing={false}
      canMutate
      creatingKind={null}
      creatingParentPath=""
      operationBusy={false}
      uploadStatus={null}
      error={null}
      onActivateDocument={() => undefined}
      onCloseDocument={() => undefined}
      onToggleDirectory={(path) =>
        setExpandedPaths((current) => {
          const next = new Set(current);
          if (next.has(path)) {
            next.delete(path);
          } else {
            next.add(path);
          }
          return next;
        })
      }
      onRefresh={onRefresh}
      onStartCreate={onStartCreate}
      onCancelCreate={() => undefined}
      onCreate={() => undefined}
      onFocus={(entry) => setSelectedPath(entry.path)}
      onSelect={(entry) => {
        setSelectedPath(entry.path);
        if (entry.kind === "directory") {
          setExpandedPaths((current) => {
            const next = new Set(current);
            if (next.has(entry.path)) {
              next.delete(entry.path);
            } else {
              next.add(entry.path);
            }
            return next;
          });
        }
      }}
      onEnterDirectory={onEnterDirectory}
      onAddSearchPath={onAddSearchPath}
      onRemoveSearchPath={onRemoveSearchPath}
      onDownload={onDownload}
      onUpload={onUpload}
      onCopyRelativePath={onCopyRelativePath}
      onRename={onRename}
      onMove={onMove}
      onDelete={onDelete}
    />
  );
}

describe("CurrentFolderPane", () => {
  it("exposes an expandable keyboard-accessible ARIA tree and preserves selection", () => {
    render(<TreeHarness />);
    const directory = screen.getByRole("treeitem", { name: "src" });
    expect(directory).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("treeitem", { name: "nested.m" })).toBeNull();

    fireEvent.keyDown(directory, { key: "ArrowRight" });
    expect(directory).toHaveAttribute("aria-expanded", "true");
    const nestedFile = screen.getByRole("treeitem", { name: "nested.m" });
    expect(nestedFile).toBeVisible();
    directory.focus();
    fireEvent.keyDown(directory, { key: "ArrowDown" });
    expect(nestedFile).toHaveFocus();
    fireEvent.click(nestedFile);
    expect(screen.getByRole("treeitem", { name: "nested.m" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("offers context and keyboard mutation actions with the selected parent", () => {
    const onRename = vi.fn();
    const onDelete = vi.fn();
    const onStartCreate = vi.fn();
    render(
      <TreeHarness
        onRename={onRename}
        onDelete={onDelete}
        onStartCreate={onStartCreate}
      />,
    );
    const rootFile = screen.getByRole("treeitem", { name: "root.m" });
    fireEvent.keyDown(rootFile, { key: "F2" });
    expect(onRename).toHaveBeenCalledWith(expect.objectContaining({ path: "root.m" }));
    fireEvent.keyDown(rootFile, { key: "Delete" });
    expect(onDelete).toHaveBeenCalledWith(expect.objectContaining({ path: "root.m" }));

    rootFile.focus();
    fireEvent.keyDown(rootFile, { key: "F10", shiftKey: true });
    const keyboardMenu = screen.getByRole("menu", { name: "Actions for root.m" });
    expect(screen.getByRole("menuitem", { name: "New file here" })).toHaveFocus();
    fireEvent.keyDown(keyboardMenu, { key: "Escape" });
    expect(rootFile).toHaveFocus();

    const directory = screen.getByRole("treeitem", { name: "src" });
    fireEvent.contextMenu(directory, { clientX: 20, clientY: 30 });
    const menu = screen.getByRole("menu", { name: "Actions for src" });
    expect(menu).toBeVisible();
    fireEvent.click(screen.getByRole("menuitem", { name: "New file here" }));
    expect(onStartCreate).toHaveBeenCalledWith("file", "src");
  });

  it("toggles folders on double click and enters them only through Open", async () => {
    const user = userEvent.setup();
    const onEnterDirectory = vi.fn();
    render(<TreeHarness onEnterDirectory={onEnterDirectory} />);
    const directory = screen.getByRole("treeitem", { name: "src" });
    await user.dblClick(directory);
    expect(directory).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("treeitem", { name: "nested.m" })).toBeVisible();
    await user.dblClick(directory);
    expect(directory).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("treeitem", { name: "nested.m" })).toBeNull();
    expect(onEnterDirectory).not.toHaveBeenCalled();

    fireEvent.contextMenu(directory);
    expect(directory).toHaveAttribute("aria-expanded", "false");
    await user.click(screen.getByRole("menuitem", { name: "Open" }));
    expect(onEnterDirectory).toHaveBeenCalledOnce();
    expect(onEnterDirectory).toHaveBeenCalledWith("src");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("offers Download only for regular files", () => {
    const onDownload = vi.fn();
    render(<TreeHarness onDownload={onDownload} />);

    fireEvent.contextMenu(screen.getByRole("treeitem", { name: "root.m" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Download" }));
    expect(onDownload).toHaveBeenCalledWith(
      expect.objectContaining({ path: "root.m", kind: "file" }),
    );

    fireEvent.contextMenu(screen.getByRole("treeitem", { name: "src" }));
    expect(screen.queryByRole("menuitem", { name: "Download" })).toBeNull();
  });

  it("copies the workspace-relative path from the context menu", async () => {
    const onCopyRelativePath = vi.fn(async () => undefined);
    render(<TreeHarness onCopyRelativePath={onCopyRelativePath} />);

    fireEvent.contextMenu(screen.getByRole("treeitem", { name: "root.m" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy Relative Path" }));

    await waitFor(() =>
      expect(onCopyRelativePath).toHaveBeenCalledWith(
        expect.objectContaining({ path: "root.m" }),
      ),
    );
    expect(await screen.findByRole("status")).toHaveTextContent("Copied root.m");
  });

  it("uploads selected files and drops files into a directory", () => {
    const onUpload = vi.fn();
    const { container } = render(<TreeHarness onUpload={onUpload} />);
    const selected = new File(["selected"], "selected.mat");
    fireEvent.change(container.querySelector('input[type="file"]')!, {
      target: { files: [selected] },
    });
    expect(onUpload).toHaveBeenCalledWith([selected], "");

    const dropped = new File(["dropped"], "dropped.mat");
    const transfer = { types: ["Files"], files: [dropped], dropEffect: "none" };
    fireEvent.dragEnter(screen.getByRole("treeitem", { name: "src" }), {
      dataTransfer: transfer,
    });
    expect(screen.getByRole("status")).toHaveTextContent("Drop files into src");
    fireEvent.drop(screen.getByRole("treeitem", { name: "src" }), {
      dataTransfer: transfer,
    });
    expect(onUpload).toHaveBeenLastCalledWith([dropped], "src");
  });

  it("flips and clamps a context menu inside the visual viewport", () => {
    expect(
      fitContextMenuToViewport(790, 590, 224, 260, {
        left: 0,
        top: 0,
        width: 800,
        height: 600,
      }),
    ).toEqual({ x: 566, y: 330 });
    expect(
      fitContextMenuToViewport(-50, -20, 900, 700, {
        left: 0,
        top: 0,
        width: 800,
        height: 600,
      }),
    ).toEqual({ x: 8, y: 8 });
  });

  it("offers an icon-only manual refresh action", () => {
    const onRefresh = vi.fn();
    render(<TreeHarness onRefresh={onRefresh} />);
    const refresh = screen.getByRole("button", { name: "Refresh Current Folder" });
    expect(refresh).toHaveAttribute("title", "Refresh Current Folder");
    expect(refresh.textContent).toBe("");
    fireEvent.click(refresh);
    expect(onRefresh).toHaveBeenCalledOnce();
  });

  it("marks path folders and exposes direct and recursive path actions", () => {
    const onAddSearchPath = vi.fn();
    const onRemoveSearchPath = vi.fn();
    const { rerender } = render(
      <TreeHarness onAddSearchPath={onAddSearchPath} />,
    );
    let directory = screen.getByRole("treeitem", { name: "src" });
    fireEvent.contextMenu(directory);
    fireEvent.click(
      screen.getByRole("menuitem", { name: "Add to Path with Subfolders" }),
    );
    expect(onAddSearchPath).toHaveBeenCalledWith(
      expect.objectContaining({ path: "src" }),
      true,
    );

    rerender(
      <TreeHarness
        searchPathWorkspacePaths={new Set(["src"])}
        onRemoveSearchPath={onRemoveSearchPath}
      />,
    );
    directory = screen.getByRole("treeitem", { name: "src PATH" });
    expect(screen.getByTitle("On MATLAB path")).toHaveTextContent("PATH");
    fireEvent.contextMenu(directory);
    fireEvent.click(screen.getByRole("menuitem", { name: "Remove from Path" }));
    expect(onRemoveSearchPath).toHaveBeenCalledWith(
      expect.objectContaining({ path: "src" }),
      false,
    );
  });
});
