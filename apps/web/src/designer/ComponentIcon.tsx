import type { ReactNode } from "react";

// Every palette type has its own geometry, reused in the tree and inspector.
const paths: Record<string, ReactNode> = {
    Window: (
        <>
            <rect x="2" y="3" width="20" height="18" rx="2" />
            <path d="M2 8h20M5 5.5h1m2 0h1" />
        </>
    ),
    Panel: (
        <>
            <rect x="2" y="4" width="20" height="16" rx="2" />
            <path d="M6 4v3h7V4M5 11h14" />
        </>
    ),
    ScrollPanel: (
        <>
            <rect x="2" y="3" width="20" height="18" rx="2" />
            <path d="M6 8h8M6 12h8M6 16h5M18 6v12m-2-10 2-2 2 2m-4 8 2 2 2-2" />
        </>
    ),
    TabGroup: (
        <>
            <path d="M2 8h20v12H2zM2 8V4h7v4m0-4h6v4m0-4h7v4" />
            <path d="M6 12h7" />
        </>
    ),
    Tab: (
        <>
            <path d="M2 9V4h9l3 5h8v11H2zM5 13h10M5 16h7" />
        </>
    ),
    GridLayout: (
        <>
            <rect x="3" y="3" width="18" height="18" rx="1" />
            <path d="M3 9h18M3 15h18M9 3v18M15 3v18" />
        </>
    ),
    RowLayout: (
        <>
            <rect x="2" y="5" width="5" height="14" rx="1" />
            <rect x="9.5" y="5" width="5" height="14" rx="1" />
            <rect x="17" y="5" width="5" height="14" rx="1" />
        </>
    ),
    ColumnLayout: (
        <>
            <rect x="4" y="2" width="16" height="5" rx="1" />
            <rect x="4" y="9.5" width="16" height="5" rx="1" />
            <rect x="4" y="17" width="16" height="5" rx="1" />
        </>
    ),
    SplitPane: (
        <>
            <rect x="2" y="4" width="20" height="16" rx="1" />
            <path d="M12 4v16m-3-9-2 1 2 1m6-2 2 1-2 1" />
        </>
    ),
    Label: (
        <>
            <path d="M4 5h16M12 5v15M8 20h8M4 5v3m16-3v3" />
        </>
    ),
    Button: (
        <>
            <rect x="2" y="6" width="20" height="12" rx="3" />
            <path d="M7 12h10M14 9l3 3-3 3" />
        </>
    ),
    TextField: (
        <>
            <rect x="2" y="5" width="20" height="14" rx="2" />
            <path d="M6 9v6M10 12h8M4 9h4m-4 6h4" />
        </>
    ),
    TextArea: (
        <>
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <path d="M7 7h10M7 11h10M7 15h6m4 3 2-2" />
        </>
    ),
    NumericField: (
        <>
            <rect x="2" y="5" width="20" height="14" rx="2" />
            <path d="M6 10l2-1v6m-2 0h4m5-10v14m2-10 1.5-2L20 9m-3 6 1.5 2 1.5-2" />
        </>
    ),
    CheckBox: (
        <>
            <rect x="3" y="3" width="18" height="18" rx="3" />
            <path d="m7 12 3 3 7-7" />
        </>
    ),
    RadioGroup: (
        <>
            <circle cx="6" cy="7" r="3" />
            <circle cx="6" cy="17" r="3" />
            <circle cx="6" cy="7" r=".7" />
            <path d="M13 7h8m-8 10h8" />
        </>
    ),
    DropDown: (
        <>
            <rect x="2" y="5" width="20" height="14" rx="2" />
            <path d="M6 12h5m4-2 3 4 3-4" />
        </>
    ),
    Slider: (
        <>
            <path d="M2 12h7m6 0h7M4 17v2m8-2v2m8-2v2" />
            <circle cx="12" cy="12" r="3" />
        </>
    ),
    Table: (
        <>
            <rect x="2" y="3" width="20" height="18" rx="1" />
            <path d="M2 8h20M2 14h20M9 3v18M16 3v18" />
            <path d="M3 4h18v3H3z" fill="currentColor" opacity=".18" />
        </>
    ),
    Image: (
        <>
            <rect x="2" y="3" width="20" height="18" rx="2" />
            <circle cx="8" cy="8" r="2" />
            <path d="m2 18 6-5 4 3 4-6 6 8" />
        </>
    ),
    PlotView: (
        <>
            <path d="M3 3v18h19M6 15l4-7 4 9 4-12 3 5" />
            <path d="M3 9h2m-2 6h2m5 6v-2m7 2v-2" />
        </>
    ),
    ProgressBar: (
        <>
            <rect x="2" y="7" width="20" height="10" rx="3" />
            <path d="M5 10v4m3-4v4m3-4v4m3-4v4" />
        </>
    ),
    StatusLamp: (
        <>
            <circle cx="12" cy="12" r="6" />
            <path d="M12 1v2m0 18v2M1 12h2m18 0h2M4 4l2 2m12 12 2 2M4 20l2-2M18 6l2-2" />
        </>
    ),
    ComponentContainer: (
        <>
            <path d="m12 2 10 5v10l-10 5-10-5V7zM2 7l10 5 10-5M12 12v10M7 4.5l10 5v5" />
        </>
    ),
};
export const COMPONENT_ICON_TYPES = Object.keys(paths);
export function ComponentIcon({
    type,
    size = 18,
}: {
    type: string;
    size?: number;
}) {
    return (
        <svg
            data-component-icon={type}
            width={size}
            height={size}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
        >
            {paths[type] ?? paths.ComponentContainer}
        </svg>
    );
}
