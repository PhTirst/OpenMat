import { useEffect, useState } from "react";
import type { ComponentSpec, PropertySpec, UiValue } from "./catalog";
import { propertyValue } from "./catalog";
import { ComponentIcon } from "./ComponentIcon";
import type { UiNode } from "./model";
import { callbackPath } from "./callbacks";
import { LayoutEditor } from "./LayoutEditor";

function EventBinding({
    node,
    event,
    controller,
    uiPath,
    onChange,
    onOpenCallback,
    appClass,
    methods = [],
}: {
    node: UiNode;
    event: string;
    controller: string;
    uiPath: string;
    onChange: (node: UiNode) => void;
    onOpenCallback: (handler: string, event: string) => void;
    appClass?: string;
    methods?: { value: string; className: string }[];
}) {
    const [value, setValue] = useState(node.events[event] ?? controller);
    const effective = node.events[event] ?? controller;
    // Reflect the draft before blur, so the source path cannot move the edit
    // button between pointer-down and click when a connection is first entered.
    const displayed = appClass ? value.trim() : effective;
    useEffect(() => setValue(effective), [effective]);
    return (
        <div className="designer-event-binding">
            <label className="designer-property designer-property-wide">
                <span>{event}</span>
                <input
                    aria-label={event}
                    value={value}
                    list={appClass ? `methods-${node.id}-${event}` : undefined}
                    placeholder="未绑定回调"
                    onChange={(e) => setValue(e.target.value)}
                    onBlur={() => {
                        const handler = value.trim();
                        if (handler === effective) return;
                        const events = { ...node.events };
                        if (handler && handler !== controller)
                            events[event] = handler;
                        else delete events[event];
                        setValue(handler || controller);
                        onChange({ ...node, events });
                    }}
                />
            </label>
            {appClass ? (
                <datalist id={`methods-${node.id}-${event}`}>
                    {methods.map((m) => (
                        <option key={m.value} value={m.value}>
                            {m.className}
                        </option>
                    ))}
                </datalist>
            ) : null}
            <p className="designer-event-origin">
                {displayed
                    ? appClass
                        ? "实例成员方法"
                        : node.events[event]
                          ? "组件专用回调"
                          : "使用应用回调"
                    : "尚未绑定回调"}
                {displayed ? (
                    <code>
                        {callbackPath(
                            appClass
                                ? (methods.find((m) => m.value === displayed)
                                      ?.className ?? appClass)
                                : effective,
                            uiPath,
                        )}
                    </code>
                ) : null}
            </p>
            <button
                type="button"
                aria-label={`${value.trim() || controller ? "编辑" : "创建"} ${event} 回调`}
                onClick={() =>
                    onOpenCallback(value.trim() || controller, event)
                }
            >
                {value.trim() || controller ? "编辑代码" : "创建并编辑"}
            </button>
        </div>
    );
}

export function PropertyInput({
    property,
    value,
    onChange,
    mixed = false,
}: {
    property: PropertySpec;
    value: UiValue;
    mixed?: boolean;
    onChange: (value: UiValue) => void;
}) {
    const label = property.label;
    if (property.type === "boolean")
        return (
            <input
                aria-label={label}
                type="checkbox"
                checked={!mixed && Boolean(value)}
                ref={(element) => {
                    if (element) element.indeterminate = mixed;
                }}
                aria-checked={mixed ? "mixed" : Boolean(value)}
                disabled={property.readonly}
                onChange={(e) => onChange(e.target.checked)}
            />
        );
    if (property.type === "choice")
        return (
            <select
                aria-label={label}
                value={mixed ? "" : String(value)}
                disabled={property.readonly}
                onChange={(e) => onChange(e.target.value)}
            >
                {mixed && (
                    <option value="" disabled>
                        多个值
                    </option>
                )}
                {property.choices?.map((c) => (
                    <option key={c}>{c}</option>
                ))}
            </select>
        );
    if (property.type === "items" || property.type === "table")
        return (
            <textarea
                key={JSON.stringify(value) + mixed}
                aria-label={label}
                rows={3}
                readOnly={property.readonly}
                defaultValue={
                    mixed
                        ? ""
                        : property.type === "items"
                          ? (value as string[]).join("\n")
                          : JSON.stringify(value)
                }
                placeholder={mixed ? "多个值" : undefined}
                onBlur={(e) => {
                    if (mixed && !e.target.value) return;
                    try {
                        const next =
                            property.type === "items"
                                ? e.target.value.split("\n")
                                : (JSON.parse(e.target.value) as UiValue);
                        onChange(next);
                        e.target.setCustomValidity("");
                    } catch {
                        e.target.setCustomValidity("请输入二维 JSON 数组");
                        e.target.reportValidity();
                    }
                }}
            />
        );
    return (
        <input
            key={JSON.stringify(value) + mixed}
            aria-label={label}
            type={
                property.type === "number"
                    ? "number"
                    : property.type === "color" && !mixed
                      ? "color"
                      : "text"
            }
            defaultValue={mixed ? "" : String(value)}
            placeholder={mixed ? "多个值" : undefined}
            readOnly={property.readonly}
            min={property.min}
            max={property.max}
            step="any"
            onBlur={(e) => {
                if (property.readonly || (mixed && !e.target.value)) return;
                if (property.type === "number") {
                    if (
                        !e.target.checkValidity() ||
                        !Number.isFinite(e.target.valueAsNumber)
                    ) {
                        e.target.reportValidity();
                        return;
                    }
                    onChange(e.target.valueAsNumber);
                } else onChange(e.target.value);
            }}
        />
    );
}
export function Inspector({
    node,
    catalog,
    tab,
    onTab,
    onChange,
    controller,
    uiPath,
    onOpenCallback,
    appClass,
    methods,
    descriptor: suppliedDescriptor,
    parentMode,
}: {
    node: UiNode;
    catalog: Readonly<Record<string, ComponentSpec>>;
    tab: string;
    onTab: (tab: string) => void;
    onChange: (node: UiNode) => void;
    controller: string;
    uiPath: string;
    onOpenCallback: (handler: string, event: string) => void;
    appClass?: string;
    methods?: { value: string; className: string }[];
    descriptor?: ComponentSpec;
    parentMode?: string;
}) {
    const descriptor = suppliedDescriptor ?? catalog[node.type];
    return (
        <aside className="designer-inspector" aria-label="检查器">
            <div className="designer-pane-title">检查器</div>
            <div className="designer-object-title">
                <ComponentIcon type={descriptor?.icon ?? node.type} size={24} />
                <div>
                    <strong>{node.name}</strong>
                    <small>{node.type}</small>
                </div>
            </div>
            <nav className="designer-tabs" aria-label="检查器页面">
                {["属性", "布局", "事件"].map((label) => (
                    <button
                        type="button"
                        key={label}
                        aria-pressed={tab === label}
                        onClick={() => onTab(label)}
                    >
                        {label}
                    </button>
                ))}
            </nav>
            <div className="designer-inspector-body">
                {tab === "属性" ? (
                    <>
                        <label className="designer-property">
                            <span>组件名称</span>
                            <input
                                key={node.id + node.name}
                                aria-label="组件名称"
                                defaultValue={node.name}
                                onBlur={(e) =>
                                    onChange({ ...node, name: e.target.value })
                                }
                            />
                        </label>
                        <div className="designer-section-label">
                            {descriptor?.className
                                ? "类的公开属性"
                                : "组件属性"}
                        </div>
                        {descriptor?.properties.map((property) => (
                            <label
                                key={property.name}
                                className={`designer-property ${["items", "table"].includes(property.type) ? "designer-property-wide" : ""}`}
                            >
                                <span title={property.name}>
                                    {property.label}
                                    {property.readonly ? " · 只读" : ""}
                                </span>
                                <PropertyInput
                                    property={property}
                                    value={propertyValue(
                                        node,
                                        property.name,
                                        catalog,
                                    )}
                                    onChange={(value) =>
                                        onChange({
                                            ...node,
                                            properties: {
                                                ...node.properties,
                                                [property.name]: value,
                                            },
                                        })
                                    }
                                />
                                {Object.hasOwn(
                                    node.properties,
                                    property.name,
                                ) && !property.readonly ? (
                                    <button
                                        type="button"
                                        className="designer-reset"
                                        title={`重置 ${property.label}`}
                                        aria-label={`重置 ${property.label}`}
                                        onClick={() => {
                                            const properties = {
                                                ...node.properties,
                                            };
                                            delete properties[property.name];
                                            onChange({ ...node, properties });
                                        }}
                                    >
                                        ↺
                                    </button>
                                ) : null}
                            </label>
                        ))}
                        {!descriptor ? (
                            <p className="designer-hint">
                                此组件类尚未注册。XML 中的属性已保留。
                            </p>
                        ) : null}
                    </>
                ) : null}
                {tab === "布局" ? (
                    <LayoutEditor
                        layout={node.layout}
                        composite={Boolean(descriptor?.composite)}
                        {...(parentMode ? { parentMode } : {})}
                        onChange={(patch) =>
                            onChange({
                                ...node,
                                layout: { ...node.layout, ...patch },
                            })
                        }
                    />
                ) : null}
                {tab === "事件" ? (
                    <>
                        <p className="designer-hint">
                            {appClass
                                ? "选择或输入 对象.成员方法，例如 app.onRun。创建或编辑会打开所属 M 类；清空即断开连接。"
                                : "显示实际使用的 .m 回调。点击编辑代码打开对应文件；清空函数名会恢复使用应用回调。"}
                        </p>
                        {descriptor?.events.length ? (
                            descriptor.events.map((event) => (
                                <EventBinding
                                    key={`${node.id}:${event}:${controller}`}
                                    node={node}
                                    event={event}
                                    controller={controller}
                                    uiPath={uiPath}
                                    onChange={onChange}
                                    onOpenCallback={onOpenCallback}
                                    {...(appClass ? { appClass } : {})}
                                    {...(methods ? { methods } : {})}
                                />
                            ))
                        ) : (
                            <p className="designer-hint">
                                此组件没有可绑定的外部事件。
                            </p>
                        )}
                    </>
                ) : null}
                <div className="designer-id">ID · {node.id}</div>
            </div>
        </aside>
    );
}
