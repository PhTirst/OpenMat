classdef SignalApp < openmat.ui.AppBase
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
    % <OpenMat:components>
    properties (SetAccess = private)
        MainWindow
        Heading
        Content
        Parameters
        FrequencyLabel
        Frequency
        AmplitudeLabel
        Amplitude
        ShowGrid
        RunButton
        Results
        Waveform
        Footer
        Status
        Progress
    end
    methods
        function bindDesignerComponents(app, components)
            app.MainWindow = components.MainWindow;
            app.Heading = components.Heading;
            app.Content = components.Content;
            app.Parameters = components.Parameters;
            app.FrequencyLabel = components.FrequencyLabel;
            app.Frequency = components.Frequency;
            app.AmplitudeLabel = components.AmplitudeLabel;
            app.Amplitude = components.Amplitude;
            app.ShowGrid = components.ShowGrid;
            app.RunButton = components.RunButton;
            app.Results = components.Results;
            app.Waveform = components.Waveform;
            app.Footer = components.Footer;
            app.Status = components.Status;
            app.Progress = components.Progress;
            app.listen(app.MainWindow, 'Startup', @(source, event) app.onStartup(source, event));
            app.listen(app.RunButton, 'Clicked', @(source, event) app.onRun(source, event));
        end
    end
    % </OpenMat:components>
end
