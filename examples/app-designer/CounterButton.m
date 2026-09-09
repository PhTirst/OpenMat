classdef CounterButton < openmat.ui.Button
    properties
        Step = 1
        Count = 0
    end
    events (NotifyAccess = private)
        Incremented
    end
    methods (Access = protected)
        function setup(obj)
            obj.Text = ['Count: ' num2str(obj.Count)];
            addlistener(obj, 'Clicked', @(source,event) obj.increment());
        end
    end
    methods
        function reset(obj, source, event)
            obj.Count = 0;
            obj.Text = 'Count: 0';
            notify(obj, 'Incremented');
        end
    end
    methods (Access = private)
        function increment(obj)
            obj.Count = obj.Count + obj.Step;
            obj.Text = ['Count: ' num2str(obj.Count)];
            notify(obj, 'Incremented');
        end
    end
end
