import {
    createBootstrapInitializeRequest,
    createKernelRequest,
    KERNEL_PROTOCOL_V3,
    MAX_AGGREGATE_DEPTH,
    MAX_AGGREGATE_ELEMENTS,
    MAX_AGGREGATE_NODES,
    MAX_PREVIEW_CODE_UNITS,
    MAX_STRING_ELEMENT_CODE_UNITS,
    SUPPORTED_KERNEL_PROTOCOLS,
} from "../protocol/kernel-v2";
import type { DisplayEventData } from "../protocol/kernel-v0";
import { FIGURE_MIME_TYPE } from "../plot/graphics-v1";
import { WebSocketKernelTransport } from "../transport/websocket-kernel-transport";
import type { KernelTransport } from "../transport/kernel-transport";
import {
    BUILTIN_CATALOG,
    isUiValue,
    type ComponentSpec,
    type PropertySpec,
    type MethodSpec,
    type UiValue,
} from "./catalog";
import {
    DEFAULT_LAYOUT,
    QUALIFIED_NAME,
    findNode,
    validateDocument,
    walk,
    type UiDocument,
    type UiNode,
} from "./model";
import type { UiInteraction } from "./UiRenderer";
import { layoutAssignments } from "./layout";
import { validateLayout } from "./layout-schema";

export const UI_MIME = "application/vnd.openmat.ui+json";
export const matlabString = (value: string): string =>
    /[\r\n]/.test(value)
        ? `char([${Array.from({ length: value.length }, (_, i) => value.charCodeAt(i)).join(" ")}])`
        : `'${value.replaceAll("'", "''")}'`;
// Char arrays are emitted as code units for multiline text; all executable names
// go through identifier validation, never through string escaping alone.
export function matlabValue(value: UiValue): string {
    if (typeof value === "string") return matlabString(value);
    if (typeof value === "boolean") return value ? "true" : "false";
    if (typeof value === "number") {
        if (!Number.isFinite(value)) throw new Error("UI 数值必须为有限数。");
        return String(value);
    }
    if (value.every((v) => typeof v === "string"))
        return `{${value.map((v) => matlabValue(v as string)).join(",")}}`;
    return `{${(value as (string | number | boolean)[][]).map((row) => row.map(matlabValue).join(",")).join(";")}}`;
}
export function matlabPropertyValue(
    property: PropertySpec,
    value: UiValue,
): string {
    if (property.valueClass === "double" && Array.isArray(value)) {
        if (
            !value.every(
                (row) =>
                    Array.isArray(row) &&
                    row.every(
                        (v) => typeof v === "number" && Number.isFinite(v),
                    ),
            )
        )
            throw new Error(`${property.name} 需要二维数值数组。`);
        return `[${(value as number[][]).map((row) => row.join(" ")).join(";")}]`;
    }
    if (property.valueClass === "string" && typeof value === "string")
        return `string(${matlabValue(value)})`;
    return matlabValue(value);
}
export function buildUiProgram(
    document: UiDocument,
    catalog: Readonly<Record<string, ComponentSpec>>,
): string {
    validateDocument(document);
    if (document.kind === "component")
        return [
            `app = ${document.appClass}();`,
            `app.Id = ${matlabString(document.root.id)};`,
            `app.Name = ${matlabString(document.root.name)};`,
            "app.initialize();",
            "openmat_ui('snapshot', app);",
        ].join("\n");
    if (document.version !== 1) return buildClassUiProgram(document, catalog);
    const lines = ["app = struct();"];
    for (const node of walk(document.root)) {
        const descriptor = catalog[node.type];
        if (!descriptor) throw new Error(`请先注册组件类 ${node.type}。`);
        const className = descriptor.className ?? "openmat.ui.Control";
        if (!QUALIFIED_NAME.test(className))
            throw new Error("组件类名称无效。");
        const target = `app.${node.name}`;
        lines.push(
            `${target} = ${className}();`,
            `${target}.Id = ${matlabString(node.id)};`,
            `${target}.Name = ${matlabString(node.name)};`,
            ...(!descriptor.className
                ? [`${target}.Type = ${matlabString(node.type)};`]
                : []),
        );
        for (const p of descriptor.properties)
            if (!p.readonly)
                lines.push(
                    `${target}.${p.name} = ${matlabPropertyValue(p, node.properties[p.name] ?? p.default)};`,
                );
        // setup runs only after instance overrides are applied; update then sees the
        // final initial values. This initialization never executes in the IDE session.
        lines.push(
            ...layoutAssignments(target, node.layout, !descriptor.composite),
        );
    }
    for (const node of walk(document.root))
        for (const child of node.children)
            lines.push(`app.${node.name}.add(app.${child.name});`);
    const startup = document.root.events.Startup ?? document.controller;
    lines.push(`app.${document.root.name}.initialize();`);
    if (startup)
        lines.push(
            `${startup}(app, struct('Source', ${matlabString(document.root.name)}, 'EventName', 'Startup'));`,
        );
    lines.push(
        ...refreshLines(document, catalog),
        `openmat_ui('snapshot', app.${document.root.name});`,
    );
    return lines.join("\n");
}

function buildClassUiProgram(
    document: UiDocument,
    catalog: Readonly<Record<string, ComponentSpec>>,
): string {
    const nodes = walk(document.root);
    const lines = [`app = ${document.appClass}();`, "components = struct();"];
    for (const node of nodes) {
        const spec =
            node.id === document.root.id
                ? (catalog[document.appClass!] ?? catalog.Window)
                : catalog[node.type];
        if (!spec) throw new Error(`请先注册组件类 ${node.type}。`);
        const target = `components.${node.name}`;
        const className = spec.className ?? `openmat.ui.${node.type}`;
        if (!QUALIFIED_NAME.test(className))
            throw new Error("组件类名称无效。");
        lines.push(
            `${target} = ${node.id === document.root.id ? "app" : `${className}()`};`,
            `${target}.Id = ${matlabString(node.id)};`,
            `${target}.Name = ${matlabString(node.name)};`,
        );
        // A subclass retains its base rendering type. A class name is not a
        // presentation primitive, and must never overwrite the native Type.
        for (const p of spec.properties)
            if (!p.readonly) {
                const value = node.properties[p.name];
                if (value !== undefined || BUILTIN_CATALOG[node.type])
                    lines.push(
                        `${target}.${p.name} = ${matlabPropertyValue(p, value ?? p.default)};`,
                    );
            }
    }
    for (const node of nodes) {
        const spec = catalog[node.type];
        lines.push(
            ...layoutAssignments(
                `components.${node.name}`,
                node.layout,
                !spec?.composite,
            ),
        );
    }
    for (const node of nodes)
        for (const child of node.children)
            lines.push(
                `components.${node.name}.add(components.${child.name});`,
            );
    lines.push("app.bindDesignerComponents(components);");
    lines.push(
        "app.initialize();",
        "notify(app, 'Startup');",
        ...refreshLines(document, catalog),
        "openmat_ui('snapshot', app);",
    );
    return lines.join("\n");
}
function refreshLines(
    document: UiDocument,
    catalog: Readonly<Record<string, ComponentSpec>>,
): string[] {
    void catalog;
    return [`app.${document.root.name}.refresh();`];
}
export function buildEventProgram(
    document: UiDocument,
    event: UiInteraction,
    catalog: Readonly<Record<string, ComponentSpec>>,
    runtimeRoot?: UiNode,
): string {
    const declared = findNode(document.root, event.id);
    const live = runtimeRoot ? findNode(runtimeRoot, event.id) : undefined;
    const node = live ?? declared;
    if (
        (!live && runtimeRoot) ||
        (event.runtimeId && live?.runtimeId !== event.runtimeId)
    )
        return "";
    if (!node) throw new Error("运行中的组件已被删除。");
    if (
        !declared &&
        (!node.runtimePath ||
            node.runtimePath.some(
                (index) =>
                    !Number.isInteger(index) || index < 0 || index > 1000,
            ))
    )
        throw new Error("运行时组件路径无效。");
    const key = event.runtimeId ?? node?.runtimeId;
    const target = key
        ? "ui_target"
        : declared
          ? `app.${node.name}`
          : `app.${document.root.name}${node.runtimePath!.map((index) => `.Children{${index + 1}}`).join("")}`;
    const lines: string[] = [];
    if (event.property && event.value !== undefined) {
        const descriptor = catalog[node.type]?.properties.find(
            (p) => p.name === event.property && !p.readonly,
        );
        if (!descriptor) throw new Error("该属性不能由 UI 修改。");
        lines.push(
            `${target}.${descriptor.name} = ${matlabPropertyValue(descriptor, event.value)};`,
        );
    }
    const handler =
        document.version === 1 && declared
            ? (node.events[event.event] ?? document.controller)
            : "";
    const eventFields = [
        `'Source', ${matlabString(node.name)}`,
        `'EventName', ${matlabString(event.event)}`,
        `'Value', ${event.value === undefined ? "[]" : matlabValue(event.value)}`,
    ];
    if (
        event.row !== undefined &&
        Number.isInteger(event.row) &&
        Number.isInteger(event.column)
    )
        eventFields.push(`'Row', ${event.row}`, `'Column', ${event.column}`);
    const callbackProperty = (
        {
            Clicked: "ButtonPushedFcn",
            ValueChanged: "ValueChangedFcn",
            CellEdited: "CellEditCallback",
            SelectionChanged: "SelectionChangedFcn",
        } as Record<string, string>
    )[event.event];
    if (
        BUILTIN_CATALOG[catalog[node.type]?.renderType ?? node.type] &&
        node.type !== "ComponentContainer" &&
        callbackProperty
    )
        lines.push(
            `ui_callback = ${target}.${callbackProperty};`,
            `if isa(ui_callback, 'function_handle')\nfeval(ui_callback, ${target}, struct(${eventFields.join(", ")}));\nend`,
        );
    if (document.version !== 1) {
        // Browser events only enter the declared primitive signals. Custom
        // signals originate in M through notify and are handled by listeners.
        const renderType = catalog[node.type]?.renderType ?? node.type;
        if (!BUILTIN_CATALOG[renderType]?.events.includes(event.event))
            throw new Error("浏览器事件不属于该控件的公开输入信号。");
        const runtimeClass = runtimeRoot
            ? findNode(runtimeRoot, node.id)?.runtimeClass
            : node.runtimeClass;
        if (runtimeClass !== "openmat.ui.Control")
            lines.push(
                `notify(${target}, ${matlabString(event.event)}, struct(${eventFields.join(", ")}));`,
            );
    }
    if (handler) {
        if (!QUALIFIED_NAME.test(handler)) throw new Error("回调名称无效。");
        lines.push(`${handler}(app, struct(${eventFields.join(", ")}));`);
    }
    return [
        ...(key
            ? [
                  `ui_target = app.${document.root.name}.findRuntime(${matlabString(key)});`,
                  "if ~isempty(ui_target)",
              ]
            : []),
        ...lines,
        ...(key ? ["end"] : []),
        ...refreshLines(document, catalog),
        `openmat_ui('snapshot', app.${document.root.name});`,
    ].join("\n");
}
interface RuntimeComponent {
    className: string;
    properties: Record<string, UiValue>;
    schema: {
        name: string;
        value: UiValue | null;
        writable: boolean;
        className: string;
    }[];
    children: RuntimeComponent[];
    events?: { name: string; declaringClass: string }[];
    methods?: MethodSpec[];
    layout?: Partial<import("./model").UiLayout>;
}
export interface UiPayload {
    version: 1;
    kind: "snapshot" | "describe";
    component: RuntimeComponent;
}
function record(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
export function parseUiPayload(encoded: string): UiPayload {
    if (encoded.length > 750_000) throw new Error("UI 消息超过限制。");
    const value: unknown = JSON.parse(encoded);
    if (
        !record(value) ||
        value.version !== 1 ||
        !["snapshot", "describe"].includes(String(value.kind))
    )
        throw new Error("UI 协议版本无效。");
    let count = 0;
    const check = (c: unknown, depth: number): void => {
        if (
            ++count > 1000 ||
            depth > 32 ||
            !record(c) ||
            typeof c.className !== "string" ||
            !record(c.properties) ||
            !Array.isArray(c.children) ||
            !Array.isArray(c.schema)
        )
            throw new Error("UI 组件快照无效。");
        if (c.layout !== undefined) {
            if (!record(c.layout)) throw new Error("UI 布局快照无效。");
            try {
                validateLayout(c.layout, DEFAULT_LAYOUT);
            } catch {
                throw new Error("UI 布局快照无效。");
            }
        }
        if (
            c.events !== undefined &&
            (!Array.isArray(c.events) ||
                c.events.some(
                    (e) =>
                        !record(e) ||
                        typeof e.name !== "string" ||
                        !/^[A-Za-z]\w*$/.test(e.name) ||
                        typeof e.declaringClass !== "string" ||
                        !QUALIFIED_NAME.test(e.declaringClass),
                ))
        )
            throw new Error("UI 事件元数据无效。");
        if (
            c.methods !== undefined &&
            (!Array.isArray(c.methods) ||
                c.methods.some(
                    (m) =>
                        !record(m) ||
                        typeof m.name !== "string" ||
                        !/^[A-Za-z]\w*$/.test(m.name) ||
                        !["public", "protected", "private"].includes(
                            String(m.access),
                        ) ||
                        typeof m.declaringClass !== "string" ||
                        !QUALIFIED_NAME.test(m.declaringClass),
                ))
        )
            throw new Error("UI 方法元数据无效。");
        for (const [key, p] of Object.entries(c.properties)) {
            if (
                !/^[A-Za-z]\w*$/.test(key) ||
                key === "constructor" ||
                key === "prototype"
            )
                throw new Error("UI 属性名称无效。");
            if (!isUiValue(p)) throw new Error("UI 属性类型无效。");
        }
        for (const field of c.schema)
            if (
                !record(field) ||
                typeof field.name !== "string" ||
                !/^[A-Za-z]\w*$/.test(field.name) ||
                ["constructor", "prototype"].includes(field.name) ||
                typeof field.writable !== "boolean" ||
                typeof field.className !== "string" ||
                (field.value !== null && !isUiValue(field.value))
            )
                throw new Error("UI 属性元数据无效。");
        c.children.forEach((child) => check(child, depth + 1));
    };
    check(value.component, 0);
    return value as unknown as UiPayload;
}
export function applySnapshot(
    document: UiDocument,
    component: RuntimeComponent,
): UiDocument {
    const convert = (
        c: RuntimeComponent,
        path: string,
        indices: number[],
    ): UiNode => {
        const id = String(
            c.properties.Id || `runtime-${c.properties.RuntimeId || path}`,
        );
        const original = findNode(document.root, id);
        const type = String(c.properties.Type || "ComponentContainer");
        return {
            ...(original ?? {
                id,
                name: String(
                    c.properties.Name || `Internal${path.replaceAll("-", "_")}`,
                ),
                type,
                layout: {
                    ...DEFAULT_LAYOUT,
                    height: BUILTIN_CATALOG[type]?.container ? 140 : 36,
                },
                events: {},
            }),
            runtimePath: indices,
            runtimeClass: c.className,
            ...(typeof c.properties.RuntimeId === "string"
                ? { runtimeId: c.properties.RuntimeId }
                : {}),
            layout: {
                ...DEFAULT_LAYOUT,
                height: BUILTIN_CATALOG[type]?.container ? 140 : 36,
                ...original?.layout,
                ...c.layout,
            },
            sourceOwned: !original,
            properties: { ...original?.properties, ...c.properties },
            children: c.children.map((child, index) =>
                convert(child, `${path}-${index}`, [...indices, index]),
            ),
        };
    };
    return { ...document, root: convert(component, "0", []) };
}
export function componentDescriptor(
    payload: UiPayload,
    className: string,
): ComponentSpec {
    if (payload.kind !== "describe") throw new Error("未收到组件属性信息。");
    const reserved = new Set([
        "Id",
        "Name",
        "Type",
        "Children",
        "Parent",
        "RuntimeId",
        "Layout",
        "Position",
        "ButtonPushedFcn",
        "ValueChangedFcn",
        "SelectionChangedFcn",
        "CellEditCallback",
    ]);
    const renderType = String(
        payload.component.properties.Type || "ComponentContainer",
    );
    const base =
        BUILTIN_CATALOG[renderType] ?? BUILTIN_CATALOG.ComponentContainer!;
    return {
        type: className,
        className,
        label: className.split(".").at(-1)!,
        icon: base.icon,
        renderType: base.type,
        group: "自定义组件",
        container: base.container,
        composite:
            payload.component.methods?.some(
                (m) =>
                    m.name === "buildDesignerComponents" &&
                    !m.declaringClass.startsWith("matlab.ui."),
            ) ?? false,
        ...(payload.component.layout &&
        payload.component.methods?.some(
            (m) =>
                m.name === "buildDesignerComponents" &&
                !m.declaringClass.startsWith("matlab.ui."),
        )
            ? { defaultLayout: payload.component.layout }
            : {}),
        events: [
            ...new Set([
                ...base.events,
                ...(payload.component.events ?? []).map((e) => e.name),
            ]),
        ],
        methods: (payload.component.methods ?? []).filter(
            (m) =>
                !m.declaringClass.startsWith("openmat.ui.") &&
                !m.declaringClass.startsWith("matlab.ui.") &&
                ![
                    "setup",
                    "update",
                    "teardown",
                    "bindDesignerComponents",
                    "applyDesignerDefaults",
                    "buildDesignerComponents",
                ].includes(m.name),
        ),
        properties: payload.component.schema
            .filter((p) => !reserved.has(p.name) && p.value !== null)
            .map((p) => ({
                name: p.name,
                label: p.name,
                default: p.value!,
                readonly: !p.writable,
                valueClass: p.className,
                type:
                    typeof p.value === "boolean"
                        ? "boolean"
                        : typeof p.value === "number"
                          ? "number"
                          : Array.isArray(p.value)
                            ? p.className !== "double" &&
                              p.value.every((v) => typeof v === "string")
                                ? "items"
                                : "table"
                            : "text",
            })),
    };
}
export interface UiSessionCallbacks {
    onPayload: (payload: UiPayload) => void;
    onDisplay: (display: DisplayEventData) => void;
    onLog: (message: string) => void;
    onLost: (message: string) => void;
}
/** One WebSocket creates one native RuntimeEngine. Commands serialize in its
 * existing kernel queue; interrupts use its independent control path. */
export class UiRuntimeSession {
    private readonly id = `openmat-ui-${crypto.randomUUID()}`;
    private sequence = 0;
    private disposed = false;
    private unsubscribers: (() => void)[] = [];
    constructor(
        private readonly transport: KernelTransport,
        private readonly callbacks: UiSessionCallbacks,
    ) {}
    get sessionId(): string {
        return this.id;
    }
    static websocket(
        url: string,
        callbacks: UiSessionCallbacks,
    ): UiRuntimeSession {
        return new UiRuntimeSession(
            new WebSocketKernelTransport(url),
            callbacks,
        );
    }
    async connect(): Promise<void> {
        this.unsubscribers.push(
            this.transport.subscribe((message) => {
                if (this.disposed) return;
                const event = message.event;
                if (event.type === "display") {
                    const encoded = event.data.representations[UI_MIME];
                    if (encoded) {
                        try {
                            this.callbacks.onPayload(parseUiPayload(encoded));
                        } catch (error) {
                            this.callbacks.onLog(String(error));
                        }
                    } else if (event.data.representations[FIGURE_MIME_TYPE])
                        this.callbacks.onDisplay(event.data);
                    else if (event.data.representations["text/plain"])
                        this.callbacks.onLog(
                            event.data.representations["text/plain"],
                        );
                } else if (event.type === "stream")
                    this.callbacks.onLog(event.data.text);
                else if (event.type === "diagnostic")
                    this.callbacks.onLog(event.data.message);
            }),
            this.transport.subscribeConnectionLoss((error) => {
                if (!this.disposed) this.callbacks.onLost(error.message);
            }),
        );
        await this.transport.connect(this.id);
        const response = await this.transport.request(
            createBootstrapInitializeRequest(this.id, this.next(), {
                client: { name: "openmat-app-designer", version: "1" },
                supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
                capabilities: {
                    executionModes: ["repl", "cell"],
                    displayMimeTypes: [UI_MIME, FIGURE_MIME_TYPE, "text/plain"],
                    maxPreviewElements: 256,
                    maxStringElementCodeUnits: MAX_STRING_ELEMENT_CODE_UNITS,
                    maxPreviewCodeUnits: MAX_PREVIEW_CODE_UNITS,
                    maxAggregateNodes: MAX_AGGREGATE_NODES,
                    maxAggregateElements: MAX_AGGREGATE_ELEMENTS,
                    maxAggregateDepth: MAX_AGGREGATE_DEPTH,
                    interrupt: true,
                    workspaceDelta: true,
                },
            }),
        );
        if (!response.ok) throw new Error(response.error.message);
        if (response.result.data.negotiatedProtocol !== KERNEL_PROTOCOL_V3)
            throw new Error("设计器需要 kernel-v3。");
    }
    async execute(code: string, sourceName = "AppDesigner"): Promise<void> {
        if (this.disposed) throw new Error("预览会话已关闭。");
        const response = await this.transport.request(
            createKernelRequest(
                KERNEL_PROTOCOL_V3,
                this.id,
                this.next(),
                "execute",
                { code, sourceName, mode: "repl" },
            ),
        );
        if (!response.ok) throw new Error(response.error.message);
        if (response.result.data.interrupted) throw new Error("执行已中断。");
    }
    async interrupt(): Promise<void> {
        if (!this.disposed)
            await this.transport.request(
                createKernelRequest(
                    KERNEL_PROTOCOL_V3,
                    this.id,
                    this.next(),
                    "interrupt",
                    {},
                ),
            );
    }
    async dispose(): Promise<void> {
        if (this.disposed) return;
        this.disposed = true;
        this.unsubscribers.forEach((unsubscribe) => unsubscribe());
        this.unsubscribers = [];
        await this.transport.disconnect();
    }
    private next(): string {
        return `ui-${++this.sequence}`;
    }
}
