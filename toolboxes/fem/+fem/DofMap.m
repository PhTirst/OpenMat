classdef DofMap
%DOFMAP Complete DOF numbering, independent of algebraic constraints.
% ElementDofs are in canonical entity order; T maps them into local basis order.
% Use build for compatible entity functionals, or the constructor for manual maps.
    properties (SetAccess = private)
        ElementPointers
        ElementDofs
        NumDofs
        NumCells
        Transforms
        TransformIds
    end
    methods
        function obj = DofMap(pointers, dofs, numDofs, transforms, transformIds)
            if ~isnumeric(numDofs) || ~isscalar(numDofs) || ~isreal(numDofs) || ...
                    ~isfinite(numDofs) || numDofs < 0 || numDofs ~= floor(numDofs)
                error('fem:InvalidDofMap', 'NumDofs must be a nonnegative integer.');
            end
            nc = numel(transformIds);
            dofs = fem.internal.indices(dofs, numDofs, 'ElementDofs');
            pointers = fem.internal.pointers(pointers, nc, numel(dofs));
            if ~iscell(transforms)
                error('fem:InvalidDofMap', 'Transforms must be a cell array of CSC matrices.');
            end
            transformIds = fem.internal.indices(transformIds, numel(transforms), 'TransformIds');
            for k = 1:numel(transforms)
                T = transforms{k};
                if ~isnumeric(T) || ~ismatrix(T) || size(T, 1) ~= size(T, 2) || ...
                        ~fem.internal.allFinite(T) || rank(full(T)) ~= size(T, 1)
                    error('fem:InvalidTransform', 'Each T must be a finite invertible square matrix.');
                end
                transforms{k} = sparse(T);
            end
            for e = 1:nc
                count = pointers(e + 1) - pointers(e);
                T = transforms{transformIds(e)};
                if count ~= size(T, 1)
                    error('fem:InvalidTransform', 'Transform size differs from the local DOF count.');
                end
            end
            obj.ElementPointers = pointers;
            obj.ElementDofs = dofs;
            obj.NumDofs = double(numDofs);
            obj.NumCells = nc;
            obj.Transforms = transforms;
            obj.TransformIds = transformIds;
        end

        function values = cellDofs(obj, e)
            e = fem.internal.entityIndex(e, obj.NumCells, 'Cell');
            values = obj.ElementDofs(obj.ElementPointers(e):obj.ElementPointers(e + 1) - 1);
        end

        function value = cellDofCount(obj, e)
            e = fem.internal.entityIndex(e, obj.NumCells, 'Cell');
            value = obj.ElementPointers(e + 1) - obj.ElementPointers(e);
        end

        function T = cellTransform(obj, e)
            e = fem.internal.entityIndex(e, obj.NumCells, 'Cell');
            T = obj.Transforms{obj.TransformIds(e)};
        end
    end
    methods (Static)
        function obj = build(mesh, elements, elementIds, varargin)
            sharing = 'conforming';
            if numel(varargin) > 1
                error('fem:InvalidSharing', 'At most one sharing mode is accepted.');
            elseif numel(varargin) == 1
                sharing = varargin{1};
            end
            if ~isa(mesh, 'fem.Mesh') || ~iscell(elements)
                error('fem:InvalidSpace', 'Supply a Mesh and a cell array of finite-element definitions.');
            end
            sharing = char(sharing);
            if ~strcmp(sharing, 'conforming') && ~strcmp(sharing, 'discontinuous')
                error('fem:InvalidSharing', 'Sharing must be conforming or discontinuous.');
            end
            elementIds = fem.internal.indices(elementIds, numel(elements), 'CellElementIds');
            if numel(elementIds) ~= mesh.NumCells
                error('fem:InvalidAssignment', 'Assign exactly one definition to each mesh cell.');
            end
            pointers = ones(mesh.NumCells + 1, 1);
            for e = 1:mesh.NumCells
                fe = elements{elementIds(e)};
                if ~isa(fe, 'fem.FiniteElement') || ~strcmp(fe.ReferenceCell.Name, mesh.cellType(e))
                    error('fem:CellMismatch', 'Finite element and mesh cell shapes differ.');
                end
                pointers(e + 1) = pointers(e) + fe.NumDofs;
            end
            dofs = zeros(pointers(end) - 1, 1);
            transforms = {};
            transformIds = zeros(mesh.NumCells, 1);
            directionCache = cell(numel(elements), 1);
            transformCache = cell(numel(elements), 1);
            vertexRecords = cell(mesh.NumVertices, 1);
            edgeRecords = cell(mesh.NumEdges, 1);
            next = 1;
            for e = 1:mesh.NumCells
                fe = elements{elementIds(e)};
                vertices = mesh.cellVertices(e);
                [edges, directions] = mesh.cellEdges(e);
                local = zeros(fe.NumDofs, 1);
                if strcmp(sharing, 'discontinuous')
                    local = (next:next + fe.NumDofs - 1).';
                    next = next + fe.NumDofs;
                else
                    entities = {vertices, edges, e};
                    for d = 1:3
                        owned = fe.EntityDofs{d};
                        keys = fe.EntityKeys{d};
                        ids = entities{d};
                        for j = 1:numel(ids)
                            slots = owned{j};
                            key = keys{j};
                            record = [];
                            if d == 1
                                record = vertexRecords{ids(j)};
                            elseif d == 2
                                record = edgeRecords{ids(j)};
                            end
                            if isempty(record)
                                globalIds = (next:next + numel(slots) - 1).';
                                next = next + numel(slots);
                                record = struct('Key', key, 'Dofs', globalIds);
                                if d == 1
                                    vertexRecords{ids(j)} = record;
                                elseif d == 2
                                    edgeRecords{ids(j)} = record;
                                end
                            elseif ~strcmp(record.Key, key) || numel(record.Dofs) ~= numel(slots)
                                error('fem:IncompatibleTrace', ...
                                    'Shared entity functionals differ; supply an explicit DofMap and constraints or use discontinuous sharing.');
                            end
                            assigned = record.Dofs;
                            local(slots) = double(assigned(:));
                        end
                    end
                end
                dofs(pointers(e):pointers(e + 1) - 1) = local;
                id = 0;
                patterns = directionCache{elementIds(e)};
                cachedIds = transformCache{elementIds(e)};
                for k = 1:numel(cachedIds)
                    if isequal(directions, patterns(:, k))
                        id = cachedIds(k);
                        break;
                    end
                end
                if id == 0
                    T = fe.dofTransform(directions);
                    for k = 1:numel(transforms)
                        if isequal(T, transforms{k})
                            id = k;
                            break;
                        end
                    end
                    if id == 0
                        transforms{end + 1} = T; %#ok<AGROW> Small encountered-transform table.
                        id = numel(transforms);
                    end
                    patterns = [patterns, directions]; %#ok<AGROW> At most 2^NumEdges patterns.
                    cachedIds = [cachedIds; id]; %#ok<AGROW>
                    directionCache{elementIds(e)} = patterns;
                    transformCache{elementIds(e)} = cachedIds;
                end
                transformIds(e) = id;
            end
            obj = fem.DofMap(pointers, dofs, next - 1, transforms, transformIds);
        end
    end
end
