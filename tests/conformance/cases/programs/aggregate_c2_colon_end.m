cells = {11, 22; 33, 44};
all_cells = cells(:);
reverse_tail = cells(end:-1:2);
last_column = cells(:, end);

records = struct('beta', {1, 2, 3, 4}, ...
    'alpha', {'a', 'b', 'c', 'd'});
selected_records = records(end:-2:1);

openmat_result = {all_cells, reverse_tail, ...
    last_column, selected_records};
