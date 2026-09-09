A = [1 2; 3 4];
C = [1+2i 2; 3 4-1i];
openmat_result = [norm(A), norm(A, 1), norm(A, Inf), norm(A, 'fro'), ...
    norm(C), norm([3 4], 3), norm([3 4], -Inf), norm([])];
