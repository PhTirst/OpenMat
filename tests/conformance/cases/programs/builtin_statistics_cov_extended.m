a = [1 10; 2 20; 4 40];
c_matrix = cov(a);
c_vector = cov([1 2 3]);
c_normalized = cov([1 2 3], 1);
c_pair = cov([1 2 3], [2 4 8]);
c_partial = cov([1 NaN; 2 4; NaN 8], 'partialrows');
c_complete = cov([1 NaN; 2 4; 4 8], 'omitrows');
c_single = cov(single([1 2 3]), [2 4 8]);
c_empty_columns = cov(zeros(0, 3));
c_no_variables = cov(zeros(3, 0));
openmat_result = { ...
    c_matrix, c_vector, c_normalized, c_pair, ...
    c_partial, c_complete, c_single, ...
    c_empty_columns, c_no_variables ...
};
