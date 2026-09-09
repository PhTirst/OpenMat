A = [1 2; 3 4];
inverse = inv(A);
pseudo = pinv([1 2]);
diagonal = trace(A);
vector = cross([1 0 0], [0 1 0]);
product = kron([1 2], [3; 4]);
openmat_result = [inverse(:).', pseudo(:).', diagonal, vector, product(:).'];
