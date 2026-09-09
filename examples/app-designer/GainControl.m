classdef GainControl < matlab.ui.componentcontainer.ComponentContainer
    properties
        Value = 1
        Caption = 'Gain'
        Active = true
    end
    properties (Access = private)
        Readout
        IncrementButton
    end
    methods (Access = protected)
        function setup(obj)
            obj.Readout = openmat.ui.Control('Label');
            obj.add(obj.Readout);
            obj.IncrementButton = openmat.ui.Control('Button');
            obj.IncrementButton.Text = '+1';
            obj.IncrementButton.ButtonPushedFcn = @(source, event) obj.increment();
            obj.add(obj.IncrementButton);
        end
        function update(obj)
            obj.Readout.Text = [obj.Caption ': ' num2str(obj.Value)];
            obj.Readout.Visible = obj.Active;
            obj.IncrementButton.Enable = obj.Active;
        end
        function increment(obj)
            obj.Value = obj.Value + 1;
        end
    end
end
