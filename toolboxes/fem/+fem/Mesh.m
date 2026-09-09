classdef Mesh
%MESH Two-dimensional mixed-cell mesh with compact indices and external IDs.
% Coordinates: 2 x Nv. Connectivity and pointers are one based.
% options.Connectivity: 'indices' (default) or 'ids'. No base guessing.
% Value methods such as withSet return an updated mesh; retain that return.
    properties (SetAccess = private)
        Coordinates
        CellTypes
        CellPointers
        CellVertices
        VertexIds
        CellIds
        NumVertices
        NumCells
        NumEdges
        EdgeVertices
        CellEdgePointers
        CellEdgeIndices
        CellEdgeDirections
        EdgeCellPointers
        EdgeCells
        EdgeLocalNumbers
        Sets
    end
    methods
        function obj = Mesh(coordinates, types, pointers, vertices, options)
            if nargin < 5
                options = struct();
            end
            if ~isstruct(options) || ~isscalar(options)
                error('fem:InvalidMeshOptions', 'Mesh options must be a scalar struct.');
            end
            if ~isnumeric(coordinates) || ~isreal(coordinates) || ...
                    size(coordinates, 1) ~= 2 || ~ismatrix(coordinates) || ...
                    any(~isfinite(coordinates(:)))
                error('fem:InvalidCoordinates', 'Coordinates must be a finite real 2-by-N array.');
            end
            if ischar(types)
                types = {types};
            elseif isstring(types)
                names = types;
                types = cell(numel(names), 1);
                for k = 1:numel(names)
                    types{k} = char(names(k));
                end
            end
            if ~iscell(types)
                error('fem:InvalidCellTypes', 'CellTypes must be a cell or string array of names.');
            end
            types = types(:);
            nc = numel(types);
            nv = size(coordinates, 2);
            vertexIds = uint64((1:nv).');
            cellIds = uint64((1:nc).');
            if fem.internal.hasField(options, 'VertexIds')
                vertexIds = fem.internal.ids(options.VertexIds, nv);
            end
            if fem.internal.hasField(options, 'CellIds')
                cellIds = fem.internal.ids(options.CellIds, nc);
            end
            if numel(unique(vertexIds)) ~= nv || numel(unique(cellIds)) ~= nc
                error('fem:DuplicateId', 'Vertex and cell IDs must each be unique.');
            end
            mode = 'indices';
            if fem.internal.hasField(options, 'Connectivity')
                mode = char(options.Connectivity);
            end
            if strcmp(mode, 'ids')
                external = fem.internal.ids(vertices, numel(vertices));
                vertices = fem.internal.lookupIds(external, vertexIds);
            elseif ~strcmp(mode, 'indices')
                error('fem:InvalidConnectivityMode', 'Connectivity must be indices or ids.');
            end
            vertices = fem.internal.indices(vertices, nv, 'CellVertices');
            pointers = fem.internal.pointers(pointers, nc, numel(vertices));
            edgePointers = ones(nc + 1, 1);
            references = cell(nc, 1);
            for e = 1:nc
                ref = fem.ReferenceCell(types{e});
                references{e} = ref;
                types{e} = ref.Name;
                local = vertices(pointers(e):pointers(e + 1) - 1);
                if numel(local) ~= ref.NumVertices || numel(unique(local)) ~= numel(local)
                    error('fem:InvalidCell', 'A cell has the wrong number of distinct vertices.');
                end
                edgePointers(e + 1) = edgePointers(e) + ref.NumEdges;
            end
            nlocal = edgePointers(end) - 1;
            pairs = zeros(nlocal, 2);
            directions = ones(nlocal, 1);
            owners = zeros(nlocal, 1);
            localNumbers = zeros(nlocal, 1);
            for e = 1:nc
                ref = references{e};
                local = vertices(pointers(e):pointers(e + 1) - 1);
                for j = 1:ref.NumEdges
                    slot = edgePointers(e) + j - 1;
                    a = local(ref.Edges(1, j));
                    b = local(ref.Edges(2, j));
                    pairs(slot, :) = [min(a, b), max(a, b)];
                    directions(slot) = 2 * (a < b) - 1;
                    owners(slot) = e;
                    localNumbers(slot) = j;
                end
            end
            [edgeRows, edgeIds] = fem.internal.uniquePairs(pairs);
            ne = size(edgeRows, 1);
            [sortedEdges, permutation] = sort(edgeIds);
            adjacencyPointers = ones(ne + 1, 1);
            cursor = 1;
            for k = 1:ne
                while cursor <= nlocal && sortedEdges(cursor) == k
                    cursor = cursor + 1;
                end
                adjacencyPointers(k + 1) = cursor;
            end
            obj.Coordinates = double(coordinates);
            obj.CellTypes = types;
            obj.CellPointers = pointers;
            obj.CellVertices = vertices;
            obj.VertexIds = vertexIds;
            obj.CellIds = cellIds;
            obj.NumVertices = nv;
            obj.NumCells = nc;
            obj.NumEdges = ne;
            obj.EdgeVertices = edgeRows.';
            obj.CellEdgePointers = edgePointers;
            obj.CellEdgeIndices = edgeIds;
            obj.CellEdgeDirections = directions;
            obj.EdgeCellPointers = adjacencyPointers;
            obj.EdgeCells = owners(permutation);
            obj.EdgeLocalNumbers = localNumbers(permutation);
            obj.Sets = {};
        end

        function value = cellType(obj, e)
            e = fem.internal.entityIndex(e, obj.NumCells, 'Cell');
            value = obj.CellTypes{e};
        end

        function value = cellVertices(obj, e)
            e = fem.internal.entityIndex(e, obj.NumCells, 'Cell');
            value = obj.CellVertices(obj.CellPointers(e):obj.CellPointers(e + 1) - 1);
        end

        function value = cellVertexCount(obj, e)
            e = fem.internal.entityIndex(e, obj.NumCells, 'Cell');
            value = obj.CellPointers(e + 1) - obj.CellPointers(e);
        end

        function value = cellCoordinates(obj, e)
            value = obj.Coordinates(:, obj.cellVertices(e));
        end

        function [edges, directions] = cellEdges(obj, e)
            e = fem.internal.entityIndex(e, obj.NumCells, 'Cell');
            slots = obj.CellEdgePointers(e):obj.CellEdgePointers(e + 1) - 1;
            edges = obj.CellEdgeIndices(slots);
            directions = obj.CellEdgeDirections(slots);
        end

        function value = boundaryEdges(obj)
            value = find(diff(obj.EdgeCellPointers) == 1);
        end

        function [cells, localEdges] = edgeCells(obj, edge)
            edge = fem.internal.entityIndex(edge, obj.NumEdges, 'Edge');
            slots = obj.EdgeCellPointers(edge):obj.EdgeCellPointers(edge + 1) - 1;
            cells = obj.EdgeCells(slots);
            localEdges = obj.EdgeLocalNumbers(slots);
        end

        function value = vertexIndex(obj, ids)
            ids = fem.internal.ids(ids, numel(ids));
            value = fem.internal.lookupIds(ids, obj.VertexIds);
        end

        function value = cellIndex(obj, ids)
            ids = fem.internal.ids(ids, numel(ids));
            value = fem.internal.lookupIds(ids, obj.CellIds);
        end

        function value = vertexId(obj, indices)
            value = obj.VertexIds(fem.internal.indices(indices, obj.NumVertices, 'Vertex'));
        end

        function value = cellId(obj, indices)
            value = obj.CellIds(fem.internal.indices(indices, obj.NumCells, 'Cell'));
        end

        function obj = withSet(obj, name, dimension, indices)
            name = char(name);
            counts = [obj.NumVertices, obj.NumEdges, obj.NumCells];
            if ~isscalar(dimension) || ~ismember(dimension, [0, 1, 2]) || isempty(name)
                error('fem:InvalidSet', 'A set needs a name and entity dimension 0, 1 or 2.');
            end
            indices = unique(fem.internal.indices(indices, counts(dimension + 1), 'Set'));
            entry = struct('Name', name, 'Dimension', dimension, 'Indices', indices);
            sets = obj.Sets;
            for k = 1:numel(sets)
                old = sets{k};
                if strcmp(old.Name, name) && old.Dimension == dimension
                    sets{k} = entry;
                    obj.Sets = sets;
                    return;
                end
            end
            sets{end + 1} = entry;
            obj.Sets = sets;
        end

        function value = entitySet(obj, name, dimension)
            for k = 1:numel(obj.Sets)
                entry = obj.Sets{k};
                if strcmp(entry.Name, char(name)) && entry.Dimension == dimension
                    value = entry.Indices;
                    return;
                end
            end
            error('fem:UnknownSet', 'No set with this name and entity dimension.');
        end

        function value = vertexSet(obj, name)
            value = obj.entitySet(name, 0);
        end

        function value = edgeSet(obj, name)
            value = obj.entitySet(name, 1);
        end

        function value = cellSet(obj, name)
            value = obj.entitySet(name, 2);
        end
    end
end
