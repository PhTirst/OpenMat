ok = true;

primary = MException('Probe:Primary', 'primary value %d', 7);
cause = MException('Probe:Cause', 'cause value');
combined = addCause(primary, cause);

ok = ok && strcmp(class(primary), 'MException');
ok = ok && strcmp(primary.identifier, 'Probe:Primary');
ok = ok && strcmp(primary.message, 'primary value 7');
ok = ok && isempty(primary.stack) && isempty(primary.cause);
ok = ok && isempty(combined.stack) && numel(combined.cause) == 1;
ok = ok && strcmp(combined.cause{1}.identifier, 'Probe:Cause');

basic = getReport(combined, 'basic', 'hyperlinks', 'off');
extended = combined.getReport('extended', 'hyperlinks', 'off');
ok = ok && contains(basic, 'primary value 7');
ok = ok && ~contains(basic, 'cause value');
ok = ok && contains(extended, 'primary value 7');
ok = ok && contains(extended, 'cause value');

try
    openmat_exception_throw(combined);
    ok = false;
catch thrown
    ok = ok && strcmp(thrown.identifier, 'Probe:Primary');
    ok = ok && strcmp(thrown.message, 'primary value 7');
    ok = ok && ~isempty(thrown.stack);
    ok = ok && strcmp(thrown.stack(1).name, 'openmat_exception_throw');
    ok = ok && numel(thrown.cause) == 1;
    ok = ok && strcmp(thrown.cause{1}.identifier, 'Probe:Cause');
end
ok = ok && isempty(primary.stack) && isempty(combined.stack);

try
    openmat_exception_call_throw_as_caller(primary);
    ok = false;
catch caller_view
    ok = ok && ~isempty(caller_view.stack);
    ok = ok && strcmp(caller_view.stack(1).name, ...
        'openmat_exception_call_throw_as_caller');
end

try
    primary.message = 'changed';
    ok = false;
catch
end

openmat_result = ok;
