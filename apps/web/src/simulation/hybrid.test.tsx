import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { HybridInspector } from "./HybridInspector";
import { BlockIcon } from "./BlockNode";
import { ScopePanel } from "./ScopePanel";
import {
    CONTROL_PRESETS,
    parseEventPlan,
    parseEventRecords,
    parseHybridKind,
} from "./hybrid";
import {
    copySelection,
    emptyDocument,
    kindIcon,
    parseDocument,
    pasteFragment,
    ports,
    serializeDocument,
    type BlockKind,
} from "./model";
import { example } from "./examples";

afterEach(() => vi.unstubAllGlobals());
describe("hybrid control authoring", () => {
    it("preserves every preset, icon, dynamic port and reset clock through copy/save/reopen", () => {
        const doc = emptyDocument();
        doc.schemaVersion = doc.model.schemaVersion = 6;
        doc.model.blocks = CONTROL_PRESETS.map((p) => ({
            id: p.id,
            kind: p.kind,
        }));
        doc.model.sampleTimes = {
            resetDiscrete: { kind: "discrete", period: 0.1 },
        };
        const fragment = copySelection(
            doc,
            new Set(doc.model.blocks.map((b) => b.id)),
        );
        let id = 0;
        const restored = parseDocument(
            serializeDocument(
                pasteFragment(doc, fragment, () => `copy${id++}`).document,
            ),
        );
        expect(restored.model.blocks).toHaveLength(16);
        expect(Object.values(restored.model.sampleTimes!)).toHaveLength(2);
        for (const preset of CONTROL_PRESETS) {
            expect(parseHybridKind(preset.kind)).toEqual(preset.kind);
            const view = render(<BlockIcon type={kindIcon(preset.kind)} />);
            expect(view.container.querySelector("svg path")).not.toBeNull();
            view.unmount();
        }
        expect(
            ports(doc.model.blocks.find((b) => b.id === "switch")!).inputs,
        ).toEqual(["in0", "in1", "in2"]);
        expect(
            ports(doc.model.blocks.find((b) => b.id === "resetDiscrete")!)
                .inputs,
        ).toEqual(["in", "reset"]);
        doc.model.schemaVersion = 5;
        expect(() => parseDocument(serializeDocument(doc))).toThrow();
    });
    it("rejects incomplete port declarations and non-string operation values on import", () => {
        for (const operation of [
            { type: "logical", operator: "and" },
            { type: "minMax", minimum: true },
            { type: "logical", operator: ["not"], inputs: 1 },
            { type: "relational", operator: ["equal"] },
            { type: "switch", criterion: ["nonzero"], threshold: 0 },
        ]) {
            const doc = emptyDocument();
            doc.schemaVersion = doc.model.schemaVersion = 6;
            const raw = JSON.parse(serializeDocument(doc));
            raw.model.blocks = [
                {
                    id: "invalid",
                    kind: { type: "control", operation, zeroCrossing: false },
                },
            ];
            expect(() => parseDocument(JSON.stringify(raw))).toThrow();
        }
        expect(() =>
            parseHybridKind({
                type: "resetIntegrator",
                initial: [0],
                gain: 1,
                discrete: false,
                reset: ["rising"],
            }),
        ).toThrow();
    });
    it("commits control parameters, rejects reversed limits, and switches NOT to one input", () => {
        const errors = vi.fn(),
            changes = vi.fn();
        function Editor({ initial }: { initial: BlockKind }) {
            const [kind, setKind] = useState(initial);
            return (
                <HybridInspector
                    kind={kind}
                    disabled={false}
                    onChange={(k) => {
                        setKind(k);
                        changes(k);
                    }}
                    onError={errors}
                />
            );
        }
        const view = render(<Editor initial={CONTROL_PRESETS[0]!.kind} />);
        fireEvent.change(screen.getByLabelText("上限"), {
            target: { value: "0.75" },
        });
        fireEvent.blur(screen.getByLabelText("上限"));
        expect(changes).toHaveBeenLastCalledWith(
            expect.objectContaining({
                operation: { type: "saturation", lower: [-1], upper: [0.75] },
            }),
        );
        fireEvent.change(screen.getByLabelText("下限"), {
            target: { value: "2" },
        });
        fireEvent.blur(screen.getByLabelText("下限"));
        expect(errors).toHaveBeenCalled();
        expect(screen.getByLabelText("下限")).toHaveValue("-1");
        view.unmount();
        render(<Editor initial={CONTROL_PRESETS[3]!.kind} />);
        fireEvent.change(screen.getByLabelText("逻辑运算"), {
            target: { value: "not" },
        });
        expect(changes).toHaveBeenLastCalledWith(
            expect.objectContaining({
                operation: { type: "logical", operator: "not", inputs: 1 },
            }),
        );
        expect(screen.queryByLabelText("输入端口数量")).not.toBeInTheDocument();
    });
    it("loads self-contained examples and validates event records and plans", () => {
        for (const name of [
            "saturatedPi",
            "switchedControl",
            "periodicReset",
        ] as const) {
            const doc = parseDocument(serializeDocument(example(name)));
            expect(doc.model.schemaVersion).toBe(6);
            for (const block of doc.model.blocks)
                if (block.kind.type === "mFunction")
                    expect(doc.sources?.[block.kind.source]).toContain(
                        "function",
                    );
        }
        const event = {
            block: "state",
            surface: "reset",
            direction: 1,
            kind: "reset",
        };
        expect(parseEventRecords([event])).toEqual([event]);
        expect(() => parseEventRecords([{ ...event, direction: 0 }])).toThrow();
        expect(() => parseEventRecords(Array(513).fill(event))).toThrow();
        expect(() =>
            parseEventPlan({ signals: { a: "complex" }, events: [] }),
        ).toThrow();
    });
    it("shows model events even between a Scope's own sample hits", () => {
        vi.stubGlobal(
            "ResizeObserver",
            class {
                observe() {}
                disconnect() {}
            },
        );
        render(
            <ScopePanel
                frames={[
                    { time: 0, values: [0], sampleHit: true, sampleHits: [0] },
                    {
                        time: 0.137,
                        values: [0],
                        sampleHit: false,
                        events: [
                            {
                                block: "state",
                                surface: "reset",
                                kind: "reset",
                                direction: 1,
                            },
                        ],
                    },
                    {
                        time: 0.2,
                        values: [1],
                        sampleHit: true,
                        sampleHits: [0],
                    },
                ]}
                scopes={[
                    {
                        block: "scope",
                        offset: 0,
                        width: 1,
                        sampleTime: { kind: "discrete", period: 0.2 },
                    },
                ]}
                sampling={{
                    clocks: [{ id: 0, period: 0.2, ticks: 20 }],
                    blocks: {},
                }}
                version={1}
                names={{ state: "积分状态" }}
                dark={false}
            />,
        );
        expect(screen.getByLabelText("Scope 记录点数")).toHaveTextContent(
            "2 个记录点",
        );
        expect(screen.getByLabelText("模型事件记录")).toHaveTextContent(
            "0.137 s · 积分状态 · 复位 ↑",
        );
        fireEvent.click(screen.getByLabelText("事件标记"));
        expect(screen.getByLabelText("事件标记")).not.toBeChecked();
    });
});
