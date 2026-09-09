# Sparse CSC arithmetic and indexing compatibility

Status: implementation note, not an accepted RFC or protocol contract.

This note records locally authored black-box observations against MATLAB
R2022b and the corresponding OpenMat sparse behavior. It does not reproduce
MATLAB source, tests, documentation, or diagnostics.

## Measured storage and class behavior

- Sparse double or logical operands produce sparse double results for `+`,
  `-`, `.*`, `./`, and matrix `*`. Complex inputs produce complex double when
  the result has a nonzero imaginary component.
- Sparse plus or minus full storage produces full storage. Sparse times full
  storage remains sparse. Sparse divided element-by-element by full storage
  remains sparse, while full storage divided by sparse storage remains full.
- Sparse element-wise division remains a sparse container even when implicit
  zero divided by implicit zero requires an explicit `NaN` at every coordinate.
  Consequently a mathematically sparse operation may legitimately produce
  structurally full CSC storage; OpenMat checks that allocation rather than
  silently switching containers.
- Arithmetic zeros are removed from canonical CSC. `NaN` and infinities remain
  stored. COO duplicates are combined in input order before explicit-zero
  removal.
- Rectangular empty shapes remain two-dimensional sparse values. Empty
  sparse-sparse arithmetic preserves the input shape.

## Measured structural behavior

- Numeric and logical vector indexing preserves the index orientation for one
  subscript. Two-subscript indexing returns the Cartesian selection shape.
  Repeated indices remain repeated in the result.
- Indexed assignment remains sparse, removes entries assigned zero, uses the
  final source value for a repeated destination, and promotes real storage when
  a genuinely complex source is assigned. Logical sparse targets retain their
  logical class and convert assigned numeric values by zero/nonzero truth.
- Linear deletion preserves vector orientation; deletion from a nonvector
  matrix returns a row vector. Whole-row and whole-column deletion preserve the
  remaining matrix dimension.
- Transpose and conjugate transpose remain sparse. Two-dimensional reshape
  preserves linear column-major order. Horizontal and vertical concatenation
  remain sparse even when a full double block must be copied into CSC.

## Implemented boundaries and remaining wiring

`openmat-sparse` owns checked arithmetic, canonical matrix multiplication,
gather/assignment/deletion, transpose, two-dimensional reshape, and
concatenation algorithms. The interpreter dispatches arithmetic, indexing,
assignment/deletion, transpose, and matrix-literal concatenation without
changing the dynamic `Value` surface.

The existing `reshape`, `cat`, `horzcat`, and `vertcat` builtin entry points
still reject sparse values before reaching these owned algorithms. Wiring them
requires narrow edits in `crates/openmat-builtins/src/core_shape.rs` and
`crates/openmat-builtins/src/core_concat.rs`, which are outside this lane's
authorized paths. Sparse matrix multiplication with one full nonscalar operand
also remains outside this lane; sparse-sparse multiplication is implemented
with canonical CSC output.
