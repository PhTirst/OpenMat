x = [1; 2; 3];
y = [10 100; 20 200; 30 300];
vector_query = interp1(x, y, [1 2.5]);
matrix_query = interp1(x, y, [1 2; 2.5 3]);
single_value = interp1([1 2 3], single([10 20 30]), [1 1.5 3]);
complex_value = interp1([1 2 3], [10+1i 20+2i 30+3i], [1 1.5 3]);
openmat_result = [ ...
    vector_query(:).', matrix_query(:).', double(single_value), complex_value, ...
    double(strcmp(class(single_value), 'single')), size(matrix_query) ...
];
