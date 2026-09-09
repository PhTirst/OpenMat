classdef QuadratureRule
%QUADRATURERULE Reference points (2 x N) and reference weights (N x 1).
% Weights do not include the physical-cell integration measure.
    properties (SetAccess = private)
        ReferenceCell
        Points
        Weights
        NumPoints
    end
    methods
        function obj = QuadratureRule(referenceCell, points, weights)
            if ~isa(referenceCell, 'fem.ReferenceCell')
                referenceCell = fem.ReferenceCell(referenceCell);
            end
            if ~isnumeric(points) || ~isreal(points) || size(points, 1) ~= 2 || ...
                    ~ismatrix(points) || any(~isfinite(points(:))) || ...
                    ~isnumeric(weights) || ~isreal(weights) || ...
                    numel(weights) ~= size(points, 2) || any(~isfinite(weights(:)))
                error('fem:InvalidQuadrature', 'Use finite 2-by-N points and N real weights.');
            end
            if any(points(:) < -1e-14) || any(points(:) > 1 + 1e-14) || ...
                    (strcmp(referenceCell.Name, 'triangle') && any(sum(points, 1) > 1 + 1e-14))
                error('fem:InvalidQuadrature', 'Quadrature points lie outside the reference cell.');
            end
            obj.ReferenceCell = referenceCell;
            obj.Points = double(points);
            obj.Weights = double(weights(:));
            obj.NumPoints = numel(weights);
        end
    end
end
