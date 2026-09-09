function K = poissonCell(space, geometry, rule, e)
%POISSONCELL User-owned scalar Laplace kernel, not a framework PDE class.
    fe = space.element(e);
    physical = geometry.evaluate(e, rule.Points);
    basis = fem.mapBasis(fe, fe.tabulate(rule.Points, 1), physical);
    K = zeros(fe.NumDofs, fe.NumDofs);
    for q = 1:rule.NumPoints
        gradient = reshape(basis.Gradients(:, :, :, q), fe.NumDofs, 2);
        K = K + rule.Weights(q) * physical.Measure(q) * (gradient * gradient.');
    end
end
