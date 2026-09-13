import type { SampleTime } from "./sampling";
import { parseSampleTime } from "./sampling";

export type Comparison =
    "equal" | "notEqual" | "less" | "lessEqual" | "greater" | "greaterEqual";
export type LogicOperator =
    "and" | "or" | "xor" | "nand" | "nor" | "not" | "nxor";
export type ControlOperation =
    | { type: "saturation"; lower: number[]; upper: number[] }
    | {
          type: "switch";
          criterion: "greater" | "greaterEqual" | "nonzero";
          threshold: number;
      }
    | { type: "relational"; operator: Comparison }
    | { type: "logical"; operator: LogicOperator; inputs: number }
    | { type: "abs" }
    | { type: "minMax"; minimum: boolean; inputs: number };
export type ControlKind = {
    type: "control";
    operation: ControlOperation;
    zeroCrossing: boolean;
};
export type ResetKind = {
    type: "resetIntegrator";
    initial: number[];
    gain: number;
    discrete: boolean;
    reset: "rising" | "falling" | "either";
};
export const CONTROL_LABELS = {
    saturation: "Saturation",
    switch: "Switch",
    relational: "Relational Operator",
    logical: "Logical Operator",
    abs: "Abs",
    minMax: "MinMax",
};
export const CONTROL_PRESETS: {
    id: string;
    label: string;
    icon: ControlOperation["type"] | "resetIntegrator" | "resetDiscrete";
    kind: ControlKind | ResetKind;
}[] = [
    {
        id: "saturation",
        label: "Saturation",
        icon: "saturation",
        kind: {
            type: "control",
            operation: { type: "saturation", lower: [-1], upper: [1] },
            zeroCrossing: true,
        },
    },
    {
        id: "switch",
        label: "Switch",
        icon: "switch",
        kind: {
            type: "control",
            operation: {
                type: "switch",
                criterion: "greaterEqual",
                threshold: 0,
            },
            zeroCrossing: true,
        },
    },
    {
        id: "relational",
        label: "Relational Operator",
        icon: "relational",
        kind: {
            type: "control",
            operation: { type: "relational", operator: "greaterEqual" },
            zeroCrossing: true,
        },
    },
    {
        id: "logical",
        label: "Logical Operator",
        icon: "logical",
        kind: {
            type: "control",
            operation: { type: "logical", operator: "and", inputs: 2 },
            zeroCrossing: false,
        },
    },
    {
        id: "abs",
        label: "Abs",
        icon: "abs",
        kind: {
            type: "control",
            operation: { type: "abs" },
            zeroCrossing: true,
        },
    },
    {
        id: "minMax",
        label: "MinMax",
        icon: "minMax",
        kind: {
            type: "control",
            operation: { type: "minMax", minimum: true, inputs: 2 },
            zeroCrossing: true,
        },
    },
    {
        id: "resetIntegrator",
        label: "Reset Integrator",
        icon: "resetIntegrator",
        kind: {
            type: "resetIntegrator",
            initial: [0],
            gain: 1,
            discrete: false,
            reset: "rising",
        },
    },
    {
        id: "resetDiscrete",
        label: "Reset Discrete Integrator",
        icon: "resetDiscrete",
        kind: {
            type: "resetIntegrator",
            initial: [0],
            gain: 1,
            discrete: true,
            reset: "rising",
        },
    },
];
export function controlInputs(operation: ControlOperation): number {
    return operation.type === "switch"
        ? 3
        : operation.type === "relational"
          ? 2
          : "inputs" in operation
            ? operation.inputs
            : 1;
}
function record(value: unknown): Record<string, unknown> {
    if (!value || typeof value !== "object" || Array.isArray(value))
        throw new Error("控制参数必须是对象。");
    return value as Record<string, unknown>;
}
function keys(value: Record<string, unknown>, allowed: string[]) {
    if (Object.keys(value).some((k) => !allowed.includes(k)))
        throw new Error("控制参数包含未知字段。");
}
function finite(value: unknown): value is number {
    return typeof value === "number" && Number.isFinite(value);
}
function vector(value: unknown): value is number[] {
    return (
        Array.isArray(value) &&
        value.length > 0 &&
        value.length <= 4096 &&
        value.every(finite)
    );
}
export function parseHybridKind(value: unknown): ControlKind | ResetKind {
    const k = record(value);
    if (k.type === "resetIntegrator") {
        keys(k, ["type", "initial", "gain", "discrete", "reset"]);
        if (
            !vector(k.initial) ||
            !finite(k.gain) ||
            typeof k.discrete !== "boolean" ||
            typeof k.reset !== "string" ||
            !["rising", "falling", "either"].includes(k.reset)
        )
            throw new Error("复位积分器参数无效。");
    } else if (k.type === "control") {
        keys(k, ["type", "operation", "zeroCrossing"]);
        if (typeof k.zeroCrossing !== "boolean")
            throw new Error("零交叉检测开关无效。");
        const o = record(k.operation);
        const allowed = {
            saturation: ["lower", "upper"],
            switch: ["criterion", "threshold"],
            relational: ["operator"],
            logical: ["operator", "inputs"],
            abs: [],
            minMax: ["minimum", "inputs"],
        };
        if (typeof o.type !== "string" || !Object.hasOwn(allowed, o.type))
            throw new Error("不支持的控制运算。");
        keys(o, ["type", ...allowed[o.type as keyof typeof allowed]]);
        switch (o.type) {
            case "saturation": {
                if (!vector(o.lower) || !vector(o.upper))
                    throw new Error("上下限需要有限实数向量。");
                const n = Math.max(o.lower.length, o.upper.length);
                if (
                    ![1, n].includes(o.lower.length) ||
                    ![1, n].includes(o.upper.length)
                )
                    throw new Error("上下限宽度不匹配。");
                for (let i = 0; i < n; i++)
                    if (
                        o.lower[i % o.lower.length]! >
                        o.upper[i % o.upper.length]!
                    )
                        throw new Error("下限不能大于上限。");
                break;
            }
            case "switch":
                if (
                    !finite(o.threshold) ||
                    typeof o.criterion !== "string" ||
                    !["greater", "greaterEqual", "nonzero"].includes(
                        o.criterion,
                    )
                )
                    throw new Error("Switch 阈值或判断方式无效。");
                break;
            case "relational":
                if (
                    typeof o.operator !== "string" ||
                    ![
                        "equal",
                        "notEqual",
                        "less",
                        "lessEqual",
                        "greater",
                        "greaterEqual",
                    ].includes(o.operator)
                )
                    throw new Error("比较方式无效。");
                break;
            case "logical":
                if (
                    typeof o.operator !== "string" ||
                    ![
                        "and",
                        "or",
                        "xor",
                        "nand",
                        "nor",
                        "not",
                        "nxor",
                    ].includes(o.operator)
                )
                    throw new Error("逻辑运算无效。");
                break;
            case "minMax":
                if (typeof o.minimum !== "boolean")
                    throw new Error("极值运算无效。");
                break;
        }
        if (
            (o.type === "logical" || o.type === "minMax") &&
            (!finite(o.inputs) ||
                !Number.isSafeInteger(o.inputs) ||
                (o.operator === "not"
                    ? o.inputs !== 1
                    : o.inputs < (o.type === "minMax" ? 1 : 2) ||
                      o.inputs > 64))
        )
            throw new Error(
                "端口数量无效：MinMax 为 1–64，逻辑运算为 2–64，NOT 为 1。",
            );
    } else throw new Error("不支持的控制方块。");
    return structuredClone(k) as unknown as ControlKind | ResetKind;
}
export interface SimulationEventRecord {
    block: string;
    surface: string;
    kind: "crossing" | "reset";
    direction: -1 | 1;
}
export interface EventPlan {
    signals: Record<string, "double" | "logical">;
    events: {
        block: string;
        surface: string;
        sampleTime: SampleTime;
        locate: boolean;
    }[];
}
export function parseEventRecords(value: unknown): SimulationEventRecord[] {
    if (!Array.isArray(value) || value.length > 512)
        throw new Error("事件记录无效。");
    return value.map((raw) => {
        const e = record(raw);
        if (
            typeof e.block !== "string" ||
            e.block.length > 128 ||
            typeof e.surface !== "string" ||
            e.surface.length > 128 ||
            (e.kind !== "crossing" && e.kind !== "reset") ||
            (e.direction !== 1 && e.direction !== -1)
        )
            throw new Error("事件记录字段无效。");
        return e as unknown as SimulationEventRecord;
    });
}
export function parseEventPlan(value: unknown): EventPlan {
    const p = record(value),
        signals = record(p.signals);
    if (
        Object.keys(signals).length > 10000 ||
        Object.values(signals).some((v) => v !== "double" && v !== "logical") ||
        !Array.isArray(p.events) ||
        p.events.length > 256
    )
        throw new Error("事件计划无效。");
    const events = p.events.map((raw) => {
        const e = record(raw);
        if (
            typeof e.block !== "string" ||
            e.block.length > 128 ||
            typeof e.surface !== "string" ||
            e.surface.length > 128 ||
            typeof e.locate !== "boolean"
        )
            throw new Error("事件定义无效。");
        return {
            block: e.block,
            surface: e.surface,
            locate: e.locate,
            sampleTime: parseSampleTime(e.sampleTime),
        };
    });
    return { signals: signals as EventPlan["signals"], events };
}
