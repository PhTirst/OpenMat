import {
    useEffect,
    useLayoutEffect,
    useRef,
    useState,
    type ReactNode,
} from "react";

const STORAGE_KEY = "openmat.designer.library-split.v1";
const DEFAULT_SPLIT = 60;
const clamp = (value: number, min: number, max: number) =>
    Math.max(min, Math.min(max, value));

export function LibraryPanels({
    palette,
    tree,
    storageKey = STORAGE_KEY,
}: {
    palette: ReactNode;
    tree: ReactNode;
    storageKey?: string;
}) {
    const [split, setSplit] = useState(() => {
        try {
            const saved = JSON.parse(
                localStorage.getItem(storageKey) ?? "null",
            );
            if (typeof saved === "number" && Number.isFinite(saved))
                return clamp(saved, 10, 90);
        } catch {
            /* Use the default if browser preferences are unavailable. */
        }
        return DEFAULT_SPLIT;
    });
    const [preview, setPreview] = useState<number | null>(null);
    const [height, setHeight] = useState(500);
    const container = useRef<HTMLElement>(null);
    const cleanup = useRef<(() => void) | null>(null);
    const available = Math.max(1, height - 8);
    const minimum = Math.max(10, Math.min(45, (140 / available) * 100));
    const maximum = Math.min(90, 100 - Math.min(45, (110 / available) * 100));
    const effective = clamp(preview ?? split, minimum, maximum);
    useLayoutEffect(() => {
        const element = container.current!;
        const measure = () => {
            if (element.clientHeight) setHeight(element.clientHeight);
        };
        measure();
        if (typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver(measure);
        observer.observe(element);
        return () => observer.disconnect();
    }, []);
    useEffect(() => {
        try {
            localStorage.setItem(storageKey, JSON.stringify(split));
        } catch {
            /* Resizing still works without storage. */
        }
    }, [split, storageKey]);
    useEffect(() => () => cleanup.current?.(), []);
    return (
        <aside
            ref={container}
            className="designer-library"
            style={{
                gridTemplateRows: `minmax(0,${effective}fr) 8px minmax(0,${100 - effective}fr)`,
            }}
        >
            {palette}
            <div
                role="separator"
                aria-label="组件库与对象树高度"
                aria-orientation="horizontal"
                aria-valuemin={Math.round(minimum)}
                aria-valuemax={Math.round(maximum)}
                aria-valuenow={Math.round(effective)}
                aria-valuetext={`组件库 ${Math.round(effective)}%，对象树 ${100 - Math.round(effective)}%`}
                tabIndex={0}
                className={`designer-library-divider${preview !== null ? " is-dragging" : ""}`}
                title="上下拖动调整高度；双击恢复默认比例"
                onDoubleClick={() => setSplit(DEFAULT_SPLIT)}
                onKeyDown={(event) => {
                    if (
                        !["ArrowUp", "ArrowDown", "Home", "End"].includes(
                            event.key,
                        )
                    )
                        return;
                    event.preventDefault();
                    event.stopPropagation();
                    setSplit(
                        event.key === "Home"
                            ? minimum
                            : event.key === "End"
                              ? maximum
                              : clamp(
                                    effective +
                                        (event.key === "ArrowUp" ? -1 : 1) *
                                            (event.shiftKey ? 10 : 5),
                                    minimum,
                                    maximum,
                                ),
                    );
                }}
                onPointerDown={(event) => {
                    if (event.button !== 0) return;
                    event.preventDefault();
                    event.stopPropagation();
                    event.currentTarget.focus();
                    cleanup.current?.();
                    const start = event.clientY;
                    const usable = Math.max(
                        1,
                        (container.current?.getBoundingClientRect().height ||
                            height) - 8,
                    );
                    let next = effective;
                    const move = (e: PointerEvent) => {
                        next = clamp(
                            effective + ((e.clientY - start) / usable) * 100,
                            minimum,
                            maximum,
                        );
                        setPreview(next);
                    };
                    const release = () => {
                        window.removeEventListener("pointermove", move);
                        window.removeEventListener("pointerup", end);
                        window.removeEventListener("pointercancel", cancel);
                        window.removeEventListener("keydown", key);
                        window.removeEventListener("blur", cancel);
                        cleanup.current = null;
                    };
                    const end = () => {
                        release();
                        setPreview(null);
                        setSplit(next);
                    };
                    const cancel = () => {
                        release();
                        setPreview(null);
                    };
                    const key = (e: KeyboardEvent) => {
                        if (e.key === "Escape") {
                            e.preventDefault();
                            cancel();
                        }
                    };
                    cleanup.current = release;
                    window.addEventListener("pointermove", move);
                    window.addEventListener("pointerup", end);
                    window.addEventListener("pointercancel", cancel);
                    window.addEventListener("keydown", key);
                    window.addEventListener("blur", cancel);
                }}
            >
                <span />
            </div>
            {tree}
        </aside>
    );
}
