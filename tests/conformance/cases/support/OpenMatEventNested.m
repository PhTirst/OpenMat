function OpenMatEventNested(source, event_data)
global openmat_event_log openmat_event_nested
openmat_event_log = [openmat_event_log, 1];
if ~openmat_event_nested
    openmat_event_nested = true;
    notify(source, 'Pulse');
end
end
