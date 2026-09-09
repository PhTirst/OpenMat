# Sparse numerical provider boundary

## Scope

`openmat-linalg` owns the provider-neutral boundary. It accepts canonical,
zero-based CSC views containing `u64` offsets/indices and either `f64` or
`Complex64` values. `openmat-sparse-provider` supplies both the in-tree
correctness oracle and a production-shaped faer adapter. No provider container
becomes a language value. Runtime integration borrows OpenMat-owned CSC buffers
through this boundary and returns independently owned language results.

The reference provider implements:

- real and complex-double SpMV and SpGEMM;
- separate symbolic and numeric phases for square LU and SPD/Hermitian-positive-
  definite Cholesky;
- reusable numeric factor handles for one or more dense, column-major right-hand
  sides; and
- tall (`m >= n`), full-column-rank real and complex least squares using a
  correctness-first dense QR workspace; and
- full-row-rank wide real and complex basic solves using the first numerically
  independent columns in left-to-right CSC order.

The generic least-squares entry remains tall-only; sparse-language
underdetermined `A\B` uses the separate basic-solve entry because R2022b does
not return the minimum-norm solution at this boundary. Column selection stores
only an `m x m` orthogonal basis and the selected square CSC matrix rather than
densifying the full `m x n` input. The reference LU and tall-QR algorithms still
intentionally densify their factors/workspaces and remain test/oracle-quality
baselines, not production algorithms for large sparse input.

## Boundary invariants

`CscPatternRef::new` validates the sentinel offset, monotone offsets, exact final
`nnz`, in-range rows, and strictly increasing row indices in every column.
`CscMatrixRef::new` additionally requires one value per row index. Borrowed views
never transfer ownership. Provider-created sparse output uses
`OwnedCscMatrix<T>`, which owns all three CSC buffers and revalidates them before
construction.

The provider advertises a signed `I32` or `I64` structural-index ABI. Rows,
columns, and `nnz` are checked before an adapter narrows them. Canonical CSC
validation makes individual offsets and row indices bounded by those checked
counts. A future native adapter must perform this check before allocating native
pivot arrays or calling foreign code.

Symbolic handles own a copy of the canonical structure. Numeric factorization
requires the same factor kind, dimensions, fingerprint, offsets, and row indices;
a hash collision alone therefore cannot attach values to the wrong analysis.
Both symbolic and numeric handles are immutable, `Send + Sync`, and provider-
owned. Solving borrows rather than consumes a numeric handle, making reuse and
concurrent read-only application explicit.

Cancellation is cooperative. The caller lends an optional `AtomicBool`; the
provider polls at phase and outer-loop checkpoints and returns the operation plus
the observed `Symbolic`, `Numeric`, or `Execution` phase. Foreign adapters must
either use a backend interrupt facility or check immediately before and after a
non-interruptible call; cancellation cannot imply that opaque native code stopped
mid-call.

Metrics contain deterministic structural and numeric work counters plus input and
output `nnz`. They deliberately exclude elapsed time. Errors distinguish invalid
CSC, dimension/index overflow, cancellation, singular pivots, non-positive-
definite minors, measured rank deficiency, unsupported operations, symbolic
pattern mismatch, allocation failure, and provider failure.

## Reference numerical behavior

The LU baseline uses dense column-major storage, partial pivoting, and exact-zero
pivot detection. Cholesky consumes a fully stored symmetric/Hermitian matrix and
builds a dense lower factor. The tall least-squares baseline uses modified
Gram-Schmidt and reports the number of accepted columns when its scale-dependent
rank threshold fails. Residual tests cover real and complex LU/Cholesky and tall
least squares; separate tests cover singular LU, non-positive-definite Cholesky,
rank-deficient least squares, real/complex multi-RHS underdetermined basic
solutions, and deficient row rank.

These choices are intentionally stricter than the eventual language wrapper.
Locally authored black-box probes against installed MATLAB R2022b measured:

- a square sparse solve with zero residual for the probe system;
- complex sparse Cholesky success (`p == 0`) with a `4.44e-16` Frobenius
  reconstruction residual;
- a tall full-rank solve returning two rows with a `1.42e-15` residual;
- a singular square solve signaling a warning and returning non-finite values;
  and
- a rank-deficient tall solve signaling a warning, returning two rows, and
  selecting a leftmost-column basic least-squares result.

The provider returns structured `Singular` and `RankDeficient` errors instead of
choosing user-visible warning/value policy. Runtime integration must map those
errors to R2022b-compatible warnings and results; that policy does not belong in a
numerical provider.

## Faer 0.24.4 provider

`FaerSparseProvider` is an in-process build-time adapter. The dependency is
exactly pinned to faer 0.24.4 with `default-features = false` and only `std` and
`sparse-linalg` enabled. The latter expands to faer's `sparse` and `linalg`
features. `rayon`, `rand`, and `npy` are not enabled. The adapter always passes
`Par::Seq`: parallel execution is intentionally off until OpenMat has a provider
thread-budget policy, and sequential execution keeps resource use and metrics
auditable. The locked faer package metadata declares MIT and Rust 1.84 or newer;
OpenMat's workspace Rust requirement is newer.

Validated OpenMat CSC is copied through faer's safe triplet constructor. Every
dimension, count, offset, and row conversion is checked before narrowing from
`u64` to `usize`, and the provider also enforces faer's signed index range. It
uses no transmute, unchecked aliasing, or borrowed-buffer lifetime extension.
Symbolic handles own the exact OpenMat pattern as well as faer's analysis;
numeric handles own their faer factors and permutations. This makes handle
destruction deterministic under ordinary Rust ownership.

The operations are actual faer sparse algorithms for both `f64` and complex64:

- SpMV calls faer's sparse-dense multiplication.
- SpGEMM calls faer's public sparse-sparse multiplication and converts its
  result back to canonical CSC, dropping exact stored zeros. It neither
  densifies nor falls back to `ReferenceSparseProvider`.
- LU retains a forced-simplicial faer symbolic analysis, row/column
  permutations, and numeric L/U data. One factor solves multiple column-major
  right-hand sides and can be applied repeatedly.
- Cholesky retains faer's sparse symbolic and numeric LLT objects after OpenMat
  first verifies that the fully stored input is symmetric/Hermitian. A
  non-positive pivot maps to `NotPositiveDefinite`.
- Tall (`m >= n`) least squares retains faer's sparse QR. A second
  forced-simplicial faer QR numeric pass exposes R's diagonal so the adapter can
  return a measured `RankDeficient` error before retaining the reusable
  high-level QR solve object.
- Wide full-row-rank basic solve uses the provider-neutral left-to-right CSC
  independence pass, then retains faer sparse execution for the selected square
  LU and its multiple right-hand sides. The generic faer QR least-squares entry
  remains an explicit `Unsupported` boundary for wide shapes so callers cannot
  accidentally substitute minimum-norm semantics.

There is no hidden reference fallback in `FaerSparseProvider`; capability and
backend reporting say `Faer` for its supported operations. Callers select
`ReferenceSparseProvider` themselves when they want the oracle/fallback.

Faer's factorization and solve calls used here do not accept an OpenMat
cancellation callback. The adapter checks cancellation immediately before and
after each non-interruptible call. Cancellation observed after a call means the
result is discarded; it does not mean faer stopped mid-call. Panics are caught
at the adapter boundary and become `ProviderFailure`, while public faer creation,
allocation, symbolic, singular, and non-positive-pivot failures map to structured
`SparseError` variants.

Faer does not expose deterministic operation counts for these APIs. Adapter
metrics therefore report owned structural counts (`input_nnz`, `output_nnz`, and
symbolic pattern visits) while `numeric_work` remains zero rather than inventing
a value. Performance smoke tests use residual and structural assertions, never
wall-clock thresholds.

## Future provider attachment

An in-process provider compiled together with OpenMat implements
`SparseProvider` directly. Its Rust types remain internal to that build and do
not cross a dynamic-library, plugin, or process boundary.

A separately distributed native provider requires a versioned C ABI. A proposed
adapter should expose one `openmat_sparse_provider_v1` function table containing:

- `abi_version`, `struct_size`, provider capabilities, and index width;
- host allocator/deallocator callbacks so each allocation is freed by its owner;
- plain C CSC descriptors with explicit scalar tags, dimensions, lengths, and
  immutable pointers valid only for the duration of a call;
- opaque symbolic and numeric handle pointers with matching destroy functions;
- separate analyze, factor, solve, SpMV, SpGEMM, and least-squares entries;
- a cancellation callback/context pair and a fixed-layout metrics output; and
- numeric status codes plus caller-sized diagnostic buffers, never Rust enums,
  strings, trait objects, panics, or allocators.

The host validates CSC and index widths before entering the C adapter, catches no
foreign exceptions across C, and converts output into host-owned buffers before
destroying provider results. A process-isolated provider should use a versioned
serialized protocol with the same lifetime and phase model.

## License selection is component-specific

This lane binds and vendors no GPL or AGPL implementation. The selected faer
0.24.4 package declares MIT; dependency review still audits its exact locked
transitive graph. Future dependency review must evaluate the exact selected
library, modules, build flags, transitive ordering libraries, BLAS, and optional
partitioner; “SuiteSparse license” is not a sufficient conclusion. The upstream
collection itself says each package has a separate license in its
[collected license file](https://github.com/DrTimothyAldenDavis/SuiteSparse/blob/dev/LICENSE.txt).

Examples relevant to this boundary in the current upstream development tree:

- AMD, CAMD, CCOLAMD, and COLAMD state BSD-3-Clause licenses.
- KLU, LDL, BTF, CSparse, and CXSparse state LGPL-2.1-or-later licenses.
- UMFPACK and SPQR state GPL-2.0-or-later licenses.
- CHOLMOD is split by module: Check, Cholesky, Utility, and Partition state
  LGPL-2.1-or-later; MatrixOps, Modify, Supernodal, and several tooling modules
  state GPL-2.0-or-later; its public/internal headers identify Apache-2.0.
- SuiteSparse:GraphBLAS identifies the compiled library as Apache-2.0 while its
  MATLAB `@GrB` interface, tests, and demos are GPL-3.0-or-later.

Versions and build composition can change, so integration must pin an exact
release and archive a component-by-component license manifest. Commercial or
alternate licensing, if considered, requires separate legal review.

## Runtime integration status

The runtime converts `openmat-sparse::CscMatrix<T>` slices into
`CscMatrixRef<T>` and selects the in-process faer provider without allowing a
provider type to enter `openmat-value`. Square, tall full-rank, wide
full-row-rank, sparse-by-full, and full-by-sparse paths are wired through that
boundary.

Structured singular and rank-deficient provider states remain provider errors,
but the runtime now converts those two categories into the R2022b-compatible
fallback payloads described in
`docs/compatibility/sparse-linear-algebra-r2022b.md`. Sparse dispatch carries a
typed optional warning alongside the successful value; the central interpreter
updates session `lastwarn`, respects warning enablement, and emits through the
normal ordered output channel. All other provider failures remain structured
runtime errors.
