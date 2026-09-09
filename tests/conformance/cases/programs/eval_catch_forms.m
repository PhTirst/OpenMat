x = 1;
eval('x = 2; openmat_missing_primary', 'y = x + 3;');
recovered = eval('openmat_missing_result', '4 + 5');
successful = eval('6 + 1', 42);

eval_handle = @eval;
handle_recovered = eval_handle('openmat_missing_handle', '8 + 1');
feval_recovered = feval('eval', 'openmat_missing_feval', '10 + 1');

caught = false;
try
    eval('x = 4; openmat_missing_primary_two', ...
        'z = x + 1; openmat_missing_catch');
catch
    caught = true;
end

caller_value = openmat_eval_catch_outer();
evalin('base', 'openmat_missing_base', 'openmat_base_catch_value = 12;');
base_value = openmat_base_catch_value;
clear openmat_base_catch_value;

openmat_result = x == 4 ...
    && y == 5 ...
    && z == 5 ...
    && recovered == 9 ...
    && successful == 7 ...
    && handle_recovered == 9 ...
    && feval_recovered == 11 ...
    && caught ...
    && caller_value == 12 ...
    && base_value == 12;

function out = openmat_eval_catch_outer()
x = 1;
child = openmat_eval_catch_inner();
out = child + x;
end

function out = openmat_eval_catch_inner()
evalin('caller', 'x = 3; openmat_missing_caller', 'local_y = 7;');
out = evalin('caller', 'openmat_missing_caller_result', 'local_y + 2');
end
