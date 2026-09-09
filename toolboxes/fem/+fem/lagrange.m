function element = lagrange(cellType, degree)
%LAGRANGE Scalar nodal P1/P2 triangles and Q1/Q2 quadrilaterals.
% Order: vertices, edge midpoints (if quadratic), then the Q2 cell centre.
    ref = fem.ReferenceCell(cellType);
    if ~isscalar(degree) || ~ismember(degree, [1, 2])
        error('fem:UnsupportedDegree', 'Initial Lagrange definitions support degrees 1 and 2.');
    end
    nv = ref.NumVertices;
    vertexDofs = cell(nv, 1);
    vertexKeys = cell(nv, 1);
    edgeDofs = cell(ref.NumEdges, 1);
    edgeKeys = cell(ref.NumEdges, 1);
    nd = nv;
    for j = 1:nv
        vertexDofs{j} = j;
        vertexKeys{j} = 'scalar-point-value';
    end
    for j = 1:ref.NumEdges
        edgeDofs{j} = [];
        edgeKeys{j} = '';
        if degree == 2
            nd = nd + 1;
            edgeDofs{j} = nd;
            edgeKeys{j} = 'scalar-edge-midpoint-value';
        end
    end
    interior = [];
    if degree == 2 && strcmp(ref.Name, 'quadrilateral')
        nd = nd + 1;
        interior = nd;
    end
    def = struct();
    def.ReferenceCell = ref;
    def.NumDofs = nd;
    def.ValueShape = 1;
    def.EntityDofs = {vertexDofs, edgeDofs, {interior}};
    def.EntityKeys = {vertexKeys, edgeKeys, {'scalar-cell-interior'}};
    def.MapType = 'identity';
    name = ref.Name;
    tabulator = @fem.internal.lagrangeBasis;
    def.TabulateFcn = @(points, order) tabulator(name, degree, points, order);
    def.TransformFcn = @(directions) speye(nd);
    element = fem.FiniteElement(def);
end
