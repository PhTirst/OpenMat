clear parula;

parula;
statement_call = isequal(size(ans), [256 3]);

value_result = parula;
value_call = isequal(size(value_result), [256 3]);

argument_call = isequal(size(parula), [256 3]);

named_handle = @parula;
handle_distinct = isa(named_handle, 'function_handle');
handle_call = isequal(size(named_handle(4)), [4 3]);

parula = 17;
shadow_value = parula == 17;
shadow_argument = isequal(size(parula), [1 1]);

clear parula;
revealed_result = parula;
revealed_value = isequal(size(revealed_result), [256 3]);
revealed_argument = isequal(size(parula), [256 3]);

openmat_result = [ ...
    statement_call, value_call, argument_call, handle_distinct, handle_call, ...
    shadow_value, shadow_argument, revealed_value, revealed_argument ...
];
