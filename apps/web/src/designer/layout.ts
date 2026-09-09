import type { UiLayout } from "./model";

export const CONTENT_LAYOUT = new Set<keyof UiLayout>([
    "mode",
    "columns",
    "gap",
    "padding",
    "rowTracks",
    "columnTracks",
]);

/** XML uses top-left X/Y. Native Position remains MATLAB's bottom-left rectangle. */
export function layoutAssignments(
    target: string,
    layout: UiLayout,
    content = true,
): string[] {
    return Object.entries(layout).flatMap(([name, value]) => {
        if (!content && CONTENT_LAYOUT.has(name as keyof UiLayout)) return [];
        const field = name[0]!.toUpperCase() + name.slice(1);
        return [
            `${target}.Layout.${field} = ${typeof value === "string" ? `'${value.replaceAll("'", "''")}'` : value};`,
        ];
    });
}
