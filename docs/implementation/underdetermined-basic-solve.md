# Underdetermined column-pivoted basic solve

This milestone provides the provider-independent binary32 and binary64
numerical core needed by the full-row-rank `m < n` portion of the MATLAB R2022b
left-division contract. It does not connect a language operator, bytecode
instruction, compiler path, or runtime dispatch.

## Public boundary

`openmat-linalg` exports:

- `solve_underdetermined_basic_f32`;
- `solve_underdetermined_basic_complex32`;
- `solve_underdetermined_basic_f64`; and
- `solve_underdetermined_basic_complex64`.

Each helper receives a `&dyn LinalgProvider`, immutable contiguous
column-major `A` and `B` arrays, and an optional shared `AtomicBool`
cancellation flag. `A` must be two-dimensional with shape `m x n` and `m < n`;
`B` must be two-dimensional with shape `m x nrhs`. The independently owned
result has shape `n x nrhs` and preserves column-major multiple-RHS layout.

The `m = 0` case returns exact zeros with shape `n x nrhs` without entering a
provider. A `0 x 0` coefficient matrix is not accepted because it does not
satisfy `m < n`.

## Deterministic basic-solution algorithm

The helper copies `A` into a private workspace and performs column-pivoted
modified Gram-Schmidt with one reorthogonalization pass. At step `k`, it
recomputes the residual Euclidean norm of every remaining column, selects the
largest, and normalizes it as the next orthogonal basis vector. Exact norm ties
are resolved by the smaller original zero-based column number, independent of
prior swaps. Complex projections use the conjugate inner product.

After `m` pivots have been selected, the helper copies those original columns,
in pivot order, into an `m x m` matrix. It calls the provider's matching
`solve_f32`, `solve_complex32`, `solve_f64`, or `solve_complex64` square-solve
contract for all right-hand sides at once. The square solution is scattered
back to the selected original rows of a zero-filled `n x nrhs` result.
Consequently every non-pivot unknown is exactly zero.

For the design discriminator

```text
A = [1 0 1; 0 1 1],  b = [1; 2],
```

the first pivot is column 3. The two remaining residual norms tie, so column 1
wins. Solving with columns `[3, 1]` and scattering gives `[-1; 0; 2]`, not the
provider `gels` minimum-norm value `[0; 1; 1]`.

## Rank threshold

Let `s` be the largest original column 2-norm and let
`d = max(m, n)`. Every selected residual pivot norm must be strictly greater
than

```text
tau = s * eps(request precision) * d.
```

This threshold scales with the matrix, is invariant under a common finite
nonzero rescaling of `A`, and follows the same precision-specific
dimension-scaling policy as the reference rectangular QR implementation. All
binary32 norms, projections, reorthogonalization, thresholds, and selected
square solves remain binary32. A pivot at or below `tau`
returns `LinalgError::RankDeficient` with provider label `openmat-linalg`, the
one-based failed pivot, and required rank `m`. No partial value is returned.

This is a numerical full-row-rank decision for the current well-conditioned
closure, not a MATLAB-compatible rank estimate or condition-warning policy.

## Checked boundaries and cancellation

Matrix rank and shared-row validation occur before numerical work. Conversions
from shape `u64` dimensions to host `usize`, checked buffer products, and
fallible reservations map to existing structured `LinalgError` variants. Both
inputs remain immutable and provider square-solve output is copied into newly
allocated result storage.

Cooperative cancellation is checked before and after allocations and the
provider call, and throughout pivot selection, norm computation,
orthogonalization, original-column copying, and result scattering. The flag is
also passed to the existing square-solve request. Native providers remain
synchronously non-interruptible while inside their LAPACK call, as specified by
the square-solve contract.

## Deliberate downstream work

This milestone closes only well-conditioned, full-row-rank real and complex
binary32/binary64 numerical solves. The following remain downstream:

- successful rank-deficient values paired with a normalized warning;
- singular and nearly singular successful-value warning policy;
- language `\`/`mldivide` dispatch and scalar/class promotion rules;
- language right division through the conjugate-transposed left-division
  relation; and
- observation-schema support for warnings attached to successful values.

Until those layers exist, `LinalgError::RankDeficient` is intentional and this
helper must not be presented as the complete MATLAB warning-bearing contract.
