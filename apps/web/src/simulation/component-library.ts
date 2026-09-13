import type { WorkspaceClient } from "../workspace/workspace-client";
import type { DesignerSourceWorkspace } from "../documents/designer-source-workspace";
import {
    callbacks,
    CALLBACK_ROLES,
    parseComponentFile,
    validateComponent,
    type ComponentDefinition,
} from "./components";
import { sourcePath } from "./function-sources";
import delay from "../../../../simulation/examples/components/delay/delay.omblock.json";
import plant from "../../../../simulation/examples/components/mass_spring/mass_spring.omblock.json";
import pi from "../../../../simulation/examples/components/pi/pi.omblock.json";

const bundledSources = import.meta.glob<string>(
    "../../../../simulation/examples/components/**/*.m",
    {
        eager: true,
        query: "?raw",
        import: "default",
    },
);
export interface LibraryEntry {
    key: string;
    definition: ComponentDefinition;
    path?: string;
    sources?: Record<string, string>;
}
function bundled(slug: string, raw: unknown): LibraryEntry {
    const definition = validateComponent(raw, true);
    return {
        key: "builtin:" + slug,
        definition,
        sources: Object.fromEntries(
            callbacks(definition).map((c) => {
                const source =
                    bundledSources[
                        `../../../../simulation/examples/components/${slug}/${c.source}`
                    ];
                if (source === undefined)
                    throw new Error("Bundled component source is missing");
                return [c.source, source];
            }),
        ),
    };
}
export const COMPONENT_TEMPLATES = [
    bundled("delay", delay.definition),
    bundled("mass_spring", plant.definition),
    bundled("pi", pi.definition),
];
export function callbackSkeletons(
    d: ComponentDefinition,
): Record<string, string> {
    return Object.fromEntries(
        CALLBACK_ROLES.flatMap((role) => {
            const c = d[role];
            if (!c) return [];
            const output =
                role === "outputsFunction"
                    ? "y"
                    : role === "derivatives"
                      ? "dx"
                      : "z";
            const width =
                role === "initialize"
                    ? d.continuousStates + d.discreteStates
                    : role === "outputsFunction"
                      ? d.outputs.reduce((s, p) => s + p.width, 0)
                      : d.continuousStates;
            const body =
                role === "update"
                    ? "q"
                    : role === "outputsFunction" &&
                        d.inputs.reduce((s, p) => s + p.width, 0) === width
                      ? "u"
                      : `zeros(${width}, 1)`;
            return [
                [
                    c.source,
                    `function ${output} = ${c.entry}(${role === "initialize" ? "p" : "t, x, q, u, p"})\n% Replace this initial implementation with your component equations.\n${output} = ${body};\nend\n`,
                ],
            ];
        }),
    );
}
export async function discoverComponents(
    workspace: WorkspaceClient,
): Promise<{ entries: LibraryEntry[]; issues: string[] }> {
    const listing = await workspace.list("", true);
    const candidates = listing.entries.filter(
        (e) => e.kind === "file" && e.path.endsWith(".omblock.json"),
    );
    const issues: string[] = [];
    if (candidates.length > 64)
        issues.push("项目组件超过 64 个，只读取前 64 个。");
    const entries: LibraryEntry[] = [];
    // Bounded batches keep large folders from creating an unbounded read queue.
    for (let start = 0; start < Math.min(candidates.length, 64); start += 8) {
        const batch = await Promise.allSettled(
            candidates.slice(start, start + 8).map(async (entry) => {
                if (entry.size !== null && entry.size > 65536)
                    throw new Error("定义超过 64 KiB");
                const file = await workspace.read(entry.path);
                return {
                    key: entry.path,
                    path: entry.path,
                    definition: parseComponentFile(file.content),
                };
            }),
        );
        batch.forEach((result, index) => {
            if (result.status === "fulfilled") entries.push(result.value);
            else
                issues.push(
                    `${candidates[start + index]!.path}: ${String(result.reason)}`,
                );
        });
    }
    return { entries, issues };
}
export async function librarySources(
    entry: LibraryEntry,
    workspace: WorkspaceClient,
    shared?: DesignerSourceWorkspace,
): Promise<Record<string, string>> {
    if (entry.sources) return entry.sources;
    const captured = callbacks(entry.definition).map((c) => {
        const path = sourcePath(entry.path ?? null, c.source);
        return { reference: c.source, path, draft: shared?.getSource(path) };
    });
    const files = await Promise.all(
        captured.map(
            async (c) =>
                [
                    c.reference,
                    c.draft?.content ?? (await workspace.read(c.path)).content,
                ] as const,
        ),
    );
    if (files.some(([, text]) => new TextEncoder().encode(text).length > 65536))
        throw new Error("组件源码超过每文件 64 KiB 上限。");
    return Object.fromEntries(files);
}
/** Each model owns editable copies; library files stay reusable. */
export function copyComponentSources(
    definition: ComponentDefinition,
    sources: Record<string, string>,
    suffix: string,
): { definition: ComponentDefinition; sources: Record<string, string> } {
    const copy = structuredClone(definition),
        result: Record<string, string> = {};
    const remap = new Map<string, { source: string; entry: string }>();
    for (const callback of callbacks(copy)) {
        const source = sources[callback.source];
        if (source === undefined)
            throw new Error(`缺少源码 ${callback.source}`);
        let mapped = remap.get(callback.source);
        if (!mapped) {
            const name = `${callback.entry.slice(0, 36)}_${suffix}_${remap.size}`;
            mapped = { source: name + ".m", entry: name };
            const escaped = callback.entry.replace(
                /[.*+?^${}()|[\]\\]/g,
                "\\$&",
            );
            const declaration = new RegExp(
                `^([ \\t]*function[ \\t]+(?:[A-Za-z]\\w*|\\[[ \\t]*[A-Za-z]\\w*[ \\t]*\\])[ \\t]*=[ \\t]*)${escaped}([ \\t]*\\()`,
                "m",
            );
            if (!declaration.test(source))
                throw new Error(`源码声明与函数名 ${callback.entry} 不符。`);
            result[mapped.source] = source.replace(declaration, `$1${name}$2`);
            remap.set(callback.source, mapped);
        }
        Object.assign(callback, mapped);
    }
    return { definition: copy, sources: result };
}
