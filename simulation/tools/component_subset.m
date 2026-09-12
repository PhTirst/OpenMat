function z = component_subset(t, x, q, u, p)
% OpenMat-authored fixed-shape language acceptance; no third-party source.
A = [1, 2; 3, 4];
b = [u; 1];
v = A*b;
w = zeros(2, 1);
for i = 2:-1:1
    w(i) = v(i) + A(i, 2);
end
R = reshape(A.', 4, 1);
if u > 2
    z = w(1) + w(2) + R(2);
elseif u < 0
    z = -w(1) + size(A, 2);
else
    z = numel(A);
end
end
