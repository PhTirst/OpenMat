% Pure m-language assembly + nonzero Dirichlet constraints; no mesh IO/plotting.
% Run with toolboxes/fem on the search path. Exact solution: 1 + 2*x - 3*y.
coordinates = [0, 1, 1, 0, 0.5; 0, 0, 1, 1, 0.5];
options = struct();
options.VertexIds = uint64([10; 0; 42; 100; 700]);
options.CellIds = uint64([1000; 5; 99; 0]);
options.Connectivity = 'indices';
mesh = fem.Mesh(coordinates, {'triangle'; 'triangle'; 'triangle'; 'triangle'}, ...
    [1; 4; 7; 10; 13], [1; 2; 5; 2; 3; 5; 3; 4; 5; 4; 1; 5], options);
mesh = mesh.withSet('boundary', 1, mesh.boundaryEdges());

V = fem.FunctionSpace(mesh, {fem.lagrange('triangle', 1)}, ones(mesh.NumCells, 1));
geometry = fem.GeometryMap(mesh);
rule = fem.quadrature('triangle', 2);
plan = fem.AssemblyPlan(V.Dofs, V.Dofs);

% Capturing a named package handle also works on current OpenMat closures.
kernel = @femexamples.poissonCell;
K = plan.assemble(@(e) kernel(V, geometry, rule, e));
F = zeros(V.Dofs.NumDofs, 1);

% Vertex indices and global DOF numbers are NOT assumed equal.
exact = zeros(V.Dofs.NumDofs, 1);
for e = 1:mesh.NumCells
    vertexIndices = mesh.cellVertices(e);
    globalDofs = V.Dofs.cellDofs(e);
    exact(globalDofs) = (1 + 2*coordinates(1, vertexIndices) - 3*coordinates(2, vertexIndices)).';
end
boundaryDofs = V.edgeDofs(mesh.edgeSet('boundary'));
constraints = fem.ConstraintMap.fixed(V.Dofs.NumDofs, boundaryDofs, exact(boundaryDofs));
[Kr, Fr] = constraints.reduce(K, F);
u = constraints.expand(Kr \ Fr);

assert(V.Dofs.NumDofs == 5);
assert(constraints.NumFreeDofs == 1);
assert(max(abs(u - exact)) < 1e-12);
disp('DOFs: 5 full, 1 independent. Recovered solution matches 1 + 2*x - 3*y.');
disp(u);
