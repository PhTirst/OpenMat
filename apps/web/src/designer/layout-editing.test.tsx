import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { BUILTIN_CATALOG } from "./catalog";
import {
    createDocument,
    createNode,
    historyReducer,
    validateDocument,
} from "./model";
import {
    arrangeNodes,
    duplicateNodes,
    moveNodes,
    nudgeNodes,
    patchLayouts,
    removeNodes,
    wrapNodes,
} from "./operations";
import { gridTracks } from "./layout-schema";
import { parseUi, serializeUi } from "./xml";
import { buildUiProgram, parseUiPayload } from "./runtime";
import { containerStyle, sizingStyle } from "./layout-style";
import { UiRenderer } from "./UiRenderer";
import { snapMove } from "./canvas-geometry";
import { commonProperties } from "./MultiInspector";
import { PropertyInput } from "./Inspector";

function fixture(mode: "absolute" | "grid" | "row" = "absolute") {
    const document = createDocument("Demo");
    document.root.layout.mode = mode;
    for (let i = 0; i < 3; i++) {
        const node = createNode("Button", document.root);
        node.layout = {
            ...node.layout,
            x: 10 + i * 120,
            y: 20 + i * 10,
            width: 80,
            height: 32,
            row: i + 1,
            column: 1,
        };
        document.root.children.push(node);
    }
    return {
        document,
        nodes: document.root.children,
        ids: document.root.children.map((n) => n.id),
    };
}
describe("layout constraints and editing transactions", () => {
    it("commits typed arrays when editing mixed list and table properties", () => {
        for (const type of ["items", "table"] as const) {
            const changed = vi.fn();
            const view = render(
                <PropertyInput
                    property={{
                        name: "Data",
                        label: "数据",
                        type,
                        default: [],
                    }}
                    value={[]}
                    mixed
                    onChange={changed}
                />,
            );
            const input = view.getByRole("textbox", { name: "数据" });
            fireEvent.blur(input);
            expect(changed).not.toHaveBeenCalled();
            fireEvent.change(input, {
                target: {
                    value: type === "items" ? "alpha\nbeta" : '[[1,"two"]]',
                },
            });
            fireEvent.blur(input);
            expect(changed).toHaveBeenCalledWith(
                type === "items" ? ["alpha", "beta"] : [[1, "two"]],
            );
            view.unmount();
        }
    });
    it("round trips an explicit layout revision while retaining legacy documents", () => {
        const { document, ids } = fixture();
        expect(serializeUi(document)).not.toContain("layoutVersion");
        const next = patchLayouts(document, ids, {
            widthMode: "fill",
            minWidth: 24,
            maxWidth: 800,
            growX: 2,
            rowTracks: "auto 1fr",
            columnTracks: "240 2fr",
        });
        const xml = serializeUi(next);
        expect(xml).toContain('layoutVersion="2"');
        expect(parseUi(xml)).toEqual(next);
        expect(buildUiProgram(next, BUILTIN_CATALOG)).toContain(
            ".Layout.ColumnTracks = '240 2fr';",
        );
        expect(() => parseUi(xml.replace(' layoutVersion="2"', ""))).toThrow(
            "属性 widthMode",
        );
        expect(() =>
            parseUi(xml.replace('layoutVersion="2"', 'layoutVersion="3"')),
        ).toThrow("布局格式版本");
    });
    it("rejects invalid sizes and CSS expressions at both XML and runtime boundaries", () => {
        expect(gridTracks("240 auto 1.5fr")).toEqual([
            "240px",
            "auto",
            "1.5fr",
        ]);
        for (const text of [
            "-1",
            "0fr",
            "1e2",
            "1FR",
            "calc(100% - 2px)",
            "1fr;display:none",
            Array(65).fill("auto").join(" "),
        ])
            expect(() => gridTracks(text)).toThrow();
        const { document, ids } = fixture();
        expect(() =>
            validateDocument(
                patchLayouts(document, ids, { minWidth: 300, maxWidth: 200 }),
            ),
        ).toThrow("最小尺寸");
        expect(() =>
            parseUiPayload(
                JSON.stringify({
                    version: 1,
                    kind: "snapshot",
                    component: {
                        className: "Panel",
                        properties: {},
                        children: [],
                        schema: [],
                        layout: { widthMode: "stretch" },
                    },
                }),
            ),
        ).toThrow("布局快照");
    });
    it("aligns and distributes a selection as one undoable change", () => {
        const { document, ids } = fixture();
        const next = arrangeNodes(document, ids, "top");
        expect(next.root.children.map((n) => n.layout.y)).toEqual([20, 20, 20]);
        const history = historyReducer(
            { past: [], present: document, future: [] },
            { type: "edit", document: next },
        );
        expect(history.past).toHaveLength(1);
        expect(historyReducer(history, { type: "undo" }).present).toEqual(
            document,
        );
        const distributed = arrangeNodes(document, ids, "distributeX");
        expect(distributed.root.children.map((n) => n.layout.x)).toEqual([
            10, 130, 250,
        ]);
        expect(
            arrangeNodes(document, ids, "sameWidth").root.children.every(
                (n) => n.layout.widthMode === "fixed",
            ),
        ).toBe(true);
    });
    it("preserves group spacing at boundaries and snaps to sibling edges", () => {
        const { document, ids } = fixture();
        const moved = nudgeNodes(document, ids, -40, 0);
        expect(moved.root.children.map((n) => n.layout.x)).toEqual([
            0, 120, 240,
        ]);
        const snap = snapMove(
            { x: 10, y: 20, width: 80, height: 32 },
            13,
            0,
            [{ x: 100, y: 0, width: 50, height: 70 }],
            6,
        );
        expect(snap.dx).toBe(10);
        expect(snap.guides).toContainEqual({ axis: "x", position: 100 });
        expect(
            snapMove(
                { x: 10, y: 20, width: 80, height: 32 },
                13,
                0,
                [],
                6,
                false,
            ).dx,
        ).toBe(13);
    });
    it("reorders flex children without changing their coordinates", () => {
        const { document, nodes, ids } = fixture("row");
        const next = moveNodes(document, ids.slice(0, 2), document.root.id, {
            index: 3,
        });
        expect(next.root.children.map((n) => n.id)).toEqual([
            ids[2],
            ids[0],
            ids[1],
        ]);
        expect(next.root.children[1]!.layout).toEqual(nodes[0]!.layout);
        expect(() => arrangeNodes(document, ids, "left")).toThrow("绝对布局");
    });
    it("moves a grid group as a block and swaps a single occupied cell", () => {
        const { document, ids } = fixture("grid");
        const next = moveNodes(document, ids.slice(0, 2), document.root.id, {
            row: 1,
            column: 2,
        });
        expect(
            next.root.children
                .filter((n) => ids.slice(0, 2).includes(n.id))
                .map((n) => [n.layout.row, n.layout.column]),
        ).toEqual([
            [1, 2],
            [2, 2],
        ]);
        const swapped = moveNodes(document, [ids[0]!], document.root.id, {
            row: 2,
            column: 1,
        });
        expect(
            swapped.root.children.find((n) => n.id === ids[1])!.layout.row,
        ).toBe(1);
        expect(
            swapped.root.children.find((n) => n.id === ids[0])!.layout.row,
        ).toBe(2);
    });
    it("duplicates selected internal receiver connections and removes dangling bindings", () => {
        const { document, nodes, ids } = fixture();
        nodes[0]!.events.Clicked = `${nodes[1]!.name}.handle`;
        const result = duplicateNodes(
            document,
            ids.slice(0, 2),
            BUILTIN_CATALOG,
        );
        const copies = result.document.root.children.filter((n) =>
            result.ids.includes(n.id),
        );
        expect(copies[0]!.events.Clicked).toBe(`${copies[1]!.name}.handle`);
        expect(nodes[0]!.events.Clicked).toBe(`${nodes[1]!.name}.handle`);
        const deleted = removeNodes(document, [ids[1]!]);
        expect(deleted.root.children[0]!.events).toEqual({});
        validateDocument(deleted);
    });
    it("wraps siblings without duplicating descendants and preserves instance identities", () => {
        const { document, ids } = fixture();
        const result = wrapNodes(
            document,
            ids.slice(0, 2),
            "RowLayout",
            BUILTIN_CATALOG,
        );
        const wrapper = result.document.root.children[0]!;
        expect(wrapper.id).toBe(result.id);
        expect(wrapper.children.map((n) => n.id)).toEqual(ids.slice(0, 2));
        expect(wrapper.layout.width).toBe(200);
        expect(wrapper.layout.x).toBe(10);
        expect(wrapper.children[1]!.layout.x).toBe(120);
        validateDocument(result.document);
        expect(() =>
            moveNodes(result.document, [wrapper.id], wrapper.children[0]!.id),
        ).toThrow("后代");
    });
    it("supports independent sizing axes and bounded grid tracks", () => {
        const { nodes } = fixture();
        const style = sizingStyle(
            {
                ...nodes[0]!.layout,
                widthMode: "fixed",
                heightMode: "fill",
                minWidth: 40,
                maxWidth: 240,
                growY: 2,
            },
            "column",
        );
        expect(style).toMatchObject({
            width: 80,
            minWidth: 40,
            maxWidth: 240,
            flex: "2 1 0px",
            height: "auto",
        });
        expect(
            containerStyle({
                ...nodes[0]!.layout,
                mode: "grid",
                columnTracks: "240 2fr",
                rowTracks: "auto 1fr",
            }),
        ).toMatchObject({
            gridTemplateColumns: "240px 2fr",
            gridTemplateRows: "auto 1fr",
        });
    });
    it("renders a scroll container with an explicit axis and a dedicated icon", () => {
        const node = createNode("ScrollPanel");
        node.properties.ScrollDirection = "vertical";
        const { container } = render(<UiRenderer node={node} running />);
        expect(container.querySelector(".ui-layout")).toHaveStyle({
            overflowX: "hidden",
            overflowY: "auto",
        });
        expect(BUILTIN_CATALOG.ScrollPanel!.icon).toBe("ScrollPanel");
        const mixed = [createNode("Button"), createNode("NumericField")];
        expect(
            commonProperties(mixed, BUILTIN_CATALOG).map((p) => p.name),
        ).toEqual(["Visible", "Enable", "Tooltip"]);
    });
});
