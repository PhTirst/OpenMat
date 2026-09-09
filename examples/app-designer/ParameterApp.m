classdef ParameterApp < openmat.ui.AppBase
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
    % <OpenMat:components>
    properties (SetAccess = private)
        MainWindow
        Heading
        Editors
        First
        Second
        Actions
        Add
        Remove
        Resize
        Status
    end
    methods
        function bindDesignerComponents(app, components)
            app.MainWindow = components.MainWindow;
            app.Heading = components.Heading;
            app.Editors = components.Editors;
            app.First = components.First;
            app.Second = components.Second;
            app.Actions = components.Actions;
            app.Add = components.Add;
            app.Remove = components.Remove;
            app.Resize = components.Resize;
            app.Status = components.Status;
            app.listen(app.MainWindow, 'Startup', @(source, event) app.onStartup(source, event));
            app.listen(app.First, 'ValueChanged', @(source, event) app.onParameter(source, event));
            app.listen(app.Second, 'ValueChanged', @(source, event) app.onParameter(source, event));
            app.listen(app.Add, 'Clicked', @(source, event) app.onAdd(source, event));
            app.listen(app.Remove, 'Clicked', @(source, event) app.onRemove(source, event));
            app.listen(app.Resize, 'Clicked', @(source, event) app.onResize(source, event));
        end
    end
    % </OpenMat:components>
end
