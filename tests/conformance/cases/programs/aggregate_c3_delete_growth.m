cells = {1, 2; 3, 4};
cells(1, :) = [];
cells{4} = 40;

records = struct('beta', {1, 2, 3}, ...
    'alpha', {'a', 'b', 'c'});
records(2) = [];
records(4).beta = 40;
records(4).alpha = 'd';

openmat_result = {cells, records};
