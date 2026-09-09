# Matrix-division runtime closure

Status: implemented for dense real and complex `double` and `single` matrices.
The bytecode/compiler operator contract remains version 16 and is unchanged by
this runtime milestone.

## Language dispatch

The interpreter preserves the source operand order supplied by bytecode. A
non-scalar coefficient in `A\B` enters the numerical bridge, while a non-scalar
right operand in `L/R` enters the right-division bridge. Scalar and `1x1`
operands retain the bytecode-v16 scalar boundary. `ElementDivide` and
`ElementLeftDivide` stay on the element-wise path and never enter a solver.

For `A` with shape `m x n` and `B` with shape `m x nrhs`, the runtime validates
the shared row count before provider dispatch and returns an independently
owned `n x nrhs` result:

| Shape | Runtime route | Meaning |
| --- | --- | --- |
| `m == n` | `solve_f64`/`solve_complex64` or `solve_f32`/`solve_complex32` | square general solve |
| `m > n` | matching `solve_rectangular_*` method | overdetermined least squares |
| `m < n` | matching `solve_underdetermined_basic_*` helper | deterministic column-pivoted basic solution |

The underdetermined route intentionally does not use the provider's
minimum-norm rectangular mode. Its helper chooses independent pivot columns,
solves the selected square system through the same injected provider, and
leaves non-pivot variables exactly zero.

Mixed real/complex operands are promoted to complex within their existing
binary64 or binary32 precision. Inputs remain borrowed and immutable, storage
remains column-major, and the result does not share storage with either input.

## Right division

Matrix right division is implemented only through

```text
L / R = (R' \ L')'
```

where every apostrophe is a conjugate transpose. The runtime first validates
that `L` and `R` have equal column counts, then constructs independent
transposed storage, applies the same left-division shape dispatch, and
conjugate-transposes the result. Thus `L` shaped `p x n` divided by `R` shaped
`m x n` produces `p x m`; complex values are not treated with an ordinary
transpose.

## Empty systems

Compatible requests with zero coefficient rows, zero coefficient columns, or
zero right-hand-side columns are completed before provider dispatch. The
result keeps the exact `n x nrhs` left-division shape and real/complex storage
kind. A zero-row underdetermined system returns an all-zero result. Right
division inherits these rules after its conjugate transposes, including a
non-empty all-zero `p x m` result when the shared original column count is zero.

These shortcuts cover the six measured R2022b empty-shape cases without asking
a native provider to factor a meaningless empty system.

## Native single dispatch

When both operands are `single`, the runtime uses the provider's native
binary32 methods. Real systems remain f32. A real/complex pair promotes only
the real input to `Complex32`, and complex systems remain `Complex32` through
factorization and result construction. Square, overdetermined, and
underdetermined-basic routes use their `f32`/`complex32` provider methods or
provider-neutral helpers directly; no input, workspace, intermediate, or
result is converted through binary64.

The returned value is a binary32 `Value::Array` with the exact result shape and
input-derived real/complex storage kind. Mixed single/double matrix division is
not yet assigned a result-class contract and is rejected as
`RuntimeLinalgError::MixedPrecisionMatrixDivision`; element-wise mixed
single/double arithmetic retains its existing behavior.

## Cancellation and errors

`CancellationToken` exposes only a crate-internal borrow of its shared
`AtomicBool`. The runtime attaches that exact flag to square and rectangular
provider requests and passes it to the underdetermined helper. Empty-result,
same-precision promotion, and transpose loops check the same flag before
allocation, during traversal, and before returning. A provider or helper
`LinalgError::Cancelled` continues to map to `RuntimeErrorKind::Cancelled`.

Matrix-rank, dimension, allocation, singularity, rank-deficiency, invalid
provider argument, and provider-failure errors remain nested as
`RuntimeLinalgError::Linalg`; no display string is parsed. Unsupported numeric
classes and mixed precision use dedicated `RuntimeLinalgError` variants.

The remaining R2022b gaps are the warning-bearing successful results for
rank-deficient, singular, and nearly singular systems described by
`docs/design/matrix-division-r2022b.md`. The current provider and observation
contracts have no warning channel or compatible successful-value policy for
those cases, so this runtime milestone preserves the structured provider errors
instead of claiming that boundary.
