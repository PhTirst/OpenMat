global openmat_event_log openmat_event_late_listener
global openmat_event_added openmat_event_added_listener
openmat_event_log = [];
openmat_event_added = false;
source = OpenMatEventSource();
openmat_event_late_listener = addlistener(source, 'Pulse', @OpenMatEventLogTwo);
first = addlistener(source, 'Pulse', @OpenMatEventMutate);
source.fire();
first_pass = openmat_event_log;
source.fire();
openmat_result = [first_pass, -1, openmat_event_log];
delete(first);
delete(openmat_event_added_listener);
delete(source);
