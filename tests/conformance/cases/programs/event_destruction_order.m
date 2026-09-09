global openmat_event_log
openmat_event_log = [];
source = OpenMatEventDestroySource();
listener = addlistener(source, 'ObjectBeingDestroyed', @OpenMatEventLogOne);
delete(source);
openmat_result = [openmat_event_log, isvalid(listener)];
