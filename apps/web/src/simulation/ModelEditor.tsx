import { CommitField } from "./CommitField";
import { SamplingInspector } from "./SamplingInspector";
import { sampleTimeText } from "./sampling";
import {
    Suspense,
    lazy,
    useCallback,
    useEffect,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
    type CSSProperties,
    type RefObject,
} from "react";
import {
    MarkerType,
    ReactFlowProvider,
    applyNodeChanges,
    useReactFlow,
    type Connection as FlowConnection,
    type NodeChange,
    type EdgeChange,
    type Viewport,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import "../designer/designer.css";
import "./simulation.css";
import { ContextMenu, type MenuGroup } from "../designer/ContextMenu";
import { LibraryPanels } from "../designer/LibraryPanels";
import type { ModelEditorSession } from "./session";
import {
    usePlatformServices,
    nativeFileKey,
} from "../platform/platform-services";
import type { PendingOperations } from "../platform/pending-operations";
import type {
    WorkspaceClient,
    WorkspaceFile,
} from "../workspace/workspace-client";
import type { IdeTheme } from "../theme";
import { BlockIcon, type FlowBlock, type SignalEdge } from "./BlockNode";
import { SimulationCanvas } from "./SimulationCanvas";
import { ScopePanel } from "./ScopePanel";
import type { SlxImport } from "./client";
import { decodeSlx, encodeSlx, type SlxAsset } from "./slx-authoring";
import SlxParameters from "./SlxParameters";
import SlxTree from "./SlxTree";
import { SimulationError, type SimulationDiagnostic } from "./client";
import type { DesignerSourceWorkspace } from "../documents/designer-source-workspace";
import { FunctionInspector } from "./FunctionInspector";
import { FunctionCodePanel } from "./FunctionCodePanel";
import { SolverInspector } from "./SolverInspector";
import { ComponentInspector } from "./ComponentInspector";
import { ComponentDialog, blankComponent } from "./ComponentDialog";
import { ComponentLibrary } from "./ComponentLibrary";
import {
    attachComponent,
    blockSources,
    callbacks,
    componentFile,
    componentFor,
    modelSources,
    sameComponentDefinition,
    validateComponentKind,
    type ComponentDefinition,
} from "./components";
import {
    callbackSkeletons,
    copyComponentSources,
    librarySources,
    type LibraryEntry,
} from "./component-library";
import {
    functionTemplate,
    readFunctionSources,
    saveFunctionSources,
    sourceBundle,
    sourcePath,
} from "./function-sources";
import { csv } from "./scope-data";
import { importLayout } from "./import-layout";
import {
    example,
    stressExample,
    PENDULUM_SOURCE,
    initializeComponentExample,
    type Example,
} from "./examples";
import { useSimulationRun, numericalSource } from "./use-simulation-run";
import {
    DEFINITIONS,
    DEFAULT_EXECUTION,
    commit,
    copySelection,
    definition,
    edgeId,
    isBlockType,
    kind,
    label,
    numericLiteral,
    parameterText,
    parseDocument,
    pasteFragment,
    ports,
    redo,
    removeSelection,
    serializeDocument,
    undo,
    type Block,
    type BlockType,
    type History,
    type ModelDocument,
    type Point,
} from "./model";
const ImportPreview = lazy(() => import("./SlxPreview"));
const uid = () =>
    `block_${crypto.randomUUID().replaceAll("-", "").slice(0, 16)}`;
const STATUS = {
    idle: "就绪",
    checking: "检查模型…",
    compiling: "准备运行…",
    running: "仿真中",
    cancelling: "正在停止…",
    finished: "已完成",
    cancelled: "已停止",
    failed: "运行失败",
};
const interactiveTarget = (target: EventTarget | null) =>
    target instanceof HTMLElement &&
    Boolean(
        target.closest(
            "input,textarea,select,[contenteditable=true],.monaco-editor,[role=separator]",
        ),
    );
interface Props {
    workspace: WorkspaceClient;
    rootPath: string;
    rootGeneration: number;
    wsUrl?: string | undefined;
    theme: IdeTheme;
    visible: boolean;
    onClose(): void;
    onSaved(): void;
    onOpenNative?(): void;
    openRequest?: { path: string; serial: number } | null;
    sessionRef: RefObject<ModelEditorSession | null>;
    pendingSaves: PendingOperations;
    sourceWorkspace?: DesignerSourceWorkspace;
}
interface Draft {
    content: string;
    saved: string;
    file: Pick<WorkspaceFile, "path" | "revision"> | null;
}
function initialDraft(key: string): Draft {
    try {
        const stored = JSON.parse(
            localStorage.getItem(key) ?? "null",
        ) as Draft | null;
        if (
            stored &&
            typeof stored.content === "string" &&
            typeof stored.saved === "string"
        ) {
            parseDocument(stored.content);
            if (
                stored.file &&
                (typeof stored.file.path !== "string" ||
                    typeof stored.file.revision !== "string")
            )
                throw new Error("file");
            return stored;
        }
    } catch {
        /* Corrupt/unavailable recovery never prevents opening the editor. */
    }
    const content = serializeDocument(example("feedback"));
    return { content, saved: content, file: null };
}
function ResizeBar({
    direction,
    name,
    value,
    min,
    max,
    onChange,
    reverse = false,
}: {
    direction: "horizontal" | "vertical";
    name: string;
    value: number;
    min: number;
    max: number;
    onChange(value: number): void;
    reverse?: boolean;
}) {
    const origin = useRef<{ at: number; value: number } | null>(null);
    const change = (value: number) =>
        onChange(Math.max(min, Math.min(max, value)));
    return (
        <div
            role="separator"
            tabIndex={0}
            aria-label={name}
            aria-orientation={direction}
            aria-valuemin={min}
            aria-valuemax={max}
            aria-valuenow={Math.round(value)}
            className={`sim-resizer ${direction}`}
            onPointerDown={(event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                event.currentTarget.setPointerCapture(event.pointerId);
                origin.current = {
                    at:
                        direction === "vertical"
                            ? event.clientX
                            : event.clientY,
                    value,
                };
            }}
            onPointerMove={(event) => {
                if (!origin.current) return;
                change(
                    origin.current.value +
                        ((direction === "vertical"
                            ? event.clientX
                            : event.clientY) -
                            origin.current.at) *
                            (reverse ? -1 : 1),
                );
            }}
            onPointerUp={(event) => {
                origin.current = null;
                event.currentTarget.releasePointerCapture(event.pointerId);
            }}
            onPointerCancel={() => {
                origin.current = null;
            }}
            onKeyDown={(event) => {
                if (
                    [
                        "ArrowLeft",
                        "ArrowRight",
                        "ArrowUp",
                        "ArrowDown",
                    ].includes(event.key)
                ) {
                    event.preventDefault();
                    change(
                        value +
                            (["ArrowLeft", "ArrowUp"].includes(event.key)
                                ? -20
                                : 20),
                    );
                }
            }}
        />
    );
}

function Editor(props: Props) {
    const { workspace, rootPath, rootGeneration, theme, visible, sessionRef } =
        props;
    const platform = usePlatformServices(),
        flow = useReactFlow<FlowBlock, SignalEdge>();
    const storageKey = `openmat.simulation.draft.v1:${rootPath}`;
    const [initial] = useState(() => initialDraft(storageKey));
    const [history, setHistory] = useState<History>(() => ({
        past: [],
        present: parseDocument(initial.content),
        future: [],
    }));
    const doc = history.present,
        docRef = useRef(doc);
    docRef.current = doc;
    const source = useMemo(() => serializeDocument(doc), [doc]);
    const [saved, setSaved] = useState(() => {
        try {
            return serializeDocument(parseDocument(initial.saved));
        } catch {
            return "";
        }
    });
    const [file, setFile] = useState<Draft["file"]>(initial.file);
    const [error, setError] = useState<string | null>(null),
        [notice, setNotice] = useState<string | null>(null);
    const invalidParameter = useRef(false);
    const [fileBusy, setFileBusy] = useState(false),
        busyRef = useRef(false);
    busyRef.current = fileBusy;
    const [filter, setFilter] = useState("");
    const [selected, setSelected] = useState<string[]>([]),
        [selectedEdges, setSelectedEdges] = useState<string[]>([]);
    const selectedRef = useRef(selected);
    selectedRef.current = selected;
    const [flowNodes, setFlowNodes] = useState<FlowBlock[]>([]);
    const [menu, setMenu] = useState<{
        x: number;
        y: number;
        node?: string;
        edge?: string;
    } | null>(null);
    const [modal, setModal] = useState<"open" | "save" | null>(null),
        [pathInput, setPathInput] = useState("");
    const [fileOptions, setFileOptions] = useState<string[]>([]);
    const [pendingAction, setPendingAction] = useState<{
        title: string;
        action(): void;
        cancel?(): void;
    } | null>(null);
    const afterSave = useRef<(() => void) | null>(null);
    const cancelSwitch = useRef<(() => void) | null>(null);
    const [preview, setPreview] = useState<SlxImport | null>(null);
    const [slxView, setSlxView] = useState(Boolean(doc.slx));
    const [slxSystem, setSlxSystem] = useState(0);
    const [slxSelected, setSlxSelected] = useState<string | null>(null);
    const structure = useMemo<SlxImport | null>(
        () =>
            preview ??
            (doc.slx && slxView
                ? {
                      runnable: doc.slx.runnable,
                      document: doc.slx.document,
                      issues: doc.slx.issues,
                  }
                : null),
        [preview, doc.slx, slxView],
    );
    const originalBlock = structure?.document.systems[slxSystem]?.blocks.find(
        (b) => b.sid === slxSelected,
    );
    const slxReady =
        !doc.slx ||
        (doc.slx.runnable && doc.slx.parameters === doc.slx.appliedParameters);
    const [pane, setPane] = useState<"scope" | "diagnostics" | "code">("scope");
    const [codePath, setCodePath] = useState<string | null>(null);
    const [componentDraft, setComponentDraft] = useState<{
        definition: ComponentDefinition;
        isNew: boolean;
    } | null>(null);
    const [libraryRefresh, setLibraryRefresh] = useState(0);
    const draggedComponent = useRef<LibraryEntry | null>(null);
    const [codeReveal, setCodeReveal] = useState<{
        lineNumber: number;
        column: number;
        requestId: number;
    }>();
    const [left, setLeft] = useState(245),
        [right, setRight] = useState(270),
        [bottom, setBottom] = useState(240);
    const importInput = useRef<HTMLInputElement>(null),
        shell = useRef<HTMLDivElement>(null);
    const clipboard = useRef<ModelDocument | null>(null),
        pasteCount = useRef(0);
    const recoveryPaused = useRef(false),
        alive = useRef(true);
    const draftRef = useRef<Draft>({ content: source, saved, file });
    draftRef.current = { content: source, saved, file };
    const run = useSimulationRun(props.wsUrl);
    const runRef = useRef(run);
    runRef.current = run;
    useEffect(() => {
        if (run.status === "failed") setPane("diagnostics");
    }, [run.status]);
    const sourceWorkspaceRef = useRef(props.sourceWorkspace);
    sourceWorkspaceRef.current = props.sourceWorkspace;
    const sourceIsDirty = () =>
        modelSources(docRef.current.model).some((reference) => {
            if (
                docRef.current.sources &&
                Object.hasOwn(docRef.current.sources, reference)
            )
                return false;
            const source = sourceWorkspaceRef.current?.getSource(
                sourcePath(draftRef.current.file?.path ?? null, reference),
            );
            return (
                source &&
                (!source.revision || source.content !== source.savedContent)
            );
        });
    const sourceIsDirtyRef = useRef(sourceIsDirty);
    sourceIsDirtyRef.current = sourceIsDirty;
    const dirty = source !== saved || sourceIsDirty();
    const numerical = useMemo(
        () =>
            numericalSource(doc.model, {
                sources: Object.fromEntries(
                    modelSources(doc.model).map((reference) => [
                        reference,
                        doc.sources?.[reference] ??
                            props.sourceWorkspace?.getSource(
                                sourcePath(file?.path ?? null, reference),
                            )?.content ??
                            "",
                    ]),
                ),
                execution: doc.execution ?? DEFAULT_EXECUTION,
            }),
        [doc.model, doc.execution, doc.sources, file, props.sourceWorkspace],
    );
    const operable = !preview && !fileBusy;
    const editable = operable && !structure;
    const report = useCallback(
        (error: unknown) =>
            setError(error instanceof Error ? error.message : String(error)),
        [],
    );
    const edit = useCallback((next: ModelDocument, regenerated = false) => {
        let version = next.model.schemaVersion;
        if (
            next.model.sampleTimes ||
            next.model.blocks.some((b) =>
                [
                    "zeroOrderHold",
                    "discreteIntegrator",
                    "rateTransition",
                ].includes(b.kind.type),
            )
        )
            version = 5;
        else if (
            version < 4 &&
            (next.sources ||
                next.slx ||
                next.model.blocks.some((b) => b.kind.type === "step"))
        )
            version = 4;
        else if (
            version < 3 &&
            (next.model.components?.length ||
                next.model.blocks.some((b) => b.kind.type === "component"))
        )
            version = 3;
        else if (
            version < 2 &&
            next.model.blocks.some((b) => b.kind.type === "mFunction")
        )
            version = 2;
        next.model.schemaVersion = version;
        if (next.schemaVersion < version) next.schemaVersion = version;
        if (next.execution && next.schemaVersion < 2) next.schemaVersion = 2;
        if (
            !regenerated &&
            next.slx &&
            numericalSource(next.model) !==
                numericalSource(docRef.current.model)
        )
            next.slx.snapshotEdited = true;
        invalidParameter.current = false;
        docRef.current = next;
        setHistory((previous) => commit(previous, next));
        setError(null);
    }, []);
    const change = useCallback(
        (mutate: (next: ModelDocument) => void) => {
            const next = structuredClone(docRef.current);
            mutate(next);
            edit(next);
        },
        [edit],
    );
    const cache = useCallback(() => {
        if (recoveryPaused.current) return;
        try {
            localStorage.setItem(storageKey, JSON.stringify(draftRef.current));
        } catch {
            if (alive.current)
                setNotice("浏览器无法缓存当前草稿，请及时保存模型文件。");
        }
    }, [storageKey]);
    useEffect(() => {
        const timer = setTimeout(cache, 250);
        return () => clearTimeout(timer);
    }, [source, saved, file, cache]);
    useEffect(() => {
        alive.current = true;
        return () => {
            cache();
            cancelSwitch.current?.();
            alive.current = false;
        };
    }, [cache]);
    useEffect(() => {
        const unload = (event: BeforeUnloadEvent) => {
            cache();
            if (dirty || runRef.current.busy) {
                event.preventDefault();
                event.returnValue = "";
            }
        };
        window.addEventListener("beforeunload", unload);
        return () => window.removeEventListener("beforeunload", unload);
    }, [cache, dirty]);
    useEffect(() => {
        setFlowNodes(
            doc.model.blocks.map((block) => ({
                id: block.id,
                type: "block",
                position: block.position ?? { x: 0, y: 0 },
                selected: selected.includes(block.id),
                data: {
                    block,
                    component: componentFor(doc.model, block),
                    label: label(doc, block),
                    invalid: run.diagnostics.some(
                        (issue) =>
                            issue.code !== "valid" && issue.block === block.id,
                    ),
                },
            })),
        );
    }, [
        doc.model.blocks,
        doc.model.components,
        doc.editor.labels,
        selected,
        run.diagnostics,
    ]);
    useEffect(() => {
        if (visible) {
            const timer = setTimeout(() => shell.current?.focus(), 0);
            return () => clearTimeout(timer);
        }
    }, [visible]);
    const replace = useCallback(
        (
            next: ModelDocument,
            location: Draft["file"] = null,
            savedContent?: string,
        ) => {
            setHistory({ past: [], present: next, future: [] });
            docRef.current = next;
            invalidParameter.current = false;
            setSaved(savedContent ?? serializeDocument(next));
            setFile(location);
            setSelected([]);
            setSelectedEdges([]);
            setPreview(null);
            setSlxView(Boolean(next.slx));
            setSlxSystem(0);
            setSlxSelected(null);
            setError(null);
            setNotice(null);
            setMenu(null);
            setCodePath(null);
            setCodeReveal(undefined);
            setPane("scope");
            draftRef.current = {
                content: serializeDocument(next),
                saved: savedContent ?? serializeDocument(next),
                file: location,
            };
            runRef.current.reset();
            requestAnimationFrame(() => {
                if (next.editor.viewport)
                    void flow.setViewport(next.editor.viewport);
                else if (!next.model.blocks.length)
                    void flow.setViewport({ x: 60, y: 50, zoom: 1 });
                else
                    void flow.fitView({
                        padding: 0.2,
                        maxZoom: 1.15,
                        duration: 180,
                    });
            });
        },
        [flow],
    );
    const confirmReplace = useCallback((title: string, action: () => void) => {
        if (runRef.current.busy || busyRef.current) {
            setError("请先停止当前仿真，再打开或新建模型。");
            return;
        }
        if (
            draftRef.current.content !== draftRef.current.saved ||
            sourceIsDirtyRef.current()
        )
            setPendingAction({ title, action });
        else action();
    }, []);
    const completeSave = useCallback(
        (snapshot: string, location: Draft["file"], displayPath: string) => {
            setFile(location);
            setSaved(snapshot);
            setModal(null);
            setNotice(`已保存 ${displayPath}`);
            draftRef.current = {
                content: serializeDocument(docRef.current),
                saved: snapshot,
                file: location,
            };
            props.onSaved();
            const next = afterSave.current;
            afterSave.current = null;
            if (
                next &&
                draftRef.current.content === snapshot &&
                !sourceIsDirtyRef.current()
            )
                next();
            else if (next) {
                cancelSwitch.current?.();
                setNotice("已保存，但保存期间又有新修改，已保留当前模型。");
            }
        },
        [props.onSaved],
    );
    const saveToWorkspace = useCallback(
        async (path: string): Promise<boolean> =>
            props.pendingSaves.run(async () => {
                if (busyRef.current || preview) return false;
                const documentSnapshot = structuredClone(docRef.current);
                const snapshot = serializeDocument(documentSnapshot);
                const modelSnapshot = documentSnapshot.model;
                busyRef.current = true;
                setFileBusy(true);
                setError(null);
                try {
                    const target = path.trim();
                    if (!target.toLowerCase().endsWith(".omsim"))
                        throw new Error("模型文件名应以 .omsim 结尾。");
                    const sources = await readFunctionSources(
                        modelSnapshot,
                        file?.path ?? null,
                        workspace,
                        sourceWorkspaceRef.current,
                        documentSnapshot.sources,
                    );
                    await saveFunctionSources(
                        sources.filter(
                            (s) =>
                                !Object.hasOwn(
                                    documentSnapshot.sources ?? {},
                                    s.reference,
                                ),
                        ),
                        target,
                        workspace,
                        rootGeneration,
                        sourceWorkspaceRef.current,
                    );
                    let revision = file?.path === target ? file.revision : null;
                    if (revision === null) {
                        await workspace.create(target, "file");
                        revision = (await workspace.read(target)).revision;
                        if (alive.current) setFile({ path: target, revision });
                    }
                    const result = await workspace.write(
                        target,
                        snapshot,
                        revision,
                        rootGeneration,
                    );
                    if (alive.current)
                        completeSave(
                            snapshot,
                            {
                                path: target,
                                revision:
                                    result.revision ??
                                    (await workspace.read(target)).revision,
                            },
                            target,
                        );
                    return true;
                } catch (error) {
                    if (alive.current) report(error);
                    return false;
                } finally {
                    busyRef.current = false;
                    if (alive.current) setFileBusy(false);
                }
            }),
        [
            completeSave,
            file,
            preview,
            props.pendingSaves,
            report,
            rootGeneration,
            workspace,
        ],
    );
    const save = useCallback(
        async (saveAs = false): Promise<boolean> => {
            if (preview || busyRef.current) return false;
            if (!saveAs && file) return saveToWorkspace(file.path);
            if (
                !platform.files ||
                modelSources(docRef.current.model).some(
                    (p) => !Object.hasOwn(docRef.current.sources ?? {}, p),
                )
            ) {
                setPathInput(
                    `${docRef.current.model.name.replace(/[<>:"/\\|?*]/g, "_")}.omsim`,
                );
                setModal("save");
                return false;
            }
            return props.pendingSaves.run(async () => {
                const snapshot = serializeDocument(docRef.current);
                busyRef.current = true;
                setFileBusy(true);
                try {
                    const location = await platform.files!.saveTextFile(
                        `${docRef.current.model.name.replace(/[<>:"/\\|?*]/g, "_")}.omsim`,
                        rootPath,
                        snapshot,
                    );
                    if (!location) return false;
                    const prefix = `${nativeFileKey(rootPath).replace(/\/$/, "")}/`,
                        absolute = nativeFileKey(location.path);
                    let savedFile: Draft["file"] = null;
                    if (absolute.startsWith(prefix)) {
                        const relative = location.path
                            .replaceAll("\\", "/")
                            .slice(prefix.length);
                        const written = await workspace.read(relative);
                        savedFile = {
                            path: relative,
                            revision: written.revision,
                        };
                    }
                    if (alive.current)
                        completeSave(snapshot, savedFile, location.path);
                    return true;
                } catch (error) {
                    if (alive.current) report(error);
                    return false;
                } finally {
                    busyRef.current = false;
                    if (alive.current) setFileBusy(false);
                }
            });
        },
        [
            completeSave,
            file,
            platform,
            preview,
            props.pendingSaves,
            report,
            rootPath,
            saveToWorkspace,
            workspace,
        ],
    );
    useLayoutEffect(() => {
        const session: ModelEditorSession = {
            path: file?.path ?? `${doc.model.name}.omsim`,
            dirty,
            hasSavedDesign: Boolean(file),
            busy: fileBusy || run.busy,
            error,
            save: () => save(),
            flush(discard) {
                if (discard) {
                    recoveryPaused.current = true;
                    try {
                        localStorage.removeItem(storageKey);
                    } catch {
                        /* optional storage */
                    }
                } else cache();
            },
            resume() {
                recoveryPaused.current = false;
                cache();
            },
            prepareSwitch() {
                if (busyRef.current || runRef.current.busy) {
                    setError("请等待文件操作结束或停止仿真，再切换当前目录。");
                    return Promise.resolve(false);
                }
                if (
                    draftRef.current.content === draftRef.current.saved &&
                    !sourceIsDirtyRef.current()
                )
                    return Promise.resolve(true);
                return new Promise<boolean>((resolve) => {
                    cancelSwitch.current?.();
                    cancelSwitch.current = () => {
                        resolve(false);
                        cancelSwitch.current = null;
                    };
                    setPendingAction({
                        title: "切换当前目录",
                        action: () => {
                            cancelSwitch.current = null;
                            resolve(true);
                        },
                        cancel: () => cancelSwitch.current?.(),
                    });
                });
            },
        };
        sessionRef.current = session;
        return () => {
            if (sessionRef.current === session) sessionRef.current = null;
        };
    }, [
        cache,
        dirty,
        doc.model.name,
        error,
        file,
        fileBusy,
        run.busy,
        save,
        sessionRef,
        storageKey,
    ]);

    const acceptImport = useCallback(
        (
            result: SlxImport,
            asset: Pick<SlxAsset, "name" | "package" | "parameters">,
        ) => {
            const next = parseDocument(
                JSON.stringify(
                    result.model ?? {
                        ...example("blank").model,
                        schemaVersion: 4,
                        name: result.document.name,
                    },
                ),
            );
            next.schemaVersion =
                next.model.schemaVersion < 4 ? 4 : next.model.schemaVersion;
            next.sources = result.sources ?? {};
            next.slx = {
                ...asset,
                profile: result.profile ?? "control-v1",
                ...(result.sampling ? { sampling: result.sampling } : {}),
                appliedParameters: asset.parameters,
                runnable: result.runnable,
                issues: result.issues,
                document: result.document,
            };
            next.model.name = next.model.name
                .replace(/^.*[\\/]/, "")
                .replace(/\.slx$/i, "");
            const positions = importLayout(
                next.model.blocks.map((block) => block.position!),
            );
            next.model.blocks.forEach((block, index) => {
                block.position = positions[index]!;
            });
            for (const system of result.document.systems)
                for (const block of system.blocks) {
                    const node = next.model.blocks.find(
                        (item) =>
                            item.id === `slx_${block.sid.replaceAll(":", "_")}`,
                    );
                    if (node) next.editor.labels[node.id] = block.name;
                }
            replace(next);
            runRef.current.setDiagnostics(result.issues);
            if (!result.runnable) setPane("diagnostics");
            setNotice(
                result.runnable
                    ? "SLX 已导入。原始层级、参数和数值源码将随 OpenMat 模型保存。"
                    : "此 SLX 暂不能运行。请查看诊断；缺失的变量可在左侧参数区补充。",
            );
        },
        [replace],
    );
    const applySlxParameters = async () => {
        const asset = docRef.current.slx;
        if (!asset || busyRef.current || runRef.current.busy) return;
        busyRef.current = true;
        setFileBusy(true);
        setError(null);
        try {
            const result = await runRef.current.client.current!.importSlx(
                new Blob([decodeSlx(asset.package)]),
                asset.name,
                asset.parameters,
                asset.profile ?? "control-v1",
            );
            if (!alive.current) return;
            const next = structuredClone(docRef.current);
            next.slx = {
                ...asset,
                profile: result.profile ?? asset.profile ?? "control-v1",
                ...(result.sampling ? { sampling: result.sampling } : {}),
                runnable: result.runnable,
                issues: result.issues,
                document: result.document,
            };
            if (!result.sampling) delete next.slx.sampling;
            if (result.runnable && result.model) {
                const imported = parseDocument(JSON.stringify(result.model));
                const oldPositions = new Map(
                    next.model.blocks.map((b) => [b.id, b.position]),
                );
                const positions = importLayout(
                    imported.model.blocks.map((b) => b.position!),
                );
                imported.model.blocks.forEach((b, i) => {
                    b.position = oldPositions.get(b.id) ?? positions[i]!;
                });
                next.model = imported.model;
                next.sources = result.sources ?? {};
                next.editor = imported.editor;
                for (const system of result.document.systems)
                    for (const block of system.blocks) {
                        const id = `slx_${block.sid.replaceAll(":", "_")}`;
                        if (next.model.blocks.some((b) => b.id === id))
                            next.editor.labels[id] = block.name;
                    }
                next.slx.appliedParameters = asset.parameters;
                delete next.slx.snapshotEdited;
            }
            edit(next, true);
            runRef.current.reset();
            runRef.current.setDiagnostics(result.issues);
            setPane(result.runnable ? "scope" : "diagnostics");
            setNotice(
                result.runnable
                    ? "参数已应用，数值模型已重新生成。"
                    : "参数检查未通过，运行已禁用；请查看诊断。",
            );
        } catch (error) {
            report(error);
        } finally {
            busyRef.current = false;
            if (alive.current) setFileBusy(false);
        }
    };
    const load = useCallback(
        async (path: string) => {
            if (busyRef.current) return;
            busyRef.current = true;
            setFileBusy(true);
            setError(null);
            try {
                if (path.toLowerCase().endsWith(".slx")) {
                    const ticket = await workspace.prepareDownload(
                        path,
                        rootGeneration,
                    );
                    if (ticket.size > 2 * 1024 * 1024)
                        throw new Error("交互式 SLX 导入上限为 2 MiB。");
                    const response = await fetch(ticket.url);
                    if (!response.ok)
                        throw new Error(`读取 SLX 失败：${response.status}`);
                    const blob = await response.blob();
                    const result =
                        await runRef.current.client.current!.importSlx(
                            blob,
                            path,
                        );
                    if (alive.current)
                        acceptImport(result, {
                            name: path,
                            package: encodeSlx(
                                new Uint8Array(await blob.arrayBuffer()),
                            ),
                            parameters: "",
                        });
                } else {
                    const opened = await workspace.read(path),
                        next = parseDocument(opened.content);
                    if (alive.current) {
                        replace(
                            next,
                            path.toLowerCase().endsWith(".omsim")
                                ? { path, revision: opened.revision }
                                : null,
                        );
                        setNotice(`已打开 ${path}`);
                    }
                }
                if (alive.current) setModal(null);
            } catch (error) {
                if (alive.current) report(error);
            } finally {
                busyRef.current = false;
                if (alive.current) setFileBusy(false);
            }
        },
        [acceptImport, replace, report, rootGeneration, workspace],
    );
    const handledOpen = useRef<number | null>(null);
    useEffect(() => {
        if (
            props.openRequest &&
            handledOpen.current !== props.openRequest.serial
        ) {
            handledOpen.current = props.openRequest.serial;
            const path = props.openRequest.path;
            confirmReplace(`打开 ${path}`, () => void load(path));
        }
    }, [confirmReplace, load, props.openRequest]);
    const open = () =>
        confirmReplace("打开其他模型", () => {
            if (platform.files && props.onOpenNative) props.onOpenNative();
            else {
                setPathInput("");
                setFileOptions([]);
                setModal("open");
                void workspace
                    .list("", true)
                    .then((snapshot) =>
                        setFileOptions(
                            snapshot.entries
                                .filter(
                                    (entry) =>
                                        entry.kind === "file" &&
                                        /\.(omsim|slx|json)$/i.test(
                                            entry.path,
                                        ) &&
                                        !entry.path.endsWith(".omblock.json"),
                                )
                                .map((entry) => entry.path),
                        ),
                    )
                    .catch(report);
            }
        });
    const importFile = async (incoming: File) => {
        setFileBusy(true);
        busyRef.current = true;
        try {
            if (incoming.name.toLowerCase().endsWith(".slx"))
                acceptImport(
                    await runRef.current.client.current!.importSlx(
                        incoming,
                        incoming.name,
                    ),
                    {
                        name: incoming.name,
                        package: encodeSlx(
                            new Uint8Array(await incoming.arrayBuffer()),
                        ),
                        parameters: "",
                    },
                );
            else {
                if (incoming.size > 8 * 1024 * 1024)
                    throw new Error("模型文件超过 8 MiB。");
                replace(parseDocument(await incoming.text()));
                setNotice("本地文件已导入，可保存到当前工作目录。");
            }
        } catch (error) {
            report(error);
        } finally {
            setFileBusy(false);
            busyRef.current = false;
        }
    };
    const add = useCallback(
        (type: BlockType, position?: Point) => {
            if (!editable) return;
            if (type === "component") {
                setComponentDraft({
                    definition: blankComponent(`custom_${uid().slice(6)}`),
                    isNew: true,
                });
                return;
            }
            const bounds = shell.current
                ?.querySelector(".sim-canvas")
                ?.getBoundingClientRect();
            const at =
                position ??
                flow.screenToFlowPosition({
                    x: (bounds?.left ?? 300) + (bounds?.width ?? 600) / 2 - 80,
                    y: (bounds?.top ?? 100) + (bounds?.height ?? 400) / 2 - 45,
                });
            const id = uid();
            const blockKind = kind(type);
            if (blockKind.type === "mFunction") {
                const name = `fn_${id.slice(6)}`;
                blockKind.source = `${name}.m`;
                blockKind.entry = name;
                const path = sourcePath(
                    draftRef.current.file?.path ?? null,
                    blockKind.source,
                );
                sourceWorkspaceRef.current?.ensureSource({
                    path,
                    content: functionTemplate(blockKind),
                    savedContent: "",
                    file: null,
                });
            }
            change((next) => {
                if (
                    [
                        "zeroOrderHold",
                        "discreteIntegrator",
                        "rateTransition",
                    ].includes(type)
                ) {
                    next.model.sampleTimes ??= {};
                    next.model.sampleTimes[id] = {
                        kind: "discrete",
                        period:
                            next.model.settings.sampleTime ??
                            next.model.settings.maxStep,
                    };
                }
                next.model.blocks.push({
                    id,
                    kind: blockKind,
                    position: {
                        x: Math.round(at.x / 10) * 10,
                        y: Math.round(at.y / 10) * 10,
                    },
                });
            });
            setSelected([id]);
            setSelectedEdges([]);
        },
        [change, editable, flow],
    );
    const deleteSelected = useCallback(
        (nodes = selectedRef.current, edges = selectedEdges) => {
            if (!editable) return;
            edit(
                removeSelection(docRef.current, new Set(nodes), new Set(edges)),
            );
            setSelected([]);
            setSelectedEdges([]);
            setMenu(null);
        },
        [edit, editable, selectedEdges],
    );
    const seedComponent = (
        definition: ComponentDefinition,
        sources: Record<string, string>,
    ) => {
        const shared = sourceWorkspaceRef.current;
        if (!shared) throw new Error("当前工作台没有连接 m 源码编辑器。");
        for (const [reference, content] of Object.entries(sources))
            shared.ensureSource({
                path: sourcePath(
                    draftRef.current.file?.path ?? null,
                    reference,
                ),
                content,
                savedContent: "",
                file: null,
            });
        return definition;
    };
    const insertComponent = async (entry: LibraryEntry, position?: Point) => {
        if (!editable || busyRef.current) return;
        busyRef.current = true;
        setFileBusy(true);
        try {
            const next = structuredClone(docRef.current);
            let definition = next.model.components?.find(
                (d) => d.id === entry.definition.id,
            );
            const reused = definition !== undefined;
            if (
                definition &&
                !sameComponentDefinition(definition, entry.definition, false)
            )
                throw new Error(
                    "组件库与模型内相同 ID 的定义不同。请从“模型内的组件”添加实例，或先编辑模型内的定义；独立组件请在库文件中使用新的 ID。",
                );
            let pendingSources: Record<string, string> = {};
            if (!definition) {
                const sources = await librarySources(
                    entry,
                    workspace,
                    sourceWorkspaceRef.current,
                );
                if (!alive.current) return;
                const copied = copyComponentSources(
                    entry.definition,
                    sources,
                    uid().slice(6),
                );
                definition = copied.definition;
                attachComponent(next, definition);
                pendingSources = copied.sources;
            }
            if (
                next.model.schemaVersion < 5 &&
                definition.sampleTime !== undefined
            ) {
                const hasDiscrete = next.model.blocks.some(
                    (b) =>
                        b.kind.type === "unitDelay" ||
                        (componentFor(next.model, b)?.discreteStates ?? 0) > 0,
                );
                if (
                    hasDiscrete &&
                    next.model.settings.sampleTime !== definition.sampleTime
                )
                    throw new Error(
                        "当前模型使用单周期格式；可在模型设置启用多速率采样，再为组件设置独立周期。",
                    );
                next.model.settings.sampleTime = definition.sampleTime;
            }
            const bounds = shell.current
                ?.querySelector(".sim-canvas")
                ?.getBoundingClientRect();
            const at =
                position ??
                flow.screenToFlowPosition({
                    x: (bounds?.left ?? 300) + (bounds?.width ?? 600) / 2 - 80,
                    y: (bounds?.top ?? 100) + (bounds?.height ?? 400) / 2 - 45,
                });
            const id = uid();
            next.model.blocks.push({
                id,
                kind: {
                    type: "component",
                    component: definition.id,
                    parameters: {},
                },
                position: {
                    x: Math.round(at.x / 10) * 10,
                    y: Math.round(at.y / 10) * 10,
                },
            });
            if (modelSources(next.model).length > 64)
                throw new Error("模型最多引用 64 个 m 源码文件。");
            seedComponent(definition, pendingSources);
            edit(next);
            setSelected([id]);
            setSelectedEdges([]);
            if (reused)
                setNotice(
                    "已添加实例，使用当前模型内保存的组件定义与 m 源码。",
                );
        } catch (e) {
            report(e);
        } finally {
            busyRef.current = false;
            if (alive.current) setFileBusy(false);
        }
    };
    const applyComponent = (definition: ComponentDefinition) => {
        if (!componentDraft) return;
        if (componentDraft.isNew) {
            setComponentDraft(null);
            void insertComponent({
                key: `new:${definition.id}`,
                definition,
                sources: callbackSkeletons(definition),
            });
            return;
        }
        const previous = componentDraft.definition;
        const newSources = callbackSkeletons(definition);
        const oldPaths = new Set(callbacks(previous).map((c) => c.source));
        const next = structuredClone(docRef.current);
        next.model.components = next.model.components!.map((d) =>
            d.id === definition.id ? definition : d,
        );
        let resetParameters = 0;
        for (const block of next.model.blocks) {
            if (
                block.kind.type !== "component" ||
                block.kind.component !== definition.id
            )
                continue;
            const current = block.kind;
            current.parameters = Object.fromEntries(
                Object.entries(current.parameters).filter(([name, value]) => {
                    try {
                        validateComponentKind(
                            { ...current, parameters: { [name]: value } },
                            [definition],
                        );
                        return true;
                    } catch {
                        resetParameters++;
                        return false;
                    }
                }),
            );
        }
        if (
            next.model.schemaVersion < 5 &&
            definition.sampleTime !== undefined
        ) {
            if (
                next.model.blocks.some((b) => {
                    const d = componentFor(next.model, b);
                    return (
                        d?.sampleTime !== undefined &&
                        d.sampleTime !== definition.sampleTime
                    );
                })
            )
                throw new Error(
                    "当前模型使用单周期格式；请在模型设置启用多速率采样。",
                );
            next.model.settings.sampleTime = definition.sampleTime;
        }
        if (modelSources(next.model).length > 64)
            throw new Error("模型最多引用 64 个 m 源码文件。");
        seedComponent(
            definition,
            Object.fromEntries(
                Object.entries(newSources).filter(
                    ([path]) => !oldPaths.has(path),
                ),
            ),
        );
        const byId = new Map(next.model.blocks.map((b) => [b.id, b]));
        next.model.connections = next.model.connections.filter(
            (e) =>
                ports(
                    byId.get(e.from.block)!,
                    next.model.components,
                ).outputs.includes(e.from.port) &&
                ports(
                    byId.get(e.to.block)!,
                    next.model.components,
                ).inputs.includes(e.to.port),
        );
        const kept = new Set(next.model.connections.map(edgeId));
        for (const id of Object.keys(next.editor.bends))
            if (!kept.has(id)) delete next.editor.bends[id];
        edit(next);
        setComponentDraft(null);
        setNotice(
            `已更新模型内的组件定义。${resetParameters ? `${resetParameters} 个不再符合定义的参数覆盖已移除。` : ""}请检查回调的输出宽度，再运行模型检查。`,
        );
    };
    const saveComponentLibrary = async (block: Block) => {
        const definition = componentFor(docRef.current.model, block);
        if (!definition || busyRef.current) return;
        busyRef.current = true;
        setFileBusy(true);
        try {
            await props.pendingSaves.run(async () => {
                const sources = await readFunctionSources(
                    { ...docRef.current.model, blocks: [block] },
                    draftRef.current.file?.path ?? null,
                    workspace,
                    sourceWorkspaceRef.current,
                );
                const target = sourcePath(
                    draftRef.current.file?.path ?? null,
                    "placeholder.m",
                ).replace(
                    /placeholder\.m$/,
                    `${definition.id}_${uid().slice(6, 14)}.omblock.json`,
                );
                await saveFunctionSources(
                    sources,
                    target,
                    workspace,
                    rootGeneration,
                    sourceWorkspaceRef.current,
                );
                await workspace.create(target, "file");
                const file = await workspace.read(target);
                await workspace.write(
                    target,
                    componentFile(definition),
                    file.revision,
                    rootGeneration,
                );
                if (alive.current) {
                    setLibraryRefresh((v) => v + 1);
                    setNotice(`组件已保存：${target}`);
                    props.onSaved();
                }
            });
        } catch (e) {
            report(e);
        } finally {
            busyRef.current = false;
            if (alive.current) setFileBusy(false);
        }
    };
    const copy = useCallback((ids = selectedRef.current) => {
        clipboard.current = copySelection(docRef.current, new Set(ids));
        pasteCount.current = 0;
        setNotice(`已复制 ${ids.length} 个方块及内部连线。`);
        setMenu(null);
    }, []);
    const paste = useCallback(() => {
        if (!clipboard.current || !editable) return;
        try {
            const pasted = pasteFragment(
                docRef.current,
                clipboard.current,
                uid,
                40 * ++pasteCount.current,
            );
            edit(pasted.document);
            setSelected(pasted.ids);
            setSelectedEdges([]);
            setMenu(null);
        } catch (error) {
            report(error);
        }
    }, [edit, editable, report]);
    const commitBend = useCallback(
        (id: string, point: Point) =>
            change((next) => {
                next.editor.bends[id] = point;
            }),
        [change],
    );
    const edges = useMemo<SignalEdge[]>(
        () =>
            doc.model.connections.map((edge) => ({
                id: edgeId(edge),
                type: "signal",
                source: edge.from.block,
                sourceHandle: edge.from.port,
                target: edge.to.block,
                targetHandle: edge.to.port,
                selected: selectedEdges.includes(edgeId(edge)),
                markerEnd: {
                    type: MarkerType.ArrowClosed,
                    width: 16,
                    height: 16,
                    color: "var(--text-muted)",
                },
                data: { bend: doc.editor.bends[edgeId(edge)], commitBend },
            })),
        [commitBend, doc.model.connections, doc.editor.bends, selectedEdges],
    );
    const connect = useCallback(
        (connection: FlowConnection) => {
            if (!connection.sourceHandle || !connection.targetHandle) return;
            change((next) => {
                next.model.connections.push({
                    from: {
                        block: connection.source,
                        port: connection.sourceHandle!,
                    },
                    to: {
                        block: connection.target,
                        port: connection.targetHandle!,
                    },
                });
            });
        },
        [change],
    );
    const validConnection = useCallback(
        (connection: FlowConnection | SignalEdge) => {
            const model = docRef.current.model,
                from = model.blocks.find(
                    (block) => block.id === connection.source,
                ),
                to = model.blocks.find(
                    (block) => block.id === connection.target,
                );
            return Boolean(
                from &&
                to &&
                connection.sourceHandle &&
                connection.targetHandle &&
                ports(from, model.components).outputs.includes(
                    connection.sourceHandle,
                ) &&
                ports(to, model.components).inputs.includes(
                    connection.targetHandle,
                ) &&
                !model.connections.some(
                    (edge) =>
                        edge.to.block === connection.target &&
                        edge.to.port === connection.targetHandle,
                ),
            );
        },
        [],
    );
    const focusBlock = useCallback(
        (id: string) => {
            setSelected([id]);
            setSelectedEdges([]);
            const node = flow.getNode(id);
            if (node)
                void flow.fitView({
                    nodes: [node],
                    maxZoom: 1.2,
                    duration: 200,
                    padding: 0.8,
                });
        },
        [flow],
    );
    const executeModel = useCallback(
        async (check = false) => {
            const imported = docRef.current.slx;
            if (
                imported &&
                (!imported.runnable ||
                    imported.parameters !== imported.appliedParameters)
            ) {
                setError("请先在左侧应用 SLX 参数并通过兼容性检查。");
                return;
            }
            if (
                invalidParameter.current ||
                busyRef.current ||
                runRef.current.busy
            )
                return;
            const snapshot = structuredClone(docRef.current);
            busyRef.current = true;
            setFileBusy(true);
            setPane(check ? "diagnostics" : "scope");
            setError(null);
            try {
                const files = await readFunctionSources(
                    snapshot.model,
                    draftRef.current.file?.path ?? null,
                    workspace,
                    sourceWorkspaceRef.current,
                    snapshot.sources,
                );
                const bundle = {
                    sources: sourceBundle(files),
                    execution: snapshot.execution ?? DEFAULT_EXECUTION,
                };
                if (!alive.current) return;
                if (check) await runRef.current.check(snapshot.model, bundle);
                else await runRef.current.run(snapshot.model, bundle);
            } catch (error) {
                if (alive.current) {
                    runRef.current.setDiagnostics([
                        error instanceof SimulationError
                            ? error.diagnostic
                            : { code: "source", message: String(error) },
                    ]);
                    setPane("diagnostics");
                }
            } finally {
                busyRef.current = false;
                if (alive.current) setFileBusy(false);
            }
        },
        [workspace],
    );
    const runModel = useCallback(() => {
        void executeModel();
    }, [executeModel]);
    const openFunction = useCallback(
        async (block: Block, issue?: SimulationDiagnostic) => {
            const reference =
                issue?.sourcePath ??
                (block.kind.type === "mFunction"
                    ? block.kind.source
                    : componentFor(docRef.current.model, block)?.outputsFunction
                          .source);
            if (!reference) return;
            const path = sourcePath(
                draftRef.current.file?.path ?? null,
                reference,
            );
            try {
                if (Object.hasOwn(docRef.current.sources ?? {}, reference)) {
                    setCodePath(reference);
                    setPane("code");
                    setBottom((current) => Math.max(current, 330));
                    return;
                }
                await readFunctionSources(
                    { ...docRef.current.model, blocks: [block] },
                    draftRef.current.file?.path ?? null,
                    workspace,
                    sourceWorkspaceRef.current,
                );
                if (!alive.current) return;
                setCodePath(path);
                setPane("code");
                setCodeReveal({
                    lineNumber: issue?.line ?? 1,
                    column: issue?.column ?? 1,
                    requestId: Date.now(),
                });
                setBottom((current) => Math.max(current, 330));
            } catch (error) {
                report(error);
            }
        },
        [workspace, report],
    );
    useEffect(() => {
        if (!visible) return;
        const keyboard = (event: KeyboardEvent) => {
            if (event.key === "F5") {
                event.preventDefault();
                event.stopImmediatePropagation();
                if (
                    !event.repeat &&
                    !event.isComposing &&
                    !event.ctrlKey &&
                    !event.shiftKey &&
                    !event.altKey &&
                    !modal &&
                    !pendingAction &&
                    !componentDraft &&
                    !preview &&
                    !busyRef.current &&
                    !runRef.current.busy &&
                    runRef.current.connected
                ) {
                    if (document.activeElement instanceof HTMLInputElement)
                        document.activeElement.blur();
                    runModel();
                }
                return;
            }
            const control = event.ctrlKey || event.metaKey;
            if (control && event.key.toLowerCase() === "s") {
                event.preventDefault();
                event.stopImmediatePropagation();
                if (editable && !modal && !pendingAction && !componentDraft) {
                    if (document.activeElement instanceof HTMLInputElement)
                        document.activeElement.blur();
                    void save(event.shiftKey);
                }
                return;
            }
            if (
                interactiveTarget(event.target) ||
                modal ||
                pendingAction ||
                componentDraft
            )
                return;
            if (event.key === "Escape") {
                setMenu(null);
                setSelected([]);
                setSelectedEdges([]);
                return;
            }
            if (!editable) return;
            if (control && event.key.toLowerCase() === "z") {
                event.preventDefault();
                setHistory((current) =>
                    event.shiftKey ? redo(current) : undo(current),
                );
            } else if (control && event.key.toLowerCase() === "y") {
                event.preventDefault();
                setHistory(redo);
            } else if (control && event.key.toLowerCase() === "c") {
                event.preventDefault();
                copy();
            } else if (control && event.key.toLowerCase() === "v") {
                event.preventDefault();
                paste();
            } else if (control && event.key.toLowerCase() === "a") {
                event.preventDefault();
                setSelected(
                    docRef.current.model.blocks.map((block) => block.id),
                );
            } else if (event.key === "Delete" || event.key === "Backspace") {
                event.preventDefault();
                deleteSelected();
            } else if (event.key === "F2") {
                event.preventDefault();
                shell.current
                    ?.querySelector<HTMLInputElement>('[aria-label="方块名称"]')
                    ?.focus();
            }
        };
        window.addEventListener("keydown", keyboard, true);
        return () => window.removeEventListener("keydown", keyboard, true);
    }, [
        copy,
        deleteSelected,
        editable,
        modal,
        paste,
        pendingAction,
        componentDraft,
        preview,
        runModel,
        save,
        visible,
    ]);
    const showMenu = useCallback(
        (
            event: { preventDefault(): void; clientX: number; clientY: number },
            target: { node?: string; edge?: string } = {},
        ) => {
            event.preventDefault();
            if (target.node && !selectedRef.current.includes(target.node)) {
                setSelected([target.node]);
                setSelectedEdges([]);
            }
            if (target.edge) {
                setSelected([]);
                setSelectedEdges([target.edge]);
            }
            setMenu({ x: event.clientX, y: event.clientY, ...target });
        },
        [],
    );
    const onNodesChange = useCallback(
        (changes: NodeChange<FlowBlock>[]) => {
            setFlowNodes((nodes) => applyNodeChanges(changes, nodes));
            const moved = changes.filter(
                (item) =>
                    item.type === "position" &&
                    item.dragging === false &&
                    item.position,
            );
            if (moved.length)
                change((next) => {
                    for (const move of moved) {
                        if (move.type !== "position") continue;
                        const block = next.model.blocks.find(
                            (item) => item.id === move.id,
                        );
                        if (block && move.position)
                            block.position = move.position;
                    }
                });
            const selections = changes.filter((item) => item.type === "select");
            if (selections.length)
                setSelected((previous) => {
                    const next = new Set(previous);
                    for (const selection of selections) {
                        if (selection.selected) next.add(selection.id);
                        else next.delete(selection.id);
                    }
                    return [...next];
                });
        },
        [change],
    );
    const onEdgesChange = useCallback((changes: EdgeChange<SignalEdge>[]) => {
        const selections = changes.filter((item) => item.type === "select");
        if (selections.length)
            setSelectedEdges((previous) => {
                const next = new Set(previous);
                for (const selection of selections) {
                    if (selection.selected) next.add(selection.id);
                    else next.delete(selection.id);
                }
                return [...next];
            });
    }, []);
    const onMoveEnd = useCallback((event: unknown, viewport: Viewport) => {
        if (
            event &&
            JSON.stringify(docRef.current.editor.viewport) !==
                JSON.stringify(viewport)
        ) {
            const next = {
                ...docRef.current,
                editor: { ...docRef.current.editor, viewport },
            };
            docRef.current = next;
            setHistory((current) => ({ ...current, present: next }));
        }
    }, []);
    const onNodeDoubleClick = useCallback(
        (_event: unknown, node: FlowBlock) => {
            setSelected([node.id]);
            if (node.data.block.kind.type === "scope") setPane("scope");
            else if (
                node.data.block.kind.type === "mFunction" ||
                node.data.block.kind.type === "component"
            )
                void openFunction(node.data.block);
            else
                requestAnimationFrame(() =>
                    shell.current
                        ?.querySelector<HTMLInputElement>(
                            '[aria-label="方块参数"]',
                        )
                        ?.focus(),
                );
        },
        [openFunction],
    );
    const onNodeContextMenu = useCallback(
        (event: React.MouseEvent, node: FlowBlock) =>
            showMenu(event, { node: node.id }),
        [showMenu],
    );
    const onEdgeContextMenu = useCallback(
        (event: React.MouseEvent, edge: SignalEdge) =>
            showMenu(event, { edge: edge.id }),
        [showMenu],
    );
    const onPaneContextMenu = useCallback(
        (event: MouseEvent | React.MouseEvent) => showMenu(event),
        [showMenu],
    );
    const onPaneClick = useCallback(() => setMenu(null), []);
    const menuGroups: MenuGroup[] = menu?.edge
        ? [
              {
                  actions: [
                      {
                          label: "删除连线",
                          shortcut: "Delete",
                          run: () => deleteSelected([], [menu.edge!]),
                      },
                      {
                          label: "重置路径",
                          run: () => {
                              change((next) => {
                                  delete next.editor.bends[menu.edge!];
                              });
                              setMenu(null);
                          },
                      },
                  ],
              },
          ]
        : menu?.node
          ? [
                {
                    actions: [
                        {
                            label: "复制",
                            shortcut: "Ctrl+C",
                            run: () => copy(),
                        },
                        {
                            label: "复制并粘贴",
                            run: () => {
                                copy();
                                paste();
                            },
                        },
                        {
                            label: "重命名",
                            shortcut: "F2",
                            run: () => {
                                setMenu(null);
                                requestAnimationFrame(() =>
                                    shell.current
                                        ?.querySelector<HTMLInputElement>(
                                            '[aria-label="方块名称"]',
                                        )
                                        ?.focus(),
                                );
                            },
                        },
                        {
                            label: "删除选中方块",
                            shortcut: "Delete",
                            danger: true,
                            run: () => deleteSelected(),
                        },
                    ],
                },
            ]
          : [
                {
                    actions: [
                        {
                            label: "粘贴",
                            shortcut: "Ctrl+V",
                            disabled: !clipboard.current,
                            run: paste,
                        },
                        {
                            label: "适应画布",
                            run: () => {
                                void flow.fitView({ padding: 0.2 });
                                setMenu(null);
                            },
                        },
                    ],
                },
                {
                    label: "添加方块",
                    actions: DEFINITIONS.map((def) => ({
                        label: def.label,
                        icon: <BlockIcon type={def.type} size={16} />,
                        run: () => {
                            add(
                                def.type,
                                flow.screenToFlowPosition({
                                    x: menu?.x ?? 0,
                                    y: menu?.y ?? 0,
                                }),
                            );
                            setMenu(null);
                        },
                    })),
                },
            ];
    const chosen =
        selected.length === 1
            ? doc.model.blocks.find((block) => block.id === selected[0])
            : undefined;
    const chosenEdge =
        selectedEdges.length === 1
            ? doc.model.connections.find(
                  (edge) => edgeId(edge) === selectedEdges[0],
              )
            : undefined;
    const applyParameter = (block: Block, value: string) => {
        if (block.kind.type === "mFunction" || block.kind.type === "component")
            return;
        try {
            const values =
                block.kind.type === "sum"
                    ? value
                          .replace(/\s/g, "")
                          .replaceAll("−", "-")
                          .split("")
                          .map((char) =>
                              char === "+" ? 1 : char === "-" ? -1 : NaN,
                          )
                    : numericLiteral(value);
            if (
                block.kind.type === "sum" &&
                (values.length < 1 ||
                    values.length > 64 ||
                    values.some(Number.isNaN))
            )
                throw new Error("Sum 输入符号应为 1–64 个 + 或 -，例如 +-。");
            change((next) => {
                const node = next.model.blocks.find(
                    (item) => item.id === block.id,
                )!;
                const type = node.kind.type;
                if (
                    type === "mFunction" ||
                    type === "component" ||
                    type === "step" ||
                    type === "zeroOrderHold"
                )
                    return;
                node.kind =
                    type === "constant"
                        ? { type, value: values }
                        : type === "gain"
                          ? { type, gain: values }
                          : type === "sum"
                            ? { type, signs: values }
                            : type === "scope"
                              ? { type }
                              : ({
                                    ...node.kind,
                                    initial: values,
                                } as Block["kind"]);
                const inputs = ports(node, next.model.components).inputs;
                next.model.connections = next.model.connections.filter(
                    (edge) =>
                        edge.to.block !== node.id ||
                        inputs.includes(edge.to.port),
                );
                const ids = new Set(next.model.connections.map(edgeId));
                for (const id of Object.keys(next.editor.bends))
                    if (!ids.has(id)) delete next.editor.bends[id];
            });
        } catch (error) {
            invalidParameter.current = true;
            report(error);
            return false;
        }
    };
    const exportCsv = async () => {
        try {
            const headers =
                run.info?.scopes.flatMap((scope) =>
                    Array.from(
                        { length: scope.width },
                        (_, i) => `${scope.block}[${i + 1}]`,
                    ),
                ) ?? [];
            await platform.saveExport(
                new Blob(
                    [
                        csv(
                            run.frames.current,
                            headers,
                            run.info?.scopes,
                            run.info?.sampling,
                        ),
                    ],
                    {
                        type: "text/csv;charset=utf-8",
                    },
                ),
                `${doc.model.name}-results.csv`,
            );
        } catch (error) {
            report(error);
        }
    };
    return (
        <div
            ref={shell}
            className="sim-editor"
            tabIndex={-1}
            aria-label="模型编辑器"
            style={
                {
                    "--sim-left": `${left}px`,
                    "--sim-right": `${right}px`,
                    "--sim-bottom": `${bottom}px`,
                } as CSSProperties
            }
        >
            <header className="sim-titlebar">
                <div className="sim-title">
                    <BlockIcon type="integrator" size={24} />
                    <strong>模型编辑器</strong>
                    <span className="sim-document-tab">
                        {preview
                            ? preview.document.name
                            : (file?.path ?? `${doc.model.name}.omsim`)}
                        {dirty && !preview ? " •" : ""}
                    </span>
                </div>
                <span
                    className={`sim-service ${run.connected ? "online" : "offline"}`}
                >
                    {run.connected ? "● Rust 仿真服务" : "○ 仿真服务未连接"}
                </span>
                <button
                    onClick={() =>
                        confirmReplace("关闭模型编辑器", props.onClose)
                    }
                    aria-label="关闭模型编辑器"
                >
                    ✕
                </button>
            </header>
            <div className="sim-toolbar">
                <button
                    disabled={fileBusy}
                    onClick={() =>
                        confirmReplace("新建模型", () =>
                            replace(example("blank")),
                        )
                    }
                >
                    ＋ 新建
                </button>
                <button disabled={fileBusy} onClick={open}>
                    打开
                </button>
                {platform.kind === "web" && (
                    <button
                        disabled={fileBusy}
                        onClick={() =>
                            confirmReplace("导入本地模型", () =>
                                importInput.current?.click(),
                            )
                        }
                    >
                        导入文件
                    </button>
                )}
                <input
                    ref={importInput}
                    hidden
                    type="file"
                    accept=".omsim,.json,.slx"
                    onChange={(event) => {
                        const incoming = event.target.files?.[0];
                        event.target.value = "";
                        if (incoming) void importFile(incoming);
                    }}
                />
                <button disabled={!operable} onClick={() => void save()}>
                    保存
                </button>
                <button disabled={!operable} onClick={() => void save(true)}>
                    另存为
                </button>
                <span className="sim-toolbar-divider" />
                <button
                    title="撤销 Ctrl+Z"
                    aria-label="撤销"
                    disabled={!operable || !history.past.length}
                    onClick={() => setHistory(undo)}
                >
                    ↶
                </button>
                <button
                    title="重做 Ctrl+Shift+Z"
                    aria-label="重做"
                    disabled={!operable || !history.future.length}
                    onClick={() => setHistory(redo)}
                >
                    ↷
                </button>
                <button
                    disabled={
                        !operable || !slxReady || run.busy || !run.connected
                    }
                    onClick={() => {
                        setPane("diagnostics");
                        void executeModel(true);
                    }}
                >
                    检查模型
                </button>
                <button
                    className="sim-run"
                    disabled={
                        !operable || !slxReady || run.busy || !run.connected
                    }
                    onClick={runModel}
                >
                    ▶ 运行 <kbd>F5</kbd>
                </button>
                <button
                    disabled={
                        !run.info || !run.busy || run.status === "cancelling"
                    }
                    onClick={() => void run.cancel()}
                >
                    ■ 停止
                </button>
                <span className="sim-toolbar-spacer" />
                {doc.slx && (
                    <button
                        disabled={fileBusy}
                        onClick={() => {
                            setSlxView(!slxView);
                            setSelected([]);
                            setSelectedEdges([]);
                        }}
                    >
                        {slxView ? "查看 / 编辑数值模型" : "查看原始 SLX 层级"}
                    </button>
                )}
                <select
                    aria-label="打开示例"
                    value=""
                    disabled={run.busy || fileBusy}
                    onChange={(event) => {
                        const value = event.target.value;
                        confirmReplace("打开示例", () => {
                            const next =
                                value === "stress"
                                    ? stressExample()
                                    : example(value as Example);
                            if (value === "pendulum") {
                                // A distinct draft filename preserves existing user source files.
                                const name = `pendulum_${crypto.randomUUID().replaceAll("-", "").slice(0, 8)}`;
                                const block = next.model.blocks.find(
                                    (block) => block.kind.type === "mFunction",
                                )!;
                                if (block.kind.type === "mFunction") {
                                    block.kind.entry = name;
                                    block.kind.source = `${name}.m`;
                                    sourceWorkspaceRef.current?.ensureSource({
                                        path: block.kind.source,
                                        content: PENDULUM_SOURCE.replace(
                                            "pendulum(",
                                            `${name}(`,
                                        ),
                                        savedContent: "",
                                        file: null,
                                    });
                                }
                            }
                            if (next.model.components?.length) {
                                const sources = initializeComponentExample(
                                    next,
                                    uid().slice(6),
                                );
                                for (const [path, content] of Object.entries(
                                    sources,
                                ))
                                    sourceWorkspaceRef.current?.ensureSource({
                                        path,
                                        content,
                                        savedContent: "",
                                        file: null,
                                    });
                            }
                            replace(next);
                        });
                    }}
                >
                    <option value="" disabled>
                        示例模型…
                    </option>
                    <option value="feedback">一阶反馈系统</option>
                    <option value="vector">两个时间常数</option>
                    <option value="counter">离散计数器</option>
                    <option value="pendulum">非线性摆 · m 函数</option>
                    <option value="customDelay">
                        自定义 Unit Delay · m 组件
                    </option>
                    <option value="massSpring">质量—弹簧—阻尼 · m 组件</option>
                    <option value="piControl">离散 PI + 连续系统</option>
                    <option value="multirate">
                        多速率 PI · 10 ms / 100 ms
                    </option>
                    <option value="stress">300 方块交互测试</option>
                </select>
            </div>
            {error && (
                <div className="sim-banner error" role="alert">
                    <span>{error}</span>
                    <button
                        aria-label="关闭错误"
                        onClick={() => {
                            invalidParameter.current = false;
                            setError(null);
                        }}
                    >
                        ✕
                    </button>
                </div>
            )}
            {!run.connected && (
                <div className="sim-banner warning">
                    <span>
                        {run.connectionError ?? "正在连接原生仿真服务…"}
                    </span>
                    <button onClick={() => void run.reconnect()}>
                        重新连接
                    </button>
                </div>
            )}
            {notice && (
                <div className="sim-banner notice">
                    <span>{notice}</span>
                    <button
                        aria-label="关闭提示"
                        onClick={() => setNotice(null)}
                    >
                        ✕
                    </button>
                </div>
            )}
            <div className="sim-workspace">
                <LibraryPanels
                    storageKey="openmat.simulation.library-split.v1"
                    palette={
                        doc.slx && slxView ? (
                            <SlxParameters
                                asset={doc.slx}
                                busy={fileBusy || run.busy}
                                onChange={(parameters) => {
                                    const next = structuredClone(
                                        docRef.current,
                                    );
                                    next.slx!.parameters = parameters;
                                    edit(next);
                                }}
                                onApply={() => void applySlxParameters()}
                                onError={setError}
                            />
                        ) : (
                            <section className="sim-palette">
                                <div className="sim-pane-title">
                                    方块库 <span>{DEFINITIONS.length}</span>
                                </div>
                                <input
                                    aria-label="搜索方块"
                                    className="sim-search"
                                    placeholder="搜索方块…"
                                    value={filter}
                                    onChange={(event) =>
                                        setFilter(event.target.value)
                                    }
                                />
                                <div className="sim-palette-scroll">
                                    {[
                                        ...new Set(
                                            DEFINITIONS.map(
                                                (def) => def.category,
                                            ),
                                        ),
                                    ].map((category) => {
                                        const items = DEFINITIONS.filter(
                                            (def) =>
                                                def.type !== "component" &&
                                                def.category === category &&
                                                `${def.label} ${def.category}`
                                                    .toLowerCase()
                                                    .includes(
                                                        filter.toLowerCase(),
                                                    ),
                                        );
                                        return items.length ? (
                                            <section key={category}>
                                                <h3>{category}</h3>
                                                {items.map((def) => (
                                                    <button
                                                        key={def.type}
                                                        draggable={editable}
                                                        disabled={!editable}
                                                        onDragStart={(
                                                            event,
                                                        ) => {
                                                            event.dataTransfer.setData(
                                                                "application/openmat-block",
                                                                def.type,
                                                            );
                                                            event.dataTransfer.effectAllowed =
                                                                "copy";
                                                        }}
                                                        onClick={() =>
                                                            add(def.type)
                                                        }
                                                        title={`拖入画布或点击添加 ${def.label}`}
                                                    >
                                                        <BlockIcon
                                                            type={def.type}
                                                        />
                                                        <span>{def.label}</span>
                                                        <span className="sim-add-hint">
                                                            ＋
                                                        </span>
                                                    </button>
                                                ))}
                                            </section>
                                        ) : null;
                                    })}
                                    <ComponentLibrary
                                        workspace={workspace}
                                        generation={rootGeneration}
                                        refresh={libraryRefresh}
                                        filter={filter}
                                        disabled={!editable}
                                        definitions={doc.model.components ?? []}
                                        onInsert={(entry) =>
                                            void insertComponent(entry)
                                        }
                                        onDrag={(entry) => {
                                            draggedComponent.current = entry;
                                        }}
                                        onCreate={() =>
                                            setComponentDraft({
                                                definition: blankComponent(
                                                    `custom_${uid().slice(6)}`,
                                                ),
                                                isNew: true,
                                            })
                                        }
                                    />
                                </div>
                            </section>
                        )
                    }
                    tree={
                        structure ? (
                            <SlxTree
                                document={structure.document}
                                active={slxSystem}
                                selected={slxSelected}
                                onSystem={(index) => {
                                    setSlxSystem(index);
                                    setSlxSelected(null);
                                }}
                                onSelect={setSlxSelected}
                            />
                        ) : preview ? (
                            <section className="sim-tree">
                                <div className="sim-pane-title">
                                    SLX 对象 · 只读
                                </div>
                                <div className="sim-tree-scroll">
                                    <div className="sim-tree-root">
                                        {preview.document.name}
                                    </div>
                                    {preview.document.systems
                                        .flatMap((system) => system.blocks)
                                        .map((block) => (
                                            <p
                                                key={block.sid}
                                                className="sim-tree-root"
                                            >
                                                {block.name}{" "}
                                                <small>
                                                    ({block.blockType})
                                                </small>
                                            </p>
                                        ))}
                                </div>
                            </section>
                        ) : (
                            <section className="sim-tree">
                                <div className="sim-pane-title">
                                    模型对象{" "}
                                    <span>{doc.model.blocks.length}</span>
                                </div>
                                <div
                                    className="sim-tree-scroll"
                                    role="tree"
                                    aria-label="模型对象树"
                                >
                                    <div className="sim-tree-root">
                                        ◇ {doc.model.name}
                                    </div>
                                    {doc.model.blocks.map((block) => (
                                        <button
                                            role="treeitem"
                                            aria-selected={selected.includes(
                                                block.id,
                                            )}
                                            key={block.id}
                                            className={
                                                selected.includes(block.id)
                                                    ? "selected"
                                                    : ""
                                            }
                                            onClick={(event) => {
                                                setSelected(
                                                    event.ctrlKey ||
                                                        event.metaKey
                                                        ? selected.includes(
                                                              block.id,
                                                          )
                                                            ? selected.filter(
                                                                  (id) =>
                                                                      id !==
                                                                      block.id,
                                                              )
                                                            : [
                                                                  ...selected,
                                                                  block.id,
                                                              ]
                                                        : [block.id],
                                                );
                                                setSelectedEdges([]);
                                            }}
                                            onDoubleClick={() =>
                                                focusBlock(block.id)
                                            }
                                            onContextMenu={(event) =>
                                                showMenu(event, {
                                                    node: block.id,
                                                })
                                            }
                                            onKeyDown={(event) => {
                                                if (
                                                    event.key ===
                                                        "ContextMenu" ||
                                                    (event.key === "F10" &&
                                                        event.shiftKey)
                                                ) {
                                                    const bounds =
                                                        event.currentTarget.getBoundingClientRect();
                                                    showMenu(
                                                        {
                                                            preventDefault:
                                                                () =>
                                                                    event.preventDefault(),
                                                            clientX:
                                                                bounds.right,
                                                            clientY: bounds.top,
                                                        },
                                                        { node: block.id },
                                                    );
                                                }
                                            }}
                                        >
                                            <BlockIcon
                                                type={
                                                    componentFor(
                                                        doc.model,
                                                        block,
                                                    )?.icon ?? block.kind.type
                                                }
                                                size={17}
                                            />
                                            <span>{label(doc, block)}</span>
                                        </button>
                                    ))}
                                </div>
                            </section>
                        )
                    }
                />
                <ResizeBar
                    direction="vertical"
                    name="方块库宽度"
                    value={left}
                    min={180}
                    max={400}
                    onChange={setLeft}
                />
                <main className="sim-center">
                    <div className="sim-canvas-header">
                        <span>
                            {structure
                                ? "SLX 结构检查"
                                : `${doc.model.blocks.length} blocks · ${doc.model.connections.length} connections`}
                        </span>
                        <span>
                            {structure
                                ? doc.slx?.snapshotEdited
                                    ? "原始结构 · 数值模型已有独立修改"
                                    : "双击子系统进入 · 左侧编辑模型参数"
                                : "双击方块编辑参数 · 空格拖动画布"}
                        </span>
                    </div>
                    <div
                        className="sim-canvas"
                        onDragOver={(event) => {
                            event.preventDefault();
                            event.dataTransfer.dropEffect = "copy";
                        }}
                        onDrop={(event) => {
                            event.preventDefault();
                            const componentKey = event.dataTransfer.getData(
                                "application/openmat-component",
                            );
                            if (
                                componentKey &&
                                draggedComponent.current?.key === componentKey
                            ) {
                                void insertComponent(
                                    draggedComponent.current,
                                    flow.screenToFlowPosition({
                                        x: event.clientX,
                                        y: event.clientY,
                                    }),
                                );
                                draggedComponent.current = null;
                                return;
                            }
                            const type = event.dataTransfer.getData(
                                "application/openmat-block",
                            );
                            if (isBlockType(type))
                                add(
                                    type,
                                    flow.screenToFlowPosition({
                                        x: event.clientX,
                                        y: event.clientY,
                                    }),
                                );
                        }}
                    >
                        {structure ? (
                            <Suspense fallback={<p>读取结构…</p>}>
                                <ImportPreview
                                    result={structure}
                                    activeSystem={slxSystem}
                                    selectedSid={slxSelected}
                                    onSystem={(index) => {
                                        setSlxSystem(index);
                                        setSlxSelected(null);
                                    }}
                                    onSelect={setSlxSelected}
                                    onBack={() => {
                                        setPreview(null);
                                        setSlxView(false);
                                        setNotice(null);
                                    }}
                                />
                            </Suspense>
                        ) : (
                            <SimulationCanvas
                                nodes={flowNodes}
                                edges={edges}
                                fitView={!doc.editor.viewport}
                                {...(doc.editor.viewport
                                    ? { defaultViewport: doc.editor.viewport }
                                    : {})}
                                onNodesChange={onNodesChange}
                                onEdgesChange={onEdgesChange}
                                onMoveEnd={onMoveEnd}
                                onConnect={connect}
                                isValidConnection={validConnection}
                                onNodeDoubleClick={onNodeDoubleClick}
                                onNodeContextMenu={onNodeContextMenu}
                                onEdgeContextMenu={onEdgeContextMenu}
                                onPaneContextMenu={onPaneContextMenu}
                                onPaneClick={onPaneClick}
                                nodesDraggable={editable}
                                nodesConnectable={editable}
                                colorMode={
                                    theme === "modern-dark" ? "dark" : "light"
                                }
                            />
                        )}
                    </div>
                    <ResizeBar
                        direction="horizontal"
                        name="结果面板高度"
                        value={bottom}
                        min={120}
                        max={480}
                        onChange={setBottom}
                        reverse
                    />
                    <section className="sim-results">
                        <div className="sim-results-tabs">
                            <button
                                className={pane === "scope" ? "active" : ""}
                                onClick={() => setPane("scope")}
                            >
                                Scope
                            </button>
                            <button
                                className={
                                    pane === "diagnostics" ? "active" : ""
                                }
                                onClick={() => setPane("diagnostics")}
                            >
                                诊断{" "}
                                {run.diagnostics.length > 0
                                    ? `(${run.diagnostics.length})`
                                    : ""}
                            </button>
                            <button
                                className={pane === "code" ? "active" : ""}
                                onClick={() => setPane("code")}
                            >
                                m 函数
                            </button>
                            <span className="sim-toolbar-spacer" />
                            {run.solverStats && (
                                <span
                                    title={`误差检验失败 ${run.solverStats.errorTestFailures}；非线性迭代 ${run.solverStats.nonlinearIterations}；采样重启 ${run.solverStats.reinitializations}`}
                                >
                                    CVODE · {run.solverStats.acceptedSteps} 步 ·{" "}
                                    {run.solverStats.rhsEvaluations} 次 RHS
                                </span>
                            )}
                            {run.info && numerical !== run.source.current && (
                                <span className="sim-stale">
                                    模型已修改 · 曲线来自上次运行
                                </span>
                            )}
                            <button
                                disabled={!run.frames.current.length}
                                onClick={() => void exportCsv()}
                            >
                                导出 CSV
                            </button>
                        </div>
                        {pane === "code" ? (
                            <FunctionCodePanel
                                path={codePath}
                                shared={props.sourceWorkspace}
                                theme={theme}
                                reveal={codeReveal}
                                onRun={runModel}
                                onSave={() => void save()}
                                embedded={
                                    codePath
                                        ? doc.sources?.[codePath]
                                        : undefined
                                }
                            />
                        ) : pane === "scope" ? (
                            <ScopePanel
                                frames={run.frames.current}
                                scopes={run.info?.scopes ?? []}
                                sampling={run.info?.sampling}
                                version={run.version}
                                names={doc.editor.labels}
                                dark={theme === "modern-dark"}
                            />
                        ) : (
                            <div className="sim-diagnostics" aria-live="polite">
                                {!run.diagnostics.length && (
                                    <p>
                                        检查模型或运行后，诊断信息会显示在这里。
                                    </p>
                                )}
                                {run.diagnostics.map((issue, index) => (
                                    <button
                                        key={index}
                                        className={
                                            issue.code === "valid"
                                                ? "valid"
                                                : ""
                                        }
                                        onClick={() => {
                                            if (!issue.block) return;
                                            focusBlock(issue.block);
                                            const block = doc.model.blocks.find(
                                                (block) =>
                                                    block.id === issue.block,
                                            );
                                            if (block && issue.sourcePath)
                                                void openFunction(block, issue);
                                        }}
                                    >
                                        <strong>
                                            {issue.code === "valid" ? "✓" : "!"}{" "}
                                            {issue.block
                                                ? `${issue.block}${issue.port ? `.${issue.port}` : ""}`
                                                : issue.code}
                                        </strong>
                                        <span>
                                            {issue.sourcePath
                                                ? `${issue.sourcePath}:${issue.line ?? 1}:${issue.column ?? 1} · `
                                                : ""}
                                            {issue.message}
                                        </span>
                                    </button>
                                ))}
                            </div>
                        )}
                    </section>
                </main>
                <ResizeBar
                    direction="vertical"
                    name="检查器宽度"
                    value={right}
                    min={210}
                    max={430}
                    onChange={setRight}
                    reverse
                />
                <aside className="sim-inspector">
                    <div className="sim-pane-title">检查器</div>
                    {structure ? (
                        <>
                            <h3>{originalBlock?.name ?? "SLX 结构检查"}</h3>
                            <p className="sim-help">
                                {structure.runnable
                                    ? "模型已通过兼容性检查。选择方块查看原始参数，双击子系统进入内部。"
                                    : "当前模型尚未通过兼容性检查。可补充左侧参数，再重新应用检查。"}
                            </p>
                            {originalBlock && (
                                <>
                                    <p className="sim-help">
                                        {originalBlock.blockType} · SID{" "}
                                        {originalBlock.sid}
                                        <br />
                                        {originalBlock.source.part}
                                    </p>
                                    <p className="sim-help">
                                        实际采样：
                                        {sampleTimeText(
                                            doc.slx?.runnable &&
                                                !doc.slx.snapshotEdited &&
                                                doc.slx.parameters ===
                                                    doc.slx.appliedParameters
                                                ? doc.slx.sampling?.blocks[
                                                      `slx_${originalBlock.sid.replaceAll(":", "_")}`
                                                  ]
                                                : undefined,
                                        )}
                                    </p>
                                    <dl className="sim-slx-original-parameters">
                                        {Object.entries(
                                            originalBlock.properties,
                                        ).map(([name, value]) => (
                                            <div key={name}>
                                                <dt>{name}</dt>
                                                <dd>{value}</dd>
                                            </div>
                                        ))}
                                    </dl>
                                </>
                            )}
                            {structure.issues
                                .filter(
                                    (issue) =>
                                        !originalBlock ||
                                        !issue.block ||
                                        issue.block === originalBlock.sid,
                                )
                                .map((issue, index) => (
                                    <p key={index} className="sim-help">
                                        <strong>
                                            {issue.block
                                                ? `SID ${issue.block}`
                                                : issue.code}
                                        </strong>
                                        <br />
                                        {issue.message}
                                    </p>
                                ))}
                        </>
                    ) : chosen ? (
                        <>
                            <div className="sim-inspector-heading">
                                <BlockIcon
                                    type={
                                        componentFor(doc.model, chosen)?.icon ??
                                        chosen.kind.type
                                    }
                                    size={30}
                                />
                                <div>
                                    <strong>
                                        {componentFor(doc.model, chosen)
                                            ?.name ??
                                            definition(chosen.kind.type).label}
                                    </strong>
                                    <small>{chosen.id}</small>
                                </div>
                            </div>
                            <label>
                                名称
                                <CommitField
                                    name="方块名称"
                                    value={label(doc, chosen)}
                                    onCommit={(value) => {
                                        if (
                                            !value.trim() ||
                                            value.length > 256
                                        ) {
                                            invalidParameter.current = true;
                                            setError(
                                                "名称必须是 1–256 个字符。",
                                            );
                                            return false;
                                        }
                                        change((next) => {
                                            Object.defineProperty(
                                                next.editor.labels,
                                                chosen.id,
                                                {
                                                    value,
                                                    enumerable: true,
                                                    configurable: true,
                                                    writable: true,
                                                },
                                            );
                                        });
                                    }}
                                />
                            </label>
                            <SamplingInspector
                                key={`sampling-${chosen.id}`}
                                model={doc.model}
                                block={chosen}
                                resolved={
                                    (run.checkedSampling?.source === numerical
                                        ? run.checkedSampling.plan
                                        : run.info &&
                                            run.source.current === numerical
                                          ? run.info.sampling
                                          : doc.slx?.runnable &&
                                              !doc.slx.snapshotEdited &&
                                              doc.slx.parameters ===
                                                  doc.slx.appliedParameters
                                            ? doc.slx.sampling
                                            : undefined
                                    )?.blocks[chosen.id]
                                }
                                onError={setError}
                                onChange={(rate) =>
                                    change((next) => {
                                        next.model.sampleTimes ??= {};
                                        if (rate)
                                            Object.defineProperty(
                                                next.model.sampleTimes,
                                                chosen.id,
                                                {
                                                    value: rate,
                                                    enumerable: true,
                                                    writable: true,
                                                    configurable: true,
                                                },
                                            );
                                        else
                                            delete next.model.sampleTimes[
                                                chosen.id
                                            ];
                                    })
                                }
                                onKindChange={(value) =>
                                    change((next) => {
                                        next.model.blocks.find(
                                            (b) => b.id === chosen.id,
                                        )!.kind = value;
                                    })
                                }
                            />
                            {chosen.kind.type === "mFunction" && (
                                <FunctionInspector
                                    value={chosen.kind}
                                    onOpen={() => void openFunction(chosen)}
                                    onError={setError}
                                    onChange={(value) =>
                                        change((next) => {
                                            next.model.blocks.find(
                                                (block) =>
                                                    block.id === chosen.id,
                                            )!.kind = value;
                                            next.model.connections =
                                                next.model.connections.filter(
                                                    (edge) =>
                                                        edge.to.block !==
                                                            chosen.id ||
                                                        value.inputs.some(
                                                            (input) =>
                                                                input.name ===
                                                                edge.to.port,
                                                        ),
                                                );
                                            const kept = new Set(
                                                next.model.connections.map(
                                                    edgeId,
                                                ),
                                            );
                                            for (const id of Object.keys(
                                                next.editor.bends,
                                            ))
                                                if (!kept.has(id))
                                                    delete next.editor.bends[
                                                        id
                                                    ];
                                        })
                                    }
                                />
                            )}
                            {chosen.kind.type === "step" && (
                                <>
                                    <label>
                                        阶跃时刻
                                        <CommitField
                                            name="Step 时间"
                                            value={String(chosen.kind.time)}
                                            onCommit={(text) => {
                                                const time = Number(text);
                                                if (
                                                    !text.trim() ||
                                                    !Number.isFinite(time)
                                                ) {
                                                    setError(
                                                        "阶跃时刻必须是有限实数。",
                                                    );
                                                    return false;
                                                }
                                                change((next) => {
                                                    const b =
                                                        next.model.blocks.find(
                                                            (b) =>
                                                                b.id ===
                                                                chosen.id,
                                                        )!;
                                                    if (b.kind.type === "step")
                                                        b.kind.time = time;
                                                });
                                            }}
                                        />
                                    </label>
                                    {(["before", "after"] as const).map(
                                        (key) => (
                                            <label key={key}>
                                                {key === "before"
                                                    ? "阶跃前"
                                                    : "阶跃后"}
                                                <CommitField
                                                    name={`Step ${key}`}
                                                    value={
                                                        chosen.kind.type ===
                                                        "step"
                                                            ? chosen.kind[
                                                                  key
                                                              ].join(" ")
                                                            : ""
                                                    }
                                                    onCommit={(text) => {
                                                        try {
                                                            const values =
                                                                numericLiteral(
                                                                    text,
                                                                );
                                                            if (
                                                                values.length >
                                                                4096
                                                            )
                                                                throw new Error(
                                                                    "Step 向量上限为 4096。",
                                                                );
                                                            const next =
                                                                    structuredClone(
                                                                        docRef.current,
                                                                    ),
                                                                b =
                                                                    next.model.blocks.find(
                                                                        (b) =>
                                                                            b.id ===
                                                                            chosen.id,
                                                                    )!;
                                                            if (
                                                                b.kind.type !==
                                                                "step"
                                                            )
                                                                return false;
                                                            const other =
                                                                key === "before"
                                                                    ? "after"
                                                                    : "before";
                                                            const width =
                                                                Math.max(
                                                                    values.length,
                                                                    b.kind[
                                                                        other
                                                                    ].length,
                                                                );
                                                            if (
                                                                (values.length !==
                                                                    1 &&
                                                                    values.length !==
                                                                        width) ||
                                                                (b.kind[other]
                                                                    .length !==
                                                                    1 &&
                                                                    b.kind[
                                                                        other
                                                                    ].length !==
                                                                        width)
                                                            )
                                                                throw new Error(
                                                                    "阶跃前后值需要相同宽度，或使用标量展开。",
                                                                );
                                                            b.kind[key] =
                                                                values.length ===
                                                                1
                                                                    ? (Array(
                                                                          width,
                                                                      ).fill(
                                                                          values[0],
                                                                      ) as number[])
                                                                    : values;
                                                            if (
                                                                b.kind[other]
                                                                    .length ===
                                                                1
                                                            )
                                                                b.kind[other] =
                                                                    Array(
                                                                        width,
                                                                    ).fill(
                                                                        b.kind[
                                                                            other
                                                                        ][0],
                                                                    ) as number[];
                                                            edit(next);
                                                        } catch (error) {
                                                            report(error);
                                                            return false;
                                                        }
                                                    }}
                                                />
                                            </label>
                                        ),
                                    )}
                                </>
                            )}
                            {chosen.kind.type === "component" &&
                                componentFor(doc.model, chosen) && (
                                    <ComponentInspector
                                        key={chosen.id}
                                        value={chosen.kind}
                                        definition={componentFor(
                                            doc.model,
                                            chosen,
                                        )!}
                                        disabled={!editable}
                                        onChange={(kind) =>
                                            change((next) => {
                                                next.model.blocks.find(
                                                    (b) => b.id === chosen.id,
                                                )!.kind = kind;
                                            })
                                        }
                                        onOpen={(callback) =>
                                            void openFunction(chosen, {
                                                code: "source",
                                                message: "",
                                                sourcePath: callback.source,
                                            })
                                        }
                                        onEdit={() =>
                                            setComponentDraft({
                                                definition: componentFor(
                                                    doc.model,
                                                    chosen,
                                                )!,
                                                isNew: false,
                                            })
                                        }
                                        onLibrary={() =>
                                            void saveComponentLibrary(chosen)
                                        }
                                        embedded={blockSources(
                                            doc.model,
                                            chosen,
                                        ).some((path) =>
                                            Object.hasOwn(
                                                doc.sources ?? {},
                                                path,
                                            ),
                                        )}
                                        onError={(message) => {
                                            invalidParameter.current = true;
                                            setError(message);
                                        }}
                                    />
                                )}
                            {chosen.kind.type !== "zeroOrderHold" &&
                                chosen.kind.type !== "scope" &&
                                chosen.kind.type !== "mFunction" &&
                                chosen.kind.type !== "step" &&
                                chosen.kind.type !== "component" && (
                                    <label>
                                        {chosen.kind.type === "sum"
                                            ? "输入符号"
                                            : chosen.kind.type === "gain"
                                              ? "增益"
                                              : chosen.kind.type === "constant"
                                                ? "常量值"
                                                : "初始状态"}
                                        <CommitField
                                            key={chosen.id}
                                            name="方块参数"
                                            value={parameterText(chosen)}
                                            onCommit={(value) =>
                                                applyParameter(chosen, value)
                                            }
                                        />
                                    </label>
                                )}
                            <p className="sim-help">
                                {chosen.kind.type === "sum"
                                    ? "每个 + 或 - 对应一个输入端口。"
                                    : chosen.kind.type === "gain"
                                      ? "当前 Gain 为逐元素增益。支持标量或与输入等宽的向量。"
                                      : chosen.kind.type === "unitDelay"
                                        ? "一个自身采样周期的延迟；可继承或单独指定周期。"
                                        : chosen.kind.type === "integrator"
                                          ? "输入为状态导数，输出为当前连续状态。"
                                          : chosen.kind.type === "scope"
                                            ? "观察连接信号。运行后在底部查看曲线。"
                                            : "支持有限实数和固定宽度的数值向量。"}
                            </p>
                            <h3>端口</h3>
                            {ports(chosen, doc.model.components).inputs.map(
                                (port) => (
                                    <div className="sim-port-row" key={port}>
                                        <span>→ {port}</span>
                                        <small>
                                            {doc.model.connections.some(
                                                (edge) =>
                                                    edge.to.block ===
                                                        chosen.id &&
                                                    edge.to.port === port,
                                            )
                                                ? "已连接"
                                                : "未连接"}
                                        </small>
                                    </div>
                                ),
                            )}
                            {ports(chosen, doc.model.components).outputs.map(
                                (port) => (
                                    <div className="sim-port-row" key={port}>
                                        <span>{port} →</span>
                                        <small>
                                            {
                                                doc.model.connections.filter(
                                                    (edge) =>
                                                        edge.from.block ===
                                                            chosen.id &&
                                                        edge.from.port === port,
                                                ).length
                                            }{" "}
                                            条连接
                                        </small>
                                    </div>
                                ),
                            )}
                            <button
                                className="sim-delete"
                                onClick={() => deleteSelected([chosen.id], [])}
                            >
                                删除方块
                            </button>
                        </>
                    ) : chosenEdge ? (
                        <>
                            <h3>信号连接</h3>
                            <p>
                                {chosenEdge.from.block}.{chosenEdge.from.port}
                            </p>
                            <p>↓</p>
                            <p>
                                {chosenEdge.to.block}.{chosenEdge.to.port}
                            </p>
                            <p className="sim-help">
                                实数 double
                                标量或固定宽度向量。最终尺寸由原生编译器检查。
                            </p>
                            <button
                                onClick={() =>
                                    deleteSelected([], [edgeId(chosenEdge)])
                                }
                            >
                                删除连线
                            </button>
                        </>
                    ) : (
                        <>
                            <h3>
                                {selected.length > 1
                                    ? `已选择 ${selected.length} 个方块`
                                    : "模型设置"}
                            </h3>
                            <label>
                                模型名称
                                <CommitField
                                    name="模型名称"
                                    value={doc.model.name}
                                    onCommit={(value) => {
                                        if (value.trim())
                                            change((next) => {
                                                next.model.name = value.slice(
                                                    0,
                                                    256,
                                                );
                                            });
                                    }}
                                />
                            </label>
                            {doc.model.schemaVersion < 5 && (
                                <button
                                    onClick={() =>
                                        change((next) => {
                                            next.model.schemaVersion = 5;
                                        })
                                    }
                                >
                                    启用多速率采样
                                </button>
                            )}
                            <SolverInspector
                                value={doc.execution}
                                capabilities={run.capabilities}
                                onError={setError}
                                onChange={(value) =>
                                    change((next) => {
                                        next.execution = value;
                                    })
                                }
                            />
                            {(
                                [
                                    ["startTime", "起始时间"],
                                    ["stopTime", "停止时间"],
                                    ["maxStep", "最大步长"],
                                    ["sampleTime", "离散采样周期"],
                                ] as const
                            ).map(([key, title]) => (
                                <label key={key}>
                                    {title}
                                    <CommitField
                                        name={title}
                                        value={String(
                                            doc.model.settings[key] ?? 0.1,
                                        )}
                                        onCommit={(value) => {
                                            const number = Number(value);
                                            if (
                                                !value.trim() ||
                                                !Number.isFinite(number)
                                            ) {
                                                invalidParameter.current = true;
                                                setError(
                                                    `${title}必须是有限实数。`,
                                                );
                                                return false;
                                            }
                                            change((next) => {
                                                next.model.settings[key] =
                                                    number;
                                            });
                                        }}
                                    />
                                </label>
                            ))}
                            <p className="sim-help">
                                求解器运行在 Rust
                                服务中。画布坐标和界面刷新不会改变仿真时间。
                            </p>
                        </>
                    )}
                    <div className="sim-inspector-footer">
                        OpenMat Simulation
                        <br />
                        {preview ? "SLX 结构检查" : "选择方块查看参数"}
                    </div>
                </aside>
            </div>
            <footer className="sim-statusbar">
                <span className={run.status === "failed" ? "error-text" : ""}>
                    {STATUS[run.status]}
                </span>
                <span>
                    t = {run.frames.current.at(-1)?.time.toPrecision(6) ?? "—"}
                </span>
                <span>
                    {run.info
                        ? `${run.info.backend} · ${run.info.solver?.type ?? "rk4"} · ${run.frames.current.length} samples`
                        : "原生仿真 · 固定步长 RK4"}
                </span>
                <span className="sim-toolbar-spacer" />
                {run.elapsed > 0 && <span>{run.elapsed.toFixed(3)} s</span>}
                <span>{dirty ? "未保存" : "无未保存修改"}</span>
            </footer>
            {menu && editable && (
                <ContextMenu
                    x={menu.x}
                    y={menu.y}
                    title={
                        menu.node
                            ? "方块操作"
                            : menu.edge
                              ? "连线操作"
                              : "画布操作"
                    }
                    groups={menuGroups}
                    onClose={(restoreFocus) => {
                        setMenu(null);
                        if (restoreFocus) shell.current?.focus();
                    }}
                />
            )}
            {componentDraft && (
                <ComponentDialog
                    value={componentDraft.definition}
                    allowInherited={doc.model.schemaVersion === 5}
                    onApply={applyComponent}
                    onClose={() => setComponentDraft(null)}
                />
            )}
            {modal && (
                <div className="sim-modal-backdrop">
                    <form
                        className="sim-modal"
                        role="dialog"
                        aria-modal="true"
                        aria-label={modal === "open" ? "打开模型" : "保存模型"}
                        onSubmit={(event) => {
                            event.preventDefault();
                            if (modal === "open") void load(pathInput);
                            else void saveToWorkspace(pathInput);
                        }}
                    >
                        <h2>{modal === "open" ? "打开模型" : "保存模型"}</h2>
                        <p className="sim-help">当前目录：{rootPath}</p>
                        <label>
                            相对路径
                            <input
                                autoFocus
                                aria-label="模型文件路径"
                                value={pathInput}
                                onChange={(event) =>
                                    setPathInput(event.target.value)
                                }
                                placeholder="models/example.omsim"
                            />
                        </label>
                        {modal === "open" && (
                            <div className="sim-file-options">
                                {fileOptions.map((path) => (
                                    <button
                                        type="button"
                                        key={path}
                                        onClick={() => setPathInput(path)}
                                        onDoubleClick={() => void load(path)}
                                    >
                                        {path}
                                    </button>
                                ))}
                                {!fileOptions.length && (
                                    <p>
                                        输入模型的相对路径，或使用工具栏导入本地文件。
                                    </p>
                                )}
                            </div>
                        )}
                        {modal === "save" && (
                            <p className="sim-help">
                                另存为需要一个新文件名，已有文件通过打开后保存进行更新。
                            </p>
                        )}
                        {error && (
                            <p className="error-text" role="alert">
                                {error}
                            </p>
                        )}
                        <div className="sim-modal-actions">
                            <button
                                type="button"
                                disabled={fileBusy}
                                onClick={() => {
                                    setModal(null);
                                    afterSave.current = null;
                                    cancelSwitch.current?.();
                                }}
                            >
                                取消
                            </button>
                            <button
                                className="sim-run"
                                type="submit"
                                disabled={fileBusy || !pathInput.trim()}
                            >
                                {fileBusy
                                    ? "处理中…"
                                    : modal === "open"
                                      ? "打开"
                                      : "保存"}
                            </button>
                        </div>
                    </form>
                </div>
            )}
            {pendingAction && (
                <div className="sim-modal-backdrop">
                    <div
                        className="sim-modal"
                        role="dialog"
                        aria-modal="true"
                        aria-label="未保存的模型"
                    >
                        <h2>模型有未保存修改</h2>
                        <p>{pendingAction.title}之前，如何处理当前修改？</p>
                        <div className="sim-modal-actions">
                            <button
                                disabled={fileBusy}
                                onClick={() => {
                                    pendingAction.cancel?.();
                                    setPendingAction(null);
                                }}
                            >
                                取消
                            </button>
                            <button
                                disabled={fileBusy}
                                onClick={() => {
                                    const action = pendingAction.action;
                                    setPendingAction(null);
                                    try {
                                        replace(
                                            parseDocument(
                                                draftRef.current.saved,
                                            ),
                                            draftRef.current.file,
                                        );
                                    } catch {
                                        replace(example("blank"));
                                    }
                                    action();
                                }}
                            >
                                放弃修改
                            </button>
                            <button
                                disabled={fileBusy}
                                className="sim-run"
                                onClick={() => {
                                    if (!platform.files && !file) {
                                        afterSave.current =
                                            pendingAction.action;
                                        setPendingAction(null);
                                        void save();
                                    } else
                                        void save().then((ok) => {
                                            if (
                                                ok &&
                                                draftRef.current.content ===
                                                    draftRef.current.saved
                                            ) {
                                                pendingAction.action();
                                                setPendingAction(null);
                                            }
                                        });
                                }}
                            >
                                保存
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
export default function ModelEditor(props: Props) {
    return (
        <ReactFlowProvider>
            <Editor {...props} />
        </ReactFlowProvider>
    );
}
