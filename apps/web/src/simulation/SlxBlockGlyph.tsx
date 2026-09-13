import type { SlxBlock } from "./client";
import type { BlockKind } from "./model";
import { BlockGlyph } from "./BlockGlyph";

export function SlxBlockGlyph({
    block,
    width,
    height,
}: {
    block: SlxBlock;
    width: number;
    height: number;
}) {
    const props = block.properties;
    let kind: BlockKind | undefined;
    let value: string | undefined;
    switch (block.blockType) {
        case "Constant":
            kind = { type: "constant", value: [1] };
            value = props.Value ?? "1";
            break;
        case "Gain":
            kind = { type: "gain", gain: [1] };
            value = props.Gain ?? "K";
            break;
        case "Sum": {
            const raw = props.Inputs ?? "++";
            const signs = /^[1-9]\d?$/.test(raw)
                ? "+".repeat(Math.min(64, Number(raw)))
                : raw.replace(/[^+-]/g, "").slice(0, 64);
            kind = {
                type: "sum",
                signs: [...signs].map((s) => (s === "+" ? 1 : -1)),
            };
            break;
        }
        case "Integrator":
            kind = { type: "integrator", initial: [0] };
            break;
        case "DiscreteIntegrator":
            kind = { type: "discreteIntegrator", initial: [0], gain: 1 };
            break;
        case "UnitDelay":
            kind = { type: "unitDelay", initial: [0] };
            break;
        case "ZeroOrderHold":
            kind = { type: "zeroOrderHold" };
            break;
        case "Product":
            kind = {
                type: "standard",
                operation: { type: "product", operations: "**" },
            };
            break;
        case "Mux":
            kind = { type: "standard", operation: { type: "mux", inputs: 2 } };
            break;
        case "Demux":
            kind = {
                type: "standard",
                operation: { type: "demux", widths: [1, 1] },
            };
            break;
        case "StateSpace":
            kind = {
                type: "standard",
                operation: {
                    type: "stateSpace",
                    a: [[1]],
                    b: [[1]],
                    c: [[1]],
                    d: [[1]],
                    initial: [0],
                },
            };
            break;
        case "Scope":
            kind = { type: "scope" };
            break;
        case "Inport":
        case "Outport":
            kind = {
                type: block.blockType === "Inport" ? "inport" : "outport",
                port: Number(props.Port ?? "1") || 1,
            };
            break;
        case "Step":
            kind = { type: "step", time: 1, before: [0], after: [1] };
            break;
        case "SubSystem":
            kind = { type: "subsystem", inputs: 1, outputs: 1 };
            break;
        case "EnablePort":
        case "TriggerPort":
            kind = {
                type: "subsystem",
                inputs: 0,
                outputs: 0,
                execution:
                    block.blockType === "EnablePort"
                        ? {
                              type: "enabled",
                              period: 1,
                              statesWhenEnabling: "held",
                              outputs: [],
                          }
                        : {
                              type: "triggered",
                              period: 1,
                              edge: "rising",
                              outputs: [],
                          },
            };
            break;
    }
    if (kind)
        return (
            <BlockGlyph
                kind={kind}
                width={width}
                height={height}
                {...(value ? { value } : {})}
                control={
                    block.blockType === "EnablePort" ||
                    block.blockType === "TriggerPort"
                }
            />
        );
    return (
        <svg
            width={width}
            height={height}
            viewBox={`0 0 ${width} ${height}`}
            className="sim-block-glyph"
            aria-hidden="true"
        >
            <rect
                className="sim-glyph-shell"
                x={1}
                y={1}
                width={width - 2}
                height={height - 2}
                rx={2}
            />
            {block.blockType === "TransferFcn" ? (
                <>
                    <text x={width / 2} y={height / 2 - 9}>
                        {props.Numerator ?? "num(s)"}
                    </text>
                    <path
                        className="sim-glyph-mark"
                        d={`M${width * 0.1} ${height / 2}H${width * 0.9}`}
                    />
                    <text x={width / 2} y={height / 2 + 18}>
                        {props.Denominator ?? "den(s)"}
                    </text>
                </>
            ) : (
                <text x={width / 2} y={height / 2 + 4}>
                    {block.blockType}
                </text>
            )}
        </svg>
    );
}
