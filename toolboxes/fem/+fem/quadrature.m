function rule = quadrature(referenceCell, degree)
%QUADRATURE Fixed Gaussian rule exact for reference polynomials up to degree.
% Triangle: total degree. Quadrilateral: degree in each coordinate.
% degree 0..8 is supported. Nonpolynomial integrands need convergence checks.
    if ~isa(referenceCell, 'fem.ReferenceCell')
        referenceCell = fem.ReferenceCell(referenceCell);
    end
    if ~isscalar(degree) || ~isreal(degree) || ~isfinite(degree) || ...
            degree < 0 || degree > 8 || degree ~= floor(degree)
        error('fem:UnsupportedQuadrature', 'Requested polynomial degree must be an integer from 0 to 8.');
    end
    triangle = strcmp(referenceCell.Name, 'triangle');
    n = max(1, ceil((degree + 1 + triangle) / 2));
    [x, w] = fem.internal.gaussLegendre(n);
    points = zeros(2, n * n);
    weights = zeros(n * n, 1);
    for j = 1:n
        for i = 1:n
            k = i + (j - 1) * n;
            if triangle
                points(:, k) = [x(i); (1 - x(i)) * x(j)];
                weights(k) = w(i) * w(j) * (1 - x(i));
            else
                points(:, k) = [x(i); x(j)];
                weights(k) = w(i) * w(j);
            end
        end
    end
    rule = fem.QuadratureRule(referenceCell, points, weights);
end
