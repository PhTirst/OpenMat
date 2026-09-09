function OpenMatEventMutate(source, event_data)
global openmat_event_log openmat_event_late_listener
global openmat_event_added openmat_event_added_listener
openmat_event_log = [openmat_event_log, 1];
if isvalid(openmat_event_late_listener)
    delete(openmat_event_late_listener);
end
if ~openmat_event_added
    openmat_event_added = true;
    openmat_event_added_listener = addlistener(source, 'Pulse', @OpenMatEventLogThree);
end
end
