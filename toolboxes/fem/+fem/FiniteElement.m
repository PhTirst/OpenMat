classdef FiniteElement
%FINITEELEMENT Reference definition with m-language callbacks.
% EntityDofs{dimension+1}{localEntity} owns local coefficient indices.
% EntityKeys describes the ordered canonical DOF functionals on an entity.
% Equal keys assert equal functionals, including scaling and ordering.
% TabulateFcn(points, order) returns Values(ndof,ncomponent,npoint) and,
% for order 1, Gradients(ndof,ncomponent,2,npoint).
% TransformFcn(edgeDirections) returns T with uLocal = T*uCanonical.
    properties (SetAccess = private)
        ReferenceCell
        NumDofs
        ValueShape
        ValueSize
        EntityDofs
        EntityKeys
        MapType
        TabulateFcn
        TransformFcn
    end
    methods
        function obj = FiniteElement(def)
            required = {'ReferenceCell', 'NumDofs', 'ValueShape', 'EntityDofs', ...
                'EntityKeys', 'TabulateFcn', 'TransformFcn', 'MapType'};
            for k = 1:numel(required)
                if ~fem.internal.hasField(def, required{k})
                    error('fem:InvalidElement', 'Missing finite-element definition field.');
                end
            end
            ref = def.ReferenceCell;
            if ~isa(ref, 'fem.ReferenceCell')
                ref = fem.ReferenceCell(ref);
            end
            nd = fem.internal.indices(def.NumDofs, inf, 'NumDofs');
            shape = fem.internal.indices(def.ValueShape, inf, 'ValueShape');
            if numel(nd) ~= 1 || isempty(shape) || ...
                    ~isa(def.TabulateFcn, 'function_handle') || ...
                    ~isa(def.TransformFcn, 'function_handle')
                error('fem:InvalidElement', 'Invalid sizes or finite-element callbacks.');
            end
            owned = def.EntityDofs;
            keys = def.EntityKeys;
            counts = [ref.NumVertices, ref.NumEdges, 1];
            if ~iscell(owned) || ~iscell(keys) || numel(owned) ~= 3 || numel(keys) ~= 3
                error('fem:InvalidElement', 'Entity descriptions need dimensions 0, 1 and 2.');
            end
            seen = zeros(nd, 1);
            for d = 1:3
                groups = owned{d};
                labels = keys{d};
                if ~iscell(groups) || ~iscell(labels) || ...
                        numel(groups) ~= counts(d) || numel(labels) ~= counts(d)
                    error('fem:InvalidElement', 'Entity layout does not match the reference cell.');
                end
                for j = 1:counts(d)
                    local = fem.internal.indices(groups{j}, nd, 'EntityDofs');
                    if numel(unique(local)) ~= numel(local) || any(seen(local) ~= 0)
                        error('fem:InvalidElement', 'Every local DOF must have exactly one owner.');
                    end
                    label = char(labels{j});
                    if ~isempty(local) && isempty(label)
                        error('fem:InvalidElement', 'Nonempty entities require a functional identity key.');
                    end
                    seen(local) = 1;
                    groups{j} = local;
                    labels{j} = label;
                end
                owned{d} = groups;
                keys{d} = labels;
            end
            if any(seen ~= 1)
                error('fem:InvalidElement', 'Every local DOF must have exactly one owner.');
            end
            map = char(def.MapType);
            if ~strcmp(map, 'identity') && ~strcmp(map, 'covariant') && ~strcmp(map, 'contravariant')
                error('fem:InvalidElement', 'Unknown basis mapping rule.');
            end
            obj.ReferenceCell = ref;
            obj.NumDofs = nd;
            obj.ValueShape = shape.';
            obj.ValueSize = prod(shape);
            obj.EntityDofs = owned;
            obj.EntityKeys = keys;
            obj.MapType = map;
            obj.TabulateFcn = def.TabulateFcn;
            obj.TransformFcn = def.TransformFcn;
        end

        function values = entityDofs(obj, dimension, entity)
            groups = obj.EntityDofs{dimension + 1};
            values = groups{entity};
        end

        function values = entityClosureDofs(obj, dimension, entity)
            values = obj.entityDofs(dimension, entity);
            if dimension == 1
                vertices = obj.ReferenceCell.Edges(:, entity);
                values = [obj.entityDofs(0, vertices(1)); ...
                    obj.entityDofs(0, vertices(2)); values];
            elseif dimension == 2
                values = (1:obj.NumDofs).';
            end
        end

        function basis = tabulate(obj, points, varargin)
            order = 0;
            if numel(varargin) > 1
                error('fem:InvalidTabulation', 'At most one derivative order is accepted.');
            elseif numel(varargin) == 1
                order = varargin{1};
            end
            if ~isscalar(order) || ~ismember(order, [0, 1]) || ...
                    ~isnumeric(points) || ~isreal(points) || size(points, 1) ~= 2 || ...
                    ~ismatrix(points) || any(~isfinite(points(:)))
                error('fem:InvalidTabulation', 'Use finite 2-by-N points and derivative order 0 or 1.');
            end
            callback = obj.TabulateFcn;
            basis = callback(points, order);
            nq = size(points, 2);
            if ~isstruct(basis) || ~fem.internal.hasField(basis, 'Values')
                error('fem:InvalidTabulation', 'Tabulation must return a Values array.');
            end
            values = basis.Values;
            if ~isnumeric(values) || size(values, 1) ~= obj.NumDofs || ...
                    size(values, 2) ~= obj.ValueSize || size(values, 3) ~= nq || ...
                    numel(values) ~= obj.NumDofs * obj.ValueSize * nq || ...
                    any(~isfinite(values(:)))
                error('fem:InvalidTabulation', 'Unexpected basis Values shape or entries.');
            end
            if order == 1
                if ~fem.internal.hasField(basis, 'Gradients')
                    error('fem:InvalidTabulation', 'First derivatives were requested.');
                end
                gradients = basis.Gradients;
                if ~isnumeric(gradients) || size(gradients, 1) ~= obj.NumDofs || ...
                        size(gradients, 2) ~= obj.ValueSize || size(gradients, 3) ~= 2 || ...
                        size(gradients, 4) ~= nq || ...
                        numel(gradients) ~= obj.NumDofs * obj.ValueSize * 2 * nq || ...
                        any(~isfinite(gradients(:)))
                    error('fem:InvalidTabulation', 'Unexpected basis Gradients shape or entries.');
                end
            end
        end

        function T = dofTransform(obj, directions)
            if numel(directions) ~= obj.ReferenceCell.NumEdges || ...
                    any(~ismember(directions(:), [-1, 1]))
                error('fem:InvalidTransform', 'Invalid local edge directions.');
            end
            callback = obj.TransformFcn;
            T = callback(directions(:));
            if ~isnumeric(T) || ~isequal(size(T), [obj.NumDofs, obj.NumDofs]) || ...
                    ~fem.internal.allFinite(T) || rank(full(T)) ~= obj.NumDofs
                error('fem:InvalidTransform', 'T must be a finite invertible local square matrix.');
            end
            T = sparse(T);
        end
    end
end
