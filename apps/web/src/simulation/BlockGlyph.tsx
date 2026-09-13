import type { ReactNode } from "react";
import type { BlockKind } from "./model";

export function polynomial(values: number[], variable = "s"): string {
    const powers: Record<number, string> = {
        2: "²",
        3: "³",
        4: "⁴",
        5: "⁵",
        6: "⁶",
    };
    const terms: string[] = [];
    values.forEach((value, i) => {
        if (value === 0) return;
        const power = values.length - i - 1;
        const coefficient = Math.abs(value);
        const number = Number(coefficient.toPrecision(5)).toString();
        const term =
            power === 0
                ? number
                : `${coefficient === 1 ? "" : number}${variable}${power === 1 ? "" : (powers[power] ?? `^${power}`)}`;
        terms.push(`${value < 0 ? "−" : terms.length ? "+" : ""}${term}`);
    });
    return terms.join(" ") || "0";
}
export function glyphSize(k: BlockKind, inputs: number, outputs: number) {
    if (k.type === "standard" && ["mux", "demux"].includes(k.operation.type))
        return {
            width: 16,
            height: Math.max(64, 24 * Math.max(inputs, outputs)),
        };
    if (k.type === "sum")
        return { width: 58, height: Math.max(58, inputs * 22) };
    if (k.type === "gain") return { width: 88, height: 68 };
    if (k.type === "inport" || k.type === "outport")
        return { width: 54, height: 30 };
    if (k.type === "subsystem")
        return {
            width: 150,
            height: Math.max(76, Math.max(inputs, outputs) * 26 + 22),
        };
    if (
        k.type === "standard" &&
        ["transferFcn", "stateSpace"].includes(k.operation.type)
    )
        return { width: 164, height: 78 };
    if (k.type === "component" || k.type === "mFunction")
        return {
            width: 138,
            height: Math.max(68, Math.max(inputs, outputs) * 24 + 20),
        };
    return {
        width: k.type === "constant" ? 94 : 88,
        height: Math.max(58, inputs * 23),
    };
}
/** Independently drawn mathematical symbols, shared by native and imported views. */
export function BlockGlyph({
    kind: k,
    width: w,
    height: h,
    value,
    control,
    icon,
}: {
    kind: BlockKind;
    width: number;
    height: number;
    value?: string;
    control?: boolean;
    icon?: ReactNode;
}) {
    const type = k.type === "standard" ? k.operation.type : k.type;
    const fraction = (numerator: string, denominator: string) => (
        <>
            <text
                x={w / 2}
                y={h / 2 - 9}
                textLength={numerator.length > 16 ? w * 0.84 : undefined}
                lengthAdjust="spacingAndGlyphs"
            >
                <title>{numerator}</title>
                {numerator}
            </text>
            <path
                className="sim-glyph-mark"
                d={`M${w * 0.16} ${h / 2}H${w * 0.84}`}
            />
            <text
                x={w / 2}
                y={h / 2 + 18}
                textLength={denominator.length > 16 ? w * 0.84 : undefined}
                lengthAdjust="spacingAndGlyphs"
            >
                <title>{denominator}</title>
                {denominator}
            </text>
        </>
    );
    let body;
    if (control)
        body = (
            <>
                <path
                    className="sim-glyph-mark"
                    d={
                        k.type === "subsystem" &&
                        k.execution?.type === "triggered"
                            ? `M${w * 0.23} ${h * 0.65}H${w * 0.5}V${h * 0.3}H${w * 0.77}`
                            : `M${w * 0.3} ${h * 0.52}l${w * 0.15} ${h * 0.17} ${w * 0.27}-${h * 0.35}`
                    }
                />
            </>
        );
    else if (k.type === "sum")
        body = k.signs.map((sign, i) => (
            <text key={i} x={15} y={((i + 1) * h) / (k.signs.length + 1) + 4}>
                {sign === 1 ? "+" : "−"}
            </text>
        ));
    else if (type === "gain" || type === "constant")
        body = (
            <text
                x={type === "gain" ? w * 0.36 : w / 2}
                y={h / 2 + 5}
                textLength={value && value.length > 9 ? w * 0.72 : undefined}
                lengthAdjust="spacingAndGlyphs"
            >
                {value ?? "1"}
            </text>
        );
    else if (
        type === "integrator" ||
        (k.type === "resetIntegrator" && !k.discrete)
    )
        body = fraction("1", "s");
    else if (type === "unitDelay") body = fraction("1", "z");
    else if (type === "discreteIntegrator" || k.type === "resetIntegrator")
        body = fraction("K Ts", "z − 1");
    else if (k.type === "standard" && k.operation.type === "transferFcn")
        body = fraction(
            polynomial(k.operation.numerator),
            polynomial(k.operation.denominator),
        );
    else if (type === "stateSpace")
        body = (
            <>
                <text x={w / 2} y={h / 2 - 7}>
                    ẋ = Ax + Bu
                </text>
                <text x={w / 2} y={h / 2 + 15}>
                    y = Cx + Du
                </text>
            </>
        );
    else if (type === "product")
        body = (
            <text className="sim-glyph-large" x={w / 2} y={h / 2 + 10}>
                ×
            </text>
        );
    else if (type === "mux" || type === "demux") body = null;
    else if (k.type === "inport" || k.type === "outport")
        body = (
            <text x={w / 2} y={h / 2 + 5}>
                {k.port}
            </text>
        );
    else if (type === "scope")
        body = (
            <path
                className="sim-glyph-mark"
                d={`M${w * 0.2} ${h * 0.7}C${w * 0.3} ${h * 0.7} ${w * 0.27} ${h * 0.3} ${w * 0.4} ${h * 0.3}S${w * 0.48} ${h * 0.7} ${w * 0.6} ${h * 0.7} ${w * 0.68} ${h * 0.4} ${w * 0.8} ${h * 0.4}`}
            />
        );
    else if (type === "step" || type === "zeroOrderHold")
        body = (
            <path
                className="sim-glyph-mark"
                d={`M${w * 0.17} ${h * 0.7}H${w * 0.42}V${h * 0.45}H${w * 0.62}V${h * 0.25}H${w * 0.83}`}
            />
        );
    else if (type === "subsystem")
        body = (
            <>
                <rect
                    className="sim-glyph-mark"
                    x={w * 0.4}
                    y={h * 0.34}
                    width={w * 0.2}
                    height={h * 0.3}
                    rx={2}
                />
                <path
                    className="sim-glyph-mark"
                    d={`M${w * 0.27} ${h * 0.5}H${w * 0.4}M${w * 0.6} ${h * 0.5}H${w * 0.73}`}
                />
            </>
        );
    else if (icon)
        body = (
            <g transform={`translate(${w / 2 - 18} ${h / 2 - 18}) scale(1.5)`}>
                {icon}
            </g>
        );
    else
        body = (
            <text x={w / 2} y={h / 2 + 5}>
                {type === "mFunction" || type === "component"
                    ? "f(u)"
                    : type === "rateTransition"
                      ? "Ts₁ → Ts₂"
                      : (value ?? "f(u)")}
            </text>
        );
    return (
        <svg
            className={`sim-block-glyph glyph-${type}`}
            width={w}
            height={h}
            viewBox={`0 0 ${w} ${h}`}
            aria-hidden="true"
        >
            {type === "gain" && !control ? (
                <path
                    className="sim-glyph-shell"
                    d={`M1 1L${w - 1} ${h / 2}L1 ${h - 1}Z`}
                />
            ) : type === "sum" && !control ? (
                <ellipse
                    className="sim-glyph-shell"
                    cx={w / 2}
                    cy={h / 2}
                    rx={w / 2 - 1}
                    ry={h / 2 - 1}
                />
            ) : (
                <rect
                    className="sim-glyph-shell"
                    x={1}
                    y={1}
                    width={w - 2}
                    height={h - 2}
                    rx={
                        type === "inport" || type === "outport"
                            ? h / 2
                            : type === "mux" || type === "demux"
                              ? 0
                              : 2
                    }
                />
            )}
            {body}
        </svg>
    );
}
