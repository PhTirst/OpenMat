global openmat_event_log
openmat_event_log = [];
source = OpenMatEventAccessSource();
listener = source.listenInside(@OpenMatEventLogOne);
source.fire();
openmat_result = [openmat_event_log, isvalid(listener)];
delete(listener);
delete(source);
