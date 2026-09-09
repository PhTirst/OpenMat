import {
    BUILTIN_CATALOG,
    isUiValue,
    type ComponentSpec,
    type UiValue,
} from "./catalog";
import { validateLayout, type UiSizing } from "./layout-schema";

export interface UiLayout extends Partial<UiSizing> {
    mode: "absolute" | "grid" | "row" | "column";
    x: number;
    y: number;
    width: number;
    height: number;
    row: number;
    column: number;
    rowSpan: number;
    columnSpan: number;
    columns: number;
    gap: number;
    padding: number;
    grow: number;
}
export interface UiNode {
    id: string;
    name: string;
    type: string;
    properties: Record<string, UiValue>;
    events: Record<string, string>;
    layout: UiLayout;
    children: UiNode[];
    /** Only live snapshots contain this path; XML serialization excludes it. */
    runtimePath?: number[];
    sourceOwned?: boolean;
    runtimeClass?: string;
    runtimeId?: string;
}
export interface UiDocument {
    version: 1 | 2 | 3;
    kind?: "component";
    controller: string;
    appClass?: string;
    root: UiNode;
}
export const IDENTIFIER = /^[A-Za-z][A-Za-z0-9_]*$/;
export const QUALIFIED_NAME = /^[A-Za-z]\w*(\.[A-Za-z]\w*)*$/;
export const MAX_NODES = 1000;
export const DEFAULT_LAYOUT: UiLayout = {
    mode: "column",
    x: 16,
    y: 16,
    width: 180,
    height: 36,
    row: 1,
    column: 1,
    rowSpan: 1,
    columnSpan: 1,
    columns: 2,
    gap: 12,
    padding: 12,
    grow: 0,
};
export function walk(node: UiNode): UiNode[] {
    return [node, ...node.children.flatMap(walk)];
}
export function findNode(root: UiNode, id: string): UiNode | undefined {
    return walk(root).find((n) => n.id === id);
}
export function findParent(root: UiNode, id: string): UiNode | undefined {
    return walk(root).find((n) => n.children.some((c) => c.id === id));
}
export function updateNode(
    root: UiNode,
    id: string,
    update: (node: UiNode) => UiNode,
): UiNode {
    return root.id === id
        ? update(root)
        : {
              ...root,
              children: root.children.map((n) => updateNode(n, id, update)),
          };
}
export function createNode(
    type: string,
    root?: UiNode,
    descriptor?: ComponentSpec,
): UiNode {
    const spec = descriptor ?? BUILTIN_CATALOG[type];
    if (!spec) throw new Error(`未注册组件：${type}`);
    const names = new Set(root ? walk(root).map((n) => n.name) : []);
    const prefix = type.split(".").at(-1) ?? "Component";
    let suffix = 1;
    while (names.has(`${prefix}${suffix}`)) suffix++;
    const mode =
        type === "GridLayout"
            ? "grid"
            : type === "RowLayout" || type === "SplitPane"
              ? "row"
              : "column";
    return {
        id: crypto.randomUUID(),
        type,
        name: `${prefix}${suffix}`,
        properties: {},
        events: {},
        children: [],
        layout: {
            ...DEFAULT_LAYOUT,
            mode,
            height: spec.container
                ? 220
                : type === "PlotView"
                  ? 260
                  : type === "Table" || type === "TextArea" || type === "Image"
                    ? 140
                    : 36,
            grow: spec.container || type === "PlotView" ? 1 : 0,
            ...spec.defaultLayout,
        },
    };
}
export function createDocument(appClass?: string): UiDocument {
    const root = createNode("Window");
    root.name = "MainWindow";
    if (appClass) root.events.Startup = "app.onStartup";
    return appClass
        ? { version: 2, appClass, controller: "", root }
        : { version: 1, controller: "", root };
}
export function createComponentDocument(appClass: string): UiDocument {
    const root = createNode("Panel");
    root.name = "Root";
    root.properties = { Title: "" };
    root.layout = { ...root.layout, width: 440, height: 100, grow: 0 };
    return { version: 3, kind: "component", controller: "", appClass, root };
}
export function moveNode(
    document: UiDocument,
    id: string,
    parentId: string,
    index?: number,
): UiDocument {
    const node = findNode(document.root, id);
    const parent = findNode(document.root, parentId);
    if (
        !node ||
        !parent ||
        id === document.root.id ||
        walk(node).some((n) => n.id === parentId)
    )
        throw new Error("不能将组件移入自身或后代。");
    assertParent(parent, node);
    const detached = updateNode(
        document.root,
        findParent(document.root, id)!.id,
        (n) => ({ ...n, children: n.children.filter((c) => c.id !== id) }),
    );
    return {
        ...document,
        root: updateNode(detached, parentId, (n) => {
            const children = [...n.children];
            children.splice(index ?? children.length, 0, node);
            return { ...n, children };
        }),
    };
}
export function assertParent(
    parent: UiNode,
    child: UiNode,
    catalog = BUILTIN_CATALOG,
): void {
    if (catalog[parent.type] && !catalog[parent.type]?.container)
        throw new Error("目标组件不能包含子组件。");
    if (child.type === "Window")
        throw new Error("一个界面文件只能有一个根窗口。");
    if (parent.type === "TabGroup" && child.type !== "Tab")
        throw new Error("选项卡组只能包含选项卡。");
    if (child.type === "Tab" && parent.type !== "TabGroup")
        throw new Error("选项卡必须放在选项卡组内。");
    if (
        parent.type === "SplitPane" &&
        parent.children.filter((c) => c.id !== child.id).length >= 2
    )
        throw new Error("分隔面板最多包含两个子组件。");
}
export function cloneNode(node: UiNode, root: UiNode): UiNode {
    const names = new Set(walk(root).map((n) => n.name));
    const copy = (n: UiNode): UiNode => {
        let name = `${n.name}Copy`;
        let index = 2;
        while (names.has(name)) name = `${n.name}Copy${index++}`;
        names.add(name);
        return {
            ...structuredClone(n),
            id: crypto.randomUUID(),
            name,
            children: n.children.map(copy),
        };
    };
    return copy(node);
}
export function validateDocument(document: UiDocument): void {
    const component = document.version === 3 && document.kind === "component";
    if (
        ![1, 2, 3].includes(document.version) ||
        (component
            ? document.root.type !== "Panel"
            : document.root.type !== "Window") ||
        (document.version === 3 && !component) ||
        (document.version !== 3 && document.kind)
    )
        throw new Error(
            "应用需要根 Window；version=3 的复合组件需要 kind=component 和根 Panel。",
        );
    if (
        document.version !== 1 &&
        (!document.appClass || !QUALIFIED_NAME.test(document.appClass))
    )
        throw new Error("应用类名称无效。");
    if (document.version !== 1 && document.controller)
        throw new Error("类应用使用成员方法连接，不使用函数控制器。");
    if (document.controller && !QUALIFIED_NAME.test(document.controller))
        throw new Error("回调函数名称无效。");
    const ids = new Set<string>();
    const names = new Set<string>();
    let count = 0;
    const visit = (n: UiNode, depth: number): void => {
        if (++count > MAX_NODES || depth > 32)
            throw new Error("界面超过 1000 个组件或 32 层嵌套的限制。");
        if (!/^[A-Za-z0-9_-]{1,80}$/.test(n.id) || ids.has(n.id))
            throw new Error(`组件 ID 无效或重复：${n.id}`);
        if (!IDENTIFIER.test(n.name) || names.has(n.name))
            throw new Error(`组件名称无效或重复：${n.name}`);
        if (!QUALIFIED_NAME.test(n.type))
            throw new Error(`组件类型无效：${n.type}`);
        ids.add(n.id);
        names.add(n.name);
        validateLayout(
            n.layout as unknown as Record<string, unknown>,
            DEFAULT_LAYOUT,
        );
        for (const [name, value] of Object.entries(n.properties)) {
            if (!isUiValue(value))
                throw new Error(
                    `属性只支持有限标量、字符串列表和二维表格：${name}`,
                );
            if (
                !IDENTIFIER.test(name) ||
                name === "__proto__" ||
                name === "constructor" ||
                name === "prototype"
            )
                throw new Error("属性名称无效。");
            const p = BUILTIN_CATALOG[n.type]?.properties.find(
                (p) => p.name === name,
            );
            if (
                p &&
                ((p.type === "number" &&
                    (typeof value !== "number" ||
                        !Number.isFinite(value) ||
                        (p.min !== undefined && value < p.min) ||
                        (p.max !== undefined && value > p.max))) ||
                    (p.type === "boolean" && typeof value !== "boolean") ||
                    (["text", "color", "choice"].includes(p.type) &&
                        typeof value !== "string") ||
                    (p.type === "choice" &&
                        !p.choices?.includes(String(value))) ||
                    (p.type === "items" &&
                        (!Array.isArray(value) ||
                            value.some((x) => typeof x !== "string"))) ||
                    (p.type === "table" &&
                        (!Array.isArray(value) ||
                            value.some((x) => !Array.isArray(x)))))
            )
                throw new Error(`属性值无效：${n.name}.${name}`);
        }
        for (const [event, handler] of Object.entries(n.events))
            if (!IDENTIFIER.test(event) || !QUALIFIED_NAME.test(handler))
                throw new Error("事件名称或回调函数名称无效。");
        n.children.forEach((child) => {
            assertParent(n, child);
            visit(child, depth + 1);
        });
    };
    visit(document.root, 0);
    if (document.version !== 1) {
        for (const node of walk(document.root)) {
            for (const handler of Object.values(node.events)) {
                const parts = handler.split(".");
                if (
                    parts.length !== 2 ||
                    !["app", "self", ...names].includes(parts[0]!)
                )
                    throw new Error(
                        "事件连接需要 对象.成员方法，例如 app.onRun。",
                    );
            }
        }
    }
}
export interface DocumentHistory {
    past: UiDocument[];
    present: UiDocument;
    future: UiDocument[];
}
export type HistoryAction =
    | { type: "edit"; document: UiDocument }
    | { type: "reset"; document: UiDocument }
    | { type: "undo" }
    | { type: "redo" };
export function historyReducer(
    state: DocumentHistory,
    action: HistoryAction,
): DocumentHistory {
    if (action.type === "reset")
        return { past: [], present: action.document, future: [] };
    if (action.type === "edit") {
        validateDocument(action.document);
        if (JSON.stringify(state.present) === JSON.stringify(action.document))
            return state;
        return {
            past: [...state.past.slice(-99), state.present],
            present: action.document,
            future: [],
        };
    }
    if (action.type === "undo") {
        const previous = state.past.at(-1);
        return previous
            ? {
                  past: state.past.slice(0, -1),
                  present: previous,
                  future: [state.present, ...state.future],
              }
            : state;
    }
    const next = state.future[0];
    return next
        ? {
              past: [...state.past, state.present],
              present: next,
              future: state.future.slice(1),
          }
        : state;
}
