import { parseEventPlan, type EventPlan } from "./hybrid";
import { parseSamplingPlan, type SamplingPlan } from "./sampling";
import type {
    SlxImport,
    SlxBlock,
    SlxLine,
    SimulationDiagnostic,
} from "./client";

export interface SlxAsset {
    profile?: "control-v1" | "multirate-v1" | "hybrid-v1";
    sampling?: SamplingPlan;
    eventPlan?: EventPlan;
    name: string;
    package: string;
    parameters: string;
    appliedParameters: string;
    runnable: boolean;
    issues: SimulationDiagnostic[];
    document: SlxImport["document"];
    snapshotEdited?: boolean;
}

export function encodeSlx(bytes: Uint8Array): string {
    let binary = "";
    for (let offset = 0; offset < bytes.length; offset += 8192)
        binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
    return btoa(binary);
}
export function decodeSlx(packageText: string): Uint8Array<ArrayBuffer> {
    if (
        packageText.length > 2796204 ||
        packageText.length % 4 !== 0 ||
        !/^[A-Za-z0-9+/]*={0,2}$/.test(packageText)
    )
        throw new Error("SLX 源文件编码无效或超过 2 MiB。");
    const value = Uint8Array.from(atob(packageText), (c) => c.charCodeAt(0));
    if (!value.length || value.length > 2 * 1024 * 1024)
        throw new Error("SLX 源文件为空或超过 2 MiB。");
    return value;
}

function record(raw: unknown): Record<string, unknown> {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
        throw new Error("SLX 元数据无效。");
    return raw as Record<string, unknown>;
}
function string(raw: unknown, limit = 65536): string {
    if (typeof raw !== "string" || new TextEncoder().encode(raw).length > limit)
        throw new Error("SLX 元数据文本无效或过长。");
    return raw;
}
function properties(raw: unknown): Record<string, string> {
    return Object.fromEntries(
        Object.entries(record(raw)).map(([key, value]) => [
            string(key, 1024),
            string(value),
        ]),
    );
}
export function validateEmbeddedSources(raw: unknown): Record<string, string> {
    let total = 0;
    const entries = Object.entries(record(raw));
    if (entries.length > 64) throw new Error("内嵌源码最多 64 个文件。");
    return Object.fromEntries(
        entries.map(([path, source]) => {
            if (
                path.length > 512 ||
                !path.endsWith(".m") ||
                /[\\:\0]/.test(path) ||
                path.split("/").some((p) => !p || p === "." || p === "..")
            )
                throw new Error("内嵌源码需要合法的相对 .m 路径。");
            const text = string(source);
            total += new TextEncoder().encode(text).length;
            if (total > 1048576) throw new Error("内嵌源码总计超过 1 MiB。");
            return [path, text];
        }),
    );
}

export function validateSlxAsset(raw: unknown): SlxAsset {
    const asset = record(raw),
        document = record(asset.document);
    if (
        !Array.isArray(document.systems) ||
        !document.systems.length ||
        document.systems.length > 1024
    )
        throw new Error("SLX 系统列表无效。");
    if (
        asset.profile !== undefined &&
        asset.profile !== "control-v1" &&
        asset.profile !== "multirate-v1" &&
        asset.profile !== "hybrid-v1"
    )
        throw new Error("不支持的 SLX 导入配置。");
    const packageText = string(asset.package, 2796204);
    decodeSlx(packageText);
    const ids = new Set<string>();
    let lineCount = 0;
    const line = (raw: unknown, depth: number): SlxLine => {
        if (depth > 64 || ++lineCount > 640000)
            throw new Error("SLX 连线超过限制。");
        const value = record(raw);
        if (!Array.isArray(value.branches)) throw new Error("SLX 分支无效。");
        return {
            properties: properties(value.properties),
            branches: value.branches.map((b) => line(b, depth + 1)),
        };
    };
    const systems = document.systems.map((raw) => {
        const system = record(raw);
        if (!Array.isArray(system.blocks) || !Array.isArray(system.lines))
            throw new Error("SLX 系统结构无效。");
        const blocks: SlxBlock[] = system.blocks.map((raw) => {
            const block = record(raw),
                sid = string(block.sid, 128);
            if (ids.has(sid) || ids.size >= 10000)
                throw new Error("SLX 方块 ID 重复或过多。");
            ids.add(sid);
            return {
                sid,
                name: string(block.name, 1024),
                blockType: string(block.blockType, 256),
                properties: properties(block.properties),
                source: { part: string(record(block.source).part, 1024) },
            };
        });
        return {
            parentBlock:
                system.parentBlock === null
                    ? null
                    : string(system.parentBlock, 128),
            blocks,
            lines: system.lines.map((raw) => line(raw, 0)),
        };
    });
    const childSystems = new Set<string>();
    const owners = new Map(
        systems.flatMap((s) =>
            s.blocks.map((b) => [b.sid, s.parentBlock] as const),
        ),
    );
    if (systems.filter((s) => s.parentBlock === null).length !== 1)
        throw new Error("SLX 需要唯一根系统。");
    for (const system of systems) {
        if (system.parentBlock === null) continue;
        if (
            !ids.has(system.parentBlock) ||
            childSystems.has(system.parentBlock)
        )
            throw new Error("SLX 子系统引用无效。");
        childSystems.add(system.parentBlock);
        const visited = new Set<string>();
        let parent: string | null | undefined = system.parentBlock;
        while (parent != null) {
            if (visited.has(parent) || visited.size >= 32)
                throw new Error("SLX 子系统循环或层级过深。");
            visited.add(parent);
            parent = owners.get(parent);
        }
    }
    if (
        typeof asset.runnable !== "boolean" ||
        !Array.isArray(asset.issues) ||
        asset.issues.length > 20000
    )
        throw new Error("SLX 兼容性记录无效。");
    const issues = asset.issues.map((raw): SimulationDiagnostic => {
        const issue = record(raw);
        return {
            code: string(issue.code, 256),
            message: string(issue.message),
            ...(issue.block === undefined
                ? {}
                : { block: string(issue.block, 128) }),
            ...(issue.parameter === undefined
                ? {}
                : { parameter: string(issue.parameter, 1024) }),
            ...(issue.part === undefined
                ? {}
                : { part: string(issue.part, 1024) }),
        };
    });
    return {
        ...(asset.profile === undefined
            ? {}
            : {
                  profile: asset.profile as
                      "control-v1" | "multirate-v1" | "hybrid-v1",
              }),
        ...(asset.eventPlan == null
            ? {}
            : { eventPlan: parseEventPlan(asset.eventPlan) }),
        ...(asset.sampling == null
            ? {}
            : { sampling: parseSamplingPlan(asset.sampling) }),
        name: string(asset.name, 1024),
        package: packageText,
        parameters: string(asset.parameters),
        appliedParameters: string(asset.appliedParameters),
        runnable: asset.runnable,
        issues,
        ...(asset.snapshotEdited === true ? { snapshotEdited: true } : {}),
        document: {
            name: string(document.name, 1024),
            ...(document.matlabRelease === undefined
                ? {}
                : { matlabRelease: string(document.matlabRelease, 64) }),
            systems,
        },
    };
}

export function slxSystemPaths(document: SlxImport["document"]): string[] {
    const owners = new Map(
        document.systems.flatMap((s, index) =>
            s.blocks.map((b) => [b.sid, { block: b, index }] as const),
        ),
    );
    return document.systems.map((system) => {
        const parts: string[] = [];
        let parent = system.parentBlock;
        for (let depth = 0; parent !== null && depth < 32; depth++) {
            const owner = owners.get(parent);
            if (!owner) break;
            parts.unshift(owner.block.name);
            parent = document.systems[owner.index]!.parentBlock;
        }
        return [document.name, ...parts].join(" / ");
    });
}
