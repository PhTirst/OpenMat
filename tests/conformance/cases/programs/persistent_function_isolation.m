first_a = openmat_persistent_isolation_a();
first_b = openmat_persistent_isolation_b();
second_a = openmat_persistent_isolation_a();
second_b = openmat_persistent_isolation_b();
openmat_result = [first_a, first_b, second_a, second_b];

function value = openmat_persistent_isolation_a()
persistent state
if isempty(state)
    state = 1;
else
    state = state + 1;
end
value = state;
end

function value = openmat_persistent_isolation_b()
persistent state
if isempty(state)
    state = 100;
else
    state = state + 10;
end
value = state;
end
