without_extra = openmat_variadic_inputs(4);
with_extra = openmat_variadic_inputs(4, 5, 6);

single_output = openmat_variadic_outputs(10);
[multiple_head, second_output, third_output] = openmat_variadic_outputs(10);

openmat_result = [without_extra, with_extra, single_output, multiple_head, second_output, third_output];
