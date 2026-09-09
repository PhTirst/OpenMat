import type { UiLayout } from "./model";
import { DEFAULT_LAYOUT } from "./model";
import { CONTENT_LAYOUT } from "./layout";
import {
    gridTracks,
    SIZING_DEFAULTS,
    STRING_LAYOUT_KEYS,
} from "./layout-schema";

const labels: Record<keyof UiLayout, string> = {
    mode: "子组件布局",
    x: "X",
    y: "Y",
    width: "宽度",
    height: "高度",
    row: "行",
    column: "列",
    rowSpan: "跨行",
    columnSpan: "跨列",
    columns: "网格列数",
    gap: "间距",
    padding: "内边距",
    grow: "伸展权重",
    widthMode: "宽度策略",
    heightMode: "高度策略",
    minWidth: "最小宽度",
    minHeight: "最小高度",
    maxWidth: "最大宽度",
    maxHeight: "最大高度",
    growX: "横向伸展",
    growY: "纵向伸展",
    rowTracks: "每行尺寸",
    columnTracks: "每列尺寸",
};
const groups: { title: string; keys: (keyof UiLayout)[] }[] = [
    {
        title: "尺寸",
        keys: [
            "widthMode",
            "width",
            "minWidth",
            "maxWidth",
            "growX",
            "heightMode",
            "height",
            "minHeight",
            "maxHeight",
            "growY",
        ],
    },
    {
        title: "在父容器中的位置",
        keys: ["x", "y", "row", "column", "rowSpan", "columnSpan", "grow"],
    },
    {
        title: "内部排列",
        keys: [
            "mode",
            "columns",
            "columnTracks",
            "rowTracks",
            "gap",
            "padding",
        ],
    },
];
export function LayoutEditor({
    layout,
    onChange,
    mixed = new Set<string>(),
    composite = false,
    parentMode,
}: {
    layout: UiLayout;
    onChange: (patch: Partial<UiLayout>) => void;
    mixed?: ReadonlySet<string>;
    composite?: boolean;
    parentMode?: string;
}) {
    const effective = { ...SIZING_DEFAULTS, ...layout };
    return (
        <>
            <p className="designer-hint">
                绝对布局拖动坐标，网格拖动行列，水平和垂直布局拖动顺序。尺寸策略分别控制两个方向。
            </p>
            {composite && (
                <p className="designer-hint">内部排列由组件定义维护。</p>
            )}
            {groups.map((group) => (
                <fieldset className="designer-layout-group" key={group.title}>
                    <legend>{group.title}</legend>
                    {group.keys.map((key) => {
                        const isMixed = mixed.has(key),
                            value = effective[key];
                        const disabled =
                            (composite && CONTENT_LAYOUT.has(key)) ||
                            (["x", "y"].includes(key) &&
                                !!parentMode &&
                                parentMode !== "absolute") ||
                            ([
                                "row",
                                "column",
                                "rowSpan",
                                "columnSpan",
                            ].includes(key) &&
                                !!parentMode &&
                                parentMode !== "grid");
                        const choices =
                            key === "mode"
                                ? [
                                      ["column", "垂直排列"],
                                      ["row", "水平排列"],
                                      ["grid", "网格"],
                                      ["absolute", "绝对定位"],
                                  ]
                                : key.endsWith("Mode")
                                  ? [
                                        ["auto", "默认"],
                                        ["fixed", "固定"],
                                        ["content", "随内容"],
                                        ["fill", "填充剩余空间"],
                                    ]
                                  : null;
                        return (
                            <label className="designer-property" key={key}>
                                <span>{labels[key]}</span>
                                {choices ? (
                                    <select
                                        aria-label={labels[key]}
                                        disabled={disabled}
                                        value={isMixed ? "" : String(value)}
                                        onChange={(e) =>
                                            onChange({ [key]: e.target.value })
                                        }
                                    >
                                        {isMixed && (
                                            <option value="" disabled>
                                                多个值
                                            </option>
                                        )}
                                        {choices.map(([entry, label]) => (
                                            <option key={entry} value={entry}>
                                                {label}
                                            </option>
                                        ))}
                                    </select>
                                ) : (
                                    <input
                                        key={`${key}:${String(value)}:${isMixed}`}
                                        aria-label={labels[key]}
                                        disabled={disabled}
                                        type={
                                            STRING_LAYOUT_KEYS.has(key)
                                                ? "text"
                                                : "number"
                                        }
                                        min={
                                            [
                                                "width",
                                                "height",
                                                "row",
                                                "column",
                                                "rowSpan",
                                                "columnSpan",
                                                "columns",
                                                "maxWidth",
                                                "maxHeight",
                                            ].includes(key)
                                                ? 1
                                                : 0
                                        }
                                        max={10000}
                                        step={
                                            [
                                                "row",
                                                "column",
                                                "rowSpan",
                                                "columnSpan",
                                                "columns",
                                            ].includes(key)
                                                ? 1
                                                : "any"
                                        }
                                        placeholder={
                                            isMixed
                                                ? "多个值"
                                                : key.endsWith("Tracks")
                                                  ? "例如 240 1fr 2fr"
                                                  : undefined
                                        }
                                        defaultValue={isMixed ? "" : value}
                                        onBlur={(e) => {
                                            if (
                                                disabled ||
                                                (isMixed && !e.target.value)
                                            )
                                                return;
                                            try {
                                                const next =
                                                    STRING_LAYOUT_KEYS.has(key)
                                                        ? e.target.value.trim()
                                                        : e.target
                                                              .valueAsNumber;
                                                if (
                                                    typeof next === "number" &&
                                                    (!Number.isFinite(next) ||
                                                        !e.target.checkValidity())
                                                )
                                                    throw new Error(
                                                        "请输入有效尺寸。",
                                                    );
                                                if (key.endsWith("Tracks"))
                                                    gridTracks(String(next));
                                                if (next !== value || isMixed)
                                                    onChange({ [key]: next });
                                                e.target.setCustomValidity("");
                                            } catch (error) {
                                                e.target.setCustomValidity(
                                                    String(error),
                                                );
                                                e.target.reportValidity();
                                            }
                                        }}
                                        onInput={(e) =>
                                            e.currentTarget.setCustomValidity(
                                                "",
                                            )
                                        }
                                    />
                                )}
                                <button
                                    type="button"
                                    className="designer-reset"
                                    aria-label={`重置布局 ${labels[key]}`}
                                    disabled={disabled}
                                    onClick={() =>
                                        onChange({
                                            [key]:
                                                (
                                                    DEFAULT_LAYOUT as unknown as Record<
                                                        string,
                                                        unknown
                                                    >
                                                )[key] ??
                                                SIZING_DEFAULTS[
                                                    key as keyof typeof SIZING_DEFAULTS
                                                ],
                                        })
                                    }
                                >
                                    ↺
                                </button>
                            </label>
                        );
                    })}
                    {group.title === "内部排列" && (
                        <p className="designer-hint">
                            行列尺寸以空格分隔：240 表示像素，auto 随内容，1fr /
                            2fr 按比例分配。留空沿用网格列数。
                        </p>
                    )}
                </fieldset>
            ))}
        </>
    );
}
