import { useState } from "react";
import { numericLiteral } from "./model";
import {
    CALLBACK_LABELS,
    CALLBACK_ROLES,
    validateComponentKind,
    type Callback,
    type ComponentDefinition,
    type ComponentKind,
} from "./components";

export function ComponentInspector({
    value,
    definition,
    disabled,
    embedded = false,
    onChange,
    onOpen,
    onEdit,
    onLibrary,
    onError,
}: {
    value: ComponentKind;
    definition: ComponentDefinition;
    disabled: boolean;
    embedded?: boolean;
    onChange(value: ComponentKind): void;
    onOpen(callback: Callback): void;
    onEdit(): void;
    onLibrary(): void;
    onError(message: string): void;
}) {
    return (
        <div className="sim-component-inspector">
            <p className="sim-help">
                {definition.continuousStates} 个连续状态 ·{" "}
                {definition.discreteStates} 个离散状态
                {definition.sampleTime
                    ? ` · Ts = ${definition.sampleTime} s`
                    : ""}
                。每个实例独立保存状态。
            </p>
            <h3>实例参数</h3>
            {definition.parameters.map((p) => (
                <ParameterInput
                    key={`${p.name}:${JSON.stringify(Object.hasOwn(value.parameters, p.name) ? value.parameters[p.name] : p.value)}`}
                    name={p.label || p.name}
                    unit={p.unit ?? ""}
                    values={
                        Object.hasOwn(value.parameters, p.name)
                            ? value.parameters[p.name]!
                            : p.value
                    }
                    disabled={disabled}
                    onInvalid={onError}
                    onCommit={(values) => {
                        const next = validateComponentKind(
                            {
                                ...value,
                                parameters: {
                                    ...value.parameters,
                                    [p.name]: values,
                                },
                            },
                            [definition],
                        );
                        onChange(next);
                    }}
                />
            ))}
            {!definition.parameters.length && (
                <p className="sim-help">这个组件没有公开参数。</p>
            )}
            <h3>M 回调函数</h3>
            {CALLBACK_ROLES.map((role) => {
                const callback = definition[role];
                return callback ? (
                    <button
                        className="sim-callback-button"
                        key={role}
                        onClick={() => onOpen(callback)}
                    >
                        <span>{CALLBACK_LABELS[role]}</span>
                        <small title={callback.source}>
                            {callback.entry}.m ↗
                        </small>
                    </button>
                ) : null;
            })}
            <p className="sim-help">
                输出和导数用于连续求值；update
                只在采样时刻执行，其结果在下一采样时刻成为可见状态。
            </p>
            {embedded && (
                <p className="sim-help">
                    此方块由 SLX 参数生成。请通过原始 SLX
                    视图修改参数；暂不支持单独导出到组件库。
                </p>
            )}
            <button disabled={disabled || embedded} onClick={onEdit}>
                编辑组件定义…
            </button>
            <button disabled={disabled || embedded} onClick={onLibrary}>
                保存到项目组件库
            </button>
        </div>
    );
}
function ParameterInput({
    name,
    unit,
    values,
    disabled,
    onCommit,
    onInvalid,
}: {
    name: string;
    unit: string;
    values: number[];
    disabled: boolean;
    onCommit(v: number[]): void;
    onInvalid(message: string): void;
}) {
    const [draft, setDraft] = useState(values.join("; "));
    const [error, setError] = useState<string | null>(null);
    const commit = () => {
        try {
            onCommit(numericLiteral(draft));
            setError(null);
        } catch (e) {
            const message = e instanceof Error ? e.message : String(e);
            setError(message);
            onInvalid(message);
        }
    };
    return (
        <label>
            <span>
                {name}
                {unit ? ` (${unit})` : ""}
            </span>
            <input
                aria-label={`组件参数 ${name}`}
                value={draft}
                disabled={disabled}
                onChange={(e) => setDraft(e.target.value)}
                onBlur={commit}
                onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                }}
            />
            {error && <small role="alert">{error}</small>}
        </label>
    );
}
