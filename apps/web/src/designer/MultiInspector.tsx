import {
    propertyValue,
    type ComponentSpec,
    type PropertySpec,
    type UiValue,
} from "./catalog";
import type { UiLayout, UiNode } from "./model";
import { SIZING_DEFAULTS } from "./layout-schema";
import { PropertyInput } from "./Inspector";
import { LayoutEditor } from "./LayoutEditor";

export function commonProperties(
    nodes: UiNode[],
    catalog: Readonly<Record<string, ComponentSpec>>,
): PropertySpec[] {
    const first = catalog[nodes[0]!.type]?.properties ?? [];
    return first.flatMap((property) => {
        const matches = nodes.map((node) =>
            catalog[node.type]?.properties.find(
                (p) => p.name === property.name,
            ),
        );
        if (
            matches.some(
                (p) =>
                    !p ||
                    p.readonly ||
                    p.type !== property.type ||
                    p.valueClass !== property.valueClass,
            )
        )
            return [];
        const min = Math.max(...matches.map((p) => p!.min ?? -Infinity)),
            max = Math.min(...matches.map((p) => p!.max ?? Infinity));
        if (min > max) return [];
        const choices = property.choices?.filter((choice) =>
            matches.every((p) => p!.choices?.includes(choice)),
        );
        if (property.type === "choice" && !choices?.length) return [];
        return [
            {
                ...property,
                ...(Number.isFinite(min) ? { min } : {}),
                ...(Number.isFinite(max) ? { max } : {}),
                ...(choices ? { choices } : {}),
            },
        ];
    });
}
export function MultiInspector({
    nodes,
    catalog,
    tab,
    onTab,
    onProperty,
    onLayout,
    parentMode,
}: {
    nodes: UiNode[];
    catalog: Readonly<Record<string, ComponentSpec>>;
    tab: string;
    onTab: (tab: string) => void;
    onProperty: (name: string, value: UiValue) => void;
    onLayout: (patch: Partial<UiLayout>) => void;
    parentMode?: string;
}) {
    const activeTab = tab === "布局" ? tab : "属性";
    const layouts = nodes.map((node) => ({
        ...SIZING_DEFAULTS,
        ...node.layout,
    }));
    const mixed = new Set(
        Object.keys(layouts[0]!).filter((key) =>
            layouts.some(
                (layout) =>
                    layout[key as keyof UiLayout] !==
                    layouts[0]![key as keyof UiLayout],
            ),
        ),
    );
    return (
        <aside className="designer-inspector" aria-label="检查器">
            <div className="designer-pane-title">检查器</div>
            <div className="designer-object-title">
                <div>
                    <strong>已选择 {nodes.length} 个组件</strong>
                    <small>{nodes.map((n) => n.name).join("、")}</small>
                </div>
            </div>
            <nav className="designer-tabs" aria-label="检查器页面">
                {["属性", "布局"].map((name) => (
                    <button
                        type="button"
                        key={name}
                        aria-pressed={activeTab === name}
                        onClick={() => onTab(name)}
                    >
                        {name}
                    </button>
                ))}
            </nav>
            <div className="designer-inspector-body">
                {tab === "布局" ? (
                    <LayoutEditor
                        layout={nodes[0]!.layout}
                        mixed={mixed}
                        composite={nodes.some(
                            (n) => catalog[n.type]?.composite,
                        )}
                        onChange={onLayout}
                        {...(parentMode ? { parentMode } : {})}
                    />
                ) : (
                    <>
                        <p className="designer-hint">
                            修改共同属性会应用到全部所选组件，可一次撤销。
                        </p>
                        {commonProperties(nodes, catalog).map((property) => {
                            const value = propertyValue(
                                nodes[0]!,
                                property.name,
                                catalog,
                            );
                            const different = nodes.some(
                                (n) =>
                                    JSON.stringify(
                                        propertyValue(
                                            n,
                                            property.name,
                                            catalog,
                                        ),
                                    ) !== JSON.stringify(value),
                            );
                            return (
                                <label
                                    key={property.name}
                                    className="designer-property"
                                >
                                    <span>{property.label}</span>
                                    <PropertyInput
                                        property={property}
                                        value={value}
                                        mixed={different}
                                        onChange={(next) =>
                                            onProperty(property.name, next)
                                        }
                                    />
                                    {different && (
                                        <span
                                            className="designer-mixed"
                                            title="多个值"
                                        >
                                            —
                                        </span>
                                    )}
                                </label>
                            );
                        })}
                    </>
                )}
            </div>
        </aside>
    );
}
