import { act, renderHook, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  WorkspaceClientError,
  workspaceDocumentUri,
  type WorkspaceClient,
  type WorkspaceFile,
} from "../workspace/workspace-client";
import {
  isDocumentDirty,
  openDocumentFromWorkspaceFile,
  parseDocumentSession,
  snapshotDocumentSession,
  type OpenDocument,
} from "./document-session";
import { useDocumentSessionPersistence } from "./use-document-session-persistence";

const rootPath = "C:\\project";
const file: WorkspaceFile = {
  path: "SignalApp.m", rootPath, rootGeneration: 1,
  content: "", revision: "", size: 0,
};
const missingFile = new WorkspaceClientError("workspace.notFound", "File does not exist.");

function renderRecovery(
  document = openDocumentFromWorkspaceFile(file),
  read = vi.fn<WorkspaceClient["read"]>().mockRejectedValue(missingFile),
  currentRootPath = rootPath,
) {
  const session = snapshotDocumentSession([document], document.id);
  const store = {
    load: vi.fn().mockResolvedValue(parseDocumentSession(session)),
    save: vi.fn().mockResolvedValue(undefined),
    clear: vi.fn().mockResolvedValue(undefined),
  };
  const workspaceClient = { read } as unknown as WorkspaceClient;
  const hook = renderHook(({ currentRootPath, generation, connectionGeneration }) => {
    const [documents, setDocuments] = useState<readonly OpenDocument[]>([]);
    const [activeDocumentId, setActiveDocumentId] = useState<string | null>(null);
    useDocumentSessionPersistence({
      documents, setDocuments, activeDocumentId, setActiveDocumentId,
      store, storeKey: "test-session", workspaceClient, workspaceReady: true,
      workspaceRootPath: currentRootPath,
      workspaceRootGeneration: generation,
      workspaceConnectionGeneration: connectionGeneration,
    });
    return { documents, activeDocumentId };
  }, {
    initialProps: { currentRootPath, generation: 8, connectionGeneration: 1 },
  });
  return { ...hook, read, store };
}

describe("new source document recovery", () => {
  it("recovers an empty unsaved source after confirmed absence and persists it again", async () => {
    const recovery = renderRecovery();
    await waitFor(() => expect(recovery.result.current.documents).toHaveLength(1));
    const document = recovery.result.current.documents[0]!;
    expect(document).toMatchObject({
      rootGeneration: 8, revision: "", content: "", recoveryStatus: "none",
      uri: workspaceDocumentUri(file.path, 8, rootPath),
    });
    expect(isDocumentDirty(document)).toBe(true);
    expect(recovery.result.current.activeDocumentId).toBe(document.id);
    act(() => window.dispatchEvent(new Event("pagehide")));
    expect(recovery.store.save).toHaveBeenCalledWith("test-session", expect.objectContaining({
      documents: [expect.objectContaining({ content: "", revision: "", rootGeneration: 8 })],
    }));
    const unload = new Event("beforeunload", { cancelable: true });
    act(() => window.dispatchEvent(unload));
    expect(unload.defaultPrevented).toBe(true);
  });

  it("marks a new source as conflicted if its filename now exists on disk", async () => {
    const read = vi.fn<WorkspaceClient["read"]>().mockResolvedValue({
      ...file, rootGeneration: 8, content: "external = 1;", revision: "disk-revision",
    });
    const recovery = renderRecovery(openDocumentFromWorkspaceFile(file), read);
    await waitFor(() => expect(recovery.result.current.documents[0]?.recoveryStatus).toBe("conflict"));
    expect(recovery.result.current.documents[0]).toMatchObject({
      content: "", savedContent: "external = 1;", revision: "disk-revision",
    });
  });

  it("keeps a missing previously saved file unavailable", async () => {
    const recovery = renderRecovery(openDocumentFromWorkspaceFile({
      ...file, content: "original = 1;", revision: "real-revision",
    }));
    await waitFor(() => expect(recovery.result.current.documents[0]?.recoveryStatus).toBe("unavailable"));
    expect(recovery.result.current.documents[0]).toMatchObject({ revision: "real-revision" });
  });

  it.each([
    new WorkspaceClientError("workspace.permissionDenied", "Access denied"),
    new Error("File does not exist."),
  ])("does not assume file absence from another read failure (%s)", async (error) => {
    const read = vi.fn<WorkspaceClient["read"]>().mockRejectedValue(error);
    const recovery = renderRecovery(openDocumentFromWorkspaceFile(file), read);
    await waitFor(() => expect(recovery.result.current.documents[0]?.recoveryStatus).toBe("unavailable"));
    expect(recovery.result.current.documents[0]?.revision).toBe("");
  });

  it("keeps another root's new draft unavailable until switching back and verifying absence", async () => {
    const recovery = renderRecovery(undefined, undefined, "D:\\another-project");
    await waitFor(() => expect(recovery.result.current.documents[0]?.recoveryStatus).toBe("unavailable"));
    expect(recovery.read).not.toHaveBeenCalled();
    recovery.rerender({ currentRootPath: rootPath, generation: 9, connectionGeneration: 1 });
    await waitFor(() => expect(recovery.result.current.documents[0]?.recoveryStatus).toBe("none"));
    expect(recovery.result.current.documents[0]).toMatchObject({
      rootGeneration: 9, rootPath, revision: "",
      uri: workspaceDocumentUri(file.path, 9, rootPath),
    });
  });

  it("rechecks absence when the same root receives a new generation", async () => {
    const recovery = renderRecovery();
    await waitFor(() => expect(recovery.result.current.documents[0]?.rootGeneration).toBe(8));
    recovery.rerender({ currentRootPath: rootPath, generation: 10, connectionGeneration: 2 });
    await waitFor(() => expect(recovery.result.current.documents[0]?.rootGeneration).toBe(10));
    expect(recovery.read).toHaveBeenCalledTimes(2);
    expect(recovery.result.current.documents[0]?.recoveryStatus).toBe("none");
  });
});
