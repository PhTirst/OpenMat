export type SampleTime =
    | { kind: "inherited" | "continuous" | "constant" }
    | { kind: "discrete"; period: number };
export interface SamplingPlan {
    clocks: { id: number; ticks: number; period: number }[];
    blocks: Record<string, SampleTime>;
}

export function parseSampleTime(raw: unknown): SampleTime {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
        throw new Error("采样时间必须是对象。");
    const r = raw as Record<string, unknown>;
    if (
        !["inherited", "continuous", "constant", "discrete"].includes(
            String(r.kind),
        ) ||
        Object.keys(r).some(
            (k) =>
                ![
                    "kind",
                    ...(r.kind === "discrete" ? ["period"] : []),
                ].includes(k),
        )
    )
        throw new Error("不支持的采样时间配置。");
    if (r.kind === "discrete") {
        if (
            typeof r.period !== "number" ||
            !Number.isFinite(r.period) ||
            r.period <= 0
        )
            throw new Error("离散采样周期必须为有限正数。");
        return { kind: "discrete", period: r.period };
    }
    return { kind: r.kind as "inherited" | "continuous" | "constant" };
}

export function parseSampleTimes(
    raw: unknown,
    ids?: ReadonlySet<string>,
): Record<string, SampleTime> {
    if (
        !raw ||
        typeof raw !== "object" ||
        Array.isArray(raw) ||
        Object.keys(raw).length > 10000
    )
        throw new Error("采样时间列表无效。");
    return Object.fromEntries(
        Object.entries(raw).map(([id, rate]) => {
            if (!id || id.length > 256 || (ids && !ids.has(id)))
                throw new Error(`采样时间引用了不存在的方块 ${id}`);
            return [id, parseSampleTime(rate)];
        }),
    );
}

export function parseSamplingPlan(raw: unknown): SamplingPlan {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
        throw new Error("采样推导结果无效。");
    const r = raw as Record<string, unknown>;
    if (!Array.isArray(r.clocks) || r.clocks.length > 64)
        throw new Error("采样时钟列表无效。");
    const clocks = r.clocks.map((c: unknown, index: number) => {
        if (!c || typeof c !== "object" || Array.isArray(c))
            throw new Error("采样时钟无效。");
        const clock = c as Record<string, unknown>;
        if (
            clock.id !== index ||
            typeof clock.ticks !== "number" ||
            !Number.isSafeInteger(clock.ticks) ||
            clock.ticks < 1 ||
            clock.ticks > 1e9 ||
            typeof clock.period !== "number" ||
            !Number.isFinite(clock.period) ||
            clock.period <= 0
        )
            throw new Error("采样时钟无效。");
        return { id: index, ticks: clock.ticks, period: clock.period };
    });
    const blocks = parseSampleTimes(r.blocks);
    for (const rate of Object.values(blocks)) {
        if (
            rate.kind === "inherited" ||
            (rate.kind === "discrete" &&
                !clocks.some((c) => c.period === rate.period))
        )
            throw new Error("采样推导结果含有未解析的时钟。");
    }
    return { clocks, blocks };
}

export function sampleTimeText(rate?: SampleTime): string {
    if (!rate) return "尚未检查";
    switch (rate.kind) {
        case "inherited":
            return "继承";
        case "continuous":
            return "连续";
        case "constant":
            return "常量";
        case "discrete":
            return `${Number(rate.period.toPrecision(12))} s（${Number((1 / rate.period).toPrecision(6))} Hz）`;
    }
}
