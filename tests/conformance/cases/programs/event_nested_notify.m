global openmat_event_log openmat_event_nested
openmat_event_log = [];
openmat_event_nested = false;
source = OpenMatEventSource();
first = addlistener(source, 'Pulse', @OpenMatEventNested);
second = addlistener(source, 'Pulse', @OpenMatEventLogTwo);
source.fire();
openmat_result = openmat_event_log;
delete(first);
delete(second);
delete(source);
