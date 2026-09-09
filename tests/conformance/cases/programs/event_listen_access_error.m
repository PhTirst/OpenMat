source = OpenMatEventAccessSource();
listener = addlistener(source, 'Pulse', @OpenMatEventLogOne);
openmat_result = isvalid(listener);
