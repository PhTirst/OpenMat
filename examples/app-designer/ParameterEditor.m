classdef ParameterEditor < openmat.ui.Panel
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
    % <OpenMat:components>
    properties (SetAccess = private)
        Root
        CaptionLabel
        Editor
        Dial
    end
    methods
        function app = ParameterEditor()
            app.applyDesignerDefaults();
        end
    end
    methods (Access = protected)
        function applyDesignerDefaults(app)
            app.Title = '';
            app.Layout.Mode = 'grid';
            app.Layout.X = 16;
            app.Layout.Y = 16;
            app.Layout.Width = 480;
            app.Layout.Height = 88;
            app.Layout.Row = 1;
            app.Layout.Column = 1;
            app.Layout.RowSpan = 1;
            app.Layout.ColumnSpan = 1;
            app.Layout.Columns = 3;
            app.Layout.Gap = 12;
            app.Layout.Padding = 12;
            app.Layout.Grow = 0;
        end
        function buildDesignerComponents(app)
            components = struct();
            components.Root = app;
            components.CaptionLabel = openmat.ui.Label();
            components.CaptionLabel.Name = 'CaptionLabel';
            components.CaptionLabel.Text = '标签';
            components.CaptionLabel.FontSize = 13;
            components.CaptionLabel.Visible = true;
            components.CaptionLabel.Enable = true;
            components.CaptionLabel.Tooltip = '';
            components.CaptionLabel.Layout.Mode = 'column';
            components.CaptionLabel.Layout.X = 16;
            components.CaptionLabel.Layout.Y = 16;
            components.CaptionLabel.Layout.Width = 180;
            components.CaptionLabel.Layout.Height = 36;
            components.CaptionLabel.Layout.Row = 1;
            components.CaptionLabel.Layout.Column = 1;
            components.CaptionLabel.Layout.RowSpan = 1;
            components.CaptionLabel.Layout.ColumnSpan = 1;
            components.CaptionLabel.Layout.Columns = 2;
            components.CaptionLabel.Layout.Gap = 12;
            components.CaptionLabel.Layout.Padding = 12;
            components.CaptionLabel.Layout.Grow = 0;
            components.Editor = openmat.ui.NumericField();
            components.Editor.Name = 'Editor';
            components.Editor.Value = 1;
            components.Editor.Min = 0;
            components.Editor.Max = 100;
            components.Editor.Step = 1;
            components.Editor.Visible = true;
            components.Editor.Enable = true;
            components.Editor.Tooltip = '';
            components.Editor.Layout.Mode = 'column';
            components.Editor.Layout.X = 16;
            components.Editor.Layout.Y = 16;
            components.Editor.Layout.Width = 180;
            components.Editor.Layout.Height = 36;
            components.Editor.Layout.Row = 1;
            components.Editor.Layout.Column = 2;
            components.Editor.Layout.RowSpan = 1;
            components.Editor.Layout.ColumnSpan = 1;
            components.Editor.Layout.Columns = 2;
            components.Editor.Layout.Gap = 12;
            components.Editor.Layout.Padding = 12;
            components.Editor.Layout.Grow = 0;
            components.Dial = openmat.ui.Slider();
            components.Dial.Name = 'Dial';
            components.Dial.Value = 50;
            components.Dial.Min = 0;
            components.Dial.Max = 100;
            components.Dial.Step = 1;
            components.Dial.Visible = true;
            components.Dial.Enable = true;
            components.Dial.Tooltip = '';
            components.Dial.Layout.Mode = 'column';
            components.Dial.Layout.X = 16;
            components.Dial.Layout.Y = 16;
            components.Dial.Layout.Width = 180;
            components.Dial.Layout.Height = 36;
            components.Dial.Layout.Row = 1;
            components.Dial.Layout.Column = 3;
            components.Dial.Layout.RowSpan = 1;
            components.Dial.Layout.ColumnSpan = 1;
            components.Dial.Layout.Columns = 2;
            components.Dial.Layout.Gap = 12;
            components.Dial.Layout.Padding = 12;
            components.Dial.Layout.Grow = 0;
            components.Root.add(components.CaptionLabel);
            components.Root.add(components.Editor);
            components.Root.add(components.Dial);
            app.Root = components.Root;
            app.CaptionLabel = components.CaptionLabel;
            app.Editor = components.Editor;
            app.Dial = components.Dial;
            app.listen(app.Editor, 'ValueChanged', @(source, event) app.onInput(source, event));
            app.listen(app.Dial, 'ValueChanged', @(source, event) app.onInput(source, event));
        end
    end
    % </OpenMat:components>
end
