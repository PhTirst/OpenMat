classdef OpenMatEventAccessSource < handle
    events (ListenAccess = private, NotifyAccess = private)
        Pulse
    end

    methods
        function listener = listenInside(object, callback)
            listener = addlistener(object, 'Pulse', callback);
        end

        function fire(object)
            notify(object, 'Pulse');
        end
    end
end
