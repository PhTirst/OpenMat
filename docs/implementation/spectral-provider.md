# SVD and general eigenvalue provider boundary

This milestone adds provider-level singular value decomposition and general
eigenvalue decomposition for `f32`, `f64`, `Complex32`, and `Complex64`. It
does not implement or wire language built-ins. Requests borrow canonical
rank-two column-major arrays, optionally carry the shared cancellation flag,
and results always own storage independent of the input.

## Object-safe contracts

`LinalgProvider` exposes four explicit SVD methods (`svd_f32`, `svd_f64`,
`svd_complex32`, and `svd_complex64`) and four explicit general eig methods
(`eig_f32`, `eig_f64`, `eig_complex32`, and `eig_complex64`). Matching
provider-neutral helpers use `SvdRequest` and `EigRequest`; default methods
return structured provider failures and never widen a missing binary32
implementation through binary64.

`SvdVectors` selects `None`, `Thin`, or `Full`. For an `m x n` input and
`k=min(m,n)`, singular values are descending, nonnegative real values shaped
`k x 1`:

| job | U | Vh |
| --- | --- | --- |
| `None` | omitted | omitted |
| `Thin` | `m x k` | `k x n` |
| `Full` | `m x m` | `n x n` |

The same shapes hold at zero extents. Full factors contain the appropriate
identity complement; for example, full SVD of `0 x n` has `U: 0 x 0` and
`Vh: n x n`, while thin output has `U: 0 x 0` and `Vh: 0 x n`.

General eig accepts only square matrices. `EigRequest` independently selects
left and right vectors. `EigResult` always stores explicit complex
eigenvalues (`n x 1`) and optional complex vector matrices (`n x n`), including
for real inputs. Right columns satisfy `A*v=lambda*v`; left columns satisfy
`u^H*A=lambda*u^H`. Consequently no caller has to decode LAPACK's real
conjugate-pair convention. A values-only request allocates neither vector
matrix. Order-zero eig returns an empty complex values column and the selected
empty vector matrices.

Both operations can report `LinalgError::NoConvergence`, carrying provider and
operation plus a reference iteration count or provider-reported unconverged
status when available. Validation, LP64 overflow, allocation failure, invalid
native arguments, provider failure, and cancellation remain distinct errors.

## Reference algorithms

Reference SVD is a scaled one-sided Jacobi algorithm in the input scalar and
real-component precision. It uses complex unitary plane rotations where
needed, a precision-scaled column-orthogonality convergence test, a finite
sweep limit, descending singular-value sorting, and deterministic orthogonal
completion for zero singular values and full factors. Wide matrices use the
same kernel on the conjugate transpose and swap the resulting factors; they do
not form a binary64 normal-equations problem.

Reference general eig first converts a real input to the matching
`Complex32` or `Complex64` type, never to a wider component type. It applies
global magnitude scaling, Householder reduction to upper Hessenberg form, and
shifted complex Householder QR similarity iterations with precision-scaled
subdiagonal deflation and an explicit iteration cap. Optional vectors are
recovered from the complex Schur form by triangular substitution and the
accumulated unitary transformations. This supports nonsymmetric, non-Hermitian,
and complex nonnormal matrices; it is not power iteration and does not assume
normality. Failure to deflate within the cap is structured `NoConvergence`.

All workspaces use fallible reservations and checked host products. Reference
loops poll cancellation at sweep, Hessenberg, and QR boundaries. Tests cover
four precisions, tall/wide/full/thin/values-only and empty SVD shapes,
reconstruction and orthogonality/unitarity, real conjugate eigenvalues,
complex nonnormal right and left residuals, COW, LP64 validation,
cancellation, and a forced iteration-limit failure.

## OpenBLAS normalization

The Windows provider uses the high-level LP64 LAPACKE `s/d/c/zgesdd` and
`s/d/c/zgeev` entry points. Every dimension, leading dimension, and exact
slice length is checked before C entry. Values-only GESDD uses job `N`, thin
uses `S`, and full uses `A`; eig independently selects `N` or `V` for left and
right vectors. Empty operations are completed without entering LAPACK, after
LP64 conversion.

Complex matrices, eigenvalues, and vectors cross C only as explicit adjacent
f32 or f64 real/imaginary pairs. No Rust complex layout is exposed. Real GEEV
output is normalized as follows: a positive imaginary value consumes that
column as the real part and the next native column as the imaginary part; the
following eigenvalue/vector is its explicit conjugate. Missing, reversed, or
unpaired encodings are structured provider failures.

Negative LAPACK `info` is `InvalidProviderArgument`, except LAPACKE's native
workspace-allocation statuses, which become `AllocationFailure`. Positive
GESDD/GEEV `info` becomes `NoConvergence`. The provider checks cancellation
immediately before and after a synchronous native call; LAPACKE itself has no
cooperative cancellation hook.

The trusted DLL remains explicit-path only. It is never searched on `PATH` or
downloaded. ABI mocks cover all eight calls, including the pair representation;
real-DLL tests remain opt-in through `OPENMAT_TEST_OPENBLAS_DLL`.

## Consumers and remaining language work

`rank` can consume a values-only SVD and apply its language-level tolerance to
the returned sorted singular values. Default/two-norm `cond` can consume the
same request and form `s_max/s_min`, including the agreed empty, zero, and
infinite cases. One- and infinity-norm condition estimates still require a
separate LU-backed estimator contract; they must not be synthesized from this
two-norm result.

Language built-ins must still select the provider, assemble MATLAB-compatible
multi-output forms, choose vector/eigenvalue ordering conventions, translate
structured errors, and apply warning behavior. Balancing controls, generalized
eigenproblems, symmetric/Hermitian specialized drivers, and sparse spectral
algorithms are not part of this provider contract.
