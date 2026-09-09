# Native single-precision linear algebra provider

Status: implemented through the `openmat-linalg` and Windows
`openmat-openblas` provider boundary and wired into `openmat-runtime` matrix
multiplication and division dispatch.

## Public provider boundary

`LinalgProvider` now has native binary32 methods parallel to the existing
binary64 contract:

| Operation | Real binary32 | Complex binary32 |
| --- | --- | --- |
| GEMM | `gemm_f32` | `gemm_complex32` |
| Square general solve | `solve_f32` | `solve_complex32` |
| Full-rank rectangular solve | `solve_rectangular_f32` | `solve_rectangular_complex32` |

The provider-neutral allocating helpers are `matrix_multiply_f32`,
`matrix_multiply_complex32`, `solve_f32`, `solve_complex32`,
`solve_rectangular_f32`, and `solve_rectangular_complex32`. The
column-pivoted underdetermined basic-solution helpers also expose
`solve_underdetermined_basic_f32` and
`solve_underdetermined_basic_complex32`, so the runtime does not widen its
`m < n` route.

The request, dimension-validation, structured-error, column-major, and
multiple-right-hand-side types remain shared across precisions. New trait
methods have structured default failures for third-party or test providers
which have not implemented native binary32. `ReferenceProvider` and
`OpenBlasProvider` override all six methods. Existing f64/`Complex64` methods
and their behavior are unchanged.

## Native binary32 numerical behavior

The reference implementation does not convert input values, workspaces,
intermediates, thresholds, or results through binary64:

- real GEMM accumulates with binary32 `mul_add`;
- complex GEMM stores and combines `Complex32` components directly;
- LU pivot selection and elimination use binary32 real or complex arithmetic;
- Householder QR/LQ norms, reflector coefficients, rank tolerances, and
  triangular solves use f32 for binary32 inputs; and
- column-pivoted basic solve uses f32 residual norms, reorthogonalization,
  rank tolerance, and the provider's native f32 square solve.

The deterministic tie rules remain the same as binary64. The rank threshold is
scaled by the epsilon of the request precision, so a matrix can correctly be
full rank under the binary64 reference contract and rank deficient under the
binary32 contract.

All operations retain contiguous column-major storage and immutable inputs.
Working copies, pivot arrays, reflector arrays, packed complex arrays, padded
LAPACK right-hand sides, and results use fallible allocation. Results do not
share storage with inputs. GEMM preserves output COW detachment. Square and
rectangular requests retain their existing cooperative cancellation behavior:
reference loops poll throughout, while native calls poll immediately before
and after synchronous LAPACKE entry.

Empty shapes follow the binary64 boundary. GEMM accepts zero output extents and
zero inner dimensions. A zero-order square solve returns an independent empty
right-hand-side-shaped result. A zero required rectangular rank returns an
independently allocated empty or zero-filled `n x nrhs` result. LP64 dimension
conversion occurs before every native empty shortcut.

## Windows OpenBLAS ABI

The explicit absolute-path loader additionally requires these undecorated
exports:

- `cblas_sgemm`
- `cblas_cgemm`
- `LAPACKE_sgesv`
- `LAPACKE_cgesv`
- `LAPACKE_sgels`
- `LAPACKE_cgels`

They use the same checked signed-32-bit LP64 dimensions, column-major layout
value `102`, leading-dimension validation, and LAPACK `info` mapping as the
binary64 calls. ILP64 is rejected before these symbols are bound. Complex
values, including GEMM alpha and beta, cross C only as explicit adjacent
`[real, imaginary]` f32 pairs. `openmat_array::Complex32` is never passed across
the C ABI.

The loader still does not inspect or modify `PATH`, search the current
directory, or download a DLL. Real-library tests run only when
`OPENMAT_TEST_OPENBLAS_DLL` explicitly names an absolute compatible DLL. Local
`extern "C"` mocks exercise all six new wrappers without a deployed DLL,
including transpose/layout values, dimensions, mutations, and adjacent complex
pair representation.

## Runtime integration

`openmat-runtime::linalg_matrix_multiply` accepts a homogeneous pair of
binary32 `Value::Array` operands in addition to its existing binary64 inputs. Real
single storage calls `matrix_multiply_f32`. If either single operand is
complex, only the real single operand is copied to `Complex32`, with positive
zero imaginary components, and the operation calls
`matrix_multiply_complex32`. The result is a binary32 `Value::Array` with
storage independent of both inputs.

`array_ops::evaluate_binary` selects that route only when both operands are
non-scalar, canonical rank-two single matrices. This placement is before the
single element-arithmetic branch. It does not change scalar or `1x1` behavior,
dotted operators, higher-rank rejection, or the accepted mixed single/double
boundary.

The single left-division bridge now dispatches without a precision conversion:

| Coefficient shape | Real single | Complex single |
| --- | --- | --- |
| square | `solve_f32` | `solve_complex32` |
| `m > n` | `solve_rectangular_f32` | `solve_rectangular_complex32` |
| `m < n` | `solve_underdetermined_basic_f32` | `solve_underdetermined_basic_complex32` |

Right division applies the same routes to `(right' \ left')` and transposes the
result back. Real transposes retain f32, and complex apostrophes map directly
between `Complex32` values with conjugation. Empty results are constructed as
f32 or `Complex32` before provider entry. Real-to-complex promotion,
transposition, empty allocation, provider requests, and the basic-solve helper
retain the existing cooperative cancellation checks.

The former `widen_f32`, `widen_single_complex`, `narrow_f64`, and
`narrow_complex64` functions no longer exist. Probe-provider tests override all
six binary32 methods and assert that the corresponding binary64 counters stay
zero. A cancellation-sensitive GEMM case also distinguishes the paths
numerically: binary32 accumulation of `1e10 + 1 - 1e10` returns `0`, whereas
widening the same stored inputs to binary64 and rounding the result would
return `1`.

Mixed single/double matrix result-class policy and warning-bearing
singular/rank-deficient success remain separate language-contract work.

## Decomposition provider follow-up

The provider now has explicit `f32` and `Complex32` LU, QR, and Cholesky
entries alongside their binary64 counterparts. They use same-precision
reference kernels and native OpenBLAS `s`/`c` LAPACKE routines; no single
factorization widens through binary64. The owned result and normalized status
contracts are described in
[`decomposition-provider.md`](decomposition-provider.md).

The provider also has native `f32` and `Complex32` SVD and general eig entries.
Reference SVD uses same-precision scaled one-sided Jacobi rotations; reference
eig converts real single only to `Complex32` and runs same-precision complex
Hessenberg/shifted-QR iteration. OpenBLAS uses `sgesdd`/`cgesdd` and
`sgeev`/`cgeev`, with explicit adjacent f32 pairs at C. Neither route widens to
binary64. Contracts and numerical limits are specified in
[`spectral-provider.md`](spectral-provider.md).

`rank` and default/two-norm `cond` can consume values-only SVD in their input
precision. Language built-ins, one/infinity condition estimators, determinant
assembly, and LU-backed inverse remain separate work.
