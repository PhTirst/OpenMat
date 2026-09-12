import type { SimulationFrame } from "./client";
/** Keep both extrema, in time order, in every horizontal pixel bucket. */
export function envelope(
    frames: readonly SimulationFrame[],
    channel: number,
    pixels: number,
): { time: number; value: number }[] {
    if (
        !frames.length ||
        !Number.isInteger(channel) ||
        channel < 0 ||
        channel >= frames[0]!.values.length ||
        !Number.isFinite(pixels)
    )
        return [];
    const first = frames[0]!.time,
        span = Math.max(Number.EPSILON, frames.at(-1)!.time - first);
    const result: { time: number; value: number }[] = [];
    let bucket = -1,
        min = -1,
        max = -1;
    const emit = () => {
        if (min < 0) return;
        for (const i of min === max
            ? [min]
            : min < max
              ? [min, max]
              : [max, min])
            result.push({
                time: frames[i]!.time,
                value: frames[i]!.values[channel]!,
            });
    };
    frames.forEach((frame, i) => {
        const next = Math.floor(
            ((frame.time - first) / span) * Math.max(1, pixels),
        );
        if (next !== bucket) {
            emit();
            bucket = next;
            min = max = i;
        } else {
            if (frame.values[channel]! < frames[min]!.values[channel]!) min = i;
            if (frame.values[channel]! > frames[max]!.values[channel]!) max = i;
        }
    });
    emit();
    if (result[0]?.time !== first)
        result.unshift({ time: first, value: frames[0]!.values[channel]! });
    const last = frames.at(-1)!;
    if (result.at(-1)?.time !== last.time)
        result.push({ time: last.time, value: last.values[channel]! });
    return result;
}
export function csv(
    frames: readonly SimulationFrame[],
    headers: string[],
): string {
    const quote = (text: string) => `"${text.replaceAll('"', '""')}"`;
    return `${["time", ...headers.map((header) => quote(`Scope:${header}`))].join(",")}\r\n${frames.map((frame) => [frame.time, ...frame.values].join(",")).join("\r\n")}\r\n`;
}
