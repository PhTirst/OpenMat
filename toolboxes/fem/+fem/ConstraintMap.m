classdef ConstraintMap
%CONSTRAINTMAP Algebraic expansion u = C*q + b; DofMap remains unchanged.
% General maps must expose identity rows at independentDofs, with zero b
% there. This establishes full column rank without densifying a global C.
% Every affine constraint space can use its independent coefficients this way.
    properties (SetAccess = private)
        C
        b
        NumDofs
        NumFreeDofs
        IndependentDofs
    end
    methods
        function obj = ConstraintMap(C, b, independentDofs)
            if ~isnumeric(C) || ~ismatrix(C) || ~fem.internal.allFinite(C) || ...
                    ~isnumeric(b) || ~isequal(size(b), [size(C, 1), 1]) || ~fem.internal.allFinite(b)
                error('fem:InvalidConstraint', 'Supply a finite matrix C and one offset per full DOF.');
            end
            C = sparse(C);
            b = full(b);
            ids = fem.internal.indices(independentDofs, size(C, 1), 'IndependentDofs');
            nq = size(C, 2);
            if numel(ids) ~= nq || numel(unique(ids)) ~= nq || ...
                    ~isequal(C(ids, :), speye(nq)) || any(b(ids) ~= 0)
                error('fem:InvalidConstraint', 'Independent rows must contain an identity block and zero offsets.');
            end
            obj.C = sparse(C);
            obj.b = full(b);
            obj.NumDofs = size(C, 1);
            obj.NumFreeDofs = nq;
            obj.IndependentDofs = ids;
        end

        function u = expand(obj, q)
            if ~isnumeric(q) || ~isequal(size(q), [obj.NumFreeDofs, 1]) || ~fem.internal.allFinite(q)
                error('fem:InvalidConstraint', 'q must contain one finite value per independent DOF.');
            end
            u = obj.C * q + obj.b;
        end

        function [reducedK, reducedF] = reduce(obj, K, F, varargin)
            form = 'bilinear';
            if numel(varargin) > 1
                error('fem:InvalidForm', 'At most one form argument is accepted.');
            elseif numel(varargin) == 1
                form = varargin{1};
            end
            if ~isnumeric(K) || ~isequal(size(K), [obj.NumDofs, obj.NumDofs]) || ...
                    ~isnumeric(F) || ~isequal(size(F), [obj.NumDofs, 1]) || ...
                    ~fem.internal.allFinite(K) || ~fem.internal.allFinite(F)
                error('fem:InvalidConstraint', 'K and F must match the full DOF count and be finite.');
            end
            if strcmp(char(form), 'sesquilinear')
                left = obj.C';
            elseif strcmp(char(form), 'bilinear')
                left = obj.C.';
            else
                error('fem:InvalidForm', 'Choose bilinear or sesquilinear explicitly.');
            end
            reducedK = left * K * obj.C;
            reducedF = left * (F - K * obj.b);
        end
    end
    methods (Static)
        function obj = fixed(numDofs, fixedDofs, values)
        %FIXED Prescribed coefficient values, not automatic boundary sampling.
            if ~isnumeric(numDofs) || ~isscalar(numDofs) || ~isreal(numDofs) || ~isfinite(numDofs) || ...
                    numDofs < 0 || numDofs ~= floor(numDofs)
                error('fem:InvalidConstraint', 'NumDofs must be a nonnegative integer.');
            end
            fixedDofs = fem.internal.indices(fixedDofs, numDofs, 'FixedDofs');
            if numel(unique(fixedDofs)) ~= numel(fixedDofs)
                error('fem:DuplicateConstraint', 'Each fixed DOF must be specified exactly once.');
            end
            if ~isnumeric(values) || numel(values) ~= numel(fixedDofs) || any(~isfinite(values(:)))
                error('fem:InvalidConstraint', 'Supply one finite prescribed value per fixed DOF.');
            end
            free = setdiff((1:numDofs).', fixedDofs);
            C = sparse(free, (1:numel(free)).', ones(numel(free), 1), numDofs, numel(free));
            b = zeros(numDofs, 1);
            b(fixedDofs) = values(:);
            obj = fem.ConstraintMap(C, b, free);
        end
    end
end
