import {
    useState,
    type CSSProperties,
    type DragEvent,
    type MouseEvent as ReactMouseEvent,
    type PointerEvent,
} from "react";
import { EmbeddedFigure } from "../components/FigureWindow";
import type { DisplayEventData } from "../protocol/kernel-v0";
import {
    BUILTIN_CATALOG,
    propertyValue,
    type ComponentSpec,
    type UiValue,
} from "./catalog";
import { ComponentIcon } from "./ComponentIcon";
import type { UiNode } from "./model";
import { sizingStyle, containerStyle } from "./layout-style";

export interface UiInteraction {
    id: string;
    name: string;
    event: string;
    value?: UiValue;
    property?: string;
    row?: number;
    column?: number;
    runtimeId?: string;
}
interface RendererProps {
    node: UiNode;
    selectedId?: string | undefined;
    selectedIds?: readonly string[] | undefined;
    running: boolean;
    catalog?: Readonly<Record<string, ComponentSpec>>;
    figures?: readonly DisplayEventData[];
    graphicsSessionId?: string | undefined;
    onSelect?: ((id: string, additive?: boolean) => void) | undefined;
    onContextMenu?:
        | ((id: string, event: ReactMouseEvent<HTMLElement>) => void)
        | undefined;
    onDrop?: ((id: string, event: DragEvent) => void) | undefined;
    onDragPreview?: ((id: string, event: DragEvent) => void) | undefined;
    onResize?:
        | ((id: string, width: number, height: number) => void)
        | undefined;
    onEvent?: ((event: UiInteraction) => void) | undefined;
    parentMode?: string;
}
const safeImage = (source: string): string | undefined =>
    /^(https?:\/\/|data:image\/(png|jpeg|webp|gif);base64,)/i.test(source)
        ? source
        : undefined;
export function UiRenderer({
    node,
    selectedId,
    selectedIds,
    running,
    catalog = BUILTIN_CATALOG,
    figures = [],
    graphicsSessionId,
    onSelect,
    onContextMenu,
    onDrop,
    onDragPreview,
    onResize,
    onEvent,
    parentMode,
}: RendererProps) {
    const p = (name: string) => propertyValue(node, name, catalog);
    const [localTab, setLocalTab] = useState(0);
    const [split, setSplit] = useState(50);
    const descriptor = catalog[node.type];
    const children = node.children;
    const enabled = p("Enable") !== false;
    const text = (name: string) => String(p(name));
    const numeric = (name: string) => Number(p(name));
    const emit = (
        event: string,
        property?: string,
        value?: UiValue,
        details?: { row: number; column: number },
    ) =>
        onEvent?.({
            id: node.id,
            name: node.name,
            ...(node.runtimeId ? { runtimeId: node.runtimeId } : {}),
            event,
            ...(property === undefined ? {} : { property }),
            ...(value === undefined ? {} : { value }),
            ...details,
        });
    const selected =
        !running &&
        (selectedIds ? selectedIds.includes(node.id) : selectedId === node.id);
    const positionValue = node.properties.Position;
    const position =
        Array.isArray(positionValue) &&
        Array.isArray(positionValue[0]) &&
        positionValue[0].length === 4 &&
        positionValue[0].every(
            (v) => typeof v === "number" && Number.isFinite(v),
        )
            ? (positionValue[0] as number[])
            : null;
    const style: CSSProperties = {
        minWidth: 0,
        minHeight: 0,
        ...(parentMode === "absolute"
            ? {
                  position: "absolute",
                  left: node.layout.x,
                  top: node.layout.y,
                  width: node.layout.width,
                  height: node.layout.height,
                  ...(position
                      ? {
                            left: position[0],
                            top: "auto",
                            bottom: position[1],
                            width: position[2],
                            height: position[3],
                        }
                      : {}),
              }
            : parentMode === "grid"
              ? {
                    gridColumn: `${node.layout.column} / span ${node.layout.columnSpan}`,
                    gridRow: `${node.layout.row} / span ${node.layout.rowSpan}`,
                    minHeight: node.layout.height,
                }
              : {
                    flex: node.layout.grow
                        ? `${node.layout.grow} 1 0`
                        : "0 0 auto",
                    ...(parentMode === "row"
                        ? { width: node.layout.width }
                        : { minHeight: node.layout.height }),
                }),
        ...sizingStyle(node.layout, parentMode),
        ...(p("Visible") === false
            ? running
                ? { display: "none" }
                : { opacity: 0.45 }
            : {}),
    };
    const layoutStyle: CSSProperties = {
        ...containerStyle(node.layout),
        ...((descriptor?.renderType ?? node.type) === "ScrollPanel"
            ? {
                  overflowX:
                      text("ScrollDirection") === "vertical"
                          ? "hidden"
                          : "auto",
                  overflowY:
                      text("ScrollDirection") === "horizontal"
                          ? "hidden"
                          : "auto",
              }
            : {}),
    };
    const renderChild = (child: UiNode, mode = node.layout.mode) => (
        <UiRenderer
            key={child.id}
            node={child}
            selectedId={selectedId}
            selectedIds={selectedIds}
            running={running}
            catalog={catalog}
            figures={figures}
            graphicsSessionId={graphicsSessionId}
            onSelect={onSelect}
            onContextMenu={onContextMenu}
            onDrop={onDrop}
            onDragPreview={onDragPreview}
            onResize={onResize}
            onEvent={onEvent}
            parentMode={mode}
        />
    );
    let content;
    switch (descriptor?.renderType ?? node.type) {
        case "Label":
            content = (
                <span style={{ fontSize: numeric("FontSize") }}>
                    {text("Text")}
                </span>
            );
            break;
        case "Button":
            content = (
                <button
                    type="button"
                    disabled={!enabled}
                    className={`ui-button ${text("Variant") === "primary" ? "ui-primary" : ""}`}
                    onClick={() => emit("Clicked")}
                >
                    {text("Text")}
                </button>
            );
            break;
        case "TextField":
            content = (
                <input
                    aria-label={node.name}
                    disabled={!enabled}
                    value={text("Value")}
                    placeholder={text("Placeholder")}
                    onChange={(e) =>
                        emit("ValueChanged", "Value", e.target.value)
                    }
                />
            );
            break;
        case "TextArea":
            content = (
                <textarea
                    aria-label={node.name}
                    disabled={!enabled}
                    value={text("Value")}
                    placeholder={text("Placeholder")}
                    onChange={(e) =>
                        emit("ValueChanged", "Value", e.target.value)
                    }
                />
            );
            break;
        case "NumericField":
            content = (
                <input
                    type="number"
                    aria-label={node.name}
                    disabled={!enabled}
                    value={numeric("Value")}
                    min={numeric("Min")}
                    max={numeric("Max")}
                    step={numeric("Step")}
                    onChange={(e) => {
                        const value = e.target.valueAsNumber;
                        if (Number.isFinite(value))
                            emit(
                                "ValueChanged",
                                "Value",
                                Math.min(
                                    numeric("Max"),
                                    Math.max(numeric("Min"), value),
                                ),
                            );
                    }}
                />
            );
            break;
        case "CheckBox":
            content = (
                <label className="ui-check">
                    <input
                        type="checkbox"
                        disabled={!enabled}
                        checked={Boolean(p("Value"))}
                        onChange={(e) =>
                            emit("ValueChanged", "Value", e.target.checked)
                        }
                    />
                    {text("Text")}
                </label>
            );
            break;
        case "RadioGroup":
            content = (
                <div className="ui-radio">
                    {(p("Items") as string[]).map((item, index) => (
                        <label key={`${index}:${item}`}>
                            <input
                                type="radio"
                                name={node.id}
                                disabled={!enabled}
                                checked={text("Value") === item}
                                onChange={() =>
                                    emit("ValueChanged", "Value", item)
                                }
                            />
                            {item}
                        </label>
                    ))}
                </div>
            );
            break;
        case "DropDown":
            content = (
                <select
                    aria-label={node.name}
                    disabled={!enabled}
                    value={text("Value")}
                    onChange={(e) =>
                        emit("ValueChanged", "Value", e.target.value)
                    }
                >
                    {(p("Items") as string[]).map((item, index) => (
                        <option key={`${index}:${item}`}>{item}</option>
                    ))}
                </select>
            );
            break;
        case "Slider":
            content = (
                <div className="ui-slider">
                    <input
                        type="range"
                        aria-label={node.name}
                        disabled={!enabled}
                        min={numeric("Min")}
                        max={numeric("Max")}
                        step={numeric("Step")}
                        value={numeric("Value")}
                        onChange={(e) =>
                            emit(
                                "ValueChanged",
                                "Value",
                                e.target.valueAsNumber,
                            )
                        }
                    />
                    <output>{numeric("Value")}</output>
                </div>
            );
            break;
        case "ProgressBar":
            content = (
                <div className="ui-progress">
                    <progress
                        aria-label={node.name}
                        value={numeric("Value")}
                        max={100}
                    />
                    <span>{numeric("Value")}%</span>
                </div>
            );
            break;
        case "StatusLamp":
            content = (
                <span className="ui-lamp">
                    <i style={{ backgroundColor: text("Color") }} />
                    {text("Text")}
                </span>
            );
            break;
        case "Image":
            content = safeImage(text("Source")) ? (
                <img
                    src={safeImage(text("Source"))}
                    alt={text("Alt")}
                    style={{
                        objectFit:
                            text("Fit") === "cover" ? "cover" : "contain",
                    }}
                />
            ) : (
                <div className="ui-placeholder">
                    <ComponentIcon type="Image" size={32} />
                    <span>{text("Alt")}</span>
                </div>
            );
            break;
        case "PlotView":
            content = running ? (
                <EmbeddedFigure
                    sessionId={graphicsSessionId}
                    figures={figures}
                    figureNumber={numeric("FigureIndex")}
                    label={text("Title")}
                />
            ) : (
                <div className="ui-plot-placeholder">
                    <span>{text("Title")}</span>
                    <div className="ui-plot-grid">
                        <ComponentIcon type="PlotView" size={42} />
                        <small>
                            Rust Plot Engine · Figure {numeric("FigureIndex")}
                        </small>
                    </div>
                </div>
            );
            break;
        case "Table": {
            const data = p("Data") as (string | number | boolean)[][];
            content = (
                <div className="ui-table-scroll">
                    <table>
                        <thead>
                            <tr>
                                {(p("Columns") as string[]).map((column, i) => (
                                    <th key={i}>{column}</th>
                                ))}
                            </tr>
                        </thead>
                        <tbody>
                            {data.map((row, rowIndex) => (
                                <tr
                                    key={rowIndex}
                                    onClick={() =>
                                        emit(
                                            "SelectionChanged",
                                            undefined,
                                            undefined,
                                            { row: rowIndex + 1, column: 1 },
                                        )
                                    }
                                >
                                    {row.map((cell, columnIndex) => (
                                        <td key={columnIndex}>
                                            {p("Editable") && running ? (
                                                <input
                                                    aria-label={`${node.name} ${rowIndex + 1},${columnIndex + 1}`}
                                                    disabled={!enabled}
                                                    value={String(cell)}
                                                    onChange={(e) => {
                                                        const updated =
                                                            data.map((r) => [
                                                                ...r,
                                                            ]);
                                                        updated[rowIndex]![
                                                            columnIndex
                                                        ] =
                                                            typeof cell ===
                                                                "number" &&
                                                            e.target.value.trim() &&
                                                            Number.isFinite(
                                                                Number(
                                                                    e.target
                                                                        .value,
                                                                ),
                                                            )
                                                                ? Number(
                                                                      e.target
                                                                          .value,
                                                                  )
                                                                : e.target
                                                                      .value;
                                                        emit(
                                                            "CellEdited",
                                                            "Data",
                                                            updated,
                                                            {
                                                                row:
                                                                    rowIndex +
                                                                    1,
                                                                column:
                                                                    columnIndex +
                                                                    1,
                                                            },
                                                        );
                                                    }}
                                                />
                                            ) : (
                                                String(cell)
                                            )}
                                        </td>
                                    ))}
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            );
            break;
        }
        case "TabGroup": {
            const active = Math.max(
                0,
                Math.min(
                    children.length - 1,
                    running ? numeric("SelectedIndex") - 1 : localTab,
                ),
            );
            content = (
                <>
                    <div
                        className="ui-tabs"
                        role="tablist"
                        aria-label={node.name}
                    >
                        {children.map((child, index) => (
                            <button
                                type="button"
                                role="tab"
                                aria-selected={active === index}
                                key={child.id}
                                onClick={() => {
                                    setLocalTab(index);
                                    if (running)
                                        emit(
                                            "SelectionChanged",
                                            "SelectedIndex",
                                            index + 1,
                                        );
                                    else onSelect?.(child.id);
                                }}
                            >
                                {String(propertyValue(child, "Title", catalog))}
                            </button>
                        ))}
                    </div>
                    <div className="ui-layout ui-tab-content">
                        {children[active] ? (
                            renderChild(children[active]!)
                        ) : (
                            <span className="ui-empty">拖入选项卡</span>
                        )}
                    </div>
                </>
            );
            break;
        }
        case "SplitPane": {
            const vertical = text("Orientation") === "vertical";
            content = (
                <div
                    className="ui-split"
                    style={{ flexDirection: vertical ? "column" : "row" }}
                >
                    {children.map((child, i) => (
                        <div
                            key={child.id}
                            className="ui-split-child"
                            style={{
                                flex: `0 0 calc(${i === 0 ? split : 100 - split}% - 4px)`,
                            }}
                        >
                            {renderChild(child, "column")}
                            {i === 0 && children.length === 2 ? (
                                <input
                                    className="ui-split-range"
                                    type="range"
                                    aria-label={`${node.name} 分隔位置`}
                                    min={15}
                                    max={85}
                                    value={split}
                                    onChange={(e) =>
                                        setSplit(e.target.valueAsNumber)
                                    }
                                />
                            ) : null}
                        </div>
                    ))}
                </div>
            );
            break;
        }
        default:
            content = (
                <>
                    {["Panel", "Window", "ComponentContainer"].includes(
                        descriptor?.renderType ?? node.type,
                    ) && text("Title") ? (
                        <div className="ui-container-title">
                            {text("Title")}
                        </div>
                    ) : null}
                    <div className="ui-layout" style={layoutStyle}>
                        {children.map((child) => renderChild(child))}
                        {children.length === 0 ? (
                            <div className="ui-empty">
                                {running
                                    ? ""
                                    : descriptor
                                      ? "拖入组件，或从组件库单击添加"
                                      : `未加载组件 ${node.type}`}
                            </div>
                        ) : null}
                    </div>
                </>
            );
    }
    const beginResize = (event: PointerEvent<HTMLButtonElement>) => {
        event.stopPropagation();
        event.preventDefault();
        const button = event.currentTarget;
        const wrapper = button.parentElement!;
        const bounds = wrapper.getBoundingClientRect();
        const sx = event.clientX;
        const sy = event.clientY;
        const scale = bounds.width / wrapper.offsetWidth;
        button.setPointerCapture(event.pointerId);
        const end = (e: globalThis.PointerEvent) => {
            button.removeEventListener("pointerup", end);
            button.removeEventListener("pointercancel", cancel);
            onResize?.(
                node.id,
                Math.max(
                    24,
                    Math.round((bounds.width + e.clientX - sx) / scale),
                ),
                Math.max(
                    24,
                    Math.round((bounds.height + e.clientY - sy) / scale),
                ),
            );
        };
        const cancel = () => {
            button.removeEventListener("pointerup", end);
            button.removeEventListener("pointercancel", cancel);
        };
        button.addEventListener("pointerup", end);
        button.addEventListener("pointercancel", cancel);
    };
    return (
        <div
            className={`ui-node ui-${descriptor?.renderType ?? node.type} ${selected ? "ui-selected" : ""} ${descriptor?.container ? "ui-container" : ""}`}
            style={style}
            title={text("Tooltip")}
            data-ui-id={node.id}
            tabIndex={running ? undefined : -1}
            onContextMenu={
                running
                    ? undefined
                    : (event) => {
                          if (onContextMenu) {
                              event.preventDefault();
                              event.stopPropagation();
                              onContextMenu(node.id, event);
                          }
                      }
            }
            onClick={
                running
                    ? undefined
                    : (e) => {
                          e.stopPropagation();
                          onSelect?.(
                              node.id,
                              e.shiftKey || e.ctrlKey || e.metaKey,
                          );
                      }
            }
            draggable={
                !running &&
                !node.sourceOwned &&
                node.type !== "Window" &&
                parentMode !== "absolute"
            }
            onDragStart={(e) => {
                if (node.sourceOwned) {
                    e.preventDefault();
                    return;
                }
                e.stopPropagation();
                e.dataTransfer.setData("application/x-openmat-node", node.id);
                e.dataTransfer.effectAllowed = "move";
            }}
            onDragOver={(e) => {
                if (!running && !node.sourceOwned && descriptor?.container) {
                    e.preventDefault();
                    e.stopPropagation();
                    onDragPreview?.(node.id, e);
                    e.dataTransfer.dropEffect = e.dataTransfer.types.includes(
                        "application/x-openmat-component",
                    )
                        ? "copy"
                        : "move";
                }
            }}
            onDrop={(e) => {
                if (!running && !node.sourceOwned && descriptor?.container) {
                    e.preventDefault();
                    e.stopPropagation();
                    onDrop?.(node.id, e);
                }
            }}
        >
            <div
                inert={!running && !descriptor?.container}
                className={`ui-content ${!running && !descriptor?.container ? "ui-design-control" : ""}`}
            >
                {content}
            </div>
            {selected ? (
                <>
                    <span className="ui-selection-label">{node.name}</span>
                    {node.type !== "Window" && selectedId === node.id ? (
                        <button
                            className="ui-resize"
                            type="button"
                            aria-label={`调整 ${node.name} 大小`}
                            onPointerDown={beginResize}
                        />
                    ) : null}
                </>
            ) : null}
        </div>
    );
}
