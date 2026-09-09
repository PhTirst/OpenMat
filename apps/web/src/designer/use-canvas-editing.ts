import {
    useEffect,
    useRef,
    useState,
    type PointerEvent as ReactPointerEvent,
    type RefObject,
} from "react";
import {
    findNode,
    findParent,
    walk,
    type UiDocument,
    type UiNode,
} from "./model";
import {
    boundsOf,
    patchNodes,
    rectOf,
    selectionRoots,
    type Rect,
} from "./operations";
import {
    intersects,
    layoutElement,
    marqueeRect,
    measureNodes,
    nodeElements,
    snapMove,
    type Guide,
} from "./canvas-geometry";

interface Visual {
    preview?: UiDocument;
    marquee?: Rect;
    guides: Guide[];
    scale: number;
}
export function resizeWindow(
    document: UiDocument,
    width: number,
    height: number,
): UiDocument {
    const component = document.kind === "component";
    width = Math.round(
        Math.max(
            component ? 24 : 240,
            Math.min(component ? 10000 : 4096, width),
        ),
    );
    height = Math.round(
        Math.max(
            component ? 24 : 160,
            Math.min(component ? 10000 : 4096, height),
        ),
    );
    return {
        ...document,
        root: component
            ? {
                  ...document.root,
                  layout: { ...document.root.layout, width, height },
              }
            : {
                  ...document.root,
                  properties: {
                      ...document.root.properties,
                      Width: width,
                      Height: height,
                  },
              },
    };
}
export function overlayPreview(root: UiNode, preview?: UiDocument): UiNode {
    if (!preview) return root;
    const nodes = new Map(walk(preview.root).map((node) => [node.id, node]));
    const visit = (current: UiNode): UiNode => {
        const node = nodes.get(current.id);
        return {
            ...current,
            ...(node
                ? {
                      layout: node.layout,
                      properties: { ...current.properties, ...node.properties },
                  }
                : {}),
            children: current.children.map(visit),
        };
    };
    return visit(root);
}
export function useCanvasEditing({
    document,
    ids,
    boardRef,
    scale,
    running,
    select,
    commit,
    onError,
}: {
    document: UiDocument;
    ids: string[];
    boardRef: RefObject<HTMLDivElement | null>;
    scale: number;
    running: boolean;
    select: (ids: string[]) => void;
    commit: (document: UiDocument) => boolean;
    onError: (error: unknown) => void;
}) {
    const [visual, setVisual] = useState<Visual | null>(null);
    const cleanup = useRef<(() => void) | null>(null);
    const suppressClick = useRef(false);
    const dragging = useRef(false);
    useEffect(() => () => cleanup.current?.(), []);
    const onPointerDown = (event: ReactPointerEvent) => {
        suppressClick.current = false;
        if (
            running ||
            event.button !== 0 ||
            !boardRef.current ||
            !(event.target instanceof Element)
        )
            return;
        const board = boardRef.current;
        const windowResize = event.target.closest("[data-window-resize]");
        const resizing = event.target.closest(".ui-resize");
        if (
            !resizing &&
            !windowResize &&
            event.target.closest("button, input, textarea, select, a")
        )
            return;
        let element = event.target.closest<HTMLElement>("[data-ui-id]");
        let node = element && findNode(document.root, element.dataset.uiId!);
        while (element && !node) {
            element =
                element.parentElement?.closest<HTMLElement>("[data-ui-id]") ??
                null;
            node = element && findNode(document.root, element.dataset.uiId!);
        }
        if (!node && !windowResize) return;
        const parent = node && findParent(document.root, node.id);
        const blank =
            !resizing &&
            !windowResize &&
            (node?.id === document.root.id ||
                event.target.classList.contains("ui-layout") ||
                event.target.classList.contains("ui-empty") ||
                event.target.classList.contains("ui-content"));
        if (
            !blank &&
            !resizing &&
            !windowResize &&
            parent?.layout.mode !== "absolute"
        )
            return;
        if (
            !blank &&
            !resizing &&
            !windowResize &&
            (event.shiftKey || event.ctrlKey || event.metaKey)
        )
            return;
        event.preventDefault();
        event.stopPropagation();
        cleanup.current?.();
        const startX = event.clientX,
            startY = event.clientY,
            boardBounds = board.getBoundingClientRect();
        const rectangles = measureNodes(board, document, scale),
            elements = nodeElements(board);
        let selectedIds =
            node && ids.includes(node.id) && !resizing
                ? ids
                : node
                  ? [node.id]
                  : [];
        let nodes = selectionRoots(document, selectedIds);
        if (
            nodes.some(
                (n) => findParent(document.root, n.id)?.id !== parent?.id,
            )
        ) {
            selectedIds = node ? [node.id] : [];
            nodes = selectionRoots(document, selectedIds);
        }
        if (!blank && !windowResize) select(selectedIds);
        const group = nodes.length
            ? boundsOf(nodes.map((n) => rectOf(n, rectangles)))
            : { x: 0, y: 0, width: 0, height: 0 };
        const parentElement = parent && elements.get(parent.id),
            parentLayout = parentElement && layoutElement(parentElement);
        const parentBounds = parentLayout?.getBoundingClientRect();
        const candidates =
            parent?.children
                .filter((n) => !selectedIds.includes(n.id))
                .map((n) => rectOf(n, rectangles)) ?? [];
        if (parentLayout)
            candidates.push({
                x: parent!.layout.padding,
                y: parent!.layout.padding,
                width: parentLayout.clientWidth - 2 * parent!.layout.padding,
                height: parentLayout.clientHeight - 2 * parent!.layout.padding,
            });
        let preview: UiDocument | undefined;
        let moved = false;
        dragging.current = true;
        const cancel = () => {
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", end);
            window.removeEventListener("pointercancel", abort);
            window.removeEventListener("keydown", key);
            cleanup.current = null;
            dragging.current = false;
            setVisual(null);
        };
        const move = (next: PointerEvent) => {
            if (
                Math.hypot(next.clientX - startX, next.clientY - startY) < 3 &&
                !moved
            )
                return;
            moved = true;
            const dx = (next.clientX - startX) / scale,
                dy = (next.clientY - startY) / scale;
            if (blank) {
                const marquee = marqueeRect(
                    (startX - boardBounds.left) / scale,
                    (startY - boardBounds.top) / scale,
                    (next.clientX - boardBounds.left) / scale,
                    (next.clientY - boardBounds.top) / scale,
                );
                const picked = (node?.children ?? [])
                    .filter((child) => {
                        const target = elements.get(child.id);
                        if (!target?.getClientRects().length) return false;
                        const bounds = target.getBoundingClientRect();
                        return intersects(marquee, {
                            x: (bounds.left - boardBounds.left) / scale,
                            y: (bounds.top - boardBounds.top) / scale,
                            width: bounds.width / scale,
                            height: bounds.height / scale,
                        });
                    })
                    .map((n) => n.id);
                select([
                    ...new Set([
                        ...(event.shiftKey || event.ctrlKey || event.metaKey
                            ? ids.filter((id) => id !== document.root.id)
                            : []),
                        ...picked,
                    ]),
                ]);
                setVisual({ marquee, guides: [], scale });
                return;
            }
            let guides: Guide[] = [];
            if (windowResize)
                preview = resizeWindow(
                    document,
                    boardBounds.width / scale + dx,
                    boardBounds.height / scale + dy,
                );
            else if (resizing) {
                const size = rectOf(node!, rectangles);
                const width = Math.max(
                    24,
                    Math.min(
                        10000,
                        Math.round((size.width + dx) / (next.altKey ? 1 : 8)) *
                            (next.altKey ? 1 : 8),
                    ),
                );
                const height = Math.max(
                    24,
                    Math.min(
                        10000,
                        Math.round((size.height + dy) / (next.altKey ? 1 : 8)) *
                            (next.altKey ? 1 : 8),
                    ),
                );
                preview = patchNodes(document, [node!.id], (n) => ({
                    ...n,
                    layout: {
                        ...n.layout,
                        width,
                        height,
                        widthMode: "fixed",
                        heightMode: "fixed",
                        grow: 0,
                    },
                }));
            } else {
                const snapped = snapMove(
                    group,
                    dx,
                    dy,
                    candidates,
                    6 / scale,
                    !next.altKey,
                );
                preview = patchNodes(
                    document,
                    nodes.map((n) => n.id),
                    (n) => ({
                        ...n,
                        layout: {
                            ...n.layout,
                            x: rectOf(n, rectangles).x + snapped.dx,
                            y: rectOf(n, rectangles).y + snapped.dy,
                        },
                    }),
                );
                guides = snapped.guides.map((guide) => ({
                    ...guide,
                    position:
                        guide.position +
                        (guide.axis === "x"
                            ? (parentBounds!.left - boardBounds.left) / scale -
                              parentLayout!.scrollLeft
                            : (parentBounds!.top - boardBounds.top) / scale -
                              parentLayout!.scrollTop),
                }));
            }
            setVisual({ preview, guides, scale });
        };
        const end = () => {
            if (moved) suppressClick.current = true;
            cancel();
            if (preview) {
                try {
                    commit(preview);
                } catch (error) {
                    onError(error);
                }
            }
        };
        const abort = () => {
            suppressClick.current = moved;
            cancel();
        };
        const key = (next: KeyboardEvent) => {
            if (next.key === "Escape") {
                next.preventDefault();
                abort();
            }
        };
        cleanup.current = cancel;
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", end);
        window.addEventListener("pointercancel", abort);
        window.addEventListener("keydown", key);
    };
    return { visual, onPointerDown, dragging, suppressClick };
}
