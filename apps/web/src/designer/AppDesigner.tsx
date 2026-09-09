import {
    lazy,
    Suspense,
    useCallback,
    useEffect,
    useLayoutEffect,
    useMemo,
    useReducer,
    useRef,
    useState,
    type DragEvent,
    type MouseEvent as ReactMouseEvent,
    type PointerEvent as ReactPointerEvent,
    type RefObject,
} from "react";
import { usePlatformServices } from "../platform/platform-services";
import { PendingOperations } from "../platform/pending-operations";
import type { DesignerSession } from "./designer-session";
import type { IdeTheme } from "../theme";
import type {
    WorkspaceClient,
    WorkspaceFile,
} from "../workspace/workspace-client";
import {
    WorkspaceClientError,
    workspaceDocumentUri,
} from "../workspace/workspace-client";
import {
    documentId,
    type OpenDocument,
} from "../documents/document-session";
import type { DesignerSourceWorkspace } from "../documents/designer-source-workspace";
import type { DisplayEventData } from "../protocol/kernel-v0";
import {
    BUILTIN_CATALOG,
    BUILTIN_COMPONENTS,
    propertyValue,
    type ComponentSpec,
} from "./catalog";
import { ComponentIcon } from "./ComponentIcon";
import { Inspector } from "./Inspector";
import { MultiInspector } from "./MultiInspector";
import { EditorTools } from "./EditorTools";
import { ContextMenu, MenuIcon, type MenuGroup } from "./ContextMenu";
import { LibraryPanels } from "./LibraryPanels";
import {
    arrangeNodes,
    convertLayout,
    duplicateNodes,
    freeCell,
    moveNodes,
    nudgeNodes,
    patchLayouts,
    patchProperties,
    removeNodes,
    reorderNodes,
    selectionRoots,
    wrapNodes,
    type Arrangement,
    type Placement,
} from "./operations";
import { measureNodes, nodeElements, placementAt } from "./canvas-geometry";
import {
    overlayPreview,
    resizeWindow,
    useCanvasEditing,
} from "./use-canvas-editing";
import { UiRenderer, type UiInteraction } from "./UiRenderer";
import {
    assertParent,
    createDocument,
    createComponentDocument,
    createNode,
    findNode,
    findParent,
    historyReducer,
    QUALIFIED_NAME,
    updateNode,
    validateDocument,
    walk,
    type UiDocument,
    type UiNode,
} from "./model";
import { parseUi, serializeUi } from "./xml";
import {
    applySnapshot,
    buildEventProgram,
    buildUiProgram,
    componentDescriptor,
    UiRuntimeSession,
    type UiPayload,
} from "./runtime";
import { signalExample, SIGNAL_CODE } from "./example";
import "./designer.css";
import { useComponentPreviews } from "./use-component-previews";
import {
    callbackPath,
    callbackLine,
    callbackTemplate,
    renamedComponentPath,
} from "./callbacks";
import {
    classTemplate,
    ensureClassMethod,
    inspectClassSource,
    methodOptions,
    methodTarget,
    syncAppClass,
    renameClassSource,
} from "./class-source";

const CodeEditor = lazy(() => import("../components/CodeEditor"));
const ignoreViewState = () => undefined;
const message = (error: unknown) =>
    error instanceof Error ? error.message : String(error);
interface Props {
    workspace: WorkspaceClient;
    rootPath: string;
    rootGeneration: number;
    wsUrl: string | undefined;
    theme: IdeTheme;
    openRequest: { path: string; serial: number } | null;
    onClose: () => void;
    onSaved: () => void;
    sourceWorkspace?: DesignerSourceWorkspace;
    visible?: boolean;
    sessionRef?: RefObject<DesignerSession | null>;
    pendingSaves?: PendingOperations;
}
interface Draft {
    restored?: boolean;
    document: UiDocument;
    code: string;
    path: string;
    savedXml: string;
    savedCode: string;
    file: WorkspaceFile | null;
    codeFile: WorkspaceFile | null;
    callbackFiles?: Record<string, CallbackSource>;
    xmlDraft?: string | null;
}
interface CallbackSource {
    code: string;
    savedCode: string;
    file: WorkspaceFile | null;
}
function draftKey(root: string): string {
    return `openmat.designer.draft.v1:${root}`;
}
function restoreDraft(root: string): Draft {
    try {
        const raw = localStorage.getItem(draftKey(root));
        if (raw) {
            const saved = JSON.parse(raw) as Omit<Draft, "document"> & {
                xml: string;
            };
            if (
                typeof saved.code === "string" &&
                typeof saved.path === "string"
            )
                return { ...saved, document: parseUi(saved.xml), restored: true };
        }
    } catch {
        /* Recovery must not prevent opening the editor. */
    }
    return {
        document: signalExample(),
        code: SIGNAL_CODE,
        path: "SignalApp.omui",
        savedXml: "",
        savedCode: "",
        file: null,
        codeFile: null,
    };
}
function controllerPath(document: UiDocument, path: string): string {
    const owner = document.appClass ?? document.controller;
    return owner ? callbackPath(owner, path) : "";
}
function sharedSource(document: OpenDocument): CallbackSource {
    return {
        code: document.content,
        savedCode: document.savedContent,
        file: document.revision
            ? {
                  path: document.path,
                  content: document.savedContent,
                  revision: document.revision,
                  size: document.savedContent.length,
                  rootGeneration: document.rootGeneration,
                  rootPath: document.rootPath,
              }
            : null,
    };
}
export default function AppDesigner({
    workspace,
    rootPath,
    rootGeneration,
    wsUrl,
    theme,
    openRequest,
    onClose,
    onSaved,
    sourceWorkspace,
    visible = true,
    sessionRef: workbenchSessionRef,
    pendingSaves: suppliedPendingSaves,
}: Props) {
    const platform = usePlatformServices();
    const [pendingSaves] = useState(() => suppliedPendingSaves ?? new PendingOperations());
    const recoveryPaused = useRef(false);
    const [initial] = useState(() => restoreDraft(rootPath));
    const [history, dispatch] = useReducer(historyReducer, {
        past: [],
        present: initial.document,
        future: [],
    });
    const document = history.present;
    const [selectedId, setPrimaryId] = useState(document.root.id);
    const [selectionIds, setSelectionIds] = useState<string[]>([
        document.root.id,
    ]);
    const selectMany = (ids: string[]) => {
        setSelectionIds(ids);
        setPrimaryId(ids.at(-1) ?? document.root.id);
    };
    const setSelectedId = (id: string) => selectMany([id]);
    const selectNode = (id: string, additive = false) => {
        if (!additive || id === document.root.id) {
            setSelectedId(id);
            return;
        }
        const ids = selectionIds.filter((key) => key !== document.root.id);
        selectMany(
            ids.includes(id) ? ids.filter((key) => key !== id) : [...ids, id],
        );
    };
    const [path, setPath] = useState(initial.path);
    const [file, setFile] = useState<WorkspaceFile | null>(initial.file);
    const [localCodeFile, setCodeFile] = useState<WorkspaceFile | null>(
        initial.codeFile,
    );
    const [localCode, setCode] = useState(initial.code);
    const [localSavedCode, setSavedCode] = useState(initial.savedCode);
    const [localCallbackFiles, setCallbackFiles] = useState<
        Record<string, CallbackSource>
    >(initial.callbackFiles ?? {});
    const primaryPath = controllerPath(document, path);
    const registeredSources = useRef(new Set<string>());
    const initialSourcesStarted = useRef(false);
    const resolveSource = (sourcePath: string, fallback: CallbackSource) => {
        const shared = sourceWorkspace?.getSource(sourcePath);
        return shared ? sharedSource(shared) : fallback;
    };
    const primarySource = resolveSource(primaryPath, {
        code: localCode,
        savedCode: localSavedCode,
        file: localCodeFile,
    });
    const { code, savedCode, file: codeFile } = primarySource;
    const callbackFiles = useMemo(
        () =>
            Object.fromEntries(
                Object.entries(localCallbackFiles).map(([sourcePath, source]) => [
                    sourcePath,
                    resolveSource(sourcePath, source),
                ]),
            ),
        [localCallbackFiles, sourceWorkspace],
    );
    const lastSharedSources = useRef(new Map<string, OpenDocument>());
    useEffect(() => {
        if (!sourceWorkspace) return;
        const previous = lastSharedSources.current;
        const current = new Map<string, OpenDocument>();
        for (const sourcePath of [primaryPath, ...Object.keys(localCallbackFiles)]) {
            const shared = sourceWorkspace.getSource(sourcePath);
            if (shared) current.set(sourcePath, shared);
            const prior = previous.get(sourcePath);
            if (shared === prior || (!shared && !prior)) continue;
            // The local snapshot is only recovery data. A closed/discarded
            // workbench draft must not be resurrected on the next visit.
            if (sourcePath === primaryPath) {
                const source = sharedSource(shared ?? prior!);
                setCode(shared ? source.code : source.savedCode);
                setSavedCode(source.savedCode);
                setCodeFile(source.file);
            } else {
                setCallbackFiles((sources) => {
                    const next = { ...sources };
                    if (shared) next[sourcePath] = sharedSource(shared);
                    else delete next[sourcePath];
                    return next;
                });
            }
        }
        lastSharedSources.current = current;
    }, [sourceWorkspace, primaryPath, localCallbackFiles]);
    const rememberSource = (
        sourcePath: string,
        source: CallbackSource,
        edited = false,
    ): CallbackSource => {
        if (!sourceWorkspace || !sourcePath) return source;
        registeredSources.current.add(sourcePath);
        const shared = sourceWorkspace.ensureSource({
            path: sourcePath,
            content: source.code,
            savedContent: source.savedCode,
            file: source.file,
        });
        if (edited) sourceWorkspace.updateSource(shared.id, source.code);
        return sharedSource(sourceWorkspace.getSource(sourcePath) ?? shared);
    };
    const editPrimarySource = (content: string) => {
        rememberSource(primaryPath, { ...primarySource, code: content }, true);
        setCode(content);
    };
    const assertSourceWritable = (sourcePath: string) => {
        const source = sourceWorkspace?.getSource(sourcePath);
        if (source && source.recoveryStatus !== "none") {
            throw new Error(
                source.recoveryMessage ??
                    `${sourcePath} 存在恢复冲突，请先在主编辑器中处理。`,
            );
        }
    };
    const [activeCallback, setActiveCallback] = useState<string | null>(null);
    const [openingCallback, setOpeningCallback] = useState(false);
    const [codeReveal, setCodeReveal] = useState<{
        lineNumber: number;
        requestId: number;
    } | null>(null);
    const callbackRequest = useRef(0);
    const [savedXml, setSavedXml] = useState(initial.savedXml);
    const [view, setView] = useState(typeof initial.xmlDraft === "string" ? "XML" : "设计");
    const [xmlDraft, setXmlDraft] = useState<string | null>(
        typeof initial.xmlDraft === "string" ? initial.xmlDraft : null,
    );
    const [inspectorTab, setInspectorTab] = useState("属性");
    const [filter, setFilter] = useState("");
    const [custom, setCustom] = useState<Record<string, ComponentSpec>>({});
    const catalog = useMemo(
        () => ({ ...BUILTIN_CATALOG, ...custom }),
        [custom],
    );
    const [className, setClassName] = useState("");
    const [registering, setRegistering] = useState(false);
    const [saving, setSaving] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [logs, setLogs] = useState<string[]>([
        "设计模式 · 拖入组件，或单击组件库添加。",
    ]);
    const [outputOpen, setOutputOpen] = useState(false);
    const [runtimeDocument, setRuntimeDocument] = useState<UiDocument | null>(
        null,
    );
    const runtimeDocumentRef = useRef<UiDocument | null>(null);
    runtimeDocumentRef.current = runtimeDocument;
    const [figures, setFigures] = useState<DisplayEventData[]>([]);
    const [busy, setBusy] = useState(false);
    const [running, setRunning] = useState(false);
    const [zoom, setZoom] = useState(0);
    const canvasRef = useRef<HTMLDivElement>(null);
    const boardRef = useRef<HTMLDivElement>(null);
    const sectionRef = useRef<HTMLElement>(null);
    const inspectorRef = useRef<HTMLDivElement>(null);
    const [renameRequest, setRenameRequest] = useState(0);
    const [contextMenu, setContextMenu] = useState<{
        x: number;
        y: number;
        target: HTMLElement;
        document: UiDocument;
    } | null>(null);
    const closeContextMenu = useCallback(
        (restoreFocus: boolean) => {
            setContextMenu(null);
            if (restoreFocus) {
                const target = contextMenu?.target;
                if (target?.isConnected) target.focus({ preventScroll: true });
                else sectionRef.current?.focus({ preventScroll: true });
            }
        },
        [contextMenu],
    );
    useEffect(() => {
        if (!renameRequest) return;
        const input = inspectorRef.current?.querySelector<HTMLInputElement>(
            '[aria-label="组件名称"]',
        );
        input?.focus();
        input?.select();
    }, [renameRequest]);
    const [dropHint, setDropHint] = useState("");
    const [canvasSpace, setCanvasSpace] = useState({ width: 740, height: 570 });
    const [widths, setWidths] = useState(() => {
        try {
            const value = JSON.parse(
                localStorage.getItem("openmat.designer.panels.v1") ?? "{}",
            );
            return {
                left: Math.max(180, Math.min(360, Number(value.left) || 220)),
                right: Math.max(240, Math.min(440, Number(value.right) || 290)),
            };
        } catch {
            return { left: 220, right: 290 };
        }
    });
    const [pendingOpen, setPendingOpen] = useState<string | null>(null);
    const sessionRef = useRef<UiRuntimeSession | null>(null);
    const metadataSessions = useRef(new Set<UiRuntimeSession>());
    const queue = useRef<UiInteraction[]>([]);
    const working = useRef(false);
    const disposed = useRef(false);
    const dragCleanup = useRef<(() => void) | null>(null);
    const runningDefinition = useRef<{
        document: UiDocument;
        catalog: Record<string, ComponentSpec>;
    } | null>(null);
    const currentXml = useMemo(() => serializeUi(document), [document]);
    const dirty =
        file?.path !== path ||
        currentXml !== savedXml ||
        (Boolean(document.appClass ?? document.controller) &&
            code !== savedCode) ||
        Object.values(callbackFiles).some(
            (source) => !source.file || source.code !== source.savedCode,
        ) ||
        (xmlDraft !== null && xmlDraft !== currentXml);
    const selected = findNode(document.root, selectedId) ?? document.root;
    const validIds = selectionIds.filter((id) => findNode(document.root, id));
    const selectedIds = validIds.length ? validIds : [selected.id];
    const selectedNodes = selectedIds.map((id) => findNode(document.root, id)!);
    const selectedRoots = selectionRoots(document, selectedIds);
    const parentModes = selectedNodes.map(
        (node) => findParent(document.root, node.id)?.layout.mode,
    );
    const parentMode = parentModes.every((mode) => mode === parentModes[0])
        ? parentModes[0]
        : undefined;
    const log = useCallback(
        (text: string) => setLogs((lines) => [...lines, text].slice(-150)),
        [],
    );
    const fail = useCallback(
        (e: unknown) => {
            const text = message(e);
            setError(text);
            log(text);
            setOutputOpen(true);
        },
        [log],
    );
    const readSharedSource = async (
        sourcePath: string,
        fallback: CallbackSource,
        restore = false,
        request?: number,
    ): Promise<CallbackSource | null> => {
        const existing = sourceWorkspace?.getSource(sourcePath);
        if (existing) {
            registeredSources.current.add(sourcePath);
            return sharedSource(existing);
        }
        let loaded: WorkspaceFile | null;
        try {
            loaded = await workspace.read(sourcePath);
        } catch (error) {
            if (!(error instanceof WorkspaceClientError) ||
                error.code !== "workspace.notFound") throw error;
            if (registeredSources.current.has(sourcePath)) {
                throw new Error(`${sourcePath} 已关闭且磁盘上不存在此文件，请重新创建源文件。`);
            }
            loaded = null;
        }
        if (request !== undefined &&
            (request !== callbackRequest.current || disposed.current)) return null;
        const source = loaded && (!restore || fallback.code === fallback.savedCode)
            ? { code: loaded.content, savedCode: loaded.content, file: loaded }
            : fallback;
        return rememberSource(sourcePath, source);
    };
    useEffect(() => {
        if (!sourceWorkspace || initialSourcesStarted.current) return;
        initialSourcesStarted.current = true;
        // A requested design must load its own controller before any example
        // template can claim the same shared document identity.
        if (openRequest) return;
        const request = ++callbackRequest.current;
        setOpeningCallback(true);
        void (async () => {
            try {
                if (primaryPath) {
                    const source = await readSharedSource(primaryPath, {
                        code: localCode,
                        savedCode: localSavedCode,
                        file: localCodeFile,
                    }, initial.restored === true, request);
                    if (!source) return;
                    setCode(source.code);
                    setSavedCode(source.savedCode);
                    setCodeFile(source.file);
                }
                for (const [sourcePath, fallback] of Object.entries(localCallbackFiles)) {
                    const source = await readSharedSource(sourcePath, fallback, true, request);
                    if (!source) return;
                    setCallbackFiles((current) => ({ ...current, [sourcePath]: source }));
                }
            } catch (error) {
                if (request === callbackRequest.current) fail(error);
            } finally {
                if (request === callbackRequest.current) setOpeningCallback(false);
            }
        })();
    }, [sourceWorkspace, openRequest]);
    const edit = (next: UiDocument, fromXml = false) => {
        try {
            if (xmlDraft !== null && !fromXml)
                throw new Error(
                    "XML 有未应用的修改，请先应用 XML 或还原草稿。",
                );
            validateDocument(next);
            dispatch({ type: "edit", document: next });
            setXmlDraft(null);
            setError(null);
            return true;
        } catch (e) {
            fail(e);
            return false;
        }
    };
    const changeNode = (node: UiNode) => {
        try {
            const old = findNode(document.root, node.id);
            if (old)
                node = convertLayout(
                    node,
                    old.layout,
                    measureNodes(boardRef.current, document, canvasScale),
                );
            return edit({
                ...document,
                root: updateNode(document.root, node.id, () => node),
            });
        } catch (error) {
            fail(error);
            return false;
        }
    };
    const persistDraft = useCallback(() => {
        localStorage.setItem(
                draftKey(rootPath),
                JSON.stringify({
                    xml: currentXml,
                    path,
                    code,
                    savedXml,
                    savedCode,
                    file,
                    codeFile,
                    callbackFiles,
                    xmlDraft,
                }),
            );
    }, [
        currentXml,
        code,
        path,
        savedXml,
        savedCode,
        file,
        codeFile,
        callbackFiles,
        rootPath,
        xmlDraft,
    ]);
    useEffect(() => {
        if (recoveryPaused.current) return;
        try { persistDraft(); } catch { /* The exit guard reports storage failures. */ }
    }, [persistDraft]);
    useEffect(() => {
        try {
            localStorage.setItem(
                "openmat.designer.panels.v1",
                JSON.stringify(widths),
            );
        } catch {
            /* Optional preference. */
        }
    }, [widths]);
    useEffect(() => {
        if (platform.kind === "desktop") return;
        const beforeUnload = (event: BeforeUnloadEvent) => {
            if (dirty) {
                event.preventDefault();
                event.returnValue = "";
            }
        };
        window.addEventListener("beforeunload", beforeUnload);
        return () => window.removeEventListener("beforeunload", beforeUnload);
    }, [dirty, platform.kind]);
    useEffect(() => {
        disposed.current = false;
        return () => {
            disposed.current = true;
            callbackRequest.current++;
            void sessionRef.current?.dispose();
            metadataSessions.current.forEach((s) => void s.dispose());
            dragCleanup.current?.();
        };
    }, []);
    useEffect(() => {
        const element = canvasRef.current;
        if (!element || typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver(() =>
            setCanvasSpace({
                width: element.clientWidth,
                height: element.clientHeight,
            }),
        );
        observer.observe(element);
        return () => observer.disconnect();
    }, [view]);
    const load = async (nextPath: string) => {
        if (running || saving) {
            fail(new Error("请先停止预览并等待保存完成。"));
            return;
        }
        setSaving(true);
        callbackRequest.current++;
        setOpeningCallback(false);
        try {
            const loaded = await workspace.read(nextPath);
            const next = parseUi(loaded.content);
            let loadedCode: WorkspaceFile | null = null;
            let loadedSource: CallbackSource = { code: "", savedCode: "", file: null };
            if (next.appClass || next.controller) {
                try {
                    loadedCode = await workspace.read(
                        controllerPath(next, nextPath),
                    );
                } catch (e) {
                    log(`回调文件未读取：${message(e)}`);
                }
                loadedSource = rememberSource(controllerPath(next, nextPath), {
                    code: loadedCode?.content ?? "",
                    savedCode: loadedCode?.content ?? "",
                    file: loadedCode,
                });
            }
            dispatch({ type: "reset", document: next });
            setSelectedId(next.root.id);
            setPath(nextPath);
            setFile(loaded);
            setCodeFile(loadedSource.file);
            setCallbackFiles({});
            setActiveCallback(null);
            setCodeReveal(null);
            setCode(loadedSource.code);
            setSavedCode(loadedSource.savedCode);
            setSavedXml(serializeUi(next));
            setXmlDraft(null);
            setError(null);
            log(`已打开 ${nextPath}`);
        } catch (e) {
            fail(e);
        } finally {
            setSaving(false);
        }
    };
    const lastOpenSerial = useRef<number | null>(null);
    useEffect(() => {
        if (!openRequest || lastOpenSerial.current === openRequest.serial)
            return;
        lastOpenSerial.current = openRequest.serial;
        if (dirty && (initial.restored || history.past.length > 0 ||
            path !== initial.path || localCode !== initial.code || xmlDraft !== null))
            setPendingOpen(openRequest.path);
        else void load(openRequest.path);
    });
    const saveText = async (
        nextPath: string,
        content: string,
        previous: WorkspaceFile | null,
        onCreated?: (file: WorkspaceFile) => void,
    ): Promise<WorkspaceFile> => {
        if (
            previous &&
            previous.rootPath !== rootPath
        )
            throw new Error(
                "工作目录已变化，请返回原目录保存，或另存为新文件。",
            );
        let revision = previous?.path === nextPath ? previous.revision : null;
        if (revision === null) {
            const created = await workspace.create(nextPath, "file");
            revision = created.revision;
            if (revision === null)
                revision = (await workspace.read(nextPath)).revision;
            const createdFile: WorkspaceFile = {
                path: nextPath,
                content: "",
                revision,
                size: 0,
                rootGeneration,
                rootPath,
            };
            if (nextPath !== path) sourceWorkspace?.acceptSaved(createdFile);
            if (onCreated) onCreated(createdFile);
            else if (nextPath === path) setFile(createdFile);
            else setCodeFile(createdFile);
        }
        const written = await workspace.write(
            nextPath,
            content,
            revision,
            rootGeneration,
        );
        return {
            path: nextPath,
            content,
            revision:
                written.revision ?? (await workspace.read(nextPath)).revision,
            size: written.size ?? content.length,
            rootGeneration,
            rootPath,
        };
    };
    const save = (fromExit = false): Promise<UiDocument | null> => pendingSaves.run(async () => {
        if (saving || (running && !fromExit) || openingCallback) return null;
        setSaving(true);
        setError(null);
        try {
            if (fromExit && running) await stop();
            const next = xmlDraft === null ? document : parseUi(xmlDraft);
            const nextSourcePath = controllerPath(next, path);
            if (sourceWorkspace) {
                if (nextSourcePath && !sourceWorkspace.getSource(nextSourcePath)) {
                    await readSharedSource(nextSourcePath, primarySource);
                }
                for (const [sourcePath, source] of Object.entries(callbackFiles)) {
                    if (!sourceWorkspace.getSource(sourcePath)) {
                        await readSharedSource(sourcePath, source);
                    }
                }
            }
            assertSourceWritable(nextSourcePath);
            Object.keys(callbackFiles).forEach(assertSourceWritable);
            const latestSource = resolveSource(nextSourcePath, primarySource);
            const nextCode =
                next.version !== 1
                    ? syncAppClass(
                          latestSource.code ||
                              classTemplate(
                                  next.appClass!,
                                  next.kind === "component",
                              ),
                          next,
                          catalog,
                      )
                    : latestSource.code;
            if (!path.toLowerCase().endsWith(".omui"))
                throw new Error("界面文件扩展名需要为 .omui。");
            if (
                (next.appClass || next.controller) &&
                (nextCode !== latestSource.savedCode ||
                    latestSource.file?.path !== nextSourcePath)
            ) {
                rememberSource(
                    nextSourcePath,
                    { ...latestSource, code: nextCode },
                    true,
                );
                setCode(nextCode);
                const result = await saveText(
                    nextSourcePath,
                    nextCode,
                    latestSource.file,
                );
                sourceWorkspace?.acceptSaved(result);
                setCodeFile(result);
                setSavedCode(nextCode);
            }
            const xml = serializeUi(next);
            for (const [sourcePath, fallback] of Object.entries(callbackFiles)) {
                assertSourceWritable(sourcePath);
                const source = resolveSource(sourcePath, fallback);
                if (source.file && source.code === source.savedCode) continue;
                const rememberFile = (file: WorkspaceFile) =>
                    setCallbackFiles((current) => ({
                        ...current,
                        [sourcePath]: { ...current[sourcePath]!, file },
                    }));
                const written = await saveText(
                    sourcePath,
                    source.code,
                    source.file,
                    rememberFile,
                );
                sourceWorkspace?.acceptSaved(written);
                setCallbackFiles((current) => ({
                    ...current,
                    [sourcePath]: {
                        ...current[sourcePath]!,
                        file: written,
                        savedCode: source.code,
                    },
                }));
            }
            const saved = await saveText(path, xml, file);
            setFile(saved);
            setSavedXml(xml);
            setXmlDraft(null);
            if (next !== document) dispatch({ type: "edit", document: next });
            log(`已保存 ${path}`);
            onSaved();
            if (next.appClass && wsUrl) await register(next.appClass);
            return next;
        } catch (e) {
            fail(e);
            return null;
        } finally {
            setSaving(false);
        }
    });
    useLayoutEffect(() => {
        if (!workbenchSessionRef) return;
        workbenchSessionRef.current = {
            path, dirty, error, busy: saving || openingCallback,
            hasSavedDesign: Boolean(file && savedXml),
            save: async () => await save(true) !== null,
            flush(discard) {
                recoveryPaused.current = true;
                if (!discard) { persistDraft(); return; }
                if (!file || !savedXml) { localStorage.removeItem(draftKey(rootPath)); return; }
                localStorage.setItem(draftKey(rootPath), JSON.stringify({
                    xml: savedXml, path: file.path, code: savedCode, savedXml, savedCode,
                    file, codeFile, xmlDraft: null,
                    callbackFiles: Object.fromEntries(Object.entries(callbackFiles)
                        .filter(([, source]) => source.file !== null)
                        .map(([sourcePath, source]) => [sourcePath, { ...source, code: source.savedCode }])),
                }));
            },
            resume() {
                if (!recoveryPaused.current) return;
                recoveryPaused.current = false;
                try { persistDraft(); } catch { /* Retain the in-memory draft. */ }
            },
        };
    });
    useEffect(() => () => { if (workbenchSessionRef) workbenchSessionRef.current = null; }, [workbenchSessionRef]);
    const openCallback = async (requestedHandler: string, event?: string) => {
        if (running || saving) return;
        const request = ++callbackRequest.current;
        setOpeningCallback(false);
        try {
            if (document.version !== 1) {
                let next = document;
                const handler =
                    requestedHandler || `app.${selected.name}${event}`;
                const target = event
                    ? methodTarget(handler, selected, document, catalog)
                    : {
                          className: document.appClass!,
                          methodName: "",
                          isApp: true,
                      };
                const primary = target.className === document.appClass;
                const sourcePath = callbackPath(target.className, path);
                let source = primary
                    ? {
                          code: code || classTemplate(document.appClass!),
                          savedCode,
                          file: codeFile,
                      }
                    : sourceWorkspace?.getSource(sourcePath)
                      ? sharedSource(sourceWorkspace.getSource(sourcePath)!)
                      : callbackFiles[sourcePath];
                if (sourceWorkspace && !sourceWorkspace.getSource(sourcePath) && source) {
                    source = await readSharedSource(sourcePath, source, false, request) ?? undefined;
                    if (!source) return;
                }
                if (!source) {
                    setOpeningCallback(true);
                    const loaded = await workspace.read(sourcePath);
                    if (request !== callbackRequest.current || disposed.current)
                        return;
                    source = rememberSource(sourcePath, {
                        code: loaded.content,
                        savedCode: loaded.content,
                        file: loaded,
                    });
                }
                source = resolveSource(sourcePath, source);
                let edited = event
                    ? ensureClassMethod(
                          source.code,
                          target.className,
                          target.methodName,
                          primary ? "private" : "public",
                      )
                    : source.code;
                if (event && selected.events[event] !== handler) {
                    next = {
                        ...document,
                        root: updateNode(
                            document.root,
                            selected.id,
                            (node) => ({
                                ...node,
                                events: { ...node.events, [event]: handler },
                            }),
                        ),
                    };
                    if (!edit(next)) return;
                }
                if (primary) {
                    edited = syncAppClass(edited, next, catalog);
                    editPrimarySource(edited);
                    setActiveCallback(null);
                } else {
                    rememberSource(
                        sourcePath,
                        { ...source, code: edited },
                        true,
                    );
                    setCallbackFiles((current) => ({
                        ...current,
                        [sourcePath]: { ...source!, code: edited },
                    }));
                    setActiveCallback(sourcePath);
                }
                const info = inspectClassSource(edited, target.className);
                setCodeReveal((previous) => ({
                    lineNumber:
                        info.methods.find((m) => m.name === target.methodName)
                            ?.lineNumber ?? 1,
                    requestId: (previous?.requestId ?? 0) + 1,
                }));
                setView("代码");
                setError(null);
                return;
            }
            let handler = requestedHandler;
            if (!handler) {
                if (!event) throw new Error("请先填写应用回调函数名。");
                handler = `${selected.name}${event}`;
                const bound = {
                    ...selected,
                    events: { ...selected.events, [event]: handler },
                };
                if (
                    !edit({
                        ...document,
                        root: updateNode(
                            document.root,
                            selected.id,
                            () => bound,
                        ),
                    })
                )
                    return;
            }
            const sourcePath = callbackPath(handler, path);
            let sourceCode: string;
            if (handler === document.controller) {
                const source = sourceWorkspace && !sourceWorkspace.getSource(sourcePath)
                    ? await readSharedSource(sourcePath, primarySource, false, request)
                    : primarySource;
                if (!source) return;
                sourceCode = source.code || callbackTemplate(handler);
                if (!source.code) editPrimarySource(sourceCode);
                setActiveCallback(null);
            } else {
                let source = sourceWorkspace?.getSource(sourcePath)
                    ? sharedSource(sourceWorkspace.getSource(sourcePath)!)
                    : callbackFiles[sourcePath];
                if (sourceWorkspace && !sourceWorkspace.getSource(sourcePath) && source) {
                    source = await readSharedSource(sourcePath, source, false, request) ?? undefined;
                    if (!source) return;
                }
                if (!source) {
                    setOpeningCallback(true);
                    let loaded: WorkspaceFile | null;
                    try {
                        loaded = await workspace.read(sourcePath);
                    } catch (error) {
                        if (
                            !(error instanceof WorkspaceClientError) ||
                            error.code !== "workspace.notFound"
                        )
                            throw error;
                        loaded = null;
                    }
                    if (request !== callbackRequest.current || disposed.current)
                        return;
                    source = rememberSource(sourcePath, {
                        code: loaded?.content ?? callbackTemplate(handler),
                        savedCode: loaded?.content ?? "",
                        file: loaded,
                    });
                    if (!loaded)
                        log(`已准备 ${sourcePath} 函数模板，保存时创建文件。`);
                }
                setCallbackFiles((current) => ({
                    ...current,
                    [sourcePath]: source!,
                }));
                sourceCode = source.code;
                setActiveCallback(sourcePath);
            }
            setCodeReveal((previous) => ({
                lineNumber: callbackLine(
                    sourceCode,
                    handler,
                    event ? selected.name : undefined,
                ),
                requestId: (previous?.requestId ?? 0) + 1,
            }));
            setView("代码");
            setError(null);
        } catch (error) {
            if (request === callbackRequest.current) fail(error);
        } finally {
            if (request === callbackRequest.current) setOpeningCallback(false);
        }
    };
    const resetCallbackFiles = () => {
        callbackRequest.current++;
        setOpeningCallback(false);
        setCallbackFiles({});
        setActiveCallback(null);
        setCodeReveal(null);
    };
    const newDesign = (component: boolean) => {
        const next = component
            ? createComponentDocument("MyComponent")
            : createDocument("MyApp");
        const nextPath = component ? "MyComponent.omui" : "Untitled.omui";
        const nextCode = syncAppClass(
            classTemplate(next.appClass!, component),
            next,
            catalog,
        );
        rememberSource(controllerPath(next, nextPath), {
            code: nextCode,
            savedCode: "",
            file: null,
        });
        dispatch({ type: "reset", document: next });
        setFile(null);
        setCodeFile(null);
        setCode(nextCode);
        resetCallbackFiles();
        setSavedCode("");
        setSavedXml("");
        setXmlDraft(null);
        setPath(nextPath);
        setSelectedId(next.root.id);
    };
    const changeClassName = (name: string) => {
        try {
            if (!document.appClass || name === document.appClass) return;
            const renamed = renameClassSource(code, document.appClass, name);
            if (edit({ ...document, appClass: name })) {
                const nextPath =
                    document.kind === "component"
                        ? renamedComponentPath(document.appClass, name, path)
                        : path;
                rememberSource(callbackPath(name, nextPath), {
                    code: renamed,
                    savedCode: "",
                    file: null,
                });
                setPath(nextPath);
                setCodeFile(null);
                setSavedCode("");
                setCode(renamed);
                setActiveCallback(null);
            }
        } catch (error) {
            fail(error);
        }
    };
    const sourcePath = activeCallback ?? controllerPath(document, path);
    const activeSource = activeCallback
        ? callbackFiles[activeCallback]
        : undefined;
    const sourceCode = activeSource?.code ?? code;
    const sourceFile = activeSource ? activeSource.file : codeFile;
    const sourceDocument = sourceWorkspace?.getSource(sourcePath);
    const activateSource = async () => {
        if (!sourceWorkspace || !sourcePath || sourceWorkspace.getSource(sourcePath)) return;
        const request = ++callbackRequest.current;
        setOpeningCallback(true);
        try {
            const source = await readSharedSource(
                sourcePath,
                activeSource ?? primarySource,
                false,
                request,
            );
            if (!source) return;
            if (activeCallback) {
                setCallbackFiles((current) => ({ ...current, [sourcePath]: source }));
            } else {
                setCode(source.code);
                setSavedCode(source.savedCode);
                setCodeFile(source.file);
            }
        } catch (error) {
            if (request === callbackRequest.current) fail(error);
        } finally {
            if (request === callbackRequest.current) setOpeningCallback(false);
        }
    };
    const activateSourceRef = useRef(activateSource);
    activateSourceRef.current = activateSource;
    useEffect(() => {
        if (visible && view === "代码") void activateSourceRef.current();
    }, [visible, view, sourcePath]);
    const stop = async () => {
        const session = sessionRef.current;
        sessionRef.current = null;
        queue.current = [];
        working.current = false;
        runningDefinition.current = null;
        setRunning(false);
        setBusy(false);
        setRuntimeDocument(null);
        setFigures([]);
        if (session) await session.dispose();
        log("预览已停止，设计属性保持保存前的定义。");
    };
    const start = async () => {
        if (running || saving || working.current) return;
        if (!wsUrl) {
            fail(
                new Error(
                    "当前是离线演示模式。设计与 XML 保存可用；运行 .m 回调需要连接 openmat-server。",
                ),
            );
            return;
        }
        const next = await save();
        if (!next) return;
        const definition = { document: next, catalog };
        runningDefinition.current = definition;
        setRunning(true);
        setBusy(true);
        setView("设计");
        setRuntimeDocument(structuredClone(next));
        setFigures([]);
        working.current = true;
        const session = UiRuntimeSession.websocket(wsUrl, {
            onPayload: (payload) => {
                if (
                    sessionRef.current === session &&
                    payload.kind === "snapshot"
                )
                    setRuntimeDocument(applySnapshot(next, payload.component));
            },
            onDisplay: (display) => {
                if (sessionRef.current === session)
                    setFigures((current) => [...current, display].slice(-64));
            },
            onLog: log,
            onLost: (detail) => {
                if (sessionRef.current === session) {
                    fail(detail);
                    void stop();
                }
            },
        });
        sessionRef.current = session;
        try {
            await session.connect();
            if (sessionRef.current !== session) return;
            await session.execute(
                buildUiProgram(next, catalog),
                `${rootPath}/${path}`,
            );
            log("应用已启动 · 独立内核会话");
        } catch (e) {
            if (sessionRef.current === session) {
                fail(e);
                await stop();
            }
        } finally {
            if (sessionRef.current === session) {
                setBusy(false);
                working.current = false;
                void drain();
            }
        }
    };
    const drain = async () => {
        if (working.current) return;
        const session = sessionRef.current;
        const definition = runningDefinition.current;
        if (!session || !definition) return;
        working.current = true;
        setBusy(true);
        try {
            while (queue.current.length && sessionRef.current === session) {
                const event = queue.current.shift()!;
                await session.execute(
                    buildEventProgram(
                        definition.document,
                        event,
                        definition.catalog,
                        runtimeDocumentRef.current?.root,
                    ),
                    `${rootPath}/${path}`,
                );
            }
        } catch (e) {
            if (sessionRef.current === session) {
                fail(e);
                queue.current = [];
            }
        } finally {
            if (sessionRef.current === session) {
                working.current = false;
                setBusy(false);
            }
        }
    };
    const runtimeEvent = (event: UiInteraction) => {
        if (!sessionRef.current) return;
        if (event.property && event.value !== undefined)
            setRuntimeDocument((current) =>
                current
                    ? {
                          ...current,
                          root: updateNode(current.root, event.id, (node) => ({
                              ...node,
                              properties: {
                                  ...node.properties,
                                  [event.property!]: event.value!,
                              },
                          })),
                      }
                    : current,
            );
        if (event.event === "ValueChanged") {
            const pending = queue.current.findIndex(
                (e) => e.id === event.id && e.event === event.event,
            );
            if (pending >= 0) queue.current.splice(pending, 1);
        }
        if (queue.current.length >= 64) {
            fail("回调队列已满，请等待或中断执行。");
            return;
        }
        queue.current.push(event);
        void drain();
    };
    const register = async (requestedName = className) => {
        const name = requestedName.trim();
        if (!QUALIFIED_NAME.test(name)) {
            fail("请输入类名，例如 GainControl 或 myapp.GainControl。");
            return;
        }
        if (!wsUrl) {
            fail("读取 .m 类需要连接原生内核。");
            return;
        }
        setRegistering(true);
        let payload: UiPayload | null = null;
        const session = UiRuntimeSession.websocket(wsUrl, {
            onPayload: (p) => {
                payload = p;
            },
            onDisplay: () => undefined,
            onLog: log,
            onLost: fail,
        });
        metadataSessions.current.add(session);
        try {
            await session.connect();
            await session.execute(
                `component = ${name}();\nopenmat_ui('describe', component);`,
                `${rootPath}/${path}`,
            );
            if (!payload) throw new Error("该类未返回组件元数据。");
            const descriptor = componentDescriptor(payload, name);
            if (!disposed.current) {
                setCustom((current) => ({ ...current, [name]: descriptor }));
                setError(null);
                log(
                    `已注册 ${name}：${descriptor.properties.length} 个公开属性。`,
                );
            }
        } catch (e) {
            if (!disposed.current) fail(e);
        } finally {
            await session.dispose();
            metadataSessions.current.delete(session);
            if (!disposed.current) setRegistering(false);
        }
    };
    const attemptedClasses = useRef(new Set<string>());
    const missingClasses = [
        ...new Set(walk(document.root).map((node) => node.type)),
        ...(document.appClass && codeFile ? [document.appClass] : []),
    ]
        .filter((type) => !catalog[type])
        .join(",");
    useEffect(() => {
        if (!wsUrl || running || registering) return;
        const name = missingClasses
            .split(",")
            .find((type) => type && !attemptedClasses.current.has(type));
        if (name) {
            attemptedClasses.current.add(name);
            void register(name);
        }
    }, [missingClasses, wsUrl, running, registering]);
    const insert = (
        type: string,
        parentId?: string,
        placement: Placement = {},
    ) => {
        try {
            let parent = parentId
                ? findNode(document.root, parentId)!
                : selected;
            if (!catalog[parent.type]?.container)
                parent = findParent(document.root, parent.id) ?? document.root;
            if (
                catalog[parent.type]?.composite &&
                parent.id !== document.root.id
            )
                throw new Error("请打开复合组件定义编辑内部布局。");
            const node = createNode(type, document.root, catalog[type]);
            assertParent(parent, node, catalog);
            if (parent.layout.mode === "grid")
                node.layout = freeCell(
                    parent,
                    node.layout,
                    placement.row,
                    placement.column,
                );
            if (parent.layout.mode === "absolute")
                node.layout = {
                    ...node.layout,
                    ...(placement.x !== undefined ? { x: placement.x } : {}),
                    ...(placement.y !== undefined ? { y: placement.y } : {}),
                };
            const children = [...parent.children];
            children.splice(placement.index ?? children.length, 0, node);
            if (
                edit({
                    ...document,
                    root: updateNode(document.root, parent.id, (n) => ({
                        ...n,
                        children,
                    })),
                })
            )
                setSelectedId(node.id);
        } catch (e) {
            fail(e);
        }
    };
    const previewDrop = (id: string, event: DragEvent) => {
        const parent = findNode(document.root, id);
        if (!parent || !boardRef.current) return;
        const placement = placementAt(
            parent,
            nodeElements(boardRef.current).get(id),
            event.clientX,
            event.clientY,
            canvasScale,
        );
        setDropHint(
            parent.layout.mode === "grid"
                ? `${parent.name} · 第 ${placement.row} 行、第 ${placement.column} 列（占用时交换或顺延）`
                : parent.layout.mode === "absolute"
                  ? `${parent.name} · X ${placement.x}, Y ${placement.y}`
                  : `${parent.name} · 插入位置 ${(placement.index ?? 0) + 1}`,
        );
    };
    const drop = (id: string, event: DragEvent) => {
        setDropHint("");
        try {
            const parent = findNode(document.root, id);
            if (!parent) return;
            const placement = placementAt(
                parent,
                boardRef.current
                    ? nodeElements(boardRef.current).get(id)
                    : undefined,
                event.clientX,
                event.clientY,
                canvasScale,
            );
            const type = event.dataTransfer.getData(
                "application/x-openmat-component",
            );
            const moving = event.dataTransfer.getData(
                "application/x-openmat-node",
            );
            if (type) insert(type, id, placement);
            else if (moving) {
                const ids = selectedIds.includes(moving)
                    ? selectedIds
                    : [moving];
                if (edit(moveNodes(document, ids, id, placement, catalog)))
                    selectMany(ids);
            }
        } catch (e) {
            fail(e);
        }
    };
    const remove = () => {
        const parent = findParent(document.root, selected.id);
        if (selectedRoots.length && edit(removeNodes(document, selectedIds))) {
            setSelectedId(parent?.id ?? document.root.id);
            sectionRef.current?.focus({ preventScroll: true });
        }
    };
    const duplicate = () => {
        try {
            const result = duplicateNodes(document, selectedIds, catalog);
            if (edit(result.document)) selectMany(result.ids);
        } catch (e) {
            fail(e);
        }
    };
    const reorder = (direction: number) => {
        try {
            edit(reorderNodes(document, selectedIds, direction));
        } catch (e) {
            fail(e);
        }
    };
    const resizePanel = (side: "left" | "right", event: ReactPointerEvent) => {
        event.preventDefault();
        dragCleanup.current?.();
        const startX = event.clientX;
        const startWidth = widths[side];
        const move = (e: PointerEvent) =>
            setWidths((current) => ({
                ...current,
                [side]: Math.max(
                    side === "left" ? 180 : 240,
                    Math.min(
                        side === "left" ? 360 : 440,
                        startWidth +
                            (e.clientX - startX) * (side === "left" ? 1 : -1),
                    ),
                ),
            }));
        const cleanup = () => {
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", cleanup);
            window.removeEventListener("pointercancel", cleanup);
        };
        dragCleanup.current = cleanup;
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", cleanup);
        window.addEventListener("pointercancel", cleanup);
    };
    const designRoot = useComponentPreviews(
        document,
        catalog,
        wsUrl,
        `${rootPath}/${path}`,
        !running && !registering,
        fail,
    );
    const baseRoot = runtimeDocument?.root ?? designRoot;
    const baseWidth = document.kind
        ? baseRoot.layout.width
        : Number(propertyValue(baseRoot, "Width", catalog));
    const baseHeight = document.kind
        ? baseRoot.layout.height
        : Number(propertyValue(baseRoot, "Height", catalog));
    const baseScale =
        zoom === 0
            ? Math.max(
                  0.25,
                  Math.min(
                      1,
                      (canvasSpace.width - 70) / baseWidth,
                      (canvasSpace.height - 95) / baseHeight,
                  ),
              )
            : zoom / 100;
    const canvas = useCanvasEditing({
        document,
        ids: selectedIds,
        boardRef,
        scale: baseScale,
        running: running || saving || xmlDraft !== null,
        select: selectMany,
        commit: edit,
        onError: fail,
    });
    const activeRoot = overlayPreview(baseRoot, canvas.visual?.preview);
    const canvasWidth = document.kind
        ? activeRoot.layout.width
        : Number(propertyValue(activeRoot, "Width", catalog));
    const canvasHeight = document.kind
        ? activeRoot.layout.height
        : Number(propertyValue(activeRoot, "Height", catalog));
    const canvasScale = canvas.visual?.scale ?? baseScale;
    const editable = !running && !saving && xmlDraft === null;
    const showContextMenu = (
        id: string,
        target: HTMLElement,
        x: number,
        y: number,
    ) => {
        if (!editable || canvas.dragging.current) return;
        if (!selectedIds.includes(id)) setSelectedId(id);
        else setPrimaryId(id);
        setContextMenu({ x, y, target, document });
    };
    const contextMenuEvent = (
        id: string,
        event: ReactMouseEvent<HTMLElement>,
    ) => {
        if (!editable) return;
        event.preventDefault();
        event.stopPropagation();
        const rect = event.currentTarget.getBoundingClientRect();
        showContextMenu(
            id,
            event.currentTarget,
            event.clientX || rect.left + 12,
            event.clientY || rect.top + 12,
        );
    };
    const designNodeId = (id: string) => {
        let node = findNode(activeRoot, id);
        while (node && !findNode(document.root, node.id))
            node = findParent(activeRoot, node.id);
        return node?.id;
    };
    const rename = () => {
        setInspectorTab("属性");
        setRenameRequest((request) => request + 1);
    };
    const arrange = (operation: Arrangement) => {
        try {
            edit(
                arrangeNodes(
                    document,
                    selectedIds,
                    operation,
                    measureNodes(boardRef.current, document, canvasScale),
                ),
            );
        } catch (e) {
            fail(e);
        }
    };
    const wrap = (type: string) => {
        try {
            const result = wrapNodes(
                document,
                selectedIds,
                type,
                catalog,
                measureNodes(boardRef.current, document, canvasScale),
            );
            if (edit(result.document)) {
                setSelectedId(result.id);
                sectionRef.current?.focus({ preventScroll: true });
            }
        } catch (e) {
            fail(e);
        }
    };
    const menuParent = findParent(
        document.root,
        selectedRoots[0]?.id ?? selected.id,
    );
    const clickedParent = findParent(document.root, selected.id);
    const siblingsSelected =
        !!menuParent &&
        selectedRoots.length > 0 &&
        selectedRoots.every(
            (node) => findParent(document.root, node.id)?.id === menuParent.id,
        );
    const canWrap =
        siblingsSelected &&
        menuParent.type !== "TabGroup" &&
        !selectedRoots.some((node) => node.type === "Tab") &&
        (menuParent.id === document.root.id ||
            !catalog[menuParent.type]?.composite);
    const canReorder = (direction: number) =>
        siblingsSelected &&
        menuParent.children.some(
            (node, index) =>
                selectedIds.includes(node.id) &&
                menuParent.children[index + direction] &&
                !selectedIds.includes(
                    menuParent.children[index + direction]!.id,
                ),
        );
    const menuGroups: MenuGroup[] = [
        {
            actions: [
                {
                    label: "复制组件",
                    shortcut: "Ctrl+D",
                    icon: <MenuIcon name="copy" />,
                    disabled: !siblingsSelected,
                    run: duplicate,
                },
                {
                    label: "重命名",
                    shortcut: "F2",
                    icon: <MenuIcon name="rename" />,
                    disabled: selectedNodes.length !== 1,
                    run: rename,
                },
                {
                    label: "选择父容器",
                    icon: <MenuIcon name="parent" />,
                    disabled: !clickedParent,
                    run: () => {
                        if (clickedParent) setSelectedId(clickedParent.id);
                    },
                },
                {
                    label: "上移",
                    icon: <MenuIcon name="up" />,
                    disabled: !canReorder(-1),
                    run: () => reorder(-1),
                },
                {
                    label: "下移",
                    icon: <MenuIcon name="down" />,
                    disabled: !canReorder(1),
                    run: () => reorder(1),
                },
            ],
        },
        {
            label: "包入容器",
            actions: [
                "RowLayout",
                "ColumnLayout",
                "GridLayout",
                "ScrollPanel",
            ].map((type) => ({
                label: `包入${catalog[type]!.label}`,
                icon: <ComponentIcon type={type} size={16} />,
                disabled: !canWrap,
                run: () => wrap(type),
            })),
        },
        ...(selectedRoots.length > 1
            ? [
                  {
                      label: "排列与尺寸",
                      actions: (
                          [
                              ["left", "左对齐"],
                              ["centerX", "水平居中"],
                              ["right", "右对齐"],
                              ["top", "顶对齐"],
                              ["centerY", "垂直居中"],
                              ["bottom", "底对齐"],
                              ["distributeX", "水平等间距"],
                              ["distributeY", "垂直等间距"],
                              ["sameWidth", "统一宽度"],
                              ["sameHeight", "统一高度"],
                          ] as const
                      ).map(([operation, label]) => ({
                          label,
                          run: () => arrange(operation),
                          disabled:
                              !siblingsSelected ||
                              (!operation.startsWith("same") &&
                                  menuParent?.layout.mode !== "absolute") ||
                              (operation.startsWith("distribute") &&
                                  selectedRoots.length < 3),
                      })),
                  },
              ]
            : []),
        {
            actions: [
                {
                    label: "删除组件",
                    shortcut: "Delete",
                    icon: <MenuIcon name="remove" />,
                    disabled: !selectedRoots.length,
                    danger: true,
                    run: remove,
                },
            ],
        },
    ];
    const allComponents = [
        ...BUILTIN_COMPONENTS,
        ...Object.values(custom).filter(
            (c) => c.className !== document.appClass,
        ),
    ].filter((c) =>
        `${c.label} ${c.type}`.toLowerCase().includes(filter.toLowerCase()),
    );
    const tree = (node: UiNode, depth = 0) => (
        <div
            role="treeitem"
            aria-selected={selectedIds.includes(node.id)}
            aria-expanded={node.children.length ? true : undefined}
            key={node.id}
        >
            <button
                type="button"
                className={`designer-tree-node ${selectedIds.includes(node.id) ? "selected" : ""}`}
                style={{ paddingLeft: depth * 14 + 10 }}
                onClick={(e) =>
                    selectNode(node.id, e.shiftKey || e.ctrlKey || e.metaKey)
                }
                onContextMenu={(e) => contextMenuEvent(node.id, e)}
                onKeyDown={(e) => {
                    if (
                        e.key === "ContextMenu" ||
                        (e.shiftKey && e.key === "F10")
                    ) {
                        e.preventDefault();
                        e.stopPropagation();
                        const rect = e.currentTarget.getBoundingClientRect();
                        showContextMenu(
                            node.id,
                            e.currentTarget,
                            rect.left + 12,
                            rect.top + 12,
                        );
                    }
                }}
                draggable={node.type !== "Window" && !running}
                onDragStart={(e) => {
                    e.dataTransfer.setData(
                        "application/x-openmat-node",
                        node.id,
                    );
                    e.dataTransfer.effectAllowed = "move";
                }}
                onDragOver={(e) => {
                    if (catalog[node.type]?.container && !running)
                        e.preventDefault();
                }}
                onDrop={(e) => {
                    if (!running) {
                        e.preventDefault();
                        drop(node.id, e);
                    }
                }}
            >
                <ComponentIcon type={catalog[node.type]?.icon ?? node.type} />
                <span>{node.name}</span>
                <small>{node.type}</small>
            </button>
            {node.children.length ? (
                <div role="group">
                    {node.children.map((child) => tree(child, depth + 1))}
                </div>
            ) : null}
        </div>
    );
    return (
        <section
            ref={sectionRef}
            className="app-designer"
            aria-label="App Designer"
            onDragEnd={() => setDropHint("")}
            onKeyDown={(e) => {
                if (e.defaultPrevented || canvas.dragging.current) return;
                const editingText =
                    e.target instanceof Element &&
                    !!e.target.closest(
                        "input,textarea,select,[contenteditable=true],.monaco-editor,[role=separator]",
                    );
                if (!editingText && !running && view === "设计") {
                    if (
                        e.key === "F2" &&
                        editable &&
                        selectedNodes.length === 1
                    ) {
                        e.preventDefault();
                        rename();
                        return;
                    }
                    if (e.key === "Delete" || e.key === "Backspace") {
                        e.preventDefault();
                        remove();
                        return;
                    }
                    if (
                        (e.ctrlKey || e.metaKey) &&
                        e.key.toLowerCase() === "d"
                    ) {
                        e.preventDefault();
                        duplicate();
                        return;
                    }
                    if (e.key.startsWith("Arrow") && selectedRoots.length) {
                        e.preventDefault();
                        const step = e.shiftKey ? 8 : 1;
                        try {
                            edit(
                                nudgeNodes(
                                    document,
                                    selectedIds,
                                    e.key === "ArrowRight"
                                        ? step
                                        : e.key === "ArrowLeft"
                                          ? -step
                                          : 0,
                                    e.key === "ArrowDown"
                                        ? step
                                        : e.key === "ArrowUp"
                                          ? -step
                                          : 0,
                                ),
                            );
                        } catch (error) {
                            fail(error);
                        }
                        return;
                    }
                }
                if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
                    e.preventDefault();
                    if (!running) void save();
                }
                if (
                    (e.ctrlKey || e.metaKey) &&
                    e.key.toLowerCase() === "z" &&
                    !editingText &&
                    !running
                ) {
                    e.preventDefault();
                    dispatch({ type: e.shiftKey ? "redo" : "undo" });
                }
                if (
                    e.key === "Delete" &&
                    e.target === e.currentTarget &&
                    !running
                )
                    remove();
            }}
            tabIndex={-1}
        >
            <header className="designer-toolbar">
                <div className="designer-app-title">
                    <ComponentIcon type="Window" size={23} />
                    <strong>App Designer</strong>
                    <span>{dirty ? "未保存" : "已保存"}</span>
                </div>
                <div className="designer-toolbar-actions">
                    <button
                        type="button"
                        disabled={running || saving}
                        onClick={() => {
                            if (dirty) setPendingOpen(":new");
                            else newDesign(false);
                        }}
                    >
                        新建
                    </button>
                    <button
                        type="button"
                        disabled={running || saving}
                        onClick={() =>
                            dirty
                                ? setPendingOpen(":component")
                                : newDesign(true)
                        }
                    >
                        新建组件
                    </button>
                    <button
                        type="button"
                        disabled={running || saving || openingCallback}
                        onClick={() => void save()}
                    >
                        {saving ? "保存中…" : "保存"}
                    </button>
                    <button
                        type="button"
                        disabled={running || !history.past.length}
                        onClick={() => dispatch({ type: "undo" })}
                        title="撤销 Ctrl+Z"
                    >
                        ↶
                    </button>
                    <button
                        type="button"
                        disabled={running || !history.future.length}
                        onClick={() => dispatch({ type: "redo" })}
                        title="重做 Ctrl+Shift+Z"
                    >
                        ↷
                    </button>
                    <span className="designer-divider" />
                    {running ? (
                        <>
                            <button
                                type="button"
                                disabled={!busy}
                                onClick={() =>
                                    void sessionRef.current
                                        ?.interrupt()
                                        .catch(fail)
                                }
                            >
                                中断
                            </button>
                            <button type="button" onClick={() => void stop()}>
                                ■ 停止
                            </button>
                        </>
                    ) : (
                        <button
                            type="button"
                            className="designer-run"
                            disabled={saving || registering || openingCallback}
                            onClick={() => void start()}
                        >
                            ▶ 保存并运行
                        </button>
                    )}
                    <button
                        type="button"
                        onClick={() => {
                            if (running) void stop();
                            onClose();
                        }}
                    >
                        返回工作台
                    </button>
                </div>
            </header>
            <div className="designer-filebar">
                <label>
                    界面文件
                    <input
                        aria-label="界面文件路径"
                        disabled={running || saving}
                        value={path}
                        onChange={(e) => setPath(e.target.value)}
                    />
                </label>
                <button
                    type="button"
                    disabled={running || saving}
                    onClick={() =>
                        dirty ? setPendingOpen(path) : void load(path)
                    }
                >
                    打开
                </button>
                <label>
                    {document.kind
                        ? "组件类"
                        : document.version !== 1
                          ? "应用类"
                          : "应用回调"}
                    <input
                        key={document.appClass ?? document.controller}
                        aria-label={
                            document.kind
                                ? "组件类名称"
                                : document.version !== 1
                                  ? "应用类名称"
                                  : "应用回调函数"
                        }
                        placeholder="例如 SignalApp"
                        defaultValue={document.appClass ?? document.controller}
                        disabled={running}
                        onBlur={(e) =>
                            document.version !== 1
                                ? changeClassName(e.target.value.trim())
                                : edit({
                                      ...document,
                                      ...(document.version !== 1
                                          ? { appClass: e.target.value.trim() }
                                          : {
                                                controller:
                                                    e.target.value.trim(),
                                            }),
                                  })
                        }
                    />
                </label>
                <button
                    type="button"
                    aria-label={
                        document.version !== 1 ? "编辑应用类" : "编辑应用回调"
                    }
                    disabled={
                        !(document.appClass ?? document.controller) ||
                        running ||
                        saving
                    }
                    onClick={() =>
                        void openCallback(
                            document.appClass ?? document.controller,
                        )
                    }
                >
                    编辑代码
                </button>
                <span className="designer-runtime-state">
                    {running
                        ? busy
                            ? "执行回调中…"
                            : "应用运行中"
                        : "设计模式"}
                </span>
                {selected.id !== document.root.id &&
                catalog[selected.type]?.composite ? (
                    <button
                        type="button"
                        disabled={running || saving}
                        onClick={() => {
                            const definitionPath = callbackPath(
                                catalog[selected.type]!.className!,
                                path,
                            ).replace(/\.m$/, ".omui");
                            if (dirty) setPendingOpen(definitionPath);
                            else void load(definitionPath);
                        }}
                    >
                        编辑组件定义
                    </button>
                ) : null}
            </div>
            {error ? (
                <div className="designer-error" role="alert">
                    <span>{error}</span>
                    <button
                        type="button"
                        aria-label="关闭错误提示"
                        onClick={() => setError(null)}
                    >
                        ×
                    </button>
                </div>
            ) : null}
            <div
                className="designer-workspace"
                style={{
                    gridTemplateColumns: `${widths.left}px 5px minmax(180px,1fr) 5px ${widths.right}px`,
                }}
            >
                <LibraryPanels
                    palette={
                        <div className="designer-palette">
                            <div className="designer-pane-title">
                                组件库 <small>{allComponents.length}</small>
                            </div>
                            <input
                                className="designer-search"
                                aria-label="搜索组件"
                                placeholder="搜索组件…"
                                value={filter}
                                onChange={(e) => setFilter(e.target.value)}
                            />
                            <div className="designer-palette-scroll">
                                {[
                                    ...new Set(
                                        allComponents.map((c) => c.group),
                                    ),
                                ].map((group) => (
                                    <details key={group} open>
                                        <summary>{group}</summary>
                                        <div className="designer-component-grid">
                                            {allComponents
                                                .filter(
                                                    (c) => c.group === group,
                                                )
                                                .map((c) => (
                                                    <button
                                                        type="button"
                                                        key={c.type}
                                                        title={`${c.type}${c.type === "Window" ? " · 根窗口已存在" : " · 拖入画布或单击添加"}`}
                                                        disabled={
                                                            running ||
                                                            c.type === "Window"
                                                        }
                                                        draggable={
                                                            !running &&
                                                            c.type !== "Window"
                                                        }
                                                        onDragStart={(e) => {
                                                            e.dataTransfer.setData(
                                                                "application/x-openmat-component",
                                                                c.type,
                                                            );
                                                            e.dataTransfer.effectAllowed =
                                                                "copy";
                                                        }}
                                                        onClick={() =>
                                                            insert(c.type)
                                                        }
                                                    >
                                                        <ComponentIcon
                                                            type={c.icon}
                                                        />
                                                        <span>{c.label}</span>
                                                    </button>
                                                ))}
                                        </div>
                                    </details>
                                ))}
                            </div>
                            <div className="designer-register">
                                <input
                                    aria-label="自定义组件类名"
                                    placeholder="MyComponent 类名"
                                    value={className}
                                    onChange={(e) =>
                                        setClassName(e.target.value)
                                    }
                                    disabled={registering || running}
                                />
                                <button
                                    type="button"
                                    disabled={registering || running}
                                    onClick={() => void register()}
                                >
                                    {registering ? "读取…" : "注册"}
                                </button>
                            </div>
                        </div>
                    }
                    tree={
                        <div className="designer-tree">
                            <div className="designer-pane-title">
                                对象树{" "}
                                <small>{walk(document.root).length}</small>
                                <span />
                                <button
                                    type="button"
                                    title="上移"
                                    disabled={
                                        running ||
                                        selected.id === document.root.id
                                    }
                                    onClick={() => reorder(-1)}
                                >
                                    ↑
                                </button>
                                <button
                                    type="button"
                                    title="下移"
                                    disabled={
                                        running ||
                                        selected.id === document.root.id
                                    }
                                    onClick={() => reorder(1)}
                                >
                                    ↓
                                </button>
                            </div>
                            <div
                                role="tree"
                                aria-multiselectable="true"
                                aria-label="对象树"
                                className="designer-tree-scroll"
                            >
                                {tree(document.root)}
                            </div>
                            <div className="designer-tree-actions">
                                <button
                                    type="button"
                                    disabled={
                                        running ||
                                        selected.id === document.root.id
                                    }
                                    onClick={duplicate}
                                >
                                    复制组件
                                </button>
                                <button
                                    type="button"
                                    disabled={
                                        running ||
                                        selected.id === document.root.id
                                    }
                                    onClick={remove}
                                >
                                    删除
                                </button>
                            </div>
                        </div>
                    }
                />
                <div
                    className="designer-separator"
                    role="separator"
                    aria-label="组件库宽度"
                    aria-orientation="vertical"
                    aria-valuenow={widths.left}
                    aria-valuemin={180}
                    aria-valuemax={360}
                    tabIndex={0}
                    onPointerDown={(e) => resizePanel("left", e)}
                    onKeyDown={(e) => {
                        if (["ArrowLeft", "ArrowRight"].includes(e.key))
                            setWidths((w) => ({
                                ...w,
                                left: Math.max(
                                    180,
                                    Math.min(
                                        360,
                                        w.left +
                                            (e.key === "ArrowRight" ? 10 : -10),
                                    ),
                                ),
                            }));
                    }}
                />
                <main className="designer-center">
                    <div className="designer-viewbar">
                        <nav className="designer-tabs" aria-label="设计器视图">
                            {["设计", "XML", "代码"].map((label) => (
                                <button
                                    type="button"
                                    key={label}
                                    aria-pressed={view === label}
                                    disabled={running && label !== "设计"}
                                    onClick={() => {
                                        if (
                                            label === "代码" &&
                                            document.version !== 1
                                        )
                                            void openCallback(
                                                document.appClass!,
                                            );
                                        else setView(label);
                                    }}
                                >
                                    {label}
                                </button>
                            ))}
                        </nav>
                        <span />
                        <label className="designer-zoom">
                            缩放
                            <select
                                aria-label="画布缩放"
                                value={zoom}
                                onChange={(e) =>
                                    setZoom(Number(e.target.value))
                                }
                            >
                                <option value={0}>适合画布</option>
                                {[50, 67, 75, 90, 100, 125, 150].map(
                                    (value) => (
                                        <option key={value} value={value}>
                                            {value}%
                                        </option>
                                    ),
                                )}
                            </select>
                        </label>
                        <small>
                            {canvasWidth} × {canvasHeight}
                        </small>
                    </div>
                    {view === "设计" && (
                        <EditorTools
                            count={selectedRoots.length}
                            disabled={running || saving || xmlDraft !== null}
                            absolute={
                                selectedRoots.length > 0 &&
                                selectedRoots.every(
                                    (n) =>
                                        findParent(document.root, n.id)?.layout
                                            .mode === "absolute",
                                )
                            }
                            arrange={arrange}
                            wrap={wrap}
                            width={canvasWidth}
                            height={canvasHeight}
                            resize={(width, height) =>
                                edit(resizeWindow(document, width, height))
                            }
                        />
                    )}
                    {dropHint && (
                        <div className="designer-drop-hint" role="status">
                            {dropHint}
                        </div>
                    )}
                    {view === "设计" ? (
                        <div
                            ref={canvasRef}
                            className={`designer-canvas ${running ? "is-running" : ""}`}
                        >
                            <div className="designer-canvas-label">
                                {String(
                                    propertyValue(activeRoot, "Title", catalog),
                                )}
                                <span>{running ? "运行预览" : "Window"}</span>
                            </div>
                            <div
                                className="designer-canvas-size"
                                style={{
                                    width: canvasWidth * canvasScale,
                                    height: canvasHeight * canvasScale,
                                }}
                            >
                                <div
                                    className="designer-artboard"
                                    ref={boardRef}
                                    onPointerDownCapture={canvas.onPointerDown}
                                    onClickCapture={(e) => {
                                        if (canvas.suppressClick.current) {
                                            canvas.suppressClick.current = false;
                                            e.stopPropagation();
                                        }
                                    }}
                                    onDragEnd={() => setDropHint("")}
                                    style={{
                                        width: canvasWidth,
                                        height: canvasHeight,
                                        transform: `scale(${canvasScale})`,
                                    }}
                                >
                                    <UiRenderer
                                        node={activeRoot}
                                        selectedId={selected.id}
                                        selectedIds={selectedIds}
                                        running={running}
                                        catalog={catalog}
                                        figures={figures}
                                        graphicsSessionId={
                                            sessionRef.current?.sessionId
                                        }
                                        onSelect={(id, additive) => {
                                            const target = designNodeId(id);
                                            if (target)
                                                selectNode(target, additive);
                                        }}
                                        onContextMenu={(id, event) => {
                                            const target = designNodeId(id);
                                            if (target)
                                                contextMenuEvent(target, event);
                                        }}
                                        onDrop={drop}
                                        onDragPreview={previewDrop}
                                        onResize={(id, width, height) =>
                                            edit({
                                                ...document,
                                                root: updateNode(
                                                    document.root,
                                                    id,
                                                    (node) => ({
                                                        ...node,
                                                        layout: {
                                                            ...node.layout,
                                                            width,
                                                            height,
                                                            grow: 0,
                                                            widthMode: "fixed",
                                                            heightMode: "fixed",
                                                        },
                                                    }),
                                                ),
                                            })
                                        }
                                        onEvent={runtimeEvent}
                                    />
                                    {!running && (
                                        <button
                                            type="button"
                                            className="designer-window-resize"
                                            data-window-resize
                                            aria-label="调整预览窗口大小"
                                            onKeyDown={(e) => {
                                                if (e.key.startsWith("Arrow")) {
                                                    e.preventDefault();
                                                    e.stopPropagation();
                                                    edit(
                                                        resizeWindow(
                                                            document,
                                                            canvasWidth +
                                                                (e.key ===
                                                                "ArrowRight"
                                                                    ? 8
                                                                    : e.key ===
                                                                        "ArrowLeft"
                                                                      ? -8
                                                                      : 0),
                                                            canvasHeight +
                                                                (e.key ===
                                                                "ArrowDown"
                                                                    ? 8
                                                                    : e.key ===
                                                                        "ArrowUp"
                                                                      ? -8
                                                                      : 0),
                                                        ),
                                                    );
                                                }
                                            }}
                                        />
                                    )}
                                    {canvas.visual?.marquee && (
                                        <div
                                            className="designer-marquee"
                                            style={{
                                                left: canvas.visual.marquee.x,
                                                top: canvas.visual.marquee.y,
                                                width: canvas.visual.marquee
                                                    .width,
                                                height: canvas.visual.marquee
                                                    .height,
                                            }}
                                        />
                                    )}
                                    {canvas.visual?.guides.map((guide, i) => (
                                        <div
                                            key={i}
                                            className={`designer-guide designer-guide-${guide.axis}`}
                                            style={
                                                guide.axis === "x"
                                                    ? { left: guide.position }
                                                    : { top: guide.position }
                                            }
                                        />
                                    ))}
                                </div>
                            </div>
                        </div>
                    ) : view === "XML" ? (
                        <div className="designer-source">
                            <div className="designer-source-info">
                                OpenMat UI · XML v{document.version}{" "}
                                <button
                                    type="button"
                                    onClick={() => {
                                        try {
                                            edit(
                                                parseUi(xmlDraft ?? currentXml),
                                                true,
                                            );
                                        } catch (e) {
                                            fail(e);
                                        }
                                    }}
                                >
                                    应用 XML
                                </button>
                                <button
                                    type="button"
                                    disabled={xmlDraft === null}
                                    onClick={() => setXmlDraft(null)}
                                >
                                    还原草稿
                                </button>
                            </div>
                            <textarea
                                aria-label="界面 XML"
                                spellCheck={false}
                                value={xmlDraft ?? currentXml}
                                onChange={(e) => setXmlDraft(e.target.value)}
                            />
                        </div>
                    ) : (
                        <div className="designer-source">
                            <div className="designer-source-info">
                                <span className="designer-source-path">
                                    {sourcePath || "先填写应用回调函数名"}
                                </span>
                                <span>
                                    {sourceFile
                                        ? "已关联 .m 文件"
                                        : "未保存 · 保存时创建 .m 文件"}
                                </span>
                                <select
                                    aria-label="回调文件"
                                    value={activeCallback ?? ""}
                                    onChange={(event) => {
                                        setActiveCallback(
                                            event.target.value || null,
                                        );
                                        setCodeReveal(null);
                                    }}
                                >
                                    <option value="">
                                        {document.appClass ||
                                        document.controller
                                            ? `${document.version !== 1 ? "应用类" : "应用回调"} · ${controllerPath(document, path)}`
                                            : "应用回调"}
                                    </option>
                                    {Object.keys(callbackFiles).map(
                                        (filePath) => (
                                            <option
                                                key={filePath}
                                                value={filePath}
                                            >
                                                {filePath}
                                                {callbackFiles[filePath]!
                                                    .code !==
                                                callbackFiles[filePath]!
                                                    .savedCode
                                                    ? " *"
                                                    : ""}
                                            </option>
                                        ),
                                    )}
                                </select>
                            </div>
                            <Suspense
                                fallback={
                                    <div className="designer-hint">
                                        加载代码编辑器…
                                    </div>
                                }
                            >
                                {sourceWorkspace && !sourceDocument ? (
                                    <div className="designer-hint">
                                        <span>{openingCallback ? "正在读取源文件…" : "源文件尚未打开。"}</span>
                                        <button
                                            type="button"
                                            disabled={openingCallback}
                                            onClick={() => void activateSource()}
                                        >
                                            打开源文件
                                        </button>
                                    </div>
                                ) : <CodeEditor
                                    value={sourceCode}
                                    theme={theme}
                                    documentId={
                                        sourceDocument?.id ??
                                        documentId(rootPath, sourcePath)
                                    }
                                    documentPath={sourcePath}
                                    documentUri={
                                        sourceDocument?.uri ??
                                        workspaceDocumentUri(
                                            sourcePath,
                                            rootGeneration,
                                            rootPath,
                                        )
                                    }
                                    documentVersion={sourceDocument?.version ?? 1}
                                    {...(sourceWorkspace
                                        ? {
                                              workspaceDocuments: sourceWorkspace.documents,
                                              editorSession: sourceWorkspace.editorSession,
                                              onWorkspaceDocumentChange: sourceWorkspace.updateSource,
                                              onOpenDocument: sourceWorkspace.openDocument,
                                          }
                                        : {})}
                                    viewState={null}
                                    reveal={codeReveal}
                                    onChange={(value) => {
                                        if (activeCallback) {
                                            rememberSource(
                                                activeCallback,
                                                {
                                                    ...callbackFiles[activeCallback]!,
                                                    code: value,
                                                },
                                                true,
                                            );
                                            setCallbackFiles((current) => ({
                                                ...current,
                                                [activeCallback]: {
                                                    ...current[activeCallback]!,
                                                    code: value,
                                                },
                                            }));
                                        } else editPrimarySource(value);
                                    }}
                                    onViewStateChange={ignoreViewState}
                                    onRun={() => void start()}
                                    onSave={() => void save()}
                                    lspUrl={sourceWorkspace?.lspUrl ?? null}
                                />}
                            </Suspense>
                        </div>
                    )}
                    <div className="designer-output">
                        <button
                            type="button"
                            className="designer-output-toggle"
                            aria-expanded={outputOpen}
                            onClick={() => setOutputOpen((o) => !o)}
                        >
                            {outputOpen ? "▾" : "▸"} 输出与诊断{" "}
                            <span>{logs.length}</span>
                        </button>
                        {outputOpen ? (
                            <div className="designer-log" role="log">
                                {logs.map((line, i) => (
                                    <pre key={i}>{line}</pre>
                                ))}
                            </div>
                        ) : null}
                    </div>
                </main>
                <div
                    className="designer-separator"
                    role="separator"
                    aria-label="检查器宽度"
                    aria-orientation="vertical"
                    aria-valuenow={widths.right}
                    aria-valuemin={240}
                    aria-valuemax={440}
                    tabIndex={0}
                    onPointerDown={(e) => resizePanel("right", e)}
                    onKeyDown={(e) => {
                        if (["ArrowLeft", "ArrowRight"].includes(e.key))
                            setWidths((w) => ({
                                ...w,
                                right: Math.max(
                                    240,
                                    Math.min(
                                        440,
                                        w.right +
                                            (e.key === "ArrowLeft" ? 10 : -10),
                                    ),
                                ),
                            }));
                    }}
                />
                <div
                    ref={inspectorRef}
                    className={
                        running
                            ? "designer-inspector-disabled"
                            : "designer-inspector-wrap"
                    }
                >
                    {selectedNodes.length > 1 ? (
                        <MultiInspector
                            nodes={selectedNodes}
                            catalog={catalog}
                            tab={inspectorTab}
                            onTab={setInspectorTab}
                            {...(parentMode ? { parentMode } : {})}
                            onProperty={(name, value) =>
                                edit(
                                    patchProperties(
                                        document,
                                        selectedIds,
                                        name,
                                        value,
                                    ),
                                )
                            }
                            onLayout={(patch) => {
                                try {
                                    edit(
                                        patchLayouts(
                                            document,
                                            selectedIds,
                                            patch,
                                            measureNodes(
                                                boardRef.current,
                                                document,
                                                canvasScale,
                                            ),
                                        ),
                                    );
                                } catch (error) {
                                    fail(error);
                                }
                            }}
                        />
                    ) : (
                        <Inspector
                            node={selected}
                            {...(parentMode ? { parentMode } : {})}
                            catalog={catalog}
                            tab={inspectorTab}
                            onTab={setInspectorTab}
                            controller={document.controller}
                            {...(document.appClass
                                ? { appClass: document.appClass }
                                : {})}
                            methods={methodOptions(
                                code,
                                document,
                                selected,
                                catalog,
                            )}
                            {...(document.appClass &&
                            selected.id === document.root.id &&
                            catalog[document.appClass]
                                ? {
                                      descriptor: {
                                          ...catalog[document.appClass]!,
                                          ...(document.kind
                                              ? { composite: false }
                                              : {}),
                                          properties: catalog[
                                              document.appClass
                                          ]!.properties.filter(
                                              (p) =>
                                                  !walk(document.root).some(
                                                      (n) => n.name === p.name,
                                                  ),
                                          ),
                                      },
                                  }
                                : {})}
                            uiPath={path}
                            onOpenCallback={(handler, event) =>
                                void openCallback(handler, event)
                            }
                            onChange={(node) => {
                                if (!running) changeNode(node);
                            }}
                        />
                    )}
                    {running ? (
                        <div className="designer-inspector-note">
                            运行时属性在预览中更新。停止后可编辑设计定义。
                        </div>
                    ) : null}
                </div>
            </div>
            <footer className="designer-status">
                <span>{running ? "独立运行会话" : "布局吸附 · 8 px"}</span>
                <span>{selected.name}</span>
                <span>XML v{document.version} · UTF-8</span>
                <span>Ctrl+S 保存 · Ctrl+Z 撤销</span>
            </footer>
            {contextMenu &&
                contextMenu.document === document &&
                editable &&
                pendingOpen === null && (
                    <ContextMenu
                        x={contextMenu.x}
                        y={contextMenu.y}
                        title={
                            selectedNodes.length > 1
                                ? `已选择 ${selectedNodes.length} 个组件`
                                : selected.name
                        }
                        groups={menuGroups}
                        onClose={closeContextMenu}
                    />
                )}
            {pendingOpen !== null ? (
                <div className="designer-modal-shade">
                    <div
                        className="designer-modal"
                        role="dialog"
                        aria-modal="true"
                        aria-label="未保存的设计"
                    >
                        <h3>替换当前设计？</h3>
                        <p>
                            当前修改尚未保存。继续将打开{" "}
                            {pendingOpen === ":new" ? "空白界面" : pendingOpen}
                            。
                        </p>
                        <button
                            type="button"
                            autoFocus
                            onClick={() => setPendingOpen(null)}
                        >
                            取消
                        </button>
                        <button
                            type="button"
                            onClick={() => {
                                const target = pendingOpen;
                                setPendingOpen(null);
                                if (target === ":component") newDesign(true);
                                else if (target === ":new") newDesign(false);
                                else void load(target);
                            }}
                        >
                            放弃修改并继续
                        </button>
                    </div>
                </div>
            ) : null}
        </section>
    );
}
