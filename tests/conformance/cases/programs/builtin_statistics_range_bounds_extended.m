a = reshape(1:8, 2, 2, 2);
r_vec = range(a, [1 2]);
[b_vec_lo, b_vec_hi] = bounds(a, [1 2]);
r_nan_default = range([NaN 1 3]);
r_nan_include = range([NaN 1 3], 'includenan');
[b_nan_lo, b_nan_hi] = bounds([NaN 1 3], 'includenan');
r_complex = range(complex([1 2], [1 2]));
[b_complex_lo, b_complex_hi] = bounds(complex([1 2], [1 2]));
r_single = range(single([1 3]));
r_logical = range(logical([0 1]));
r_integer = range(int8([-128 127]));
[b_integer_lo, b_integer_hi] = bounds(int8([-2 3]));
r_char = range('az');
[b_char_lo, b_char_hi] = bounds('az');
r_empty = range(zeros(0, 3));
[b_empty_lo, b_empty_hi] = bounds(zeros(0, 3), 2);
openmat_result = { ...
    r_vec, b_vec_lo, b_vec_hi, ...
    r_nan_default, r_nan_include, b_nan_lo, b_nan_hi, ...
    r_complex, b_complex_lo, b_complex_hi, ...
    r_single, r_logical, r_integer, b_integer_lo, b_integer_hi, ...
    r_char, b_char_lo, b_char_hi, ...
    r_empty, b_empty_lo, b_empty_hi ...
};
