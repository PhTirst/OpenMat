import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ConditionalInspector } from "./ConditionalInspector";
import {
    defaultConditional,
    parseConditional,
    parseExecutionEvents,
} from "./conditional";
import { example, initializeComponentExample } from "./examples";
import { FunctionCodePanel } from "./FunctionCodePanel";
import {
    copySelection,
    parseDocument,
    pasteFragment,
    ports,
    removeSelection,
    serializeDocument,
} from "./model";
import { ungroupBlock } from "./hierarchy";
import { scopeFrames, csv } from "./scope-data";
import { componentFile, parseComponentFile } from "./components";
import { copyComponentSources } from "./component-library";
import type { SimulationFrame, ScopeInfo } from "./client";

describe("conditional authoring", () => {
    it("round trips both examples including embedded callbacks and control ports", () => {
        for (const name of ["enabledControl", "triggeredCounter"] as const) {
            const doc = example(name);
            expect(parseDocument(serializeDocument(doc))).toEqual(doc);
            const group = doc.model.blocks.find(
                (b) => b.kind.type === "subsystem",
            )!;
            const inputs = ports(group).inputs;
            expect(inputs).toContain(
                name === "enabledControl" ? "enable" : "trigger",
            );
            doc.model.schemaVersion = 7;
            expect(() => parseDocument(JSON.stringify(doc))).toThrow();
        }
    });
    it("copies a conditional domain with independent child IDs and embedded component definitions", () => {
        const doc = example("triggeredCounter");
        let n = 0;
        const result = pasteFragment(
            doc,
            copySelection(doc, new Set(["counter"])),
            () => `copy_${++n}`,
        );
        const copy = result.document.model.blocks.find(
            (b) => b.id === result.ids[0],
        )!;
        expect(copy.kind).toEqual(
            doc.model.blocks.find((b) => b.id === "counter")!.kind,
        );
        expect(
            result.document.model.blocks.filter((b) => b.parent === copy.id),
        ).toHaveLength(3);
        expect(parseDocument(serializeDocument(result.document))).toEqual(
            result.document,
        );
        expect(() => ungroupBlock(doc, "counter")).toThrow(/条件/);
        const definition = doc.model.components![0]!;
        expect(parseComponentFile(componentFile(definition))).toEqual(
            definition,
        );
        expect(
            copyComponentSources(definition, doc.sources!, "local").definition
                .sampleTime,
        ).toBe(-1);
        const old = JSON.parse(componentFile(definition));
        old.schemaVersion = 1;
        expect(() => parseComponentFile(JSON.stringify(old))).toThrow();
    });
    it("initializes editable bundled m callbacks without losing the trigger source", () => {
        const doc = example("triggeredCounter");
        const copied = initializeComponentExample(doc, "test");
        expect(Object.keys(copied)).toHaveLength(3);
        const definition = doc.model.components![0]!;
        expect(doc.sources![definition.outputsFunction.source]).toContain(
            "q(1) + 1",
        );
        expect(doc.sources!["trigger-wave.m"]).toContain("sin(");
        expect(parseDocument(serializeDocument(doc))).toEqual(doc);
        const change = vi.fn();
        const view = render(
            <FunctionCodePanel
                path="source.m"
                theme="modern-light"
                onRun={vi.fn()}
                onSave={vi.fn()}
                embedded="function y=f()\ny=1;\nend"
                onEmbeddedChange={change}
            />,
        );
        fireEvent.change(screen.getByLabelText("内嵌 m 回调"), {
            target: { value: "function y=f()\ny=2;\nend" },
        });
        expect(change).toHaveBeenCalledOnce();
        view.rerender(
            <FunctionCodePanel
                path="source.m"
                theme="modern-light"
                onRun={vi.fn()}
                onSave={vi.fn()}
                embedded="generated"
            />,
        );
        expect(screen.getByLabelText("内嵌 m 回调")).toHaveAttribute(
            "readonly",
        );
    });
    it("retains the control wire and corresponding output policy when deleting a numbered output", () => {
        const doc = example("enabledControl");
        const group = doc.model.blocks.find((b) => b.id === "controller")!;
        if (group.kind.type !== "subsystem" || !group.kind.execution)
            throw Error("fixture");
        group.kind.outputs = 2;
        group.kind.execution.outputs.push({
            initial: [42],
            whenDisabled: "held",
        });
        doc.model.blocks.push({
            id: "second",
            parent: "controller",
            kind: { type: "outport", port: 2 },
        });
        doc.model.connections.push({
            from: { block: "limit", port: "out" },
            to: { block: "second", port: "in" },
        });
        const after = removeSelection(doc, new Set(["y"]), new Set());
        const next = after.model.blocks.find((b) => b.id === "controller")!;
        expect(next.kind).toMatchObject({
            outputs: 1,
            execution: { outputs: [{ initial: [42], whenDisabled: "held" }] },
        });
        expect(
            after.model.connections.some(
                (e) => e.to.block === "controller" && e.to.port === "enable",
            ),
        ).toBe(true);
    });
    it("keeps the parent mode editor while directing control and output settings to their owners", () => {
        const kind = {
            type: "subsystem" as const,
            inputs: 1,
            outputs: 1,
            execution: defaultConditional("enabled", 1),
        };
        kind.execution.outputs[0] = { initial: [2, 3], whenDisabled: "reset" };
        const change = vi.fn(),
            enter = vi.fn();
        const view = render(
            <ConditionalInspector
                kind={kind}
                disabled={false}
                onChange={change}
                onError={vi.fn()}
                onEnter={enter}
            />,
        );
        expect(screen.queryByLabelText("输出 1 初值")).not.toBeInTheDocument();
        fireEvent.click(screen.getByText("进入子系统"));
        expect(enter).toHaveBeenCalledOnce();
        fireEvent.change(screen.getByLabelText("子系统执行方式"), {
            target: { value: "triggered" },
        });
        fireEvent.click(screen.getByText("应用执行设置"));
        const next = change.mock.calls[0]![0];
        expect(next.execution).toMatchObject({
            type: "triggered",
            edge: "rising",
            outputs: [{ initial: [2, 3], whenDisabled: "held" }],
        });
        view.rerender(
            <ConditionalInspector
                kind={next}
                disabled={false}
                onChange={change}
                onError={vi.fn()}
                onEnter={enter}
            />,
        );
        view.rerender(
            <ConditionalInspector
                kind={kind}
                disabled={false}
                onChange={change}
                onError={vi.fn()}
                onEnter={enter}
            />,
        );
        expect(screen.getByLabelText("子系统执行方式")).toHaveValue("enabled");
    });
    it("rejects ambiguous output policies and unknown execution events", () => {
        expect(() =>
            parseConditional(
                {
                    ...defaultConditional("triggered", 1),
                    outputs: [{ initial: [0], whenDisabled: "reset" }],
                },
                1,
            ),
        ).toThrow();
        expect(() =>
            parseConditional(
                {
                    ...defaultConditional("enabled", 1),
                    outputs: [{ initial: [], whenDisabled: "held" }],
                },
                1,
            ),
        ).toThrow();
        expect(() =>
            parseExecutionEvents([{ block: "counter", kind: "__proto__" }]),
        ).toThrow();
    });
    it("plots and exports inner Scopes only at accepted invocations", () => {
        const frames: SimulationFrame[] = [
            { time: 0, values: [0], sampleHit: true, sampleHits: [0] },
            {
                time: 0.1,
                values: [1],
                sampleHit: true,
                sampleHits: [0],
                executionHits: ["counter"],
            },
            { time: 0.2, values: [1], sampleHit: true, sampleHits: [0] },
        ];
        const scope: ScopeInfo = {
            block: "inner",
            offset: 0,
            width: 1,
            execution: "counter",
        };
        expect(scopeFrames(frames, scope)).toEqual([frames[1]]);
        expect(csv(frames, ["inner"], [scope])).toBe(
            'time,"Scope:inner"\r\n0.1,1\r\n',
        );
    });
});
