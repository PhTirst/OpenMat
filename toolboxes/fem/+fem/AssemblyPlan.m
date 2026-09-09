classdef AssemblyPlan
%ASSEMBLYPLAN Reusable COO coordinates, not writable final-CSC slot offsets.
% Local matrices/vectors must be in LOCAL basis order, without T applied.
% form is 'bilinear' (default) or explicitly 'sesquilinear'.
% Test/trial cells must be paired in the same order by the caller.
    properties (SetAccess = private)
        TestDofs
        TrialDofs
        Form
        ValuePointers
        Rows
        Columns
    end
    methods
        function obj = AssemblyPlan(testDofs, trialDofs, form)
            if nargin < 3
                form = 'bilinear';
            end
            if ~isa(testDofs, 'fem.DofMap') || ~isa(trialDofs, 'fem.DofMap') || ...
                    testDofs.NumCells ~= trialDofs.NumCells
                error('fem:InvalidAssembly', 'Supply two DofMaps with paired cell lists.');
            end
            form = char(form);
            if ~strcmp(form, 'bilinear') && ~strcmp(form, 'sesquilinear')
                error('fem:InvalidForm', 'Choose bilinear or sesquilinear explicitly.');
            end
            pointers = ones(testDofs.NumCells + 1, 1);
            for e = 1:testDofs.NumCells
                pointers(e + 1) = pointers(e) + testDofs.cellDofCount(e) * trialDofs.cellDofCount(e);
            end
            rows = zeros(pointers(end) - 1, 1);
            columns = rows;
            for e = 1:testDofs.NumCells
                gtest = testDofs.cellDofs(e);
                gtrial = trialDofs.cellDofs(e);
                [i, j] = ndgrid(gtest, gtrial);
                slots = pointers(e):pointers(e + 1) - 1;
                rows(slots) = i(:);
                columns(slots) = j(:);
            end
            obj.TestDofs = testDofs;
            obj.TrialDofs = trialDofs;
            obj.Form = form;
            obj.ValuePointers = pointers;
            obj.Rows = rows;
            obj.Columns = columns;
        end

        function matrix = assemble(obj, localMatrices)
        % Pass a cell array or callback(e), allowing one-cell-at-a-time kernels.
            callback = isa(localMatrices, 'function_handle');
            if ~callback && (~iscell(localMatrices) || numel(localMatrices) ~= obj.TestDofs.NumCells)
                error('fem:InvalidAssembly', 'Supply one matrix per cell or a callback.');
            end
            values = zeros(numel(obj.Rows), 1);
            for e = 1:obj.TestDofs.NumCells
                if callback
                    local = localMatrices(e);
                else
                    local = localMatrices{e};
                end
                expected = [obj.TestDofs.cellDofCount(e), obj.TrialDofs.cellDofCount(e)];
                if ~isnumeric(local) || ~isequal(size(local), expected) || ~fem.internal.allFinite(local)
                    error('fem:InvalidLocalMatrix', 'Local matrix has an invalid shape or entries.');
                end
                left = obj.TestDofs.cellTransform(e);
                right = obj.TrialDofs.cellTransform(e);
                if strcmp(obj.Form, 'sesquilinear')
                    local = left' * local * right;
                else
                    local = left.' * local * right;
                end
                slots = obj.ValuePointers(e):obj.ValuePointers(e + 1) - 1;
                values(slots) = local(:);
            end
            matrix = sparse(obj.Rows, obj.Columns, values, obj.TestDofs.NumDofs, obj.TrialDofs.NumDofs);
        end

        function vector = assembleVector(obj, localVectors)
            callback = isa(localVectors, 'function_handle');
            if ~callback && (~iscell(localVectors) || numel(localVectors) ~= obj.TestDofs.NumCells)
                error('fem:InvalidAssembly', 'Supply one vector per cell or a callback.');
            end
            values = zeros(numel(obj.TestDofs.ElementDofs), 1);
            for e = 1:obj.TestDofs.NumCells
                if callback
                    local = localVectors(e);
                else
                    local = localVectors{e};
                end
                if ~isnumeric(local) || ~isequal(size(local), [obj.TestDofs.cellDofCount(e), 1]) || ...
                        ~fem.internal.allFinite(local)
                    error('fem:InvalidLocalVector', 'Local vector has an invalid shape or entries.');
                end
                T = obj.TestDofs.cellTransform(e);
                if strcmp(obj.Form, 'sesquilinear')
                    local = T' * local;
                else
                    local = T.' * local;
                end
                slots = obj.TestDofs.ElementPointers(e):obj.TestDofs.ElementPointers(e + 1) - 1;
                values(slots) = local;
            end
            vector = full(sparse(obj.TestDofs.ElementDofs, ones(numel(values), 1), values, ...
                obj.TestDofs.NumDofs, 1));
        end
    end
end
