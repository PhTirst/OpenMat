import common from "../../../../simulation/blocks/common.json";
import type { Block, Model, ModelDocument, Point } from "./model";
import { matrixText } from "./authoring";

export interface ParameterDescriptor {
    name: string;
    label: string;
    group: "main" | "signal" | "execution";
    editor: "expression" | "text" | "choice" | "readonly";
    path?: string;
    shape?: string;
    defaultValue: string;
    fixed?: string;
    conditional?: boolean;
    extension?: boolean;
    help?: string;
    options?: { value: string; supported: boolean }[];
}
export interface BlockDescriptor {
    id: string;
    slx: string;
    label: string;
    parameters: ParameterDescriptor[];
}
export const PARAMETER_CATALOG = common as {
    schemaVersion: number;
    baseline: string;
    groups: Record<string, string>;
    blocks: BlockDescriptor[];
};
export interface AuthoringParameters {
    source: string;
    bindings: Record<string, Record<string, string>>;
}
export interface ParameterResolution {
    model: Model;
    values: Record<string, Record<string, string>>;
}

export function blockDescriptor(block: Block): BlockDescriptor | undefined {
    const k = block.kind;
    const key =
        k.type === "standard"
            ? k.operation.type
            : k.type === "subsystem"
              ? k.execution?.type
              : k.type === "resetIntegrator"
                ? k.discrete
                    ? "discreteIntegrator"
                    : "integrator"
                : k.type;
    return PARAMETER_CATALOG.blocks.find((b) => b.id === key);
}
export function conditionalParent(model: Model, block: Block) {
    const parent = model.blocks.find((b) => b.id === block.parent);
    return parent?.kind.type === "subsystem" && parent.kind.execution
        ? parent
        : undefined;
}
export function parameterFields(model: Model, block: Block) {
    return (
        blockDescriptor(block)?.parameters.filter(
            (field) =>
                !field.conditional || Boolean(conditionalParent(model, block)),
        ) ?? []
    );
}
function display(raw: unknown): string {
    if (Array.isArray(raw))
        return matrixText(
            Array.isArray(raw[0]) ? (raw as number[][]) : [raw as number[]],
        );
    return String(raw ?? "");
}
export function parameterValue(
    model: Model,
    block: Block,
    field: ParameterDescriptor,
): string {
    const k = block.kind;
    if (field.fixed !== undefined) return field.fixed;
    switch (field.path) {
        case "sampleTime": {
            const s = model.sampleTimes?.[block.id];
            if (s)
                return s.kind === "discrete"
                    ? String(s.period)
                    : s.kind === "inherited"
                      ? "-1"
                      : s.kind === "continuous"
                        ? "0"
                        : "inf";
            if (k.type === "constant") return "inf";
            if (k.type === "step") return "0";
            if (
                k.type === "unitDelay" ||
                k.type === "discreteIntegrator" ||
                k.type === "zeroOrderHold" ||
                (k.type === "resetIntegrator" && k.discrete)
            )
                return String(
                    model.settings.sampleTime ?? model.settings.maxStep,
                );
            return "-1";
        }
        case "signs":
            return k.type === "sum"
                ? k.signs.map((s) => (s === 1 ? "+" : "-")).join("")
                : "";
        case "operations":
            return k.type === "standard" && k.operation.type === "product"
                ? k.operation.operations
                : "";
        case "externalReset":
            return k.type === "resetIntegrator" ? k.reset : "none";
        case "controlPeriod":
            return k.type === "subsystem" ? String(k.execution?.period) : "";
        case "statesWhenEnabling":
            return k.type === "subsystem" && k.execution?.type === "enabled"
                ? k.execution.statesWhenEnabling
                : "held";
        case "triggerType":
            return k.type === "subsystem" && k.execution?.type === "triggered"
                ? k.execution.edge
                : "rising";
        case "initialOutput":
        case "outputWhenDisabled": {
            const p = conditionalParent(model, block);
            const output =
                p?.kind.type === "subsystem" && k.type === "outport"
                    ? p.kind.execution?.outputs[k.port - 1]
                    : undefined;
            return field.path === "initialOutput"
                ? display(output?.initial)
                : (output?.whenDisabled ?? "held");
        }
        default: {
            let value: unknown = block;
            for (const key of field.path?.split(".") ?? [])
                value =
                    value && typeof value === "object"
                        ? (value as Record<string, unknown>)[key]
                        : undefined;
            return display(value);
        }
    }
}
export function parameterDraft(
    doc: ModelDocument,
    block: Block,
): Record<string, string> {
    return Object.fromEntries(
        parameterFields(doc.model, block).map((field) => [
            field.name,
            doc.parameters?.bindings[block.id]?.[field.name] ??
                parameterValue(doc.model, block, field),
        ]),
    );
}

export function validateParameters(
    raw: unknown,
    model: Model,
): AuthoringParameters {
    const object = (value: unknown): Record<string, unknown> => {
        if (!value || typeof value !== "object" || Array.isArray(value))
            throw new Error("模型参数数据无效。");
        return value as Record<string, unknown>;
    };
    const data = object(raw);
    const size = (value: string) => new TextEncoder().encode(value).length;
    if (
        Object.keys(data).some((k) => !["source", "bindings"].includes(k)) ||
        typeof data.source !== "string" ||
        size(data.source) > 65536
    )
        throw new Error("模型参数源文本超过 64 KiB 或格式无效。");
    let count = 0,
        bytes = 0;
    const bindings = Object.fromEntries(
        Object.entries(object(data.bindings)).map(([id, fields]) => {
            const block = model.blocks.find((b) => b.id === id);
            if (!block) throw new Error(`参数引用了不存在的方块 ${id}。`);
            const definitions = parameterFields(model, block);
            bytes += size(id);
            return [
                id,
                Object.fromEntries(
                    Object.entries(object(fields)).map(([name, value]) => {
                        const field = definitions.find((f) => f.name === name);
                        if (
                            !field ||
                            field.editor === "readonly" ||
                            typeof value !== "string"
                        )
                            throw new Error(
                                `方块 ${id} 的参数 ${name} 不受支持。`,
                            );
                        bytes += size(name) + size(value);
                        if (++count > 1024 || bytes > 65536)
                            throw new Error("参数绑定超过 1024 项或 64 KiB。");
                        return [name, value];
                    }),
                ),
            ];
        }),
    );
    return { source: data.source, bindings };
}

export const controlNodeId = (parent: string) => `@control:${parent}`;
export const controlOwnerId = (id: string) =>
    id.startsWith("@control:") ? id.slice(9) : undefined;
export function controlBlock(
    doc: ModelDocument,
    parent: string | undefined,
): Block | undefined {
    const owner = doc.model.blocks.find((b) => b.id === parent);
    if (owner?.kind.type !== "subsystem" || !owner.kind.execution)
        return undefined;
    return {
        ...owner,
        id: controlNodeId(owner.id),
        parent: owner.id,
        position: doc.editor.controlPositions?.[owner.id] ?? {
            x: 160,
            y: -110,
        },
    };
}
export function setControlPosition(
    doc: ModelDocument,
    owner: string,
    position: Point,
) {
    doc.schemaVersion = 9;
    doc.editor.controlPositions ??= {};
    doc.editor.controlPositions[owner] = position;
}

/** Remove bindings whose authoring target disappeared after a structural edit. */
export function pruneParameterBindings(doc: ModelDocument) {
    if (doc.parameters) {
        doc.parameters.bindings = Object.fromEntries(
            Object.entries(doc.parameters.bindings).flatMap(([id, fields]) => {
                const block = doc.model.blocks.find((b) => b.id === id);
                if (!block) return [];
                const definitions = parameterFields(doc.model, block);
                const kept = Object.fromEntries(
                    Object.entries(fields).filter(([name]) =>
                        definitions.some((f) => f.name === name),
                    ),
                );
                return Object.keys(kept).length ? [[id, kept]] : [];
            }),
        );
    }
    if (doc.editor.controlPositions)
        doc.editor.controlPositions = Object.fromEntries(
            Object.entries(doc.editor.controlPositions).filter(([id]) => {
                const b = doc.model.blocks.find((b) => b.id === id);
                return (
                    b?.kind.type === "subsystem" && Boolean(b.kind.execution)
                );
            }),
        );
}

/** Carry supported source expressions into the editable snapshot without rewriting SLX. */
export function adoptSlxParameters(doc: ModelDocument) {
    if (!doc.slx?.runnable) return;
    const bindings: AuthoringParameters["bindings"] = {};
    for (const system of doc.slx.document.systems) {
        for (const source of system.blocks) {
            const isControl =
                source.blockType === "EnablePort" ||
                source.blockType === "TriggerPort";
            const sid = isControl ? system.parentBlock : source.sid;
            if (!sid) continue;
            const id = `slx_${sid.replaceAll(":", "_")}`;
            const block = doc.model.blocks.find((b) => b.id === id);
            if (!block || blockDescriptor(block)?.slx !== source.blockType)
                continue;
            const fields = parameterFields(doc.model, block).filter(
                (f) =>
                    f.editor !== "readonly" &&
                    !f.extension &&
                    Object.hasOwn(source.properties, f.name),
            );
            const values = Object.fromEntries(
                fields.map((f) => [f.name, source.properties[f.name]!]),
            );
            if (Object.keys(values).length)
                bindings[id] = { ...bindings[id], ...values };
        }
    }
    if (Object.keys(bindings).length) {
        doc.schemaVersion = 9;
        doc.parameters = validateParameters(
            { source: doc.slx.parameters, bindings },
            doc.model,
        );
    }
}
