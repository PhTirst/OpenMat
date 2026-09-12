/** OpenMat authoring data. No React Flow implementation fields are persisted. */
export type BlockType =
    | "constant"
    | "sum"
    | "gain"
    | "integrator"
    | "unitDelay"
    | "scope";
export type BlockKind =
    | { type: "constant"; value: number[] }
    | { type: "sum"; signs: number[] }
    | { type: "gain"; gain: number[] }
    | { type: "integrator" | "unitDelay"; initial: number[] }
    | { type: "scope" };
export interface Point {
    x: number;
    y: number;
}
export interface Block {
    id: string;
    kind: BlockKind;
    position?: Point;
}
export interface Port {
    block: string;
    port: string;
}
export interface Connection {
    from: Port;
    to: Port;
}
export interface Model {
    schemaVersion: 1;
    name: string;
    settings: {
        startTime: number;
        stopTime: number;
        maxStep: number;
        sampleTime?: number;
    };
    blocks: Block[];
    connections: Connection[];
}
export interface ModelDocument {
    format: "openmat-simulation";
    schemaVersion: 1;
    model: Model;
    editor: {
        labels: Record<string, string>;
        bends: Record<string, Point>;
        viewport?: Point & { zoom: number };
    };
}
export interface BlockDefinition {
    type: BlockType;
    label: string;
    category: string;
    icon: BlockType;
    inputs: string[];
    outputs: string[];
    parameter?: string;
    default?: number[];
}
// Offline authoring baseline; the native catalog is verified against these six renderers.
export const DEFINITIONS: readonly BlockDefinition[] = [
    {
        type: "constant",
        label: "Constant",
        category: "信号源",
        icon: "constant",
        inputs: [],
        outputs: ["out"],
        parameter: "value",
        default: [1],
    },
    {
        type: "sum",
        label: "Sum",
        category: "数学运算",
        icon: "sum",
        inputs: ["in0", "in1"],
        outputs: ["out"],
        parameter: "signs",
        default: [1, -1],
    },
    {
        type: "gain",
        label: "Gain",
        category: "数学运算",
        icon: "gain",
        inputs: ["in"],
        outputs: ["out"],
        parameter: "gain",
        default: [1],
    },
    {
        type: "integrator",
        label: "Integrator",
        category: "连续",
        icon: "integrator",
        inputs: ["in"],
        outputs: ["out"],
        parameter: "initial",
        default: [0],
    },
    {
        type: "unitDelay",
        label: "Unit Delay",
        category: "离散",
        icon: "unitDelay",
        inputs: ["in"],
        outputs: ["out"],
        parameter: "initial",
        default: [0],
    },
    {
        type: "scope",
        label: "Scope",
        category: "观察器",
        icon: "scope",
        inputs: ["in"],
        outputs: [],
    },
];
export const definition = (type: BlockType): BlockDefinition =>
    DEFINITIONS.find((item) => item.type === type)!;
export const isBlockType = (value: string): value is BlockType =>
    DEFINITIONS.some((item) => item.type === value);
export function ports(block: Block): { inputs: string[]; outputs: string[] } {
    return block.kind.type === "sum"
        ? {
              inputs: block.kind.signs.map((_, index) => `in${index}`),
              outputs: ["out"],
          }
        : definition(block.kind.type);
}
export function kind(type: BlockType): BlockKind {
    switch (type) {
        case "constant":
            return { type, value: [1] };
        case "sum":
            return { type, signs: [1, -1] };
        case "gain":
            return { type, gain: [1] };
        case "integrator":
        case "unitDelay":
            return { type, initial: [0] };
        case "scope":
            return { type };
    }
}
export const edgeId = (edge: Connection): string =>
    [edge.from.block, edge.from.port, edge.to.block, edge.to.port]
        .map(encodeURIComponent)
        .join("|");
export const label = (doc: ModelDocument, block: Block): string =>
    Object.hasOwn(doc.editor.labels, block.id)
        ? doc.editor.labels[block.id]!
        : definition(block.kind.type).label;
export function parameterText(block: Block): string {
    const data = block.kind;
    if (data.type === "scope") return "signal → time";
    if (data.type === "sum")
        return data.signs.map((sign) => (sign === 1 ? "+" : "−")).join(" ");
    const values =
        data.type === "constant"
            ? data.value
            : data.type === "gain"
              ? data.gain
              : data.initial;
    return values.length === 1 ? String(values[0]) : `[${values.join(", ")}]`;
}
export function numericLiteral(text: string): number[] {
    const raw = text
        .trim()
        .replace(/^\[(.*)\]$/s, "$1")
        .trim();
    if (!raw || !/^[\d.eE+\-,;\s]+$/.test(raw))
        throw new Error(
            "请输入实数或数值向量，例如 2、[1, 2]；此处不执行 m 表达式。",
        );
    if (/^[,;]|[,;]$|[,;]\s*[,;]/.test(raw))
        throw new Error("数值向量不能包含空元素。");
    const values = raw.split(/[\s,;]+/).map(Number);
    if (values.length > 4096 || values.some((value) => !Number.isFinite(value)))
        throw new Error("请输入有限实数，向量最多 4096 个元素。");
    return values;
}
export function emptyDocument(name = "Untitled"): ModelDocument {
    return {
        format: "openmat-simulation",
        schemaVersion: 1,
        model: {
            schemaVersion: 1,
            name,
            settings: {
                startTime: 0,
                stopTime: 5,
                maxStep: 0.02,
                sampleTime: 0.1,
            },
            blocks: [],
            connections: [],
        },
        editor: { labels: {}, bends: {} },
    };
}
export function fromModel(model: Model): ModelDocument {
    const doc = emptyDocument(model.name);
    doc.model = structuredClone(model);
    doc.model.blocks.forEach((block, index) => {
        block.position ??= {
            x: 80 + (index % 5) * 215,
            y: 100 + Math.floor(index / 5) * 150,
        };
    });
    return doc;
}
function object(value: unknown, context: string): Record<string, unknown> {
    if (!value || typeof value !== "object" || Array.isArray(value))
        throw new Error(`${context} 必须是对象。`);
    return value as Record<string, unknown>;
}
function keys(
    value: Record<string, unknown>,
    allowed: string[],
    context: string,
): void {
    const extra = Object.keys(value).find((key) => !allowed.includes(key));
    if (extra) throw new Error(`${context} 包含不支持的字段 ${extra}。`);
}
function text(value: unknown, context: string): string {
    if (typeof value !== "string" || !value.trim() || value.length > 256)
        throw new Error(`${context} 必须是 1–256 个字符。`);
    return value;
}
function number(value: unknown, context: string): number {
    if (typeof value !== "number" || !Number.isFinite(value))
        throw new Error(`${context} 必须是有限实数。`);
    return value;
}
function point(value: unknown): Point {
    const p = object(value, "坐标");
    keys(p, ["x", "y"], "坐标");
    const result = { x: number(p.x, "x"), y: number(p.y, "y") };
    if (Math.abs(result.x) > 1e7 || Math.abs(result.y) > 1e7)
        throw new Error("坐标超出允许范围。");
    return result;
}
export function parseModel(value: unknown): Model {
    const data = object(value, "模型");
    keys(
        data,
        ["schemaVersion", "name", "settings", "blocks", "connections"],
        "模型",
    );
    if (data.schemaVersion !== 1) throw new Error("不支持的模型版本。");
    const settings = object(data.settings, "仿真设置");
    keys(
        settings,
        ["startTime", "stopTime", "maxStep", "sampleTime"],
        "仿真设置",
    );
    if (
        !Array.isArray(data.blocks) ||
        data.blocks.length > 10000 ||
        !Array.isArray(data.connections) ||
        data.connections.length > 640000
    )
        throw new Error("模型的方块或连接列表无效或过大。");
    const ids = new Set<string>();
    const blocks: Block[] = data.blocks.map((raw) => {
        const block = object(raw, "方块");
        keys(block, ["id", "kind", "position"], "方块");
        const id = text(block.id, "方块 ID");
        if (ids.has(id)) throw new Error(`重复的方块 ID：${id}`);
        ids.add(id);
        const k = object(block.kind, "方块类型");
        if (typeof k.type !== "string" || !isBlockType(k.type))
            throw new Error(`不支持的方块类型：${String(k.type)}`);
        const def = definition(k.type);
        keys(k, def.parameter ? ["type", def.parameter] : ["type"], "方块参数");
        if (def.parameter) {
            const values = k[def.parameter];
            if (
                !Array.isArray(values) ||
                !values.length ||
                values.length > 262144
            )
                throw new Error(`${id} 参数必须是非空数值数组。`);
            values.forEach((value) => number(value, `${id} 参数`));
            if (
                k.type === "sum" &&
                (values.length < 1 ||
                    values.length > 64 ||
                    values.some((sign) => sign !== 1 && sign !== -1))
            )
                throw new Error("Sum 必须有 1–64 个 +1/-1 输入符号。");
        }
        return {
            id,
            kind: structuredClone(k) as BlockKind,
            ...(block.position === undefined
                ? {}
                : { position: point(block.position) }),
        };
    });
    const byId = new Map(blocks.map((block) => [block.id, block]));
    const inputDrivers = new Set<string>();
    const connections = data.connections.map((raw) => {
        const e = object(raw, "连接");
        keys(e, ["from", "to"], "连接");
        const readPort = (
            rawPort: unknown,
            direction: "inputs" | "outputs",
        ): Port => {
            const p = object(rawPort, "端口");
            keys(p, ["block", "port"], "端口");
            const block = text(p.block, "端口方块"),
                port = text(p.port, "端口名称");
            const node = byId.get(block);
            if (!node || !ports(node)[direction].includes(port))
                throw new Error(`不存在的端口 ${block}.${port}`);
            return { block, port };
        };
        const edge = {
            from: readPort(e.from, "outputs"),
            to: readPort(e.to, "inputs"),
        };
        const target = JSON.stringify(edge.to);
        if (inputDrivers.has(target))
            throw new Error(
                `输入端口 ${edge.to.block}.${edge.to.port} 有多个信号源。`,
            );
        inputDrivers.add(target);
        return edge;
    });
    return {
        schemaVersion: 1,
        name: text(data.name, "模型名称"),
        blocks,
        connections,
        settings: {
            startTime: number(settings.startTime, "起始时间"),
            stopTime: number(settings.stopTime, "停止时间"),
            maxStep: number(settings.maxStep, "最大步长"),
            ...(settings.sampleTime === undefined
                ? {}
                : { sampleTime: number(settings.sampleTime, "离散周期") }),
        },
    };
}
export function parseDocument(source: string): ModelDocument {
    if (new TextEncoder().encode(source).length > 8 * 1024 * 1024)
        throw new Error("模型文件超过 8 MiB。");
    const raw = object(JSON.parse(source), "模型文件");
    if (!Object.hasOwn(raw, "format")) return fromModel(parseModel(raw));
    keys(raw, ["format", "schemaVersion", "model", "editor"], "模型文件");
    if (raw.format !== "openmat-simulation" || raw.schemaVersion !== 1)
        throw new Error("不支持的编辑器文件格式或版本。");
    const doc = fromModel(parseModel(raw.model));
    const editor = object(raw.editor, "编辑器数据");
    keys(editor, ["labels", "bends", "viewport"], "编辑器数据");
    const ids = new Set(doc.model.blocks.map((block) => block.id));
    for (const [id, name] of Object.entries(
        object(editor.labels, "方块名称"),
    )) {
        if (!ids.has(id)) throw new Error(`名称引用了不存在的方块 ${id}`);
        Object.defineProperty(doc.editor.labels, id, {
            value: text(name, "方块名称"),
            enumerable: true,
            writable: true,
            configurable: true,
        });
    }
    const edges = new Set(doc.model.connections.map(edgeId));
    for (const [id, bend] of Object.entries(object(editor.bends, "连线路径"))) {
        if (!edges.has(id)) throw new Error("连线路径引用了不存在的连接。");
        doc.editor.bends[id] = point(bend);
    }
    if (editor.viewport !== undefined) {
        const view = object(editor.viewport, "视图");
        keys(view, ["x", "y", "zoom"], "视图");
        const zoom = number(view.zoom, "缩放");
        if (zoom < 0.1 || zoom > 2.5) throw new Error("视图缩放超出允许范围。");
        doc.editor.viewport = { ...point({ x: view.x, y: view.y }), zoom };
    }
    return doc;
}
export const serializeDocument = (doc: ModelDocument): string =>
    `${JSON.stringify(doc, null, 2)}\n`;
export function removeSelection(
    doc: ModelDocument,
    nodes: ReadonlySet<string>,
    edges: ReadonlySet<string>,
): ModelDocument {
    const next = structuredClone(doc);
    next.model.blocks = next.model.blocks.filter(
        (block) => !nodes.has(block.id),
    );
    next.model.connections = next.model.connections.filter(
        (edge) =>
            !nodes.has(edge.from.block) &&
            !nodes.has(edge.to.block) &&
            !edges.has(edgeId(edge)),
    );
    for (const id of nodes) delete next.editor.labels[id];
    const kept = new Set(next.model.connections.map(edgeId));
    for (const id of Object.keys(next.editor.bends))
        if (!kept.has(id)) delete next.editor.bends[id];
    return next;
}
export function copySelection(
    doc: ModelDocument,
    selected: ReadonlySet<string>,
): ModelDocument {
    const fragment = structuredClone(doc);
    fragment.model.blocks = fragment.model.blocks.filter((block) =>
        selected.has(block.id),
    );
    fragment.model.connections = fragment.model.connections.filter(
        (edge) => selected.has(edge.from.block) && selected.has(edge.to.block),
    );
    fragment.editor.labels = Object.fromEntries(
        Object.entries(fragment.editor.labels).filter(([id]) =>
            selected.has(id),
        ),
    );
    const edges = new Set(fragment.model.connections.map(edgeId));
    fragment.editor.bends = Object.fromEntries(
        Object.entries(fragment.editor.bends).filter(([id]) => edges.has(id)),
    );
    delete fragment.editor.viewport;
    return fragment;
}
export function pasteFragment(
    doc: ModelDocument,
    fragment: ModelDocument,
    idFactory: () => string,
    offset = 40,
): { document: ModelDocument; ids: string[] } {
    const next = structuredClone(doc),
        remap = new Map(
            fragment.model.blocks.map((block) => [block.id, idFactory()]),
        );
    for (const block of fragment.model.blocks) {
        const id = remap.get(block.id)!;
        next.model.blocks.push({
            ...structuredClone(block),
            id,
            position: {
                x: (block.position?.x ?? 0) + offset,
                y: (block.position?.y ?? 0) + offset,
            },
        });
        next.editor.labels[id] = `${label(fragment, block)} copy`;
    }
    for (const edge of fragment.model.connections) {
        const copied = {
            from: { block: remap.get(edge.from.block)!, port: edge.from.port },
            to: { block: remap.get(edge.to.block)!, port: edge.to.port },
        };
        next.model.connections.push(copied);
        const bend = fragment.editor.bends[edgeId(edge)];
        if (bend)
            next.editor.bends[edgeId(copied)] = {
                x: bend.x + offset,
                y: bend.y + offset,
            };
    }
    return { document: next, ids: [...remap.values()] };
}
export interface History {
    past: ModelDocument[];
    present: ModelDocument;
    future: ModelDocument[];
}
export function commit(history: History, next: ModelDocument): History {
    if (serializeDocument(history.present) === serializeDocument(next))
        return history;
    return {
        past: [...history.past.slice(-79), history.present],
        present: next,
        future: [],
    };
}
export function undo(history: History): History {
    const present = history.past.at(-1);
    return present
        ? {
              past: history.past.slice(0, -1),
              present,
              future: [history.present, ...history.future],
          }
        : history;
}
export function redo(history: History): History {
    const present = history.future[0];
    return present
        ? {
              past: [...history.past, history.present],
              present,
              future: history.future.slice(1),
          }
        : history;
}
