x = 4;
expression_value = eval('x + 1');
eval('x = x + 2; y = x * 3;');

[pair_first, pair_second] = eval('openmat_eval_pair()');

ans = 99;
string_value = eval("4 + 5");
output_preserved_ans = ans == 99;
eval('1 + 2');
statement_ans = ans;

x = 1;
runtime_error_caught = false;
try
    eval('x = 2; openmat_eval_missing_name');
catch
    runtime_error_caught = true;
end
error_side_effect = x;

invalid_output_caught = false;
try
    invalid_output = eval('x = 100');
catch
    invalid_output_caught = true;
end
invalid_output_preserved_x = x;

function_workspace_value = openmat_eval_local(4);
feval_value = feval('eval', 'y + 1');
eval_handle = @eval;
eval_handle('z = y + 2;');

openmat_result = expression_value == 5 ...
    && y == 18 ...
    && pair_first == 2 && pair_second == 3 ...
    && string_value == 9 ...
    && output_preserved_ans ...
    && statement_ans == 3 ...
    && runtime_error_caught && error_side_effect == 2 ...
    && invalid_output_caught && invalid_output_preserved_x == 2 ...
    && function_workspace_value == 11 ...
    && feval_value == 19 ...
    && z == 20;

function [first, second] = openmat_eval_pair()
first = 2;
second = 3;
end

function out = openmat_eval_local(seed)
x = seed;
value = eval('x + 1');
eval('extra = x + 2;');
out = value + extra;
end
