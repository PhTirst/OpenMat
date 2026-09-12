import {
    validSourcePath,
    type Model,
    type Block,
    type ModelDocument,
} from "./model";

export type ComponentIcon =
    | "function"
    | "plant"
    | "controller"
    | "filter"
    | "delay";
export interface Callback {
    source: string;
    entry: string;
}
export interface ComponentParameter {
    name: string;
    value: number[];
    label?: string;
    unit?: string;
    minimum?: number;
    maximum?: number;
}
export interface ComponentDefinition {
    id: string;
    name: string;
    category: string;
    icon: ComponentIcon;
    inputs: { name: string; width: number }[];
    outputs: { name: string; width: number }[];
    parameters: ComponentParameter[];
    continuousStates: number;
    discreteStates: number;
    sampleTime?: number;
    initialize?: Callback;
    outputsFunction: Callback;
    derivatives?: Callback;
    update?: Callback;
}
export interface ComponentKind {
    type: "component";
    component: string;
    parameters: Record<string, number[]>;
}
export const CALLBACK_ROLES = [
    "initialize",
    "outputsFunction",
    "derivatives",
    "update",
] as const;
export type CallbackRole = (typeof CALLBACK_ROLES)[number];
export const CALLBACK_LABELS: Record<CallbackRole, string> = {
    initialize: "初始化",
    outputsFunction: "输出",
    derivatives: "连续导数",
    update: "离散更新",
};
export function callbacks(definition: ComponentDefinition): Callback[] {
    return CALLBACK_ROLES.flatMap((role) =>
        definition[role] ? [definition[role]] : [],
    );
}
export function componentFor(
    model: Pick<Model, "components">,
    block: Block,
): ComponentDefinition | undefined {
    const k = block.kind;
    return k.type === "component"
        ? model.components?.find((d) => d.id === k.component)
        : undefined;
}
export function blockSources(model: Model, block: Block): string[] {
    if (block.kind.type === "mFunction") return [block.kind.source];
    const definition = componentFor(model, block);
    return definition ? callbacks(definition).map((c) => c.source) : [];
}
export function modelSources(model: Model): string[] {
    return [...new Set(model.blocks.flatMap((b) => blockSources(model, b)))];
}
const record = (value: unknown): Record<string, unknown> => {
    if (!value || typeof value !== "object" || Array.isArray(value))
        throw new Error("组件字段必须是对象。");
    return value as Record<string, unknown>;
};
const keys = (value: Record<string, unknown>, fields: string[]) => {
    if (Object.keys(value).some((k) => !fields.includes(k)))
        throw new Error("组件包含未知字段。");
};
const identifier = (v: unknown): v is string =>
    typeof v === "string" && /^[A-Za-z][A-Za-z0-9_]{0,62}$/.test(v);
const integer = (v: unknown, min = 0): v is number =>
    typeof v === "number" && Number.isSafeInteger(v) && v >= min && v <= 4096;
const finite = (v: unknown): v is number =>
    typeof v === "number" && Number.isFinite(v);
const text = (v: unknown, max: number, empty = false): v is string =>
    typeof v === "string" &&
    new TextEncoder().encode(v).length <= max &&
    (empty || v.trim().length > 0);
export function validateComponent(raw: unknown): ComponentDefinition {
    const d = record(raw);
    keys(d, [
        "id",
        "name",
        "category",
        "icon",
        "inputs",
        "outputs",
        "parameters",
        "continuousStates",
        "discreteStates",
        "sampleTime",
        ...CALLBACK_ROLES,
    ]);
    if (
        typeof d.id !== "string" ||
        !/^[A-Za-z0-9_-]{1,128}$/.test(d.id) ||
        !text(d.name, 256) ||
        !text(d.category, 128, true) ||
        !["function", "plant", "controller", "filter", "delay"].includes(
            String(d.icon),
        ) ||
        !integer(d.continuousStates) ||
        !integer(d.discreteStates) ||
        d.continuousStates + d.discreteStates > 4096 ||
        Boolean(d.initialize) !== d.continuousStates + d.discreteStates > 0 ||
        Boolean(d.derivatives) !== d.continuousStates > 0 ||
        Boolean(d.update) !== d.discreteStates > 0 ||
        (d.sampleTime !== undefined) !== d.discreteStates > 0 ||
        (d.sampleTime !== undefined &&
            (!finite(d.sampleTime) || d.sampleTime <= 0))
    )
        throw new Error(
            "组件名称、图标、状态数量或生命周期回调不完整；离散状态需要正的采样周期。",
        );
    for (const role of CALLBACK_ROLES) {
        if (d[role] === undefined && role !== "outputsFunction") continue;
        const c = record(d[role]);
        keys(c, ["source", "entry"]);
        if (
            typeof c.source !== "string" ||
            !validSourcePath(c.source) ||
            !identifier(c.entry)
        )
            throw new Error("回调需要相对 .m 路径和合法函数名。");
    }
    for (const direction of ["inputs", "outputs"] as const) {
        const ports = d[direction];
        if (
            !Array.isArray(ports) ||
            ports.length > 64 ||
            (direction === "outputs" && !ports.length)
        )
            throw new Error("组件最多 64 个输入/输出，至少一个输出。");
        const names = new Set<string>();
        let total = 0;
        for (const raw of ports) {
            const p = record(raw);
            keys(p, ["name", "width"]);
            if (
                !identifier(p.name) ||
                names.has(p.name) ||
                !integer(p.width, 1)
            )
                throw new Error("端口名称须合法且不重复，宽度为 1–4096。");
            names.add(p.name);
            total += p.width;
        }
        if (total > 4096) throw new Error("组件输入/输出的总宽度最多 4096。");
    }
    if (!Array.isArray(d.parameters) || d.parameters.length > 64)
        throw new Error("组件最多 64 个参数。");
    const names = new Set<string>();
    let total = 0;
    for (const raw of d.parameters) {
        const p = record(raw);
        keys(p, ["name", "value", "label", "unit", "minimum", "maximum"]);
        if (
            !identifier(p.name) ||
            names.has(p.name) ||
            !Array.isArray(p.value) ||
            !integer(p.value.length, 1) ||
            (p.label !== undefined && !text(p.label, 256, true)) ||
            (p.unit !== undefined && !text(p.unit, 64, true)) ||
            (p.minimum !== undefined && !finite(p.minimum)) ||
            (p.maximum !== undefined && !finite(p.maximum)) ||
            (finite(p.minimum) && finite(p.maximum) && p.minimum > p.maximum) ||
            p.value.some(
                (v) =>
                    !finite(v) ||
                    (finite(p.minimum) && v < p.minimum) ||
                    (finite(p.maximum) && v > p.maximum),
            )
        )
            throw new Error("参数需要唯一合法名称、有限数值和有效上下限。");
        names.add(p.name);
        total += p.value.length;
    }
    if (total > 4096) throw new Error("参数总宽度最多 4096。");
    return structuredClone(d) as unknown as ComponentDefinition;
}
export function validateComponentKind(
    raw: unknown,
    definitions: ComponentDefinition[],
): ComponentKind {
    const kind = record(raw);
    keys(kind, ["type", "component", "parameters"]);
    const definition = definitions.find((d) => d.id === kind.component);
    if (kind.type !== "component" || !definition)
        throw new Error("实例引用了不存在的组件定义。");
    const overrides = record(kind.parameters ?? {});
    for (const [name, values] of Object.entries(overrides)) {
        const p = definition.parameters.find((p) => p.name === name);
        if (
            !p ||
            !Array.isArray(values) ||
            values.length !== p.value.length ||
            values.some(
                (v) =>
                    !finite(v) ||
                    (p.minimum !== undefined && v < p.minimum) ||
                    (p.maximum !== undefined && v > p.maximum),
            )
        )
            throw new Error(`参数 ${name} 的形状或数值无效。`);
    }
    return {
        type: "component",
        component: definition.id,
        parameters: structuredClone(overrides) as Record<string, number[]>,
    };
}
export function parseComponentFile(content: string): ComponentDefinition {
    if (new TextEncoder().encode(content).length > 65536)
        throw new Error("组件定义超过 64 KiB。");
    const file = record(JSON.parse(content));
    keys(file, ["format", "schemaVersion", "definition"]);
    if (file.format !== "openmat-component" || file.schemaVersion !== 1)
        throw new Error("不支持的组件库格式。");
    return validateComponent(file.definition);
}
export const componentFile = (definition: ComponentDefinition): string =>
    JSON.stringify(
        { format: "openmat-component", schemaVersion: 1, definition },
        null,
        2,
    ) + "\n";
function canonical(value: unknown): unknown {
    if (Array.isArray(value)) return value.map(canonical);
    if (value && typeof value === "object")
        return Object.fromEntries(
            Object.entries(value)
                .sort(([a], [b]) => a.localeCompare(b))
                .map(([key, item]) => [key, canonical(item)]),
        );
    return value;
}
/** Model-local source copies have different filenames, but retain the interface. */
export function sameComponentDefinition(
    a: ComponentDefinition,
    b: ComponentDefinition,
    includeSources = true,
): boolean {
    const normalized = (d: ComponentDefinition) => {
        const value: Record<string, unknown> = { ...d };
        if (!includeSources)
            for (const role of CALLBACK_ROLES)
                if (value[role]) value[role] = true;
        return JSON.stringify(canonical(value));
    };
    return normalized(a) === normalized(b);
}
export function attachComponent(
    doc: ModelDocument,
    definition: ComponentDefinition,
): void {
    validateComponent(definition);
    const existing = doc.model.components?.find((d) => d.id === definition.id);
    if (existing && !sameComponentDefinition(existing, definition))
        throw new Error(
            "相同 ID 的组件定义不同；请编辑模型内的定义，或在组件库文件中使用新的 ID。",
        );
    if (!existing) {
        if ((doc.model.components?.length ?? 0) >= 64)
            throw new Error("模型最多 64 个组件定义。");
        (doc.model.components ??= []).push(structuredClone(definition));
    }
    doc.schemaVersion = 3;
    doc.model.schemaVersion = 3;
}
