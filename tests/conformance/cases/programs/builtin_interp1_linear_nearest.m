linear_value = interp1([10 20 30], [1 1.5 3]);
nearest_tie = interp1([1 2 3], [10 20 30], [1.5 2.5], 'nearest');
descending = interp1([3 2 1], [30 20 10], [1.5 2.5]);
nonmonotonic = interp1([3 1 2], [300 10 20], [1 1.5 2.5 3]);
linear_extrap = interp1([1 2 3], [10 20 30], [0 4], 'linear', 'extrap');
nearest_extrap = interp1([1 2 3], [10 20 30], [0 4], 'nearest', 'extrap');
numeric_extrap = interp1([1 2 3], [10 20 30], [0 4], 'linear', 99);
missing_extrap = interp1([1 2 3], [10 20 30], [0 4], 'linear');
openmat_result = [ ...
    linear_value, nearest_tie, descending, nonmonotonic, ...
    linear_extrap, nearest_extrap, numeric_extrap, missing_extrap ...
];
