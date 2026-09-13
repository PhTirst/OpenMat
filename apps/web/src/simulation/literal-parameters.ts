import { parseMatrix } from "./authoring";
import { conditionalParent, parameterFields } from "./block-parameters";
import { parseModel, type ModelDocument } from "./model";

/** Preserve offline literal editing. Expressions are evaluated only by the native service. */
export function applyLiteralParameters(
    doc: ModelDocument,
    id: string,
    fields: Record<string, string>,
): ModelDocument {
    const next = structuredClone(doc);
    const block = next.model.blocks.find((b) => b.id === id);
    if (!block) throw new Error("方块已不存在。");
    const definitions = parameterFields(next.model, block);
    for (const [name, text] of Object.entries(fields)) {
        const field = definitions.find((f) => f.name === name);
        if (!field || field.editor === "readonly")
            throw new Error("参数不受支持。");
        if (field.fixed) {
            if (text !== field.fixed) throw new Error("此参数选项尚未支持。");
            continue;
        }
        if (
            field.options &&
            !field.options.some((o) => o.value === text && o.supported)
        )
            throw new Error("此参数选项尚未支持。");
        const path = field.path!;
        const scalar = () => {
            const rows = parseMatrix(text);
            if (rows.length !== 1 || rows[0]!.length !== 1)
                throw new Error("参数需要一个有限实数。");
            return rows[0]![0]!;
        };
        if (path === "sampleTime") {
            let n: number;
            if (["inf", "[inf 0]"].includes(text.trim().toLowerCase()))
                n = Infinity;
            else {
                let rows: number[][];
                try {
                    rows = parseMatrix(text);
                } catch {
                    throw new Error(
                        "模型变量和参数表达式需要连接原生仿真服务。",
                    );
                }
                const sample = rows.flat();
                if (
                    (rows.length !== 1 && rows[0]!.length !== 1) ||
                    sample.length < 1 ||
                    sample.length > 2 ||
                    (sample.length === 2 && sample[1] !== 0)
                )
                    throw new Error(
                        "采样时间需要标量或 [周期 0]，暂不支持非零偏移。",
                    );
                n = sample[0]!;
            }
            if (n < 0 && n !== -1)
                throw new Error("采样时间需要 -1、0、inf 或正数。");
            next.model.sampleTimes ??= {};
            next.model.sampleTimes[id] =
                n === -1
                    ? { kind: "inherited" }
                    : n === 0
                      ? { kind: "continuous" }
                      : n === Infinity
                        ? { kind: "constant" }
                        : { kind: "discrete", period: n };
            if (next.model.schemaVersion < 5) next.model.schemaVersion = 5;
            continue;
        }
        if (path === "signs" || path === "operations") {
            let s = text.replace(/[\s|]/g, "");
            if (/^\d+$/.test(s)) {
                const count = Number(s);
                if (count < (path === "signs" ? 1 : 2) || count > 64)
                    throw new Error("输入数量超出范围。");
                s = (path === "signs" ? "+" : "*").repeat(count);
            }
            if (path === "signs" && block.kind.type === "sum") {
                if (!/^[+-]{1,64}$/.test(s))
                    throw new Error("输入符号需要 + 或 -。");
                block.kind.signs = [...s].map((c) => (c === "+" ? 1 : -1));
            } else if (
                block.kind.type === "standard" &&
                block.kind.operation.type === "product"
            ) {
                if (!/^[*/]{2,64}$/.test(s))
                    throw new Error("输入顺序需要 * 或 /。");
                block.kind.operation.operations = s;
            }
            continue;
        }
        if (path === "externalReset") {
            const k = block.kind;
            if (
                k.type !== "integrator" &&
                k.type !== "discreteIntegrator" &&
                k.type !== "resetIntegrator"
            )
                throw new Error("此方块不支持复位参数。");
            const discrete =
                k.type === "discreteIntegrator" ||
                (k.type === "resetIntegrator" && k.discrete);
            const gain = "gain" in k ? k.gain : 1;
            if (text === "none" && !discrete && gain !== 1)
                throw new Error("请先将连续积分增益设为 1，再移除复位端口。");
            block.kind =
                text === "none"
                    ? discrete
                        ? {
                              type: "discreteIntegrator",
                              initial: k.initial,
                              gain,
                          }
                        : { type: "integrator", initial: k.initial }
                    : {
                          type: "resetIntegrator",
                          discrete,
                          gain,
                          initial: k.initial,
                          reset: text as "rising" | "falling" | "either",
                      };
            if (text !== "none" && next.model.schemaVersion < 6)
                next.model.schemaVersion = 6;
            continue;
        }
        let value: unknown = text;
        if (field.editor === "expression") {
            let matrix: number[][];
            try {
                matrix = parseMatrix(text);
            } catch {
                throw new Error("模型变量和参数表达式需要连接原生仿真服务。");
            }
            if (field.shape === "matrix") value = matrix;
            else if (field.shape === "scalar" || field.shape === "count")
                value = scalar();
            else {
                if (matrix.length > 1 && matrix[0]!.length > 1)
                    throw new Error("此参数需要标量或向量。");
                value = matrix.flat();
            }
        }
        if (path === "initialOutput" || path === "outputWhenDisabled") {
            const parent = conditionalParent(next.model, block);
            if (
                parent?.kind.type !== "subsystem" ||
                !parent.kind.execution ||
                block.kind.type !== "outport"
            )
                throw new Error("输出策略需要条件子系统。");
            const output = parent.kind.execution.outputs[block.kind.port - 1]!;
            if (path === "initialOutput") output.initial = value as number[];
            else {
                if (
                    parent.kind.execution.type === "triggered" &&
                    value !== "held"
                )
                    throw new Error("触发子系统输出需要保持。");
                output.whenDisabled = value as "held" | "reset";
            }
        } else if (
            ["controlPeriod", "statesWhenEnabling", "triggerType"].includes(
                path,
            )
        ) {
            if (block.kind.type !== "subsystem" || !block.kind.execution)
                throw new Error("控制端口不存在。");
            const e = block.kind.execution;
            if (path === "controlPeriod") e.period = value as number;
            else if (path === "statesWhenEnabling" && e.type === "enabled")
                e.statesWhenEnabling = value as "held" | "reset";
            else if (path === "triggerType" && e.type === "triggered")
                e.edge = value as "rising" | "falling" | "either";
        } else {
            const keys = path.split(".");
            let target = block as unknown as Record<string, unknown>;
            for (const key of keys.slice(0, -1))
                target = target[key] as Record<string, unknown>;
            target[keys.at(-1)!] = value;
        }
    }
    const k = block.kind;
    if (
        k.type === "standard" &&
        k.operation.type === "stateSpace" &&
        k.operation.initial.length === 1
    )
        k.operation.initial = Array(k.operation.a.length).fill(
            k.operation.initial[0],
        ) as number[];
    next.model = parseModel(next.model);
    return next;
}
