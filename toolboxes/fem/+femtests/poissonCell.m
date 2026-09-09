function K = poissonCell(space, geometry, rule, e)
%POISSONCELL Example user kernel; the framework has no built-in PDE model.
    element = space.element(e);
    g = geometry.evaluate(e, rule.Points);
    basis = fem.mapBasis(element, element.tabulate(rule.Points, 1), g);
    K = zeros(element.NumDofs, element.NumDofs);
    for q = 1:rule.NumPoints
        gradient = reshape(basis.Gradients(:, :, :, q), element.NumDofs, 2);
        K = K + (rule.Weights(q) * g.Measure(q)) * (gradient * gradient.');
    end
end
