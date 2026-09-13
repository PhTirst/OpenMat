import { useEffect, useRef, useState } from "react";
import { numericLiteral } from "./model";
import {
    CALLBACK_ROLES,
    validateComponent,
    type CallbackRole,
    type ComponentDefinition,
    type ComponentIcon,
    type ComponentParameter,
} from "./components";

function newCallback(id: string, role: CallbackRole) {
    const entry = `c_${id.replace(/[^A-Za-z0-9_]/g, "_").slice(0, 40)}_${role}`;
    return { source: `${entry}.m`, entry };
}

export function blankComponent(id: string): ComponentDefinition {
    return {
        id,
        name: "My Component",
        category: "我的组件",
        icon: "function",
        inputs: [{ name: "u", width: 1 }],
        outputs: [{ name: "y", width: 1 }],
        parameters: [],
        continuousStates: 0,
        discreteStates: 0,
        outputsFunction: newCallback(id, "outputsFunction"),
    };
}
export function ComponentDialog({
    value,
    allowInherited = false,
    onApply,
    onClose,
}: {
    value: ComponentDefinition;
    allowInherited?: boolean;
    onApply(d: ComponentDefinition): void;
    onClose(): void;
}) {
    const [draft, setDraft] = useState(() => structuredClone(value));
    const [inputs, setInputs] = useState(
        value.inputs.map((p) => `${p.name}: ${p.width}`).join("\n"),
    );
    const [outputs, setOutputs] = useState(
        value.outputs.map((p) => `${p.name}: ${p.width}`).join("\n"),
    );
    const [parameters, setParameters] = useState(
        value.parameters.map((p) => ({ ...p, literal: p.value.join("; ") })),
    );
    const [error, setError] = useState<string | null>(null);
    const form = useRef<HTMLFormElement>(null);
    useEffect(() => {
        const previous = document.activeElement;
        form.current?.querySelector<HTMLInputElement>("input")?.focus();
        return () => {
            if (previous instanceof HTMLElement && previous.isConnected)
                previous.focus();
        };
    }, []);
    const apply = () => {
        try {
            const ports = (text: string) =>
                text
                    .split("\n")
                    .filter((l) => l.trim())
                    .map((l) => {
                        const [name, width, ...rest] = l.split(":");
                        if (rest.length)
                            throw new Error("端口每行使用 名称: 宽度。");
                        return { name: name!.trim(), width: Number(width) };
                    });
            const next: ComponentDefinition = {
                ...draft,
                inputs: ports(inputs),
                outputs: ports(outputs),
                parameters: parameters.map(({ literal, ...p }) => ({
                    ...p,
                    value: numericLiteral(literal),
                })),
            };
            const enabled = {
                initialize: next.continuousStates + next.discreteStates > 0,
                outputsFunction: true,
                derivatives: next.continuousStates > 0,
                update: next.discreteStates > 0,
            };
            for (const role of CALLBACK_ROLES) {
                if (!enabled[role]) {
                    if (role !== "outputsFunction") delete next[role];
                } else next[role] ??= newCallback(next.id, role);
            }
            if (!next.discreteStates) delete next.sampleTime;
            onApply(validateComponent(next, allowInherited));
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const parameter = (
        index: number,
        change: Partial<ComponentParameter & { literal: string }>,
    ) =>
        setParameters((p) =>
            p.map((v, i) => (i === index ? { ...v, ...change } : v)),
        );
    return (
        <div
            className="sim-dialog-shade"
            onKeyDown={(e) => {
                if (e.key === "Escape") {
                    e.stopPropagation();
                    onClose();
                }
                if (e.key === "Tab") {
                    const nodes = Array.from(
                        form.current?.querySelectorAll<HTMLElement>(
                            "input,textarea,select,button",
                        ) ?? [],
                    );
                    const first = nodes[0],
                        last = nodes.at(-1);
                    if (e.shiftKey && document.activeElement === first) {
                        e.preventDefault();
                        last?.focus();
                    } else if (!e.shiftKey && document.activeElement === last) {
                        e.preventDefault();
                        first?.focus();
                    }
                }
            }}
        >
            <form
                ref={form}
                className="sim-component-dialog"
                role="dialog"
                aria-modal="true"
                aria-label="组件定义"
                onSubmit={(e) => {
                    e.preventDefault();
                    apply();
                }}
            >
                <h2>组件定义</h2>
                <p className="sim-help">
                    定义端口、参数和状态，随后在 m
                    编辑器里编写回调。更改定义会作用于当前模型的所有同类实例。
                </p>
                <div className="sim-component-grid">
                    <label>
                        名称
                        <input
                            aria-label="组件名称"
                            value={draft.name}
                            onChange={(e) =>
                                setDraft({ ...draft, name: e.target.value })
                            }
                        />
                    </label>
                    <label>
                        分类
                        <input
                            aria-label="组件分类"
                            value={draft.category}
                            onChange={(e) =>
                                setDraft({ ...draft, category: e.target.value })
                            }
                        />
                    </label>
                    <label>
                        图标
                        <select
                            aria-label="组件图标"
                            value={draft.icon}
                            onChange={(e) =>
                                setDraft({
                                    ...draft,
                                    icon: e.target.value as ComponentIcon,
                                })
                            }
                        >
                            <option value="function">函数</option>
                            <option value="plant">连续系统</option>
                            <option value="controller">控制器</option>
                            <option value="filter">滤波器</option>
                            <option value="delay">延迟</option>
                        </select>
                    </label>
                    <label>
                        连续状态数
                        <input
                            aria-label="连续状态数"
                            type="number"
                            min={0}
                            max={4096}
                            value={draft.continuousStates}
                            onChange={(e) =>
                                setDraft({
                                    ...draft,
                                    continuousStates: Number(e.target.value),
                                })
                            }
                        />
                    </label>
                    <label>
                        离散状态数
                        <input
                            aria-label="离散状态数"
                            type="number"
                            min={0}
                            max={4096}
                            value={draft.discreteStates}
                            onChange={(e) =>
                                setDraft({
                                    ...draft,
                                    discreteStates: Number(e.target.value),
                                    sampleTime: draft.sampleTime ?? 0.1,
                                })
                            }
                        />
                    </label>
                    {draft.discreteStates > 0 && (
                        <label>
                            采样周期 (s)
                            {allowInherited && draft.continuousStates === 0
                                ? "，-1 为继承"
                                : ""}
                            <input
                                aria-label="组件采样周期"
                                type="number"
                                min={
                                    allowInherited &&
                                    draft.continuousStates === 0
                                        ? -1
                                        : 0
                                }
                                step="any"
                                value={draft.sampleTime ?? 0.1}
                                onChange={(e) =>
                                    setDraft({
                                        ...draft,
                                        sampleTime: Number(e.target.value),
                                    })
                                }
                            />
                        </label>
                    )}
                    <label>
                        输入 · 每行 名称: 宽度
                        <textarea
                            aria-label="组件输入端口"
                            rows={3}
                            value={inputs}
                            onChange={(e) => setInputs(e.target.value)}
                        />
                    </label>
                    <label>
                        输出 · 每行 名称: 宽度
                        <textarea
                            aria-label="组件输出端口"
                            rows={3}
                            value={outputs}
                            onChange={(e) => setOutputs(e.target.value)}
                        />
                    </label>
                </div>
                <h3>公开参数</h3>
                <p className="sim-help">
                    参数按此顺序拼成列向量 p，数组元素采用 MATLAB 的列顺序。
                </p>
                {parameters.map((p, i) => (
                    <fieldset className="sim-component-parameter" key={i}>
                        <legend>参数 {i + 1}</legend>
                        <label>
                            变量名
                            <input
                                aria-label={`参数 ${i + 1} 名称`}
                                value={p.name}
                                onChange={(e) =>
                                    parameter(i, { name: e.target.value })
                                }
                            />
                        </label>
                        <label>
                            显示名
                            <input
                                aria-label={`参数 ${i + 1} 显示名`}
                                value={p.label ?? ""}
                                onChange={(e) =>
                                    parameter(i, { label: e.target.value })
                                }
                            />
                        </label>
                        <label>
                            默认值
                            <input
                                aria-label={`参数 ${i + 1} 默认值`}
                                value={p.literal}
                                onChange={(e) =>
                                    parameter(i, { literal: e.target.value })
                                }
                            />
                        </label>
                        <label>
                            单位
                            <input
                                aria-label={`参数 ${i + 1} 单位`}
                                value={p.unit ?? ""}
                                onChange={(e) =>
                                    parameter(i, { unit: e.target.value })
                                }
                            />
                        </label>
                        <label>
                            下限
                            <input
                                aria-label={`参数 ${i + 1} 下限`}
                                type="number"
                                step="any"
                                value={p.minimum ?? ""}
                                onChange={(e) =>
                                    setParameters((ps) =>
                                        ps.map((v, j) => {
                                            if (i !== j) return v;
                                            const n = { ...v };
                                            if (e.target.value === "")
                                                delete n.minimum;
                                            else
                                                n.minimum = Number(
                                                    e.target.value,
                                                );
                                            return n;
                                        }),
                                    )
                                }
                            />
                        </label>
                        <label>
                            上限
                            <input
                                aria-label={`参数 ${i + 1} 上限`}
                                type="number"
                                step="any"
                                value={p.maximum ?? ""}
                                onChange={(e) =>
                                    setParameters((ps) =>
                                        ps.map((v, j) => {
                                            if (i !== j) return v;
                                            const n = { ...v };
                                            if (e.target.value === "")
                                                delete n.maximum;
                                            else
                                                n.maximum = Number(
                                                    e.target.value,
                                                );
                                            return n;
                                        }),
                                    )
                                }
                            />
                        </label>
                        <button
                            type="button"
                            onClick={() =>
                                setParameters((ps) =>
                                    ps.filter((_, j) => j !== i),
                                )
                            }
                        >
                            移除参数 {i + 1}
                        </button>
                    </fieldset>
                ))}
                <button
                    type="button"
                    onClick={() =>
                        setParameters((p) => [
                            ...p,
                            {
                                name: `parameter${p.length + 1}`,
                                value: [1],
                                literal: "1",
                            },
                        ])
                    }
                >
                    添加参数
                </button>
                {error && (
                    <p role="alert" className="sim-banner error">
                        {error}
                    </p>
                )}
                <div className="sim-dialog-actions">
                    <button type="button" onClick={onClose}>
                        取消
                    </button>
                    <button type="submit" className="primary">
                        应用定义
                    </button>
                </div>
            </form>
        </div>
    );
}
