import { BUILTIN_CATALOG, type ComponentSpec, type UiValue } from "./catalog";
import {
    assertParent,
    createNode,
    findNode,
    findParent,
    updateNode,
    walk,
    type UiDocument,
    type UiLayout,
    type UiNode,
} from "./model";
import { gridTracks } from "./layout-schema";

export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}
export type Rects = Readonly<Record<string, Rect>>;
export const rectOf = (node: UiNode, rects: Rects = {}): Rect =>
    rects[node.id] ?? node.layout;
export function boundsOf(rects: Rect[]): Rect {
    const x = Math.min(...rects.map((r) => r.x)),
        y = Math.min(...rects.map((r) => r.y));
    return {
        x,
        y,
        width: Math.max(...rects.map((r) => r.x + r.width)) - x,
        height: Math.max(...rects.map((r) => r.y + r.height)) - y,
    };
}
export function selectionRoots(
    document: UiDocument,
    ids: readonly string[],
): UiNode[] {
    const selected = new Set(ids);
    return walk(document.root).filter((node) => {
        if (!selected.has(node.id) || node.id === document.root.id)
            return false;
        let parent = findParent(document.root, node.id);
        while (parent) {
            if (selected.has(parent.id) && parent.id !== document.root.id)
                return false;
            parent = findParent(document.root, parent.id);
        }
        return true;
    });
}
export function patchNodes(
    document: UiDocument,
    ids: readonly string[],
    patch: (node: UiNode) => UiNode,
): UiDocument {
    const selected = new Set(ids);
    const visit = (node: UiNode): UiNode => {
        const next = selected.has(node.id) ? patch(node) : node;
        return { ...next, children: next.children.map(visit) };
    };
    return { ...document, root: visit(document.root) };
}
export function convertLayout(
    node: UiNode,
    previous: UiLayout,
    rects: Rects = {},
): UiNode {
    if (previous.mode === node.layout.mode) return node;
    const parent = { ...node, children: [] as UiNode[] };
    for (const child of node.children) {
        const layout =
            node.layout.mode === "absolute"
                ? {
                      ...child.layout,
                      ...rectOf(child, rects),
                      widthMode: "fixed" as const,
                      heightMode: "fixed" as const,
                  }
                : node.layout.mode === "grid"
                  ? freeCell(parent, child.layout)
                  : child.layout;
        parent.children.push({ ...child, layout });
    }
    return parent;
}
export function patchLayouts(
    document: UiDocument,
    ids: readonly string[],
    patch: Partial<UiLayout>,
    rects: Rects = {},
): UiDocument {
    return patchNodes(document, ids, (node) =>
        convertLayout(
            { ...node, layout: { ...node.layout, ...patch } },
            node.layout,
            rects,
        ),
    );
}
export function patchProperties(
    document: UiDocument,
    ids: readonly string[],
    name: string,
    value: UiValue,
): UiDocument {
    return patchNodes(document, ids, (node) => ({
        ...node,
        properties: { ...node.properties, [name]: value },
    }));
}
function siblings(document: UiDocument, ids: readonly string[], minimum = 1) {
    const nodes = selectionRoots(document, ids);
    const parent = nodes[0] && findParent(document.root, nodes[0].id);
    if (
        nodes.length < minimum ||
        !parent ||
        nodes.some((n) => findParent(document.root, n.id)?.id !== parent.id)
    )
        throw new Error(`请选择同一容器内至少 ${minimum} 个组件。`);
    return { nodes, parent };
}
export function removeNodes(
    document: UiDocument,
    ids: readonly string[],
): UiDocument {
    const roots = selectionRoots(document, ids);
    const removed = new Set(
        roots.flatMap((node) => walk(node).map((n) => n.id)),
    );
    const names = new Set(
        roots.flatMap((node) => walk(node).map((n) => n.name)),
    );
    const visit = (node: UiNode): UiNode => ({
        ...node,
        events: Object.fromEntries(
            Object.entries(node.events).filter(
                ([, binding]) => !names.has(binding.split(".")[0]!),
            ),
        ),
        children: node.children.filter((n) => !removed.has(n.id)).map(visit),
    });
    return { ...document, root: visit(document.root) };
}
export function columnCount(layout: UiLayout): number {
    return gridTracks(layout.columnTracks ?? "").length || layout.columns;
}
export function cellsOverlap(a: UiLayout, b: UiLayout): boolean {
    return (
        a.row < b.row + b.rowSpan &&
        b.row < a.row + a.rowSpan &&
        a.column < b.column + b.columnSpan &&
        b.column < a.column + a.columnSpan
    );
}
export function freeCell(
    parent: UiNode,
    layout: UiLayout,
    row = 1,
    column = 1,
): UiLayout {
    const columns = columnCount(parent.layout);
    if (layout.columnSpan > columns)
        throw new Error("组件跨列超过目标网格列数。");
    let index = Math.max(0, (row - 1) * columns + column - 1);
    for (let attempt = 0; attempt < 10000; attempt++, index++) {
        const next = {
            ...layout,
            row: Math.floor(index / columns) + 1,
            column: (index % columns) + 1,
        };
        if (next.row > 10000) break;
        if (
            next.column + next.columnSpan - 1 <= columns &&
            parent.children.every((n) => !cellsOverlap(n.layout, next))
        )
            return next;
    }
    throw new Error("目标网格没有可用位置。");
}
export interface Placement {
    x?: number;
    y?: number;
    row?: number;
    column?: number;
    index?: number;
}
export function moveNodes(
    document: UiDocument,
    ids: readonly string[],
    parentId: string,
    placement: Placement = {},
    catalog = BUILTIN_CATALOG,
): UiDocument {
    const nodes = selectionRoots(document, ids);
    const target = findNode(document.root, parentId);
    if (
        !nodes.length ||
        !target ||
        nodes.some((node) => walk(node).some((n) => n.id === parentId))
    )
        throw new Error("不能将组件移入自身或后代。");
    if (target.id !== document.root.id && catalog[target.type]?.composite)
        throw new Error("请打开复合组件定义编辑内部布局。");
    const selected = new Set(nodes.map((n) => n.id));
    const strip = (node: UiNode): UiNode => ({
        ...node,
        children: node.children.filter((n) => !selected.has(n.id)).map(strip),
    });
    const root = strip(document.root);
    let parent = findNode(root, parentId)!;
    const gridGroup =
        parent.layout.mode === "grid" &&
        nodes.every(
            (n) =>
                findParent(document.root, n.id)?.id ===
                findParent(document.root, nodes[0]!.id)?.id,
        ) &&
        findParent(document.root, nodes[0]!.id)?.layout.mode === "grid";
    const gridLayouts = new Map<string, UiLayout>();
    if (gridGroup) {
        const oldRow = Math.min(...nodes.map((n) => n.layout.row)),
            oldColumn = Math.min(...nodes.map((n) => n.layout.column));
        const columns = columnCount(parent.layout);
        if (
            nodes.length === 1 &&
            findParent(document.root, nodes[0]!.id)?.id === parentId &&
            placement.row &&
            placement.column
        ) {
            const node = nodes[0]!,
                desired = {
                    ...node.layout,
                    row: placement.row,
                    column: placement.column,
                };
            const collisions = parent.children.filter((n) =>
                cellsOverlap(n.layout, desired),
            );
            const other = collisions[0];
            if (
                collisions.length === 1 &&
                other &&
                other.layout.rowSpan === node.layout.rowSpan &&
                other.layout.columnSpan === node.layout.columnSpan &&
                other.layout.row === desired.row &&
                other.layout.column === desired.column
            ) {
                parent = {
                    ...parent,
                    children: parent.children.map((n) =>
                        n.id === other.id
                            ? {
                                  ...n,
                                  layout: {
                                      ...n.layout,
                                      row: node.layout.row,
                                      column: node.layout.column,
                                  },
                              }
                            : n,
                    ),
                };
            }
        }
        let index =
            ((placement.row ?? oldRow) - 1) * columns +
            (placement.column ?? oldColumn) -
            1;
        for (let attempt = 0; attempt < 10000; attempt++, index++) {
            const row = Math.floor(index / columns) + 1,
                column = (index % columns) + 1;
            const layouts = nodes.map((n) => ({
                ...n.layout,
                row: n.layout.row + row - oldRow,
                column: n.layout.column + column - oldColumn,
            }));
            if (
                layouts.every(
                    (layout) =>
                        layout.row <= 10000 &&
                        layout.column + layout.columnSpan - 1 <= columns &&
                        parent.children.every(
                            (n) => !cellsOverlap(n.layout, layout),
                        ),
                )
            ) {
                nodes.forEach((n, i) => gridLayouts.set(n.id, layouts[i]!));
                break;
            }
        }
        if (!gridLayouts.size) throw new Error("目标网格无法容纳所选组件。");
    }
    const oldBounds = boundsOf(nodes.map((n) => n.layout));
    let index = placement.index ?? target.children.length;
    index -= target.children
        .slice(0, index)
        .filter((n) => selected.has(n.id)).length;
    for (const node of nodes) {
        assertParent(parent, node, catalog);
        let layout = node.layout;
        if (parent.layout.mode === "absolute")
            layout = {
                ...layout,
                x: (placement.x ?? oldBounds.x) + node.layout.x - oldBounds.x,
                y: (placement.y ?? oldBounds.y) + node.layout.y - oldBounds.y,
            };
        if (parent.layout.mode === "grid")
            layout =
                gridLayouts.get(node.id) ??
                freeCell(parent, layout, placement.row, placement.column);
        const children = [...parent.children];
        children.splice(index++, 0, { ...node, layout });
        parent = { ...parent, children };
    }
    return { ...document, root: updateNode(root, parentId, () => parent) };
}
export function duplicateNodes(
    document: UiDocument,
    ids: readonly string[],
    catalog: Readonly<Record<string, ComponentSpec>>,
): { document: UiDocument; ids: string[] } {
    const { nodes, parent } = siblings(document, ids);
    const names = new Set(walk(document.root).map((n) => n.name));
    const rename = new Map<string, string>();
    for (const node of nodes.flatMap(walk)) {
        let name = `${node.name}Copy`,
            index = 2;
        while (names.has(name)) name = `${node.name}Copy${index++}`;
        names.add(name);
        rename.set(node.name, name);
    }
    const copy = (node: UiNode): UiNode => ({
        ...structuredClone(node),
        id: crypto.randomUUID(),
        name: rename.get(node.name)!,
        events: Object.fromEntries(
            Object.entries(node.events).map(([event, binding]) => {
                const [receiver, method] = binding.split(".");
                return [
                    event,
                    document.version !== 1 && method && rename.has(receiver!)
                        ? `${rename.get(receiver!)}.${method}`
                        : binding,
                ];
            }),
        ),
        children: node.children.map(copy),
    });
    let destination = parent;
    const copies = nodes.map((node) => {
        const next = copy(node);
        assertParent(destination, next, catalog);
        if (parent.layout.mode === "absolute")
            next.layout = {
                ...next.layout,
                x: next.layout.x + 16,
                y: next.layout.y + 16,
            };
        if (parent.layout.mode === "grid")
            next.layout = freeCell(destination, next.layout);
        destination = {
            ...destination,
            children: [...destination.children, next],
        };
        return next;
    });
    return {
        document: {
            ...document,
            root: updateNode(document.root, parent.id, () => destination),
        },
        ids: copies.map((n) => n.id),
    };
}
export type Arrangement =
    | "left"
    | "centerX"
    | "right"
    | "top"
    | "centerY"
    | "bottom"
    | "distributeX"
    | "distributeY"
    | "sameWidth"
    | "sameHeight";
export function arrangeNodes(
    document: UiDocument,
    ids: readonly string[],
    operation: Arrangement,
    rects: Rects = {},
): UiDocument {
    const { nodes, parent } = siblings(
        document,
        ids,
        operation.startsWith("distribute") ? 3 : 2,
    );
    const first = findNode(document.root, ids[0]!) ?? nodes[0]!;
    if (operation === "sameWidth" || operation === "sameHeight") {
        const width = operation === "sameWidth";
        return patchLayouts(
            document,
            nodes.map((n) => n.id),
            width
                ? { width: rectOf(first, rects).width, widthMode: "fixed" }
                : { height: rectOf(first, rects).height, heightMode: "fixed" },
        );
    }
    if (parent.layout.mode !== "absolute")
        throw new Error(
            "对齐和等间距适用于绝对布局；当前容器通过行列或顺序排列组件。",
        );
    const bounds = boundsOf(nodes.map((n) => rectOf(n, rects)));
    const positions = new Map<string, Partial<UiLayout>>();
    if (operation.startsWith("distribute")) {
        const x = operation === "distributeX",
            axis = x ? "x" : "y",
            size = x ? "width" : "height";
        const ordered = [...nodes].sort(
            (a, b) => rectOf(a, rects)[axis] - rectOf(b, rects)[axis],
        );
        const gap =
            (bounds[size] -
                ordered.reduce((sum, n) => sum + rectOf(n, rects)[size], 0)) /
            (nodes.length - 1);
        if (gap < 0) throw new Error("组件间空间不足，无法等间距分布。");
        let offset = bounds[axis];
        for (const node of ordered) {
            positions.set(node.id, { [axis]: offset });
            offset += rectOf(node, rects)[size] + gap;
        }
    } else
        for (const node of nodes) {
            const rect = rectOf(node, rects);
            const patch =
                operation === "left"
                    ? { x: bounds.x }
                    : operation === "right"
                      ? { x: bounds.x + bounds.width - rect.width }
                      : operation === "centerX"
                        ? { x: bounds.x + (bounds.width - rect.width) / 2 }
                        : operation === "top"
                          ? { y: bounds.y }
                          : operation === "bottom"
                            ? { y: bounds.y + bounds.height - rect.height }
                            : {
                                  y:
                                      bounds.y +
                                      (bounds.height - rect.height) / 2,
                              };
            positions.set(node.id, patch);
        }
    return patchNodes(
        document,
        nodes.map((n) => n.id),
        (node) => ({
            ...node,
            layout: { ...node.layout, ...positions.get(node.id) },
        }),
    );
}
export function reorderNodes(
    document: UiDocument,
    ids: readonly string[],
    direction: number,
): UiDocument {
    const { nodes, parent } = siblings(document, ids);
    const selected = new Set(nodes.map((n) => n.id)),
        children = [...parent.children];
    const indices =
        direction < 0
            ? children.map((_, i) => i)
            : children.map((_, i) => i).reverse();
    for (const i of indices) {
        const j = i + Math.sign(direction);
        if (
            children[j] &&
            selected.has(children[i]!.id) &&
            !selected.has(children[j]!.id)
        )
            [children[i], children[j]] = [children[j]!, children[i]!];
    }
    return {
        ...document,
        root: updateNode(document.root, parent.id, (n) => ({ ...n, children })),
    };
}
export function nudgeNodes(
    document: UiDocument,
    ids: readonly string[],
    dx: number,
    dy: number,
): UiDocument {
    const { nodes, parent } = siblings(document, ids);
    if (parent.layout.mode === "row" || parent.layout.mode === "column")
        return reorderNodes(document, ids, dx || dy);
    if (parent.layout.mode === "grid") {
        const selected = new Set(nodes.map((n) => n.id));
        const moved = nodes.map((node) => ({
            ...node,
            layout: {
                ...node.layout,
                row: node.layout.row + Math.sign(dy),
                column: node.layout.column + Math.sign(dx),
            },
        }));
        if (
            moved.some(
                (n) =>
                    n.layout.row < 1 ||
                    n.layout.column < 1 ||
                    n.layout.column + n.layout.columnSpan - 1 >
                        columnCount(parent.layout) ||
                    parent.children.some(
                        (other) =>
                            !selected.has(other.id) &&
                            cellsOverlap(other.layout, n.layout),
                    ),
            )
        )
            return document;
        return patchNodes(
            document,
            ids,
            (n) => moved.find((m) => m.id === n.id) ?? n,
        );
    }
    const bounds = boundsOf(nodes.map((n) => n.layout));
    dx = Math.max(-bounds.x, dx);
    dy = Math.max(-bounds.y, dy);
    return patchNodes(
        document,
        nodes.map((n) => n.id),
        (n) => ({
            ...n,
            layout: { ...n.layout, x: n.layout.x + dx, y: n.layout.y + dy },
        }),
    );
}
export function wrapNodes(
    document: UiDocument,
    ids: readonly string[],
    type: string,
    catalog: Readonly<Record<string, ComponentSpec>>,
    rects: Rects = {},
): { document: UiDocument; id: string } {
    const { nodes, parent } = siblings(document, ids);
    if (parent.type === "TabGroup" || nodes.some((n) => n.type === "Tab"))
        throw new Error("选项卡结构不能包入普通布局容器。");
    const wrapper = createNode(type, document.root, catalog[type]);
    const selected = new Set(nodes.map((n) => n.id));
    const bounds = boundsOf(nodes.map((n) => rectOf(n, rects)));
    wrapper.layout = {
        ...wrapper.layout,
        ...bounds,
        padding: 0,
        widthMode: "fill",
        heightMode: "fill",
    };
    if (parent.layout.mode === "absolute")
        wrapper.layout = {
            ...wrapper.layout,
            widthMode: "fixed",
            heightMode: "fixed",
        };
    if (parent.layout.mode === "row")
        wrapper.layout = { ...wrapper.layout, widthMode: "fixed" };
    if (parent.layout.mode === "column")
        wrapper.layout = { ...wrapper.layout, heightMode: "fixed" };
    if (parent.layout.mode === "grid") {
        const row = Math.min(...nodes.map((n) => n.layout.row)),
            column = Math.min(...nodes.map((n) => n.layout.column));
        wrapper.layout = {
            ...wrapper.layout,
            row,
            column,
            rowSpan:
                Math.max(...nodes.map((n) => n.layout.row + n.layout.rowSpan)) -
                row,
            columnSpan:
                Math.max(
                    ...nodes.map((n) => n.layout.column + n.layout.columnSpan),
                ) - column,
        };
        if (
            parent.children.some(
                (n) =>
                    !selected.has(n.id) &&
                    cellsOverlap(wrapper.layout, n.layout),
            )
        )
            throw new Error("请选择连续网格区域，避免覆盖未选择的组件。");
    }
    wrapper.children = nodes.map((node, index) => ({
        ...node,
        layout: {
            ...node.layout,
            x: rectOf(node, rects).x - bounds.x,
            y: rectOf(node, rects).y - bounds.y,
            row: Math.floor(index / wrapper.layout.columns) + 1,
            column: (index % wrapper.layout.columns) + 1,
            rowSpan: 1,
            columnSpan: 1,
        },
    }));
    let inserted = false;
    const children = parent.children.flatMap((node) => {
        if (!selected.has(node.id)) return [node];
        if (inserted) return [];
        inserted = true;
        return [wrapper];
    });
    return {
        document: {
            ...document,
            root: updateNode(document.root, parent.id, (n) => ({
                ...n,
                children,
            })),
        },
        id: wrapper.id,
    };
}
