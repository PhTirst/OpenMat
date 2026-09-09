[skipped_value, skipped_was_empty] = ...
    openmat_persistent_conditional_step(false);
[first_value, first_was_empty] = ...
    openmat_persistent_conditional_step(true);
[skipped_again_value, skipped_again_was_empty] = ...
    openmat_persistent_conditional_step(false);
[second_value, second_was_empty] = ...
    openmat_persistent_conditional_step(true);
openmat_result = [skipped_value, double(skipped_was_empty), ...
    first_value, double(first_was_empty), ...
    skipped_again_value, double(skipped_again_was_empty), ...
    second_value, double(second_was_empty)];

function [value, was_empty] = openmat_persistent_conditional_step(enable)
value = -1;
was_empty = false;
if enable
    persistent state
    was_empty = isempty(state);
    if was_empty
        state = 8;
    else
        state = state + 1;
    end
    value = state;
end
end
