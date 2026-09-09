# Dense decomposition provider boundary

This milestone adds provider-level factorization primitives for the four dense
element classes `f32`, `f64`, `Complex32`, and `Complex64`. It does not wire the
language built-ins. Every request borrows a canonical rank-two column-major
`DenseArray`, may carry the existing cooperative cancellation flag, and every
result owns storage independent of the input.

## Object-safe contract

`LinalgProvider` has four explicit methods for each operation rather than a
generic method:

- `factor_lu_f32`, `factor_lu_f64`, `factor_lu_complex32`, and
  `factor_lu_complex64`;
- `qr_f32`, `qr_f64`, `qr_complex32`, and `qr_complex64`; and
- `cholesky_f32`, `cholesky_f64`, `cholesky_complex32`, and
  `cholesky_complex64`.

The matching provider-neutral helpers accept `FactorRequest`, `QrRequest`, or
`CholeskyRequest`. Default trait methods return a structured provider failure,
so an older provider remains source-compatible without silently widening or
running an unrelated implementation.

`LuResult` owns the rectangular packed `L`/`U` matrix, the full final row order,
swap parity, and the first exact zero pivot. The permutation is the zero-based
original row at each factor row and satisfies `P*A = L*U`. The strict lower
trapezoid stores the unit-diagonal `L` multipliers; the upper trapezoid stores
`U`. A zero pivot is an `Option<u64>` data status, not an error from the factor
operation. Square solve reuses the same factorization kernel and converts that
status to its existing one-based `SingularMatrix` error.

`QrRequest` selects `Full` or `Thin` explicit vectors and optional column
pivoting. For an `m x n` input, full output is `Q: m x m`, `R: m x n`; thin
output uses `k=min(m,n)`, `Q: m x k`, `R: k x n`. The zero-based column vector
stores the original column at each factor position and satisfies
`A(:, permutation) = Q*R`; it is identity for an unpivoted request. The result
also carries the same-precision numerical-rank estimate and first deficient
zero-based diagonal used for provider diagnostics. They are data and do not
turn a valid QR factorization into a rank-deficiency error.

`CholeskyRequest` selects the upper or lower triangle. `CholeskyResult` returns
a triangular factor and a zero-based `first_non_positive_minor`. On failure,
only the successful leading block is retained; the unused triangle and all
entries outside that block are exact zeros. The later built-in can therefore
implement `[R,p]` without parsing text, while its one-output form can translate
the same status to the required language error.

## Reference algorithms and boundaries

`ReferenceProvider` stays in the requested scalar precision. Rectangular LU and
square solve share one partial-pivoting elimination kernel. QR factorization,
rectangular solve, and explicit Q construction share the existing Householder
storage and reflector-application kernels; optional column pivoting chooses the
largest remaining active-column norm with deterministic first-column ties.
Complex QR uses conjugate inner products and Hermitian reflectors.

Cholesky uses the requested source triangle and real diagonal updates in the
input precision. Complex values are never widened to a real or complex
binary64 work matrix. All working copies, permutations, reflectors, explicit
factors, and empty results use fallible allocation. Dimension products are
checked before allocation, inputs remain immutable/COW-shared, and reference
loops check cancellation at factorization and transformation boundaries.

Empty shapes preserve algebraic dimensions. In particular, full QR of `m x 0`
returns `eye(m)` and `m x 0` R, full QR of `0 x n` returns `0 x 0` Q and
`0 x n` R, LU preserves its input shape and full row permutation, and Cholesky
of `0 x 0` returns an empty factor with no failure status.

## OpenBLAS normalization

The Windows provider checks `m`, `n`, `k`, `lda`, explicit-Q columns, and
Cholesky order through the LP64 boundary before any empty shortcut or native
call. GETRF pivot sequences are applied to an identity row vector to produce
the shared final permutation and parity. GEQP3 `jpvt` is validated as a unique
one-based permutation and converted elementwise to zero-based original column
indices. Positive GETRF/POTRF `info` is range-checked and converted from
one-based to the shared zero-based status. Negative `info` remains a structured
`InvalidProviderArgument`; an impossible pivot or status is a structured
provider failure.

POTRF's overwritten workspace is not exposed directly. The provider copies
only the selected triangle inside the successfully completed leading order and
zeros every other entry. Complex LAPACKE calls use explicit adjacent f32 or f64
real/imaginary pairs for matrices and Householder `tau`; no Rust complex layout
crosses the C ABI.

## SVD and general eig follow-up

The four-precision SVD and general eig contracts, same-precision reference
algorithms, OpenBLAS normalization, and `rank`/two-norm `cond` consumption are
now implemented and specified in
[`spectral-provider.md`](spectral-provider.md). `det` can consume the LU
diagonal plus swap parity, while `inv` still needs a language-level LU/solve
assembly decision. One- and infinity-norm condition estimates still require a
dedicated LU-backed estimator contract.

Runtime/built-in access to the interpreter-selected provider, output assembly,
one-based permutation conversion, warning-bearing singular success, and
MATLAB-compatible error mapping remain outside this provider-only change.
