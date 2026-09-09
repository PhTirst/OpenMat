global openmat_event_log
openmat_event_log = [];
source = OpenMatEventDestroySource();
listener = addlistener(source, 'ObjectBeingDestroyed', @OpenMatEventLogOne);
clear source
before_count = numel(openmat_event_log);
clear listener
after_count = numel(openmat_event_log);
openmat_result = [before_count, after_count, openmat_event_log];
