import { useMemo, useState } from "react";
import type { SlxBlock, SlxImport, SlxLine } from "./client";
import { importLayout } from "./import-layout";
import { slxSystemPaths } from "./slx-authoring";
import { SlxBlockGlyph } from "./SlxBlockGlyph";
import { SlxParameterInspector } from "./SlxParameterInspector";

function rectangle(block: SlxBlock, index: number) {
    const values = (block.properties.Position ?? "")
        .replace(/[\[\]]/g, "")
        .split(/[,;\s]+/)
        .filter(Boolean)
        .map(Number);
    if (
        values.length === 4 &&
        values.every(Number.isFinite) &&
        values[2]! > values[0]! &&
        values[3]! > values[1]!
    )
        return {
            x: values[0]!,
            y: values[1]!,
            w: Math.max(80, values[2]! - values[0]!),
            h: Math.max(48, values[3]! - values[1]!),
        };
    return {
        x: (index % 5) * 180,
        y: Math.floor(index / 5) * 120,
        w: 120,
        h: 60,
    };
}
function endpoints(
    lines: SlxLine[],
    source?: string,
): { from: string; to: string }[] {
    return lines.flatMap((line) => {
        const from = line.properties.Src ?? source;
        const result =
            from && line.properties.Dst
                ? [
                      {
                          from: from.split("#")[0]!,
                          to: line.properties.Dst.split("#")[0]!,
                      },
                  ]
                : [];
        return [...result, ...endpoints(line.branches, from)];
    });
}

/** Inspection preserves imported block identities; unsupported blocks never become runnable placeholders. */
export default function SlxPreview({
    result,
    onBack,
    activeSystem,
    selectedSid,
    onSystem,
    onSelect,
}: {
    result: SlxImport;
    onBack(): void;
    activeSystem?: number;
    selectedSid?: string | null;
    onSystem?(index: number): void;
    onSelect?(sid: string): void;
}) {
    const [localSystem, setLocalSystem] = useState(0),
        [localSelected, setLocalSelected] = useState<string | null>(null);
    const systemIndex = activeSystem ?? localSystem,
        selected = selectedSid === undefined ? localSelected : selectedSid;
    const setSelected = onSelect ?? setLocalSelected;
    const setSystemIndex = (index: number) => {
        (onSystem ?? setLocalSystem)(index);
        setLocalSelected(null);
    };
    const paths = useMemo(
        () => slxSystemPaths(result.document),
        [result.document],
    );
    const childSystems = useMemo(
        () =>
            new Map(
                result.document.systems.map((system, index) => [
                    system.parentBlock,
                    index,
                ]),
            ),
        [result.document],
    );
    const ownerSystems = useMemo(
        () =>
            new Map(
                result.document.systems.flatMap((system, index) =>
                    system.blocks.map((b) => [b.sid, index] as const),
                ),
            ),
        [result.document],
    );
    const system =
        result.document.systems[systemIndex] ?? result.document.systems[0];
    const boxes = useMemo(() => {
        const original = (system?.blocks ?? []).map((block, index) => ({
            block,
            ...rectangle(block, index),
        }));
        const positions = importLayout(original);
        return original.map((box, index) => ({
            ...box,
            ...positions[index]!,
            w: 160,
            h: 72,
        }));
    }, [system]);
    const byId = useMemo(
        () => new Map(boxes.map((box) => [box.block.sid, box])),
        [boxes],
    );
    const lines = useMemo(() => endpoints(system?.lines ?? []), [system]);
    let minX = 0,
        minY = 0,
        maxX = 600,
        maxY = 300;
    for (const box of boxes) {
        minX = Math.min(minX, box.x - 40);
        minY = Math.min(minY, box.y - 40);
        maxX = Math.max(maxX, box.x + box.w + 40);
        maxY = Math.max(maxY, box.y + box.h + 40);
    }
    const block = byId.get(selected ?? "")?.block;
    return (
        <div className="sim-slx-preview">
            <div className="sim-slx-toolbar">
                <strong>{result.document.name}</strong>
                <span>{result.document.matlabRelease ?? "SLX"} · 原始结构</span>
                <select
                    aria-label="SLX 系统"
                    value={systemIndex}
                    onChange={(event) => {
                        setSystemIndex(Number(event.target.value));
                        setLocalSelected(null);
                    }}
                >
                    {result.document.systems.map((item, index) => (
                        <option key={index} value={index}>
                            {item.parentBlock
                                ? `子系统 ${item.parentBlock}`
                                : "根系统"}{" "}
                            ({item.blocks.length})
                        </option>
                    ))}
                </select>
                {system?.parentBlock && (
                    <button
                        onClick={() =>
                            setSystemIndex(
                                ownerSystems.get(system.parentBlock!) ?? 0,
                            )
                        }
                    >
                        返回上层
                    </button>
                )}
                <button onClick={onBack}>
                    {result.runnable ? "查看数值模型" : "返回当前模型"}
                </button>
            </div>
            <nav className="sim-slx-breadcrumb" aria-label="SLX 当前位置">
                {paths[systemIndex]}
            </nav>
            <div className="sim-slx-body">
                <svg
                    className="sim-slx-graph"
                    viewBox={`${minX} ${minY} ${maxX - minX} ${maxY - minY}`}
                    role="img"
                    aria-label="SLX 只读方块图"
                >
                    {lines.map((line, index) => {
                        const from = byId.get(line.from),
                            to = byId.get(line.to);
                        if (!from || !to) return null;
                        const sx = from.x + from.w,
                            sy = from.y + from.h / 2,
                            tx = to.x,
                            ty = to.y + to.h / 2,
                            middle = (sx + tx) / 2;
                        return (
                            <path
                                key={index}
                                d={`M ${sx} ${sy} H ${middle} V ${ty} H ${tx}`}
                                fill="none"
                                stroke="var(--text-muted)"
                                strokeWidth={1.5}
                            />
                        );
                    })}
                    {boxes.map((box) => (
                        <g
                            key={box.block.sid}
                            className={`sim-symbol-block ${selected === box.block.sid ? "is-selected" : ""}`}
                            role="button"
                            tabIndex={0}
                            aria-label={`${box.block.name} (${box.block.blockType})`}
                            onClick={() => setSelected(box.block.sid)}
                            onDoubleClick={() => {
                                const index = childSystems.get(box.block.sid);
                                if (index !== undefined) setSystemIndex(index);
                            }}
                            onKeyDown={(event) => {
                                if (
                                    event.key === "Enter" ||
                                    event.key === " "
                                ) {
                                    event.preventDefault();
                                    setSelected(box.block.sid);
                                }
                            }}
                        >
                            <g transform={`translate(${box.x},${box.y})`}>
                                <SlxBlockGlyph
                                    block={box.block}
                                    width={box.w}
                                    height={box.h}
                                />
                            </g>
                            <rect
                                x={box.x}
                                y={box.y}
                                width={box.w}
                                height={box.h}
                                rx={5}
                                fill="transparent"
                                stroke="none"
                                strokeWidth={
                                    selected === box.block.sid ? 3 : 1.5
                                }
                            />
                            <text
                                x={box.x + box.w / 2}
                                y={box.y + box.h + 17}
                                textAnchor="middle"
                                fill="var(--text)"
                                fontSize={12}
                            >
                                {box.block.name.slice(0, 20)}
                            </text>
                            {childSystems.has(box.block.sid) && (
                                <text
                                    x={box.x + box.w - 12}
                                    y={box.y + 15}
                                    textAnchor="end"
                                    fill="var(--accent)"
                                    fontSize={12}
                                >
                                    ↳
                                </text>
                            )}
                            <text
                                x={box.x + box.w / 2}
                                y={box.y + box.h / 2 + 14}
                                textAnchor="middle"
                                fill="var(--text-muted)"
                                fontSize={10}
                                display="none"
                            >
                                {box.block.blockType}
                            </text>
                        </g>
                    ))}
                </svg>
                {!onSelect && (
                    <aside className="sim-slx-properties">
                        {block ? (
                            <>
                                <strong>{block.name}</strong>
                                <p>
                                    SID {block.sid} · {block.blockType}
                                </p>
                                <small>{block.source.part}</small>
                                <SlxParameterInspector block={block} />
                            </>
                        ) : (
                            <p>
                                选择方块查看 SLX
                                原始参数。底部诊断列出不能运行的原因。
                            </p>
                        )}
                    </aside>
                )}
            </div>
        </div>
    );
}
