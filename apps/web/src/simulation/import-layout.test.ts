import { describe, expect, it } from "vitest";
import { importLayout } from "./import-layout";

describe("SLX display layout", () => {
    it("keeps almost aligned imported glyphs in a row while making space for full cards", () => {
        const points = [
            { x: 30, y: 80 },
            { x: 105, y: 75 },
            { x: 210, y: 75 },
            { x: 325, y: 75 },
            { x: 190, y: 160 },
        ];
        const layout = importLayout(points);
        expect(new Set(layout.slice(0, 4).map((point) => point.y)).size).toBe(
            1,
        );
        expect(layout[4]!.y).toBeGreaterThan(layout[3]!.y);
        for (let a = 0; a < layout.length; a++)
            for (let b = a + 1; b < layout.length; b++) {
                expect(
                    Math.abs(layout[a]!.x - layout[b]!.x) >= 200 ||
                        Math.abs(layout[a]!.y - layout[b]!.y) >= 140,
                ).toBe(true);
            }
        expect(points[0]).toEqual({ x: 30, y: 80 });
    });
    it("separates coincident positions, handles negative coordinates and empty systems", () => {
        expect(importLayout([])).toEqual([]);
        const layout = importLayout([
            { x: -500, y: -500 },
            { x: -500, y: -500 },
            { x: -500, y: -500 },
        ]);
        expect(
            new Set(layout.map((point) => `${point.x}:${point.y}`)).size,
        ).toBe(3);
    });
});
