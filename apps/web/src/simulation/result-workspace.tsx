import { useState } from "react";
import type { ScopeInfo, SimulationFrame } from "./client";

const RESERVED = new Set(
    "break case catch classdef continue else elseif end for function global if otherwise parfor persistent return spmd switch try while properties methods events enumeration arguments".split(
        " ",
    ),
);
export function resultMatrixCode(
    name: string,
    frames: readonly SimulationFrame[],
    scope: ScopeInfo,
): string {
    if (!/^[A-Za-z][A-Za-z0-9_]{0,62}$/.test(name) || RESERVED.has(name))
        throw new Error(
            "变量名须以字母开头，只包含字母、数字和下划线，最多 63 个字符。",
        );
    if (!frames.length) throw new Error("没有可以导入的记录点。");
    const parts = [`${name} = [\n`];
    let size = parts[0]!.length;
    for (const frame of frames) {
        const values = frame.values.slice(
            scope.offset,
            scope.offset + scope.width,
        );
        if (
            values.length !== scope.width ||
            ![frame.time, ...values].every(Number.isFinite)
        )
            throw new Error("仿真结果包含无效数值。");
        const line = [frame.time, ...values].map(String).join(" ") + ";\n";
        size += line.length;
        if (size > 512 * 1024)
            throw new Error(
                "结果超过本次工作区导入上限（512 KiB 数值文本），请使用 CSV 导出。",
            );
        parts.push(line);
    }
    parts.push("];\n");
    return parts.join("");
}
export function ResultWorkspaceExport({
    frames,
    scope,
    disabled,
    onExecute,
}: {
    frames: readonly SimulationFrame[];
    scope: ScopeInfo | undefined;
    disabled: boolean;
    onExecute(code: string): Promise<boolean>;
}) {
    const [name, setName] = useState("simout"),
        [busy, setBusy] = useState(false),
        [message, setMessage] = useState("");
    return (
        <div className="sim-workspace-export">
            <label>
                m 变量
                <input
                    aria-label="仿真结果变量名"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    disabled={busy}
                />
            </label>
            <button
                disabled={disabled || busy || !scope || !frames.length}
                onClick={async () => {
                    if (!scope) return;
                    setBusy(true);
                    setMessage("");
                    try {
                        const code = resultMatrixCode(name, frames, scope);
                        if (!(await onExecute(code)))
                            throw new Error(
                                "未写入工作区，请检查 kernel 状态和命令窗口。",
                            );
                        setMessage(
                            `已写入 ${name}：${frames.length} 行，首列为时间，后续为信号通道。`,
                        );
                    } catch (e) {
                        setMessage(e instanceof Error ? e.message : String(e));
                    } finally {
                        setBusy(false);
                    }
                }}
            >
                {busy ? "正在写入…" : "写入 m 工作区"}
            </button>
            <span role="status">{message}</span>
        </div>
    );
}
