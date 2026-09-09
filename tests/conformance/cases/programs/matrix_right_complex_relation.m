left = [1+2i, 3-1i];
right = [1+1i, 0; 0, 1-1i; 1, 1];

quotient = left / right;
conjugate_transpose_relation = (right' \ left')';

openmat_result = [quotient, quotient - conjugate_transpose_relation];
