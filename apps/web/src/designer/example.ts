import { createDocument, createNode, type UiNode } from "./model";
import type { UiValue } from "./catalog";
import { syncAppClass } from "./class-source";

export function signalExample() {
    const document = createDocument("SignalApp");
    const root = document.root;
    root.properties = { Title: "信号分析工具", Width: 880, Height: 560 };
    root.layout.padding = 20;
    root.events.Startup = "app.onStartup";
    const add = (
        parent: UiNode,
        type: string,
        name: string,
        properties: Record<string, UiValue> = {},
    ) => {
        const node = createNode(type, root);
        node.name = name;
        node.properties = properties;
        parent.children.push(node);
        return node;
    };
    const heading = add(root, "Label", "Heading", {
        Text: "信号分析",
        FontSize: 21,
    });
    heading.layout.height = 38;
    const row = add(root, "RowLayout", "Content");
    row.layout.padding = 0;
    row.layout.gap = 20;
    const params = add(row, "Panel", "Parameters", { Title: "参数设置" });
    params.layout.grow = 0;
    params.layout.width = 230;
    add(params, "Label", "FrequencyLabel", { Text: "频率 / Hz" });
    add(params, "NumericField", "Frequency", {
        Value: 2,
        Min: 0.1,
        Max: 20,
        Step: 0.1,
    });
    add(params, "Label", "AmplitudeLabel", { Text: "幅度" });
    add(params, "Slider", "Amplitude", { Value: 1, Min: 0, Max: 5, Step: 0.1 });
    add(params, "CheckBox", "ShowGrid", { Text: "显示网格", Value: true });
    add(params, "Button", "RunButton", {
        Text: "运行分析",
        Variant: "primary",
    }).events.Clicked = "app.onRun";
    const results = add(row, "Panel", "Results", { Title: "时域波形" });
    const plot = add(results, "PlotView", "Waveform", {
        Title: "正弦信号",
        FigureIndex: 1,
    });
    plot.layout.grow = 1;
    const status = add(root, "RowLayout", "Footer");
    status.layout.grow = 0;
    status.layout.height = 36;
    status.layout.padding = 0;
    add(status, "StatusLamp", "Status", { Text: "就绪", Color: "#247951" });
    const progress = add(status, "ProgressBar", "Progress", { Value: 0 });
    progress.layout.grow = 1;
    return document;
}
export const SIGNAL_CODE = syncAppClass(
    `classdef SignalApp < openmat.ui.AppBase
    properties
        RunCount = 0
    end
    methods (Access = private)
        function onStartup(app, source, event)
            app.onRun(source, event);
        end
        function onRun(app, source, event)
            app.RunCount = app.RunCount + 1;
            app.Status.Text = '计算中';
            app.Progress.Value = 30;
            t = linspace(0, 2, 400);
            y = app.Amplitude.Value * sin(2 * pi * app.Frequency.Value * t);
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
            app.Progress.Value = 100;
            app.Status.Text = '完成';
        end
    end
end
`,
    signalExample(),
);
