a = inv(zeros(0, 0));
b = pinv(zeros(0, 3));
c = kron(zeros(0, 2), ones(3, 4));
openmat_result = [size(a), size(b), size(c)];
