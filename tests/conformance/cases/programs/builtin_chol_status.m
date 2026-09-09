a = complex([4, 1; 1, 3], [0, 1; -1, 0]);
[r, upper_status] = chol(a);
[l, lower_status] = chol(a, 'lower');
[partial, failure_status] = chol([1, 2, 0; 2, 1, 0; 0, 0, 3]);
upper_residual = a - r' * r;
lower_residual = a - l * l';
openmat_result = [ ...
    norm(upper_residual(:)), norm(lower_residual(:)), ...
    upper_status, lower_status, failure_status, size(partial, 1), det(a) ...
];
