full_value = conv([1 2 3], [1 1]);
same_value = conv([1 2 3], [1 1], 'same');
valid_value = conv([1 2 3], [1 1], 'valid');
matrix_value = conv2([1 2; 3 4], [1 2; 3 4]);
[quotient, remainder] = deconv([1 3 2], [1 1]);
openmat_result = [full_value, same_value, valid_value, matrix_value(:).', quotient, remainder];
