source = OpenMatEventSource();
listener = addlistener(source, 'Pulse', @OpenMatEventLogOne);
delete(source);
openmat_result = [isvalid(source), isvalid(listener)];
