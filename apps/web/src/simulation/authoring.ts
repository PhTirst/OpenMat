import { parseConditional, type ConditionalExecution } from "./conditional";
export interface TimeSeries {
    times: number[];
    values: number[][];
}
export type StandardOperation =
    | { type: "product"; operations: string }
    | { type: "mux"; inputs: number }
    | { type: "demux"; widths: number[] }
    | {
          type: "stateSpace";
          a: number[][];
          b: number[][];
          c: number[][];
          d: number[][];
          initial: number[];
      }
    | { type: "transferFcn"; numerator: number[]; denominator: number[] };
export type AuthoringKind =
    | { type: "standard"; operation: StandardOperation }
    | {
          type: "subsystem";
          inputs: number;
          outputs: number;
          execution?: ConditionalExecution;
      }
    | { type: "inport"; port: number; data?: TimeSeries }
    | { type: "outport"; port: number };
export const STANDARD_LABELS = {
    product: "Product",
    mux: "Mux",
    demux: "Demux",
    stateSpace: "State-Space",
    transferFcn: "Transfer Fcn",
};
export const STANDARD_PRESETS: {
    id: StandardOperation["type"];
    label: string;
    kind: AuthoringKind;
}[] = [
    {
        id: "product",
        label: "Product",
        kind: {
            type: "standard",
            operation: { type: "product", operations: "**" },
        },
    },
    {
        id: "mux",
        label: "Mux",
        kind: { type: "standard", operation: { type: "mux", inputs: 2 } },
    },
    {
        id: "demux",
        label: "Demux",
        kind: {
            type: "standard",
            operation: { type: "demux", widths: [1, 1] },
        },
    },
    {
        id: "stateSpace",
        label: "State-Space",
        kind: {
            type: "standard",
            operation: {
                type: "stateSpace",
                a: [[1]],
                b: [[1]],
                c: [[1]],
                d: [[1]],
                initial: [0],
            },
        },
    },
    {
        id: "transferFcn",
        label: "Transfer Fcn",
        kind: {
            type: "standard",
            operation: {
                type: "transferFcn",
                numerator: [1],
                denominator: [1, 1],
            },
        },
    },
];
const finite = (v: unknown): v is number =>
    typeof v === "number" && Number.isFinite(v);
const count = (v: unknown, min: number, max: number): v is number =>
    finite(v) && Number.isInteger(v) && v >= min && v <= max;
const vector = (v: unknown, max = 4096): v is number[] =>
    Array.isArray(v) && v.length > 0 && v.length <= max && v.every(finite);
function object(v: unknown): Record<string, unknown> {
    if (!v || typeof v !== "object" || Array.isArray(v))
        throw new Error("方块参数需要对象。");
    return v as Record<string, unknown>;
}
function keys(v: Record<string, unknown>, allowed: string[]) {
    if (Object.keys(v).some((k) => !allowed.includes(k)))
        throw new Error("方块包含未知参数。");
}
export function parseTimeSeries(raw: unknown): TimeSeries {
    const d = object(raw);
    keys(d, ["times", "values"]);
    if (
        !vector(d.times, 4096) ||
        d.times.some((t, i, all) => t < 0 || (i > 0 && t <= all[i - 1]!)) ||
        !Array.isArray(d.values) ||
        d.values.length !== d.times.length
    )
        throw new Error("输入数据需要严格递增的非负时间，最多 4096 行。");
    const times = d.times,
        values = d.values;
    if (
        !values.every((v) => vector(v, 64)) ||
        values.some((v) => v.length !== values[0]!.length) ||
        values.length * values[0]!.length > 65536
    )
        throw new Error(
            "输入数据需要等宽有限数值，每行 1–64 个通道，总计最多 65536 个值。",
        );
    const rows = values as number[][];
    for (let i = 1; i < times.length; i++)
        for (let c = 0; c < rows[i]!.length; c++)
            if (
                !Number.isFinite(
                    (rows[i]![c]! - rows[i - 1]![c]!) /
                        (times[i]! - times[i - 1]!),
                )
            )
                throw new Error("输入数据插值斜率超出有限数值范围。");
    return structuredClone({ times, values: rows });
}
export function parseAuthoringKind(raw: unknown): AuthoringKind {
    const k = object(raw),
        bad = () => new Error("方块参数的类型或尺寸无效。");
    if (k.type === "subsystem") {
        keys(k, ["type", "inputs", "outputs", "execution"]);
        if (!count(k.inputs, 0, 64) || !count(k.outputs, 0, 64)) throw bad();
        if (k.execution !== undefined) parseConditional(k.execution, k.outputs);
    } else if (k.type === "inport" || k.type === "outport") {
        keys(
            k,
            k.type === "inport" ? ["type", "port", "data"] : ["type", "port"],
        );
        if (!count(k.port, 1, 64)) throw bad();
        if (k.data !== undefined) parseTimeSeries(k.data);
    } else if (k.type === "standard") {
        keys(k, ["type", "operation"]);
        const o = object(k.operation);
        switch (o.type) {
            case "product":
                keys(o, ["type", "operations"]);
                if (
                    typeof o.operations !== "string" ||
                    !/^[*/]{2,64}$/.test(o.operations)
                )
                    throw bad();
                break;
            case "mux":
                keys(o, ["type", "inputs"]);
                if (!count(o.inputs, 2, 64)) throw bad();
                break;
            case "demux":
                keys(o, ["type", "widths"]);
                if (
                    !Array.isArray(o.widths) ||
                    !count(o.widths.length, 2, 64) ||
                    !o.widths.every((n) => count(n, 1, 4096)) ||
                    o.widths.reduce((s, n) => s + n, 0) > 4096
                )
                    throw bad();
                break;
            case "transferFcn": {
                keys(o, ["type", "numerator", "denominator"]);
                if (
                    !vector(o.numerator, 33) ||
                    !vector(o.denominator, 33) ||
                    o.numerator.length > o.denominator.length ||
                    o.denominator[0] === 0
                )
                    throw bad();
                const leading = o.denominator[0]!;
                if (
                    [...o.numerator, ...o.denominator].some(
                        (v) => !Number.isFinite(v / leading),
                    )
                )
                    throw bad();
                const den = o.denominator.map((v) => v / leading);
                const num = Array<number>(den.length - o.numerator.length)
                    .fill(0)
                    .concat(o.numerator.map((v) => v / leading));
                if (
                    den.some(
                        (v, i) =>
                            i > 0 && !Number.isFinite(num[i]! - num[0]! * v),
                    )
                )
                    throw bad();
                break;
            }
            case "stateSpace": {
                keys(o, ["type", "a", "b", "c", "d", "initial"]);
                const matrix = (v: unknown): v is number[][] =>
                    Array.isArray(v) &&
                    count(v.length, 1, 32) &&
                    v.every((row) => vector(row, 32)) &&
                    v.every((row) => row.length === v[0]!.length);
                if (
                    !matrix(o.a) ||
                    !matrix(o.b) ||
                    !matrix(o.c) ||
                    !matrix(o.d) ||
                    !vector(o.initial, 32)
                )
                    throw bad();
                const n = o.a.length,
                    m = o.b[0]!.length,
                    r = o.c.length;
                if (
                    o.a[0]!.length !== n ||
                    o.b.length !== n ||
                    o.c[0]!.length !== n ||
                    o.d.length !== r ||
                    o.d[0]!.length !== m ||
                    o.initial.length !== n
                )
                    throw new Error(
                        "矩阵需要 A(n,n)、B(n,m)、C(r,n)、D(r,m)，初始状态为 n 个值。",
                    );
                break;
            }
            default:
                throw bad();
        }
    } else throw bad();
    return structuredClone(k) as AuthoringKind;
}
export function authoringPorts(k: AuthoringKind): {
    inputs: string[];
    outputs: string[];
} {
    const names = (n: number, prefix: string, first = 0) =>
        Array.from({ length: n }, (_, i) => `${prefix}${i + first}`);
    if (k.type === "subsystem")
        return {
            inputs: [
                ...names(k.inputs, "in", 1),
                ...(k.execution
                    ? [k.execution.type === "enabled" ? "enable" : "trigger"]
                    : []),
            ],
            outputs: names(k.outputs, "out", 1),
        };
    if (k.type === "inport") return { inputs: [], outputs: ["out"] };
    if (k.type === "outport") return { inputs: ["in"], outputs: [] };
    const o = k.operation;
    return {
        inputs:
            o.type === "product"
                ? names(o.operations.length, "in")
                : o.type === "mux"
                  ? names(o.inputs, "in")
                  : ["in"],
        outputs: o.type === "demux" ? names(o.widths.length, "out") : ["out"],
    };
}
export function parseMatrix(text: string): number[][] {
    const raw = text.trim();
    if (raw.startsWith("[") !== raw.endsWith("]"))
        throw new Error("矩阵括号不匹配。");
    const body = raw.startsWith("[") ? raw.slice(1, -1) : raw;
    const rows = body
        .split(/[;\r\n]+/)
        .filter((s) => s.trim())
        .map((row) =>
            row
                .trim()
                .split(/[\s,]+/)
                .map((cell) => {
                    if (
                        !/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(
                            cell,
                        ) ||
                        !Number.isFinite(Number(cell))
                    )
                        throw new Error(
                            "这里只接受有限数值矩阵，例如 [-1 0; 0 -2]。",
                        );
                    return Number(cell);
                }),
        );
    if (!rows.length || rows.some((row) => row.length !== rows[0]!.length))
        throw new Error("矩阵各行长度必须相同。");
    return rows;
}
export const matrixText = (m: number[][]) =>
    `[${m.map((r) => r.join(" ")).join("; ")}]`;
export const timeSeriesCsv = (d: TimeSeries) =>
    [
        "time," + d.values[0]!.map((_, i) => `u${i + 1}`).join(","),
        ...d.times.map((t, i) => [t, ...d.values[i]!].join(",")),
    ].join("\n");
export function parseTimeSeriesCsv(text: string): TimeSeries {
    if (new TextEncoder().encode(text).length > 2 * 1024 * 1024)
        throw new Error("CSV 文件超过 2 MiB。");
    const lines = text
        .replace(/^\uFEFF/, "")
        .trim()
        .split(/\r?\n/)
        .filter((line) => line.trim());
    if (
        lines.length &&
        /^(?:time(?:\s*\(s\))?|t|时间)\s*[,\t]/i.test(lines[0]!)
    )
        lines.shift();
    const rows = lines.map((line) =>
        line
            .trim()
            .split(/[,\t]/)
            .map((cell) => {
                const t = cell.trim();
                if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(t))
                    throw new Error(
                        "CSV 需要 time,u1,u2… 列，数据部分只能包含数值。",
                    );
                return Number(t);
            }),
    );
    return parseTimeSeries({
        times: rows.map((r) => r[0]),
        values: rows.map((r) => r.slice(1)),
    });
}
