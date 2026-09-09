ok = true;

lastwarn('');
warning('Probe:Visible', 'value %d', 7);
[message, identifier] = lastwarn();
ok = ok && strcmp(identifier, 'Probe:Visible');
ok = ok && strcmp(message, 'value 7');

previous = warning('off', 'Probe:Muted');
ok = ok && strcmp(previous.state, 'on');
ok = ok && strcmp(previous.identifier, 'Probe:Muted');

lastwarn('');
warning('Probe:Muted', 'hidden');
[message, identifier] = lastwarn();
ok = ok && strcmp(identifier, 'Probe:Muted');
ok = ok && strcmp(message, 'hidden');

queried = warning('query', 'Probe:Muted');
ok = ok && strcmp(queried.state, 'off');
ok = ok && strcmp(queried.identifier, 'Probe:Muted');

warning('on', 'Probe:Muted');
warning('plain %d', 3);
[message, identifier] = lastwarn();
ok = ok && strcmp(identifier, '');
ok = ok && strcmp(message, 'plain 3');

[previous_message, previous_identifier] = lastwarn('next', 'Probe:Next');
ok = ok && strcmp(previous_identifier, '');
ok = ok && strcmp(previous_message, 'plain 3');
[message, identifier] = lastwarn();
ok = ok && strcmp(identifier, 'Probe:Next');
ok = ok && strcmp(message, 'next');

openmat_result = ok;
