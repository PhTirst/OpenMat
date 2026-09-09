import {
  useEffect,
  useCallback,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import type { WorkspaceClient } from "../workspace/workspace-client";
import { WorkspaceClientError } from "../workspace/workspace-client";
import {
  isDocumentDirty,
  restoreDocument,
  restoreNewDocument,
  snapshotDocumentSession,
  type OpenDocument,
  type DesktopWorkspaceSession,
} from "./document-session";
import type { DocumentSessionStore } from "./document-session-storage";
import { SessionWriter } from "./session-writer";

interface DocumentSessionPersistenceOptions {
  readonly documents: readonly OpenDocument[];
  readonly setDocuments: Dispatch<SetStateAction<readonly OpenDocument[]>>;
  readonly activeDocumentId: string | null;
  readonly setActiveDocumentId: Dispatch<SetStateAction<string | null>>;
  readonly store: DocumentSessionStore;
  readonly storeKey: string;
  readonly workspaceClient: WorkspaceClient;
  readonly workspaceReady: boolean;
  readonly workspaceRootPath: string;
  readonly workspaceRootGeneration: number;
  readonly workspaceConnectionGeneration: number;
  readonly desktopWorkspace?: DesktopWorkspaceSession | undefined;
  readonly warnOnBrowserExit?: boolean;
}

export interface DocumentSessionPersistence {
  readonly ready: boolean;
  flush(documents?: readonly OpenDocument[], workspace?: DesktopWorkspaceSession): Promise<void>;
  resume(): void;
}

export function useDocumentSessionPersistence({
  documents,
  setDocuments,
  activeDocumentId,
  setActiveDocumentId,
  store,
  storeKey,
  workspaceClient,
  workspaceReady,
  workspaceRootPath,
  workspaceRootGeneration,
  workspaceConnectionGeneration,
  desktopWorkspace,
  warnOnBrowserExit = true,
}: DocumentSessionPersistenceOptions): DocumentSessionPersistence {
  const [loadState, setLoadState] = useState<"pending" | "ready" | "failed">(
    "pending",
  );
  const revalidationKeys = useRef<Set<string>>(new Set());
  const documentsRef = useRef(documents);
  const activeDocumentIdRef = useRef(activeDocumentId);
  documentsRef.current = documents;
  activeDocumentIdRef.current = activeDocumentId;
  const workspaceRef = useRef(desktopWorkspace);
  workspaceRef.current = desktopWorkspace;
  const writer = useMemo(() => new SessionWriter(store, storeKey), [store, storeKey]);
  const timerRef = useRef<number | undefined>(undefined);
  const pausedRef = useRef(false);
  const snapshot = useCallback((override?: readonly OpenDocument[], workspace = workspaceRef.current) =>
    snapshotDocumentSession(override ?? documentsRef.current, activeDocumentIdRef.current, workspace), []);
  const flush = useCallback(async (override?: readonly OpenDocument[], workspace?: DesktopWorkspaceSession) => {
    if (loadState !== "ready") {
      if (documentsRef.current.length > 0) throw new Error("Editor recovery is not ready. Keep OpenMat open and retry after the workspace reconnects.");
      return; // Preserve the previous session if startup has not restored it yet.
    }
    pausedRef.current = true;
    window.clearTimeout(timerRef.current);
    await writer.save(snapshot(override, workspace));
  }, [loadState, snapshot, writer]);
  const resume = useCallback(() => {
    if (!pausedRef.current) return;
    pausedRef.current = false;
    if (loadState === "ready") void writer.save(snapshot()).catch(() => {});
  }, [loadState, snapshot, writer]);

  useEffect(() => {
    if (!workspaceReady || loadState !== "pending") {
      return;
    }
    let disposed = false;
    const restoreSession = async (): Promise<void> => {
      let loadSucceeded = false;
      try {
        const session = await store.load(storeKey);
        loadSucceeded = true;
        if (disposed || session === null) {
          return;
        }
        const restoredDocuments = await Promise.all(
          session.documents.map(async (persisted) => {
            if (!workspaceRootsEqual(persisted.rootPath, workspaceRootPath)) {
              return restoreDocument(
                persisted,
                null,
                `Recovered from ${persisted.rootPath}. Switch back to that Current Folder before saving this draft.`,
              );
            }
            try {
              return restoreDocument(
                persisted,
                await workspaceClient.read(persisted.path),
                "",
              );
            } catch (error: unknown) {
              const newDocument = isMissingFile(error)
                ? restoreNewDocument(persisted, workspaceRootGeneration)
                : null;
              if (newDocument !== null) {
                return newDocument;
              }
              return restoreDocument(
                persisted,
                null,
                `The disk file could not be verified: ${toErrorMessage(error)}`,
              );
            }
          }),
        );
        if (disposed) {
          return;
        }
        setDocuments((current) => {
          const currentById = new Map(
            current.map((document) => [document.id, document]),
          );
          const restored = restoredDocuments.map(
            (document) => currentById.get(document.id) ?? document,
          );
          const restoredIds = new Set(restored.map((document) => document.id));
          return [
            ...restored,
            ...current.filter((document) => !restoredIds.has(document.id)),
          ];
        });
        const restoredIds = new Set(
          restoredDocuments.map((document) => document.id),
        );
        setActiveDocumentId((current) => {
          if (current !== null) {
            return current;
          }
          if (
            session.activeDocumentId !== null &&
            restoredIds.has(session.activeDocumentId)
          ) {
            return session.activeDocumentId;
          }
          return restoredDocuments[0]?.id ?? null;
        });
      } catch (error: unknown) {
        console.warn("OpenMat could not restore the editor session.", error);
      } finally {
        if (!disposed) {
          setLoadState(loadSucceeded ? "ready" : "failed");
        }
      }
    };
    void restoreSession();
    return () => {
      disposed = true;
    };
  }, [
    loadState,
    setActiveDocumentId,
    setDocuments,
    store,
    storeKey,
    workspaceClient,
    workspaceReady,
    workspaceRootGeneration,
    workspaceRootPath,
  ]);

  useEffect(() => {
    if (loadState !== "ready" || pausedRef.current) {
      return;
    }
    timerRef.current = window.setTimeout(() => {
      void writer
        .save(snapshot())
        .catch((error: unknown) =>
          console.warn("OpenMat could not persist the editor session.", error),
        );
    }, 300);
    return () => window.clearTimeout(timerRef.current);
  }, [activeDocumentId, documents, desktopWorkspace, loadState, snapshot, writer]);

  useEffect(() => {
    if (loadState !== "ready") {
      return;
    }
    const persistBeforePageExit = (): void => {
      if (pausedRef.current) return;
      void writer
        .save(snapshot())
        .catch(() => {});
    };
    window.addEventListener("pagehide", persistBeforePageExit);
    return () => window.removeEventListener("pagehide", persistBeforePageExit);
  }, [loadState, snapshot, writer]);

  useEffect(() => {
    if (loadState !== "ready" || !workspaceReady) {
      return;
    }
    const candidates = documents.filter(
      (document) =>
        workspaceRootsEqual(document.rootPath, workspaceRootPath) &&
        (document.rootGeneration !== workspaceRootGeneration ||
          document.recoveryStatus === "unavailable"),
    );
    if (candidates.length === 0) {
      return;
    }
    const revalidationKey = `${workspaceConnectionGeneration.toString()}\0${workspaceRootGeneration.toString()}\0${workspaceRootPath}`;
    if (revalidationKeys.current.has(revalidationKey)) {
      return;
    }
    revalidationKeys.current.add(revalidationKey);
    let disposed = false;
    const revalidate = async (): Promise<void> => {
      const results = await Promise.all(
        candidates.map(async (document) => {
          try {
            return {
              id: document.id,
              file: await workspaceClient.read(document.path),
              error: null,
              missing: false,
            };
          } catch (error: unknown) {
            return {
              id: document.id,
              file: null,
              error: toErrorMessage(error),
              missing: isMissingFile(error),
            };
          }
        }),
      );
      if (disposed) {
        return;
      }
      const resultsById = new Map(results.map((result) => [result.id, result]));
      setDocuments((current) =>
        current.map((document) => {
          const result = resultsById.get(document.id);
          if (result === undefined) {
            return document;
          }
          const persisted = snapshotDocumentSession(
            [document],
            document.id,
          ).documents[0]!;
          const newDocument = result.missing
            ? restoreNewDocument(persisted, workspaceRootGeneration)
            : null;
          if (newDocument !== null) {
            return newDocument;
          }
          return restoreDocument(
            persisted,
            result.file,
            `The disk file could not be verified: ${result.error ?? "unknown error"}`,
          );
        }),
      );
    };
    void revalidate();
    return () => {
      disposed = true;
    };
  }, [
    documents,
    loadState,
    setDocuments,
    workspaceClient,
    workspaceConnectionGeneration,
    workspaceReady,
    workspaceRootGeneration,
    workspaceRootPath,
  ]);

  const hasDirtyDocuments = documents.some(isDocumentDirty);
  useEffect(() => {
    if (!hasDirtyDocuments || !warnOnBrowserExit) {
      return;
    }
    const warnBeforeUnload = (event: BeforeUnloadEvent): void => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warnBeforeUnload);
    return () => window.removeEventListener("beforeunload", warnBeforeUnload);
  }, [hasDirtyDocuments, warnOnBrowserExit]);
  return { ready: loadState === "ready", flush, resume };
}

function workspaceRootsEqual(left: string, right: string): boolean {
  const windowsPaths =
    /^[A-Za-z]:[\\/]/.test(left) && /^[A-Za-z]:[\\/]/.test(right);
  return windowsPaths ? left.toLowerCase() === right.toLowerCase() : left === right;
}

function toErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Unknown workspace error";
}

function isMissingFile(error: unknown): boolean {
  return error instanceof WorkspaceClientError && error.code === "workspace.notFound";
}
