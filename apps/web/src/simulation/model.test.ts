import { describe, expect, it } from "vitest";
import { example } from "./examples";
import {
    commit,
    copySelection,
    edgeId,
    label,
    numericLiteral,
    parseDocument,
    pasteFragment,
    redo,
    removeSelection,
    serializeDocument,
    undo,
} from "./model";
import { numericalSource } from "./use-simulation-run";
import { csv, envelope } from "./scope-data";

describe("simulation authoring document", () => {
    it("round-trips labels, positions, routing and viewport without canvas internals or results", () => {
        const doc = example("feedback");
        doc.editor.bends[edgeId(doc.model.connections[0]!)] = { x: 240, y: 70 };
        doc.editor.viewport = { x: 15, y: 32, zoom: 0.7 };
        const written = serializeDocument(doc);
        expect(parseDocument(written)).toEqual(doc);
        expect(written).not.toMatch(/reactFlow|selected|samples|measured/);
        const moved = structuredClone(doc.model);
        moved.blocks[0]!.position = { x: 123, y: 456 };
        expect(numericalSource(moved)).toBe(numericalSource(doc.model));
        moved.settings.stopTime++;
        expect(numericalSource(moved)).not.toBe(numericalSource(doc.model));
    });
    it("copies only internal edges and remaps every copied identity and bend", () => {
        const original = example("feedback"),
            ids = new Set(["input", "sum"]);
        const edge = original.model.connections[0]!;
        original.editor.bends[edgeId(edge)] = { x: 20, y: 30 };
        const fragment = copySelection(original, ids);
        expect(fragment.model.connections).toHaveLength(1);
        let next = 0;
        const pasted = pasteFragment(
            original,
            fragment,
            () => `copy_${++next}`,
        );
        expect(pasted.document.model.connections.at(-1)).toEqual({
            from: { block: "copy_1", port: "out" },
            to: { block: "copy_2", port: "in0" },
        });
        expect(
            pasted.document.editor.bends[
                edgeId(pasted.document.model.connections.at(-1)!)
            ],
        ).toEqual({ x: 60, y: 70 });
        const removed = removeSelection(
            pasted.document,
            new Set(["copy_1"]),
            new Set(),
        );
        expect(removed.model.connections).toEqual(original.model.connections);
        expect(Object.keys(removed.editor.bends)).toEqual([edgeId(edge)]);
        expect(parseDocument(serializeDocument(removed))).toEqual(removed);
    });
    it("undoes a compound delete in one step and discards redo after a new edit", () => {
        const initial = example("feedback"),
            deleted = removeSelection(initial, new Set(["state"]), new Set());
        const edited = commit(
            { past: [], present: initial, future: [] },
            deleted,
        );
        expect(undo(edited).present).toEqual(initial);
        expect(redo(undo(edited)).present).toEqual(deleted);
        expect(commit(undo(edited), example("counter")).future).toEqual([]);
    });
    it("saves incomplete graphs but rejects invalid references and unsupported versions", () => {
        const doc = example("feedback");
        doc.model.connections = [];
        expect(parseDocument(serializeDocument(doc)).model.connections).toEqual(
            [],
        );
        expect(() =>
            parseDocument(
                serializeDocument({ ...doc, schemaVersion: 2 } as never),
            ),
        ).toThrow(/版本/);
        doc.model.connections = [
            {
                from: { block: "missing", port: "out" },
                to: { block: "state", port: "in" },
            },
        ];
        expect(() => parseDocument(serializeDocument(doc))).toThrow(/不存在/);
        expect(() =>
            parseDocument(
                '{"format":"openmat-simulation","schemaVersion":1,"model":{}}',
            ),
        ).toThrow();
    });
    it("does not confuse imported object property names with block labels", () => {
        const doc = example("blank");
        doc.model.blocks = [{ id: "constructor", kind: { type: "scope" } }];
        expect(
            label(parseDocument(serializeDocument(doc)), doc.model.blocks[0]!),
        ).toBe("Scope");
    });
    it.each(["[1, 2, -3e-2]", "1 2 -3e-2", "[1;2;-3e-2]"])(
        "parses literal vectors: %s",
        (value) => {
            expect(numericLiteral(value)).toEqual([1, 2, -0.03]);
        },
    );
    it.each([
        "eval('1')",
        "sin(1)",
        "[1,]",
        "[1,,2]",
        "[]",
        "Inf",
        "NaN",
        "1e999",
        "0x10",
    ])("rejects expressions and invalid numeric input: %s", (value) => {
        expect(() => numericLiteral(value)).toThrow();
    });
});

describe("Scope results", () => {
    it("retains both positive and negative narrow peaks when reducing for display", () => {
        const frames = Array.from({ length: 10000 }, (_, i) => ({
            time: i / 1000,
            sampleHit: false,
            values: [i === 2222 ? 12 : i === 2223 ? -15 : 0],
        }));
        const reduced = envelope(frames, 0, 100);
        expect(reduced.length).toBeLessThan(205);
        expect(reduced.some((point) => point.value === 12)).toBe(true);
        expect(reduced.some((point) => point.value === -15)).toBe(true);
        expect(
            reduced.every(
                (point, i) => !i || point.time > reduced[i - 1]!.time,
            ),
        ).toBe(true);
        const exported = csv(frames, ['=formula,"name"']);
        expect(exported.split("\r\n")).toHaveLength(10002);
        expect(exported.split("\r\n")[0]).toBe(
            'time,"Scope:=formula,""name"""',
        );
    });
});
