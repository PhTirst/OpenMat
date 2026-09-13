import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { BlockParameterForm } from "./BlockParameterForm";
import { SlxParameterInspector } from "./SlxParameterInspector";
import { example } from "./examples";
import { applyLiteralParameters } from "./literal-parameters";
import {
    adoptSlxParameters,
    controlBlock,
    controlNodeId,
    parameterDraft,
    setControlPosition,
    validateParameters,
} from "./block-parameters";
import {
    commit,
    copySelection,
    parseDocument,
    pasteFragment,
    removeSelection,
    serializeDocument,
    undo,
} from "./model";
import { groupBlocks } from "./hierarchy";

function parameterExample() {
    const doc = example("feedback");
    const input = doc.model.blocks.find((b) => b.kind.type === "constant")!;
    doc.model.blocks.push({
        id: "test_gain",
        kind: { type: "gain", gain: [2] },
    });
    doc.model.connections.push({
        from: { block: input.id, port: "out" },
        to: { block: "test_gain", port: "in" },
    });
    return doc;
}

describe("common block parameters", () => {
    it("preserves zero-offset sample-time syntax during offline edits", () => {
        const doc = parameterExample();
        const edited = applyLiteralParameters(doc, "test_gain", {
            SampleTime: "[0.05 0]",
        });
        expect(edited.model.sampleTimes?.test_gain).toEqual({
            kind: "discrete",
            period: 0.05,
        });
        expect(() =>
            applyLiteralParameters(doc, "test_gain", {
                SampleTime: "[0.05 0.01]",
            }),
        ).toThrow("非零偏移");
        expect(() =>
            applyLiteralParameters(doc, "test_gain", { SampleTime: "[Ts 0]" }),
        ).toThrow("原生仿真服务");
    });
    it("retains supported SLX expressions and maps internal controls to the execution owner", () => {
        const doc = parameterExample();
        doc.model.blocks.find((b) => b.id === "test_gain")!.id = "slx_2";
        doc.slx = {
            name: "authored.slx",
            package: "AQ==",
            parameters: "K = 2;",
            appliedParameters: "K = 2;",
            runnable: true,
            issues: [],
            document: {
                name: "authored",
                systems: [
                    {
                        parentBlock: null,
                        lines: [],
                        blocks: [
                            {
                                sid: "2",
                                name: "Gain",
                                blockType: "Gain",
                                properties: {
                                    Gain: "2*K",
                                    Multiplication: "Element-wise(K.*u)",
                                    Position: "[10 10 40 40]",
                                },
                                source: { part: "simulink/blockdiagram.xml" },
                            },
                        ],
                    },
                ],
            },
        };
        adoptSlxParameters(doc);
        expect(doc.parameters).toEqual({
            source: "K = 2;",
            bindings: {
                slx_2: { Gain: "2*K", Multiplication: "Element-wise(K.*u)" },
            },
        });
        expect(doc.slx.package).toBe("AQ==");
        expect(doc.schemaVersion).toBe(9);

        const conditional = example("enabledControl");
        const owner = conditional.model.blocks.find(
            (b) => b.kind.type === "subsystem" && b.kind.execution,
        )!;
        owner.id = "slx_1";
        conditional.slx = {
            ...doc.slx,
            document: {
                name: "conditional",
                systems: [
                    {
                        parentBlock: "1",
                        lines: [],
                        blocks: [
                            {
                                sid: "3",
                                name: "Enable",
                                blockType: "EnablePort",
                                properties: { StatesWhenEnabling: "reset" },
                                source: {
                                    part: "simulink/systems/system_1.xml",
                                },
                            },
                        ],
                    },
                ],
            },
        };
        adoptSlxParameters(conditional);
        expect(conditional.parameters?.bindings).toEqual({
            slx_1: { StatesWhenEnabling: "reset" },
        });
    });
    it("keeps expression text through save, copy and undo without conflating document and numerical versions", () => {
        const doc = parameterExample();
        doc.schemaVersion = 9;
        const gain = doc.model.blocks.find((b) => b.kind.type === "gain")!;
        doc.parameters = {
            source: "K = 2;",
            bindings: { [gain.id]: { Gain: "K" } },
        };
        const opened = parseDocument(serializeDocument(doc));
        expect(opened.parameters).toEqual(doc.parameters);
        const fragment = copySelection(opened, new Set([gain.id]));
        const pasted = pasteFragment(opened, fragment, () => "copy_gain");
        expect(pasted.document.parameters?.bindings.copy_gain).toEqual({
            Gain: "K",
        });
        const grouped = groupBlocks(
            pasted.document,
            new Set(["copy_gain"]),
            () => "grouped",
        ).document;
        expect(grouped.schemaVersion).toBe(9);
        expect(grouped.model.schemaVersion).toBe(7);
        const history = commit(
            { past: [], present: opened, future: [] },
            pasted.document,
        );
        expect(undo(history).present).toEqual(opened);
        const removed = removeSelection(
            pasted.document,
            new Set(["copy_gain"]),
            new Set(),
        );
        expect(removed.parameters?.bindings.copy_gain).toBeUndefined();
    });
    it("rejects unknown bindings and refuses to mix conflicting parameter workspaces on paste", () => {
        const doc = parameterExample(),
            gain = doc.model.blocks.find((b) => b.kind.type === "gain")!;
        expect(() =>
            validateParameters(
                { source: "", bindings: { missing: { Gain: "K" } } },
                doc.model,
            ),
        ).toThrow(/不存在/);
        expect(() =>
            validateParameters(
                { source: "", bindings: { [gain.id]: { Unknown: "2" } } },
                doc.model,
            ),
        ).toThrow(/不受支持/);
        doc.schemaVersion = 9;
        doc.parameters = {
            source: "K=2;",
            bindings: { [gain.id]: { Gain: "K" } },
        };
        const fragment = copySelection(doc, new Set([gain.id]));
        fragment.parameters!.source = "K=9;";
        expect(() => pasteFragment(doc, fragment, () => "copy")).toThrow(
            /参数定义不同/,
        );
        const raw = JSON.parse(serializeDocument(doc));
        raw.schemaVersion = 8;
        expect(() => parseDocument(JSON.stringify(raw))).toThrow(/版本 9/);
    });
    it("places conditional policy fields on the control and numbered output, retaining layout through copy/delete/undo", () => {
        const doc = example("enabledControl"),
            parent = doc.model.blocks.find((b) => b.id === "controller")!;
        const output = doc.model.blocks.find(
            (b) => b.parent === parent.id && b.kind.type === "outport",
        )!;
        setControlPosition(doc, parent.id, { x: 150, y: -90 });
        expect(controlBlock(doc, parent.id)?.id).toBe(controlNodeId(parent.id));
        const changed = applyLiteralParameters(
            applyLiteralParameters(doc, parent.id, {
                StatesWhenEnabling: "reset",
            }),
            output.id,
            { InitialOutput: "[2 3]", OutputWhenDisabled: "reset" },
        );
        expect(parameterDraft(changed, parent).StatesWhenEnabling).toBe("held"); // old snapshot stays untouched
        expect(
            parameterDraft(
                changed,
                changed.model.blocks.find((b) => b.id === parent.id)!,
            ).StatesWhenEnabling,
        ).toBe("reset");
        expect(
            changed.model.blocks.find((b) => b.id === parent.id)?.kind,
        ).toMatchObject({
            execution: {
                outputs: [{ initial: [2, 3], whenDisabled: "reset" }],
            },
        });
        const copied = copySelection(changed, new Set([parent.id]));
        let id = 0;
        const pasted = pasteFragment(changed, copied, () => `copy_${id++}`);
        expect(
            pasted.document.editor.controlPositions?.[pasted.ids[0]!],
        ).toEqual({ x: 150, y: -90 });
        const removed = removeSelection(
            changed,
            new Set([controlNodeId(parent.id)]),
            new Set(),
        );
        expect(
            removed.model.blocks.find((b) => b.id === parent.id)?.kind,
        ).not.toHaveProperty("execution");
        expect(
            removed.model.connections.some((e) => e.to.port === "enable"),
        ).toBe(false);
        expect(
            undo(commit({ past: [], present: changed, future: [] }, removed))
                .present,
        ).toEqual(changed);
        expect(parseDocument(serializeDocument(pasted.document))).toEqual(
            pasted.document,
        );
    });
    it("sends expressions unchanged, exposes unsupported multiplication choices, and retains invalid drafts", async () => {
        const doc = parameterExample(),
            gain = doc.model.blocks.find((b) => b.kind.type === "gain")!;
        const apply = vi.fn().mockRejectedValue(new Error("K 尚未定义"));
        render(
            <BlockParameterForm
                doc={doc}
                block={gain}
                disabled={false}
                onApply={apply}
            />,
        );
        expect(
            screen.getByRole("option", { name: "Matrix(K*u)（暂不支持）" }),
        ).toBeDisabled();
        fireEvent.change(screen.getByLabelText("增益"), {
            target: { value: "K" },
        });
        fireEvent.click(screen.getByText("应用参数"));
        await waitFor(() => expect(apply).toHaveBeenCalledWith({ Gain: "K" }));
        expect(await screen.findByRole("alert")).toHaveTextContent(
            "K 尚未定义",
        );
        expect(screen.getByLabelText("增益")).toHaveValue("K");
    });
    it("restores form values when history changes and rejects offline expressions atomically", () => {
        const doc = parameterExample(),
            gain = doc.model.blocks.find((b) => b.kind.type === "gain")!;
        const changed = applyLiteralParameters(doc, gain.id, { Gain: "3" });
        const view = render(
            <BlockParameterForm
                doc={changed}
                block={changed.model.blocks.find((b) => b.id === gain.id)!}
                disabled={false}
                onApply={vi.fn()}
            />,
        );
        expect(screen.getByLabelText("增益")).toHaveValue("[3]");
        view.rerender(
            <BlockParameterForm
                doc={doc}
                block={gain}
                disabled={false}
                onApply={vi.fn()}
            />,
        );
        expect(screen.getByLabelText("增益")).toHaveValue(
            parameterDraft(doc, gain).Gain,
        );
        expect(() =>
            applyLiteralParameters(doc, gain.id, { Gain: "2*pi" }),
        ).toThrow(/原生/);
        expect(
            changed.model.blocks.find((b) => b.id === gain.id)?.kind,
        ).toEqual({ type: "gain", gain: [3] });
    });
    it("groups SLX properties and identifies reference defaults without inventing effective values", () => {
        render(
            <SlxParameterInspector
                block={{
                    sid: "1",
                    name: "gain",
                    blockType: "Gain",
                    properties: {
                        Gain: "K",
                        Multiplication: "Matrix(K*u)",
                        Position: "[0 0 50 40]",
                    },
                    source: { part: "model.xml" },
                }}
            />,
        );
        expect(screen.getByText("主要参数")).toBeInTheDocument();
        expect(screen.getByText("K")).toBeInTheDocument();
        expect(screen.getByText(/当前选项暂不支持/)).toBeInTheDocument();
        expect(screen.getByText(/未显式存储/)).toBeInTheDocument();
        expect(screen.getByText("Position")).toBeInTheDocument();
    });
});
