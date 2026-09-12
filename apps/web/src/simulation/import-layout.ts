import type { Point } from "./model";

/** Imported glyphs can be much smaller than our cards. Preserve aligned rows/columns,
 * but reserve a full card cell for each block; original properties stay in SLX inspection. */
export function importLayout(points: Point[]): Point[] {
    function lanes(axis: "x" | "y") {
        const anchors: number[] = [];
        for (const value of points
            .map((point) => point[axis])
            .sort((a, b) => a - b)) {
            if (!anchors.length || value - anchors.at(-1)! > 30)
                anchors.push(value);
        }
        return (value: number) => {
            let lo = 0,
                hi = anchors.length;
            while (lo < hi) {
                const mid = (lo + hi) >>> 1;
                if (anchors[mid]! <= value) lo = mid + 1;
                else hi = mid;
            }
            return Math.max(0, lo - 1);
        };
    }
    const column = lanes("x"),
        row = lanes("y"),
        occupied = new Set<string>();
    // Source ordering must not change placement when two blocks occupy one cell.
    const order = points
        .map((point, index) => ({ point, index }))
        .sort(
            (a, b) =>
                a.point.y - b.point.y ||
                a.point.x - b.point.x ||
                a.index - b.index,
        );
    const result: Point[] = new Array(points.length);
    for (const { point, index } of order) {
        const x = column(point.x);
        let y = row(point.y);
        while (occupied.has(`${x}:${y}`)) y++;
        occupied.add(`${x}:${y}`);
        result[index] = { x: 80 + x * 240, y: 80 + y * 170 };
    }
    return result;
}
