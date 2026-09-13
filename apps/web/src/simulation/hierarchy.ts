import { initialOutput } from "./conditional";
import {
    edgeId,
    type Block,
    type ModelDocument,
    type Connection,
    type Port,
} from "./model";

export function descendants(
    blocks: Block[],
    selected: ReadonlySet<string>,
): Set<string> {
    const result = new Set(selected),
        children = new Map<string, string[]>();
    for (const b of blocks)
        if (b.parent)
            children.set(b.parent, [...(children.get(b.parent) ?? []), b.id]);
    const queue = [...selected];
    for (let i = 0; i < queue.length; i++)
        for (const id of children.get(queue[i]!) ?? [])
            if (!result.has(id)) {
                result.add(id);
                queue.push(id);
            }
    return result;
}
export function validateHierarchy(blocks: Block[]) {
    const byId = new Map(blocks.map((b) => [b.id, b]));
    const groups = new Map<string, Block[]>();
    for (const b of blocks) {
        let parent = b.parent;
        const seen = new Set([b.id]);
        while (parent) {
            const p = byId.get(parent);
            if (
                !p ||
                p.kind.type !== "subsystem" ||
                seen.has(parent) ||
                seen.size > 32
            )
                throw new Error("父系统无效、存在循环或层级超过 32 层。");
            seen.add(parent);
            parent = p.parent;
        }
        if (b.kind.type === "inport" || b.kind.type === "outport") {
            if (b.parent && b.kind.type === "inport" && b.kind.data)
                throw new Error(
                    "子系统内部 Inport 从父系统接收信号，不能绑定独立数据。",
                );
            const key = JSON.stringify([b.parent, b.kind.type]);
            groups.set(key, [...(groups.get(key) ?? []), b]);
        }
    }
    for (const parent of [
        undefined,
        ...blocks.filter((b) => b.kind.type === "subsystem").map((b) => b.id),
    ])
        for (const type of ["inport", "outport"] as const) {
            const ports = groups.get(JSON.stringify([parent, type])) ?? [];
            const numbers = ports
                .map((b) => (b.kind as { port: number }).port)
                .sort((a, b) => a - b);
            const p = parent ? byId.get(parent) : undefined;
            const expected =
                p?.kind.type === "subsystem"
                    ? type === "inport"
                        ? p.kind.inputs
                        : p.kind.outputs
                    : ports.length;
            if (
                numbers.length > 64 ||
                numbers.length !== expected ||
                numbers.some((n, i) => n !== i + 1)
            )
                throw new Error(
                    "Inport / Outport 编号必须连续，并与子系统端口数量一致。",
                );
        }
}
export function renumberBoundaries(doc: ModelDocument) {
    const mappings = new Map<string, Map<string, string>>();
    for (const parent of [
        undefined,
        ...doc.model.blocks
            .filter((b) => b.kind.type === "subsystem")
            .map((b) => b.id),
    ])
        for (const type of ["inport", "outport"] as const) {
            const ports = doc.model.blocks
                .filter((b) => b.parent === parent && b.kind.type === type)
                .sort(
                    (a, b) =>
                        (a.kind as { port: number }).port -
                        (b.kind as { port: number }).port,
                );
            const map = mappings.get(parent ?? "") ?? new Map<string, string>();
            mappings.set(parent ?? "", map);
            const owner = doc.model.blocks.find((b) => b.id === parent);
            const execution =
                owner?.kind.type === "subsystem"
                    ? owner.kind.execution
                    : undefined;
            const policies =
                type === "outport" && execution
                    ? ports.map((b) =>
                          b.kind.type === "outport"
                              ? (execution.outputs[b.kind.port - 1] ??
                                initialOutput())
                              : initialOutput(),
                      )
                    : undefined;
            ports.forEach((b, i) => {
                if (b.kind.type !== "inport" && b.kind.type !== "outport")
                    return;
                map.set(
                    `${type === "inport" ? "in" : "out"}${b.kind.port}`,
                    `${type === "inport" ? "in" : "out"}${i + 1}`,
                );
                b.kind.port = i + 1;
            });
            const p = doc.model.blocks.find((b) => b.id === parent);
            if (p?.kind.type === "subsystem") {
                p.kind[type === "inport" ? "inputs" : "outputs"] = ports.length;
                if (policies && p.kind.execution)
                    p.kind.execution.outputs = policies;
            }
        }
    doc.model.connections = doc.model.connections.filter((edge) => {
        for (const p of [edge.from, edge.to]) {
            const map = mappings.get(p.block);
            if (map && p.port !== "enable" && p.port !== "trigger") {
                const next = map.get(p.port);
                if (!next) return false;
                p.port = next;
            }
        }
        return true;
    });
}
function upgrade(doc: ModelDocument) {
    doc.schemaVersion = doc.model.schemaVersion = Math.max(
        doc.schemaVersion,
        doc.model.schemaVersion,
        7,
    ) as ModelDocument["schemaVersion"];
}
const key = (p: Port) => JSON.stringify(p);
const wire = (from: Port, to: Port): Connection => ({
    from: { ...from },
    to: { ...to },
});

export function groupBlocks(
    doc: ModelDocument,
    selected: ReadonlySet<string>,
    idFactory: () => string,
): { document: ModelDocument; id: string } {
    const peers = doc.model.blocks.filter((b) => selected.has(b.id));
    if (
        !peers.length ||
        peers.some(
            (b) =>
                b.parent !== peers[0]!.parent ||
                b.kind.type === "inport" ||
                b.kind.type === "outport",
        )
    )
        throw new Error("请选择同一系统中的计算方块，保留现有边界端口。");
    const next = structuredClone(doc);
    upgrade(next);
    const id = idFactory(),
        parent = peers[0]!.parent;
    const incoming = new Map<string, { source: Port; port: Block }>(),
        outgoing = new Map<string, { source: Port; port: Block }>();
    const minX = Math.min(...peers.map((b) => b.position?.x ?? 0)),
        minY = Math.min(...peers.map((b) => b.position?.y ?? 0)),
        maxX = Math.max(...peers.map((b) => b.position?.x ?? 0));
    const replacements: Connection[] = [];
    for (const e of next.model.connections) {
        const a = selected.has(e.from.block),
            b = selected.has(e.to.block);
        if (a === b) {
            replacements.push(e);
            continue;
        }
        if (b) {
            let found = incoming.get(key(e.from));
            if (!found) {
                const port: Block = {
                    id: idFactory(),
                    parent: id,
                    position: { x: 30, y: 100 + incoming.size * 130 },
                    kind: { type: "inport", port: incoming.size + 1 },
                };
                found = { source: e.from, port };
                incoming.set(key(e.from), found);
                replacements.push(
                    wire(e.from, { block: id, port: `in${incoming.size}` }),
                );
            }
            replacements.push(
                wire({ block: found.port.id, port: "out" }, e.to),
            );
        } else {
            let found = outgoing.get(key(e.from));
            if (!found) {
                const port: Block = {
                    id: idFactory(),
                    parent: id,
                    position: {
                        x: 460 + maxX - minX,
                        y: 100 + outgoing.size * 130,
                    },
                    kind: { type: "outport", port: outgoing.size + 1 },
                };
                found = { source: e.from, port };
                outgoing.set(key(e.from), found);
                replacements.push(wire(e.from, { block: port.id, port: "in" }));
            }
            replacements.push(
                wire(
                    {
                        block: id,
                        port: `out${(found.port.kind as { port: number }).port}`,
                    },
                    e.to,
                ),
            );
        }
    }
    if (incoming.size > 64 || outgoing.size > 64)
        throw new Error("一个子系统最多支持 64 个输入和 64 个输出。");
    for (const b of next.model.blocks)
        if (selected.has(b.id)) {
            b.parent = id;
            b.position = {
                x: (b.position?.x ?? 0) - minX + 220,
                y: (b.position?.y ?? 0) - minY + 100,
            };
        }
    next.model.blocks.push(
        {
            id,
            ...(parent ? { parent } : {}),
            position: { x: minX, y: minY },
            kind: {
                type: "subsystem",
                inputs: incoming.size,
                outputs: outgoing.size,
            },
        },
        ...[...incoming.values(), ...outgoing.values()].map((e) => e.port),
    );
    next.editor.labels[id] = "Subsystem";
    for (const { port } of [...incoming.values(), ...outgoing.values()])
        next.editor.labels[port.id] =
            `${port.kind.type === "inport" ? "In" : "Out"}${(port.kind as { port: number }).port}`;
    next.model.connections = replacements;
    cleanBends(next);
    for (const edge of next.model.connections)
        if (selected.has(edge.from.block) || selected.has(edge.to.block))
            delete next.editor.bends[edgeId(edge)];
    validateHierarchy(next.model.blocks);
    return { document: next, id };
}
export function ungroupBlock(doc: ModelDocument, id: string): ModelDocument {
    const next = structuredClone(doc),
        sub = next.model.blocks.find((b) => b.id === id);
    if (sub?.kind.type !== "subsystem") throw new Error("请选择一个子系统。");
    if (sub.kind.execution)
        throw new Error(
            "条件子系统保留执行边界；如需展开，请先在检查器中改为普通子系统。",
        );
    const boundaries = next.model.blocks.filter(
        (b) =>
            b.parent === id &&
            (b.kind.type === "inport" || b.kind.type === "outport"),
    );
    const removed = new Set([id, ...boundaries.map((b) => b.id)]),
        drivers = new Map(
            next.model.connections.map((e) => [key(e.to), e.from]),
        );
    const aliases = new Map<string, Port | undefined>();
    for (const b of boundaries) {
        if (b.kind.type === "inport")
            aliases.set(
                key({ block: b.id, port: "out" }),
                drivers.get(key({ block: id, port: `in${b.kind.port}` })),
            );
        else if (b.kind.type === "outport")
            aliases.set(
                key({ block: id, port: `out${b.kind.port}` }),
                drivers.get(key({ block: b.id, port: "in" })),
            );
    }
    const resolve = (source: Port): Port | undefined => {
        let p: Port | undefined = source;
        const seen = new Set<string>();
        while (p && aliases.has(key(p))) {
            if (seen.has(key(p)))
                throw new Error("边界端口存在循环，无法展开。");
            seen.add(key(p));
            p = aliases.get(key(p));
        }
        return p;
    };
    next.model.connections = next.model.connections.flatMap((e) => {
        if (removed.has(e.to.block)) return [];
        const source = resolve(e.from);
        return source ? [wire(source, e.to)] : [];
    });
    next.model.blocks = next.model.blocks.filter((b) => !removed.has(b.id));
    const moved = new Set(
        next.model.blocks.filter((b) => b.parent === id).map((b) => b.id),
    );
    for (const b of next.model.blocks)
        if (b.parent === id) {
            if (sub.parent) b.parent = sub.parent;
            else delete b.parent;
            b.position = {
                x: (b.position?.x ?? 0) + (sub.position?.x ?? 0) - 220,
                y: (b.position?.y ?? 0) + (sub.position?.y ?? 0) - 100,
            };
        }
    for (const removedId of removed) {
        delete next.editor.labels[removedId];
        if (next.model.sampleTimes) delete next.model.sampleTimes[removedId];
    }
    cleanBends(next);
    for (const edge of next.model.connections)
        if (moved.has(edge.from.block) || moved.has(edge.to.block))
            delete next.editor.bends[edgeId(edge)];
    validateHierarchy(next.model.blocks);
    return next;
}
export function cleanBends(doc: ModelDocument) {
    const kept = new Set(doc.model.connections.map(edgeId));
    for (const id of Object.keys(doc.editor.bends))
        if (!kept.has(id)) delete doc.editor.bends[id];
}
