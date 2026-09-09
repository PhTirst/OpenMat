# Core numeric built-ins at the MATLAB R2022b boundary

Status: both provider-independent tranches plus provider-backed `lu`, `qr`,
`chol`, `det`, `svd`, `eig`, `rank`, and `cond` are implemented. `inv` remains
blocked on warning-bearing success and inverse-result contracts.

This design targets the baseline `pre-publication baseline`.
It records clean-room behavior measured with a locally installed, licensed
MATLAB R2022b. Temporary probes lived under the operating-system temporary
directory and printed only OpenMat-authored labels, classes, shapes, numeric
values, pivot vectors, residuals, and error identifiers. No MATLAB source,
tests, documentation, or diagnostic prose is copied here. Octave was not used
as a compatibility oracle.

## 1. Provider-independent scope

The following names are registered only because their implemented overloads
cover the applicable R2022b class, shape, complex, `single`, integer, logical,
empty, dimension, and multi-output boundaries measured for this tranche:

| Family | Implemented names | Observable boundary |
| --- | --- | --- |
| Rounding | `fix`, `floor`, `ceil` | `double` and `single` preserve class and shape; complex inputs are component-wise and demote when the rounded result is wholly real; integers preserve exact storage; logical and char inputs return `double`; shaped empties survive. |
| Elementary | `exp`, `log`, `log10`, `sin`, `cos`, `tan` | Real/complex `double` and `single` preserve class and shape; negative real logarithms use the complex principal branch; shaped empties survive; logical, integer, and char inputs are rejected. |
| Logical reductions | `any`, `all` | All numeric, logical, char, and integer storage is accepted; default dimension, scalar dimension, `vecdim`, and `'all'` are supported; `NaN` elements are omitted; empty identities are false for `any` and true for `all`, with the exact reduced shape. |
| Search | `find` | One to three outputs, positive `k`, `'first'`/`'last'`, row-vector orientation, N-D trailing-subscript collapse, exact value class, complex values, `NaN` as nonzero, and shaped empties are covered. |
| Matrix selection | `diag`, `triu`, `tril` | `double`, `single`, complex, logical, char, and all fixed-width integer storage is preserved; signed diagonal offsets and empty matrices are covered; matrix forms reject significant dimensions above two. |
| Rearrangement | `squeeze`, `permute`, `repmat` | Numeric, complex, logical, char, and integer storage is preserved; transformations are column-major; singleton and zero extents are exact; dimension arguments accept real integer `double`, `single`, logical, and exact integer values without routing exact integers through `f64`. |
| Statistics | `mean`, `median`, `std`, `var` | Default dimension, scalar dimension, unique `vecdim`, `'all'`, `includenan`/`omitnan`, empty reductions, complex values, and native binary32 accumulation are covered. `mean` also implements `default`, `double`, and `native` output modes; `std`/`var` implement zero/one normalization. |
| Ordering | `sort` | Stable column-major sorting supports default or scalar dimension, ascending/descending direction, one- or two-output forms, `MissingPlacement`, and `ComparisonMethod`; `double`, `single`, complex, logical, char, and fixed-width integer classes are preserved. |
| Concatenation | `cat`, `horzcat`, `vertcat` | Checked N-D shape growth, column-major copying, typed empties, and R2022b numeric/logical/char dominance are covered. The first fixed-width integer class dominates numeric peers, char dominates nonlogical real numeric inputs, and either single input makes a floating result single. |
| Sequences | `linspace`, `logspace` | Scalar double/single/logical/char endpoints, mixed-precision dominance, complex endpoints and counts, default or explicit counts, fractional/nonpositive/NaN/infinite count behavior, exact row shape, and the `logspace(...,pi,...)` endpoint rule are covered. |
| Decimal rounding | `round` | Unary and digits forms, `decimals`/`significant`, decimal-half correction for double, binary32-aware single rounding, component-wise complex values, signed zero, shaped empties, and unary logical/char/integer class rules are covered. |

`find(A)` is a row only when `A` is a row vector. Otherwise it is a column.
For N-D inputs, `[i,j] = find(A)` collapses source dimensions 2 through N into
`j`. The third output retains the input value class, while `i` and `j` are
`double`. A complex value is nonzero when either component is nonzero.

`diag(v,k)` creates a square matrix of order `numel(v)+abs(k)`. `diag(A,k)`
returns a column. Measured `diag(zeros(0,3))`, `diag(zeros(3,0))`, and `diag([])`
produce `0x1`, `0x1`, and `0x0`, respectively. `triu` and `tril` demote a
complex result to real storage when every retained imaginary component is zero,
matching the measured R2022b result.

`squeeze` retains at least two dimensions and keeps an existing row vector a
row; a `1x1xN` value becomes `Nx1`. `permute` requires a complete one-based
permutation and may append singleton axes. `repmat(A,n)` and a one-element
repeat vector repeat both first dimensions. Measured negative integer repeat
counts become zero extents; fractional and non-finite counts are rejected.

## 2. Second-tranche compatibility details

Statistics use one reduction planner and preserve the unreduced extents while
replacing every selected extent with one. Dimensions beyond the current rank
are accepted as no-op reductions; duplicate or empty `vecdim` values are
rejected. Missing complex values are those with a NaN component. Double output
uses binary64 accumulation, ordinary/native single output uses binary32
accumulation, and `mean(single(...),'double')` widens before accumulation.
Integer `mean(...,'native')` uses checked exact sums and R2022b midpoint
rounding. Measured logical native mean uses logical accumulation followed by a
double division, including its non-arithmetic result for multiple true values.

`median` orders complex values by magnitude and then phase. It preserves every
fixed-width integer class and logical storage. Char has an unusual measured
boundary: an odd reduction count returns char, an even count returns the
binary64 midpoint, and empty char is rejected. `std` and `var` reject integer
storage, preserve single for single input, and always return real dispersion
for complex input using squared magnitudes. A singleton has zero dispersion
under either normalization; an empty or all-omitted group returns NaN.

`sort` is stable even for duplicate values and signed zeros. Real ascending
sort places missing values last and descending sort places them first unless
`MissingPlacement` overrides that position. Complex automatic/absolute order
uses magnitude followed by `atan2` phase; `ComparisonMethod='real'` compares
real then imaginary components. The index output is one-based double with the
same shape. A dimension beyond rank returns the input order and all-one
indices.

Concatenation validates every nonconcatenated extent, including zero extents,
before allocating. The concatenation axis may append singleton dimensions.
Numeric conversion uses constructor-compatible rounding and saturation for a
dominant integer or char class; complex values select the dominant floating
precision. The registered built-ins intentionally reject string, cell, struct,
and object inputs: those are aggregate/object concatenation overloads rather
than the core numeric boundary, and require class-aware scalar conversion plus
language-copy semantics. They are not silently coerced to numeric storage.

`linspace` and `logspace` floor finite counts, return `1x0` for nonpositive or
negative-infinite counts, and return a one-element NaN for a NaN count. A count
of one returns the second endpoint. For a complex count, the floored real part
selects length and the floored imaginary part participates in interpolation;
this also reproduces the measured finite/NaN/infinite imaginary-count results.
R2022b can attach a warning to some complex-count successes, but the current
warning channel cannot observe it. `round` requires positive significant
digits, accepts negative decimal digits, and does not expose a `TieBreaker`
name-value in R2022b.

## 3. Decomposition audit: measured R2022b behavior

`lu`, `qr`, `chol`, `svd`, `eig`, `rank`, `cond`, `det`, and `inv` accept real
or complex `double` and `single` matrices. Representative logical and `int8`
inputs were rejected, as were significant dimensions above two. `det`, `inv`,
`chol`, and ordinary `eig` additionally require square inputs. The following
multi-output contract was measured on real, complex, `single`, and empty
representatives.

### 3.1 Factorizations

For `A` of shape `m x n`, let `k=min(m,n)`.

| Call | Measured output contract |
| --- | --- |
| `X = lu(A)` | Packed LU in an `m x n` array. |
| `[L,U] = lu(A)` | `L` is `m x k`, `U` is `k x n`, and `A=L*U`; row permutation is folded into `L`. |
| `[L,U,P] = lu(A)` | `P*A=L*U`; `P` is `m x m`. Numeric matrices, including `P`, have the input precision. |
| `[L,U,p] = lu(A,'vector')` | `A(p,:)=L*U`; `p` is a one-based `1 x m double` row even for `single` input. |
| `[Q,R] = qr(A)` | Full `Q` is `m x m`; `R` is `m x n`; `A=Q*R`. |
| `[Q,R,E] = qr(A)` | Column-pivoted result with `A*E=Q*R`; `E` is `n x n` and has input precision. |
| `[Q,R,p] = qr(A,'vector')` | `A(:,p)=Q*R`; `p` is a one-based `1 x n double` row. |
| `[Q,R] = qr(A,0)` | Economy result; for measured `m>=n`, `Q` is `m x k` and `R` is `k x n`. |
| `R = qr(A)` / `R = qr(A,0)` | One output is `R`; default QR keeps `m` rows while `0` or `'econ'` keeps `k` rows. |
| `[Q,R,p] = qr(A,0)` | Economy QR with a one-based permutation vector; `[Q,R,E]=qr(A,'econ')` instead returns a permutation matrix. Explicit `'matrix'`/`'vector'` may follow `'econ'`, but cannot be combined with numeric `0`. |
| `R = chol(A)` | Upper-triangular factor; a non-positive-definite input is an error. |
| `[R,p] = chol(A)` | `p` is `0` on success. On failure it is the one-based first failing leading-minor index and `R` is exactly `(p-1) x (p-1)`; this form does not throw for non-positive-definiteness or a non-real complex diagonal. `p` is scalar `double`, including for `single` input. |
| `[L,p] = chol(A,'lower')` | Lower-triangular conjugate form with the same status convention. |

For `zeros(0,3)`, three-output LU has shapes `L=0x0`, `U=0x3`, `P=0x0`;
three-output QR has `Q=0x0`, `R=0x3`, and `E=eye(3)`. Empty Cholesky
returns a `0x0` factor and scalar status zero.

### 3.2 Singular values and eigenvalues

| Call | Measured output contract |
| --- | --- |
| `s = svd(A)` | Real singular-value column of length `k`, in input precision. |
| `[U,S,V] = svd(A)` | Full factors with `A=U*S*V'`; shapes are `m x m`, `m x n`, `n x n`. Two outputs return the same `U,S` shapes. |
| `[U,S,V] = svd(A,'econ')` | Thin factors for tall and wide inputs: `m x k`, `k x k`, `n x k`. |
| `[U,S,V] = svd(A,0)` | Legacy economy form: thin only when `m>n`; when `m<=n`, including `0xN`, it has full shapes. |
| `[U,s,V] = svd(A,'vector')` | Full vectors with a `k x 1` singular-value column. `'matrix'` is the explicit matrix form; either selector composes with `'econ'`, and a trailing selector composes with numeric `0`. |
| `d = eig(A)` | `n x 1` eigenvalue column, complex when required. |
| `[V,D] = eig(A)` | Right eigenvectors in `V`; eigenvalues on diagonal of `D`. |
| `[V,D,W] = eig(A)` | `W` contains left eigenvectors and is the third output. |
| `[V,d] = eig(A,'vector')` | Right eigenvectors followed by an eigenvalue column rather than a diagonal matrix. |

All SVD factor outputs preserve `single`; singular values and `S` are real even
for complex input. Eigenvectors and eigenvalues become complex when a real
matrix has non-real eigenpairs and still preserve `single`; outputs whose
components are all exactly real use real storage. For `zeros(0,3)`, full and
numeric-zero SVD return `U=0x0`, `S=0x3`, `V=eye(3)`, while `'econ'` returns
`U=0x0`, `S=0x0`, `V=3x0`. For `zeros(3,0)`, full SVD retains a `3x3` `U`,
whereas both economy spellings return `U=3x0`. One-output empty eig is `0x1`;
its vector matrices and diagonal eigenvalue matrix are `0x0`.

Singular-vector signs/phases and eigenvalue order are not stable semantic
values. Future differential tests must compare reconstruction, orthogonality,
eigen residuals, dimensions, class, and consistent pairing instead of requiring
the provider to reproduce one vendor's exact vectors.

### 3.3 Derived functions and tolerances

- `rank(A)` returns scalar `double`, including for `single` input. Its default
  threshold is `max(size(A))*eps(max(s))` in the input precision and counts
  singular values strictly greater than the threshold. On
  `diag([1,2*eps(1)])` the measured rank was one; replacing `2` with `5` made
  it two. A real `double`, `single`, or integer scalar tolerance replaces this
  threshold; negative, NaN, and infinity retain their strict-comparison
  consequences, while logical and complex tolerances are rejected. Empty
  `0x3` has rank zero. A matrix element containing NaN or infinity is a domain
  error before provider dispatch.
- Default `cond(A)` is the two-norm singular-value ratio and preserves
  `single` for `single` input. Exact singular `diag([1,0])` gives infinity,
  while `cond(zeros(0,3))` gives zero in the input precision. Numeric `1` and
  positive infinity, plus `'inf'` and `'fro'`, are implemented for square
  matrices as `norm(A,p)*norm(inv(A),p)` using a four-precision provider solve;
  an exact-singular solve maps directly to infinity and carries no warning.
  All norms return zero for `0x0`; non-2 norms reject non-square empties and
  nonempty rectangular matrices. A mathematically
  rank-deficient matrix can still produce a finite large value when the
  computed smallest singular value is nonzero; the provider result, not a
  symbolic rank guess, controls this boundary.
- `det(A)` preserves input precision, returns zero for an exactly singular
  matrix, and returns one for `0x0`.
- `inv(A)` preserves input precision and complexness and returns `0x0` for
  `0x0`. Exact singular input produced a value together with warning identifier
  `MATLAB:singularMatrix`, rather than a provider error alone. The successful
  value depends on the factorization: measured examples included diagonal
  infinities for zero/diagonal singular matrices and dense infinities for a
  rank-one matrix, so a built-in cannot substitute one generic infinite array.

## 4. Integrated factorization and invocation contracts

`LinalgProvider` on this baseline has object-safe `f32`, `f64`, `complex32`,
and `complex64` entry points for LU, explicit QR, Cholesky, SVD, ordinary eig,
GEMM, and solve. The request types borrow canonical column-major arrays and the
runtime cancellation flag. Owned results carry rectangular packed LU,
permutations and status metadata, explicit QR factors, Cholesky factors,
descending real singular values with optional `U`/`V^H`, and explicit complex
ordinary eigenvalues with optional left/right column vectors. The native
binary32 provider path already exists and is never widened through binary64
storage.

`BuiltinContext` now borrows the selected `&dyn LinalgProvider` and exposes the
same atomic cancellation flag used by provider requests. `BuiltinContext::new`
retains a static reference-provider default for standalone callers; the
interpreter clones its session `Arc<dyn LinalgProvider>` before creating the
object-aware language-copy closure, then borrows that clone for the invocation.
A mock-provider interpreter test proves built-in dispatch reaches the injected
provider rather than a fixed `ReferenceProvider`.

`lu`, `qr`, and `chol` call one corresponding provider method and construct only
language-level shapes and permutations. `det` calls LU once, returns exact zero
when `first_zero_pivot` is present, returns one for `0x0`, and otherwise
multiplies the LU diagonal with swap parity. No built-in copies a decomposition
algorithm or refactors solely to recover metadata. Complex results whose
imaginary components are all zero are demoted to real storage as measured.

## 5. Integrated spectral and remaining inverse contracts

`svd` and `eig` consume the integrated four-precision provider contract:

```text
SvdRequest<T> { matrix, vectors: None|Full|Thin, cancellation }
SvdResult<T>  { singular_values: DenseArray<Real<T>>, u: Option<_>,
                vh: Option<_> }
EigRequest<T> { matrix, left_vectors, right_vectors, cancellation }
EigResult<T>  { eigenvalues: DenseArray<Complex<Real<T>>>,
                left_vectors: Option<_>, right_vectors: Option<_> }
```

`SvdResult` returns `V^H` at the provider boundary; the built-in performs the
checked conjugate transpose and assembles either a diagonal `S` or singular
value column. Real-input eig is normalized into paired complex eigenvalues and
vectors before reaching builtins. `rank` and two-norm `cond` each use one
no-vectors SVD call. One/infinity/Frobenius `cond` uses one provider solve with
an identity right-hand side and stable language-layer norms; it does not copy
an inverse or decomposition algorithm into builtins. Structured statuses
distinguish cancellation, allocation, invalid provider argument, provider
failure, no convergence, and exact singularity.

Ordinary `eig(A)` intentionally exposes only the default provider balancing and
`'matrix'`/`'vector'` output choice. Generalized `eig(A,B)`, explicit
`'balance'`/`'nobalance'`, and Hermitian-specific sorting selectors are rejected
because the current request has no generalized, balance-control, or sorted
Hermitian contract; none is silently mapped to ordinary eig.

`inv` additionally needs a four-precision inverse result that retains the
computed matrix even when an exact-zero pivot makes the outcome warning-bearing:

```text
InverseRequest<T> { lu: &LuResult<T>, cancellation }
InverseResult<T>  { inverse: DenseArray<T>, exact_zero_pivot: Option<u64> }
```

Current `solve_*` returns an error and no value on exact singularity, so it
cannot reproduce the measured singular inverse payload. A built-in must not
invent that payload or run a private fallback inversion.

## 6. Warning and conformance boundary

The protocol diagnostic model has warning severity, but runtime built-ins do
not. `BuiltinResult` is only `Result<Vec<Value>, BuiltinError>`, and
`OutputEvent` contains only display events. Consequently the interpreter cannot
return a successful inverse and an ordered normalized warning to the kernel;
the conformance observation schema also has no warning list.

The required shared change is a successful built-in outcome containing
`values` plus ordered `BuiltinWarning` records. At minimum the record needs a
stable OpenMat category such as `matrix-singular` and the operation name; the
interpreter must attach the call source location, and the kernel must map it to
a warning diagnostic with a stable OpenMat code without copying provider or
MATLAB prose. Conformance observations then need an ordered warning-category
array independent of the successful numeric payload. Until both this channel
and `InverseResult` exist, `inv` remains unregistered rather than treating a
singular success as an error or silently succeeding without its warning.

For numeric factorization cases, exact class, size, real/complex status,
permutation identities, and empty shapes remain exact. Residual-based `double`
checks should begin around `1e-12`, and `single` around `5e-6`, scaled by matrix
norm and dimensions. Rank thresholds are never relaxed through comparator
tolerance.
