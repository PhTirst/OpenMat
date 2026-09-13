import { useEffect, useState } from "react";
import type { Block, ModelDocument } from "./model";
import {
    blockDescriptor,
    conditionalParent,
    PARAMETER_CATALOG,
    parameterDraft,
    parameterFields,
    parameterValue,
} from "./block-parameters";

export function BlockParameterForm({
    doc,
    block,
    disabled,
    onApply,
    onPending,
}: {
    doc: ModelDocument;
    block: Block;
    disabled: boolean;
    onApply(fields: Record<string, string>): Promise<void>;
    onPending?(pending: boolean): void;
}) {
    const original = parameterDraft(doc, block);
    const revision = JSON.stringify(original);
    const [draft, setDraft] = useState(original);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState("");
    const fields = parameterFields(doc.model, block);
    useEffect(() => {
        setDraft(JSON.parse(revision) as Record<string, string>);
        setError("");
    }, [revision]);
    const dirty = JSON.stringify(draft) !== revision;
    useEffect(() => {
        onPending?.(dirty);
        return () => onPending?.(false);
    }, [dirty, onPending]);
    const apply = async () => {
        setBusy(true);
        setError("");
        try {
            await onApply(
                Object.fromEntries(
                    Object.entries(draft).filter(
                        ([name, value]) => value !== original[name],
                    ),
                ),
            );
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };
    const parent = conditionalParent(doc.model, block);
    const triggered =
        parent?.kind.type === "subsystem" &&
        parent.kind.execution?.type === "triggered";
    return (
        <div
            className="sim-parameter-form"
            aria-label={`${blockDescriptor(block)?.label ?? "方块"} 参数`}
        >
            {Object.entries(PARAMETER_CATALOG.groups).map(([group, title]) => {
                const visible = fields.filter((f) => f.group === group);
                if (!visible.length) return null;
                return (
                    <fieldset key={group} disabled={disabled || busy}>
                        <legend>{title}</legend>
                        {visible.map((field) => (
                            <label
                                key={field.name}
                                className="sim-parameter-field"
                            >
                                <span>
                                    {field.label}{" "}
                                    <small>
                                        {field.extension ? "OpenMat · " : ""}
                                        {field.name}
                                    </small>
                                </span>
                                {field.editor === "choice" ? (
                                    <select
                                        aria-label={field.label}
                                        value={draft[field.name] ?? ""}
                                        onChange={(e) =>
                                            setDraft((p) => ({
                                                ...p,
                                                [field.name]: e.target.value,
                                            }))
                                        }
                                    >
                                        {field.options?.map((o) => {
                                            const supported =
                                                o.supported &&
                                                !(
                                                    triggered &&
                                                    field.name ===
                                                        "OutputWhenDisabled" &&
                                                    o.value !== "held"
                                                );
                                            return (
                                                <option
                                                    key={o.value}
                                                    value={o.value}
                                                    disabled={!supported}
                                                >
                                                    {o.value}
                                                    {supported
                                                        ? ""
                                                        : "（暂不支持）"}
                                                </option>
                                            );
                                        })}
                                    </select>
                                ) : field.editor === "readonly" ? (
                                    <output>{original[field.name]}</output>
                                ) : (
                                    <textarea
                                        aria-label={field.label}
                                        rows={field.shape === "matrix" ? 3 : 1}
                                        spellCheck={false}
                                        value={draft[field.name] ?? ""}
                                        className={
                                            draft[field.name] !==
                                            original[field.name]
                                                ? "is-modified"
                                                : undefined
                                        }
                                        onChange={(e) =>
                                            setDraft((p) => ({
                                                ...p,
                                                [field.name]: e.target.value,
                                            }))
                                        }
                                        onKeyDown={(e) => {
                                            if (
                                                e.key === "Enter" &&
                                                !e.shiftKey &&
                                                field.shape !== "matrix"
                                            ) {
                                                e.preventDefault();
                                                void apply();
                                            }
                                        }}
                                    />
                                )}
                                {field.help && (
                                    <span className="sim-help">
                                        {field.help}
                                    </span>
                                )}
                                {field.editor === "expression" &&
                                    doc.parameters?.bindings[block.id]?.[
                                        field.name
                                    ] !== undefined && (
                                        <span className="sim-parameter-resolved">
                                            已解析：
                                            {parameterValue(
                                                doc.model,
                                                block,
                                                field,
                                            )}
                                        </span>
                                    )}
                            </label>
                        ))}
                    </fieldset>
                );
            })}
            <details className="sim-parameter-signal">
                <summary>信号属性</summary>
                <p className="sim-help">
                    信号宽度和实数 /
                    逻辑类型由模型检查确定。当前暂不提供复数、定点或矩阵信号配置。
                </p>
            </details>
            {fields.some((f) => f.editor !== "readonly" && !f.fixed) && (
                <div className="sim-parameter-actions">
                    <button
                        disabled={disabled || busy || !dirty}
                        onClick={() => void apply()}
                    >
                        {busy ? "正在应用…" : "应用参数"}
                    </button>
                    <button
                        disabled={busy || !dirty}
                        onClick={() => {
                            setDraft(original);
                            setError("");
                        }}
                    >
                        还原编辑
                    </button>
                </div>
            )}
            {dirty && (
                <p className="sim-help" role="status">
                    参数已修改，应用后生效。
                </p>
            )}
            {error && (
                <p role="alert" className="sim-parameter-error">
                    {error}
                </p>
            )}
        </div>
    );
}

export function ModelParameters({
    value,
    disabled,
    onApply,
    onPending,
}: {
    value: string;
    disabled: boolean;
    onApply(source: string): Promise<void>;
    onPending?(pending: boolean): void;
}) {
    const [draft, setDraft] = useState(value),
        [error, setError] = useState(""),
        [busy, setBusy] = useState(false);
    useEffect(() => {
        setDraft(value);
        setError("");
    }, [value]);
    const dirty = draft !== value;
    useEffect(() => {
        onPending?.(dirty);
        return () => onPending?.(false);
    }, [dirty, onPending]);
    const apply = async () => {
        setBusy(true);
        setError("");
        try {
            await onApply(draft);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };
    return (
        <div className="sim-model-parameters">
            <p className="sim-help">
                定义本模型的参数，例如 K = 2; Ts = 0.05;。方块中可填写 K、2*pi
                或矩阵表达式。
            </p>
            <textarea
                aria-label="模型参数定义"
                spellCheck={false}
                value={draft}
                rows={8}
                disabled={disabled || busy}
                onChange={(e) => setDraft(e.target.value)}
            />
            <button
                disabled={disabled || busy || draft === value}
                onClick={() => void apply()}
            >
                {busy ? "正在求值…" : "应用模型参数"}
            </button>
            <button
                disabled={busy || !dirty}
                onClick={() => {
                    setDraft(value);
                    setError("");
                }}
            >
                还原编辑
            </button>
            {error && (
                <p role="alert" className="sim-parameter-error">
                    {error}
                </p>
            )}
        </div>
    );
}
