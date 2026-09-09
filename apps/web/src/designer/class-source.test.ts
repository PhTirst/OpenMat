import { describe, expect, it } from "vitest";
import {
    classTemplate,
    ensureClassMethod,
    inspectClassSource,
    methodOptions,
    syncAppClass,
} from "./class-source";
import { BUILTIN_CATALOG } from "./catalog";
import { createDocument, createNode } from "./model";
import {
    buildEventProgram,
    buildUiProgram,
    componentDescriptor,
    parseUiPayload,
} from "./runtime";
import { parseUi, serializeUi } from "./xml";

function fixture() {
    const document = createDocument("CounterApp");
    const button = createNode("Button", document.root);
    button.name = "Increment";
    button.events.Clicked = "app.onIncrement";
    document.root.children.push(button);
    return { document, button };
}

describe("class-based designer", () => {
    it("keeps array strings and transpose expressions from closing class blocks", () => {
        const { document } = fixture();
        const source = `classdef CounterApp < openmat.ui.AppBase
    methods
        function onClick(app, source, event)
            app.Text = [source.Caption ': ' num2str(source.Value) ' / events: ' num2str(app.Changes)];
            values = [1 2]';
            if true
                values = values';
            end
        end
    end
end
`;
        expect(inspectClassSource(source, "CounterApp").endOffset).toBe(
            source.lastIndexOf("end"),
        );
        const synced = syncAppClass(source, document);
        expect(synced).toContain(
            "        end\n    end\n    % <OpenMat:components>",
        );
        expect(syncAppClass(synced, document)).toBe(synced);
    });
    it("persists instance-method connections and constructs a native app object", () => {
        const { document, button } = fixture();
        const reopened = parseUi(serializeUi(document));
        expect(reopened).toEqual(document);
        const program = buildUiProgram(reopened, BUILTIN_CATALOG);
        expect(program).toContain("app = CounterApp();");
        expect(program).toContain(
            "components.Increment = openmat.ui.Button();",
        );
        expect(program).toContain("app.bindDesignerComponents(components);");
        expect(program).not.toContain("app = struct()");
        const event = buildEventProgram(
            document,
            { id: button.id, name: button.name, event: "Clicked" },
            BUILTIN_CATALOG,
        );
        expect(event).toContain("notify(app.Increment, 'Clicked'");
        expect(event).not.toContain("onIncrement(app,");
    });

    it("adds fields and private handlers without changing user methods or local functions", () => {
        const { document } = fixture();
        const source = `classdef CounterApp < openmat.ui.AppBase
    properties
        Total = 0
        Text = 'end; methods; function fake(obj)'
    end
    methods (Access = private)
        function onIncrement(app, source, event)
            % end
            v = [1, 2]; app.Total = app.Total + v(end);
        end
    end
end
function result = helper()
    result = 'untouched';
end
`;
        const synced = syncAppClass(source, document);
        expect(synced).toContain(
            "app.listen(app.Increment, 'Clicked', @(source, event) app.onIncrement(source, event));",
        );
        expect(synced).toContain("v = [1, 2]; app.Total = app.Total + v(end);");
        expect(synced).toContain(
            "function result = helper()\n    result = 'untouched';\nend",
        );
        expect(syncAppClass(synced, document)).toBe(synced);
        const added = ensureClassMethod(synced, "CounterApp", "onReset");
        expect(
            inspectClassSource(added, "CounterApp").methods.find(
                (m) => m.name === "onReset",
            )?.access,
        ).toBe("private");
        expect(ensureClassMethod(added, "CounterApp", "onReset")).toBe(added);
        expect(
            methodOptions(added, document, document.root, BUILTIN_CATALOG).map(
                (m) => m.value,
            ),
        ).toContain("app.onReset");
        expect(
            methodOptions(added, document, document.root, BUILTIN_CATALOG).map(
                (m) => m.value,
            ),
        ).not.toContain("app.fake");
    });

    it("reflects a Button subclass with its own properties, signal and methods", () => {
        const payload = parseUiPayload(
            JSON.stringify({
                version: 1,
                kind: "describe",
                component: {
                    className: "CounterButton",
                    properties: { Type: "Button" },
                    children: [],
                    schema: [
                        {
                            name: "Step",
                            value: 2,
                            writable: true,
                            className: "double",
                        },
                    ],
                    events: [
                        {
                            name: "Incremented",
                            declaringClass: "CounterButton",
                        },
                    ],
                    methods: [
                        {
                            name: "increment",
                            declaringClass: "CounterButton",
                            access: "public",
                        },
                    ],
                },
            }),
        );
        const descriptor = componentDescriptor(payload, "CounterButton");
        expect(descriptor.renderType).toBe("Button");
        expect(descriptor.container).toBe(false);
        expect(descriptor.icon).toBe("Button");
        expect(descriptor.events).toEqual(["Clicked", "Incremented"]);
        expect(descriptor.methods?.[0]?.name).toBe("increment");
        expect(descriptor.properties[0]?.name).toBe("Step");
    });

    it("fails safely on invalid classes and dangling event targets", () => {
        const { document, button } = fixture();
        expect(() =>
            syncAppClass("function CounterApp(app,event)\nend", document),
        ).toThrow("classdef");
        expect(() => syncAppClass(classTemplate("Other"), document)).toThrow(
            "CounterApp",
        );
        button.events.Clicked = "Missing.onClick";
        expect(() => serializeUi(document)).toThrow("对象.成员方法");
    });

    it("keeps command arguments and compact blocks intact and excludes static handlers", () => {
        const { document, button } = fixture();
        const source = `classdef CounterApp < openmat.ui.AppBase
    methods (Static)
        function result = utility()
            disp end
            if true, result = 1; end
        end
    end
end
`;
        const synced = syncAppClass(source, document);
        expect(synced).toContain(
            "disp end\n            if true, result = 1; end",
        );
        expect(inspectClassSource(synced, "CounterApp").methods[0]?.name).toBe(
            "utility",
        );
        expect(
            methodOptions(synced, document, button, BUILTIN_CATALOG).map(
                (m) => m.value,
            ),
        ).not.toContain("app.utility");
        expect(() =>
            ensureClassMethod(synced, "CounterApp", "utility"),
        ).toThrow("静态方法");
    });
});
