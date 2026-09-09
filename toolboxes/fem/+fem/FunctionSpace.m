classdef FunctionSpace
%FUNCTIONSPACE Per-cell finite-element assignment on one mesh snapshot.
% Fourth argument: sharing mode, or an explicit DofMap (e.g. hp constraints).
    properties (SetAccess = private)
        Mesh
        Elements
        CellElementIds
        Dofs
    end
    methods
        function obj = FunctionSpace(mesh, elements, ids, mapping)
            if nargin < 4
                mapping = 'conforming';
            end
            if ~isa(mesh, 'fem.Mesh') || ~iscell(elements)
                error('fem:InvalidSpace', 'Supply a Mesh and a cell array of finite-element definitions.');
            end
            ids = fem.internal.indices(ids, numel(elements), 'CellElementIds');
            if numel(ids) ~= mesh.NumCells
                error('fem:InvalidAssignment', 'Assign exactly one definition to each mesh cell.');
            end
            for e = 1:mesh.NumCells
                fe = elements{ids(e)};
                if ~isa(fe, 'fem.FiniteElement') || ~strcmp(fe.ReferenceCell.Name, mesh.cellType(e))
                    error('fem:CellMismatch', 'Finite element and mesh cell shapes differ.');
                end
            end
            if ~isa(mapping, 'fem.DofMap')
                mapping = fem.DofMap.build(mesh, elements, ids, mapping);
            end
            if mapping.NumCells ~= mesh.NumCells
                error('fem:InvalidDofMap', 'DofMap and mesh cell counts differ.');
            end
            for e = 1:mesh.NumCells
                fe = elements{ids(e)};
                if mapping.cellDofCount(e) ~= fe.NumDofs
                    error('fem:InvalidDofMap', 'DofMap and finite-element local sizes differ.');
                end
            end
            obj.Mesh = mesh;
            obj.Elements = elements;
            obj.CellElementIds = ids;
            obj.Dofs = mapping;
        end

        function element = element(obj, e)
            e = fem.internal.entityIndex(e, obj.Mesh.NumCells, 'Cell');
            element = obj.Elements{obj.CellElementIds(e)};
        end

        function values = edgeDofs(obj, edges)
        %EDGEDOFS All canonical DOFs coupled to the closure of these edges.
        % Values of moment DOFs must still be supplied by the user, not sampled.
            edges = fem.internal.indices(edges, obj.Mesh.NumEdges, 'Edge');
            values = [];
            for k = 1:numel(edges)
                [cells, locals] = obj.Mesh.edgeCells(edges(k));
                for j = 1:numel(cells)
                    fe = obj.element(cells(j));
                    slots = fe.entityClosureDofs(1, locals(j));
                    T = obj.Dofs.cellTransform(cells(j));
                    columns = any(full(T(slots, :)) ~= 0, 1);
                    globalIds = obj.Dofs.cellDofs(cells(j));
                    values = [values; globalIds(columns)]; %#ok<AGROW> Boundary query, not assembly.
                end
            end
            values = unique(values);
        end
    end
end
