import { BUILTIN_CATALOG, type ComponentSpec } from "./catalog";
import {
    createComponentDocument,
    createDocument,
    createNode,
    type UiNode,
} from "./model";
import { syncAppClass } from "./class-source";

export function parameterEditorDocument() {
    const document = createComponentDocument("ParameterEditor");
    document.root.layout = {
        ...document.root.layout,
        mode: "grid",
        columns: 3,
        width: 480,
        height: 88,
        padding: 12,
        gap: 12,
    };
    for (const [index, type, name] of [
        [1, "Label", "CaptionLabel"],
        [2, "NumericField", "Editor"],
        [3, "Slider", "Dial"],
    ] as const) {
        const child = createNode(type, document.root);
        child.name = name;
        child.layout.column = index;
        if (type !== "Label") child.events.ValueChanged = "app.onInput";
        document.root.children.push(child);
    }
    return document;
}

export const PARAMETER_EDITOR_CODE = syncAppClass(
    `classdef ParameterEditor < openmat.ui.Panel
    properties
        Caption = 'Parameter'
        Value = 0
        Min = 0
        Max = 100
    end
    events (NotifyAccess = private)
        ValueChanged
    end
    methods (Access = protected)
        function update(obj)
            obj.CaptionLabel.Text = obj.Caption;
            obj.Editor.Min = obj.Min;
            obj.Editor.Max = obj.Max;
            obj.Editor.Value = obj.Value;
            obj.Dial.Min = obj.Min;
            obj.Dial.Max = obj.Max;
            obj.Dial.Value = obj.Value;
        end
    end
    methods (Access = private)
        function onInput(obj, source, event)
            obj.Value = min(obj.Max, max(obj.Min, source.Value));
            obj.refresh();
            notify(obj, 'ValueChanged', event);
        end
    end
end
`,
    parameterEditorDocument(),
);

const parameterSpec: ComponentSpec = {
    ...BUILTIN_CATALOG.Panel!,
    type: "ParameterEditor",
    label: "ParameterEditor",
    className: "ParameterEditor",
    composite: true,
    defaultLayout: parameterEditorDocument().root.layout,
    properties: [
        ...BUILTIN_CATALOG.Panel!.properties,
        {
            name: "Caption",
            label: "Caption",
            type: "text",
            default: "Parameter",
        },
        ...["Value", "Min", "Max"].map((name) => ({
            name,
            label: name,
            type: "number" as const,
            default: name === "Max" ? 100 : 0,
        })),
    ],
    events: ["ValueChanged"],
};
export const PARAMETER_CATALOG = {
    ...BUILTIN_CATALOG,
    ParameterEditor: parameterSpec,
};
export function parameterAppDocument() {
    const document = createDocument("ParameterApp");
    const root = document.root;
    root.properties = { Title: "复合组件实例", Width: 1040, Height: 400 };
    function add(parent: UiNode, type: string, name: string, properties = {}) {
        const child = createNode(
            type,
            root,
            PARAMETER_CATALOG[type as keyof typeof PARAMETER_CATALOG],
        );
        child.name = name;
        child.properties = properties;
        parent.children.push(child);
        return child;
    }
    add(root, "Label", "Heading", {
        Text: "独立实例 · 共享组件定义",
        FontSize: 20,
    });
    const editors = add(root, "GridLayout", "Editors");
    editors.layout.columns = 2;
    editors.layout.padding = 0;
    const first = add(editors, "ParameterEditor", "First", {
        Caption: "频率",
        Value: 10,
        Min: 0,
        Max: 100,
    });
    const second = add(editors, "ParameterEditor", "Second", {
        Caption: "幅度",
        Value: 25,
        Min: 0,
        Max: 50,
    });
    second.layout.column = 2;
    first.events.ValueChanged = second.events.ValueChanged = "app.onParameter";
    const tools = add(root, "RowLayout", "Actions");
    tools.layout.height = 44;
    tools.layout.grow = 0;
    tools.layout.padding = 0;
    for (const [name, text, method] of [
        ["Add", "添加动态实例", "onAdd"],
        ["Remove", "删除动态实例", "onRemove"],
        ["Resize", "切换窗口宽度", "onResize"],
    ]) {
        add(tools, "Button", name!, { Text: text }).events.Clicked =
            `app.${method}`;
    }
    add(root, "Label", "Status", { Text: "就绪" });
    return document;
}
export const PARAMETER_APP_CODE = syncAppClass(
    `classdef ParameterApp < openmat.ui.AppBase
    properties
        Changes = 0
    end
    properties (Access = private)
        Dynamic = []
    end
    methods (Access = private)
        function onStartup(app, source, event)
            app.Status.Text = '两个实例已就绪';
        end
        function onParameter(app, source, event)
            app.Changes = app.Changes + 1;
            app.Status.Text = [source.Caption ': ' num2str(source.Value) ' / events: ' num2str(app.Changes)];
        end
        function onAdd(app, source, event)
            if isempty(app.Dynamic) || ~isvalid(app.Dynamic)
                app.Dynamic = ParameterEditor();
                app.Dynamic.Caption = '动态参数';
                app.Dynamic.Value = 40;
                app.Dynamic.Layout.Row = 3;
                app.Dynamic.Layout.ColumnSpan = app.Editors.Layout.Columns;
                app.listen(app.Dynamic, 'ValueChanged', @(source,event) app.onParameter(source,event));
                app.Editors.add(app.Dynamic);
                app.Status.Text = '动态实例已创建';
            end
        end
        function onRemove(app, source, event)
            if ~isempty(app.Dynamic) && isvalid(app.Dynamic)
                delete(app.Dynamic);
                app.Dynamic = [];
                app.Status.Text = '动态实例已删除';
            end
        end
        function onResize(app, source, event)
            if app.Width > 800
                app.Width = 700;
                app.Editors.Layout.Columns = 1;
                app.Second.Layout.Row = 2;
                app.Second.Layout.Column = 1;
            else
                app.Width = 1040;
                app.Editors.Layout.Columns = 2;
                app.Second.Layout.Row = 1;
                app.Second.Layout.Column = 2;
            end
            if ~isempty(app.Dynamic) && isvalid(app.Dynamic)
                app.Dynamic.Layout.ColumnSpan = app.Editors.Layout.Columns;
            end
        end
    end
end
`,
    parameterAppDocument(),
    PARAMETER_CATALOG,
);
