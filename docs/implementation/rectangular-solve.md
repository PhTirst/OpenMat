# Rectangular dense solve provider contract

This milestone provides a provider-only contract for dense binary32 and
binary64 systems. It does not wire `\`, `mldivide`, or any other language
operation. Both operands are immutable, contiguous column-major matrices:

```text
A: m x n
B: m x nrhs
X: n x nrhs
```

`RectangularSolveKind` records the shape-selected meaning instead of collapsing
three different problems into a generic “solve” label:

| Shape | Kind | Full-rank result |
| --- | --- | --- |
| `m == n` | `Square` | unique solution of `A X = B` |
| `m > n` | `OverdeterminedLeastSquares` | minimizes each `||A x - b||_2` |
| `m < n` | `UnderdeterminedMinimumNorm` | minimum-`||x||_2` solution of `A x = b` |

The result always has `n` rows. The square case intentionally remains in this
contract so callers can validate and classify one request before choosing a
provider method, but it is independent of the existing LU/`gesv` square-solve
API. Existing `solve_f64`, `solve_complex64`, GEMM, dot, and norm methods retain
their behavior and signatures.

## Public boundary

`openmat-linalg` exposes:

- `RectangularSolveRequest`, including the shared atomic cancellation flag;
- `validate_rectangular_solve` and `RectangularSolveDimensions`;
- `solve_rectangular_f32` and `solve_rectangular_complex32`;
- `solve_rectangular_f64` and `solve_rectangular_complex64`;
- matching `LinalgProvider` methods; and
- `Lp64RectangularSolveDimensions` for checked native ABI conversion.

Validation requires both operands to be matrices and requires the row count of
`B` to equal `m`. Providers must not mutate or return shared storage from `A` or
`B`. A zero required rank is well-defined: `0 x n` with `n > 0` returns the
all-zero minimum-norm `n x nrhs` result; `m x 0` and `0 x 0` return independently
owned empty results. LP64 dimension checks occur before those shortcuts.

## Reference algorithm

`ReferenceProvider` uses Householder orthogonal/unitary transformations, not
normal equations:

- for `m >= n`, it factors `A = Q R`, applies `Q^H` to `B`, and solves the
  leading triangular system;
- for `m < n`, it factors `A^H = Q R`, solves `R^H Y = B`, pads `Y` with zeros,
  and applies `Q` to obtain the minimum-norm solution.

The same paths support `f32`, `Complex32`, `f64`, and `Complex64`; complex
reflectors use conjugate inner products. Binary32 norms, reflector
coefficients, thresholds, and arithmetic remain binary32. Allocations for
factors, reflector coefficients, transformed right-hand sides, and results use
fallible reservations. Cancellation is checked before work, during
factorization and transformations, during triangular solves, and before
returning.

The reference rank check compares each new triangular diagonal norm with a
tolerance scaled by the epsilon of the request precision. A detected short
rank returns
`LinalgError::RankDeficient { deficient_diagonal, required_rank, .. }`; no
partial solution is returned. The error reports the failed triangular diagonal,
not a rank estimate that the non-rank-revealing algorithm did not compute.

## OpenBLAS algorithm and ABI

The Windows provider binds the stable LAPACKE C exports `LAPACKE_sgels`,
`LAPACKE_cgels`, `LAPACKE_dgels`, and `LAPACKE_zgels` from the same
caller-supplied absolute LP64 DLL used for GEMM and `gesv`. Calls use
column-major layout and `trans = 'N'`. The FFI boundary checks:

- LP64 conversions for `m`, `n`, `nrhs`, `lda = max(1, m)`, and
  `ldb = max(1, m, n)`;
- exact `m * n` coefficient storage;
- exact `max(m, n) * nrhs` padded right-hand-side workspace storage; and
- explicit `[real, imaginary]` f32 or f64 packing for every complex value.

OpenMat fallibly copies `A`, builds the padded LAPACK `B` workspace, checks
cancellation immediately before and after the synchronous native call, and
fallibly extracts the first `n` rows. Inputs remain immutable. `info < 0` maps
to `InvalidProviderArgument`; `info > 0` maps to `RankDeficient` at the reported
triangular diagonal.

## Rank and MATLAB-compatibility boundary

This is deliberately a **full-rank QR/LQ contract**. `gels` assumes full rank;
its positive `info` detects a zero triangular diagonal but is not a
rank-revealing numerical algorithm. A nearly dependent matrix can therefore
produce an ill-conditioned result without a warning, and backend rank decisions
need not match the reference tolerance.

The contract does not expose a rank estimate on success, reciprocal condition
number, warning channel, truncated solution, SVD factors, or a MATLAB-compatible
fallback sequence. Rank-deficient and condition-aware language behavior must be
designed as a later contract using a rank-revealing QR or SVD family (for
example `gelsy`, `gelsd`, or an explicitly chosen equivalent), with an agreed
tolerance and warning policy. Until then, language integration must not present
this provider method as complete MATLAB `A\B` dispatch.

## Tests

Reference tests cover binary32 and binary64 shape classifications, multiple
right-hand sides, immutable inputs, cancellation, zero-rank shapes, LP64
overflow, structured rank deficiency, real least-squares residual
orthogonality, and real and complex minimum-norm solutions. Windows OpenBLAS
tests compare real overdetermined and complex underdetermined results in both
precisions with the reference provider and verify residual/minimum-norm
properties. Real DLL tests run only when `OPENMAT_TEST_OPENBLAS_DLL` names an
absolute compatible DLL.
