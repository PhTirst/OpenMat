import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { BUILTIN_COMPONENTS } from "./catalog";
import { ComponentIcon, COMPONENT_ICON_TYPES } from "./ComponentIcon";
import {
    createDocument,
    createNode,
    cloneNode,
    historyReducer,
    moveNode,
    validateDocument,
} from "./model";
import { parseUi, serializeUi } from "./xml";
import { signalExample } from "./example";
import {
    buildEventProgram,
    buildUiProgram,
    matlabValue,
    matlabPropertyValue,
    componentDescriptor,
    applySnapshot,
    parseUiPayload,
} from "./runtime";
import { BUILTIN_CATALOG } from "./catalog";

describe("UI XML document", () => {
    it("round trips nested layouts, Chinese text, arrays and callback references", () => {
        const document = signalExample();
        document.root.properties.Title = '测试 & <one> "two"\n第二行';
        const table = createNode("Table", document.root);
        table.properties = {
            Data: [
                ["a&b", 1, true],
                ["<x>", 2, false],
            ],
            Columns: ["名字", "值", "启用"],
        };
        table.events.CellEdited = "app.onEdit";
        document.root.children.push(table);
        const encoded = serializeUi(document);
        expect(encoded).toContain("&amp;");
        expect(encoded).toContain("<Layout");
        expect(parseUi(encoded)).toEqual(document);
        expect(serializeUi(parseUi(encoded))).toBe(encoded);
    });
    it("rejects XML entities, unknown schemas, invalid numbers and duplicate IDs", () => {
        const xml = serializeUi(createDocument());
        expect(() =>
            parseUi(
                '<!DOCTYPE a [<!ENTITY x SYSTEM "file:///etc/passwd">]>' + xml,
            ),
        ).toThrow("DTD");
        expect(() =>
            parseUi(
                xml.replace('version="1" controller', 'version="4" controller'),
            ),
        ).toThrow("version=1");
        expect(() =>
            parseUi(xml.replace("<Layout ", '<Layout unsupported="x" ')),
        ).toThrow("不支持");
        expect(() =>
            parseUi(xml.replace('width="180"', 'width="NaN"')),
        ).toThrow("布局数值");
        const document = createDocument();
        const child = createNode("Button", document.root);
        child.id = document.root.id;
        document.root.children.push(child);
        expect(() => validateDocument(document)).toThrow("重复");
    });
    it("preserves unloaded component definitions and refuses unknown XML elements", () => {
        const document = createDocument();
        const external = createNode("ComponentContainer", document.root);
        external.type = "GainControl";
        external.properties.Gain = 2;
        external.children.push(createNode("Label", document.root));
        document.root.children.push(external);
        expect(parseUi(serializeUi(document))).toEqual(document);
        expect(() =>
            parseUi(
                serializeUi(document).replace(
                    "</OpenMatUI>",
                    "<FutureFeature/></OpenMatUI>",
                ),
            ),
        ).toThrow();
    });
    it("keeps stable IDs through moves and undo, without cycles or shared history mutation", () => {
        const document = createDocument();
        const panel = createNode("Panel", document.root);
        const button = createNode("Button", document.root);
        document.root.children.push(panel, button);
        const moved = moveNode(document, button.id, panel.id);
        expect(document.root.children).toHaveLength(2);
        expect(moved.root.children[0]?.children[0]?.id).toBe(button.id);
        expect(() => moveNode(moved, panel.id, button.id)).toThrow();
        const history = historyReducer(
            { past: [], present: document, future: [] },
            { type: "edit", document: moved },
        );
        const undone = historyReducer(history, { type: "undo" });
        expect(undone.present).toBe(document);
        expect(historyReducer(undone, { type: "redo" }).present).toBe(moved);
        const copy = cloneNode(panel, document.root);
        expect(copy.id).not.toBe(panel.id);
        expect(copy.name).not.toBe(panel.name);
    });
    it("rejects invalid property shapes before rendering", () => {
        const document = createDocument();
        const child = createNode("Slider");
        child.properties.Value = "bad";
        document.root.children.push(child);
        expect(() => serializeUi(document)).toThrow("属性值无效");
    });
});
describe("component icons", () => {
    it("gives every built-in component distinct SVG geometry in every category", () => {
        expect(new Set(COMPONENT_ICON_TYPES)).toEqual(
            new Set(BUILTIN_COMPONENTS.map((c) => c.type)),
        );
        const geometries = BUILTIN_COMPONENTS.map((c) =>
            renderToStaticMarkup(
                createElement(ComponentIcon, { type: c.type }),
            ).replace(/data-component-icon="[^"]+"/g, ""),
        );
        expect(new Set(geometries).size).toBe(BUILTIN_COMPONENTS.length);
    });
});
describe("native UI programs", () => {
    it("reflects native numeric matrices and preserves their type in generated source", () => {
        const descriptor = componentDescriptor(
            {
                version: 1,
                kind: "describe",
                component: {
                    className: "GainControl",
                    properties: {},
                    children: [],
                    schema: [
                        {
                            name: "Weights",
                            value: [
                                [1, 2],
                                [3, 4],
                            ],
                            className: "double",
                            writable: true,
                        },
                        {
                            name: "Caption",
                            value: "Gain",
                            className: "string",
                            writable: true,
                        },
                        {
                            name: "ReadOnly",
                            value: 2,
                            className: "double",
                            writable: false,
                        },
                    ],
                },
            },
            "GainControl",
        );
        expect(
            matlabPropertyValue(descriptor.properties[0]!, [
                [5, 6],
                [7, 8],
            ]),
        ).toBe("[5 6;7 8]");
        expect(matlabPropertyValue(descriptor.properties[1]!, "x'y")).toBe(
            "string('x''y')",
        );
        expect(descriptor.properties[2]?.readonly).toBe(true);
    });
    it("routes source-created child callbacks through validated runtime paths without changing XML", () => {
        const document = createDocument();
        const before = serializeUi(document);
        const root = applySnapshot(document, {
            className: "openmat.ui.Control",
            properties: { Id: document.root.id, Type: "Window" },
            schema: [],
            children: [
                {
                    className: "openmat.ui.Control",
                    properties: { Type: "Button", Text: "+1" },
                    schema: [],
                    children: [],
                },
            ],
        }).root;
        const child = root.children[0]!;
        expect(child.sourceOwned).toBe(true);
        expect(
            buildEventProgram(
                document,
                { id: child.id, name: child.name, event: "Clicked" },
                BUILTIN_CATALOG,
                root,
            ),
        ).toContain(`app.${document.root.name}.Children{1}.ButtonPushedFcn`);
        expect(serializeUi(document)).toBe(before);
        child.runtimePath = [-1];
        expect(() =>
            buildEventProgram(
                document,
                { id: child.id, name: child.name, event: "Clicked" },
                BUILTIN_CATALOG,
                root,
            ),
        ).toThrow("路径无效");
    });
    it("uses native handle controls, quoted values, public overrides before setup and isolated app state", () => {
        const document = signalExample();
        const source = buildUiProgram(document, BUILTIN_CATALOG);
        expect(source).toContain(
            "components.Frequency = openmat.ui.NumericField();",
        );
        expect(source.indexOf("components.Frequency.Value = 2;")).toBeLessThan(
            source.indexOf("app.initialize();"),
        );
        expect(source).toContain("notify(app, 'Startup');");
        expect(source).toContain("openmat_ui('snapshot', app)");
        expect(matlabValue("a'; error('injected')\n")).toMatch(/^char\(\[/);
        expect(matlabValue("a'b")).toBe("'a''b'");
        const node = document.root.children[1]!.children[0]!.children[1]!;
        expect(
            buildEventProgram(
                document,
                {
                    id: node.id,
                    name: node.name,
                    event: "ValueChanged",
                    property: "Value",
                    value: 4,
                },
                BUILTIN_CATALOG,
            ),
        ).toContain("app.Frequency.Value = 4;");
    });
    it("rejects incompatible UI message versions and malicious property names", () => {
        expect(() => parseUiPayload('{"version":2}')).toThrow();
        expect(() =>
            parseUiPayload(
                '{"version":1,"kind":"snapshot","component":{"className":"x","properties":{"constructor":1},"schema":[],"children":[]}}',
            ),
        ).toThrow();
    });
});
