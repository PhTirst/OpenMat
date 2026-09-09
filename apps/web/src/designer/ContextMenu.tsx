import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

export interface MenuAction {
    label: string;
    run: () => void;
    disabled?: boolean;
    shortcut?: string;
    icon?: ReactNode;
    danger?: boolean;
}
export interface MenuGroup {
    label?: string;
    actions: MenuAction[];
}

const actionPaths = {
    copy: "M8 8h12v13H8zM16 8V3H3v13h5",
    rename: "m4 16-1 5 5-1L21 7l-4-4ZM14 6l4 4",
    remove: "M3 6h18M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7",
    up: "m5 12 7-7 7 7M12 5v15",
    down: "m5 12 7 7 7-7M12 19V4",
    parent: "M19 19h-7V5m-6 6 6-6 6 6",
};
export function MenuIcon({ name }: { name: keyof typeof actionPaths }) {
    return (
        <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
        >
            <path d={actionPaths[name]} />
        </svg>
    );
}

export function ContextMenu({
    x,
    y,
    title,
    groups,
    onClose,
}: {
    x: number;
    y: number;
    title: string;
    groups: MenuGroup[];
    onClose: (restoreFocus: boolean) => void;
}) {
    const menuRef = useRef<HTMLDivElement>(null);
    const [position, setPosition] = useState({ x, y });
    useLayoutEffect(() => {
        const menu = menuRef.current!;
        const bounds = menu.getBoundingClientRect();
        setPosition({
            x: Math.max(8, Math.min(x, window.innerWidth - bounds.width - 8)),
            y: Math.max(8, Math.min(y, window.innerHeight - bounds.height - 8)),
        });
        menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({
            preventScroll: true,
        });
    }, [x, y]);
    useEffect(() => {
        const outside = (event: Event) => {
            if (
                event.target instanceof Node &&
                !menuRef.current?.contains(event.target)
            )
                onClose(false);
        };
        const close = () => onClose(false);
        window.addEventListener("pointerdown", outside, true);
        window.addEventListener("scroll", outside, true);
        window.addEventListener("resize", close);
        window.addEventListener("blur", close);
        return () => {
            window.removeEventListener("pointerdown", outside, true);
            window.removeEventListener("scroll", outside, true);
            window.removeEventListener("resize", close);
            window.removeEventListener("blur", close);
        };
    }, [onClose]);
    return (
        <div
            ref={menuRef}
            role="menu"
            aria-label="组件操作"
            className="designer-context-menu"
            style={{ left: position.x, top: position.y }}
            tabIndex={-1}
            onContextMenu={(event) => {
                event.preventDefault();
                event.stopPropagation();
            }}
            onKeyDown={(event) => {
                const shortcut = `${event.ctrlKey || event.metaKey ? "Ctrl+" : ""}${event.key.length === 1 ? event.key.toUpperCase() : event.key === "Backspace" ? "Delete" : event.key}`;
                const action = groups
                    .flatMap((group) => group.actions)
                    .find((item) => item.shortcut === shortcut);
                if (action) {
                    event.preventDefault();
                    event.stopPropagation();
                    if (!action.disabled) {
                        onClose(true);
                        action.run();
                    }
                    return;
                }
                const items = [
                    ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
                        "button:not(:disabled)",
                    ),
                ];
                const index = items.indexOf(
                    document.activeElement as HTMLButtonElement,
                );
                if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                    event.preventDefault();
                    event.stopPropagation();
                    items[
                        (index +
                            (event.key === "ArrowDown" ? 1 : -1) +
                            items.length) %
                            items.length
                    ]?.focus();
                } else if (event.key === "Home" || event.key === "End") {
                    event.preventDefault();
                    event.stopPropagation();
                    items[event.key === "Home" ? 0 : items.length - 1]?.focus();
                } else if (event.key === "Escape" || event.key === "Tab") {
                    event.preventDefault();
                    event.stopPropagation();
                    onClose(true);
                }
            }}
        >
            <div className="designer-context-title">{title}</div>
            {groups.map((group, index) => (
                <div
                    role="group"
                    aria-label={group.label}
                    className="designer-context-group"
                    key={index}
                >
                    {group.label && (
                        <div className="designer-context-heading">
                            {group.label}
                        </div>
                    )}
                    {group.actions.map((action) => (
                        <button
                            type="button"
                            role="menuitem"
                            tabIndex={-1}
                            key={action.label}
                            disabled={action.disabled}
                            className={action.danger ? "is-danger" : undefined}
                            onClick={() => {
                                onClose(true);
                                action.run();
                            }}
                        >
                            <span className="designer-context-icon" aria-hidden>
                                {action.icon}
                            </span>
                            <span>{action.label}</span>
                            {action.shortcut && <kbd>{action.shortcut}</kbd>}
                        </button>
                    ))}
                </div>
            ))}
        </div>
    );
}
