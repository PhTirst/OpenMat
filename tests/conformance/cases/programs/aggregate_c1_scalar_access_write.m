cells = {10, 20};
paren_result = cells(2);
brace_result = cells{2};
cells{1} = [];
cells(2) = {int8(-8)};

record = struct('first', 1, 'second', 2);
field_result = record.second;
record.third = {3, true};

openmat_result = {paren_result, brace_result, cells, ...
    field_result, record};
