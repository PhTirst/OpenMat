# Runtime to linear-algebra bridge

The runtime bridge is intentionally a narrow Rust-internal boundary. It converts
dynamic `Value` operands into the typed, column-major matrices already accepted
by `openmat-linalg`. Standalone bridge calls receive a borrowed
`&dyn LinalgProvider`; each `Interpreter` now owns an injected
`Arc<dyn LinalgProvider>` and borrows that provider for dispatched operations.
There is no process-global provider.

## API and value mapping

`openmat-runtime` exports four bridge functions:

- `linalg_matrix_multiply(provider, left, right)` performs ordinary GEMM.
- `linalg_square_solve(provider, coefficients, right_hand_side, cancellation)`
  performs the original binary64 square general solve and accepts an optional
  borrowed `AtomicBool` cancellation flag.
- `linalg_matrix_left_divide(provider, coefficients, right_hand_side,
  cancellation)` selects square, overdetermined, or underdetermined-basic solve.
- `linalg_matrix_right_divide(provider, left, right, cancellation)` evaluates
  right division through a conjugate-transposed left solve.

Matrix multiplication and division accept homogeneous binary64 or binary32
operands. Binary64 representations are `Value::Double`, `Value::Complex`,
`Value::Array(ArrayData::F64)`, and
`Value::Array(ArrayData::ComplexF64)`. Binary32 representations are
`Value::Array(ArrayData::F32)` and
`Value::Array(ArrayData::ComplexF32)`. Scalar-optimized double values
become temporary `1x1` matrices for a direct bridge call. Explicit arrays must
have canonical rank two; zero extents such as `0xN`, `Nx0`, and `0x0` remain
unchanged. Array inputs are borrowed directly and are never mutably accessed.

When both operands are real, the bridge calls the real provider operation for
their precision. If either operand is complex, every real operand is promoted
within that same precision by copying the real component and setting the
imaginary component to positive zero. There is no complex-to-real demotion,
integer conversion, or binary32/binary64 conversion. Left division selects the
matching square, rectangular, or underdetermined-basic typed provider route.
Right division performs same-precision transpose or conjugate-transpose
storage mappings around that left solve.

Binary64 results are returned as explicit real or complex `Value::Array`
matrices, even for `1x1` direct bridge results. Binary32 results remain
`Value::Array` with f32 or `Complex32` storage. GEMM writes into
bridge-allocated output storage, and solve contracts require independently
owned results. Therefore successful results do not share input COW buffers,
while cloning or borrowing an input before the call remains valid and does not
detach it.

## Interpreter construction and multiply dispatch

The existing `Interpreter::new`, `Interpreter::with_registry`, and
`Interpreter::with_components` APIs remain source-compatible and install an
`Arc<ReferenceProvider>`. Hosts that already selected a provider can use
`Interpreter::with_linalg_provider`, or the full
`Interpreter::with_components_and_linalg_provider` constructor. The
`linalg_provider` accessor exposes the retained shared owner for inspection or
further sharing.

After object-operator selection, `array_ops` sends `BinaryOperator::Multiply`
to `linalg_matrix_multiply` only when both operands are non-scalar, canonical
rank-two matrices of the same supported precision. Valid real products call
`gemm_f64` or `gemm_f32`; a product with either complex operand calls
`gemm_complex64` or `gemm_complex32` after same-precision promotion of any real
input. This route includes shaped empty matrices and allocates independent
output storage.

The dispatch predicate is deliberately exact. Scalar `*` (including a scalar
array combined with a matrix), `.*`, mixed single/double multiplication,
logical-matrix multiplication, higher-rank rejection, and object operator
dispatch retain their previous runtime paths and semantics. The old runtime
loop is no longer used for ordinary homogeneous two-dimensional double or
single matrix multiplication; it remains only for the pre-existing fallback.

Non-scalar `A\B` and `L/R` dispatch to the division bridges before element-wise
single handling. Scalar and `1x1` values retain the bytecode-v16 scalar
boundary, and `ElementDivide`/`ElementLeftDivide` never enter a provider.

## Error and cancellation boundary

`RuntimeLinalgError::Linalg` owns the original structured `LinalgError`.
Dimension, rank, square-shape, allocation, singularity, cancellation, provider
argument, and provider execution details are not converted to text. The other
bridge error variant records an unsupported dynamic operand with its
`ValueKind` and optional exact `DType`.

Interpreter dispatch converts this error narrowly. `LinalgError::Cancelled`
becomes the existing `RuntimeErrorKind::Cancelled`. Every other
`RuntimeLinalgError` is retained as
`ArrayRuntimeError::LinearAlgebra` beneath the existing structured
`RuntimeErrorKind::InvalidExecutionState` envelope. This avoids a new
workspace-wide top-level error variant while preserving dimension, provider,
operation, argument, and provider-detail fields for runtime callers. The
interpreter adds the failing instruction's source location and complete bytecode
stack after this conversion; it never rebuilds an error from its display text.
`RuntimeError::linalg_error` and `RuntimeErrorKind::linalg_error` provide a
stable inspection path through the compatibility envelope.

The optional solve cancellation flag is borrowed into `SolveRequest`. Reference
providers can poll it during factorization and substitution. Native providers
retain their existing pre-call/post-call polling limitations; this bridge does
not claim that a blocking LAPACK call becomes interruptible.

GEMM requests do not currently carry a cancellation flag. The interpreter
checks its token at the instruction boundary before provider dispatch, and a
provider-reported `LinalgError::Cancelled` crosses back as runtime cancellation.
An already-cancelled interpreter therefore does not enter the provider. A
blocking native GEMM remains subject to that provider's own cancellation
limitations.

## Remaining boundary

This integration covers ordinary homogeneous two-dimensional double and single
matrix `*`, `\`, and `/`. It deliberately does not implement or route matrix
power or dispatch the standalone `linalg_square_solve` API directly from
bytecode. Mixed single/double matrix result-class policy and warning-bearing
singular or rank-deficient successful results remain separate language
contracts. OpenBLAS DLL discovery, selection, loading, and deployment also
remain outside the runtime bridge; the provider accepts only the existing
explicit absolute-path opt-in.
