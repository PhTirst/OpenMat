import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { example } from "./examples";
import { groupBlocks, ungroupBlock, validateHierarchy } from "./hierarchy";
import {
    commit,
    copySelection,
    edgeId,
    parseDocument,
    pasteFragment,
    redo,
    removeSelection,
    serializeDocument,
    undo,
} from "./model";
import {
    parseAuthoringKind,
    parseMatrix,
    parseTimeSeriesCsv,
    timeSeriesCsv,
} from "./authoring";
import { AuthoringInspector } from "./AuthoringInspector";
import { ResultWorkspaceExport, resultMatrixCode } from "./result-workspace";

const ids = () => {
    let id = 0;
    return () => `new_${++id}`;
};
const edges = (doc: ReturnType<typeof example>) =>
    doc.model.connections.map(edgeId).sort();

describe("native virtual hierarchy", () => {
    it("groups feedback and fanout, preserves unrelated routing, and expands to the original graph", () => {
        const doc = example("feedback"),
            factory = ids();
        const extra = {
            from: { block: "input", port: "out" },
            to: { block: "extra", port: "in" },
        };
        doc.model.blocks.push({
            id: "extra",
            kind: { type: "scope" },
            position: { x: 920, y: 300 },
        });
        doc.model.connections.push(extra);
        doc.editor.bends[edgeId(extra)] = { x: 100, y: 300 };
        const group = groupBlocks(doc, new Set(["state"]), factory);
        expect(
            group.document.model.blocks.find((b) => b.id === group.id)?.kind,
        ).toEqual({ type: "subsystem", inputs: 1, outputs: 1 });
        // Both the Scope and the feedback Sum use the same external output.
        expect(
            group.document.model.connections.filter(
                (e) => e.from.block === group.id,
            ),
        ).toHaveLength(2);
        expect(group.document.editor.bends[edgeId(extra)]).toEqual({
            x: 100,
            y: 300,
        });
        expect(parseDocument(serializeDocument(group.document))).toEqual(
            group.document,
        );
        const expanded = ungroupBlock(group.document, group.id);
        expect(edges(expanded)).toEqual(edges(doc));
        expect(expanded.model.blocks).toEqual(doc.model.blocks);
        expect(expanded.editor.bends).toEqual(doc.editor.bends);
        const h = commit(
            { past: [], present: doc, future: [] },
            group.document,
        );
        expect(undo(h).present).toEqual(doc);
        expect(redo(undo(h)).present).toEqual(group.document);
    });
    it("copies all nested descendants with independent IDs and removes them as a unit", () => {
        const doc = example("experimentControl"),
            factory = ids();
        const nested = groupBlocks(
            doc,
            new Set(["proportional", "integral", "pi_sum"]),
            factory,
        ).document;
        const fragment = copySelection(nested, new Set(["controller"]));
        const pasted = pasteFragment(nested, fragment, factory);
        expect(pasted.ids).toHaveLength(1);
        expect(pasted.document.model.blocks).toHaveLength(
            nested.model.blocks.length + fragment.model.blocks.length,
        );
        const originalIds = new Set(nested.model.blocks.map((b) => b.id));
        const copied = pasted.document.model.blocks.filter(
            (b) => !originalIds.has(b.id),
        );
        expect(copied.filter((b) => !b.parent)).toHaveLength(1);
        expect(
            copied.every(
                (b) => !b.parent || copied.some((p) => p.id === b.parent),
            ),
        ).toBe(true);
        expect(parseDocument(serializeDocument(pasted.document))).toEqual(
            pasted.document,
        );
        const removed = removeSelection(
            pasted.document,
            new Set(pasted.ids),
            new Set(),
        );
        expect(removed.model).toEqual(nested.model);
    });
    it("renumbers deleted boundary ports and rewires the surviving parent connection", () => {
        const doc = example("experimentControl");
        const second = {
            id: "input_two",
            parent: "controller",
            kind: { type: "inport" as const, port: 2 },
        };
        doc.model.blocks.push(second);
        const container = doc.model.blocks.find((b) => b.id === "controller")!;
        if (container.kind.type !== "subsystem") throw new Error("fixture");
        container.kind.inputs = 2;
        doc.model.connections.push({
            from: { block: "input", port: "out" },
            to: { block: "controller", port: "in2" },
        });
        const next = removeSelection(doc, new Set(["ctrl_in"]), new Set());
        expect(
            next.model.blocks.find((b) => b.id === "input_two")?.kind,
        ).toEqual({ type: "inport", port: 1 });
        expect(
            next.model.connections.filter((e) => e.to.block === "controller"),
        ).toEqual([
            {
                from: { block: "input", port: "out" },
                to: { block: "controller", port: "in1" },
            },
        ]);
        expect(() => parseDocument(serializeDocument(next))).not.toThrow();
    });
    it("rejects cross-parent selection, recursive parents and more than 64 ports", () => {
        const doc = example("experimentControl");
        expect(() =>
            groupBlocks(doc, new Set(["plant", "proportional"]), ids()),
        ).toThrow(/同一系统/);
        doc.model.blocks.find((b) => b.id === "controller")!.parent =
            "controller";
        expect(() => validateHierarchy(doc.model.blocks)).toThrow(/循环/);
        expect(() =>
            validateHierarchy(
                Array.from({ length: 65 }, (_, i) => ({
                    id: `p${i}`,
                    kind: { type: "outport", port: i + 1 },
                })),
            ),
        ).toThrow();
    });
});

describe("standard parameters and embedded data", () => {
    it("converts copied root inputs to inherited subsystem boundaries without carrying data or clock overrides", () => {
        const doc = example("experimentControl");
        doc.model.sampleTimes = { input: { kind: "continuous" } };
        const fragment = copySelection(doc, new Set(["input"]));
        const pasted = pasteFragment(doc, fragment, ids(), 40, "controller");
        const added = pasted.document.model.blocks.find(
            (b) => b.id === pasted.ids[0],
        )!;
        expect(added.kind).toEqual({ type: "inport", port: 2 });
        expect(added.parent).toBe("controller");
        expect(pasted.document.model.sampleTimes?.[added.id]).toBeUndefined();
        expect(() =>
            parseDocument(serializeDocument(pasted.document)),
        ).not.toThrow();
    });
    it("accepts finite literal matrices and preserves imported nonuniform time data on save", () => {
        expect(parseMatrix("[-1 0; 0 -2]")).toEqual([
            [-1, 0],
            [0, -2],
        ]);
        expect(() => parseMatrix("[1; eval('x')]")).toThrow();
        const input = parseTimeSeriesCsv(
            "\ufefftime,reference,measurement\r\n0,1,0\r\n0.137,1,0.2\r\n1,1,0.9",
        );
        expect(parseTimeSeriesCsv(timeSeriesCsv(input))).toEqual(input);
        const doc = example("experimentControl");
        doc.model.blocks[0]!.kind = { type: "inport", port: 1, data: input };
        expect(
            parseDocument(serializeDocument(doc)).model.blocks[0]!.kind,
        ).toEqual(doc.model.blocks[0]!.kind);
        for (const bad of [
            "0,1\n0,2",
            "0,1,2\n1,3",
            "-1,1\n0,2",
            "0,1\n1,NaN",
            "0,1\n1,",
            "time,u\n",
        ])
            expect(() => parseTimeSeriesCsv(bad)).toThrow();
    });
    it("applies a changed State-Space dimension atomically and retains the prior block on invalid input", () => {
        const onChange = vi.fn(),
            onError = vi.fn();
        render(
            <AuthoringInspector
                kind={{
                    type: "standard",
                    operation: {
                        type: "stateSpace",
                        a: [[-1]],
                        b: [[1]],
                        c: [[1]],
                        d: [[0]],
                        initial: [0],
                    },
                }}
                disabled={false}
                onChange={onChange}
                onError={onError}
                onEnter={vi.fn()}
            />,
        );
        fireEvent.change(screen.getByLabelText("A 矩阵"), {
            target: { value: "[-1 0; 0 -2]" },
        });
        fireEvent.click(screen.getByRole("button", { name: "应用参数" }));
        expect(onChange).not.toHaveBeenCalled();
        expect(onError).toHaveBeenCalledOnce();
        for (const [name, value] of [
            ["B 矩阵", "[1; 1]"],
            ["C 矩阵", "[1 1]"],
            ["初始状态", "[0 0]"],
        ])
            fireEvent.change(screen.getByLabelText(name!), {
                target: { value },
            });
        fireEvent.click(screen.getByRole("button", { name: "应用参数" }));
        expect(onChange).toHaveBeenCalledWith({
            type: "standard",
            operation: {
                type: "stateSpace",
                a: [
                    [-1, 0],
                    [0, -2],
                ],
                b: [[1], [1]],
                c: [[1, 1]],
                d: [[0]],
                initial: [0, 0],
            },
        });
    });
    it("rejects improper and overflowing transfer realizations", () => {
        for (const [numerator, denominator] of [
            [[1, 2], [1]],
            [[1], [0, 1]],
            [
                [1e308, 0],
                [1, 1e308],
            ],
        ])
            expect(() =>
                parseAuthoringKind({
                    type: "standard",
                    operation: { type: "transferFcn", numerator, denominator },
                }),
            ).toThrow();
    });
});

describe("m workspace result export", () => {
    const scope = { block: "s", width: 2, offset: 1 };
    const frames = [
        { time: 0, values: [99, 1, 2], sampleHit: false },
        { time: 0.2, values: [98, 3, 4], sampleHit: false },
    ];
    it("exports only the chosen channels with time and rejects invalid identifiers or values", () => {
        expect(resultMatrixCode("simout", frames, scope)).toBe(
            "simout = [\n0 1 2;\n0.2 3 4;\n];\n",
        );
        for (const name of [
            "x;system('bad')",
            "1x",
            "function",
            "a".repeat(64),
        ])
            expect(() => resultMatrixCode(name, frames, scope)).toThrow();
        expect(() =>
            resultMatrixCode(
                "x",
                [{ ...frames[0]!, values: [0, NaN, 1] }],
                scope,
            ),
        ).toThrow();
        expect(() =>
            resultMatrixCode("x", Array(70000).fill(frames[1]), scope),
        ).toThrow(/512 KiB/);
    });
    it("writes only after the user action and reports kernel success or failure", async () => {
        const onExecute = vi
            .fn()
            .mockResolvedValueOnce(false)
            .mockResolvedValueOnce(true);
        render(
            <ResultWorkspaceExport
                frames={frames}
                scope={scope}
                disabled={false}
                onExecute={onExecute}
            />,
        );
        expect(onExecute).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "写入 m 工作区" }));
        await screen.findByText(/未写入工作区/);
        fireEvent.click(screen.getByRole("button", { name: "写入 m 工作区" }));
        await screen.findByText(/已写入 simout：2 行/);
        await waitFor(() => expect(onExecute).toHaveBeenCalledTimes(2));
    });
});
