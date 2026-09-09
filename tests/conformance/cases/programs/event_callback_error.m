global openmat_event_log
openmat_event_log = [];
source = OpenMatEventSource();
second = addlistener(source, 'Pulse', @OpenMatEventLogTwo);
first = addlistener(source, 'Pulse', @OpenMatEventThrow);
caught = false;
try
    source.fire();
catch
    caught = true;
end
openmat_result = [caught, numel(openmat_event_log) == 0];
delete(first);
delete(second);
delete(source);
