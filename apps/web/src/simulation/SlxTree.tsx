import type { SlxImport } from "./client";
import { slxSystemPaths } from "./slx-authoring";

export default function SlxTree({
    document,
    active,
    selected,
    onSystem,
    onSelect,
}: {
    document: SlxImport["document"];
    active: number;
    selected: string | null;
    onSystem(index: number): void;
    onSelect(sid: string): void;
}) {
    const paths = slxSystemPaths(document);
    const children = new Map(
        document.systems.map((system, index) => [system.parentBlock, index]),
    );
    const system = document.systems[active] ?? document.systems[0]!;
    return (
        <section className="sim-tree">
            <div className="sim-pane-title">SLX 层级</div>
            <div className="sim-tree-scroll">
                <nav aria-label="SLX 系统层级">
                    {document.systems.map((system, index) => (
                        <button
                            key={system.parentBlock ?? "root"}
                            title={paths[index]}
                            className={index === active ? "selected" : ""}
                            onClick={() => onSystem(index)}
                        >
                            <span>
                                {system.parentBlock === null ? "◇" : "▣"}{" "}
                                {paths[index]}
                            </span>
                        </button>
                    ))}
                </nav>
                <div role="tree" aria-label="SLX 方块树">
                    {system.blocks.map((block) => (
                        <button
                            role="treeitem"
                            aria-selected={block.sid === selected}
                            key={block.sid}
                            className={block.sid === selected ? "selected" : ""}
                            onClick={() => onSelect(block.sid)}
                            onDoubleClick={() => {
                                const child = children.get(block.sid);
                                if (child !== undefined) onSystem(child);
                            }}
                        >
                            <span>
                                {children.has(block.sid) ? "▣" : "▫"}{" "}
                                {block.name}
                            </span>
                            <small>{block.blockType}</small>
                        </button>
                    ))}
                </div>
            </div>
        </section>
    );
}
