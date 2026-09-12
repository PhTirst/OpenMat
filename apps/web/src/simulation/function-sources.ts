import type { DesignerSourceWorkspace } from "../documents/designer-source-workspace";
import type { WorkspaceClient } from "../workspace/workspace-client";
import { SimulationError } from "./client";
import { validSourcePath, type FunctionKind, type Model } from "./model";
import { modelSources, blockSources } from "./components";

export interface FunctionSourceSnapshot {
    reference: string;
    path: string;
    content: string;
    revision: string;
}
export function sourcePath(
    modelPath: string | null,
    reference: string,
): string {
    if (!validSourcePath(reference))
        throw new Error("函数源码必须使用相对 .m 路径，不能包含 ..。");
    const parent = modelPath
        ?.replaceAll("\\", "/")
        .split("/")
        .slice(0, -1)
        .join("/");
    return parent ? `${parent}/${reference}` : reference;
}
export function functionTemplate(kind: FunctionKind): string {
    const arguments_ = [...kind.inputs, ...kind.parameters]
        .map((p) => p.name)
        .join(", ");
    const output =
        kind.inputs.length && kind.inputs[0]!.width === kind.outputWidth
            ? kind.inputs[0]!.name
            : `[${Array.from({ length: kind.outputWidth }, () => "0").join("; ")}]`;
    return `function y = ${kind.entry}(${arguments_})\n% Pure, fixed-size real column inputs and output.\ny = ${output};\nend\n`;
}
export async function readFunctionSources(
    model: Model,
    modelPath: string | null,
    workspace: WorkspaceClient,
    shared?: DesignerSourceWorkspace,
): Promise<FunctionSourceSnapshot[]> {
    const references = modelSources(model);
    // Capture all existing drafts before awaiting any filesystem reads.
    const drafts = references.map((reference) => {
        const path = sourcePath(modelPath, reference),
            doc = shared?.getSource(path);
        return { reference, path, doc };
    });
    const files: FunctionSourceSnapshot[] = [];
    for (const { reference, path, doc } of drafts) {
        try {
            if (doc)
                files.push({
                    reference,
                    path,
                    content: doc.content,
                    revision: doc.revision,
                });
            else {
                const file = await workspace.read(path);
                shared?.ensureSource({
                    path,
                    content: file.content,
                    savedContent: file.content,
                    file,
                });
                files.push({
                    reference,
                    path,
                    content: file.content,
                    revision: file.revision,
                });
            }
        } catch (error) {
            const block = model.blocks.find((b) =>
                blockSources(model, b).includes(reference),
            );
            throw new SimulationError({
                code: "source_file",
                message: `无法读取 ${path}：${error instanceof Error ? error.message : String(error)}`,
                sourcePath: reference,
                ...(block ? { block: block.id } : {}),
            });
        }
    }
    let bytes = 0;
    for (const file of files) {
        const size = new TextEncoder().encode(file.content).length;
        bytes += size;
        if (size > 65536 || bytes > 1048576 || files.length > 64)
            throw new Error(
                "函数源码超过上限：每文件 64 KiB、总共 1 MiB、最多 64 个文件。",
            );
    }
    return files;
}
export function sourceBundle(
    files: FunctionSourceSnapshot[],
): Record<string, string> {
    return Object.fromEntries(
        files.map((file) => [file.reference, file.content]),
    );
}
export async function saveFunctionSources(
    files: FunctionSourceSnapshot[],
    modelPath: string,
    workspace: WorkspaceClient,
    rootGeneration: number,
    shared?: DesignerSourceWorkspace,
): Promise<void> {
    for (const snapshot of files) {
        const target = sourcePath(modelPath, snapshot.reference);
        let revision = target === snapshot.path ? snapshot.revision : "";
        if (!revision) {
            // create is exclusive: Save As cannot overwrite an unrelated .m file.
            await workspace.create(target, "file");
            const created = await workspace.read(target);
            revision = created.revision;
            // Keep the new file's baseline even if the subsequent write fails.
            // A retry can then use its revision without losing the source draft.
            if (target === snapshot.path) shared?.acceptSaved(created);
        }
        const written = await workspace.write(
            target,
            snapshot.content,
            revision,
            rootGeneration,
        );
        const file = await workspace.read(target);
        if (file.revision !== written.revision)
            throw new Error(
                `${target} 在保存期间被外部修改，请检查文件后重试。`,
            );
        shared?.acceptSaved(file);
    }
}
