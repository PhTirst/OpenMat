function Validate_OpenMatMat73Stage2(outputDirectory)
%VALIDATE_OPENMATMAT73STAGE2 Validate only locally-authored OpenMat fixtures.

arguments
    outputDirectory (1, 1) string
end

loaded = load(fullfile(outputDirectory, 'openmat-stage2-sparse-v73.mat'));

assert(issparse(loaded.real));
assert(isequal(size(loaded.real), [3 5]));
assert(isequal(full(loaded.real), ...
    [1.5 0 0 0 0; 0 0 0 9 0; 0 -2 0 0 0]));

assert(issparse(loaded.complex));
assert(~isreal(loaded.complex));
assert(isequal(size(loaded.complex), [4 3]));
assert(loaded.complex(2, 1) == 3 + 4i);
assert(loaded.complex(4, 3) == -5 + 2i);

assert(issparse(loaded.logical));
assert(islogical(loaded.logical));
assert(isequal(full(loaded.logical), logical([1 0 1; 0 1 0])));

assert(issparse(loaded.empty_rows));
assert(isequal(size(loaded.empty_rows), [0 4]));
assert(issparse(loaded.empty_columns));
assert(isequal(size(loaded.empty_columns), [3 0]));
assert(issparse(loaded.allocated_empty));
assert(isequal(size(loaded.allocated_empty), [5 6]));
assert(nnz(loaded.allocated_empty) == 0);
assert(issparse(loaded.logical_empty));
assert(islogical(loaded.logical_empty));
assert(isequal(size(loaded.logical_empty), [2 0]));

assert(iscell(loaded.nested));
assert(isequal(size(loaded.nested), [2 1]));
assert(issparse(loaded.nested{1}));
assert(issparse(loaded.nested{2}));

assert(isstruct(loaded.nested_struct));
assert(issparse(loaded.nested_struct.left));
assert(issparse(loaded.nested_struct.right));
assert(islogical(loaded.nested_struct.right));
end
