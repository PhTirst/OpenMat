first = openmat_persistent_array_step(3);
second = openmat_persistent_array_step(-2);
openmat_result = [first, second];

function value = openmat_persistent_array_step(amount)
persistent state
if isempty(state)
    state = [2, 4, 6];
end
before = state;
state(2) = state(2) + amount;
value = [before, state];
end
