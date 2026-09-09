import {
    createDocument,
    createNode,
    type UiLayout,
    type UiNode,
} from "./model";
import type { UiValue } from "./catalog";
import { syncAppClass } from "./class-source";

export function layoutExample() {
    const document = createDocument("LayoutLab"),
        root = document.root;
    root.properties = {
        Title: "信号实验室 · 可调整布局",
        Width: 1100,
        Height: 720,
    };
    root.layout = {
        ...root.layout,
        mode: "grid",
        columns: 2,
        columnTracks: "260 1fr",
        rowTracks: "auto 1fr 150",
        padding: 20,
        gap: 16,
    };
    const add = (
        parent: UiNode,
        type: string,
        name: string,
        properties: Record<string, UiValue> = {},
        layout: Partial<UiLayout> = {},
    ) => {
        const node = createNode(type, root);
        node.name = name;
        node.properties = properties;
        node.layout = {
            ...node.layout,
            widthMode: "fill",
            heightMode: "fixed",
            height: 36,
            ...layout,
        };
        parent.children.push(node);
        return node;
    };
    add(
        root,
        "Label",
        "Heading",
        { Text: "信号实验室", FontSize: 22 },
        { columnSpan: 2, heightMode: "content" },
    );
    const parameters = add(
        root,
        "ScrollPanel",
        "Parameters",
        { ScrollDirection: "vertical" },
        { row: 2, heightMode: "fill", minHeight: 80, padding: 12 },
    );
    add(
        parameters,
        "Label",
        "ParameterTitle",
        { Text: "参数设置", FontSize: 16 },
        { heightMode: "content" },
    );
    for (const [name, caption, value, min, max] of [
        ["Frequency", "频率 / Hz", 2, 0.1, 20],
        ["Amplitude", "幅度", 1, 0.1, 5],
        ["Phase", "相位 / rad", 0, 0, 6.28],
        ["Duration", "时长 / s", 2, 0.1, 10],
        ["Samples", "采样点数", 400, 20, 2000],
    ] as const) {
        add(
            parameters,
            "Label",
            `${name}Label`,
            { Text: caption },
            { heightMode: "content" },
        );
        add(parameters, "NumericField", name, {
            Value: value,
            Min: min,
            Max: max,
            Step: name === "Samples" ? 1 : 0.1,
        });
    }
    add(parameters, "CheckBox", "ShowGrid", { Text: "显示网格", Value: true });
    add(parameters, "Button", "Run", {
        Text: "计算并绘图",
        Variant: "primary",
    }).events.Clicked = "app.onRun";
    const plot = add(
        root,
        "Panel",
        "Results",
        { Title: "时域波形" },
        {
            row: 2,
            column: 2,
            heightMode: "fill",
            minWidth: 160,
            minHeight: 100,
        },
    );
    add(
        plot,
        "PlotView",
        "Waveform",
        { FigureIndex: 1 },
        { heightMode: "fill", minHeight: 60, growY: 1 },
    );
    const logs = add(
        root,
        "Panel",
        "LogPanel",
        { Title: "运行日志" },
        { row: 3, columnSpan: 2, heightMode: "fill" },
    );
    add(
        logs,
        "TextArea",
        "Log",
        { Value: "等待运行", Enable: false },
        { heightMode: "fill", minHeight: 30 },
    );
    return document;
}
export const LAYOUT_LAB_CODE = syncAppClass(
    `classdef LayoutLab < openmat.ui.AppBase
    properties
        Runs = 0
    end
    methods (Access = private)
        function onStartup(app, source, event)
            app.onRun(source, event);
        end
        function onRun(app, source, event)
            app.Runs = app.Runs + 1;
            t = linspace(0, app.Duration.Value, round(app.Samples.Value));
            y = app.Amplitude.Value * sin(2 * pi * app.Frequency.Value * t + app.Phase.Value);
            figure(1);
            plot(t, y);
            title('Signal');
            xlabel('Time / s');
            ylabel('Amplitude');
            if app.ShowGrid.Value
                grid on;
            else
                grid off;
            end
            app.Log.Value = ['Run #' num2str(app.Runs) char(10) 'Frequency: ' num2str(app.Frequency.Value) ' Hz; samples: ' num2str(numel(t)) char(10) 'Calculation complete.'];
        end
    end
end
`,
    layoutExample(),
);
