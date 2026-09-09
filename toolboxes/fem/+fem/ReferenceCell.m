classdef ReferenceCell
%REFERENCECELL Local topology, vertex order and reference coordinates.
% Edges are directed columns. Mesh cell vertex order must follow this order.
    properties (SetAccess = private)
        Name
        Dimension
        Vertices
        Edges
        NumVertices
        NumEdges
    end
    methods
        function obj = ReferenceCell(name)
            name = char(name);
            switch name
                case 'triangle'
                    vertices = [0, 1, 0; 0, 0, 1];
                    edges = [1, 2, 3; 2, 3, 1];
                case 'quadrilateral'
                    vertices = [0, 1, 1, 0; 0, 0, 1, 1];
                    edges = [1, 2, 3, 4; 2, 3, 4, 1];
                otherwise
                    error('fem:UnsupportedCell', 'Supported cells are triangle and quadrilateral.');
            end
            obj.Name = name;
            obj.Dimension = 2;
            obj.Vertices = vertices;
            obj.Edges = edges;
            obj.NumVertices = size(vertices, 2);
            obj.NumEdges = size(edges, 2);
        end
    end
end
