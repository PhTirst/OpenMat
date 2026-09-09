classdef OpenMatEventSource < handle
    events
        Pulse
    end

    methods
        function fire(object)
            notify(object, 'Pulse');
        end
    end
end
