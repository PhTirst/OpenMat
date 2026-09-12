import { memo } from "react";
import {
    Background,
    BackgroundVariant,
    Controls,
    MiniMap,
    ReactFlow,
    useStore,
    type ReactFlowProps,
    type ReactFlowState,
} from "@xyflow/react";
import {
    EDGE_TYPES,
    NODE_TYPES,
    type FlowBlock,
    type SignalEdge,
} from "./BlockNode";

const GRID: [number, number] = [10, 10];
const PAN_BUTTONS = [1, 2];
const FIT = { padding: 0.2, maxZoom: 1.15 };
const isOverview = (state: ReactFlowState) => state.transform[2] < 0.4;
/** Stable graph props keep Scope sample refreshes out of React Flow's store. */
export const SimulationCanvas = memo(function SimulationCanvas(
    props: ReactFlowProps<FlowBlock, SignalEdge>,
) {
    const overview = useStore(isOverview);
    return (
        <ReactFlow<FlowBlock, SignalEdge>
            {...props}
            className={overview ? "sim-overview" : ""}
            nodeTypes={NODE_TYPES}
            edgeTypes={EDGE_TYPES}
            fitViewOptions={FIT}
            minZoom={0.1}
            maxZoom={2.5}
            snapToGrid
            snapGrid={GRID}
            deleteKeyCode={null}
            multiSelectionKeyCode="Shift"
            selectionKeyCode="Shift"
            panActivationKeyCode="Space"
            selectionOnDrag
            panOnDrag={PAN_BUTTONS}
        >
            <Background
                variant={BackgroundVariant.Dots}
                gap={20}
                size={1.1}
                color="var(--border-strong)"
            />
            <Controls showInteractive={false} />
            <MiniMap
                pannable
                zoomable
                nodeColor="var(--accent-soft)"
                maskColor="var(--bg-pane)"
            />
        </ReactFlow>
    );
});
