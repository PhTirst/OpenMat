import { useEffect, useState } from "react";
import type { WorkspaceClient } from "../workspace/workspace-client";
import { BlockIcon } from "./BlockNode";
import {
    COMPONENT_TEMPLATES,
    discoverComponents,
    type LibraryEntry,
} from "./component-library";
import type { ComponentDefinition } from "./components";

export function ComponentLibrary({
    workspace,
    generation,
    refresh,
    filter,
    disabled,
    definitions,
    onInsert,
    onDrag,
    onCreate,
}: {
    workspace: WorkspaceClient;
    generation: number;
    refresh: number;
    filter: string;
    disabled: boolean;
    definitions: ComponentDefinition[];
    onInsert(entry: LibraryEntry): void;
    onDrag(entry: LibraryEntry): void;
    onCreate(): void;
}) {
    const [entries, setEntries] = useState<LibraryEntry[]>([]),
        [issues, setIssues] = useState<string[]>([]);
    const [loading, setLoading] = useState(false),
        [version, setVersion] = useState(0);
    useEffect(() => {
        let active = true;
        setLoading(true);
        void discoverComponents(workspace)
            .then((result) => {
                if (active) {
                    setEntries(result.entries);
                    setIssues(result.issues);
                }
            })
            .catch((e) => {
                if (active) {
                    setEntries([]);
                    setIssues([String(e)]);
                }
            })
            .finally(() => {
                if (active) setLoading(false);
            });
        return () => {
            active = false;
        };
    }, [workspace, generation, refresh, version]);
    const item = (entry: LibraryEntry) => (
        <button
            key={entry.key}
            title={entry.path ?? entry.definition.category}
            disabled={disabled}
            draggable={!disabled}
            onClick={() => onInsert(entry)}
            onDragStart={(e) => {
                onDrag(entry);
                e.dataTransfer.setData(
                    "application/openmat-component",
                    entry.key,
                );
                e.dataTransfer.effectAllowed = "copy";
            }}
        >
            <BlockIcon type={entry.definition.icon} />
            <span>
                {entry.definition.name}
                <small className="sim-library-category">
                    {entry.definition.category}
                </small>
            </span>
            <span className="sim-add-hint">＋</span>
        </button>
    );
    const visible = (entry: LibraryEntry) =>
        `${entry.definition.name} ${entry.definition.category}`
            .toLowerCase()
            .includes(filter.toLowerCase());
    return (
        <>
            <section>
                <h3>组件模板</h3>
                {COMPONENT_TEMPLATES.filter(visible).map(item)}
                <button disabled={disabled} onClick={onCreate}>
                    <BlockIcon type="component" />
                    <span>新建自定义组件…</span>
                </button>
            </section>
            {!!definitions.length && (
                <section>
                    <h3>模型内的组件</h3>
                    {definitions
                        .map((definition) => ({
                            key: `model:${definition.id}`,
                            definition,
                        }))
                        .filter(visible)
                        .map(item)}
                </section>
            )}
            <section>
                <h3 className="sim-library-heading">
                    项目组件库
                    <button
                        aria-label="刷新项目组件库"
                        disabled={loading}
                        onClick={() => setVersion((v) => v + 1)}
                    >
                        ↻
                    </button>
                </h3>
                {entries.filter(visible).map(item)}
                {loading ? (
                    <p className="sim-help">正在读取组件库…</p>
                ) : (
                    !entries.length && (
                        <p className="sim-help">
                            从检查器保存组件后，这里会显示项目中的 .omblock.json
                            文件。
                        </p>
                    )
                )}
                {!!issues.length && (
                    <details className="sim-help">
                        <summary>组件库读取提示 ({issues.length})</summary>
                        {issues.map((issue, i) => (
                            <p key={i}>{issue}</p>
                        ))}
                    </details>
                )}
            </section>
        </>
    );
}
