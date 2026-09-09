global openmat_event_log
openmat_event_log = [];
source = OpenMatEventSource();
listener = addlistener(source, 'Pulse', @OpenMatEventLogOne);
delete(listener);
source.fire();
temporary = addlistener(source, 'Pulse', @OpenMatEventLogOne);
clear temporary
source.fire();
openmat_result = [numel(openmat_event_log) == 0, isvalid(listener)];
delete(source);
