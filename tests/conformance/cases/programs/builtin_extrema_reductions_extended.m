A = [3 1 1; NaN 2 0];
[minimum, minimum_index] = min(A);
[maximum, maximum_index] = max(A);
[all_minimum, all_index] = min([3 1; 2 0], [], 'all', 'linear');
[vector_minimum, vector_index] = min(reshape(1:24, [2 3 4]), [], [1 3], 'linear');
[absolute_minimum, absolute_index] = min([-5 3], [], 'ComparisonMethod', 'abs');
openmat_result = [minimum, minimum_index, maximum, maximum_index, ...
    all_minimum, all_index, vector_minimum, vector_index, ...
    absolute_minimum, absolute_index];
