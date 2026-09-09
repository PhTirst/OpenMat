[first_value, first_was_empty] = openmat_persistent_counter_step();
[second_value, second_was_empty] = openmat_persistent_counter_step();
[third_value, third_was_empty] = openmat_persistent_counter_step();
openmat_result = [double(first_was_empty), first_value, ...
    double(second_was_empty), second_value, ...
    double(third_was_empty), third_value];

function [value, was_empty] = openmat_persistent_counter_step()
persistent state
was_empty = isempty(state);
if was_empty
    state = 10;
else
    state = state + 1;
end
value = state;
end
