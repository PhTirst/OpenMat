import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
    parameterEditorDocument,
    PARAMETER_EDITOR_CODE,
    PARAMETER_CATALOG,
} from "./component-example";
import { parseUi, serializeUi } from "./xml";
import { classTemplate, syncAppClass } from "./class-source";
import {
    applySnapshot,
    buildEventProgram,
    buildUiProgram,
    parseUiPayload,
} from "./runtime";
import { createDocument, createNode, DEFAULT_LAYOUT } from "./model";
import { UiRenderer } from "./UiRenderer";
import { callbackPath, renamedComponentPath } from "./callbacks";

vi.mock("../components/FigureWindow", () => ({ EmbeddedFigure: () => null }));

describe("reusable component definitions", () => {
    it("keeps component definitions paired with their class across package renames", () => {
        expect(
            renamedComponentPath(
                "MyComponent",
                "widgets.Parameter",
                "MyComponent.omui",
            ),
        ).toBe("+widgets/Parameter.omui");
        expect(
            callbackPath("widgets.Parameter", "+widgets/Parameter.omui"),
        ).toBe("+widgets/Parameter.m");
        expect(
            renamedComponentPath(
                "widgets.Parameter",
                "controls.Value",
                "views/+widgets/Parameter.omui",
            ),
        ).toBe("views/+controls/Value.omui");
        expect(
            renamedComponentPath("Parameter", "Other", "CustomFilename.omui"),
        ).toBe("CustomFilename.omui");
    });

    it("rejects invalid runtime layout before rendering it", () => {
        for (const layout of [
            { columns: 1.5 },
            { row: 0 },
            { width: 0 },
            { extra: 2 },
            { constructor: 2 },
        ]) {
            expect(() =>
                parseUiPayload(
                    JSON.stringify({
                        version: 1,
                        kind: "snapshot",
                        component: {
                            className: "openmat.ui.Panel",
                            properties: {},
                            schema: [],
                            children: [],
                            layout,
                        },
                    }),
                ),
            ).toThrow("布局快照");
        }
    });
    it("round trips an editable component and generates each instance's private wiring", () => {
        const document = parameterEditorDocument();
        const xml = serializeUi(document);
        expect(xml).toContain(
            'version="3" kind="component" class="ParameterEditor"',
        );
        expect(parseUi(xml)).toEqual(document);
        expect(PARAMETER_EDITOR_CODE).toContain(
            "function app = ParameterEditor()",
        );
        expect(PARAMETER_EDITOR_CODE).toContain(
            "function buildDesignerComponents(app)",
        );
        expect(PARAMETER_EDITOR_CODE).toContain(
            "components.Editor = openmat.ui.NumericField();",
        );
        expect(PARAMETER_EDITOR_CODE).toContain(
            "app.listen(app.Editor, 'ValueChanged'",
        );
        expect(PARAMETER_EDITOR_CODE).not.toContain(".Id =");
        expect(syncAppClass(PARAMETER_EDITOR_CODE, document)).toBe(
            PARAMETER_EDITOR_CODE,
        );
        expect(buildUiProgram(document, PARAMETER_CATALOG)).not.toContain(
            "components.Editor =",
        );
        expect(() =>
            syncAppClass(classTemplate("Other", true), document),
        ).toThrow();
    });

    it("preserves runtime layout and object identity across sibling removal", () => {
        const document = createDocument("Demo");
        const runtime = (id: string, value: number) => ({
            className: "openmat.ui.NumericField",
            properties: { Type: "NumericField", RuntimeId: id, Value: value },
            schema: [],
            children: [],
            layout: { mode: "column", column: 2 },
        });
        const snapshot = (children: unknown[]) =>
            parseUiPayload(
                JSON.stringify({
                    version: 1,
                    kind: "snapshot",
                    component: {
                        className: "Demo",
                        properties: {
                            Id: document.root.id,
                            Type: "Window",
                            RuntimeId: "ui-root",
                        },
                        schema: [],
                        layout: { mode: "grid", columns: 2 },
                        children,
                    },
                }),
            ).component;
        const first = applySnapshot(
            document,
            snapshot([runtime("ui-a", 1), runtime("ui-b", 2)]),
        );
        const second = applySnapshot(document, snapshot([runtime("ui-b", 2)]));
        expect(first.root.children[1]?.id).toBe(second.root.children[0]?.id);
        expect(second.root.children[0]?.layout.column).toBe(2);
        const event = {
            id: "runtime-ui-b",
            name: "Internal",
            runtimeId: "ui-b",
            event: "ValueChanged",
            property: "Value",
            value: 3,
        };
        const code = buildEventProgram(
            document,
            event,
            PARAMETER_CATALOG,
            second.root,
        );
        expect(code).toContain("findRuntime('ui-b')");
        expect(code).not.toContain("Children{");
        expect(
            buildEventProgram(
                document,
                { ...event, id: "runtime-ui-a", runtimeId: "ui-a" },
                PARAMETER_CATALOG,
                second.root,
            ),
        ).toBe("");
    });

    it("renders native absolute Position and lets grid constraints take precedence", () => {
        const node = createNode("Button");
        node.properties = {
            Text: "Position probe",
            Position: [[11, 22, 130, 40]],
        };
        node.layout = { ...DEFAULT_LAYOUT, row: 2, column: 3 };
        const { container, rerender } = render(
            <UiRenderer node={node} running parentMode="absolute" />,
        );
        expect(
            screen.getByRole("button", { name: "Position probe" }),
        ).toBeInTheDocument();
        expect(container.firstChild).toHaveStyle({
            left: "11px",
            bottom: "22px",
            width: "130px",
            height: "40px",
        });
        rerender(<UiRenderer node={node} running parentMode="grid" />);
        expect(container.firstChild).toHaveStyle({
            gridColumn: "3 / span 1",
            gridRow: "2 / span 1",
        });
        expect(container.firstChild).not.toHaveStyle({ bottom: "22px" });
    });
});
