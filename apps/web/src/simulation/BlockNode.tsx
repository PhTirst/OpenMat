import { memo, useState } from "react";
import {
    BaseEdge,
    EdgeLabelRenderer,
    Handle,
    Position,
    useReactFlow,
    type Edge,
    type EdgeProps,
    type Node,
    type NodeProps,
} from "@xyflow/react";
import {
    parameterText,
    ports,
    type Block,
    type BlockType,
    type Point,
} from "./model";

export function BlockIcon({
    type,
    size = 24,
}: {
    type: BlockType | "unknown";
    size?: number;
}) {
    const shapes = {
        mFunction: (
            <>
                <rect x="2" y="2" width="20" height="20" rx="3" />
                <path d="M5 16V8l4 5 4-5v8m3-6 3 2-3 2" />
            </>
        ),
        constant: (
            <>
                <rect x="3" y="3" width="18" height="18" rx="3" />
                <path d="m9 9 3-2v10m-3 0h6" />
            </>
        ),
        sum: (
            <>
                <circle cx="12" cy="12" r="9" />
                <path d="M12 7v10M7 12h10" />
            </>
        ),
        gain: (
            <>
                <path d="M3 3v18l18-9ZM8 9v6m0-3 4-3m-4 3 4 3" />
            </>
        ),
        integrator: (
            <>
                <rect x="2" y="2" width="20" height="20" rx="3" />
                <path d="M15 5c-5-2-3 16-7 14M7 12h10" />
            </>
        ),
        unitDelay: (
            <>
                <rect x="2" y="2" width="20" height="20" rx="3" />
                <path d="M6 10h7l-7 7h7m2-11h3m1-2v5" />
            </>
        ),
        scope: (
            <>
                <rect x="2" y="3" width="20" height="16" rx="3" />
                <path d="M5 14c3 0 2-7 5-7s2 7 5 7 2-5 4-5M8 22h8m-4-3v3" />
            </>
        ),
        unknown: (
            <>
                <rect x="3" y="3" width="18" height="18" rx="3" />
                <path d="M9 9a3 3 0 1 1 5 2c-2 1-2 2-2 3m0 3h.01" />
            </>
        ),
    };
    return (
        <svg
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
            {shapes[type]}
        </svg>
    );
}
export type BlockNodeData = {
    block: Block;
    label: string;
    invalid?: boolean;
    readonly?: boolean;
    detail?: string;
} & Record<string, unknown>;
export type FlowBlock = Node<BlockNodeData, "block">;
export const BlockNode = memo(function BlockNode({
    data,
    selected,
}: NodeProps<FlowBlock>) {
    const p = ports(data.block);
    return (
        <div
            className={`sim-block ${selected ? "is-selected" : ""} ${data.invalid ? "is-invalid" : ""}`}
            style={{ minHeight: Math.max(92, p.inputs.length * 28 + 36) }}
        >
            <div className="sim-block-title">
                <BlockIcon type={data.block.kind.type} />
                <span>{data.label}</span>
            </div>
            <div
                className="sim-block-value"
                title={data.detail ?? parameterText(data.block)}
            >
                {data.detail ?? parameterText(data.block)}
            </div>
            <div className="sim-block-kind">{data.block.kind.type}</div>
            {p.inputs.map((name, i) => (
                <Handle
                    key={name}
                    type="target"
                    position={Position.Left}
                    id={name}
                    isConnectable={!data.readonly}
                    style={{
                        top: `${((i + 1) / (p.inputs.length + 1)) * 100}%`,
                    }}
                    title={`${data.label}.${name} · 输入`}
                    aria-label={`${data.label} 输入 ${name}`}
                >
                    <span className="sim-port-name input">
                        {data.block.kind.type === "sum"
                            ? data.block.kind.signs[i] === 1
                                ? "+"
                                : "−"
                            : ""}
                    </span>
                </Handle>
            ))}
            {p.outputs.map((name) => (
                <Handle
                    key={name}
                    type="source"
                    position={Position.Right}
                    id={name}
                    isConnectable={!data.readonly}
                    title={`${data.label}.${name} · 输出`}
                    aria-label={`${data.label} 输出 ${name}`}
                />
            ))}
        </div>
    );
});

export type SignalEdgeData = {
    bend?: Point | undefined;
    commitBend?: (id: string, point: Point) => void;
} & Record<string, unknown>;
export type SignalEdge = Edge<SignalEdgeData, "signal">;
export const OrthogonalEdge = memo(function OrthogonalEdge(
    props: EdgeProps<SignalEdge>,
) {
    const {
        id,
        sourceX: sx,
        sourceY: sy,
        targetX: tx,
        targetY: ty,
        selected,
        markerEnd,
        data,
    } = props;
    const flow = useReactFlow();
    const [dragPoint, setDragPoint] = useState<Point | null>(null);
    const custom = dragPoint ?? data?.bend;
    const back = tx <= sx + 40;
    const bend =
        custom ??
        (back
            ? {
                  x: tx - 25,
                  y: ty - sy > 140 ? (sy + ty) / 2 : Math.max(sy, ty) + 100,
              }
            : { x: (sx + tx) / 2, y: (sy + ty) / 2 });
    const path =
        custom || back
            ? `M ${sx} ${sy} H ${sx + 25} V ${bend.y} H ${bend.x} V ${ty} H ${tx}`
            : `M ${sx} ${sy} H ${bend.x} V ${ty} H ${tx}`;
    return (
        <>
            <BaseEdge
                id={id}
                path={path}
                {...(markerEnd ? { markerEnd } : {})}
                style={{
                    stroke: selected ? "var(--accent)" : "var(--text-muted)",
                    strokeWidth: selected ? 2.3 : 1.6,
                }}
                interactionWidth={18}
            />
            {selected && data?.commitBend && (
                <EdgeLabelRenderer>
                    <button
                        className="sim-edge-bend nodrag nopan"
                        aria-label="拖动调整连线路径"
                        title="拖动或用方向键调整路径；右键可重置"
                        style={{
                            transform: `translate(-50%, -50%) translate(${bend.x}px,${bend.y}px)`,
                        }}
                        onKeyDown={(event) => {
                            if (
                                ![
                                    "ArrowLeft",
                                    "ArrowRight",
                                    "ArrowUp",
                                    "ArrowDown",
                                ].includes(event.key)
                            )
                                return;
                            event.preventDefault();
                            event.stopPropagation();
                            data.commitBend?.(id, {
                                x:
                                    bend.x +
                                    (event.key === "ArrowLeft"
                                        ? -10
                                        : event.key === "ArrowRight"
                                          ? 10
                                          : 0),
                                y:
                                    bend.y +
                                    (event.key === "ArrowUp"
                                        ? -10
                                        : event.key === "ArrowDown"
                                          ? 10
                                          : 0),
                            });
                        }}
                        onPointerDown={(event) => {
                            event.preventDefault();
                            event.stopPropagation();
                            event.currentTarget.setPointerCapture(
                                event.pointerId,
                            );
                            setDragPoint(bend);
                        }}
                        onPointerMove={(event) => {
                            if (
                                event.currentTarget.hasPointerCapture(
                                    event.pointerId,
                                )
                            )
                                setDragPoint(
                                    flow.screenToFlowPosition({
                                        x: event.clientX,
                                        y: event.clientY,
                                    }),
                                );
                        }}
                        onPointerUp={(event) => {
                            if (
                                !event.currentTarget.hasPointerCapture(
                                    event.pointerId,
                                )
                            )
                                return;
                            event.currentTarget.releasePointerCapture(
                                event.pointerId,
                            );
                            data.commitBend?.(
                                id,
                                flow.screenToFlowPosition({
                                    x: event.clientX,
                                    y: event.clientY,
                                }),
                            );
                            setDragPoint(null);
                        }}
                        onPointerCancel={() => setDragPoint(null)}
                    />
                </EdgeLabelRenderer>
            )}
        </>
    );
});
export const NODE_TYPES = { block: BlockNode };
export const EDGE_TYPES = { signal: OrthogonalEdge };
