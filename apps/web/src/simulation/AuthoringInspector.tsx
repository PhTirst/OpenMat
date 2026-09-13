import { useEffect, useRef, useState } from "react";
import {
    authoringPorts,
    matrixText,
    parseAuthoringKind,
    parseMatrix,
    parseTimeSeriesCsv,
    timeSeriesCsv,
    type AuthoringKind,
} from "./authoring";

export function AuthoringInspector({
    kind,
    parent,
    disabled,
    onChange,
    onError,
    onEnter,
}: {
    kind: AuthoringKind;
    parent?: string | undefined;
    disabled: boolean;
    onChange(k: AuthoringKind): void;
    onError(message: string): void;
    onEnter(): void;
}) {
    const source = JSON.stringify(kind);
    const fields = (k: AuthoringKind): Record<string, string> => {
        if (k.type === "inport")
            return {
                data: k.data ? timeSeriesCsv(k.data) : "time,u1\n0,0\n1,1",
            };
        if (k.type !== "standard") return {};
        const o = k.operation;
        switch (o.type) {
            case "product":
                return { operations: o.operations };
            case "mux":
                return { inputs: String(o.inputs) };
            case "demux":
                return { widths: o.widths.join(", ") };
            case "transferFcn":
                return {
                    numerator: matrixText([o.numerator]),
                    denominator: matrixText([o.denominator]),
                };
            case "stateSpace":
                return {
                    a: matrixText(o.a),
                    b: matrixText(o.b),
                    c: matrixText(o.c),
                    d: matrixText(o.d),
                    initial: matrixText([o.initial]),
                };
        }
    };
    const [draft, setDraft] = useState(() => fields(kind)),
        [message, setMessage] = useState("");
    const file = useRef<HTMLInputElement>(null);
    useEffect(() => {
        setDraft(fields(JSON.parse(source) as AuthoringKind));
        setMessage("");
    }, [source]);
    const apply = () => {
        try {
            let candidate: unknown = kind;
            if (kind.type === "inport")
                candidate = {
                    ...kind,
                    data: parseTimeSeriesCsv(draft.data ?? ""),
                };
            else if (kind.type === "standard") {
                const op = kind.operation;
                switch (op.type) {
                    case "product":
                        candidate = {
                            type: "standard",
                            operation: {
                                ...op,
                                operations: (draft.operations ?? "").replace(
                                    /\s/g,
                                    "",
                                ),
                            },
                        };
                        break;
                    case "mux":
                        candidate = {
                            type: "standard",
                            operation: { ...op, inputs: Number(draft.inputs) },
                        };
                        break;
                    case "demux":
                        candidate = {
                            type: "standard",
                            operation: {
                                ...op,
                                widths: parseMatrix(draft.widths ?? "").flat(),
                            },
                        };
                        break;
                    case "transferFcn":
                        candidate = {
                            type: "standard",
                            operation: {
                                ...op,
                                numerator: parseMatrix(
                                    draft.numerator ?? "",
                                ).flat(),
                                denominator: parseMatrix(
                                    draft.denominator ?? "",
                                ).flat(),
                            },
                        };
                        break;
                    case "stateSpace":
                        candidate = {
                            type: "standard",
                            operation: {
                                type: op.type,
                                a: parseMatrix(draft.a ?? ""),
                                b: parseMatrix(draft.b ?? ""),
                                c: parseMatrix(draft.c ?? ""),
                                d: parseMatrix(draft.d ?? ""),
                                initial: parseMatrix(
                                    draft.initial ?? "",
                                ).flat(),
                            },
                        };
                        break;
                }
            }
            onChange(parseAuthoringKind(candidate));
            setMessage("参数已应用。");
        } catch (e) {
            const message = e instanceof Error ? e.message : String(e);
            setMessage(message);
            onError(message);
        }
    };
    if (kind.type === "subsystem")
        return (
            <fieldset className="sim-hybrid-fields">
                <legend>子系统</legend>
                <p>
                    {kind.inputs} 个输入 · {kind.outputs} 个输出
                </p>
                <button onClick={onEnter}>进入子系统</button>
                <p className="sim-help">
                    进入内部后添加 Inport / Outport
                    可增加边界端口。虚拟子系统在编译时展开，内部状态独立保留。
                </p>
            </fieldset>
        );
    if (kind.type === "outport" || (kind.type === "inport" && parent))
        return (
            <p className="sim-help">
                {kind.type === "inport" ? "输入" : "输出"}端口 {kind.port}。
                {parent
                    ? "端口编号随增删自动整理，与父系统接口对应。"
                    : "根级输出会自动记录到仿真结果，可导入 m 工作区。"}
            </p>
        );
    const labels: Record<string, string> = {
        operations: "乘除顺序",
        inputs: "输入端口数量",
        widths: "输出宽度",
        numerator: "分子系数",
        denominator: "分母系数",
        a: "A 矩阵",
        b: "B 矩阵",
        c: "C 矩阵",
        d: "D 矩阵",
        initial: "初始状态",
        data: "时间序列 CSV",
    };
    return (
        <fieldset className="sim-hybrid-fields" disabled={disabled}>
            <legend>
                {kind.type === "inport" ? "实验输入数据" : "标准方块参数"}
            </legend>
            {Object.entries(draft).map(([key, value]) => (
                <label key={key}>
                    {labels[key]}
                    <textarea
                        aria-label={labels[key]}
                        value={value}
                        rows={key === "data" ? 8 : 2}
                        onChange={(e) =>
                            setDraft((old) => ({
                                ...old,
                                [key]: e.target.value,
                            }))
                        }
                    />
                </label>
            ))}
            {kind.type === "inport" && (
                <>
                    <input
                        hidden
                        ref={file}
                        type="file"
                        accept=".csv,.tsv,text/csv,text/tab-separated-values"
                        onChange={async (e) => {
                            const selected = e.target.files?.[0];
                            e.target.value = "";
                            if (!selected) return;
                            try {
                                if (selected.size > 2 * 1024 * 1024)
                                    throw new Error("CSV 文件超过 2 MiB。");
                                const text = await selected.text();
                                parseTimeSeriesCsv(text);
                                setDraft({ data: text });
                                setMessage(
                                    `已读取 ${selected.name}，点击应用数据。`,
                                );
                            } catch (e) {
                                onError(
                                    e instanceof Error ? e.message : String(e),
                                );
                            }
                        }}
                    />
                    <button onClick={() => file.current?.click()}>
                        导入 CSV 数据
                    </button>
                    <p className="sim-help">
                        第一列为秒，后续列为信号。线性插值，范围外保持端点值。数据随模型保存。
                    </p>
                </>
            )}
            {kind.type === "standard" &&
                kind.operation.type === "stateSpace" && (
                    <p className="sim-help">
                        支持同时修改矩阵尺寸，然后一起应用。A(n,n)、B(n,m)、C(r,n)、D(r,m)，最多
                        32 个状态。
                    </p>
                )}
            {kind.type === "standard" && kind.operation.type === "product" && (
                <p className="sim-help">
                    例如 ** 为相乘，*/ 为第一路除以第二路；支持标量展开。
                </p>
            )}
            <button onClick={apply}>
                {kind.type === "inport" ? "应用数据" : "应用参数"}
            </button>
            <p role="status" className="sim-help">
                {message}
            </p>
            {kind.type === "standard" && (
                <p className="sim-help">
                    {authoringPorts(kind).inputs.length} 个输入 ·{" "}
                    {authoringPorts(kind).outputs.length} 个输出
                </p>
            )}
        </fieldset>
    );
}
