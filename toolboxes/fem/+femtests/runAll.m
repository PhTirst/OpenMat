function runAll()
%RUNALL Independently authored checks, shared by OpenMat and MATLAB R2022b.
    tests = {@testIds, @testInvalidMesh, @testTopologySets, @testBasis, ...
        @testDefinitionValidation, @testDofMap, @testCellAssignments, ...
        @testQuadrature, @testGeometry, @testInvalidGeometry, ...
        @testAssembly, @testComplexAssembly, @testConstraints, ...
        @testGeneralConstraints, @testEmpty, @testPoissonPatch, ...
        @testQueryValidation, @testCurvedGeometry, @testPermutationTransform};
    for k = 1:numel(tests)
        callback = tests{k};
        callback();
        disp(['FEM test passed: ', num2str(k)]);
    end
    disp(['FEM test groups passed: ', num2str(numel(tests))]);
end

function mesh = twoTriangles()
    mesh = fem.Mesh([0, 1, 0, 1; 0, 0, 1, 1], ...
        {'triangle'; 'triangle'}, [1; 4; 7], [1; 2; 3; 2; 4; 3]);
end

function testIds()
    large = bitor(bitshift(uint64(1), 60), uint64(3));
    next = bitor(bitshift(uint64(1), 60), uint64(4));
    ids = [uint64(0); uint64(10); large; next];
    options = struct();
    options.VertexIds = ids;
    options.CellIds = uint64([900; 0]);
    options.Connectivity = 'ids';
    mesh = fem.Mesh([0, 1, 0, 1; 0, 0, 1, 1], {'triangle'; 'triangle'}, ...
        [1; 4; 7], ids([1; 2; 3; 2; 4; 3]), options);
    assert(isequal(mesh.CellVertices, [1; 2; 3; 2; 4; 3]));
    assert(isequal(mesh.vertexIndex([large; uint64(0)]), [3; 1]));
    assert(isequal(mesh.vertexId([4; 3]), ids([4; 3])));
    assert(mesh.cellIndex(uint64(0)) == 2);
    assert(isequal(mesh.cellId(1), uint64(900)));
    expectError(@() mesh.vertexIndex(uint64(999)), 'fem:UnknownId');
end

function testInvalidMesh()
    x = [0, 1, 0; 0, 0, 1];
    makeMesh = @fem.Mesh;
    expectError(@() makeMesh(x, {'triangle'}, [0; 3], [1; 2; 3]), 'fem:InvalidIndex');
    expectError(@() makeMesh(x, {'triangle'}, [1; 3], [1; 2; 3]), 'fem:InvalidPointers');
    expectError(@() makeMesh(x, {'triangle'}, [1; 4], [1; 2; 2]), 'fem:InvalidCell');
    expectError(@() makeMesh(x, {'triangle'}, [1; 4], [0; 1; 2]), 'fem:InvalidIndex');
    options = struct('VertexIds', uint64([1; 1; 2]));
    expectError(@() makeMesh(x, {'triangle'}, [1; 4], [1; 2; 3], options), 'fem:DuplicateId');
    options = struct('Connectivity', 'ids', 'VertexIds', uint64([0; 10; 20]));
    expectError(@() makeMesh(x, {'triangle'}, [1; 4], [0; 10; 30], options), 'fem:UnknownId');
    options = struct('VertexIds', [0; 10; 2^54]);
    expectError(@() makeMesh(x, {'triangle'}, [1; 4], [1; 2; 3], options), 'fem:InvalidId');
end

function testTopologySets()
    mesh = twoTriangles();
    [a, da] = mesh.cellEdges(1);
    [b, db] = mesh.cellEdges(2);
    edge = intersect(a, b);
    assert(numel(edge) == 1 && da(a == edge) == -db(b == edge));
    [cells, locals] = mesh.edgeCells(edge);
    assert(isequal(cells, [1; 2]) && isequal(locals, [2; 3]));
    assert(mesh.cellVertexCount(1) == 3);
    changed = mesh.withSet('wall', 1, mesh.boundaryEdges());
    changed = changed.withSet('wall', 0, [1; 3]);
    assert(numel(changed.edgeSet('wall')) == 4);
    assert(isequal(changed.vertexSet('wall'), [1; 3]));
    assert(isempty(mesh.Sets));
    expectError(@() mesh.edgeSet('wall'), 'fem:UnknownSet');
    changed = changed.withSet('wall', 1, edge);
    assert(isequal(changed.edgeSet('wall'), edge));
end

function testBasis()
    names = {'triangle', 'quadrilateral'};
    for c = 1:2
        for degree = 1:2
            fe = fem.lagrange(names{c}, degree);
            ref = fe.ReferenceCell;
            nodes = ref.Vertices;
            if degree == 2
                nodes = [nodes, (ref.Vertices(:, ref.Edges(1, :)) + ref.Vertices(:, ref.Edges(2, :))) / 2]; %#ok<AGROW>
                if c == 2
                    nodes = [nodes, [0.5; 0.5]]; %#ok<AGROW>
                end
            end
            nodal = fe.tabulate(nodes, 1);
            near(reshape(nodal.Values, fe.NumDofs, fe.NumDofs), eye(fe.NumDofs), 1e-13);
            points = [0.12, 0.25, 0.51; 0.13, 0.6, 0.25];
            basis = fe.tabulate(points, 1);
            near(sum(basis.Values, 1), ones(1, 1, 3), 1e-13);
            near(sum(basis.Gradients, 1), zeros(1, 1, 2, 3), 1e-13);
            for d = 1:2
                offset = zeros(2, 3);
                offset(d, :) = 1e-6;
                plus = fe.tabulate(points + offset, 0);
                minus = fe.tabulate(points - offset, 0);
                exact = reshape(basis.Gradients(:, :, d, :), [fe.NumDofs, 1, 3]);
                near((plus.Values - minus.Values) / 2e-6, exact, 2e-9);
            end
        end
    end
end

function def = edgeDefinition()
    def = struct();
    def.ReferenceCell = 'triangle';
    def.NumDofs = 3;
    def.ValueShape = 2;
    def.EntityDofs = {{[], [], []}, {1, 2, 3}, {[]}};
    def.EntityKeys = {{'', '', ''}, {'test-edge-moment', 'test-edge-moment', 'test-edge-moment'}, {''}};
    def.MapType = 'covariant';
    def.TabulateFcn = @edgeBasis;
    def.TransformFcn = @(directions) sparse(diag(directions));
end

function basis = edgeBasis(points, order)
    basis = struct('Values', ones(3, 2, size(points, 2)));
    if order == 1
        basis.Gradients = zeros(3, 2, 2, size(points, 2));
    end
end

function testDefinitionValidation()
    def = edgeDefinition();
    fe = fem.FiniteElement(def);
    assert(fe.NumDofs == 3);
    def.EntityDofs = {{[], [], []}, {1, 1, 3}, {[]}};
    makeElement = @fem.FiniteElement;
    expectError(@() makeElement(def), 'fem:InvalidElement');
    def = edgeDefinition();
    def.TransformFcn = @(directions) sparse(3, 3);
    fe = fem.FiniteElement(def);
    expectError(@() fe.dofTransform([1; 1; 1]), 'fem:InvalidTransform');
    def = edgeDefinition();
    def.TabulateFcn = @(points, order) struct('Values', zeros(2, 2, size(points, 2)));
    fe = fem.FiniteElement(def);
    expectError(@() fe.tabulate([0; 0], 0), 'fem:InvalidTabulation');
end

function testDofMap()
    mesh = twoTriangles();
    p2 = fem.lagrange('triangle', 2);
    V = fem.FunctionSpace(mesh, {p2}, [1; 1]);
    assert(V.Dofs.NumDofs == 9 && V.Dofs.cellDofCount(1) == 6);
    assert(numel(intersect(V.Dofs.cellDofs(1), V.Dofs.cellDofs(2))) == 3);
    assert(numel(V.Dofs.Transforms) == 1 && issparse(V.Dofs.cellTransform(1)));
    assert(numel(V.edgeDofs(mesh.boundaryEdges())) == 8);
    edge = fem.FiniteElement(edgeDefinition());
    W = fem.FunctionSpace(mesh, {edge}, [1; 1]);
    assert(W.Dofs.NumDofs == 5);
    g1 = W.Dofs.cellDofs(1);
    g2 = W.Dofs.cellDofs(2);
    [meshEdges1, directions1] = mesh.cellEdges(1);
    [meshEdges2, directions2] = mesh.cellEdges(2);
    assert(numel(intersect(meshEdges1, meshEdges2)) == 1);
    common = intersect(g1, g2);
    assert(numel(common) == 1);
    T1 = W.Dofs.cellTransform(1);
    T2 = W.Dofs.cellTransform(2);
    near(diag(full(T1)), directions1, 0);
    near(diag(full(T2)), directions2, 0);
    assert(T1(g1 == common, g1 == common) == -T2(g2 == common, g2 == common));
end

function testCellAssignments()
    mesh = twoTriangles();
    elements = {fem.lagrange('triangle', 1), fem.lagrange('triangle', 2)};
    makeSpace = @fem.FunctionSpace;
    expectError(@() makeSpace(mesh, elements, [1; 2]), 'fem:IncompatibleTrace');
    V = fem.FunctionSpace(mesh, elements, [1; 2], 'discontinuous');
    assert(V.Dofs.NumDofs == 9);
    assert(V.Dofs.cellDofCount(1) == 3 && V.Dofs.cellDofCount(2) == 6);
    x = [0, 1, 2, 0, 1, 2; 0, 0, 0, 1, 1, 1];
    mesh = fem.Mesh(x, {'triangle'; 'triangle'; 'quadrilateral'}, ...
        [1; 4; 7; 11], [1; 2; 4; 2; 5; 4; 2; 3; 6; 5]);
    V = fem.FunctionSpace(mesh, {elements{2}, fem.lagrange('quadrilateral', 2)}, [1; 1; 2]);
    assert(mesh.NumEdges == 8 && V.Dofs.NumDofs == 15);
    geometry = fem.GeometryMap(mesh);
    q = fem.quadrature('quadrilateral', 2);
    g = geometry.evaluate(3, q.Points);
    near(sum(q.Weights .* g.Measure), 1, 1e-13);
    invalid = fem.DofMap([1; 2; 3; 4], [1; 2; 3], 3, {speye(1)}, [1; 1; 1]);
    definitions = {elements{2}, fem.lagrange('quadrilateral', 2)};
    expectError(@() makeSpace(mesh, definitions, [1; 1; 2], invalid), ...
        'fem:InvalidDofMap');
end

function testQuadrature()
    for degree = 0:8
        triangle = fem.quadrature('triangle', degree);
        quad = fem.quadrature('quadrilateral', degree);
        for a = 0:degree
            for b = 0:degree-a
                exact = prod(1:a) * prod(1:b) / prod(1:a+b+2);
                approximate = sum(triangle.Weights .* (triangle.Points(1, :).^a .* triangle.Points(2, :).^b).');
                near(approximate, exact, 2e-13);
            end
            approximate = sum(quad.Weights .* (quad.Points(1, :).^a .* quad.Points(2, :).^degree).');
            near(approximate, 1 / ((a+1)*(degree+1)), 2e-13);
        end
    end
end

function testGeometry()
    mesh = fem.Mesh([0, 2, 0; 0, 0, 3], {'triangle'}, [1; 4], [1; 2; 3]);
    geometry = fem.GeometryMap(mesh);
    fe = fem.lagrange('triangle', 1);
    q = fem.quadrature('triangle', 2);
    g = geometry.evaluate(1, q.Points);
    near(g.DetJ, 6*ones(q.NumPoints, 1), 1e-13);
    near(sum(q.Weights .* g.Measure), 3, 1e-13);
    basis = fem.mapBasis(fe, fe.tabulate(q.Points, 1), g);
    near(reshape(basis.Gradients(:, :, :, 1), 3, 2), [-1/2, -1/3; 1/2, 0; 0, 1/3], 1e-13);
    reversed = fem.Mesh(mesh.Coordinates, {'triangle'}, [1; 4], [1; 3; 2]);
    reverseMap = fem.GeometryMap(reversed);
    rg = reverseMap.evaluate(1, q.Points);
    near(rg.DetJ, -g.DetJ, 1e-13);
    custom = fem.GeometryMap(mesh, @customGeometry);
    cg = custom.evaluate(1, q.Points);
    near(cg.Points, [2*q.Points(1, :); 3*q.Points(2, :)], 1e-13);
    edge = fem.FiniteElement(edgeDefinition());
    eb = edge.tabulate(q.Points, 0);
    mapped = fem.mapBasis(edge, eb, g);
    near(mapped.Values(:, :, 1), [0.5, 1/3; 0.5, 1/3; 0.5, 1/3], 1e-13);
    mapper = @fem.mapBasis;
    expectError(@() mapper(edge, edge.tabulate(q.Points, 1), g), 'fem:UnsupportedMapping');
end

function g = customGeometry(e, points) %#ok<INUSD> Cell-independent test mapping.
    g = struct();
    g.Points = [2*points(1, :); 3*points(2, :)];
    g.Jacobians = repmat([2, 0; 0, 3], [1, 1, size(points, 2)]);
end

function testInvalidGeometry()
    mesh = fem.Mesh([0, 1, 2; 0, 0, 0], {'triangle'}, [1; 4], [1; 2; 3]);
    makeGeometry = @fem.GeometryMap;
    expectError(@() makeGeometry(mesh), 'fem:DegenerateCell');
    mesh = fem.Mesh([0, 1, 0, 1; 0, 1, 1, 0], {'quadrilateral'}, [1; 5], [1; 2; 3; 4]);
    expectError(@() makeGeometry(mesh), 'fem:FoldedCell');
    mesh = fem.Mesh(1e-10*[0, 1, 0; 0, 0, 1], {'triangle'}, [1; 4], [1; 2; 3]);
    map = fem.GeometryMap(mesh);
    geometry = map.evaluate(1, [0; 0]);
    near(geometry.DetJ / 1e-20, 1, 1e-13);
end

function testAssembly()
    T = sparse([1, 2; 0, -1]);
    tests = fem.DofMap([1; 3; 5], [1; 2; 2; 3], 3, {T, speye(2)}, [1; 2]);
    trials = fem.DofMap([1; 4; 7], [1; 2; 3; 2; 3; 4], 4, {sparse([0, 1, 0; 1, 0, 0; 0, 0, -1])}, [1; 1]);
    local = {[1, 2, 3; 4, 5, 6], [7, 8, 9; 10, 11, 12]};
    plan = fem.AssemblyPlan(tests, trials);
    K = plan.assemble(local);
    expected = zeros(3, 4);
    for e = 1:2
        g = tests.cellDofs(e);
        h = trials.cellDofs(e);
        left = tests.cellTransform(e);
        right = trials.cellTransform(e);
        expected(g, h) = expected(g, h) + left.'*local{e}*right;
    end
    near(full(K), expected, 0);
    viaCallback = plan.assemble(@(e) local{e});
    near(full(viaCallback), expected, 0);
    F = plan.assembleVector({[1; 2], [3; 4]});
    near(F, [1; 3; 4], 0);
    near(full(plan.assemble({zeros(2, 3), zeros(2, 3)})), zeros(3, 4), 0);
    near(full(plan.assemble(local)), expected, 0);
    expectError(@() plan.assemble({zeros(3, 3), zeros(2, 3)}), 'fem:InvalidLocalMatrix');
end

function testComplexAssembly()
    T = sparse([1i, 0; 0, 1]);
    dofs = fem.DofMap([1; 3], [1; 2], 2, {T}, 1);
    local = [2, 1i; -1i, 3];
    plan = fem.AssemblyPlan(dofs, dofs, 'sesquilinear');
    near(full(plan.assemble({local})), full(T'*local*T), 1e-13);
    near(plan.assembleVector({[1i; 2]}), [1; 2], 1e-13);
    bilinear = fem.AssemblyPlan(dofs, dofs, 'bilinear');
    near(full(bilinear.assemble({local})), full(T.'*local*T), 1e-13);
end

function testConstraints()
    constraints = fem.ConstraintMap.fixed(3, 1, 5);
    K = sparse([2, -1, 0; -1, 2, -1; 0, -1, 2]);
    F = [0; 0; 0];
    [Kr, Fr] = constraints.reduce(K, F);
    near(full(Kr), [2, -1; -1, 2], 0);
    near(Fr, [5; 0], 0);
    u = constraints.expand(Kr \ Fr);
    near(u, [5; 10/3; 5/3], 1e-12);
    assert(constraints.NumDofs == 3 && constraints.NumFreeDofs == 2);
    residual = K*u-F;
    near(residual(2:3), [0; 0], 1e-12);
    fixed = @fem.ConstraintMap.fixed;
    expectError(@() fixed(3, [1; 1], [0; 1]), 'fem:DuplicateConstraint');
    none = fem.ConstraintMap.fixed(3, [], []);
    near(none.expand([1; 2; 3]), [1; 2; 3], 0);
    allFixed = fem.ConstraintMap.fixed(3, [1; 2; 3], [4; 5; 6]);
    [emptyK, emptyF] = allFixed.reduce(K, F);
    assert(isequal(size(emptyK), [0, 0]) && isequal(size(emptyF), [0, 1]));
    near(allFixed.expand(zeros(0, 1)), [4; 5; 6], 0);
end

function testGeneralConstraints()
    C = sparse([1, 0; 0, 1; 0.5, 0.5; 0, 0]);
    constraints = fem.ConstraintMap(C, [0; 0; 0; 7], [1; 2]);
    near(constraints.expand([2; 4]), [2; 4; 3; 7], 0);
    [K, F] = constraints.reduce(speye(4), zeros(4, 1));
    near(full(K), full(C.'*C), 0);
    near(F, zeros(2, 1), 0);
    makeConstraint = @fem.ConstraintMap;
    expectError(@() makeConstraint(sparse([1, 1; 1, 1]), zeros(2, 1), [1; 2]), 'fem:InvalidConstraint');
    complexC = sparse([1; 1i]);
    complexMap = fem.ConstraintMap(complexC, zeros(2, 1), 1);
    [K, F] = complexMap.reduce(speye(2), [1; 1i], 'sesquilinear');
    near(full(K), 2, 0);
    near(F, 2, 0);
end

function testEmpty()
    mesh = fem.Mesh(zeros(2, 0), {}, 1, []);
    assert(mesh.NumCells == 0 && mesh.NumEdges == 0 && isempty(mesh.boundaryEdges()));
    V = fem.FunctionSpace(mesh, {}, []);
    plan = fem.AssemblyPlan(V.Dofs, V.Dofs);
    assert(isequal(size(plan.assemble({})), [0, 0]));
    assert(isequal(size(plan.assembleVector({})), [0, 1]));
    constraints = fem.ConstraintMap.fixed(0, [], []);
    assert(isequal(size(constraints.expand(zeros(0, 1))), [0, 1]));
end

function testPoissonPatch()
    % Four triangles, one unconstrained centre vertex. Affine exact solution.
    x = [0, 1, 1, 0, 0.5; 0, 0, 1, 1, 0.5];
    mesh = fem.Mesh(x, {'triangle'; 'triangle'; 'triangle'; 'triangle'}, ...
        [1; 4; 7; 10; 13], [1; 2; 5; 2; 3; 5; 3; 4; 5; 4; 1; 5]);
    V = fem.FunctionSpace(mesh, {fem.lagrange('triangle', 1)}, ones(4, 1));
    geometry = fem.GeometryMap(mesh);
    rule = fem.quadrature('triangle', 2);
    plan = fem.AssemblyPlan(V.Dofs, V.Dofs);
    kernel = @femtests.poissonCell;
    K = plan.assemble(@(e) kernel(V, geometry, rule, e));
    exact = zeros(V.Dofs.NumDofs, 1);
    for e = 1:mesh.NumCells
        vertices = mesh.cellVertices(e);
        globalIds = V.Dofs.cellDofs(e);
        exact(globalIds) = (1 + 2*x(1, vertices) - 3*x(2, vertices)).';
    end
    boundary = V.edgeDofs(mesh.boundaryEdges());
    constraints = fem.ConstraintMap.fixed(V.Dofs.NumDofs, boundary, exact(boundary));
    [Kr, Fr] = constraints.reduce(K, zeros(V.Dofs.NumDofs, 1));
    u = constraints.expand(Kr \ Fr);
    near(u, exact, 1e-12);
    near(full(K), full(K.'), 1e-13);
    assert(constraints.NumFreeDofs == 1 && V.Dofs.NumDofs == 5);
end

function testQueryValidation()
    mesh = twoTriangles();
    expectError(@() mesh.cellVertices([]), 'fem:InvalidIndex');
    expectError(@() mesh.cellVertices([1; 2]), 'fem:InvalidIndex');
    fe = fem.lagrange('triangle', 1);
    basis = fe.tabulate([0; 0]);
    near(reshape(basis.Values, 3, 1), [1; 0; 0], 0);
    direct = fem.DofMap.build(mesh, {fe}, [1; 1]);
    assert(direct.NumDofs == 4);
    assert(isempty(mesh.vertexIndex(uint64([]))));
    c = fem.ConstraintMap([1; 2], [0; 3], 1);
    near(c.expand(4), [4; 11], 0);
    invalid = @fem.ConstraintMap;
    invalidMatrix = sparse([1; nan(1)]);
    expectError(@() invalid(invalidMatrix, zeros(2, 1), 1), 'fem:InvalidConstraint');
    named = fem.Mesh(mesh.Coordinates, ["triangle"; "triangle"], mesh.CellPointers, mesh.CellVertices);
    assert(named.NumEdges == 5);
end

function testCurvedGeometry()
    mesh = twoTriangles();
    map = fem.GeometryMap(mesh, @curvedGeometry);
    points = [0.1, 0.2; 0.2, 0.3];
    g = map.evaluate(1, points);
    near(g.DetJ, (1 + 0.2*points(1, :)).', 1e-13);
    near(g.Points, [points(1, :); points(2, :).*(1+0.2*points(1, :))], 1e-13);
    fe = fem.lagrange('triangle', 1);
    basis = fem.mapBasis(fe, fe.tabulate(points, 1), g);
    % For N2=xi, grad_x N2 = [1,0] under this triangular map.
    near(reshape(basis.Gradients(2, 1, :, :), 2, 2), [1, 1; 0, 0], 1e-13);
end

function geometry = curvedGeometry(e, points) %#ok<INUSD> Cell-independent test mapping.
    geometry = struct();
    geometry.Points = [points(1, :); points(2, :).*(1+0.2*points(1, :))];
    geometry.Jacobians = zeros(2, 2, size(points, 2));
    for q = 1:size(points, 2)
        geometry.Jacobians(:, :, q) = [1, 0; 0.2*points(2, q), 1+0.2*points(1, q)];
    end
end

function testPermutationTransform()
    def = edgeDefinition();
    def.NumDofs = 6;
    def.ValueShape = 1;
    def.EntityDofs = {{[], [], []}, {[1; 2], [3; 4], [5; 6]}, {[]}};
    def.EntityKeys = {{'', '', ''}, {'paired-edge-values', 'paired-edge-values', 'paired-edge-values'}, {''}};
    def.MapType = 'identity';
    def.TabulateFcn = @(points, order) struct('Values', zeros(6, 1, size(points, 2)));
    def.TransformFcn = @pairedEdgeTransform;
    fe = fem.FiniteElement(def);
    mesh = twoTriangles();
    V = fem.FunctionSpace(mesh, {fe}, [1; 1]);
    assert(V.Dofs.NumDofs == 10);
    u = (1:10).';
    local1 = V.Dofs.cellTransform(1) * u(V.Dofs.cellDofs(1));
    local2 = V.Dofs.cellTransform(2) * u(V.Dofs.cellDofs(2));
    near(local1([3; 4]), local2([6; 5]), 0);
    plan = fem.AssemblyPlan(V.Dofs, V.Dofs);
    K = plan.assemble({eye(6), eye(6)});
    assert(issparse(K));
    near(full(K*u), full(K)*u, 0);
end

function T = pairedEdgeTransform(directions)
    T = eye(6);
    for edge = 1:3
        if directions(edge) < 0
            slots = (2*edge-1):(2*edge);
            T(slots, slots) = [0, 1; 1, 0];
        end
    end
    T = sparse(T);
end

function near(actual, expected, tolerance)
    assert(isequal(size(actual), size(expected)));
    if ~isempty(actual)
        assert(max(abs(actual(:) - expected(:))) <= tolerance);
    end
end

function expectError(callback, identifier)
    caught = false;
    try
        callback();
    catch exception
        caught = true;
        if ~strcmp(exception.identifier, identifier)
            disp(['Expected error: ', identifier]);
            disp(exception.identifier);
            disp(exception.message);
        end
        assert(strcmp(exception.identifier, identifier));
    end
    assert(caught);
end
