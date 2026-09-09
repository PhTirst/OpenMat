cells = {11, 22; 33, 44};
[first, second, third] = cells{:};
[cells{[1, 4]}] = deal(101, 404);

records = struct('beta', {1, 2, 3}, ...
    'alpha', {'a', 'b', 'c'});
[x, y, z] = records.beta;
[records([1, 3]).alpha] = deal('left', 'right');

openmat_result = {first, second, third, cells, ...
    x, y, z, records};
