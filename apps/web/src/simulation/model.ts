import {
    CONTROL_LABELS,
    CONTROL_PRESETS,
    controlInputs,
    parseHybridKind,
    type ControlKind,
    type ResetKind,
    type ControlOperation,
} from "./hybrid";
import { parseSampleTimes, type SampleTime } from "./sampling";
/** OpenMat authoring data. No React Flow implementation fields are persisted. */
import {
    validateEmbeddedSources,
    validateSlxAsset,
    type SlxAsset,
} from "./slx-authoring";
import {
    componentFor,
    validateComponent,
    validateComponentKind,
    attachComponent,
    modelSources,
    type ComponentDefinition,
    type ComponentKind,
} from "./components";
export type BlockType =
    | "control"
    | "resetIntegrator"
    | "step"
    | "constant"
    | "sum"
    | "gain"
    | "integrator"
    | "unitDelay"
    | "zeroOrderHold"
    | "discreteIntegrator"
    | "rateTransition"
    | "scope"
    | "mFunction"
    | "component";
export interface FunctionKind {
    type: "mFunction";
    source: string;
    entry: string;
    inputs: { name: string; width: number }[];
    parameters: { name: string; value: number[] }[];
    outputWidth: number;
}
export interface ExecutionOptions {
    backend: "reference" | "llvm";
    solver:
        | { type: "rk4" }
        | {
              type: "cvode";
              method: "adams" | "bdf";
              relativeTolerance: number;
              absoluteTolerance: number;
          };
}
export const DEFAULT_EXECUTION: ExecutionOptions = {
    backend: "reference",
    solver: { type: "rk4" },
};
export function validSourcePath(path: string): boolean {
    return (
        path.length <= 512 &&
        path.endsWith(".m") &&
        !/[\\:\0]/.test(path) &&
        path.split("/").every((part) => !!part && part !== "." && part !== "..")
    );
}
export function validateFunctionKind(raw: unknown): FunctionKind {
    const k = object(raw, "函数定义");
    keys(
        k,
        ["type", "source", "entry", "inputs", "parameters", "outputWidth"],
        "函数定义",
    );
    const identifier = (n: unknown): n is string =>
        typeof n === "string" && /^[A-Za-z][A-Za-z0-9_]{0,62}$/.test(n);
    const width = (n: unknown): boolean =>
        typeof n === "number" && Number.isSafeInteger(n) && n >= 1 && n <= 4096;
    if (
        k.type !== "mFunction" ||
        typeof k.source !== "string" ||
        !validSourcePath(k.source) ||
        !identifier(k.entry) ||
        !width(k.outputWidth) ||
        !Array.isArray(k.inputs) ||
        !Array.isArray(k.parameters) ||
        k.inputs.length + k.parameters.length > 64
    )
        throw new Error(
            "函数需要相对 .m 路径、合法函数名和 1–4096 的端口宽度，参数总数最多 64。",
        );
    const names = new Set<string>();
    for (const [items, parameter] of [
        [k.inputs, false],
        [k.parameters, true],
    ] as const) {
        for (const raw of items) {
            const p = object(raw, "函数参数");
            keys(
                p,
                parameter ? ["name", "value"] : ["name", "width"],
                "函数参数",
            );
            if (!identifier(p.name) || names.has(p.name))
                throw new Error("输入和参数名称必须合法且不能重复。");
            names.add(p.name);
            if (
                parameter
                    ? !Array.isArray(p.value) ||
                      p.value.length < 1 ||
                      p.value.length > 4096 ||
                      p.value.some(
                          (v) => typeof v !== "number" || !Number.isFinite(v),
                      )
                    : !width(p.width)
            )
                throw new Error(
                    "输入宽度应为 1–4096，参数应为有限实数列向量。",
                );
        }
    }
    return structuredClone(k) as unknown as FunctionKind;
}
export function parseExecution(raw: unknown): ExecutionOptions {
    const e = object(raw, "执行设置");
    keys(e, ["backend", "solver"], "执行设置");
    if (e.backend !== "reference" && e.backend !== "llvm")
        throw new Error("不支持的计算后端。");
    const s = object(e.solver, "求解器");
    if (s.type === "rk4") keys(s, ["type"], "RK4");
    else if (s.type === "cvode") {
        keys(
            s,
            ["type", "method", "relativeTolerance", "absoluteTolerance"],
            "CVODE",
        );
        const r = number(s.relativeTolerance, "相对误差"),
            a = number(s.absoluteTolerance, "绝对误差");
        if (
            (s.method !== "adams" && s.method !== "bdf") ||
            r < 1e-14 ||
            r > 0.1 ||
            a <= 0
        )
            throw new Error(
                "CVODE 需要合法方法、1e-14–0.1 的相对误差和正的绝对误差。",
            );
    } else throw new Error("不支持的求解器。");
    return structuredClone(e) as unknown as ExecutionOptions;
}
export type BlockKind =
    | ControlKind
    | ResetKind
    | { type: "step"; time: number; before: number[]; after: number[] }
    | { type: "constant"; value: number[] }
    | { type: "sum"; signs: number[] }
    | { type: "gain"; gain: number[] }
    | { type: "integrator" | "unitDelay"; initial: number[] }
    | { type: "zeroOrderHold" }
    | { type: "discreteIntegrator"; initial: number[]; gain: number }
    | { type: "rateTransition"; initial: number[]; deterministic: boolean }
    | { type: "scope" }
    | FunctionKind
    | ComponentKind;
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
    schemaVersion: 1 | 2 | 3 | 4 | 5 | 6;
    name: string;
    settings: {
        startTime: number;
        stopTime: number;
        maxStep: number;
        sampleTime?: number;
    };
    blocks: Block[];
    connections: Connection[];
    components?: ComponentDefinition[];
    sampleTimes?: Record<string, SampleTime>;
}
export interface ModelDocument {
    format: "openmat-simulation";
    schemaVersion: 1 | 2 | 3 | 4 | 5 | 6;
    sources?: Record<string, string>;
    slx?: SlxAsset;
    execution?: ExecutionOptions;
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
    icon: BlockType | ControlOperation["type"] | "resetDiscrete";
    preset?: string;
    inputs: string[];
    outputs: string[];
    parameter?: string;
    default?: number[];
}
// Offline authoring baseline; the native catalog is verified against these renderers.
export const DEFINITIONS: readonly BlockDefinition[] = [
    {
        type: "control",
        label: "Control",
        category: "控制逻辑",
        icon: "control",
        inputs: ["in0"],
        outputs: ["out"],
    },
    {
        type: "resetIntegrator",
        label: "Reset Integrator",
        category: "控制逻辑",
        icon: "resetIntegrator",
        inputs: ["in", "reset"],
        outputs: ["out"],
        parameter: "initial",
        default: [0],
    },
    {
        type: "step",
        label: "Step",
        category: "信号源",
        icon: "step",
        inputs: [],
        outputs: ["out"],
    },
    {
        type: "component",
        label: "Component",
        category: "自定义组件",
        icon: "component",
        inputs: [],
        outputs: [],
    },
    {
        type: "mFunction",
        label: "M Function",
        category: "自定义函数",
        icon: "mFunction",
        inputs: ["u"],
        outputs: ["out"],
    },
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
        type: "zeroOrderHold",
        label: "Zero-Order Hold",
        category: "离散",
        icon: "zeroOrderHold",
        inputs: ["in"],
        outputs: ["out"],
    },
    {
        type: "discreteIntegrator",
        label: "Discrete Integrator",
        category: "离散",
        icon: "discreteIntegrator",
        inputs: ["in"],
        outputs: ["out"],
        parameter: "initial",
        default: [0],
    },
    {
        type: "rateTransition",
        label: "Rate Transition",
        category: "离散",
        icon: "rateTransition",
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
export const PALETTE_DEFINITIONS: readonly BlockDefinition[] = [
    ...DEFINITIONS.filter(
        (d) => d.type !== "control" && d.type !== "resetIntegrator",
    ),
    ...CONTROL_PRESETS.map((p) => ({
        type: p.kind.type,
        preset: p.id,
        label: p.label,
        icon: p.icon,
        category: "控制逻辑",
        inputs:
            p.kind.type === "control"
                ? Array.from(
                      { length: controlInputs(p.kind.operation) },
                      (_, i) => `in${i}`,
                  )
                : ["in", "reset"],
        outputs: ["out"],
    })),
];
export function kindIcon(k: BlockKind): BlockDefinition["icon"] {
    return k.type === "control"
        ? k.operation.type
        : k.type === "resetIntegrator" && k.discrete
          ? "resetDiscrete"
          : k.type;
}
export function ports(
    block: Block,
    components?: ComponentDefinition[],
): { inputs: string[]; outputs: string[] } {
    if (block.kind.type === "control")
        return {
            inputs: Array.from(
                { length: controlInputs(block.kind.operation) },
                (_, i) => `in${i}`,
            ),
            outputs: ["out"],
        };
    if (block.kind.type === "component") {
        const d = componentFor(
            { ...(components ? { components } : {}) },
            block,
        );
        return {
            inputs: d?.inputs.map((p) => p.name) ?? [],
            outputs: d?.outputs.map((p) => p.name) ?? [],
        };
    }
    if (block.kind.type === "mFunction")
        return {
            inputs: block.kind.inputs.map((input) => input.name),
            outputs: ["out"],
        };
    return block.kind.type === "sum"
        ? {
              inputs: block.kind.signs.map((_, index) => `in${index}`),
              outputs: ["out"],
          }
        : definition(block.kind.type);
}
export function kind(type: BlockType): BlockKind {
    switch (type) {
        case "control":
            return structuredClone(CONTROL_PRESETS[0]!.kind);
        case "resetIntegrator":
            return {
                type,
                initial: [0],
                gain: 1,
                discrete: false,
                reset: "rising",
            };
        case "step":
            return { type, time: 1, before: [0], after: [1] };
        case "component":
            return { type, component: "", parameters: {} };
        case "mFunction":
            return {
                type,
                source: "my_function.m",
                entry: "my_function",
                inputs: [{ name: "u", width: 1 }],
                parameters: [],
                outputWidth: 1,
            };
        case "constant":
            return { type, value: [1] };
        case "sum":
            return { type, signs: [1, -1] };
        case "gain":
            return { type, gain: [1] };
        case "integrator":
        case "unitDelay":
            return { type, initial: [0] };
        case "discreteIntegrator":
            return { type, initial: [0], gain: 1 };
        case "rateTransition":
            return { type, initial: [0], deterministic: true };
        case "zeroOrderHold":
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
        : (componentFor(doc.model, block)?.name ??
          (block.kind.type === "control"
              ? CONTROL_LABELS[block.kind.operation.type]
              : block.kind.type === "resetIntegrator" && block.kind.discrete
                ? "Reset Discrete Integrator"
                : definition(block.kind.type).label));
export function parameterText(block: Block): string {
    const data = block.kind;
    if (data.type === "control") return CONTROL_LABELS[data.operation.type];
    if (data.type === "resetIntegrator")
        return `${data.discrete ? "Σ" : "∫"} · reset ${data.reset}`;
    if (data.type === "step")
        return `t ≥ ${data.time}: ${data.after.join(", ")}`;
    if (data.type === "component") return data.component;
    if (data.type === "mFunction")
        return `${data.entry}(${data.inputs.map((input) => input.name).join(", ")})`;
    if (data.type === "zeroOrderHold") return "sample & hold";
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
    doc.schemaVersion = model.schemaVersion;
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
        [
            "schemaVersion",
            "name",
            "settings",
            "blocks",
            "connections",
            "components",
            "sampleTimes",
        ],
        "模型",
    );
    if (
        data.schemaVersion !== 1 &&
        data.schemaVersion !== 2 &&
        data.schemaVersion !== 3 &&
        data.schemaVersion !== 4 &&
        data.schemaVersion !== 5 &&
        data.schemaVersion !== 6
    )
        throw new Error("不支持的模型版本。");
    if (
        data.components !== undefined &&
        (!Array.isArray(data.components) || data.components.length > 64)
    )
        throw new Error("模型组件定义列表无效。");
    const schemaVersion = data.schemaVersion;
    const components = ((data.components ?? []) as unknown[]).map((d) =>
        validateComponent(d, schemaVersion >= 5),
    );
    if (components.length && data.schemaVersion < 3)
        throw new Error("组件定义需要模型版本 3。");
    if (new Set(components.map((d) => d.id)).size !== components.length)
        throw new Error("组件 ID 重复。");
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
        if (k.type === "component") {
            if (schemaVersion < 3)
                throw new Error("自定义组件需要模型版本 3。");
            Object.assign(k, validateComponentKind(k, components));
        } else if (k.type === "control" || k.type === "resetIntegrator") {
            if (schemaVersion < 6)
                throw new Error("控制和复位方块需要模型版本 6。");
            Object.assign(k, parseHybridKind(k));
        } else if (k.type === "step") {
            if (schemaVersion < 4) throw new Error("Step 需要模型版本 4。");
            keys(k, ["type", "time", "before", "after"], "Step 参数");
            number(k.time, "阶跃时刻");
            for (const values of [k.before, k.after]) {
                if (
                    !Array.isArray(values) ||
                    !values.length ||
                    values.length > 4096
                )
                    throw new Error("Step 前后值需要非空的有限实数向量。");
                values.forEach((v) => number(v, "Step 参数"));
            }
            if ((k.before as number[]).length !== (k.after as number[]).length)
                throw new Error("Step 前后值宽度必须相同。");
        } else if (
            ["zeroOrderHold", "discreteIntegrator", "rateTransition"].includes(
                k.type,
            )
        ) {
            if (Number(data.schemaVersion) < 5)
                throw new Error("此离散方块需要模型版本 5。");
            keys(
                k,
                k.type === "zeroOrderHold"
                    ? ["type"]
                    : [
                          "type",
                          "initial",
                          k.type === "discreteIntegrator"
                              ? "gain"
                              : "deterministic",
                      ],
                "离散方块参数",
            );
            if (k.type === "discreteIntegrator") number(k.gain, "积分增益");
            if (
                k.type === "rateTransition" &&
                typeof k.deterministic !== "boolean"
            )
                throw new Error("速率转换需要布尔延迟设置。");
        } else if (k.type === "mFunction") {
            if (data.schemaVersion === 1)
                throw new Error("M Function 需要模型版本 2。");
            validateFunctionKind(k);
        } else
            keys(
                k,
                def.parameter ? ["type", def.parameter] : ["type"],
                "方块参数",
            );
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
            if (!node || !ports(node, components)[direction].includes(port))
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
    if (data.sampleTimes !== undefined && Number(data.schemaVersion) < 5)
        throw new Error("逐方块采样时间需要模型版本 5。");
    return {
        schemaVersion: data.schemaVersion,
        ...(data.sampleTimes === undefined
            ? {}
            : { sampleTimes: parseSampleTimes(data.sampleTimes, ids) }),
        ...(data.components !== undefined ? { components } : {}),
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
    keys(
        raw,
        [
            "format",
            "schemaVersion",
            "model",
            "editor",
            "execution",
            "sources",
            "slx",
        ],
        "模型文件",
    );
    if (
        raw.format !== "openmat-simulation" ||
        (raw.schemaVersion !== 1 &&
            raw.schemaVersion !== 2 &&
            raw.schemaVersion !== 3 &&
            raw.schemaVersion !== 4 &&
            raw.schemaVersion !== 5 &&
            raw.schemaVersion !== 6)
    )
        throw new Error("不支持的编辑器文件格式或版本。");
    const doc = fromModel(parseModel(raw.model));
    doc.schemaVersion = raw.schemaVersion;
    if (raw.sources !== undefined || raw.slx !== undefined) {
        if (
            raw.schemaVersion !== 4 &&
            raw.schemaVersion !== 5 &&
            raw.schemaVersion !== 6
        )
            throw new Error("内嵌源码和 SLX 层级需要文件版本 4。");
        if (raw.sources !== undefined)
            doc.sources = validateEmbeddedSources(raw.sources);
        if (raw.slx !== undefined) doc.slx = validateSlxAsset(raw.slx);
    }
    if (doc.schemaVersion < doc.model.schemaVersion)
        throw new Error("文件版本不能早于模型版本。");
    if (
        raw.schemaVersion === 1 &&
        (doc.model.schemaVersion !== 1 || raw.execution !== undefined)
    )
        throw new Error("此模型功能需要文件版本 2。");
    if (raw.execution !== undefined)
        doc.execution = parseExecution(raw.execution);
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
    for (const id of nodes) {
        delete next.editor.labels[id];
        if (next.model.sampleTimes) delete next.model.sampleTimes[id];
    }
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
    if (fragment.model.sampleTimes)
        fragment.model.sampleTimes = Object.fromEntries(
            Object.entries(fragment.model.sampleTimes).filter(([id]) =>
                selected.has(id),
            ),
        );
    delete fragment.editor.viewport;
    delete fragment.slx;
    if (fragment.sources) {
        const references = new Set(modelSources(fragment.model));
        fragment.sources = Object.fromEntries(
            Object.entries(fragment.sources).filter(([path]) =>
                references.has(path),
            ),
        );
    }
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
    if (next.model.schemaVersion < fragment.model.schemaVersion)
        next.model.schemaVersion = fragment.model.schemaVersion;
    if (next.schemaVersion < next.model.schemaVersion)
        next.schemaVersion = next.model.schemaVersion;
    if (fragment.sources) {
        next.sources ??= {};
        for (const [path, content] of Object.entries(fragment.sources)) {
            if (
                Object.hasOwn(next.sources, path) &&
                next.sources[path] !== content
            )
                throw new Error(`内嵌源码冲突：${path}`);
            next.sources[path] = content;
        }
        if (next.schemaVersion < 4) next.schemaVersion = 4;
        if (next.model.schemaVersion < 4) next.model.schemaVersion = 4;
    }
    for (const d of fragment.model.components ?? []) {
        if (
            fragment.model.blocks.some(
                (b) => b.kind.type === "component" && b.kind.component === d.id,
            )
        )
            attachComponent(next, d);
    }
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
        if (
            fragment.model.sampleTimes &&
            Object.hasOwn(fragment.model.sampleTimes, block.id)
        ) {
            next.model.sampleTimes ??= {};
            Object.defineProperty(next.model.sampleTimes, id, {
                value: structuredClone(fragment.model.sampleTimes[block.id]),
                enumerable: true,
                writable: true,
                configurable: true,
            });
        }
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
