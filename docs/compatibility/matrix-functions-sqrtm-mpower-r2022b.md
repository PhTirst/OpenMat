# `sqrtm` and matrix power observations for MATLAB R2022b

## Status and scope

This note records black-box observations made against the installed MATLAB
R2022b before implementing the reusable matrix-function service, then records
the resulting compatibility boundary. The probe was locally authored and
inspected only public input/output behavior. It did not copy MATLAB source,
tests, documentation, or diagnostic messages.

The implementation is complete for finite floating-point inputs covered below.
It uses a complex Schur factorization and never substitutes an eigenvector
formula such as `V * f(D) / V`, which would fail for defective matrices and be
unreliable for ill-conditioned eigenvector bases.

## Probe environment

- executable: `$env:MATLAB_EXE`
- reported version: `9.13.0.2049777 (R2022b)`
- release: `2022b`
- invocation: a self-authored script executed with MATLAB `-batch`

All numeric matrices below use ordinary MATLAB column-major construction. Error
observations retain only stable identifiers, not MATLAB message text.

## `sqrtm` observations

### Input and result behavior

| Input | Observed result |
| --- | --- |
| `[]` | real `double` `0x0`; two-output residual is `NaN` |
| positive real scalar | real principal square root, preserving `single` or `double` |
| negative real scalar | positive-imaginary principal root, preserving the floating class |
| positive real diagonal matrix | real diagonal principal root |
| real diagonal matrix with a negative entry | complex principal root |
| `[0 -1; 1 0]` | real principal root with relative residual near binary64 epsilon |
| positive Jordan block `[4 1; 0 4]` | real root `[2 0.25; 0 2]` |
| nilpotent Jordan block `[0 1; 0 0]` | non-finite real result containing `Inf`; residual `NaN`; no warning identifier |
| `[1 1e8; 0 1]` | real root `[1 5e7; 0 1]` |
| non-square `2x3` | `MATLAB:sqrtm:inputMustBeSquare` |
| empty non-square `0x3` | `MATLAB:sqrtm:inputMustBeSquare` |
| input containing `NaN` or `Inf` | `MATLAB:sqrtm:inputMustBeFinite` |
| `int32` matrix | `MATLAB:sqrtm:inputType` |
| non-real floating input | complex result preserving `single` or `double` |

The negative-scalar and negative-diagonal cases establish the principal branch:
for example, `sqrtm(-4)` is `+2i`, and the negative unit eigenvalue maps to
`+i`. This is matrix-function behavior, not element-wise `sqrt` dispatch.

### Output count and residual

`nargout('sqrtm')` reports `3`. A call that discards all outputs succeeds, as do
calls requesting one, two, or three outputs. Four outputs fail with
`MATLAB:TooManyOutputs`.

The meaning of output two depends on the requested output count. With exactly
two outputs,

```matlab
[X, resnorm] = sqrtm(A)
```

the observed value is

```matlab
norm(X * X - A, 1) / norm(A, 1)
```

For `A = [1 2; 3 4]`, both the returned value and that expression were
`5.9336629442368287e-16`. The relative Frobenius and infinity-norm residuals
were different. For the zero-order empty matrix the denominator convention
produces `NaN`.

When three outputs are requested, output two is not the two-output residual. For
the same matrix, two-output result two was `5.9336629442368287e-16`, while the
three-output results two and three were `1.1306507024215573` and
`2.8477505624022008`. A compatible built-in therefore has to branch on requested
output count and must not label the three-output second value as `resnorm`.
Additional self-authored evaluations established that the stability factor is
`norm(X, 1)^2 / norm(A, 1)`. The third output is the relative one-norm condition
estimate formed from the inverse Frechet operator
`I kron X + transpose(X) kron I`.

## Non-integer matrix power observations

For a non-scalar square floating matrix and scalar exponent, `A ^ p` applies a
matrix function. It is not element-wise power. Integer exponents retain the
existing repeated-multiplication/solve semantics.

| Case | Observed result |
| --- | --- |
| `[4 1; 0 9] ^ 0.5` | `[2 0.2; 0 3]` to roundoff |
| the same matrix to `1.5`, `-0.5`, or `0.25` | principal matrix powers, real for this spectrum |
| `diag([-1 4]) ^ 0.5` | complex principal result `diag([i 2])` |
| `diag([-1 8]) ^ (1/3)` | first diagonal `0.5 + sqrt(3)/2*i`, second diagonal `2` |
| complex triangular matrix to `0.5` | complex principal matrix power |
| `[4 1; 0 4] ^ 1.5` | Jordan derivative term is preserved: `[8 3; 0 8]` |
| `[0 1; 0 0] ^ 0.5` | non-finite real result containing `Inf`; no warning identifier |
| `[] ^ 0.5` | real `double` `0x0` |
| non-square matrix to `0.5` | `MATLAB:mpower:notScalarAndSquareMatrix` |
| matrix containing `NaN` or `Inf` to `0.5` | a same-size matrix of `NaN`, rather than an input-validation error |
| integer-class matrix to `0.5` | `MATLAB:mpower:integerNotSupported` |
| either operand is `single` | matrix result uses `single` storage |
| complex scalar exponent | supported and follows the principal complex branch |

The integer probes `A^2`, `A^0`, and `A^-1` produced the product, identity, and
inverse respectively. The existing integer route must remain ahead of the
matrix-function route so its multiplication and provider-solve behavior does
not change.

## Implemented numerical architecture

The reusable service consumes a unitary Schur factorization

```text
A = Q T Q^H
```

and evaluates the principal function on triangular `T`, including repeated
eigenvalues without dividing by an eigenvector matrix. Real inputs are promoted
to complex Schur form so negative-real eigenvalues can follow the principal
branch; exactly real results are canonicalized back to real storage.

- `sqrtm` uses the triangular square-root recurrence followed by
  `Q * sqrt(T) * Q^H`; its two-output residual is computed independently in the
  matrix one-norm.
- General non-integer `A^p` uses a principal triangular logarithm by inverse
  scaling and repeated square roots, followed by a scaled matrix exponential.
  `p == 0.5` uses the direct square-root route. Diagonal Schur forms use scalar
  principal powers directly.
- Binary32 uses native complex32 Schur factorization, performs the triangular
  recurrence in binary64, and rounds once to complex32 output.
- Exact integer exponents are detected before this service and continue through
  the pre-existing exponentiation-by-squaring and provider-solve path.

The provider-neutral API is:

```rust
pub struct SchurRequest<'array, C> {
    pub matrix: &'array DenseArray<C>,
    cancellation: Option<&'array AtomicBool>,
}

impl<'array, C> SchurRequest<'array, C> {
    pub const fn new(matrix: &'array DenseArray<C>) -> Self;
    pub const fn with_cancellation_flag(self, flag: &'array AtomicBool) -> Self;
    pub fn check_cancellation(&self) -> Result<(), LinalgError>;
}

pub struct SchurResult<C> {
    pub form: DenseArray<C>,
    pub vectors: DenseArray<C>,
}

pub trait LinalgProvider {
    fn schur_complex32(
        &self,
        request: SchurRequest<'_, Complex32>,
    ) -> Result<SchurResult<Complex32>, LinalgError>;

    fn schur_complex64(
        &self,
        request: SchurRequest<'_, Complex64>,
    ) -> Result<SchurResult<Complex64>, LinalgError>;
}
```

Both methods return `A = Q*T*Q^H`, with `form = T` upper triangular and
`vectors = Q` unitary, without sorting or promising eigenvalue order. Validation
requires a rank-two square input and checked allocation. Cancellation is checked
before and after the native call. A negative LAPACKE `info` maps to
`InvalidProviderArgument`; a positive `info` maps to `NoConvergence` with the
unconverged index/count retained when LAPACK defines one. The native provider
must not silently fall back to `geev`.

The Windows LP64 OpenBLAS 0.3.34 distribution pinned by the repository was
downloaded and hash-checked during implementation. Its export table contains
both `LAPACKE_cgees` and `LAPACKE_zgees`, and provider tests executed both
symbols against the real DLL. Binding requires both symbols and reports a
structured missing-symbol error; there is no `geev` or reference fallback.

## Remaining boundaries

- The reference provider exposes its existing Hessenberg plus shifted-QR Schur
  implementation for deterministic tests. Production OpenBLAS uses LAPACKE
  GEES, which is the numerically authoritative path.
- The three-output condition estimate explicitly builds and solves the
  `n^2`-by-`n^2` Frechet operator. This matches the probed one-norm semantics but
  has `O(n^4)` storage and dense-solve cost, so it is intended for compatibility
  rather than large-matrix throughput.
- A singular, non-diagonal Schur form requiring the general logarithm can fail
  with a structured singular-matrix error. The directly measured singular
  square-root path remains supported and produces the observed non-finite
  result.
- Non-finite matrix bases follow the measured `A^p` behavior by returning a
  same-shape NaN matrix before Schur factorization. Non-finite exponents are not
  part of the committed conformance surface.
