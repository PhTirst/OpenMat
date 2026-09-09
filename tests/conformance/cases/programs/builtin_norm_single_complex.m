A = single([1 2; 3 4]);
C = complex(single([1 2; 3 4]), single([2 0; 0 -1]));
openmat_result = [norm(A), norm(C)];
