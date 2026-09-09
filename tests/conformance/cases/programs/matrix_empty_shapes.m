left_empty_rhs = eye(2) \ zeros(2, 0);
left_empty_unknowns = zeros(3, 0) \ zeros(3, 2);
left_empty_equations = zeros(0, 3) \ zeros(0, 2);
left_empty_both = [] \ [];
right_empty_rows = zeros(0, 2) / eye(2);
right_empty_divisor_rows = zeros(2, 0) / zeros(3, 0);

openmat_result = {left_empty_rhs, left_empty_unknowns, ...
    left_empty_equations, left_empty_both, right_empty_rows, ...
    right_empty_divisor_rows};
