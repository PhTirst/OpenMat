classdef OpenMatEventDestroySource < handle
    methods
        function delete(object)
            global openmat_event_log
            openmat_event_log = [openmat_event_log, 2];
        end
    end
end
