import type { SlxBlock } from "./client";
import { PARAMETER_CATALOG } from "./block-parameters";

export function SlxParameterInspector({ block }: { block: SlxBlock }) {
    const descriptor = PARAMETER_CATALOG.blocks.find(
        (b) => b.slx === block.blockType,
    );
    const known = new Set(descriptor?.parameters.map((p) => p.name));
    const other = Object.entries(block.properties).filter(
        ([name]) => !known.has(name),
    );
    return (
        <div className="sim-parameter-form">
            {Object.entries(PARAMETER_CATALOG.groups).map(([group, title]) => {
                const fields =
                    descriptor?.parameters.filter(
                        (f) => f.group === group && !f.extension,
                    ) ?? [];
                if (!fields.length) return null;
                return (
                    <fieldset key={group}>
                        <legend>{title}</legend>
                        <dl>
                            {fields.map((field) => {
                                const explicit = Object.hasOwn(
                                    block.properties,
                                    field.name,
                                );
                                const value = explicit
                                    ? block.properties[field.name]!
                                    : field.defaultValue;
                                const unsupported =
                                    explicit &&
                                    field.options &&
                                    !field.options.some(
                                        (o) => o.supported && o.value === value,
                                    );
                                return (
                                    <div
                                        className={`sim-slx-field ${unsupported ? "is-unsupported" : ""}`}
                                        key={field.name}
                                    >
                                        <dt>
                                            <span>{field.label}</span>
                                            <span>{field.name}</span>
                                        </dt>
                                        <dd>{value}</dd>
                                        {!explicit && (
                                            <p className="sim-help">
                                                未显式存储；这里显示 R2022b
                                                标准库参考默认值，实际值以导入检查为准。
                                            </p>
                                        )}
                                        {unsupported && (
                                            <p className="sim-help">
                                                当前选项暂不支持，原始参数已保留。
                                            </p>
                                        )}
                                    </div>
                                );
                            })}
                        </dl>
                    </fieldset>
                );
            })}
            <details open={!descriptor}>
                <summary>其他原始属性 · {other.length}</summary>
                <dl className="sim-slx-original-parameters">
                    {other.map(([name, value]) => (
                        <div className="sim-slx-field" key={name}>
                            <dt>{name}</dt>
                            <dd>{value}</dd>
                        </div>
                    ))}
                </dl>
            </details>
            <p className="sim-help">
                这是 SLX
                源结构的参数。可在模型参数区修改变量并重新检查，或进入数值模型编辑受支持的方块。
            </p>
        </div>
    );
}
