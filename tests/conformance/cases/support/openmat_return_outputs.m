function [first, second, third] = openmat_return_outputs(value, stop_early)
first = value;
second = -1;
third = -1;
if stop_early
    second = value + 1;
    third = value + 2;
    return;
end
second = value * 2;
third = value * 3;
end
