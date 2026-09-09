import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import { DesktopCloseDialog, type DesktopCloseActions } from "./components/DesktopCloseDialog";
import type { DesignerSession } from "./designer/designer-session";
import { PendingOperations } from "./platform/pending-operations";
import { useCommitBarrier } from "./platform/use-commit-barrier";
import { DESKTOP_CLOSE_PREPARE_EVENT } from "./platform/desktop-lifecycle";
import { CommandHistoryPane } from "./components/CommandHistoryPane";
import { CommandWindowPane } from "./components/CommandWindowPane";
import { CurrentFolderAddressBar } from "./components/CurrentFolderAddressBar";
import { CurrentFolderPane } from "./components/CurrentFolderPane";
import { DirectoryPickerDialog } from "./components/DirectoryPickerDialog";
import { EditorPane } from "./components/EditorPane";
import type { CodeEditorProps } from "./components/CodeEditor";
import { FigureWindow } from "./components/FigureWindow";
import { SettingsIcon } from "./components/Icons";
import { ComponentIcon } from "./designer/ComponentIcon";
import { ResizableIdeGrid } from "./components/ResizableIdeGrid";
import { SettingsPanel } from "./components/SettingsPanel";
import {
  VariableEditorWindow,
  createVariableEditorRange,
  variableEditorWindowId,
  type VariableEditorCellEdit,
  type VariableEditorCommitResult,
  type VariableEditorPreview,
} from "./components/VariableEditorWindow";
import { WorkspacePane } from "./components/WorkspacePane";
import {
  ClearWorkspaceDialog,
  DeleteEntryDialog,
  MoveEntryDialog,
  RenameEntryDialog,
  SaveConflictDialog,
  SaveCopyDialog,
  UnsavedChangesDialog,
  type DeleteImpact,
  type SaveConflictDecision,
  type UnsavedChangesDecision,
} from "./components/WorkspaceOperationDialogs";
import {
  DEFAULT_CONNECTION_RETRY_DELAYS,
  summarizeSessionConnection,
  useConnectionRecovery,
} from "./connection/use-connection-recovery";
import {
  documentId,
  isDocumentDirty,
  openDocumentFromWorkspaceFile,
  restoreDocument,
  snapshotDocumentSession,
  type DocumentViewState,
  type OpenDocument,
  type DesktopWorkspaceSession,
} from "./documents/document-session";
import {
  documentSessionKey,
  IndexedDbDocumentSessionStore,
  type DocumentSessionStore,
} from "./documents/document-session-storage";
import { useDocumentSessionPersistence } from "./documents/use-document-session-persistence";
import type { DesignerSourceWorkspace } from "./documents/designer-source-workspace";
import { SharedEditorSession } from "./lsp/shared-editor-session";
import type { OpenMatFileRename } from "./lsp/monaco-lsp";
import { resolveLspWebSocketUrl } from "./lsp/url";
import {
  KERNEL_PROTOCOL_V3,
  MAX_AGGREGATE_DEPTH,
  MAX_AGGREGATE_ELEMENTS,
  MAX_AGGREGATE_NODES,
  MAX_PREVIEW_CODE_UNITS,
  MAX_STRING_ELEMENT_CODE_UNITS,
  SUPPORTED_KERNEL_PROTOCOLS,
  createBootstrapInitializeRequest,
  createKernelRequest,
  type KernelProtocol,
} from "./protocol/kernel-v2";
import type {
  ExecuteMode,
  DisplayEventData,
  MatrixRange,
  VariableSummary,
} from "./protocol/kernel-v0";
import { FIGURE_MIME_TYPE } from "./plot/graphics-v1";
import { kernelWebSocketUrl } from "./runtime-config";
import { createPlatformServices, nativeFileKey, PlatformContext, usePlatformServices, type PlatformServices } from "./platform/platform-services";
import { ideReducer, initialIdeState } from "./state/ide-state";
import {
  persistTheme,
  restoreTheme,
  type IdeTheme,
} from "./theme";
import type { KernelTransport } from "./transport/kernel-transport";
import { MockKernelTransport } from "./transport/mock-kernel-transport";
import { WebSocketKernelTransport } from "./transport/websocket-kernel-transport";
import type { OpenMatAppRegistry } from "./windowing/app-registry";
import {
  createOpenMatAppRegistry,
  registerBuiltInOpenMatApps,
} from "./windowing/built-in-apps";
import {
  OpenMatDynamicAppLoader,
  OpenMatWindowLayer,
  OpenMatWindowManagerProvider,
  useOpenMatWindowManagerActions,
  type OpenMatDynamicAppSource,
} from "./windowing/WindowManager";
import { MockWorkspaceClient } from "./workspace/mock-workspace-client";
import {
  WorkspaceClientError,
  type CreatableWorkspaceEntryKind,
  type DirectoryBrowserSnapshot,
  type SearchPathSnapshot,
  type WorkspaceClient,
  type WorkspaceEntry,
  type WorkspaceFile,
  type WorkspaceSnapshot,
  joinWorkspacePath,
  workspaceDocumentUri,
  workspaceParentPath,
  parseWorkspaceDocumentUri,
} from "./workspace/workspace-client";
import { WebSocketWorkspaceClient } from "./workspace/websocket-workspace-client";

const SESSION_ID = "openmat-web-alpha";
const AppDesigner = lazy(() => import("./designer/AppDesigner"));
const INSPECTION_ELEMENT_LIMIT = 256;

interface AppProps {
  readonly platform?: PlatformServices;
  readonly transport?: KernelTransport;
  readonly workspaceClient?: WorkspaceClient;
  readonly wsUrl?: string;
  readonly appRegistry?: OpenMatAppRegistry;
  readonly dynamicAppModules?: readonly OpenMatDynamicAppSource[];
  readonly reconnectDelays?: readonly number[];
  readonly documentSessionStore?: DocumentSessionStore;
}

interface VariableEditorState {
  readonly variable: VariableSummary;
  readonly range: MatrixRange;
  readonly preview: VariableEditorPreview | null;
  readonly revision: number | null;
  readonly loading: boolean;
  readonly error: string | null;
}

interface DocumentSaveOutcome {
  readonly status: "saved" | "reloaded" | "copied" | "kept" | "failed";
  readonly documentId: string;
}

const NO_DYNAMIC_APP_MODULES: readonly OpenMatDynamicAppSource[] = [];

function toErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Unknown transport error";
}

function toWorkspaceError(error: unknown): {
  readonly code: string;
  readonly message: string;
} {
  return error instanceof WorkspaceClientError
    ? { code: error.code, message: error.message }
    : { code: "workspace.unknownFailure", message: toErrorMessage(error) };
}

function isWorkspaceRevisionConflict(
  error: unknown,
): error is WorkspaceClientError {
  return (
    error instanceof WorkspaceClientError &&
    error.code === "workspace.revisionConflict"
  );
}

export function startBrowserDownload(url: string, name: string): void {
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.rel = "noopener";
  link.referrerPolicy = "no-referrer";
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();
}

function pathIsWithin(path: string, parent: string): boolean {
  return path === parent || path.startsWith(`${parent}/`);
}

function remapPath(path: string, previousPath: string, nextPath: string): string {
  return pathIsWithin(path, previousPath)
    ? `${nextPath}${path.slice(previousPath.length)}`
    : path;
}

export function createDefaultKernelTransport(
  configuredUrl: string | undefined = kernelWebSocketUrl(),
): KernelTransport {
  const url = configuredUrl?.trim();
  return url === undefined || url.length === 0
    ? new MockKernelTransport()
    : new WebSocketKernelTransport(url);
}

export function createDefaultWorkspaceClient(
  configuredUrl: string | undefined = kernelWebSocketUrl(),
): WorkspaceClient {
  const url = configuredUrl?.trim();
  return url === undefined || url.length === 0
    ? new MockWorkspaceClient()
    : new WebSocketWorkspaceClient(url);
}

function workspaceSourceName(rootPath: string, relativePath: string): string {
  const separator = rootPath.includes("\\") ? "\\" : "/";
  const root = rootPath.replace(/[\\/]$/, "");
  return `${root}${separator}${relativePath.replaceAll("/", separator)}`;
}

function documentUriKey(uri: string): string | null {
  try {
    const url = new URL(uri);
    return JSON.stringify([
      url.protocol,
      url.host,
      url.username,
      url.password,
      url.pathname.split("/").map((segment) => decodeURIComponent(segment)),
      url.search,
      url.hash,
    ]);
  } catch {
    return null;
  }
}

type AppWorkbenchProps = Omit<AppProps, "appRegistry">;

function AppWorkbench({
  transport: suppliedTransport,
  workspaceClient: suppliedWorkspaceClient,
  wsUrl,
  dynamicAppModules = NO_DYNAMIC_APP_MODULES,
  reconnectDelays = DEFAULT_CONNECTION_RETRY_DELAYS,
  documentSessionStore: suppliedDocumentSessionStore,
}: AppWorkbenchProps) {
  const platform = usePlatformServices();
  const nativeFileBusy = useRef(false);
  const [pendingSaves] = useState(() => new PendingOperations());
  const waitForCommit = useCommitBarrier();
  const designerSession = useRef<DesignerSession | null>(null);
  const desktopCloseInFlight = useRef(false);
  const desktopCloseHandler = useRef<() => void>(() => {});
  const windowManager = useOpenMatWindowManagerActions();
  const [transport] = useState<KernelTransport>(
    () => suppliedTransport ?? createDefaultKernelTransport(wsUrl),
  );
  const [workspaceClient] = useState<WorkspaceClient>(
    () => suppliedWorkspaceClient ?? createDefaultWorkspaceClient(wsUrl),
  );
  const [documentStore] = useState<DocumentSessionStore>(
    () => suppliedDocumentSessionStore ?? new IndexedDbDocumentSessionStore(),
  );
  const [documentStoreKey] = useState(() =>
    documentSessionKey(wsUrl ?? kernelWebSocketUrl(), platform.kind, platform.workspaceIdentity),
  );
  const lspUrl = useMemo(
    () =>
      resolveLspWebSocketUrl(
        import.meta.env.VITE_OPENMAT_LSP_URL,
        wsUrl ?? kernelWebSocketUrl(),
      ) ?? null,
    [wsUrl],
  );
  const [state, dispatch] = useReducer(ideReducer, initialIdeState);
  const [theme, setTheme] = useState<IdeTheme>(restoreTheme);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [designerVisible, setDesignerVisible] = useState(false);
  const [designerMounted, setDesignerMounted] = useState(false);
  const [restoredDesktopWorkspace, setRestoredDesktopWorkspace] = useState<DesktopWorkspaceSession | null>(null);
  const [designerOpenRequest, setDesignerOpenRequest] = useState<{
    path: string;
    serial: number;
  } | null>(null);
  const [folder, setFolder] = useState<WorkspaceSnapshot>({
    rootName: "workspace",
    rootPath: "",
    rootGeneration: 0,
    path: "",
    recursive: false,
    entries: [],
  });
  const [folderLoading, setFolderLoading] = useState(true);
  const [folderReady, setFolderReady] = useState(false);
  const [folderServiceError, setFolderServiceError] = useState<string | null>(
    null,
  );
  const [searchPath, setSearchPath] = useState<SearchPathSnapshot>({
    generation: 0,
    directories: [],
  });
  const searchPathWorkspacePaths = useMemo(
    () =>
      new Set(
        searchPath.directories.flatMap((directory) =>
          directory.workspacePath === null ? [] : [directory.workspacePath],
        ),
      ),
    [searchPath],
  );
  const [selectedWorkspacePath, setSelectedWorkspacePath] = useState<string | null>(
    null,
  );
  const [expandedWorkspacePaths, setExpandedWorkspacePaths] = useState<
    ReadonlySet<string>
  >(() => new Set());
  const loadedWorkspacePaths = useRef<Set<string>>(new Set([""]));
  const loadingWorkspacePaths = useRef<Set<string>>(new Set());
  const currentFolderGeneration = useRef(0);
  const [creatingKind, setCreatingKind] =
    useState<CreatableWorkspaceEntryKind | null>(null);
  const [creatingParentPath, setCreatingParentPath] = useState("");
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const [workspaceUploadStatus, setWorkspaceUploadStatus] = useState<string | null>(
    null,
  );
  const [workspaceRefreshing, setWorkspaceRefreshing] = useState(false);
  const workspaceRefreshInFlight = useRef<Promise<void> | null>(null);
  const workspaceRefreshQueued = useRef(false);
  const [directoryPickerOpen, setDirectoryPickerOpen] = useState(false);
  const [directoryPickerSnapshot, setDirectoryPickerSnapshot] =
    useState<DirectoryBrowserSnapshot | null>(null);
  const [directoryPickerLoading, setDirectoryPickerLoading] = useState(false);
  const [directoryPickerError, setDirectoryPickerError] = useState<string | null>(null);
  const directoryPickerRequestSequence = useRef(0);
  const [workspaceError, setWorkspaceError] = useState<{
    readonly code: string;
    readonly message: string;
  } | null>(null);
  const [openDocuments, setOpenDocuments] = useState<readonly OpenDocument[]>([]);
  const [editorSession] = useState(() => new SharedEditorSession());
  const sourceFileRenames = useRef(new Map<string, {
    path: string; originalText: string; renamedText: string;
  }>());
  const folderRef = useRef(folder);
  folderRef.current = folder;
  const openDocumentsRef = useRef<readonly OpenDocument[]>(openDocuments);
  openDocumentsRef.current = openDocuments;
  const [activeDocumentId, setActiveDocumentId] = useState<string | null>(null);
  const [editorReveal, setEditorReveal] = useState<
    NonNullable<CodeEditorProps["reveal"]> | null
  >(null);
  const editorRevealSequence = useRef(0);
  const editorNavigationSequence = useRef(0);
  const activeDocument = useMemo(
    () =>
      openDocuments.find((document) => document.id === activeDocumentId) ?? null,
    [activeDocumentId, openDocuments],
  );
  const [editorLoading, setEditorLoading] = useState(false);
  const [savingDocumentIds, setSavingDocumentIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const editorSaving =
    activeDocumentId !== null && savingDocumentIds.has(activeDocumentId);
  const [editorError, setEditorError] = useState<string | null>(null);
  const [command, setCommand] = useState("");
  const [commandHistory, setCommandHistory] = useState<readonly string[]>([]);
  const [graphicsFigures, setGraphicsFigures] = useState<
    readonly DisplayEventData[]
  >([]);
  const kernelConnection = useConnectionRecovery(reconnectDelays);
  const workspaceConnection = useConnectionRecovery(reconnectDelays);
  const {
    generation: kernelConnectionGeneration,
    markConnected: markKernelConnected,
    markFailed: markKernelFailed,
    retryNow: retryKernelNow,
  } = kernelConnection;
  const {
    generation: workspaceConnectionGeneration,
    markConnected: markWorkspaceConnected,
    markFailed: markWorkspaceFailed,
    retryNow: retryWorkspaceNow,
  } = workspaceConnection;
  const workspaceConnectedOnce = useRef(false);
  const [selectedVariableName, setSelectedVariableName] = useState<string | null>(
    null,
  );
  const [variableEditors, setVariableEditors] = useState<
    ReadonlyMap<string, VariableEditorState>
  >(() => new Map());
  const requestSequence = useRef(0);
  const variableEditorRequestSequences = useRef<Map<string, number>>(new Map());
  const documentOpenSequences = useRef<Map<string, number>>(new Map());
  const activeDocumentIdRef = useRef(activeDocumentId);
  activeDocumentIdRef.current = activeDocumentId;

  const invalidateVariableEditorRequests = useCallback(() => {
    for (const [name, sequence] of variableEditorRequestSequences.current) {
      variableEditorRequestSequences.current.set(name, sequence + 1);
    }
  }, []);

  const closeAllVariableEditors = useCallback(() => {
    invalidateVariableEditorRequests();
    setVariableEditors(new Map());
  }, [invalidateVariableEditorRequests]);

  const installCurrentFolderSnapshot = useCallback((snapshot: WorkspaceSnapshot) => {
    currentFolderGeneration.current = snapshot.rootGeneration;
    loadedWorkspacePaths.current = new Set([""]);
    loadingWorkspacePaths.current.clear();
    setExpandedWorkspacePaths(new Set());
    setSelectedWorkspacePath(null);
    setCreatingKind(null);
    setFolder(snapshot);
    setFolderReady(true);
    setFolderServiceError(null);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.openmatTheme = theme;
    document.documentElement.style.colorScheme =
      theme === "modern-dark" ? "dark" : "light";
    persistTheme(theme);
  }, [theme]);

  useEffect(() => {
    let disposed = false;
    const attemptGeneration = workspaceConnectionGeneration;
    setFolderLoading(!workspaceConnectedOnce.current);
    setFolderReady(false);
    setFolderServiceError(null);
    const reportConnectionFailure = (error: unknown): void => {
      if (disposed) {
        return;
      }
      const message = toErrorMessage(error);
      setFolderReady(false);
      setFolderServiceError(message);
      markWorkspaceFailed(attemptGeneration, message);
    };
    const loadCurrentDirectory = async (expectedGeneration?: number): Promise<void> => {
      const [snapshot, nextSearchPath] = await Promise.all([
        workspaceClient.list("", false),
        typeof workspaceClient.searchPath === "function"
          ? workspaceClient.searchPath()
          : Promise.resolve({ generation: 0, directories: [] }),
      ]);
      if (
        !disposed &&
        (expectedGeneration === undefined ||
          snapshot.rootGeneration === expectedGeneration)
      ) {
        installCurrentFolderSnapshot(snapshot);
        setSearchPath(nextSearchPath);
      }
    };
    const unsubscribeDirectory = workspaceClient.onCurrentDirectoryChanged((directory) => {
      if (disposed || !workspaceConnectedOnce.current) {
        return;
      }
      if (directory.generation === currentFolderGeneration.current) {
        return;
      }
      currentFolderGeneration.current = directory.generation;
      void loadCurrentDirectory(directory.generation).catch((error: unknown) => {
        if (!disposed) {
          currentFolderGeneration.current = 0;
          setFolderServiceError(toErrorMessage(error));
        }
      });
    });
    const unsubscribeSearchPath =
      typeof workspaceClient.onSearchPathChanged === "function"
        ? workspaceClient.onSearchPathChanged((snapshot) => {
            if (!disposed) {
              setSearchPath(snapshot);
            }
          })
        : () => {};
    const unsubscribeConnectionLoss = workspaceClient.subscribeConnectionLoss(
      reportConnectionFailure,
    );
    const connect = async (): Promise<void> => {
      try {
        await workspaceClient.connect();
        if (!disposed && !workspaceConnectedOnce.current && platform.kind === "desktop" && platform.restoreWorkspace !== false) {
          try {
            const recovered = (await documentStore.load(documentStoreKey))?.desktopWorkspace;
            if (disposed) return;
            if (recovered) {
              await workspaceClient.changeDirectory(recovered.rootPath);
              if (!disposed) setRestoredDesktopWorkspace(recovered);
            }
          } catch (error) {
            if (!disposed) setWorkspaceError({ code: "workspace.restoreFailed",
              message: `Could not restore the previous Current Folder: ${toErrorMessage(error)}. Existing drafts remain available.` });
          }
        }
        if (disposed) return;
        await loadCurrentDirectory();
        if (!disposed) {
          workspaceConnectedOnce.current = true;
          markWorkspaceConnected(attemptGeneration);
        }
      } catch (error: unknown) {
        reportConnectionFailure(error);
      } finally {
        if (!disposed) {
          setFolderLoading(false);
        }
      }
    };
    void connect();
    return () => {
      disposed = true;
      unsubscribeDirectory();
      unsubscribeSearchPath();
      unsubscribeConnectionLoss();
      void workspaceClient.disconnect();
    };
  }, [
    installCurrentFolderSnapshot,
    markWorkspaceConnected,
    markWorkspaceFailed,
    workspaceClient,
    workspaceConnectionGeneration,
    documentStore,
    documentStoreKey,
    platform.kind,
    platform.restoreWorkspace,
  ]);

  const desktopWorkspace = useMemo(() => platform.kind === "desktop"
    ? { rootPath: folder.rootPath, designerMounted, designerVisible } : undefined,
    [platform.kind, folder.rootPath, designerMounted, designerVisible]);
  const documentPersistence = useDocumentSessionPersistence({
    documents: openDocuments,
    setDocuments: setOpenDocuments,
    activeDocumentId,
    setActiveDocumentId,
    store: documentStore,
    storeKey: documentStoreKey,
    workspaceClient,
    workspaceReady: folderReady,
    workspaceRootPath: folder.rootPath,
    workspaceRootGeneration: folder.rootGeneration,
    workspaceConnectionGeneration,
    desktopWorkspace,
    warnOnBrowserExit: platform.kind !== "desktop",
  });
  useEffect(() => {
    if (!documentPersistence.ready || restoredDesktopWorkspace === null) return;
    setDesignerMounted(restoredDesktopWorkspace.designerMounted);
    setDesignerVisible(restoredDesktopWorkspace.designerVisible);
    setRestoredDesktopWorkspace(null);
  }, [documentPersistence.ready, restoredDesktopWorkspace]);

  useEffect(() => {
    if (!platform.lifecycle) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void platform.lifecycle.onCloseRequested(() => desktopCloseHandler.current())
      .then((remove) => { if (disposed) remove(); else unlisten = remove; })
      .catch((error: unknown) => setWorkspaceError({ code: "desktop.closeGuardFailed", message: toErrorMessage(error) }));
    return () => { disposed = true; unlisten?.(); };
  }, [platform.lifecycle]);

  const nextRequestId = useCallback(() => {
    requestSequence.current += 1;
    return `web-request-${requestSequence.current.toString().padStart(4, "0")}`;
  }, []);

  const fetchWorkspaceForProtocol = useCallback(async (
    protocol: KernelProtocol,
  ): Promise<readonly VariableSummary[]> => {
    const response = await transport.request(
      createKernelRequest(
        protocol,
        SESSION_ID,
        nextRequestId(),
        "listWorkspace",
        {},
      ),
    );
    if (!response.ok) {
      throw new Error(response.error.message);
    }
    return response.result.data.variables;
  }, [nextRequestId, transport]);

  useEffect(() => {
    let disposed = false;
    const attemptGeneration = kernelConnectionGeneration;
    closeAllVariableEditors();
    dispatch({ type: "connectionStarted" });
    const reportConnectionFailure = (message: string): void => {
      if (disposed) {
        return;
      }
      closeAllVariableEditors();
      dispatch({ type: "connectionFailed", message });
      markKernelFailed(attemptGeneration, message);
    };

    const unsubscribe = transport.subscribe((event) => {
      if (event.sessionId === SESSION_ID) {
        const displayData =
          event.event.type === "display"
            ? (event.event.data as DisplayEventData)
            : null;
        if (displayData?.representations[FIGURE_MIME_TYPE] !== undefined) {
          setGraphicsFigures((current) => [...current, displayData]);
          return;
        }
        dispatch({
          type: "eventReceived",
          event,
        });
      }
    });
    const unsubscribeConnectionLoss = transport.subscribeConnectionLoss(
      (error) => {
        reportConnectionFailure(toErrorMessage(error));
      },
    );

    const initialize = async (): Promise<void> => {
      try {
        await transport.connect(SESSION_ID);
        const response = await transport.request(
          createBootstrapInitializeRequest(
            SESSION_ID,
            nextRequestId(),
            {
              client: { name: "openmat-web", version: "0.1.0-alpha.0" },
              supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
              capabilities: {
                executionModes: ["cell", "repl"],
                displayMimeTypes: [
                  "text/plain",
                  FIGURE_MIME_TYPE,
                  "application/vnd.openmat.plot+json",
                  "application/vnd.openmat.command-window-clear+json",
                ],
                maxPreviewElements: INSPECTION_ELEMENT_LIMIT,
                maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
                maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
                maxAggregateNodes: MAX_AGGREGATE_NODES,
                maxAggregateElements: MAX_AGGREGATE_ELEMENTS,
                maxAggregateDepth: MAX_AGGREGATE_DEPTH,
                interrupt: true,
                workspaceDelta: true,
              },
            },
          ),
        );
        if (disposed) {
          return;
        }
        if (!response.ok) {
          reportConnectionFailure(response.error.message);
          return;
        }
        if (response.result.data.negotiatedProtocol !== KERNEL_PROTOCOL_V3) {
          reportConnectionFailure(
            `Kernel negotiated unsupported protocol ${response.result.data.negotiatedProtocol}`,
          );
          return;
        }

        dispatch({
          type: "connectionReady",
          protocol: response.result.data.negotiatedProtocol,
          capabilities: response.result.data.capabilities,
        });
        markKernelConnected(attemptGeneration);

        try {
          const variables = await fetchWorkspaceForProtocol(
            response.result.data.negotiatedProtocol,
          );
          if (!disposed) {
            dispatch({ type: "workspaceLoaded", variables });
          }
        } catch (error: unknown) {
          if (!disposed) {
            dispatch({ type: "operationFailed", message: toErrorMessage(error) });
          }
        }
      } catch (error: unknown) {
        reportConnectionFailure(toErrorMessage(error));
      }
    };

    void initialize();

    return () => {
      disposed = true;
      unsubscribe();
      unsubscribeConnectionLoss();
      void transport.disconnect();
    };
  }, [
    closeAllVariableEditors,
    fetchWorkspaceForProtocol,
    kernelConnectionGeneration,
    markKernelConnected,
    markKernelFailed,
    nextRequestId,
    transport,
  ]);

  const executeSource = useCallback(
    async (
      source: string,
      sourceName: string,
      mode: ExecuteMode,
      submittedCommand?: string,
    ) => {
      if (
        state.connection !== "connected" ||
        state.negotiatedProtocol === null ||
        state.activeRequestId !== null ||
        state.kernelStatus === "busy" ||
        state.capabilities?.executionModes.includes(mode) !== true
      ) {
        return;
      }

      closeAllVariableEditors();

      const requestId = nextRequestId();
      const request = createKernelRequest(
        state.negotiatedProtocol,
        SESSION_ID,
        requestId,
        "execute",
        { code: source, sourceName, mode },
      );
      dispatch({
        type: "executionStarted",
        requestId,
        ...(submittedCommand === undefined
          ? {}
          : { command: submittedCommand }),
      });

      try {
        const response = await transport.request(request);
        if (!response.ok) {
          dispatch({ type: "executionFailed", message: response.error.message });
          return;
        }
        dispatch({ type: "executionFinished", requestId });
        if (response.result.data.interrupted) {
          dispatch({ type: "systemMessage", message: "Execution interrupted." });
        }

        // A negotiated workspaceDelta stream is ordered before the execute
        // response on the same WebSocket. Avoid immediately requesting and
        // rebuilding the complete workspace a second time.
        if (state.capabilities.workspaceDelta !== true) {
          try {
            const variables = await fetchWorkspaceForProtocol(
              state.negotiatedProtocol,
            );
            dispatch({ type: "workspaceLoaded", variables });
          } catch (error: unknown) {
            dispatch({ type: "operationFailed", message: toErrorMessage(error) });
          }
        }
      } catch (error: unknown) {
        dispatch({ type: "executionFailed", message: toErrorMessage(error) });
      }
    },
    [
      fetchWorkspaceForProtocol,
      closeAllVariableEditors,
      nextRequestId,
      state.activeRequestId,
      state.capabilities,
      state.connection,
      state.kernelStatus,
      state.negotiatedProtocol,
      transport,
    ],
  );

  const runEditor = useCallback(() => {
    if (activeDocument !== null) {
      const sourceName =
        activeDocument.rootPath === folder.rootPath &&
        activeDocument.rootGeneration === folder.rootGeneration
          ? activeDocument.path
          : workspaceSourceName(activeDocument.rootPath, activeDocument.path);
      void executeSource(activeDocument.content, sourceName, "cell");
    }
  }, [activeDocument, executeSource, folder.rootGeneration, folder.rootPath]);

  const submitCommand = useCallback(
    (value: string) => {
      const submitted = value.trim();
      if (
        submitted.length === 0 ||
        state.connection !== "connected" ||
        state.negotiatedProtocol === null ||
        state.activeRequestId !== null ||
        state.kernelStatus === "busy" ||
        state.capabilities?.executionModes.includes("repl") !== true
      ) {
        return;
      }
      setCommandHistory((current) => [...current, submitted]);
      setCommand("");
      void executeSource(submitted, "Command Window", "repl", submitted);
    },
    [
      executeSource,
      state.activeRequestId,
      state.capabilities,
      state.connection,
      state.kernelStatus,
      state.negotiatedProtocol,
    ],
  );

  const clearWorkspaceVariable = useCallback(
    (variable: VariableSummary) => {
      if (!/^[A-Za-z][A-Za-z0-9_]*$/u.test(variable.name)) {
        dispatch({
          type: "operationFailed",
          message: `Cannot safely clear workspace variable ${variable.name}.`,
        });
        return;
      }
      void executeSource(
        `clear ${variable.name}`,
        "Workspace",
        "repl",
      );
    },
    [executeSource],
  );

  const clearAllWorkspaceVariables = useCallback(() => {
    void windowManager
      .openDialog<"clear" | "cancel">({
        label: "Clear workspace",
        dismissResult: "cancel",
        render: ({ close }) => (
          <ClearWorkspaceDialog
            variableCount={state.workspace.length}
            onConfirm={() => close("clear")}
            onCancel={() => close("cancel")}
          />
        ),
      })
      .then((decision) => {
        if (decision === "clear") {
          void executeSource("clear", "Workspace", "repl");
        }
      });
  }, [executeSource, state.workspace.length, windowManager]);

  const cancelExecution = useCallback(async () => {
    if (
      state.activeRequestId === null ||
      state.negotiatedProtocol === null
    ) {
      return;
    }

    const request = createKernelRequest(
      state.negotiatedProtocol,
      SESSION_ID,
      nextRequestId(),
      "interrupt",
      {},
    );
    try {
      const response = await transport.request(request);
      if (!response.ok) {
        dispatch({ type: "operationFailed", message: response.error.message });
      } else if (!response.result.data.accepted) {
        dispatch({
          type: "systemMessage",
          message: "Kernel reported that no active execution accepted the interrupt.",
        });
      }
    } catch (error: unknown) {
      dispatch({ type: "operationFailed", message: toErrorMessage(error) });
    }
  }, [
    nextRequestId,
    state.activeRequestId,
    state.negotiatedProtocol,
    transport,
  ]);

  const requestVariableRange = useCallback(
    async (variable: VariableSummary, range: MatrixRange) => {
      if (
        state.connection !== "connected" ||
        state.negotiatedProtocol === null ||
        state.kernelStatus === "busy"
      ) {
        return;
      }

      const editorRequestSequence =
        (variableEditorRequestSequences.current.get(variable.name) ?? 0) + 1;
      variableEditorRequestSequences.current.set(
        variable.name,
        editorRequestSequence,
      );
      const maxElements = Math.min(
        state.capabilities?.maxPreviewElements ?? INSPECTION_ELEMENT_LIMIT,
        INSPECTION_ELEMENT_LIMIT,
      );
      try {
        const response = await transport.request(
          createKernelRequest(
            state.negotiatedProtocol,
            SESSION_ID,
            nextRequestId(),
            "inspect",
            {
              name: variable.name,
              range,
              maxElements,
            },
          ),
        );
        if (
          editorRequestSequence !==
          variableEditorRequestSequences.current.get(variable.name)
        ) {
          return;
        }
        if (!response.ok) {
          setVariableEditors((current) => {
            const editor = current.get(variable.name);
            if (editor === undefined) {
              return current;
            }
            const next = new Map(current);
            next.set(variable.name, {
              ...editor,
              loading: false,
              error: response.error.message,
            });
            return next;
          });
          return;
        }
        const data = response.result.data;
        if (!("revision" in data) || !("preview" in data)) {
          throw new Error("Kernel returned an unversioned inspect response.");
        }
        setVariableEditors((current) => {
          const editor = current.get(variable.name);
          if (editor === undefined) {
            return current;
          }
          const next = new Map(current);
          next.set(variable.name, {
            ...editor,
            preview: data.preview,
            revision: data.revision,
            loading: false,
            error: null,
          });
          return next;
        });
      } catch (error: unknown) {
        if (
          editorRequestSequence !==
          variableEditorRequestSequences.current.get(variable.name)
        ) {
          return;
        }
        setVariableEditors((current) => {
          const editor = current.get(variable.name);
          if (editor === undefined) {
            return current;
          }
          const next = new Map(current);
          next.set(variable.name, {
            ...editor,
            loading: false,
            error: toErrorMessage(error),
          });
          return next;
        });
      }
    },
    [
      nextRequestId,
      state.capabilities?.maxPreviewElements,
      state.connection,
      state.kernelStatus,
      state.negotiatedProtocol,
      transport,
    ],
  );

  const maxInspectionElements = Math.min(
    state.capabilities?.maxPreviewElements ?? INSPECTION_ELEMENT_LIMIT,
    INSPECTION_ELEMENT_LIMIT,
  );

  const openVariable = useCallback(
    (variable: VariableSummary) => {
      if (
        state.connection !== "connected" ||
        state.negotiatedProtocol === null ||
        state.kernelStatus === "busy" ||
        state.activeRequestId !== null
      ) {
        return;
      }
      if (variableEditors.has(variable.name)) {
        setSelectedVariableName(variable.name);
        windowManager.restoreWindow(variableEditorWindowId(variable.name));
        return;
      }
      const range = createVariableEditorRange(
        variable.dimensions,
        maxInspectionElements,
      );
      setSelectedVariableName(variable.name);
      setVariableEditors((current) => {
        const next = new Map(current);
        next.set(variable.name, {
          variable,
          range,
          preview: null,
          revision: null,
          loading: true,
          error: null,
        });
        return next;
      });
      void requestVariableRange(variable, range);
    },
    [
      maxInspectionElements,
      requestVariableRange,
      state.activeRequestId,
      state.connection,
      state.kernelStatus,
      state.negotiatedProtocol,
      variableEditors,
      windowManager,
    ],
  );

  const changeVariableRange = useCallback(
    (variableName: string, range: MatrixRange) => {
      const editor = variableEditors.get(variableName);
      if (editor === undefined) {
        return;
      }
      const variable = editor.variable;
      setVariableEditors((current) => {
        if (!current.has(variableName)) {
          return current;
        }
        const next = new Map(current);
        next.set(variableName, {
          variable,
          range,
          preview: null,
          revision: null,
          loading: true,
          error: null,
        });
        return next;
      });
      void requestVariableRange(variable, range);
    },
    [requestVariableRange, variableEditors],
  );

  const reloadVariableEditor = useCallback(
    (variableName: string) => {
      const editor = variableEditors.get(variableName);
      if (editor === undefined) {
        return;
      }
      setVariableEditors((current) => {
        const currentEditor = current.get(variableName);
        if (currentEditor === undefined) {
          return current;
        }
        const next = new Map(current);
        next.set(variableName, {
          ...currentEditor,
          preview: null,
          revision: null,
          loading: true,
          error: null,
        });
        return next;
      });
      void requestVariableRange(editor.variable, editor.range);
    },
    [requestVariableRange, variableEditors],
  );

  const commitVariableElement = useCallback(
    async (
      variableName: string,
      edit: VariableEditorCellEdit,
    ): Promise<VariableEditorCommitResult> => {
      const editor = variableEditors.get(variableName);
      if (
        editor === undefined ||
        editor.revision === null ||
        state.connection !== "connected" ||
        state.negotiatedProtocol !== KERNEL_PROTOCOL_V3 ||
        state.kernelStatus === "busy"
      ) {
        return {
          ok: false,
          conflict: false,
          message: "The variable is not ready for editing.",
        };
      }

      try {
        const response = await transport.request(
          createKernelRequest(
            KERNEL_PROTOCOL_V3,
            SESSION_ID,
            nextRequestId(),
            "setVariableElement",
            {
              name: variableName,
              indices: edit.indices,
              value: edit.value,
              expectedRevision: editor.revision,
            },
          ),
        );
        if (!response.ok) {
          const conflict =
            response.error.category === "workspace.revisionConflict";
          return {
            ok: false,
            conflict,
            message: conflict
              ? "The workspace changed after this page loaded. Reload before applying the edit."
              : response.error.message,
          };
        }

        const updatedVariable = response.result.data.variable;
        dispatch({
          type: "workspaceLoaded",
          variables: state.workspace.some(
            (variable) => variable.name === updatedVariable.name,
          )
            ? state.workspace.map((variable) =>
                variable.name === updatedVariable.name
                  ? updatedVariable
                  : variable,
              )
            : [...state.workspace, updatedVariable],
        });
        setVariableEditors((current) => {
          const currentEditor = current.get(variableName);
          if (currentEditor === undefined) {
            return current;
          }
          const next = new Map(current);
          next.set(variableName, {
            ...currentEditor,
            variable: updatedVariable,
            revision: response.result.data.revision,
            loading: true,
            error: null,
          });
          return next;
        });
        await requestVariableRange(updatedVariable, editor.range);
        return { ok: true };
      } catch (error: unknown) {
        return {
          ok: false,
          conflict: false,
          message: toErrorMessage(error),
        };
      }
    },
    [
      nextRequestId,
      requestVariableRange,
      state.connection,
      state.kernelStatus,
      state.negotiatedProtocol,
      state.workspace,
      transport,
      variableEditors,
    ],
  );

  const closeVariableEditor = useCallback((variableName: string) => {
    variableEditorRequestSequences.current.set(
      variableName,
      (variableEditorRequestSequences.current.get(variableName) ?? 0) + 1,
    );
    setVariableEditors((current) => {
      if (!current.has(variableName)) {
        return current;
      }
      const next = new Map(current);
      next.delete(variableName);
      return next;
    });
  }, []);

  const editorDirty =
    activeDocument !== null &&
    isDocumentDirty(activeDocument);

  const navigateCurrentFolder = useCallback(
    async (path: string): Promise<string | null> => {
      if (!folderReady || workspaceBusy) {
        return null;
      }
      setWorkspaceBusy(true);
      setWorkspaceError(null);
      try {
        const directory = await workspaceClient.changeDirectory(path);
        if (currentFolderGeneration.current !== directory.generation) {
          currentFolderGeneration.current = directory.generation;
          installCurrentFolderSnapshot(await workspaceClient.list("", false));
        }
        return directory.path;
      } catch (error: unknown) {
        setWorkspaceError(toWorkspaceError(error));
        return null;
      } finally {
        setWorkspaceBusy(false);
      }
    },
    [folderReady, installCurrentFolderSnapshot, workspaceBusy, workspaceClient],
  );

  const browseDirectoryPicker = useCallback(
    async (path: string): Promise<void> => {
      const sequence = directoryPickerRequestSequence.current + 1;
      directoryPickerRequestSequence.current = sequence;
      setDirectoryPickerLoading(true);
      setDirectoryPickerError(null);
      try {
        const snapshot = await workspaceClient.browseDirectories(path);
        if (directoryPickerRequestSequence.current === sequence) {
          setDirectoryPickerSnapshot(snapshot);
        }
      } catch (error: unknown) {
        if (directoryPickerRequestSequence.current === sequence) {
          setDirectoryPickerError(toWorkspaceError(error).message);
        }
      } finally {
        if (directoryPickerRequestSequence.current === sequence) {
          setDirectoryPickerLoading(false);
        }
      }
    },
    [workspaceClient],
  );

  const closeDirectoryPicker = useCallback(() => {
    directoryPickerRequestSequence.current += 1;
    setDirectoryPickerOpen(false);
    setDirectoryPickerSnapshot(null);
    setDirectoryPickerLoading(false);
    setDirectoryPickerError(null);
  }, []);

  const openDirectoryPicker = useCallback(async () => {
    if (!folderReady || workspaceBusy || nativeFileBusy.current) {
      return;
    }
    if (platform.files !== undefined) {
      nativeFileBusy.current = true;
      setWorkspaceBusy(true);
      setWorkspaceError(null);
      try {
        const path = await platform.files.pickDirectory(folder.rootPath);
        if (path !== null) {
          await workspaceClient.changeDirectory(path);
          installCurrentFolderSnapshot(await workspaceClient.list("", false));
        }
      } catch (error) {
        setWorkspaceError(toWorkspaceError(error));
      } finally {
        nativeFileBusy.current = false;
        setWorkspaceBusy(false);
      }
      return;
    }
    setDirectoryPickerOpen(true);
    setDirectoryPickerSnapshot(null);
    setDirectoryPickerError(null);
    void browseDirectoryPicker(folder.rootPath);
  }, [browseDirectoryPicker, folder.rootPath, folderReady, installCurrentFolderSnapshot, platform, workspaceBusy, workspaceClient]);

  const selectDirectoryPickerFolder = useCallback(async () => {
    if (
      directoryPickerSnapshot === null ||
      directoryPickerLoading ||
      workspaceBusy ||
      !folderReady
    ) {
      return;
    }
    setWorkspaceBusy(true);
    setWorkspaceError(null);
    setDirectoryPickerError(null);
    try {
      const directory = await workspaceClient.changeDirectory(
        directoryPickerSnapshot.path,
      );
      if (currentFolderGeneration.current !== directory.generation) {
        currentFolderGeneration.current = directory.generation;
        installCurrentFolderSnapshot(await workspaceClient.list("", false));
      }
      closeDirectoryPicker();
    } catch (error: unknown) {
      const workspaceError = toWorkspaceError(error);
      setWorkspaceError(workspaceError);
      setDirectoryPickerError(workspaceError.message);
    } finally {
      setWorkspaceBusy(false);
    }
  }, [
    closeDirectoryPicker,
    directoryPickerLoading,
    directoryPickerSnapshot,
    folderReady,
    installCurrentFolderSnapshot,
    workspaceBusy,
    workspaceClient,
  ]);

  const refreshWorkspace = useCallback(async () => {
    const paths = [...loadedWorkspacePaths.current];
    const snapshots = await Promise.all(
      paths.map((path) => workspaceClient.list(path, false)),
    );
    const root = snapshots.find((snapshot) => snapshot.path === "");
    if (root === undefined) {
      throw new Error("Workspace root was not loaded.");
    }
    const entries = new Map<string, WorkspaceEntry>();
    for (const snapshot of snapshots) {
      for (const entry of snapshot.entries) {
        entries.set(entry.path, entry);
      }
    }
    setFolder({ ...root, entries: [...entries.values()] });
    setFolderServiceError(null);
  }, [workspaceClient]);

  const queueWorkspaceRefresh = useCallback(async () => {
    if (workspaceRefreshInFlight.current !== null) {
      workspaceRefreshQueued.current = true;
      return workspaceRefreshInFlight.current;
    }
    const refresh = async (): Promise<void> => {
      do {
        workspaceRefreshQueued.current = false;
        await refreshWorkspace();
      } while (workspaceRefreshQueued.current);
    };
    const inFlight = refresh();
    workspaceRefreshInFlight.current = inFlight;
    try {
      await inFlight;
    } finally {
      if (workspaceRefreshInFlight.current === inFlight) {
        workspaceRefreshInFlight.current = null;
      }
    }
  }, [refreshWorkspace]);

  const refreshCurrentFolder = useCallback(async () => {
    if (!folderReady || workspaceRefreshing) {
      return;
    }
    setWorkspaceRefreshing(true);
    setWorkspaceError(null);
    try {
      await queueWorkspaceRefresh();
    } catch (error: unknown) {
      setWorkspaceError(toWorkspaceError(error));
    } finally {
      setWorkspaceRefreshing(false);
    }
  }, [folderReady, queueWorkspaceRefresh, workspaceRefreshing]);

  useEffect(() => {
    let disposed = false;
    let refreshTimer: number | null = null;
    const unsubscribe = workspaceClient.onWorkspaceChanged((change) => {
      if (change.rootGeneration !== currentFolderGeneration.current) {
        return;
      }
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
      }
      refreshTimer = window.setTimeout(() => {
        refreshTimer = null;
        if (
          disposed ||
          change.rootGeneration !== currentFolderGeneration.current
        ) {
          return;
        }
        setWorkspaceRefreshing(true);
        void queueWorkspaceRefresh()
          .catch((error: unknown) => {
            if (!disposed) {
              setWorkspaceError(toWorkspaceError(error));
            }
          })
          .finally(() => {
            if (!disposed) {
              setWorkspaceRefreshing(false);
            }
          });
      }, 60);
    });
    return () => {
      disposed = true;
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
      }
      unsubscribe();
    };
  }, [queueWorkspaceRefresh, workspaceClient]);

  const loadWorkspaceDirectory = useCallback(
    async (path: string) => {
      if (
        loadedWorkspacePaths.current.has(path) ||
        loadingWorkspacePaths.current.has(path)
      ) {
        return;
      }
      loadingWorkspacePaths.current.add(path);
      setWorkspaceError(null);
      try {
        const snapshot = await workspaceClient.list(path, false);
        loadedWorkspacePaths.current.add(path);
        setFolder((current) => ({
          ...current,
          entries: [
            ...current.entries.filter(
              (entry) => workspaceParentPath(entry.path) !== path,
            ),
            ...snapshot.entries,
          ],
        }));
      } catch (error: unknown) {
        setWorkspaceError(toWorkspaceError(error));
      } finally {
        loadingWorkspacePaths.current.delete(path);
      }
    },
    [workspaceClient],
  );

  const toggleWorkspaceDirectory = useCallback(
    (path: string) => {
      const expanding = !expandedWorkspacePaths.has(path);
      setExpandedWorkspacePaths((current) => {
        const next = new Set(current);
        if (next.has(path)) {
          next.delete(path);
        } else {
          next.add(path);
        }
        return next;
      });
      if (expanding) {
        void loadWorkspaceDirectory(path);
      }
    },
    [expandedWorkspacePaths, loadWorkspaceDirectory],
  );

  const addWorkspaceSearchPath = useCallback(
    async (entry: WorkspaceEntry, recursive: boolean) => {
      if (entry.kind !== "directory" || workspaceBusy || !folderReady) {
        return;
      }
      setWorkspaceBusy(true);
      setWorkspaceError(null);
      try {
        if (typeof workspaceClient.addSearchPath !== "function") {
          throw new WorkspaceClientError(
            "workspace.unsupportedRequest",
            "This workspace service does not support MATLAB search paths.",
          );
        }
        setSearchPath(
          await workspaceClient.addSearchPath(entry.path, recursive, "begin"),
        );
      } catch (error: unknown) {
        setWorkspaceError(toWorkspaceError(error));
      } finally {
        setWorkspaceBusy(false);
      }
    },
    [folderReady, workspaceBusy, workspaceClient],
  );

  const removeWorkspaceSearchPath = useCallback(
    async (entry: WorkspaceEntry, recursive: boolean) => {
      if (entry.kind !== "directory" || workspaceBusy || !folderReady) {
        return;
      }
      setWorkspaceBusy(true);
      setWorkspaceError(null);
      try {
        if (typeof workspaceClient.removeSearchPath !== "function") {
          throw new WorkspaceClientError(
            "workspace.unsupportedRequest",
            "This workspace service does not support MATLAB search paths.",
          );
        }
        setSearchPath(await workspaceClient.removeSearchPath(entry.path, recursive));
      } catch (error: unknown) {
        setWorkspaceError(toWorkspaceError(error));
      } finally {
        setWorkspaceBusy(false);
      }
    },
    [folderReady, workspaceBusy, workspaceClient],
  );

  const openWorkspaceFile = useCallback(
    async (entry: WorkspaceEntry, rootPath = folder.rootPath) => {
      if (entry.kind !== "file" || workspaceBusy) {
        return;
      }
      setEditorReveal(null);
      editorNavigationSequence.current += 1;
      const requestedDocumentId = documentId(rootPath, entry.path);
      const existing = openDocumentsRef.current.find(
        (document) => document.id === requestedDocumentId,
      );
      if (existing !== undefined && existing.recoveryStatus !== "unavailable") {
        setActiveDocumentId(existing.id);
        setSelectedWorkspacePath(entry.path);
        setEditorError(null);
        return;
      }
      const sequence = (documentOpenSequences.current.get(requestedDocumentId) ?? 0) + 1;
      documentOpenSequences.current.set(requestedDocumentId, sequence);
      setEditorLoading(activeDocument === null);
      setEditorError(null);
      try {
        const file = await workspaceClient.read(entry.path);
        if (file.rootPath !== rootPath) {
          throw new Error("Current Folder changed while opening the file. Please try again.");
        }
        if (sequence !== documentOpenSequences.current.get(requestedDocumentId)) {
          return;
        }
        const persistedExisting =
          existing === undefined
            ? null
            : snapshotDocumentSession([existing], existing.id).documents[0] ?? null;
        const opened =
          persistedExisting === null
            ? openDocumentFromWorkspaceFile(file)
            : restoreDocument(persistedExisting, file, "");
        setOpenDocuments((current) => {
          const existingIndex = current.findIndex(
            (document) => document.id === opened.id,
          );
          if (existingIndex < 0) {
            return [...current, opened];
          }
          return current.map((document, index) =>
            index === existingIndex ? opened : document,
          );
        });
        setActiveDocumentId(opened.id);
        setSelectedWorkspacePath(file.path);
      } catch (error: unknown) {
        if (sequence === documentOpenSequences.current.get(requestedDocumentId)) {
          setEditorError(toErrorMessage(error));
          setWorkspaceError(toWorkspaceError(error));
        }
      } finally {
        if (sequence === documentOpenSequences.current.get(requestedDocumentId)) {
          setEditorLoading(false);
        }
      }
    }, [
      activeDocument,
      folder.rootPath,
      workspaceBusy,
      workspaceClient,
    ],
  );

  const openNativeFile = useCallback(async () => {
    if (!platform.files || !folderReady || workspaceBusy || nativeFileBusy.current) return;
    nativeFileBusy.current = true;
    setWorkspaceBusy(true);
    setWorkspaceError(null);
    try {
      const selected = await platform.files.pickFile(folder.rootPath);
      if (selected === null) return;
      const directory = await workspaceClient.changeDirectory(selected.directory);
      installCurrentFolderSnapshot(await workspaceClient.list("", false));
      if (selected.name.toLowerCase().endsWith(".omui")) {
        setDesignerOpenRequest(previous => ({ path: selected.name, serial: (previous?.serial ?? 0) + 1 }));
        setDesignerMounted(true);
        setDesignerVisible(true);
      } else {
        const existing = openDocumentsRef.current.find(document =>
          nativeFileKey(workspaceSourceName(document.rootPath, document.path)) === nativeFileKey(selected.path));
        if (existing && existing.recoveryStatus !== "unavailable") {
          setActiveDocumentId(existing.id);
          setSelectedWorkspacePath(selected.name);
          setEditorReveal(null);
          setEditorError(null);
        } else {
          await openWorkspaceFile({ name: selected.name, path: selected.name, kind: "file", size: null, revision: null }, directory.path);
        }
      }
    } catch (error) {
      setWorkspaceError(toWorkspaceError(error));
    } finally {
      nativeFileBusy.current = false;
      setWorkspaceBusy(false);
    }
  }, [folder.rootPath, folderReady, installCurrentFolderSnapshot, openWorkspaceFile, platform, workspaceBusy, workspaceClient]);

  const revealNativePath = useCallback(async (path: string) => {
    if (!platform.files || workspaceBusy) return;
    try {
      await platform.files.revealPath(folder.rootPath, path);
    } catch (error) {
      setWorkspaceError(toWorkspaceError(error));
    }
  }, [folder.rootPath, platform, workspaceBusy]);

  const selectWorkspaceEntry = useCallback(
    (entry: WorkspaceEntry) => {
      setWorkspaceError(null);
      if (entry.kind === "directory") {
        setSelectedWorkspacePath(entry.path);
        toggleWorkspaceDirectory(entry.path);
      } else if (entry.kind === "file") {
        if (entry.path.toLowerCase().endsWith(".omui")) {
          setDesignerOpenRequest(previous => ({
            path: entry.path,
            serial: (previous?.serial ?? 0) + 1,
          }));
          setDesignerMounted(true);
          setDesignerVisible(true);
        } else {
          void openWorkspaceFile(entry);
        }
      } else {
        setSelectedWorkspacePath(entry.path);
        setWorkspaceError({
          code: "workspace.unsupportedEntry",
          message: `'${entry.path}' is not a regular file or directory.`,
        });
      }
    },
    [openWorkspaceFile, toggleWorkspaceDirectory],
  );

  const downloadWorkspaceEntry = useCallback(
    async (entry: WorkspaceEntry): Promise<void> => {
      if (entry.kind !== "file" || workspaceBusy || !folderReady) {
        return;
      }
      setWorkspaceBusy(true);
      setWorkspaceError(null);
      try {
        const download = await workspaceClient.prepareDownload(
          entry.path,
          folder.rootGeneration,
        );
        startBrowserDownload(download.url, download.name);
      } catch (error: unknown) {
        setWorkspaceError(toWorkspaceError(error));
      } finally {
        setWorkspaceBusy(false);
      }
    },
    [folder.rootGeneration, folderReady, workspaceBusy, workspaceClient],
  );

  const uploadWorkspaceFiles = useCallback(
    async (files: readonly File[], parentPath: string): Promise<void> => {
      if (files.length === 0 || workspaceBusy || !folderReady) {
        return;
      }
      setWorkspaceBusy(true);
      setWorkspaceError(null);
      let uploaded = 0;
      let firstError: { readonly code: string; readonly message: string } | null = null;
      let lastUploadedPath: string | null = null;
      try {
        for (const [index, file] of files.entries()) {
          const path = joinWorkspacePath(parentPath, file.name);
          setWorkspaceUploadStatus(
            `Uploading ${index + 1} of ${files.length}: ${file.name}`,
          );
          try {
            const entry = await workspaceClient.upload(
              path,
              file,
              folder.rootGeneration,
              false,
            );
            uploaded += 1;
            lastUploadedPath = entry.path;
          } catch (error: unknown) {
            firstError ??= toWorkspaceError(error);
          }
        }
        if (uploaded > 0) {
          loadedWorkspacePaths.current.add(parentPath);
          if (parentPath.length > 0) {
            setExpandedWorkspacePaths((current) => new Set(current).add(parentPath));
          }
          await refreshWorkspace();
          setSelectedWorkspacePath(lastUploadedPath);
        }
        if (firstError !== null) {
          setWorkspaceError({
            code: firstError.code,
            message:
              uploaded === 0
                ? firstError.message
                : `${uploaded} file${uploaded === 1 ? "" : "s"} uploaded; another upload failed: ${firstError.message}`,
          });
        }
      } catch (error: unknown) {
        setWorkspaceError(toWorkspaceError(error));
      } finally {
        setWorkspaceUploadStatus(null);
        setWorkspaceBusy(false);
      }
    }, [folder.rootGeneration, folderReady, refreshWorkspace, workspaceBusy, workspaceClient],
  );

  const copyWorkspaceRelativePath = useCallback(
    async (entry: WorkspaceEntry): Promise<void> => {
      setWorkspaceError(null);
      try {
        if (navigator.clipboard?.writeText === undefined) {
          throw new Error("Clipboard access requires a secure browser context.");
        }
        await navigator.clipboard.writeText(entry.path);
      } catch (error: unknown) {
        setWorkspaceError({
          code: "workspace.clipboardUnavailable",
          message: `Could not copy '${entry.path}': ${toErrorMessage(error)}`,
        });
        throw error;
      }
    },
    [],
  );

  const createFolderEntry = useCallback(
    async (name: string) => {
      if (creatingKind === null || workspaceBusy || !folderReady) {
        return;
      }
      const path = joinWorkspacePath(creatingParentPath, name);
      const opensInEditor = creatingKind === "file" && /\.m$/i.test(name);
      setWorkspaceBusy(true);
      setWorkspaceError(null);
      try {
        const created = await workspaceClient.create(path, creatingKind);
        setSelectedWorkspacePath(created.path);
        setFolder((current) => ({
          ...current,
          entries: [
            ...current.entries.filter((entry) => entry.path !== created.path),
            created,
          ],
        }));
        setCreatingKind(null);
        if (creatingParentPath.length > 0) {
          setExpandedWorkspacePaths((current) =>
            new Set(current).add(creatingParentPath),
          );
        }
        if (opensInEditor) {
          const file = await workspaceClient.read(created.path);
          const opened = openDocumentFromWorkspaceFile(file);
          setOpenDocuments((current) => {
            const withoutDuplicate = current.filter(
              (document) => document.id !== opened.id,
            );
            return [...withoutDuplicate, opened];
          });
          setActiveDocumentId(opened.id);
          setEditorError(null);
        }
        try {
          await refreshWorkspace();
        } catch (error: unknown) {
          setFolderServiceError(
            `Created ${created.path}, but refresh failed: ${toErrorMessage(error)}`,
          );
        }
      } catch (error: unknown) {
        setWorkspaceError(toWorkspaceError(error));
      } finally {
        setWorkspaceBusy(false);
      }
    },
    [
      creatingKind,
      creatingParentPath,
      folderReady,
      refreshWorkspace,
      workspaceBusy,
      workspaceClient,
    ],
  );

  const applyMovedPath = useCallback(
    (previousPath: string, nextPath: string) => {
      setSelectedWorkspacePath((current) =>
        current === null ? null : remapPath(current, previousPath, nextPath),
      );
      setExpandedWorkspacePaths(
        (current) =>
          new Set(
            [...current].map((path) => remapPath(path, previousPath, nextPath)),
          ),
      );
      loadedWorkspacePaths.current = new Set(
        [...loadedWorkspacePaths.current].map((path) =>
          remapPath(path, previousPath, nextPath),
        ),
      );
      const movedDocuments = openDocumentsRef.current.map((document) => {
        if (
          document.rootPath !== folder.rootPath ||
          document.rootGeneration !== folder.rootGeneration ||
          !pathIsWithin(document.path, previousPath)
        ) {
          return document;
        }
        const path = remapPath(document.path, previousPath, nextPath);
        const id = documentId(document.rootPath, path);
        const movePendingPath = (pending: string) => joinWorkspacePath(
          workspaceParentPath(path), pending.slice(pending.lastIndexOf("/") + 1),
        );
        const pendingPath = document.pendingPath === undefined
          ? undefined : movePendingPath(document.pendingPath);
        const stagedRename = sourceFileRenames.current.get(document.id);
        sourceFileRenames.current.delete(document.id);
        if (stagedRename !== undefined) {
          const stagedPath = movePendingPath(stagedRename.path);
          if (stagedPath !== path) {
            sourceFileRenames.current.set(id, { ...stagedRename, path: stagedPath });
          }
        }
        const { pendingPath: _pending, ...moved } = document;
        return {
          ...moved,
          ...(pendingPath === undefined || pendingPath === path ? {} : { pendingPath }),
          id,
          path,
          uri: workspaceDocumentUri(path, document.rootGeneration, document.rootPath),
          version: document.version + 1,
        };
      });
      openDocumentsRef.current = movedDocuments;
      setOpenDocuments(movedDocuments);
      if (
        activeDocument !== null &&
        activeDocument.rootPath === folder.rootPath &&
        activeDocument.rootGeneration === folder.rootGeneration &&
        pathIsWithin(activeDocument.path, previousPath)
      ) {
        setActiveDocumentId(
          documentId(
            activeDocument.rootPath,
            remapPath(activeDocument.path, previousPath, nextPath),
          ),
        );
      }
    },
    [activeDocument, folder.rootGeneration, folder.rootPath],
  );

  const renameWorkspaceEntry = useCallback(
    async (entry: WorkspaceEntry) => {
      if (workspaceBusy || savingDocumentIds.size > 0) {
        return;
      }
      const siblingNames = folder.entries
        .filter(
          (candidate) =>
            workspaceParentPath(candidate.path) === workspaceParentPath(entry.path),
        )
        .map((candidate) => candidate.name);
      await windowManager.openDialog<"renamed" | "cancel">({
        label: `Rename ${entry.name}`,
        dismissResult: "cancel",
        render: ({ close }) => (
          <RenameEntryDialog
            entry={entry}
            siblingNames={siblingNames}
            onRename={async (requested) => {
              setWorkspaceBusy(true);
              setWorkspaceError(null);
              try {
                const moved = await workspaceClient.rename(entry.path, requested);
                applyMovedPath(moved.previousPath, moved.entry.path);
              } catch (error: unknown) {
                return toWorkspaceError(error).message;
              } finally {
                setWorkspaceBusy(false);
              }
              try {
                await refreshWorkspace();
              } catch (error: unknown) {
                setWorkspaceError(toWorkspaceError(error));
              }
              return null;
            }}
            onComplete={() => close("renamed")}
            onCancel={() => close("cancel")}
          />
        ),
      });
    }, [
      applyMovedPath,
      folder.entries,
      refreshWorkspace,
      savingDocumentIds.size,
      workspaceBusy,
      workspaceClient,
      windowManager,
    ],
  );

  const moveWorkspaceEntry = useCallback(
    async (entry: WorkspaceEntry) => {
      if (workspaceBusy || savingDocumentIds.size > 0) {
        return;
      }
      await windowManager.openDialog<"moved" | "cancel">({
        label: `Move ${entry.name}`,
        dismissResult: "cancel",
        render: ({ close }) => (
          <MoveEntryDialog
            entry={entry}
            rootName={folder.rootName}
            loadDirectories={async (path) => {
              const snapshot = await workspaceClient.list(path, false);
              return snapshot.entries
                .filter((candidate) => candidate.kind === "directory")
                .map((candidate) => ({
                  name: candidate.name,
                  path: candidate.path,
                }))
                .sort((left, right) =>
                  left.name.localeCompare(right.name, undefined, {
                    numeric: true,
                    sensitivity: "base",
                  }),
                );
            }}
            onMove={async (requested) => {
              setWorkspaceBusy(true);
              setWorkspaceError(null);
              try {
                const moved = await workspaceClient.move(entry.path, requested);
                applyMovedPath(moved.previousPath, moved.entry.path);
              } catch (error: unknown) {
                return toWorkspaceError(error).message;
              } finally {
                setWorkspaceBusy(false);
              }
              try {
                await refreshWorkspace();
              } catch (error: unknown) {
                setWorkspaceError(toWorkspaceError(error));
              }
              return null;
            }}
            onComplete={() => close("moved")}
            onCancel={() => close("cancel")}
          />
        ),
      });
    }, [
      applyMovedPath,
      folder.rootName,
      refreshWorkspace,
      savingDocumentIds.size,
      workspaceBusy,
      workspaceClient,
      windowManager,
    ],
  );

  const deleteWorkspaceEntry = useCallback(
    async (entry: WorkspaceEntry) => {
      if (workspaceBusy || savingDocumentIds.size > 0) {
        return;
      }
      const deletedDocuments = openDocuments.filter(
        (document) =>
          document.rootPath === folder.rootPath &&
          document.rootGeneration === folder.rootGeneration &&
          pathIsWithin(document.path, entry.path),
      );
      const dirtyDeletedCount = deletedDocuments.filter(isDocumentDirty).length;
      await windowManager.openDialog<"deleted" | "cancel">({
        label: `Delete ${entry.name}`,
        dismissResult: "cancel",
        closeOnBackdrop: false,
        render: ({ close }) => (
          <DeleteEntryDialog
            entry={entry}
            dirtyDocumentCount={dirtyDeletedCount}
            loadImpact={async (): Promise<DeleteImpact> => {
              if (entry.kind !== "directory") {
                return { files: 1, directories: 0 };
              }
              const snapshot = await workspaceClient.list(entry.path, true);
              return snapshot.entries.reduce<DeleteImpact>(
                (impact, candidate) =>
                  candidate.kind === "directory"
                    ? { ...impact, directories: impact.directories + 1 }
                    : { ...impact, files: impact.files + 1 },
                { files: 0, directories: 1 },
              );
            }}
            onDelete={async () => {
              setWorkspaceBusy(true);
              setWorkspaceError(null);
              try {
                await workspaceClient.delete(
                  entry.path,
                  entry.kind === "directory",
                );
                if (deletedDocuments.length > 0) {
                  const deletedIds = new Set(
                    deletedDocuments.map((document) => document.id),
                  );
                  for (const id of deletedIds) {
                    documentOpenSequences.current.set(
                      id,
                      (documentOpenSequences.current.get(id) ?? 0) + 1,
                    );
                  }
                  const remaining = openDocuments.filter(
                    (document) => !deletedIds.has(document.id),
                  );
                  setOpenDocuments(remaining);
                  if (
                    activeDocumentId !== null &&
                    deletedIds.has(activeDocumentId)
                  ) {
                    setActiveDocumentId(remaining[0]?.id ?? null);
                  }
                  setEditorError(null);
                }
                setSelectedWorkspacePath((current) =>
                  current !== null && pathIsWithin(current, entry.path)
                    ? null
                    : current,
                );
                setExpandedWorkspacePaths(
                  (current) =>
                    new Set(
                      [...current].filter(
                        (path) => !pathIsWithin(path, entry.path),
                      ),
                    ),
                );
                loadedWorkspacePaths.current = new Set(
                  [...loadedWorkspacePaths.current].filter(
                    (path) => !pathIsWithin(path, entry.path),
                  ),
                );
              } catch (error: unknown) {
                return toWorkspaceError(error).message;
              } finally {
                setWorkspaceBusy(false);
              }
              try {
                await refreshWorkspace();
              } catch (error: unknown) {
                setWorkspaceError(toWorkspaceError(error));
              }
              return null;
            }}
            onComplete={() => close("deleted")}
            onCancel={() => close("cancel")}
          />
        ),
      });
    }, [
      activeDocumentId,
      folder.rootGeneration,
      folder.rootPath,
      openDocuments,
      refreshWorkspace,
      savingDocumentIds.size,
      workspaceBusy,
      workspaceClient,
      windowManager,
    ],
  );

  const replaceOpenDocument = useCallback(
    (
      documentIdToReplace: string,
      replace: (document: OpenDocument) => OpenDocument,
    ): OpenDocument | null => {
      let replaced: OpenDocument | null = null;
      const next = openDocumentsRef.current.map((document) => {
        if (document.id !== documentIdToReplace) return document;
        replaced = replace(document);
        return replaced;
      });
      if (replaced !== null) {
        openDocumentsRef.current = next;
        setOpenDocuments(next);
      }
      return replaced;
    },
    [],
  );

  const reloadDocumentFromDisk = useCallback(
    async (document: OpenDocument): Promise<boolean> => {
      try {
        const file = await workspaceClient.read(document.path);
        if (file.rootPath !== document.rootPath || file.rootGeneration !== document.rootGeneration) {
          throw new Error("Switch back to this document's workspace before reloading it.");
        }
        sourceFileRenames.current.delete(document.id);
        replaceOpenDocument(document.id, (current) => {
          const { pendingPath: _pending, ...reloaded } = current;
          return {
          ...reloaded,
          content: file.content,
          savedContent: file.content,
          revision: file.revision,
          version: current.version + 1,
          recoveryStatus: "none",
          recoveryMessage: null,
        }; });
        setFolder((current) =>
          current.rootGeneration === file.rootGeneration
            ? {
                ...current,
                entries: current.entries.map((candidate) =>
                  candidate.path === file.path
                    ? {
                        ...candidate,
                        size: file.size,
                        revision: file.revision,
                      }
                    : candidate,
                ),
              }
            : current,
        );
        setEditorError(null);
        return true;
      } catch (error: unknown) {
        const message = toWorkspaceError(error);
        setWorkspaceError(message);
        if (activeDocumentIdRef.current === document.id) {
          setEditorError(message.message);
        }
        return false;
      }
    },
    [replaceOpenDocument, workspaceClient],
  );

  const saveNativeDocumentAs = useCallback((document: OpenDocument): Promise<DocumentSaveOutcome> => pendingSaves.run(async () => {
    if (!platform.files || nativeFileBusy.current || workspaceBusy || !folderReady) {
      return { status: "kept", documentId: document.id };
    }
    nativeFileBusy.current = true;
    setWorkspaceBusy(true);
    setSavingDocumentIds(current => new Set(current).add(document.id));
    setEditorError(null);
    let savedPath: string | null = null;
    try {
      const name = (document.pendingPath ?? document.path).split("/").at(-1) ?? "script.m";
      const parent = workspaceParentPath(document.path);
      const initialDirectory = parent === "" ? document.rootPath : workspaceSourceName(document.rootPath, parent);
      const selected = await platform.files.saveTextFile(name, initialDirectory, document.content);
      if (selected === null) return { status: "kept", documentId: document.id };
      savedPath = selected.path;
      const directory = await workspaceClient.changeDirectory(selected.directory);
      installCurrentFolderSnapshot(await workspaceClient.list("", false));
      const file = await workspaceClient.read(selected.name);
      if (file.rootPath !== directory.path || file.rootGeneration !== directory.generation || file.content !== document.content) {
        throw new Error("The destination changed before it could be opened. Your original editor remains available.");
      }
      const opened = openDocumentFromWorkspaceFile(file);
      const target = openDocumentsRef.current.find(current => current.id !== document.id &&
        nativeFileKey(workspaceSourceName(current.rootPath, current.path)) === nativeFileKey(selected.path));
      if (target && isDocumentDirty(target)) {
        // Saving a copy must never discard a draft already open at the destination.
        setEditorError(`Saved to ${selected.path}. The destination has an open draft; both editors were preserved.`);
        return { status: "kept", documentId: document.id };
      }
      const source = openDocumentsRef.current.find(current => current.id === document.id);
      if (!source) throw new Error("The source editor was closed. The saved file is available in Current Folder.");
      const replacement: OpenDocument = {
        ...opened,
        content: source.content,
        savedContent: document.content,
        version: source.version + 1,
        viewState: source.viewState,
      };
      const next = openDocumentsRef.current.filter(current => current.id !== target?.id)
        .map(current => current.id === document.id ? replacement : current);
      sourceFileRenames.current.delete(document.id);
      openDocumentsRef.current = next;
      setOpenDocuments(next);
      setActiveDocumentId(replacement.id);
      setSelectedWorkspacePath(file.path);
      return { status: "copied", documentId: replacement.id };
    } catch (error) {
      setEditorError(`${savedPath === null ? "" : `Saved to ${savedPath}, but could not switch the editor: `}${toErrorMessage(error)}`);
      return { status: "failed", documentId: document.id };
    } finally {
      nativeFileBusy.current = false;
      setWorkspaceBusy(false);
      setSavingDocumentIds(current => {
        const next = new Set(current);
        next.delete(document.id);
        return next;
      });
    }
  }), [folderReady, installCurrentFolderSnapshot, pendingSaves, platform, workspaceBusy, workspaceClient]);

  const saveDocumentCopy = useCallback(
    async (document: OpenDocument): Promise<DocumentSaveOutcome> => {
      if (platform.files) return saveNativeDocumentAs(document);
      if (
        document.rootPath !== folder.rootPath ||
        document.rootGeneration !== folder.rootGeneration
      ) {
        setEditorError(
          "Switch Current Folder back to this document’s workspace before saving a copy.",
        );
        return { status: "failed", documentId: document.id };
      }
      const siblingNames = folder.entries
        .filter(
          (entry) =>
            workspaceParentPath(entry.path) === workspaceParentPath(document.path),
        )
        .map((entry) => entry.name);
      let copiedDocumentId = document.id;
      const decision = await windowManager.openDialog<"copied" | "cancel">({
        label: `Save a copy of ${document.path}`,
        dismissResult: "cancel",
        render: ({ close }) => (
          <SaveCopyDialog
            document={document}
            siblingNames={siblingNames}
            onSave={async (path) => {
              let created = false;
              try {
                const emptyEntry = await workspaceClient.create(path, "file");
                created = true;
                if (emptyEntry.revision === null) {
                  throw new WorkspaceClientError(
                    "workspace.invalidResponse",
                    "Workspace create did not return a file revision.",
                  );
                }
                const savedEntry = await workspaceClient.write(
                  path,
                  document.content,
                  emptyEntry.revision,
                  document.rootGeneration,
                );
                if (savedEntry.revision === null) {
                  throw new WorkspaceClientError(
                    "workspace.invalidResponse",
                    "Workspace save did not return a file revision.",
                  );
                }
                copiedDocumentId = documentId(document.rootPath, path);
                sourceFileRenames.current.delete(document.id);
                const replaced = replaceOpenDocument(document.id, (current) => {
                  const { pendingPath: _pending, ...copied } = current;
                  return {
                  ...copied,
                  id: copiedDocumentId,
                  path,
                  uri: workspaceDocumentUri(path, document.rootGeneration, document.rootPath),
                  savedContent: document.content,
                  revision: savedEntry.revision!,
                  version: current.version + 1,
                  recoveryStatus: "none",
                  recoveryMessage: null,
                }; });
                if (replaced === null) {
                  return "The source editor was closed before the copy completed.";
                }
                setActiveDocumentId((current) =>
                  current === document.id ? copiedDocumentId : current,
                );
                setSelectedWorkspacePath(path);
                setFolder((current) => ({
                  ...current,
                  entries: [
                    ...current.entries.filter(
                      (candidate) => candidate.path !== savedEntry.path,
                    ),
                    savedEntry,
                  ],
                }));
                setEditorError(null);
                return null;
              } catch (error: unknown) {
                if (created) {
                  try {
                    await workspaceClient.delete(path, false);
                  } catch {
                    // The empty copy is recoverable and will appear after refresh.
                  }
                }
                return toWorkspaceError(error).message;
              }
            }}
            onComplete={() => close("copied")}
            onCancel={() => close("cancel")}
          />
        ),
      });
      return decision === "copied"
        ? { status: "copied", documentId: copiedDocumentId }
        : { status: "kept", documentId: document.id };
    },
    [folder.entries, folder.rootGeneration, folder.rootPath, platform, replaceOpenDocument, saveNativeDocumentAs, windowManager, workspaceClient],
  );

  const resolveDocumentSaveConflict = useCallback(
    async (
      document: OpenDocument,
      message: string,
    ): Promise<DocumentSaveOutcome> => {
      const decision = await windowManager.openDialog<SaveConflictDecision>({
        label: `Resolve save conflict for ${document.path}`,
        dismissResult: "keep",
        closeOnBackdrop: false,
        render: ({ close }) => (
          <SaveConflictDialog
            document={document}
            message={message}
            onDecision={close}
          />
        ),
      });
      if (decision === "reload") {
        return (await reloadDocumentFromDisk(document))
          ? { status: "reloaded", documentId: document.id }
          : { status: "failed", documentId: document.id };
      }
      if (decision === "saveCopy") {
        return saveDocumentCopy(document);
      }
      if (document.recoveryStatus === "conflict") {
        replaceOpenDocument(document.id, (current) => ({
          ...current,
          recoveryStatus: "none",
          recoveryMessage: null,
        }));
      }
      return { status: "kept", documentId: document.id };
    },
    [reloadDocumentFromDisk, replaceOpenDocument, saveDocumentCopy, windowManager],
  );

  const saveDocument = useCallback(
    (saving: OpenDocument, allowConflictDialog = true): Promise<DocumentSaveOutcome> => pendingSaves.run(async () => {
      if (saving.recoveryStatus === "unavailable") {
        setEditorError(saving.recoveryMessage ?? "Switch back to this document's Current Folder to verify the recovered file before saving.");
        return { status: "failed", documentId: saving.id };
      }
      if (saving.recoveryStatus === "conflict") {
        if (!allowConflictDialog) return { status: "failed", documentId: saving.id };
        return resolveDocumentSaveConflict(
          saving,
          saving.recoveryMessage ??
            "This recovered editor was based on an older disk revision.",
        );
      }
      if (!isDocumentDirty(saving)) {
        return { status: "saved", documentId: saving.id };
      }
      setSavingDocumentIds((current) => new Set(current).add(saving.id));
      setEditorError(null);
      try {
        if (saving.pendingPath !== undefined) {
          const root = await workspaceClient.currentDirectory();
          if (root.path !== saving.rootPath || root.generation !== saving.rootGeneration) {
            throw new Error("Switch back to this document's workspace before saving its file rename.");
          }
          try {
            await workspaceClient.read(saving.pendingPath);
            throw new Error(`Cannot rename the source: ${saving.pendingPath} already exists.`);
          } catch (error: unknown) {
            if (!(error instanceof WorkspaceClientError && error.code === "workspace.notFound")) throw error;
          }
        }
        let revision = saving.revision;
        if (revision === "") {
          const root = await workspaceClient.currentDirectory();
          if (root.path !== saving.rootPath || root.generation !== saving.rootGeneration) {
            throw new Error("Switch back to this document's workspace before creating the source file.");
          }
          const created = await workspaceClient.create(saving.path, "file");
          if (created.revision === null) throw new Error("Workspace create did not return a file revision.");
          revision = created.revision;
          // Remember successful creation even if the subsequent write fails.
          // Retrying saves into this file with its revision rather than creating again.
          replaceOpenDocument(saving.id, (current) => ({
            ...current, revision, savedContent: "",
          }));
        }
        const entry = await workspaceClient.write(
          saving.path,
          saving.content,
          revision,
          saving.rootGeneration,
        );
        if (entry.revision === null) {
          throw new WorkspaceClientError(
            "workspace.invalidResponse",
            "Workspace save did not return a file revision.",
          );
        }
        replaceOpenDocument(saving.id, (current) => ({
          ...current,
          savedContent: saving.content,
          revision: entry.revision!,
          recoveryStatus: "none",
          recoveryMessage: null,
        }));
        let finalEntry = entry;
        let finalId = saving.id;
        if (saving.pendingPath !== undefined && openDocumentsRef.current.find(
          (document) => document.id === saving.id,
        )?.pendingPath === saving.pendingPath) {
          const result = await workspaceClient.rename(
            saving.path, saving.pendingPath.slice(saving.pendingPath.lastIndexOf("/") + 1),
            saving.rootGeneration,
          );
          if (result.entry.path !== saving.pendingPath) throw new Error("Workspace rename returned an unexpected path.");
          finalEntry = result.entry;
          finalId = documentId(saving.rootPath, saving.pendingPath);
          replaceOpenDocument(saving.id, (current) => {
            const { pendingPath: _pending, ...saved } = current;
            return { ...saved, id: finalId, path: saving.pendingPath!,
              uri: workspaceDocumentUri(saving.pendingPath!, saving.rootGeneration, saving.rootPath),
              revision: finalEntry.revision ?? entry.revision!, version: current.version + 1 };
          });
          sourceFileRenames.current.delete(saving.id);
          setActiveDocumentId((current) => current === saving.id ? finalId : current);
          setSelectedWorkspacePath((current) => current === saving.path ? saving.pendingPath! : current);
        }
        setFolder((current) =>
          current.rootGeneration === saving.rootGeneration
            ? {
                ...current,
                entries: current.entries.some((candidate) => candidate.path === saving.path)
                  ? current.entries.map((candidate) => candidate.path === saving.path ? finalEntry : candidate)
                  : [...current.entries, finalEntry],
              }
            : current,
        );
        return { status: "saved", documentId: finalId };
      } catch (error: unknown) {
        if (isWorkspaceRevisionConflict(error) && allowConflictDialog) {
          return resolveDocumentSaveConflict(saving, error.message);
        }
        const message = toWorkspaceError(error);
        setWorkspaceError(message);
        if (activeDocumentIdRef.current === saving.id) {
          setEditorError(message.message);
        }
        return { status: "failed", documentId: saving.id };
      } finally {
        setSavingDocumentIds((current) => {
          const next = new Set(current);
          next.delete(saving.id);
          return next;
        });
      }
    }),
    [pendingSaves, replaceOpenDocument, resolveDocumentSaveConflict, workspaceClient],
  );

  desktopCloseHandler.current = () => {
    if (!platform.lifecycle || desktopCloseInFlight.current) return;
    desktopCloseInFlight.current = true;
    window.dispatchEvent(new Event(DESKTOP_CLOSE_PREPARE_EVENT));
    const actions: DesktopCloseActions = {
      async waitForWrites() {
        await pendingSaves.wait();
        await platform.waitForWrites?.();
        await waitForCommit();
        if (designerMounted && designerSession.current === null) throw new Error("App Designer is still loading. Please retry shortly.");
        if (designerSession.current?.busy) throw new Error("App Designer is still opening a file. Please wait and retry.");
      },
      unsavedNames() {
        const names = openDocumentsRef.current.filter(isDocumentDirty).map((document) => workspaceSourceName(document.rootPath, document.path));
        if (designerSession.current?.dirty) names.push(`App Designer: ${designerSession.current.path}`);
        return names;
      },
      async saveAll() {
        if (designerSession.current?.dirty && !await designerSession.current.save()) {
          throw new Error(designerSession.current?.error ?? "Could not save App Designer. Check its source or recovery conflict and retry.");
        }
        await waitForCommit();
        for (const document of openDocumentsRef.current.filter(isDocumentDirty)) {
          const current = openDocumentsRef.current.find((candidate) => candidate.id === document.id);
          if (!current || !isDocumentDirty(current)) continue;
          const outcome = await saveDocument(current, false);
          if (outcome.status !== "saved") throw new Error(`Could not save ${current.path}. Check the Current Folder, file permissions, or resolve its conflict in the editor.`);
        }
        await waitForCommit();
        const remaining = actions.unsavedNames();
        if (remaining.length > 0) throw new Error(`There are still unsaved changes in ${remaining.join(", ")}. Please retry Save All.`);
      },
      async finish(discard) {
        const designer = designerSession.current;
        const documents = discard ? openDocumentsRef.current.flatMap((document) => {
          if (document.revision === "") return [];
          const { pendingPath: _pending, ...saved } = document;
          return [{ ...saved, content: document.savedContent, recoveryStatus: "none" as const, recoveryMessage: null }];
        }) : openDocumentsRef.current;
        const restoreDesigner = designerMounted && !(discard && designer && !designer.hasSavedDesign);
        await documentPersistence.flush(documents, { rootPath: folderRef.current.rootPath,
          designerMounted: Boolean(restoreDesigner), designerVisible: Boolean(restoreDesigner && designerVisible) });
        designer?.flush(discard);
        await platform.lifecycle!.close();
      },
      resume() { documentPersistence.resume(); designerSession.current?.resume(); },
    };
    void windowManager.openDialog<void>({
      label: "Close OpenMat", dismissResult: undefined, closeOnEscape: false, closeOnBackdrop: false,
      render: ({ close }) => <DesktopCloseDialog actions={actions} onClosed={() => close(undefined)} />,
    }).finally(() => { desktopCloseInFlight.current = false; });
  };

  const saveActiveDocument = useCallback(() => {
    if (activeDocument === null || !editorDirty || editorSaving) return;
    void saveDocument(activeDocument);
  }, [activeDocument, editorDirty, editorSaving, saveDocument]);

  const removeOpenDocument = useCallback((closingId: string): void => {
    sourceFileRenames.current.delete(closingId);
    const current = openDocumentsRef.current;
    const closingIndex = current.findIndex((document) => document.id === closingId);
    if (closingIndex < 0) return;
    documentOpenSequences.current.set(
      closingId,
      (documentOpenSequences.current.get(closingId) ?? 0) + 1,
    );
    const remaining = current.filter((document) => document.id !== closingId);
    openDocumentsRef.current = remaining;
    setOpenDocuments(remaining);
    setActiveDocumentId((active) =>
      active === closingId
        ? (remaining[Math.min(closingIndex, remaining.length - 1)]?.id ?? null)
        : active,
    );
    if (activeDocumentIdRef.current === closingId) {
      setEditorError(null);
    }
  }, []);

  const closeDocument = useCallback(
    (closingId: string) => {
      const closing = openDocumentsRef.current.find(
        (document) => document.id === closingId,
      );
      if (closing === undefined || savingDocumentIds.has(closingId)) return;
      if (!isDocumentDirty(closing)) {
        removeOpenDocument(closingId);
        return;
      }
      void windowManager
        .openDialog<UnsavedChangesDecision>({
          label: `Close ${closing.path}`,
          dismissResult: "cancel",
          closeOnBackdrop: false,
          render: ({ close }) => (
            <UnsavedChangesDialog document={closing} onDecision={close} />
          ),
        })
        .then(async (decision) => {
          if (decision === "discard") {
            removeOpenDocument(closing.id);
            return;
          }
          if (decision !== "save") return;
          const outcome = await saveDocument(closing);
          if (
            outcome.status === "saved" ||
            outcome.status === "reloaded" ||
            outcome.status === "copied"
          ) {
            removeOpenDocument(outcome.documentId);
          }
        });
    },
    [removeOpenDocument, saveDocument, savingDocumentIds, windowManager],
  );

  const activateDocument = useCallback(
    (documentIdToActivate: string) => {
      const document = openDocumentsRef.current.find(
        (candidate) => candidate.id === documentIdToActivate,
      );
      if (document === undefined) {
        return;
      }
      setActiveDocumentId(document.id);
      editorNavigationSequence.current += 1;
      setEditorReveal(null);
      setEditorLoading(false);
      setEditorError(null);
      if (
        document.rootPath === folder.rootPath &&
        document.rootGeneration === folder.rootGeneration
      ) {
        setSelectedWorkspacePath(document.path);
      }
    },
    [folder.rootGeneration, folder.rootPath],
  );

  const ensureLspDocuments = useCallback(async (uris: readonly string[]): Promise<boolean> => {
    const requested = [...new Set(uris)];
    if (requested.length > 128) throw new Error("Too many files in one refactoring operation.");
    const root = folderRef.current;
    const missing = requested.filter((uri) => !openDocumentsRef.current.some(
      (document) => documentUriKey(document.uri) === documentUriKey(uri),
    ));
    const targets = missing.map((uri) => {
      const target = parseWorkspaceDocumentUri(uri);
      if (target === null || target.rootPath !== root.rootPath ||
        target.rootGeneration !== root.rootGeneration || !target.path.toLowerCase().endsWith(".m")) {
        throw new Error("The language target is outside the current workspace.");
      }
      return target;
    });
    // Fetch the entire batch before changing any editor state. A failed read must
    // never leave a partially prepared workspace refactoring behind.
    const files = await Promise.all(targets.map(async (target) => {
      const file = await workspaceClient.read(target.path);
      if (file.rootPath !== target.rootPath || file.rootGeneration !== target.rootGeneration ||
        file.path !== target.path) throw new Error("The workspace changed while opening language targets.");
      return file;
    }));
    if (folderRef.current.rootPath !== root.rootPath ||
      folderRef.current.rootGeneration !== root.rootGeneration) {
      throw new Error("The workspace changed while opening language targets.");
    }
    const next = [...openDocumentsRef.current];
    for (const file of files) {
      const opened = openDocumentFromWorkspaceFile(file);
      if (!next.some((document) => document.id === opened.id)) next.push(opened);
    }
    if (files.length > 0) {
      openDocumentsRef.current = next;
      setOpenDocuments(next);
    }
    // Rename re-requests immediately after loading. didOpen must precede it,
    // without relying on a later React render/effect to synchronize the models.
    editorSession.syncDocuments(next);
    return true;
  }, [editorSession, workspaceClient]);

  const openLspDocument = useCallback<
    NonNullable<CodeEditorProps["onOpenDocument"]>
  >(
    (uri, selection) => {
      const navigationSequence = ++editorNavigationSequence.current;
      const key = documentUriKey(uri);
      if (key === null) return false;
      const revealDocument = (document: OpenDocument): boolean => {
        activateDocument(document.id);
        setDesignerVisible(false);
        if (selection !== undefined) {
          editorRevealSequence.current += 1;
          setEditorReveal({
            documentUri: document.uri,
            lineNumber: selection.startLineNumber,
            column: selection.startColumn,
            endLineNumber: selection.endLineNumber,
            endColumn: selection.endColumn,
            requestId: editorRevealSequence.current,
          });
        }
        return true;
      };
      const existing = openDocumentsRef.current.find(
        (candidate) => documentUriKey(candidate.uri) === key,
      );
      if (existing !== undefined) return revealDocument(existing);
      const target = parseWorkspaceDocumentUri(uri);
      if (target === null || target.rootPath !== folderRef.current.rootPath ||
        target.rootGeneration !== folderRef.current.rootGeneration) return false;
      return ensureLspDocuments([uri]).then(() => {
        if (navigationSequence !== editorNavigationSequence.current) return false;
        const document = openDocumentsRef.current.find(
          (candidate) => documentUriKey(candidate.uri) === key,
        );
        return document !== undefined && revealDocument(document);
      }).catch((error: unknown) => {
        setEditorError(toErrorMessage(error));
        return false;
      });
    },
    [activateDocument, ensureLspDocuments],
  );

  const updateDocumentViewState = useCallback(
    (documentIdToUpdate: string, viewState: DocumentViewState) => {
      setOpenDocuments((current) =>
        current.map((document) =>
          document.id === documentIdToUpdate &&
          (document.viewState === null ||
            document.viewState.lineNumber !== viewState.lineNumber ||
            document.viewState.column !== viewState.column ||
            document.viewState.scrollTop !== viewState.scrollTop ||
            document.viewState.scrollLeft !== viewState.scrollLeft)
            ? { ...document, viewState }
            : document,
        ),
      );
    },
    [],
  );

  const resolveActiveDocumentConflict = useCallback(() => {
    if (activeDocument?.recoveryStatus !== "conflict") return;
    void resolveDocumentSaveConflict(
      activeDocument,
      activeDocument.recoveryMessage ??
        "This recovered editor was based on an older disk revision.",
    );
  }, [activeDocument, resolveDocumentSaveConflict]);

  const updateWorkspaceDocumentContent = useCallback(
    (documentIdToUpdate: string, nextCode: string) => {
      const document = openDocumentsRef.current.find(
        (candidate) => candidate.id === documentIdToUpdate,
      );
      if (document === undefined || document.content === nextCode) return;
      const rename = sourceFileRenames.current.get(documentIdToUpdate);
      replaceOpenDocument(documentIdToUpdate, (current) => {
        const updated = { ...current, content: nextCode, version: current.version + 1 };
        if (rename !== undefined && nextCode === rename.renamedText) {
          return { ...updated, pendingPath: rename.path };
        }
        if (rename !== undefined && nextCode === rename.originalText) {
          const { pendingPath: _pending, ...restored } = updated;
          return restored;
        }
        if (rename !== undefined && current.pendingPath === undefined) {
          sourceFileRenames.current.delete(documentIdToUpdate);
        }
        return updated;
      });
      if (activeDocumentIdRef.current === documentIdToUpdate) {
        setEditorError(null);
      }
    },
    [replaceOpenDocument],
  );

  const updateActiveDocumentContent = useCallback(
    (nextCode: string) => {
      if (activeDocumentId !== null) {
        updateWorkspaceDocumentContent(activeDocumentId, nextCode);
      }
    },
    [activeDocumentId, updateWorkspaceDocumentContent],
  );

  const ensureDesignerSource = useCallback<DesignerSourceWorkspace["ensureSource"]>((source) => {
    const root = folderRef.current;
    if (root.rootPath !== folder.rootPath || root.rootGeneration !== folder.rootGeneration ||
      (source.file !== null && source.file.rootPath !== root.rootPath)) {
      throw new Error("The source belongs to a different workspace.");
    }
    const id = documentId(root.rootPath, source.path);
    const existing = openDocumentsRef.current.find((document) => document.id === id);
    if (existing !== undefined) return existing;
    const uri = workspaceDocumentUri(source.path, root.rootGeneration, root.rootPath);
    if (parseWorkspaceDocumentUri(uri) === null) throw new Error("Invalid source path.");
    const opened: OpenDocument = {
      id, uri, path: source.path, rootPath: root.rootPath, rootGeneration: root.rootGeneration,
      content: source.content, savedContent: source.savedContent,
      revision: source.file?.revision ?? "", version: 1,
      recoveryStatus: "none", recoveryMessage: null, viewState: null,
    };
    const next = [...openDocumentsRef.current, opened];
    openDocumentsRef.current = next;
    setOpenDocuments(next);
    setActiveDocumentId((current) => current ?? opened.id);
    editorSession.syncDocuments(next);
    return opened;
  }, [editorSession, folder.rootGeneration, folder.rootPath]);

  const acceptDesignerSourceSave = useCallback((file: WorkspaceFile) => {
    const id = documentId(file.rootPath, file.path);
    replaceOpenDocument(id, (current) => ({
      ...current, savedContent: file.content, revision: file.revision,
      recoveryStatus: "none", recoveryMessage: null,
    }));
  }, [replaceOpenDocument]);

  const updateDesignerSource = useCallback((id: string, content: string) => {
    if (!editorSession.editDocument(id, content)) updateWorkspaceDocumentContent(id, content);
  }, [editorSession, updateWorkspaceDocumentContent]);

  const stageSourceFileRenames = useCallback((renames: readonly OpenMatFileRename[]): boolean => {
    const staged: Array<{ id: string; path: string; originalText: string; renamedText: string }> = [];
    const targets = new Set<string>();
    for (const rename of renames) {
      const old = parseWorkspaceDocumentUri(rename.oldUri);
      const next = parseWorkspaceDocumentUri(rename.newUri);
      if (old === null || next === null || old.rootPath !== next.rootPath ||
        old.rootGeneration !== next.rootGeneration || old.rootPath !== folderRef.current.rootPath ||
        old.rootGeneration !== folderRef.current.rootGeneration || old.path === next.path ||
        workspaceParentPath(old.path) !== workspaceParentPath(next.path) ||
        !old.path.endsWith(".m") || !next.path.endsWith(".m") || targets.has(rename.newUri)) return false;
      const document = openDocumentsRef.current.find((candidate) => documentUriKey(candidate.uri) === documentUriKey(rename.oldUri));
      if (document === undefined || document.pendingPath !== undefined ||
        document.recoveryStatus !== "none" || openDocumentsRef.current.some((candidate) =>
          candidate.id !== document.id && candidate.rootPath === next.rootPath && candidate.path === next.path)) return false;
      targets.add(rename.newUri);
      staged.push({ id: document.id, path: next.path, originalText: rename.originalText, renamedText: rename.renamedText });
    }
    // Staging alone never changes a document or a file. Activate its pending path
    // only when Monaco reports that the matching versioned text edit was applied.
    for (const rename of staged) sourceFileRenames.current.set(rename.id, rename);
    return true;
  }, []);

  const designerSourceWorkspace = useMemo<DesignerSourceWorkspace>(() => ({
    documents: openDocuments, editorSession, lspUrl,
    getSource: (path) => openDocumentsRef.current.find(
      (document) => document.id === documentId(folder.rootPath, path),
    ),
    ensureSource: ensureDesignerSource,
    updateSource: updateDesignerSource,
    acceptSaved: acceptDesignerSourceSave,
    openDocument: openLspDocument,
  }), [acceptDesignerSourceSave, editorSession, ensureDesignerSource, folder.rootPath,
    lspUrl, openDocuments, openLspDocument, updateDesignerSource]);

  useEffect(() => {
    editorSession.configure({
      url: lspUrl,
      onChange: updateWorkspaceDocumentContent,
      ensureDocuments: ensureLspDocuments,
      stageFileRenames: stageSourceFileRenames,
    });
  }, [editorSession, ensureLspDocuments, lspUrl, stageSourceFileRenames, updateWorkspaceDocumentContent]);

  useEffect(() => {
    editorSession.syncDocuments(openDocuments);
  }, [editorSession, openDocuments]);

  useEffect(() => () => editorSession.dispose(), [editorSession]);

  useEffect(() => {
    if (
      selectedVariableName !== null &&
      !state.workspace.some(
        (variable) => variable.name === selectedVariableName,
      )
    ) {
      setSelectedVariableName(null);
    }
    const workspaceByName = new Map(
      state.workspace.map((variable) => [variable.name, variable]),
    );
    setVariableEditors((current) => {
      let changed = false;
      const next = new Map(current);
      for (const [name, editor] of current) {
        const variable = workspaceByName.get(name);
        if (variable === undefined) {
          variableEditorRequestSequences.current.set(
            name,
            (variableEditorRequestSequences.current.get(name) ?? 0) + 1,
          );
          next.delete(name);
          changed = true;
        } else if (variable !== editor.variable) {
          next.set(name, { ...editor, variable });
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [
    selectedVariableName,
    state.workspace,
  ]);

  const canRun =
    activeDocument !== null &&
    !editorLoading &&
    state.connection === "connected" &&
    state.negotiatedProtocol !== null &&
    state.capabilities?.executionModes.includes("cell") === true &&
    state.kernelStatus !== "busy" &&
    state.activeRequestId === null;
  const canSave =
    activeDocument !== null &&
    activeDocument.recoveryStatus === "none" &&
    editorDirty &&
    !editorLoading &&
    !editorSaving;
  const canSubmitCommand =
    state.connection === "connected" &&
    state.negotiatedProtocol !== null &&
    state.capabilities?.executionModes.includes("repl") === true &&
    state.kernelStatus !== "busy" &&
    state.activeRequestId === null;
  const commandWindowEnabled =
    state.connection === "connected" &&
    state.negotiatedProtocol !== null &&
    state.capabilities?.executionModes.includes("repl") === true;
  const canCancel =
    state.negotiatedProtocol !== null &&
    state.activeRequestId !== null &&
    state.capabilities?.interrupt === true;
  const canInspect =
    state.connection === "connected" &&
    state.negotiatedProtocol !== null &&
    state.kernelStatus !== "busy" &&
    state.activeRequestId === null;
  const editorTabs = useMemo(
    () =>
      openDocuments.map((document) => ({
        id: document.id,
        path: document.path,
        dirty: isDocumentDirty(document),
        saving: savingDocumentIds.has(document.id),
        recoveryStatus: document.recoveryStatus,
      })),
    [openDocuments, savingDocumentIds],
  );
  const figureDisplays = useMemo(
    () => [...state.figures, ...graphicsFigures],
    [graphicsFigures, state.figures],
  );
  const closeFigureDisplay = useCallback(
    (index: number) => {
      if (index < state.figures.length) {
        dispatch({ type: "figureClosed", index });
        return;
      }
      const graphicsIndex = index - state.figures.length;
      setGraphicsFigures((current) =>
        current.filter((_figure, itemIndex) => itemIndex !== graphicsIndex),
      );
    },
    [state.figures.length],
  );
  const reportDynamicAppError = useCallback((message: string) => {
    dispatch({
      type: "systemMessage",
      message: `Dynamic application failed to load: ${message}`,
    });
  }, []);
  const sessionConnection = summarizeSessionConnection(
    kernelConnection,
    workspaceConnection,
    state.kernelStatus,
  );
  const connectionError =
    (kernelConnection.phase === "connected" ? null : kernelConnection.error) ??
    (workspaceConnection.phase === "connected"
      ? null
      : workspaceConnection.error);
  const visibleError = connectionError ?? state.error;
  const automaticRetryLabel =
    sessionConnection.retryDelayMs === null
      ? null
      : `Retrying automatically in ${Math.max(
          0.1,
          sessionConnection.retryDelayMs / 1000,
        ).toLocaleString(undefined, { maximumFractionDigits: 1 })}s.`;
  const retryInterruptedConnections = useCallback((): void => {
    retryKernelNow();
    retryWorkspaceNow();
  }, [retryKernelNow, retryWorkspaceNow]);

  return (
    <div className="app-shell" data-theme={theme}>
      <header className="topbar">
        <a className="brand" href="#editor-title" aria-label="OpenMat editor">
          <span className="brand-mark" aria-hidden="true">
            OM
          </span>
          <span>
            <strong>OpenMat</strong>
            <small>MATLAB-compatible {platform.kind === "desktop" ? "desktop" : "web"} IDE · Alpha</small>
          </span>
        </a>
        <CurrentFolderAddressBar
          path={folder.rootPath}
          busy={workspaceBusy}
          disabled={!folderReady}
          onNavigate={navigateCurrentFolder}
          onChoose={() => void openDirectoryPicker()}
          onParent={() => void navigateCurrentFolder("..")}
        />
        <button
          className="icon-button settings-button"
          type="button"
          title="Settings"
          aria-label="Settings"
          aria-expanded={settingsOpen}
          aria-controls="settings-panel"
          onClick={() => setSettingsOpen((open) => !open)}
        >
          <SettingsIcon />
        </button>
        <button
          className="designer-launch-button"
          type="button"
          aria-expanded={designerVisible}
          onClick={() => {
            setDesignerMounted(true);
            setDesignerVisible(value => !value);
          }}
        >
          <ComponentIcon type="Window" />App Designer
        </button>
        <div
          className="connection-indicator"
          role="status"
          aria-live="polite"
          title={sessionConnection.detail}
        >
          <span
            className={`status-dot ${sessionConnection.phase} ${state.kernelStatus}`}
            aria-hidden="true"
          />
          <span>{sessionConnection.label}</span>
        </div>
      </header>

      {settingsOpen ? (
        <div id="settings-panel">
          <SettingsPanel
            theme={theme}
            onThemeChange={setTheme}
            onClose={() => setSettingsOpen(false)}
          />
        </div>
      ) : null}

      {directoryPickerOpen ? (
        <DirectoryPickerDialog
          initialPath={folder.rootPath}
          snapshot={directoryPickerSnapshot}
          loading={directoryPickerLoading}
          selecting={workspaceBusy}
          error={directoryPickerError}
          onBrowse={(path) => void browseDirectoryPicker(path)}
          onConfirm={() => void selectDirectoryPickerFolder()}
          onCancel={closeDirectoryPicker}
        />
      ) : null}

      {visibleError === null ? null : (
        <div className="error-banner" role="alert">
          <span>
            {visibleError}
            {automaticRetryLabel === null ? null : ` ${automaticRetryLabel}`}
          </span>
          {sessionConnection.phase === "recovering" ||
          sessionConnection.phase === "failed" ? (
            <button
              className="button button-secondary"
              type="button"
              onClick={retryInterruptedConnections}
            >
              Retry connection
            </button>
          ) : null}
        </div>
      )}

      <ResizableIdeGrid>
        <CurrentFolderPane
          openEditors={editorTabs}
          activeDocumentId={activeDocumentId}
          entries={folder.entries}
          rootName={folder.rootName}
          rootPath={folder.rootPath}
          serviceLabel={
            folderServiceError ??
            (workspaceClient.kind === "mock"
              ? workspaceClient.label
              : "Workspace root enforced by openmat-server")
          }
          selectedPath={selectedWorkspacePath}
          expandedPaths={expandedWorkspacePaths}
          searchPathWorkspacePaths={searchPathWorkspacePaths}
          loading={folderLoading}
          refreshing={workspaceRefreshing}
          canMutate={folderReady && savingDocumentIds.size === 0}
          creatingKind={creatingKind}
          creatingParentPath={creatingParentPath}
          operationBusy={workspaceBusy}
          uploadStatus={workspaceUploadStatus}
          error={workspaceError}
          onActivateDocument={activateDocument}
          onCloseDocument={closeDocument}
          onToggleDirectory={toggleWorkspaceDirectory}
          onRefresh={() => void refreshCurrentFolder()}
          onStartCreate={(kind, parentPath) => {
            setCreatingKind(kind);
            setCreatingParentPath(parentPath);
            setWorkspaceError(null);
          }}
          onCancelCreate={() => {
            setCreatingKind(null);
            setWorkspaceError(null);
          }}
          onCreate={(name) => void createFolderEntry(name)}
          onFocus={(entry) => setSelectedWorkspacePath(entry.path)}
          onSelect={selectWorkspaceEntry}
          onEnterDirectory={(path) => void navigateCurrentFolder(path)}
          onAddSearchPath={(entry, recursive) =>
            void addWorkspaceSearchPath(entry, recursive)
          }
          onRemoveSearchPath={(entry, recursive) =>
            void removeWorkspaceSearchPath(entry, recursive)
          }
          onDownload={(entry) => void downloadWorkspaceEntry(entry)}
          fileTransfers={platform.kind === "web"}
          onOpenFile={platform.files ? () => void openNativeFile() : undefined}
          onRevealPath={platform.files ? (path) => void revealNativePath(path) : undefined}
          onUpload={(files, parentPath) =>
            void uploadWorkspaceFiles(files, parentPath)
          }
          onCopyRelativePath={copyWorkspaceRelativePath}
          onRename={(entry) => void renameWorkspaceEntry(entry)}
          onMove={(entry) => void moveWorkspaceEntry(entry)}
          onDelete={(entry) => void deleteWorkspaceEntry(entry)}
        />
        <EditorPane
          activeDocumentId={activeDocumentId}
          documentPath={activeDocument?.path ?? null}
          documentUri={activeDocument?.uri ?? null}
          documentVersion={activeDocument?.version ?? 0}
          documentViewState={activeDocument?.viewState ?? null}
          code={activeDocument?.content ?? ""}
          loading={editorLoading}
          saving={editorSaving}
          error={editorError}
          recoveryStatus={activeDocument?.recoveryStatus ?? "none"}
          recoveryMessage={activeDocument?.recoveryMessage ?? null}
          canRun={canRun}
          canSave={canSave}
          canCancel={canCancel}
          theme={theme}
          onCodeChange={updateActiveDocumentContent}
          onViewStateChange={updateDocumentViewState}
          onRun={runEditor}
          onSave={() => void saveActiveDocument()}
          onSaveAs={platform.files ? () => { if (activeDocument) void saveNativeDocumentAs(activeDocument); } : undefined}
          canSaveAs={folderReady && !workspaceBusy && !editorLoading && activeDocument !== null}
          onCancel={() => void cancelExecution()}
          onResolveRecoveryConflict={resolveActiveDocumentConflict}
          lspUrl={lspUrl}
          editorSession={editorSession}
          pendingPath={activeDocument?.pendingPath ?? null}
          workspaceDocuments={openDocuments}
          onWorkspaceDocumentChange={updateWorkspaceDocumentContent}
          onOpenDocument={openLspDocument}
          reveal={editorReveal}
        />
        <CommandWindowPane
          entries={state.commandWindowEntries}
          command={command}
          history={commandHistory}
          enabled={commandWindowEnabled}
          canSubmit={canSubmitCommand}
          busy={state.kernelStatus === "busy"}
          onCommandChange={setCommand}
          onSubmit={submitCommand}
          onClear={() => dispatch({ type: "commandWindowCleared" })}
        />
        <WorkspacePane
          variables={state.workspace}
          selectedName={selectedVariableName}
          canOpen={canInspect}
          canClear={canSubmitCommand}
          onSelect={(variable) => setSelectedVariableName(variable.name)}
          onOpen={openVariable}
          onClear={clearWorkspaceVariable}
          onClearAll={clearAllWorkspaceVariables}
        />
        <CommandHistoryPane
          commands={commandHistory}
          onRecall={setCommand}
          onExecute={submitCommand}
        />
      </ResizableIdeGrid>

      <FigureWindow
        figures={figureDisplays}
        onClose={closeFigureDisplay}
      />

      {[...variableEditors.values()].map((variableEditor) => (
        <VariableEditorWindow
          key={variableEditor.variable.name}
          variable={variableEditor.variable}
          range={variableEditor.range}
          preview={variableEditor.preview}
          revision={variableEditor.revision}
          loading={variableEditor.loading}
          error={variableEditor.error}
          maxElements={maxInspectionElements}
          canNavigate={canInspect}
          canEdit={canInspect && state.negotiatedProtocol === KERNEL_PROTOCOL_V3}
          onRangeChange={(range) =>
            changeVariableRange(variableEditor.variable.name, range)
          }
          onCellCommit={(edit) =>
            commitVariableElement(variableEditor.variable.name, edit)
          }
          onReload={() =>
            reloadVariableEditor(variableEditor.variable.name)
          }
          onClose={() => closeVariableEditor(variableEditor.variable.name)}
        />
      ))}

      <OpenMatDynamicAppLoader
        sources={dynamicAppModules}
        onError={reportDynamicAppError}
      />
      <OpenMatWindowLayer />

      {designerMounted ? (
        <div hidden={!designerVisible}>
          <Suspense fallback={<div className="app-designer">加载 App Designer…</div>}>
            <AppDesigner
              key={`${folder.rootPath}:${folder.rootGeneration}`}
              workspace={workspaceClient}
              rootPath={folder.rootPath}
              rootGeneration={folder.rootGeneration}
              wsUrl={wsUrl ?? kernelWebSocketUrl()}
              theme={theme}
              openRequest={designerOpenRequest}
              onClose={() => setDesignerVisible(false)}
              onSaved={() => void refreshCurrentFolder()}
              sourceWorkspace={designerSourceWorkspace}
              visible={designerVisible}
              sessionRef={designerSession}
              pendingSaves={pendingSaves}
            />
          </Suspense>
        </div>
      ) : null}

      <footer className="statusbar">
        <span>
          {state.negotiatedProtocol === null
            ? "kernel bootstrap"
            : state.negotiatedProtocol.replace("openmat-", "")}
        </span>
        <span>UTF-8</span>
        <span>{activeDocument?.path ?? "no document"}</span>
        <span className="statusbar-spacer" />
        <span>
          {transport.kind === "mock"
            ? "Mock demo transport"
            : "WebSocket transport"}
        </span>
        <span>{openDocuments.length} open files</span>
        <span>{state.workspace.length} variables</span>
        <span>{folder.entries.length} folder items</span>
      </footer>
    </div>
  );
}

export function App(props: AppProps) {
  const [platform] = useState(() => props.platform ?? createPlatformServices());
  const [registry] = useState(() => {
    if (props.appRegistry === undefined) {
      return createOpenMatAppRegistry();
    }
    registerBuiltInOpenMatApps(props.appRegistry);
    return props.appRegistry;
  });
  return (
    <PlatformContext.Provider value={platform}>
      <OpenMatWindowManagerProvider registry={registry}>
        <AppWorkbench {...props} />
      </OpenMatWindowManagerProvider>
    </PlatformContext.Provider>
  );
}
