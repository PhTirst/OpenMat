classdef GeometryMap
%GEOMETRYMAP Geometry is independent of the field finite-element assignment.
% Default: affine triangles / bilinear quads from Mesh vertex coordinates.
% Optional callback(e, xi) supplies Points(2,N) and Jacobians(2,2,N), e.g.
% from separate high-order geometry nodes captured by an m function handle.
    properties (SetAccess = private)
        Mesh
        EvaluateFcn
    end
    methods
        function obj = GeometryMap(mesh, callback)
            if nargin < 2
                callback = [];
            end
            if ~isa(mesh, 'fem.Mesh') || ...
                    (~isa(callback, 'function_handle') && ~(isnumeric(callback) && isempty(callback)))
                error('fem:InvalidGeometry', 'Supply a Mesh and optionally a geometry callback.');
            end
            obj.Mesh = mesh;
            obj.EvaluateFcn = callback;
            if ~isa(callback, 'function_handle')
                for e = 1:mesh.NumCells
                    ref = fem.ReferenceCell(mesh.cellType(e));
                    check = obj.evaluate(e, ref.Vertices);
                    if any(sign(check.DetJ) ~= sign(check.DetJ(1)))
                        error('fem:FoldedCell', 'A bilinear cell folds or has inconsistent Jacobian signs.');
                    end
                end
            end
        end

        function geometry = evaluate(obj, e, points)
            e = fem.internal.indices(e, obj.Mesh.NumCells, 'Cell');
            if numel(e) ~= 1 || ~isnumeric(points) || ~isreal(points) || ...
                    size(points, 1) ~= 2 || ~ismatrix(points) || any(~isfinite(points(:)))
                error('fem:InvalidGeometry', 'Use one cell and finite 2-by-N reference points.');
            end
            nq = size(points, 2);
            if ~isa(obj.EvaluateFcn, 'function_handle')
                coordinates = obj.Mesh.cellCoordinates(e);
                basis = fem.internal.lagrangeBasis(obj.Mesh.cellType(e), 1, points, 1);
                values = reshape(basis.Values, [size(coordinates, 2), nq]);
                geometry = struct();
                geometry.Points = coordinates * values;
                geometry.Jacobians = zeros(2, 2, nq);
                % Subtract one vertex before differentiation to avoid loss of
                % accuracy caused solely by a large coordinate translation.
                relative = coordinates - coordinates(:, 1);
                for q = 1:nq
                    gradient = reshape(basis.Gradients(:, :, :, q), [size(coordinates, 2), 2]);
                    geometry.Jacobians(:, :, q) = relative * gradient;
                end
            else
                callback = obj.EvaluateFcn;
                geometry = callback(e, points);
            end
            if ~isstruct(geometry) || ~fem.internal.hasField(geometry, 'Points') || ...
                    ~fem.internal.hasField(geometry, 'Jacobians')
                error('fem:InvalidGeometry', 'Geometry callback must return Points and Jacobians.');
            end
            x = geometry.Points;
            jacobians = geometry.Jacobians;
            if ~isnumeric(x) || ~isreal(x) || ~isequal(size(x), [2, nq]) || ...
                    any(~isfinite(x(:))) || ~isnumeric(jacobians) || ~isreal(jacobians) || ...
                    size(jacobians, 1) ~= 2 || size(jacobians, 2) ~= 2 || ...
                    size(jacobians, 3) ~= nq || numel(jacobians) ~= 4*nq || ...
                    any(~isfinite(jacobians(:)))
                error('fem:InvalidGeometry', 'Invalid geometry output arrays.');
            end
            determinants = zeros(nq, 1);
            for q = 1:nq
                J = jacobians(:, :, q);
                scale = max(abs(J(:)));
                if scale == 0
                    error('fem:DegenerateCell', 'A geometry Jacobian is singular.');
                end
                scaledDet = det(J / scale);
                determinants(q) = scaledDet * scale * scale;
                if abs(scaledDet) <= 64*eps || ~isfinite(determinants(q)) || determinants(q) == 0
                    error('fem:DegenerateCell', 'A geometry Jacobian is singular or numerically unresolved.');
                end
            end
            if nq > 0 && any(sign(determinants) ~= sign(determinants(1)))
                error('fem:FoldedCell', 'Geometry Jacobian signs differ at evaluation points.');
            end
            geometry.DetJ = determinants;
            geometry.Measure = abs(determinants);
        end
    end
end
