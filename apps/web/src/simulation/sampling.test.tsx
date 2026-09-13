import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
    copySelection,
    emptyDocument,
    parseDocument,
    pasteFragment,
    removeSelection,
    serializeDocument,
} from "./model";
import {
    parseSamplingPlan,
    parseSampleTime,
    type SamplingPlan,
} from "./sampling";
import { csv, scopeFrames } from "./scope-data";
import { SamplingInspector } from "./SamplingInspector";

const sampling: SamplingPlan = {
    clocks: [
        { id: 0, ticks: 1, period: 0.1 },
        { id: 1, ticks: 2, period: 0.2 },
    ],
    blocks: {
        a: { kind: "discrete", period: 0.1 },
        b: { kind: "discrete", period: 0.2 },
    },
};

describe("multirate authoring", () => {
    it("preserves version five and remaps only copied sample times", () => {
        const doc = emptyDocument();
        doc.schemaVersion = doc.model.schemaVersion = 5;
        doc.sources = {};
        doc.model.blocks = [
            {
                id: "a",
                kind: {
                    type: "rateTransition",
                    initial: [-2],
                    deterministic: true,
                },
            },
            {
                id: "b",
                kind: { type: "discreteIntegrator", initial: [1], gain: 3 },
            },
        ];
        doc.model.sampleTimes = sampling.blocks;
        const restored = parseDocument(serializeDocument(doc));
        const fragment = copySelection(restored, new Set(["b"]));
        expect(Object.keys(fragment.model.sampleTimes!)).toEqual(["b"]);
        const pasted = pasteFragment(restored, fragment, () => "copy").document;
        expect(pasted.model.schemaVersion).toBe(5);
        expect(pasted.model.sampleTimes?.copy).toEqual({
            kind: "discrete",
            period: 0.2,
        });
        const removed = removeSelection(pasted, new Set(["b"]), new Set());
        expect(removed.model.sampleTimes?.b).toBeUndefined();
        expect(
            parseDocument(serializeDocument(removed)).model.blocks,
        ).toHaveLength(2);
        removed.schemaVersion = removed.model.schemaVersion = 4;
        expect(() => parseDocument(serializeDocument(removed))).toThrow();
    });

    it("rejects unbound clocks, offsets and invalid rates", () => {
        expect(parseSamplingPlan(sampling)).toEqual(sampling);
        expect(() => parseSamplingPlan({ ...sampling, clocks: [] })).toThrow();
        for (const value of [
            { kind: "discrete", period: 0 },
            { kind: "continuous", period: 1 },
            { kind: "discrete", period: 0.1, offset: 0.05 },
        ])
            expect(() => parseSampleTime(value)).toThrow();
    });

    it("exports only observed times and leaves cells blank for clocks that did not hit", () => {
        const frames = [
            { time: 0, values: [1, 10], sampleHit: true, sampleHits: [0, 1] },
            { time: 0.05, values: [1, 10], sampleHit: false },
            { time: 0.1, values: [2, 10], sampleHit: true, sampleHits: [0] },
            { time: 0.2, values: [3, 20], sampleHit: true, sampleHits: [0, 1] },
        ];
        const scopes = [
            { block: "a", offset: 0, width: 1, sampleTime: sampling.blocks.a! },
            { block: "b", offset: 1, width: 1, sampleTime: sampling.blocks.b! },
        ];
        expect(
            scopeFrames(frames, scopes[1], sampling).map((f) => f.time),
        ).toEqual([0, 0.2]);
        expect(csv(frames, ["a", "b"], scopes, sampling)).toContain(
            "0.1,2,\r\n",
        );
        expect(csv(frames, ["a", "b"], scopes, sampling)).not.toContain("0.05");
        expect(csv(frames, ["a", "b"])).toContain("0.05,1,10");
    });

    it("edits rates and explains the deterministic slow-to-fast latency", () => {
        const model = emptyDocument().model;
        const block = {
            id: "rt",
            kind: {
                type: "rateTransition" as const,
                initial: [-1],
                deterministic: true,
            },
        };
        model.sampleTimes = { rt: { kind: "discrete", period: 0.1 } };
        const onChange = vi.fn(),
            onKindChange = vi.fn();
        render(
            <SamplingInspector
                model={model}
                block={block}
                resolved={{ kind: "discrete", period: 0.1 }}
                onChange={onChange}
                onKindChange={onKindChange}
                onError={vi.fn()}
            />,
        );
        expect(screen.getByLabelText("实际采样时间")).toHaveTextContent(
            "10 Hz",
        );
        fireEvent.change(screen.getByLabelText("方块采样周期"), {
            target: { value: "0.2" },
        });
        fireEvent.blur(screen.getByLabelText("方块采样周期"));
        expect(onChange).toHaveBeenCalledWith({
            kind: "discrete",
            period: 0.2,
        });
        fireEvent.click(screen.getByLabelText("确定性速率转换"));
        expect(onKindChange).toHaveBeenCalledWith({
            ...block.kind,
            deterministic: false,
        });
        expect(
            screen.getByText(/慢转快启用时延迟一个慢采样周期/),
        ).toBeInTheDocument();
    });
});
