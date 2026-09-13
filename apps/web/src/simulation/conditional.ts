export interface ConditionalOutput {
    initial: number[];
    whenDisabled: "held" | "reset";
}
export type ConditionalExecution =
    | {
          type: "enabled";
          period: number;
          statesWhenEnabling: "held" | "reset";
          outputs: ConditionalOutput[];
      }
    | {
          type: "triggered";
          period: number;
          edge: "rising" | "falling" | "either";
          outputs: ConditionalOutput[];
      };
export interface ExecutionEvent {
    block: string;
    kind: "enabled" | "disabled" | "triggered" | "statesReset";
}
export const EXECUTION_LABELS = {
    enabled: "启用",
    disabled: "停用",
    triggered: "触发",
    statesReset: "状态复位",
};
export const initialOutput = (): ConditionalOutput => ({
    initial: [0],
    whenDisabled: "held",
});
export function defaultConditional(
    type: "enabled" | "triggered",
    outputs: number,
): ConditionalExecution {
    const common = {
        period: 0.1,
        outputs: Array.from({ length: outputs }, initialOutput),
    };
    return type === "enabled"
        ? { type, ...common, statesWhenEnabling: "held" }
        : { type, ...common, edge: "rising" };
}
export function parseConditional(
    raw: unknown,
    count: number,
): ConditionalExecution {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
        throw new Error("条件执行设置无效。");
    const e = raw as Record<string, unknown>;
    const allowed =
        e.type === "enabled"
            ? ["type", "period", "statesWhenEnabling", "outputs"]
            : ["type", "period", "edge", "outputs"];
    if (
        Object.keys(e).some((k) => !allowed.includes(k)) ||
        (e.type !== "enabled" && e.type !== "triggered") ||
        typeof e.period !== "number" ||
        !Number.isFinite(e.period) ||
        e.period <= 0 ||
        (e.type === "enabled"
            ? !["held", "reset"].includes(String(e.statesWhenEnabling))
            : !["rising", "falling", "either"].includes(String(e.edge))) ||
        !Array.isArray(e.outputs) ||
        e.outputs.length !== count
    )
        throw new Error(
            "条件执行需要正采样周期，并为每个输出配置初值与保持方式。",
        );
    for (const raw of e.outputs) {
        if (!raw || typeof raw !== "object" || Array.isArray(raw))
            throw new Error("输出策略无效。");
        const p = raw as Record<string, unknown>;
        if (
            Object.keys(p).some(
                (k) => !["initial", "whenDisabled"].includes(k),
            ) ||
            !Array.isArray(p.initial) ||
            p.initial.length < 1 ||
            p.initial.length > 4096 ||
            p.initial.some(
                (v) => typeof v !== "number" || !Number.isFinite(v),
            ) ||
            !["held", "reset"].includes(String(p.whenDisabled)) ||
            (e.type === "triggered" && p.whenDisabled !== "held")
        )
            throw new Error("输出初值需要有限实数；触发子系统的输出必须保持。");
    }
    return structuredClone(e) as unknown as ConditionalExecution;
}
export function parseExecutionEvents(raw: unknown): ExecutionEvent[] {
    if (raw === undefined) return [];
    if (
        !Array.isArray(raw) ||
        raw.length > 128 ||
        raw.some(
            (e) =>
                !e ||
                typeof e !== "object" ||
                typeof e.block !== "string" ||
                !Object.hasOwn(EXECUTION_LABELS, e.kind),
        )
    )
        throw new Error("条件执行事件数据无效。");
    return raw as ExecutionEvent[];
}
