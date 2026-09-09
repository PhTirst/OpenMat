caller_value = openmat_scope_outer();

base_value = openmat_scope_base_writer();
base_observed = evalin('base', 'openmat_scope_base_value');
evalin('base', 'clear openmat_scope_base_value openmat_scope_base_seen');

source_value = [1, 2];
openmat_scope_copy(source_value);
source_value(1) = 9;
copied_value = openmat_scope_copy_value(1);

openmat_result = caller_value == 35 ...
    && base_value == 12 ...
    && base_observed == 11 ...
    && copied_value == 1;

function out = openmat_scope_outer()
x = 1;
child_value = openmat_scope_inner();
out = child_value + x + y + z;
end

function out = openmat_scope_inner()
assignin('caller', 'x', 3);
seen_x = evalin('caller', 'x');
evalin('caller', 'y = x + 2;');
feval('assignin', 'caller', 'z', 7);

caught = false;
try
    evalin('caller', 'x = 4; openmat_scope_missing_name');
catch
    caught = true;
end

evalin_handle = @evalin;
error_side_effect = evalin_handle('caller', 'x');
out = seen_x + evalin('caller', 'y') + evalin('caller', 'z') ...
    + error_side_effect + double(caught) - 1;
end

function out = openmat_scope_base_writer()
evalin("base", ...
    "assignin('base', 'openmat_scope_base_value', 11); openmat_scope_base_seen = openmat_scope_base_value + 1;");
out = evalin("base", "openmat_scope_base_seen");
end

function openmat_scope_copy(value)
assignin('caller', 'openmat_scope_copy_value', value);
end
