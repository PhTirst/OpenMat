a = 13;
b = 5;

a + b;
addition_is_expression = ans == 18;
a - b;
subtraction_is_expression = ans == 8;

global openmat_command_semantics_arguments;
openmat_command_semantics_arguments = {};

openmat_command_semantics_capture -v7.3 sample.mat alpha;
dash_arguments = isequal(openmat_command_semantics_arguments, ...
    {'-v7.3', 'sample.mat', 'alpha'});

openmat_command_semantics_capture a + b;
operator_arguments = isequal(openmat_command_semantics_arguments, ...
    {'a', '+', 'b'});

openmat_command_semantics_capture alpha beta gamma;
plain_arguments = isequal(openmat_command_semantics_arguments, ...
    {'alpha', 'beta', 'gamma'});

clear global openmat_command_semantics_arguments;
openmat_result = [addition_is_expression, subtraction_is_expression, ...
    dash_arguments, operator_arguments, plain_arguments];

function openmat_command_semantics_capture(varargin)
global openmat_command_semantics_arguments;
openmat_command_semantics_arguments = varargin;
end
