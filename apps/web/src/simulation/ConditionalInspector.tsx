import { useEffect, useState } from "react";
import { matrixText, parseMatrix, type AuthoringKind } from "./authoring";
import { defaultConditional, parseConditional } from "./conditional";

type Subsystem = Extract<AuthoringKind, { type: "subsystem" }>;
function fields(kind: Subsystem) {
    const e = kind.execution;
    return {
        mode: e?.type ?? "virtual",
        period: String(e?.period ?? 0.1),
        states: e?.type === "enabled" ? e.statesWhenEnabling : "held",
        edge: e?.type === "triggered" ? e.edge : "rising",
        outputs: Array.from({ length: kind.outputs }, (_, i) => ({
            initial: matrixText([e?.outputs[i]?.initial ?? [0]]),
            whenDisabled: e?.outputs[i]?.whenDisabled ?? "held",
        })),
    };
}
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
    const source = JSON.stringify(kind);
    const [draft, setDraft] = useState(() => fields(kind));
    const [message, setMessage] = useState("");
    useEffect(() => {
        setDraft(fields(JSON.parse(source) as Subsystem));
        setMessage("");
    }, [source]);
    const apply = () => {
        try {
            const next: Subsystem = { ...kind };
            if (draft.mode === "virtual") delete next.execution;
            else {
                const type = draft.mode === "enabled" ? "enabled" : "triggered";
                next.execution = parseConditional(
                    {
                        ...defaultConditional(type, kind.outputs),
                        period: Number(draft.period),
                        ...(type === "enabled"
                            ? { statesWhenEnabling: draft.states }
                            : { edge: draft.edge }),
                        outputs: draft.outputs.map((p) => ({
                            initial: parseMatrix(p.initial).flat(),
                            whenDisabled:
                                type === "triggered" ? "held" : p.whenDisabled,
                        })),
                    },
                    kind.outputs,
                );
            }
            onChange(next);
            setMessage("");
        } catch (e) {
            const text = e instanceof Error ? e.message : String(e);
            setMessage(text);
            onError(text);
        }
    };
    return (
        <fieldset className="sim-hybrid-fields">
            <legend>子系统执行</legend>
            <p>
                {kind.inputs} 个数据输入 · {kind.outputs} 个输出
            </p>
            <button onClick={onEnter}>进入子系统</button>
            <label>
                执行方式
                <select
                    aria-label="子系统执行方式"
                    disabled={disabled}
                    value={draft.mode}
                    onChange={(e) =>
                        setDraft({ ...draft, mode: e.target.value })
                    }
                >
                    <option value="virtual">普通子系统</option>
                    <option value="enabled">启用 Enabled</option>
                    <option value="triggered">触发 Triggered</option>
                </select>
            </label>
            {draft.mode !== "virtual" && (
                <>
                    <label>
                        控制采样周期（秒）
                        <input
                            aria-label="控制采样周期"
                            disabled={disabled}
                            value={draft.period}
                            onChange={(e) =>
                                setDraft({ ...draft, period: e.target.value })
                            }
                        />
                    </label>
                    {draft.mode === "enabled" ? (
                        <label>
                            重新启用时的内部状态
                            <select
                                aria-label="重新启用时的内部状态"
                                disabled={disabled}
                                value={draft.states}
                                onChange={(e) =>
                                    setDraft({
                                        ...draft,
                                        states: e.target.value as
                                            "held" | "reset",
                                    })
                                }
                            >
                                <option value="held">保持</option>
                                <option value="reset">恢复初值</option>
                            </select>
                        </label>
                    ) : (
                        <label>
                            触发边沿
                            <select
                                aria-label="触发边沿"
                                disabled={disabled}
                                value={draft.edge}
                                onChange={(e) =>
                                    setDraft({
                                        ...draft,
                                        edge: e.target.value as
                                            "rising" | "falling" | "either",
                                    })
                                }
                            >
                                <option value="rising">上升沿</option>
                                <option value="falling">下降沿</option>
                                <option value="either">任意边沿</option>
                            </select>
                        </label>
                    )}
                    {draft.outputs.map((p, i) => (
                        <div className="sim-conditional-output" key={i}>
                            <label>
                                输出 {i + 1} 初值
                                <input
                                    aria-label={`输出 ${i + 1} 初值`}
                                    disabled={disabled}
                                    value={p.initial}
                                    onChange={(e) =>
                                        setDraft({
                                            ...draft,
                                            outputs: draft.outputs.map(
                                                (v, j) =>
                                                    j === i
                                                        ? {
                                                              ...v,
                                                              initial:
                                                                  e.target
                                                                      .value,
                                                          }
                                                        : v,
                                            ),
                                        })
                                    }
                                />
                            </label>
                            {draft.mode === "enabled" && (
                                <label>
                                    停用时的输出
                                    <select
                                        aria-label={`输出 ${i + 1} 停用方式`}
                                        disabled={disabled}
                                        value={p.whenDisabled}
                                        onChange={(e) =>
                                            setDraft({
                                                ...draft,
                                                outputs: draft.outputs.map(
                                                    (v, j) =>
                                                        j === i
                                                            ? {
                                                                  ...v,
                                                                  whenDisabled:
                                                                      e.target
                                                                          .value as
                                                                          | "held"
                                                                          | "reset",
                                                              }
                                                            : v,
                                                ),
                                            })
                                        }
                                    >
                                        <option value="held">
                                            保持最后一次输出
                                        </option>
                                        <option value="reset">
                                            恢复输出初值
                                        </option>
                                    </select>
                                </label>
                            )}
                        </div>
                    ))}
                    <p className="sim-help">
                        控制端口接收标量信号。内部使用离散状态，连续对象放在外层。触发子系统的内部方块跟随触发执行，初始时刻不触发。
                    </p>
                </>
            )}
            <button disabled={disabled} onClick={apply}>
                应用执行设置
            </button>
            {message && (
                <p role="alert" className="sim-help">
                    {message}
                </p>
            )}
            <p className="sim-help">
                进入内部后添加 Inport / Outport
                可增加数据端口。切换执行方式会移除不再存在的控制端口连线，可撤销恢复。
            </p>
        </fieldset>
    );
}
