export type SizeMode = "auto" | "fixed" | "content" | "fill";
export interface UiSizing {
    widthMode: SizeMode;
    heightMode: SizeMode;
    minWidth: number;
    minHeight: number;
    maxWidth: number;
    maxHeight: number;
    growX: number;
    growY: number;
    rowTracks: string;
    columnTracks: string;
}
export const SIZING_DEFAULTS: UiSizing = {
    widthMode: "auto",
    heightMode: "auto",
    minWidth: 0,
    minHeight: 0,
    maxWidth: 10000,
    maxHeight: 10000,
    growX: 1,
    growY: 1,
    rowTracks: "",
    columnTracks: "",
};
export const STRING_LAYOUT_KEYS = new Set([
    "mode",
    "widthMode",
    "heightMode",
    "rowTracks",
    "columnTracks",
]);
/** A deliberately small grammar, shared by persistence and the renderer. */
export function gridTracks(source: string): string[] {
    const entries = source.trim() ? source.trim().split(/\s+/) : [];
    if (entries.length > 64) throw new Error("网格最多设置 64 个行列尺寸。");
    return entries.map((entry) => {
        if (entry === "auto") return "auto";
        const match = /^(\d+(?:\.\d+)?)(px|fr)?$/.exec(entry);
        if (!match || Number(match[1]) <= 0 || Number(match[1]) > 10000)
            throw new Error(
                "行列尺寸使用正数像素、auto 或比例，例如 240 1fr 2fr。",
            );
        return `${Number(match[1])}${match[2] ?? "px"}`;
    });
}
export function validateLayout(
    layout: Record<string, unknown>,
    base: object,
): void {
    for (const [name, value] of Object.entries(layout)) {
        if (!Object.hasOwn(base, name) && !Object.hasOwn(SIZING_DEFAULTS, name))
            throw new Error(`布局字段无效：${name}`);
        if (STRING_LAYOUT_KEYS.has(name)) {
            if (typeof value !== "string")
                throw new Error(`布局文本无效：${name}`);
            if (
                name === "mode" &&
                !["absolute", "grid", "row", "column"].includes(value)
            )
                throw new Error("布局模式无效。");
            if (
                (name === "widthMode" || name === "heightMode") &&
                !["auto", "fixed", "content", "fill"].includes(value)
            )
                throw new Error("尺寸策略无效。");
            if (name.endsWith("Tracks")) gridTracks(value);
            continue;
        }
        if (
            typeof value !== "number" ||
            !Number.isFinite(value) ||
            value < 0 ||
            value > 10000
        )
            throw new Error(`布局数值无效：${name}`);
        if (
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
            ].includes(name) &&
            value < 1
        )
            throw new Error(`布局数值必须大于零：${name}`);
        if (
            ["row", "column", "rowSpan", "columnSpan", "columns"].includes(
                name,
            ) &&
            !Number.isInteger(value)
        )
            throw new Error(`布局索引必须为整数：${name}`);
    }
    const sizing = { ...SIZING_DEFAULTS, ...layout } as UiSizing;
    if (
        sizing.minWidth > sizing.maxWidth ||
        sizing.minHeight > sizing.maxHeight
    )
        throw new Error("最小尺寸不能超过最大尺寸。");
}
