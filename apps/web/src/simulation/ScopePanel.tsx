import type { SamplingPlan } from "./sampling";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { ScopeInfo, SimulationFrame } from "./client";
import { envelope, scopeFrames } from "./scope-data";
const COLORS = [
    "#2689da",
    "#d88034",
    "#31a67c",
    "#a16bce",
    "#d65978",
    "#619d36",
    "#aa7d41",
    "#39a3ab",
];
export const ScopePanel = memo(function ScopePanel({
    frames,
    scopes,
    sampling,
    version,
    names,
    dark,
}: {
    frames: readonly SimulationFrame[];
    scopes: readonly ScopeInfo[];
    sampling?: SamplingPlan | undefined;
    version: number;
    names: Record<string, string>;
    dark: boolean;
}) {
    const canvas = useRef<HTMLCanvasElement>(null);
    const [selected, setSelected] = useState("");
    const [stairs, setStairs] = useState(false);
    const [showEvents, setShowEvents] = useState(true);
    const eventFrames = useMemo(
        () => frames.filter((f) => f.events?.length),
        [frames, version],
    );
    const eventCount = eventFrames.reduce(
        (sum, f) => sum + f.events!.length,
        0,
    );
    const recentEvents = eventFrames
        .slice(-50)
        .flatMap((f) => f.events!.map((event) => ({ ...event, time: f.time })))
        .slice(-50);
    const [startChannel, setStartChannel] = useState(0);
    const scope = scopes.find((item) => item.block === selected) ?? scopes[0];
    const observed = useMemo(
        () => scopeFrames(frames, scope, sampling),
        [frames, scope, sampling, version],
    );
    const stepped =
        stairs ||
        scope?.sampleTime?.kind === "discrete" ||
        scope?.sampleTime?.kind === "constant";
    const offset = scope?.offset ?? 0,
        width = scope?.width ?? 0;
    const firstChannel = Math.min(
        Number.isFinite(startChannel)
            ? Math.max(0, Math.floor(startChannel))
            : 0,
        Math.max(0, width - 1),
    );
    const shown = Math.min(8, width - firstChannel);
    useEffect(() => {
        const target = canvas.current;
        if (!target) return;
        let scheduled = 0;
        const draw = () => {
            const rect = target.getBoundingClientRect(),
                ratio = window.devicePixelRatio || 1;
            if (rect.width < 10 || rect.height < 10) return;
            target.width = Math.round(rect.width * ratio);
            target.height = Math.round(rect.height * ratio);
            const ctx = target.getContext("2d");
            if (!ctx) return;
            ctx.scale(ratio, ratio);
            const w = rect.width,
                h = rect.height,
                left = 58,
                right = w - 18,
                top = 18,
                bottom = h - 32;
            ctx.fillStyle = dark ? "#20252b" : "#fff";
            ctx.fillRect(0, 0, w, h);
            ctx.font = '11px "Segoe UI", sans-serif';
            if (!observed.length || !width) {
                ctx.fillStyle = dark ? "#a3afbe" : "#667488";
                ctx.textAlign = "center";
                ctx.fillText(
                    "连接 Scope 并运行模型，查看真实仿真曲线",
                    w / 2,
                    h / 2,
                );
                return;
            }
            let low = Infinity,
                high = -Infinity;
            const series = Array.from({ length: shown }, (_, ch) =>
                envelope(observed, offset + firstChannel + ch, right - left),
            );
            series.forEach((points) =>
                points.forEach((point) => {
                    low = Math.min(low, point.value);
                    high = Math.max(high, point.value);
                }),
            );
            if (!Number.isFinite(low) || !Number.isFinite(high)) return;
            const padding = Math.max(
                (high - low) * 0.08,
                Math.abs(high) * 0.01,
                1e-6,
            );
            low -= padding;
            high += padding;
            const begin = frames[0]!.time,
                end = frames.at(-1)!.time;
            const dx = Math.max(end - begin, 1e-9),
                dy = high - low;
            ctx.lineWidth = 1;
            for (let i = 0; i <= 4; i++) {
                const x = left + ((right - left) * i) / 4,
                    y = top + ((bottom - top) * i) / 4;
                ctx.strokeStyle = dark ? "#39424d" : "#e6ebf1";
                ctx.beginPath();
                ctx.moveTo(x, top);
                ctx.lineTo(x, bottom);
                ctx.moveTo(left, y);
                ctx.lineTo(right, y);
                ctx.stroke();
                ctx.fillStyle = dark ? "#a3afbe" : "#667488";
                ctx.textAlign = "center";
                ctx.fillText(
                    String(
                        Number(
                            (begin + ((end - begin) * i) / 4).toPrecision(4),
                        ),
                    ),
                    x,
                    bottom + 18,
                );
                ctx.textAlign = "right";
                ctx.fillText(
                    (high - (dy * i) / 4).toPrecision(4),
                    left - 7,
                    y + 4,
                );
            }
            ctx.save();
            ctx.beginPath();
            ctx.rect(left, top, right - left, bottom - top);
            ctx.clip();
            series.forEach((points, ch) => {
                ctx.strokeStyle = COLORS[ch]!;
                ctx.lineWidth = 1.7;
                ctx.beginPath();
                let previousY = 0;
                points.forEach((point, i) => {
                    const x =
                            left + ((point.time - begin) / dx) * (right - left),
                        y =
                            bottom -
                            ((point.value - low) / dy) * (bottom - top);
                    if (!i) ctx.moveTo(x, y);
                    else {
                        if (stepped) ctx.lineTo(x, previousY);
                        ctx.lineTo(x, y);
                    }
                    previousY = y;
                });
                if (points.length === 1) {
                    const point = points[0]!;
                    ctx.arc(
                        left + ((point.time - begin) / dx) * (right - left),
                        bottom - ((point.value - low) / dy) * (bottom - top),
                        2,
                        0,
                        2 * Math.PI,
                    );
                }
                ctx.stroke();
            });
            if (showEvents && eventFrames.length) {
                const columns = new Map<number, boolean>();
                for (const frame of eventFrames) {
                    const x = Math.round(
                        left + ((frame.time - begin) / dx) * (right - left),
                    );
                    columns.set(
                        x,
                        (columns.get(x) ?? false) ||
                            frame.events!.some((e) => e.kind === "reset"),
                    );
                }
                ctx.setLineDash([3, 4]);
                ctx.lineWidth = 1;
                for (const [x, reset] of columns) {
                    ctx.strokeStyle = reset
                        ? "#d65978"
                        : dark
                          ? "#98a8bb"
                          : "#8694a4";
                    ctx.beginPath();
                    ctx.moveTo(x, top);
                    ctx.lineTo(x, bottom);
                    ctx.stroke();
                }
                ctx.setLineDash([]);
            }
            ctx.restore();
            ctx.fillStyle = dark ? "#a3afbe" : "#667488";
            ctx.textAlign = "right";
            ctx.fillText("t (s)", right, h - 2);
        };
        const schedule = () => {
            cancelAnimationFrame(scheduled);
            scheduled = requestAnimationFrame(draw);
        };
        const observer = new ResizeObserver(schedule);
        observer.observe(target);
        schedule();
        return () => {
            observer.disconnect();
            cancelAnimationFrame(scheduled);
        };
    }, [
        frames,
        observed,
        offset,
        width,
        shown,
        firstChannel,
        dark,
        stepped,
        showEvents,
        eventFrames,
    ]);
    return (
        <div className="sim-scope">
            <div className="sim-scope-toolbar">
                <span aria-label="Scope 记录点数">
                    {observed.length} 个记录点
                </span>
                <label>
                    Scope{" "}
                    <select
                        aria-label="选择 Scope"
                        value={scope?.block ?? ""}
                        onChange={(event) => {
                            setSelected(event.target.value);
                            setStartChannel(0);
                        }}
                    >
                        {!scopes.length && <option value="">无观察器</option>}
                        {scopes.map((item) => (
                            <option key={item.block} value={item.block}>
                                {Object.hasOwn(names, item.block)
                                    ? names[item.block]
                                    : item.block}{" "}
                                ({item.width})
                            </option>
                        ))}
                    </select>
                </label>
                <label>
                    <input
                        type="checkbox"
                        checked={stepped}
                        disabled={
                            scope?.sampleTime?.kind === "discrete" ||
                            scope?.sampleTime?.kind === "constant"
                        }
                        onChange={(event) => setStairs(event.target.checked)}
                    />
                    阶梯显示
                </label>
                {width > 8 && (
                    <label>
                        首通道
                        <input
                            aria-label="首通道"
                            type="number"
                            min={1}
                            max={width}
                            value={firstChannel + 1}
                            onChange={(event) =>
                                setStartChannel(
                                    Math.max(0, Number(event.target.value) - 1),
                                )
                            }
                        />
                    </label>
                )}
                <span className="sim-scope-legend">
                    {Array.from({ length: shown }, (_, i) => (
                        <span key={i} style={{ color: COLORS[i] }}>
                            ● y[{firstChannel + i + 1}]
                        </span>
                    ))}
                </span>
                {!sampling && (
                    <span>{frames.length.toLocaleString()} samples</span>
                )}
            </div>
            {eventCount > 0 && (
                <div className="sim-event-toolbar">
                    <label>
                        <input
                            type="checkbox"
                            checked={showEvents}
                            onChange={(e) => setShowEvents(e.target.checked)}
                        />
                        事件标记
                    </label>
                    <details>
                        <summary>
                            模型事件 {eventCount} 次 · 复位为红色虚线
                        </summary>
                        <ol aria-label="模型事件记录">
                            {recentEvents.map((event, index) => (
                                <li key={index}>
                                    <time>
                                        {Number(event.time.toPrecision(9))} s
                                    </time>
                                    {" · "}
                                    {names[event.block] ?? event.block}
                                    {" · "}
                                    {event.kind === "reset" ? "复位" : "穿越"}
                                    {event.direction > 0 ? " ↑" : " ↓"}
                                    {" · "}
                                    {event.surface}
                                </li>
                            ))}
                        </ol>
                        {eventCount > 50 && (
                            <p>
                                显示最近 50 次事件；仿真结果保留完整事件记录。
                            </p>
                        )}
                    </details>
                </div>
            )}
            <canvas ref={canvas} aria-label="Scope 仿真结果曲线" role="img" />
        </div>
    );
});
