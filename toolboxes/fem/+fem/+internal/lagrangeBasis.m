function basis = lagrangeBasis(name, degree, points, order)
    nq = size(points, 2);
    x = points(1, :);
    y = points(2, :);
    if strcmp(name, 'triangle')
        l = [1 - x - y; x; y];
        dl = [-1, -1; 1, 0; 0, 1];
        if degree == 1
            values = l;
            gradients = repmat(reshape(dl, [3, 1, 2, 1]), [1, 1, 1, nq]);
        else
            values = zeros(6, nq);
            gradients = zeros(6, 1, 2, nq);
            for j = 1:3
                values(j, :) = l(j, :) .* (2 * l(j, :) - 1);
                for d = 1:2
                    gradients(j, 1, d, :) = reshape((4 * l(j, :) - 1) * dl(j, d), [1, 1, 1, nq]);
                end
            end
            edges = [1, 2, 3; 2, 3, 1];
            for j = 1:3
                a = edges(1, j);
                b = edges(2, j);
                values(3 + j, :) = 4 * l(a, :) .* l(b, :);
                for d = 1:2
                    derivative = 4 * (dl(a, d) * l(b, :) + l(a, :) * dl(b, d));
                    gradients(3 + j, 1, d, :) = reshape(derivative, [1, 1, 1, nq]);
                end
            end
        end
    else
        if degree == 1
            lx = [1 - x; x];
            ly = [1 - y; y];
            dx = [-ones(1, nq); ones(1, nq)];
            dy = dx;
            nodes = [1, 2, 2, 1; 1, 1, 2, 2];
        else
            lx = [2*x.^2 - 3*x + 1; 2*x.^2 - x; 4*x.*(1-x)];
            ly = [2*y.^2 - 3*y + 1; 2*y.^2 - y; 4*y.*(1-y)];
            dx = [4*x - 3; 4*x - 1; 4 - 8*x];
            dy = [4*y - 3; 4*y - 1; 4 - 8*y];
            nodes = [1, 2, 2, 1, 3, 2, 3, 1, 3; 1, 1, 2, 2, 1, 3, 2, 3, 3];
        end
        nd = size(nodes, 2);
        values = zeros(nd, nq);
        gradients = zeros(nd, 1, 2, nq);
        for j = 1:nd
            a = nodes(1, j);
            b = nodes(2, j);
            values(j, :) = lx(a, :) .* ly(b, :);
            gradients(j, 1, 1, :) = reshape(dx(a, :) .* ly(b, :), [1, 1, 1, nq]);
            gradients(j, 1, 2, :) = reshape(lx(a, :) .* dy(b, :), [1, 1, 1, nq]);
        end
    end
    basis = struct();
    basis.Values = reshape(values, [size(values, 1), 1, nq]);
    if order == 1
        basis.Gradients = gradients;
    end
end
