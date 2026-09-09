# Core numeric built-ins implementation

Status: provider-independent tranches plus provider-backed factorization and
ordinary spectral tranches implemented on baseline
`pre-publication baseline`.

The implementation now adds thirty-seven fully registered names. The latest
tranche changes only the built-ins crate, focused conformance cases, and these
documents; it consumes the baseline's existing four-precision spectral and
solve provider API without changing runtime production, `openmat-linalg`,
`openmat-openblas`, the root manifest, kernel, protocol, or shared conformance
index. R2022b observations and the remaining inverse boundary are recorded in
`docs/design/core-numeric-builtins-r2022b.md`.

## Module layout

| Module | Registered names | Implementation notes |
| --- | --- | --- |
| `core_elementary.rs` | `fix`, `floor`, `ceil`, `exp`, `log`, `log10`, `sin`, `cos`, `tan` | Class-dispatched element traversal for binary64/binary32 real and complex storage; exact integer identity for rounding; principal complex formulas; shaped-empty preservation. |
| `core_logical_reduction.rs` | `any`, `all` | Shared reduction planner for default dimension, scalar dimension, `vecdim`, and `'all'`; column-major output mapping; `NaN` omission and distinct empty identities. |
| `core_find.rs` | `find` | One traversal collects linear indices and optional exact values; output planner handles row orientation and N-D subscript collapse; requested-output count selects one to three results. |
| `core_matrix.rs` | `diag`, `triu`, `tril` | Typed storage macros preserve logical, char, every integer class, `single`, and complex data; offsets and empty shapes use checked `u64` arithmetic. |
| `core_rearrange.rs` | `squeeze`, `permute`, `repmat` | Shape planners are separate from typed column-major copy loops; exact integer dimension components never pass through floating point; appended singleton permutation axes are handled without per-element coordinate allocation. |
| `core_statistics.rs` | `mean`, `median`, `std`, `var` | One checked reduction planner handles default, scalar, vector, and all-dimension forms; type-dispatched accumulators preserve native binary32 and exact native integer behavior; median groups retain measured class rules. |
| `core_sort.rs` | `sort` | A fallibly allocated stable merge sort works independently along any scalar dimension; typed comparators implement missing placement, direction, magnitude/phase or real complex order, and one-based index output. |
| `core_concat.rs` | `cat`, `horzcat`, `vertcat` | An N-D column-major mapping validates every nonconcatenated extent and applies measured numeric/logical/char class dominance with checked rounding and saturation. One-input forms preserve COW storage. |
| `core_sequences.rs` | `linspace`, `logspace` | Checked row allocation and separate binary64/binary32 interpolation paths cover real/complex endpoints, special counts, exact endpoints, complex division, and powers of ten. |
| `core_round.rs` | `round` | Decimal and significant modes dispatch over double/single real and complex storage; a one-ULP decimal-half correction matches double behavior while widened scaling preserves measured single decisions. |
| `core_linalg.rs` | `lu`, `qr`, `chol`, `det` | Four-precision provider dispatch; packed/split LU and row permutations; full/thin pivoted QR with matrix/vector permutations; upper/lower Cholesky status and partial factors; determinant from one LU diagonal and swap parity. |
| `core_spectral.rs` | `svd`, `eig`, `rank`, `cond` | Four-precision SVD/eig dispatch; full/thin/legacy-zero and matrix/vector SVD assembly; ordinary right/left eig outputs; precision-specific strict rank thresholds; values-only two-norm condition numbers plus provider-solve one/infinity/Frobenius forms. |

`lib.rs` registers these functions through the existing deterministic built-in
registry. `inv` remains unregistered.

## Runtime invariants

All allocation sizes are computed with checked `u64` shape arithmetic and then
validated against host `usize`. Result vectors reserve fallibly. Traversal
loops poll `BuiltinContext::check_cancelled` before or during work, including
exact integer conversion. Inputs are borrowed and results own independent
storage unless the operation is a deliberate exact COW identity.

Array addressing is column-major. Reductions decode a linear input offset into
source coordinates and remove or retain reduced axes according to the planned
output shape. `permute` precomputes source strides once, decodes each output
coordinate in destination-axis order, and accumulates the matching input
offset. `repmat` maps each output coordinate modulo the corresponding input
extent and short-circuits zero-element outputs.

Complex arrays are not blindly preserved when the R2022b result is observably
real. Negative real `log`/`log10` promotes to complex principal-branch output;
triangular selection demotes an all-real result after zeroing. `any`/`all`
classify a complex element from both components and omit it when either
component is `NaN`, matching the measured boundary.

`BuiltinContext` borrows a read-only `LinalgProvider` plus its compatible
cancellation flag. Standalone contexts keep the static reference provider for
backward compatibility and can explicitly borrow a provider for focused tests.
The interpreter clones the session-selected provider before creating its
object-aware language-copy closure, avoiding overlapping mutable/immutable
borrows while ensuring built-ins use the same provider as matrix operators.

Factorization inputs are borrowed and provider results own independent storage.
All language-layer factor splitting, triangular copying, permutation assembly,
complex demotion, and partial-factor trimming use checked shapes, fallible
allocation, column-major offsets, and periodic cancellation checks. Provider
result shapes, permutation uniqueness, and status indices are validated before
being exposed as language values.

Spectral inputs are likewise borrowed and provider results are independently
owned. SVD validates singular/vector shapes, constructs `S`, and converts
provider `V^H` to language `V` with a checked conjugate transpose. Eig validates
ordinary eigenvalue and requested left/right vector shapes, constructs a
diagonal `D` only when requested, and demotes exact all-real complex outputs.
Rank rejects nonfinite matrices before values-only SVD and computes
`max(size(A))*eps(max(s))` in the input precision. Two-norm condition numbers
use values-only SVD; other implemented norms use one provider solve and stable
column-sum, row-sum, or hypot accumulation without a private decomposition.

## Test coverage

Crate tests exercise:

- `double`, `single`, real/complex, logical, char, and fixed-width integers
  where each function accepts them;
- shaped empty, row, column, matrix, and significant N-D forms;
- dimension, vector-dimension, `'all'`, offset, permutation, repetition,
  direction, `k`, and requested-output overloads;
- exact integer dimension conversion, column-major order, cancellation,
  allocation/overflow guards, type errors, and domain errors; and
- complex branch behavior and class/shape preservation.

The provider-backed tests additionally exercise real/complex `double` and
`single`, rectangular/wide/empty LU and QR, packed and multi-output forms,
economy and pivot flags, one-based vector permutations, upper/lower Cholesky,
non-positive and non-real-diagonal status, determinant parity/singularity,
provider injection, COW independence, structured errors, and cancellation.
Spectral tests add tall/wide/square/empty full and economy SVD, legacy zero and
singular-value vector forms, complex-single factors, ordinary eig value/matrix/
vector and left/right forms, exact realness demotion, input-precision rank
thresholds, all four condition norms, exact singularity, shaped empties, COW,
errors, and cancellation.

Twenty-three `builtin_` conformance cases are self-contained and require no shared
index edit. The eight second-tranche cases add statistics values/classes,
stable sort plus indices, typed concatenation, column-major N-D concatenation,
and double/single sequence-rounding behavior. Four third-tranche cases compare
LU/QR/Cholesky reconstruction, orthogonality, permutation identities, status,
determinant, precision, and shapes rather than provider-dependent QR signs.
Six fourth-tranche cases compare SVD reconstruction/unitarity, eig left/right
residuals and trace/determinant invariants, rank thresholds, condition norms,
precision, and shapes rather than provider-dependent signs, phases, or order.
Every numeric payload remains at or below the kernel's eight-element
full-preview boundary. Their checked-in observations were produced by MATLAB
R2022b; both the focused oracle and real OpenMat fourth-tranche selections pass.

## Deferred work

Non-numeric aggregate/object overloads of concatenation remain outside this
core numeric tranche. Complex sequence-count successes may carry a warning in
R2022b; OpenMat's current warning/observation interface cannot represent that
ordered warning independently of the successful value.

`inv` remains unregistered because exact-singular R2022b behavior is a computed
value plus warning; the runtime built-in result/output path cannot yet carry a
warning-bearing success, and current solve methods discard the singular inverse
payload as an error. The design document specifies both the ordered normalized
warning channel and inverse-result metadata required before implementation.

Ordinary eig does not claim generalized `eig(A,B)`, explicit balance control,
or Hermitian-specific sorting because the current provider request does not
express those operations. They are rejected rather than mapped to ordinary eig.
