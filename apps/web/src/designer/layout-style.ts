import type { CSSProperties } from "react";
import type { UiLayout } from "./model";
import { gridTracks } from "./layout-schema";

export function sizingStyle(
    layout: UiLayout,
    parentMode?: string,
): CSSProperties {
    const style: CSSProperties = {};
    const horizontal = parentMode === "row";
    for (const axis of ["width", "height"] as const) {
        const mode =
            layout[axis === "width" ? "widthMode" : "heightMode"] ?? "auto";
        const min = layout[axis === "width" ? "minWidth" : "minHeight"];
        const max = layout[axis === "width" ? "maxWidth" : "maxHeight"];
        if (min !== undefined)
            style[axis === "width" ? "minWidth" : "minHeight"] = min;
        if (max !== undefined)
            style[axis === "width" ? "maxWidth" : "maxHeight"] = max;
        if (mode === "auto") continue;
        style[axis === "width" ? "minWidth" : "minHeight"] = min ?? 0;
        const mainAxis = horizontal ? axis === "width" : axis === "height";
        if (mode === "fixed") style[axis] = layout[axis];
        else if (mode === "content") style[axis] = "max-content";
        else style[axis] = "auto";
        if (parentMode === "absolute" && mode === "fill") {
            if (axis === "width") style.right = layout.x;
            else style.bottom = layout.y;
        } else if (parentMode === "grid") {
            style[axis === "width" ? "justifySelf" : "alignSelf"] =
                mode === "fill" ? "stretch" : "start";
        } else if (parentMode === "row" || parentMode === "column") {
            if (mainAxis) {
                const weight =
                    layout[axis === "width" ? "growX" : "growY"] ?? 1;
                style.flex = mode === "fill" ? `${weight} 1 0px` : "0 0 auto";
            } else style.alignSelf = mode === "fill" ? "stretch" : "start";
        }
    }
    return style;
}

export function containerStyle(layout: UiLayout): CSSProperties {
    const style: CSSProperties = { gap: layout.gap, padding: layout.padding };
    if (layout.mode === "absolute")
        return { ...style, position: "relative", minHeight: 0 };
    if (layout.mode === "grid") {
        const columns = gridTracks(layout.columnTracks ?? "");
        const rows = gridTracks(layout.rowTracks ?? "");
        return {
            ...style,
            display: "grid",
            gridTemplateColumns: columns.length
                ? columns.join(" ")
                : `repeat(${layout.columns}, minmax(0, 1fr))`,
            ...(rows.length ? { gridTemplateRows: rows.join(" ") } : {}),
            gridAutoRows: rows.length
                ? "minmax(min-content, auto)"
                : "minmax(0, 1fr)",
            alignContent: "stretch",
        };
    }
    return {
        ...style,
        display: "flex",
        flexDirection: layout.mode === "row" ? "row" : "column",
    };
}
