import { describe, expect, it } from "vitest";
// @ts-expect-error Monaco exposes this URI runtime without a declaration file.
import { URI } from "monaco-editor/esm/vs/base/common/uri.js";
import {
  documentId,
  isDocumentDirty,
  openDocumentFromWorkspaceFile,
  parseDocumentSession,
  restoreDocument,
  restoreNewDocument,
  snapshotDocumentSession,
} from "./document-session";
import { documentSessionKey } from "./document-session-storage";
import { workspaceDocumentUri } from "../workspace/workspace-client";

const diskFile = {
  path: "seed.m",
  rootPath: "C:\\project",
  rootGeneration: 4,
  content: "seed = 1;\n",
  revision: "revision-1",
  size: 10,
};

describe("document session recovery", () => {
  it("keeps different roots distinct when a restarted server reuses a generation", () => {
    const first = openDocumentFromWorkspaceFile({
      ...diskFile,
      path: "source/main.m",
      rootPath: "C:\\first workspace",
      rootGeneration: 1,
    });
    const second = openDocumentFromWorkspaceFile({
      ...diskFile,
      path: first.path,
      rootPath: "D:\\second-workspace",
      rootGeneration: 1,
    });
    expect(first.uri).toBe(
      "openmat-workspace://root-1/C%253A%255Cfirst%2520workspace/source/main.m",
    );
    expect(second.uri).not.toBe(first.uri);
    expect(second.id).not.toBe(first.id);
    const persisted = snapshotDocumentSession(
      [{ ...first, content: "unsaved = 1;\n" }], first.id,
    ).documents[0]!;
    expect(restoreDocument(persisted, null, "Previous root unavailable")).toMatchObject({
      uri: first.uri,
      content: "unsaved = 1;\n",
      recoveryStatus: "unavailable",
    });
  });

  it("retains the legacy helper format when no root path is supplied", () => {
    expect(workspaceDocumentUri("source/main.m", 1)).toBe(
      "openmat-workspace://root-1/source/main.m",
    );
  });

  it.each(["C:\\work\\OpenMat\\workspace root", "/home/user/workspace root"])(
    "preserves root identity through the actual Monaco URI parser for %s",
    (rootPath) => {
      const uri = workspaceDocumentUri("source/helper.m", 1, rootPath);
      const canonical = URI.parse(uri).toString();
      expect(canonical).toBe(uri);
      expect(URI.parse(canonical).toString()).toBe(uri);
      const decodedPath = URI.parse(uri).path as string;
      expect(decodedPath.split("/")).toHaveLength(4);
      expect(decodedPath).toMatch(/\/source\/helper\.m$/);
      expect(decodeURIComponent(decodedPath.split("/")[1]!)).toBe(rootPath);
    },
  );

  it("round-trips bounded versioned session data", () => {
    const document = {
      ...openDocumentFromWorkspaceFile(diskFile),
      content: "seed = 2;\n",
      viewState: { lineNumber: 2, column: 3, scrollTop: 18, scrollLeft: 0 },
    };
    const snapshot = snapshotDocumentSession([document], document.id);

    expect(parseDocumentSession(snapshot)).toEqual(snapshot);
    expect(parseDocumentSession({ version: 99, documents: [] })).toBeNull();
  });

  it("round-trips a pending same-directory rename without changing the saved identity", () => {
    const document = {
      ...openDocumentFromWorkspaceFile({ ...diskFile, path: "source/seed.m" }),
      pendingPath: "source/calculate.m",
    };
    const snapshot = snapshotDocumentSession([document], document.id);
    expect(parseDocumentSession(snapshot)).toEqual(snapshot);
    expect(snapshot.documents[0]).toMatchObject({
      path: "source/seed.m", pendingPath: "source/calculate.m",
    });
    expect(isDocumentDirty(document)).toBe(true);
    const restored = restoreDocument(snapshot.documents[0]!, {
      ...diskFile, path: "source/seed.m", rootGeneration: 8,
    }, "unavailable");
    expect(restored).toMatchObject({
      id: document.id, path: "source/seed.m", pendingPath: "source/calculate.m",
      content: diskFile.content, savedContent: diskFile.content, recoveryStatus: "none",
      rootGeneration: 8,
    });
    expect(isDocumentDirty(restored)).toBe(true);
  });

  it.each([
    "", "source/seed.m", "calculate.m", "other/calculate.m", "source/calculate.txt",
    "source/../calculate.m", "source/./calculate.m", "source//calculate.m",
    "source\\calculate.m", "/source/calculate.m", "C:/source/calculate.m",
    "source/calculate\0.m", null, 123,
  ])("rejects an unsafe or incompatible pending rename: %j", (pendingPath) => {
    const document = openDocumentFromWorkspaceFile({ ...diskFile, path: "source/seed.m" });
    const snapshot = snapshotDocumentSession([document], document.id);
    expect(parseDocumentSession({
      ...snapshot,
      documents: [{ ...snapshot.documents[0], pendingPath }],
    })?.documents).toEqual([]);
    if (typeof pendingPath === "string") {
      expect(snapshotDocumentSession([{ ...document, pendingPath }], document.id)
        .documents[0]).not.toHaveProperty("pendingPath");
    }
  });

  it("preserves pending rename and conflict if the source changed on disk", () => {
    const document = { ...openDocumentFromWorkspaceFile(diskFile), pendingPath: "calculate.m" };
    const persisted = snapshotDocumentSession([document], document.id).documents[0]!;
    const changedFile = { ...diskFile, revision: "changed-revision", content: "external = 2;" };
    const restored = restoreDocument(persisted, changedFile, "unavailable");
    expect(restored).toMatchObject({
      pendingPath: "calculate.m", content: diskFile.content,
      savedContent: changedFile.content, revision: changedFile.revision,
      recoveryStatus: "conflict",
    });
    const persistedAgain = snapshotDocumentSession([restored], restored.id).documents[0]!;
    expect(restoreDocument(persistedAgain, changedFile, "unavailable")).toMatchObject({
      pendingPath: "calculate.m", recoveryStatus: "conflict",
    });
  });

  it("preserves a pending rename while unavailable and while recovering an absent new source", () => {
    const document = {
      ...openDocumentFromWorkspaceFile({ ...diskFile, content: "", revision: "" }),
      pendingPath: "calculate.m",
    };
    const persisted = snapshotDocumentSession([document], document.id).documents[0]!;
    expect(restoreDocument(persisted, null, "Other root")).toMatchObject({
      pendingPath: "calculate.m", recoveryStatus: "unavailable",
    });
    const restored = restoreNewDocument(persisted, 8)!;
    expect(restored).toMatchObject({ pendingPath: "calculate.m", rootGeneration: 8 });
    expect(isDocumentDirty(restored)).toBe(true);
  });

  it("restores a draft directly when its base revision is unchanged", () => {
    const persisted = snapshotDocumentSession(
      [{ ...openDocumentFromWorkspaceFile(diskFile), content: "draft = 1;\n" }],
      null,
    ).documents[0]!;

    expect(restoreDocument(persisted, diskFile, "unavailable")).toMatchObject({
      content: "draft = 1;\n",
      savedContent: "seed = 1;\n",
      revision: "revision-1",
      recoveryStatus: "none",
    });
  });

  it("persists and restores an empty new source as an unsaved document", () => {
    const document = openDocumentFromWorkspaceFile({
      ...diskFile, content: "", revision: "", size: 0,
    });
    const snapshot = snapshotDocumentSession([document], document.id);
    expect(isDocumentDirty(document)).toBe(true);
    expect(parseDocumentSession(snapshot)).toEqual(snapshot);
    const restored = restoreNewDocument(snapshot.documents[0]!, 12)!;
    expect(restored).toMatchObject({
      id: document.id, rootGeneration: 12, revision: "", content: "",
      recoveryStatus: "none", recoveryMessage: null,
      uri: workspaceDocumentUri(document.path, 12, document.rootPath),
    });
    expect(isDocumentDirty(restored)).toBe(true);
  });

  it.each(["", "generated = 1;\n"])(
    "protects a recovered new source when a disk file now exists (%j)",
    (content) => {
      const persisted = snapshotDocumentSession([
        openDocumentFromWorkspaceFile({ ...diskFile, content, revision: "" }),
      ], null).documents[0]!;
      expect(restoreDocument(persisted, diskFile, "unavailable")).toMatchObject({
        content, revision: diskFile.revision, savedContent: diskFile.content,
        recoveryStatus: "conflict",
      });
    },
  );

  it("never treats a missing previously saved document as a new source", () => {
    const persisted = snapshotDocumentSession([
      openDocumentFromWorkspaceFile(diskFile),
    ], null).documents[0]!;
    expect(restoreNewDocument(persisted, 12)).toBeNull();
    expect(restoreDocument(persisted, null, "Missing disk file")).toMatchObject({
      revision: diskFile.revision, recoveryStatus: "unavailable",
    });
  });

  it("keeps a new-file collision unresolved across recovery even when both files are empty", () => {
    const persisted = snapshotDocumentSession([
      openDocumentFromWorkspaceFile({ ...diskFile, content: "", revision: "" }),
    ], null).documents[0]!;
    const emptyDiskFile = { ...diskFile, content: "" };
    const conflicted = restoreDocument(persisted, emptyDiskFile, "unavailable");
    expect(conflicted.recoveryStatus).toBe("conflict");
    const persistedAgain = snapshotDocumentSession([conflicted], null).documents[0]!;
    expect(restoreDocument(persistedAgain, emptyDiskFile, "unavailable").recoveryStatus)
      .toBe("conflict");
  });

  it("blocks a recovered draft when the disk revision changed", () => {
    const persisted = snapshotDocumentSession(
      [{ ...openDocumentFromWorkspaceFile(diskFile), content: "draft = 1;\n" }],
      null,
    ).documents[0]!;
    const changedDiskFile = {
      ...diskFile,
      content: "external = 1;\n",
      revision: "revision-external",
    };

    const conflicted = restoreDocument(persisted, changedDiskFile, "unavailable");
    expect(conflicted).toMatchObject({
      id: documentId("C:\\project", "seed.m"),
      content: "draft = 1;\n",
      savedContent: "external = 1;\n",
      revision: "revision-external",
      recoveryStatus: "conflict",
    });
    const persistedAgain = snapshotDocumentSession(
      [conflicted],
      conflicted.id,
    ).documents[0]!;
    expect(
      restoreDocument(persistedAgain, changedDiskFile, "unavailable")
        .recoveryStatus,
    ).toBe("conflict");
  });

  it("uses a URL-scoped key without credentials or transient query data", () => {
    expect(
      documentSessionKey("wss://user:secret@example.test/kernel?ticket=secret"),
    ).toBe("workspace:wss://example.test/kernel");
  });
});
