# OpenBLAS LAPACK solves and decomposition boundary

This milestone implements the numerical-provider boundary for the square dense
system

```text
A * X = B
```

`A` is `n x n`, `B` and the result are `n x nrhs`, and every matrix is
contiguous column-major storage. The implemented element classes are `f32`,
`Complex32`, `f64`, and `Complex64`. The binary32 implementations never widen
through binary64 storage or intermediates. A separate provider contract handles
full-rank rectangular least-squares and minimum-norm systems; see
[`rectangular-solve.md`](rectangular-solve.md). Sparse arrays, transposed solves,
and condition estimation are not part of this milestone.

LU, explicit full/thin QR with optional column pivoting, and upper/lower
Cholesky are also available through the provider. Their owned result contracts
and normalized zero-based status are specified in
[`decomposition-provider.md`](decomposition-provider.md). Four-precision SVD
and general eig are specified in [`spectral-provider.md`](spectral-provider.md).
Condition estimation remains out of scope.

## Provider contract

`openmat-linalg` exposes `SolveRequest`, `validate_solve`, `solve_f32`,
`solve_complex32`, `solve_f64`, and `solve_complex64`, with matching methods on
`LinalgProvider`. Shape validation is provider-neutral: both operands must be
matrices, `A` must be square, and the row count of `B` must equal the order of
`A`.

Providers treat `A` and `B` as immutable. Both current implementations make
fallibly allocated working copies, perform any in-place factorization only on
those copies, and construct the returned `DenseArray` from independent
storage. This preserves the array crate's copy-on-write contract even when the
inputs have other owners.

`ReferenceProvider` uses deterministic Gaussian elimination with partial
pivoting. At each column it selects the first row having the largest scalar
absolute value or complex magnitude. It supports multiple right-hand sides and
reports an exact zero factorization pivot as `LinalgError::SingularMatrix`.
Tests use scaled numerical tolerances; neither the reference implementation nor
OpenBLAS is expected to be bit-for-bit identical to MATLAB or to each other.

An order-zero `0 x 0` coefficient matrix accepts `0 x nrhs` right-hand sides
and returns an independent empty result of the same shape. An LP64 provider
still validates `n`, `nrhs`, `lda`, and `ldb` before taking that empty shortcut.

## DLL and ABI requirements

On Windows, deployment supplies a trusted OpenBLAS DLL by an explicit absolute
path. The loader does not search or modify `PATH` or use the current directory.
The DLL and its deployed dependencies must provide these undecorated exports:

- `openblas_get_config`
- `openblas_get_corename`
- `cblas_sgemm`
- `cblas_cgemm`
- `cblas_dgemm`
- `cblas_zgemm`
- `cblas_ddot`
- `cblas_dnrm2`
- `LAPACKE_sgesv`
- `LAPACKE_cgesv`
- `LAPACKE_dgesv`
- `LAPACKE_zgesv`
- `LAPACKE_sgels`
- `LAPACKE_cgels`
- `LAPACKE_dgels`
- `LAPACKE_zgels`
- `LAPACKE_sgetrf`, `LAPACKE_dgetrf`, `LAPACKE_cgetrf`, `LAPACKE_zgetrf`
- `LAPACKE_sgeqrf`, `LAPACKE_dgeqrf`, `LAPACKE_cgeqrf`, `LAPACKE_zgeqrf`
- `LAPACKE_sorgqr`, `LAPACKE_dorgqr`
- `LAPACKE_cungqr`, `LAPACKE_zungqr`
- `LAPACKE_sgeqp3`, `LAPACKE_dgeqp3`, `LAPACKE_cgeqp3`, `LAPACKE_zgeqp3`
- `LAPACKE_spotrf`, `LAPACKE_dpotrf`, `LAPACKE_cpotrf`, `LAPACKE_zpotrf`
- `LAPACKE_sgesdd`, `LAPACKE_dgesdd`, `LAPACKE_cgesdd`, `LAPACKE_zgesdd`
- `LAPACKE_sgeev`, `LAPACKE_dgeev`, `LAPACKE_cgeev`, `LAPACKE_zgeev`

OpenMat reads `openblas_get_config` first. Builds advertising `USE64BITINT`,
`INTERFACE64`, or `OPENBLAS_USE64BITINT` are classified as ILP64 and rejected
before any size-dependent compute symbol is bound. The accepted interface is
LP64: every `lapack_int`/`blasint` size and pivot is a signed 32-bit integer.

The solve boundary uses the stable LAPACKE C ABI and the
`LAPACK_COL_MAJOR`/`CBLAS_COL_MAJOR` layout value `102`. Before each native call
it validates all of the following:

- checked conversions for `n`, `nrhs`, `lda`, and `ldb`;
- `n > 0`, `nrhs >= 0`, and exact column-major leading dimensions;
- exact `n * n`, `n * nrhs`, and `n` slice lengths with checked host products;
- a pivot buffer allocated with `Vec::try_reserve_exact`;
- independent mutable coefficient, pivot, and right-hand-side buffers.

Complex values cross FFI only as explicit adjacent `[real, imaginary]` pairs:
f32 pairs for `Complex32` and f64 pairs for `Complex64`. Neither Rust complex
representation is used as an external ABI.
`info < 0` becomes `LinalgError::InvalidProviderArgument` with the reported
one-based argument position; `info > 0` becomes `LinalgError::SingularMatrix`
with the reported one-based pivot.

The rectangular boundary uses `LAPACKE_sgels`, `LAPACKE_cgels`,
`LAPACKE_dgels`, and `LAPACKE_zgels` with `trans = 'N'`. It checks `m`, `n`,
`nrhs`, `lda`, and `ldb` through the same LP64 conversion boundary. LAPACK
requires a mutable `max(m, n) x nrhs` right-hand-side workspace: OpenMat
allocates it fallibly, copies immutable `B` into its first `m` rows, and
extracts only the first `n` result rows after the call. `info > 0` becomes
`LinalgError::RankDeficient`; the limitations of that signal are recorded in
the rectangular contract document.

The decomposition boundary validates `m`, `n`, `k`, explicit-Q columns, order,
and every leading dimension and slice length before GETRF, GEQRF/GEQP3,
ORGQR/UNGQR, or POTRF. GETRF `ipiv`, GEQP3 `jpvt`, and positive GETRF/POTRF
`info` are range-checked and normalized from LAPACK's one-based values to the
shared zero-based structures. Cholesky failure is returned as data and the
provider zeros the unused triangle and all entries outside the successful
leading block.

The spectral boundary validates the same LP64 matrix sizes plus GESDD `ldu`
and `ldvt`, vector-job shapes, and GEEV value/vector buffers. Positive
GESDD/GEEV `info` becomes structured `NoConvergence`; LAPACKE workspace
allocation statuses become `AllocationFailure`. Real GEEV conjugate-pair
columns are converted to explicit complex values and left/right vectors before
leaving the provider. Complex calls continue to expose only adjacent component
pairs at the ABI.

## Threads, cancellation, and resource limits

OpenBLAS may create or use its own worker threads according to the deployed
library configuration. The provider does not change process-global OpenBLAS
thread settings. Calls through one or more provider values are synchronous from
the caller's perspective; the DLL stays loaded for the complete call.

`SolveRequest`, `RectangularSolveRequest`, `FactorRequest`, `QrRequest`,
`CholeskyRequest`, `SvdRequest`, and `EigRequest` can carry a shared atomic
cancellation flag. The reference provider checks it during factorization,
orthogonal transformations, and substitution. LAPACKE has no cooperative
cancellation parameter, so `OpenBlasProvider` checks immediately before and
after each blocking native call. Cancellation cannot stop a LAPACK call already
executing; the server/kernel boundary remains responsible for terminating and
restarting a kernel stuck in native code.

Coefficient copies, right-hand-side copies, packed complex buffers, result
conversion, and the LAPACK pivot buffer use fallible reservations and report
`LinalgError::AllocationFailure`. There is currently no configurable numerical
workspace byte budget in this crate.

## Testing and deployment check

Reference-provider and mock-FFI tests always run for binary32 and binary64.
Real DLL tests run only when
`OPENMAT_TEST_OPENBLAS_DLL` contains an absolute path to a compatible deployed
DLL. If the variable is absent, each such test prints an explicit skip reason
and returns; tests never download a numerical library or search the machine for
one.

## Later `A\B` integration

Language/runtime integration is deliberately outside this milestone. Wiring
the language operator will still need to:

1. select real or complex solve after applying language-level type promotion;
2. pass the workspace cancellation flag into `SolveRequest`;
3. translate structured shape, singularity, cancellation, allocation, and
   provider errors to the agreed MATLAB-compatible observable behavior; and
4. dispatch rectangular systems to `RectangularSolveRequest` only after the
   language layer has chosen its MATLAB-compatible rank, warning, and fallback
   policy; the provider contract itself does not make that language decision.
