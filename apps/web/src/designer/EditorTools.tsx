import type { Arrangement } from "./operations";

export function EditorTools({
    count,
    absolute,
    disabled,
    arrange,
    wrap,
    width,
    height,
    resize,
}: {
    count: number;
    absolute: boolean;
    disabled: boolean;
    arrange: (operation: Arrangement) => void;
    wrap: (type: string) => void;
    width: number;
    height: number;
    resize: (width: number, height: number) => void;
}) {
    return (
        <div
            className="designer-edit-tools"
            role="toolbar"
            aria-label="布局编辑工具"
        >
            <span>{count > 1 ? `已选 ${count} 项` : "布局编辑"}</span>
            <select
                aria-label="批量排版"
                value=""
                disabled={disabled || count < 2}
                onChange={(e) => arrange(e.target.value as Arrangement)}
            >
                <option value="" disabled>
                    对齐与尺寸
                </option>
                {(
                    [
                        ["left", "左对齐"],
                        ["centerX", "水平居中"],
                        ["right", "右对齐"],
                        ["top", "顶对齐"],
                        ["centerY", "垂直居中"],
                        ["bottom", "底对齐"],
                        ["distributeX", "水平等间距"],
                        ["distributeY", "垂直等间距"],
                        ["sameWidth", "统一宽度"],
                        ["sameHeight", "统一高度"],
                    ] as const
                ).map(([value, label]) => (
                    <option
                        key={value}
                        value={value}
                        disabled={
                            (!absolute && !value.startsWith("same")) ||
                            (value.startsWith("distribute") && count < 3)
                        }
                    >
                        {label}
                    </option>
                ))}
            </select>
            <select
                aria-label="包入布局容器"
                value=""
                disabled={disabled || count < 1}
                onChange={(e) => wrap(e.target.value)}
            >
                <option value="" disabled>
                    包入容器
                </option>
                <option value="RowLayout">水平布局</option>
                <option value="ColumnLayout">垂直布局</option>
                <option value="GridLayout">网格布局</option>
                <option value="ScrollPanel">滚动容器</option>
            </select>
            <span className="designer-tool-spacer" />
            <label>
                预览{" "}
                <input
                    key={`width-${width}`}
                    aria-label="预览宽度"
                    type="number"
                    min={24}
                    max={10000}
                    defaultValue={width}
                    disabled={disabled}
                    onBlur={(e) => {
                        if (
                            e.target.checkValidity() &&
                            Number.isFinite(e.target.valueAsNumber) &&
                            e.target.valueAsNumber !== width
                        )
                            resize(e.target.valueAsNumber, height);
                    }}
                />
            </label>
            <span>×</span>
            <input
                key={`height-${height}`}
                aria-label="预览高度"
                type="number"
                min={24}
                max={10000}
                defaultValue={height}
                disabled={disabled}
                onBlur={(e) => {
                    if (
                        e.target.checkValidity() &&
                        Number.isFinite(e.target.valueAsNumber) &&
                        e.target.valueAsNumber !== height
                    )
                        resize(width, e.target.valueAsNumber);
                }}
            />
        </div>
    );
}
