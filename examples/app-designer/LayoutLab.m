classdef LayoutLab < openmat.ui.AppBase
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
    % <OpenMat:components>
    properties (SetAccess = private)
        MainWindow
        Heading
        Parameters
        ParameterTitle
        FrequencyLabel
        Frequency
        AmplitudeLabel
        Amplitude
        PhaseLabel
        Phase
        DurationLabel
        Duration
        SamplesLabel
        Samples
        ShowGrid
        Run
        Results
        Waveform
        LogPanel
        Log
    end
    methods
        function bindDesignerComponents(app, components)
            app.MainWindow = components.MainWindow;
            app.Heading = components.Heading;
            app.Parameters = components.Parameters;
            app.ParameterTitle = components.ParameterTitle;
            app.FrequencyLabel = components.FrequencyLabel;
            app.Frequency = components.Frequency;
            app.AmplitudeLabel = components.AmplitudeLabel;
            app.Amplitude = components.Amplitude;
            app.PhaseLabel = components.PhaseLabel;
            app.Phase = components.Phase;
            app.DurationLabel = components.DurationLabel;
            app.Duration = components.Duration;
            app.SamplesLabel = components.SamplesLabel;
            app.Samples = components.Samples;
            app.ShowGrid = components.ShowGrid;
            app.Run = components.Run;
            app.Results = components.Results;
            app.Waveform = components.Waveform;
            app.LogPanel = components.LogPanel;
            app.Log = components.Log;
            app.listen(app.MainWindow, 'Startup', @(source, event) app.onStartup(source, event));
            app.listen(app.Run, 'Clicked', @(source, event) app.onRun(source, event));
        end
    end
    % </OpenMat:components>
end
