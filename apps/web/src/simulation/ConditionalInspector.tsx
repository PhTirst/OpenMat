import { useEffect, useState } from "react";
import type { AuthoringKind } from "./authoring";
import { defaultConditional } from "./conditional";

type Subsystem = Extract<AuthoringKind, { type: "subsystem" }>;
export function ConditionalInspector({
    kind,
    disabled,
    onChange,
    onError,
    onEnter,
}: {
    kind: Subsystem;
    disabled: boolean;
    onChange(kind: Subsystem): void;
    onError(message: string): void;
    onEnter(): void;
}) {
    const current = kind.execution?.type ?? "virtual";
    const [mode, setMode] = useState<"virtual" | "enabled" | "triggered">(
        current,
    );
    useEffect(() => {
        setMode(current);
    }, [current]);
    const apply = () => {
        try {
            const next = structuredClone(kind);
            if (mode === "virtual") delete next.execution;
            else if (mode !== current) {
                const execution = defaultConditional(mode, kind.outputs);
                if (kind.execution) {
                    execution.period = kind.execution.period;
                    execution.outputs = kind.execution.outputs.map((p) => ({
                        ...p,
                        whenDisabled:
                            mode === "triggered" ? "held" : p.whenDisabled,
                    }));
                }
                next.execution = execution;
            }
            onChange(next);
        } catch (e) {
            onError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <fieldset className="sim-hybrid-fields" disabled={disabled}>
            <legend>子系统</legend>
            <p>
                {kind.inputs} 个数据输入 · {kind.outputs} 个输出
            </p>
            <button onClick={onEnter}>进入子系统</button>
            <label>
                执行方式
                <select
                    aria-label="子系统执行方式"
                    value={mode}
                    onChange={(e) => setMode(e.target.value as typeof mode)}
                >
                    <option value="virtual">普通子系统</option>
                    <option value="enabled">启用 Enabled</option>
                    <option value="triggered">触发 Triggered</option>
                </select>
            </label>
            <button disabled={mode === current} onClick={apply}>
                应用执行设置
            </button>
            {kind.execution ? (
                <p className="sim-help">
                    进入内部，选择{" "}
                    {kind.execution.type === "enabled" ? "Enable" : "Trigger"}{" "}
                    编辑控制参数；选择 Outport 编辑初始输出和禁用策略。
                    当前控制周期 {kind.execution.period}{" "}
                    s。内部仅支持离散状态，连续对象放在外层。
                </p>
            ) : (
                <p className="sim-help">
                    进入内部后可以添加 Enable 或 Trigger 控制端口，以及 Inport /
                    Outport 数据端口。
                </p>
            )}
            <p className="sim-help">
                切换执行方式会移除不再存在的控制连线，可以撤销恢复。
            </p>
        </fieldset>
    );
}
