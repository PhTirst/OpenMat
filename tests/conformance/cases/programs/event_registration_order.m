global openmat_event_log
openmat_event_log = [];
source = OpenMatEventSource();
first = addlistener(source, 'Pulse', @OpenMatEventLogOne);
second = addlistener(source, 'Pulse', @OpenMatEventLogTwo);
source.fire();
openmat_result = openmat_event_log;
delete(first);
delete(second);
delete(source);
