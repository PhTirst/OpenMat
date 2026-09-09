import { findParent, type UiDocument, type UiNode } from "./model";
import {
    boundsOf,
    columnCount,
    type Placement,
    type Rect,
    type Rects,
} from "./operations";

export function nodeElements(board: HTMLElement): Map<string, HTMLElement> {
    return new Map(
        [...board.querySelectorAll<HTMLElement>("[data-ui-id]")].map((el) => [
            el.dataset.uiId!,
            el,
        ]),
    );
}
export function layoutElement(element: HTMLElement): HTMLElement {
    return (
        element.querySelector<HTMLElement>(
            ":scope > .ui-content > .ui-layout",
        ) ?? element
    );
}
export function measureNodes(
    board: HTMLElement | null,
    document: UiDocument,
    scale: number,
): Rects {
    if (!board) return {};
    const elements = nodeElements(board),
        result: Record<string, Rect> = {};
    for (const [id, element] of elements) {
        const parent = findParent(document.root, id),
            parentElement = parent && elements.get(parent.id);
        if (!parentElement || !element.getClientRects().length) continue;
        const layout = layoutElement(parentElement),
            origin = layout.getBoundingClientRect(),
            rect = element.getBoundingClientRect();
        result[id] = {
            x: (rect.left - origin.left) / scale + layout.scrollLeft,
            y: (rect.top - origin.top) / scale + layout.scrollTop,
            width: rect.width / scale,
            height: rect.height / scale,
        };
    }
    return result;
}
export function placementAt(
    parent: UiNode,
    element: HTMLElement | undefined,
    clientX: number,
    clientY: number,
    scale: number,
): Placement {
    if (!element) return {};
    const layout = layoutElement(element),
        rect = layout.getBoundingClientRect();
    const x = (clientX - rect.left) / scale + layout.scrollLeft,
        y = (clientY - rect.top) / scale + layout.scrollTop;
    if (parent.layout.mode === "absolute")
        return {
            x: Math.max(0, Math.round(x / 8) * 8),
            y: Math.max(0, Math.round(y / 8) * 8),
        };
    if (parent.layout.mode === "grid") {
        const style = getComputedStyle(layout);
        const track = (
            offset: number,
            source: string,
            fallback: number,
            limit: number,
        ) => {
            const sizes = source
                .split(/\s+/)
                .filter((s) => /^\d+(\.\d+)?px$/.test(s))
                .map(parseFloat);
            let position = parent.layout.padding;
            for (let i = 0; i < Math.max(sizes.length, limit); i++) {
                const size = sizes[i] ?? sizes.at(-1) ?? fallback;
                if (offset < position + size + parent.layout.gap / 2)
                    return i + 1;
                position += size + parent.layout.gap;
            }
            return Math.max(sizes.length, limit);
        };
        return {
            column: Math.min(
                columnCount(parent.layout),
                track(
                    x,
                    style.gridTemplateColumns,
                    100,
                    columnCount(parent.layout),
                ),
            ),
            row: track(
                y,
                style.gridTemplateRows,
                48,
                Math.max(
                    1,
                    ...parent.children.map(
                        (n) => n.layout.row + n.layout.rowSpan,
                    ),
                ),
            ),
        };
    }
    const horizontal = parent.layout.mode === "row";
    const elements = new Map(
        [...layout.children]
            .filter(
                (child): child is HTMLElement =>
                    child instanceof HTMLElement && !!child.dataset.uiId,
            )
            .map((el) => [el.dataset.uiId, el]),
    );
    const index = parent.children.findIndex((node) => {
        const child = elements.get(node.id);
        if (!child) return false;
        const bounds = child.getBoundingClientRect();
        return horizontal
            ? clientX < bounds.left + bounds.width / 2
            : clientY < bounds.top + bounds.height / 2;
    });
    return { index: index < 0 ? parent.children.length : index };
}
export interface Guide {
    axis: "x" | "y";
    position: number;
}
export function snapMove(
    group: Rect,
    dx: number,
    dy: number,
    targets: Rect[],
    tolerance: number,
    snap = true,
): { dx: number; dy: number; guides: Guide[] } {
    const guides: Guide[] = [];
    if (snap)
        for (const axis of ["x", "y"] as const) {
            const size = axis === "x" ? "width" : "height",
                delta = axis === "x" ? dx : dy;
            const edges = [
                group[axis] + delta,
                group[axis] + delta + group[size] / 2,
                group[axis] + delta + group[size],
            ];
            let adjustment: number | undefined,
                line = 0;
            for (const target of targets)
                for (const edge of [
                    target[axis],
                    target[axis] + target[size] / 2,
                    target[axis] + target[size],
                ])
                    for (const moving of edges)
                        if (
                            Math.abs(edge - moving) <= tolerance &&
                            (adjustment === undefined ||
                                Math.abs(edge - moving) < Math.abs(adjustment))
                        ) {
                            adjustment = edge - moving;
                            line = edge;
                        }
            const next =
                adjustment === undefined
                    ? Math.round((group[axis] + delta) / 8) * 8 - group[axis]
                    : delta + adjustment;
            if (axis === "x") dx = next;
            else dy = next;
            if (adjustment !== undefined) guides.push({ axis, position: line });
        }
    return { dx: Math.max(-group.x, dx), dy: Math.max(-group.y, dy), guides };
}
export function intersects(a: Rect, b: Rect): boolean {
    return (
        a.x < b.x + b.width &&
        b.x < a.x + a.width &&
        a.y < b.y + b.height &&
        b.y < a.y + a.height
    );
}
export function marqueeRect(
    x1: number,
    y1: number,
    x2: number,
    y2: number,
): Rect {
    return boundsOf([
        { x: x1, y: y1, width: 0, height: 0 },
        { x: x2, y: y2, width: 0, height: 0 },
    ]);
}
