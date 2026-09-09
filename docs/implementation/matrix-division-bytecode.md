# Matrix-division bytecode v16 foundation

Status: bytecode, compiler, scalar/element runtime, and dense matrix numerical
routing are implemented. The remaining compatibility gap is warning-bearing
success for rank-deficient, singular, and nearly singular systems.

## Versioned operator contract

Bytecode version `16.0` adds two `BinaryOperator` values without changing the
meaning of the existing right-division values:

| Source spelling | Bytecode operator | Logical discriminant |
| --- | --- | ---: |
| `/` | `Divide` | `4` |
| `./` | `ElementDivide` | `5` |
| `\` | `LeftDivide` | `14` |
| `.\` | `ElementLeftDivide` | `15` |

The complete v15 operator vocabulary retains logical discriminants `0` through
`13`. The v16 additions use new values `14` and `15`. The bytecode model exposes
explicit encode/decode helpers for these format-level values. A serializer must
use those helpers and must never cast the Rust enum, inspect
`mem::discriminant`, or infer a wire value from declaration order.

The verifier accepts exact bytecode version `16.0`. Version `15.0` artifacts are
rejected with the existing structured `UnsupportedVersion` error; there is no
implicit upgrade or operator guessing.

## Compiler lowering

HIR lowering preserves source operand order for all four division operators.
For a source expression `lhs \ rhs`, the `Binary` instruction therefore names
the register produced for `lhs` in its `lhs` operand and the register produced
for `rhs` in its `rhs` operand. The compiler emits `LeftDivide` directly and
does not exchange registers or disguise the operation as `Divide`.

`ElementLeftDivide` follows the same rule. Every emitted `Binary` instruction
retains the complete expression `SourceLocation`, including the source
identifier and half-open byte range. This makes division direction available
to runtime numerical dispatch, object-method vocabulary, diagnostics, and
future serialized tooling.

## Runtime foundation

The runtime implements direction at the arithmetic operation, not in compiler
lowering:

```text
a \ b   = b / a       when both operands are scalar numerics
a .\ b  = b ./ a      with the existing scalar-expansion shape rules
```

Scalar double, complex double, and logical operands use the existing numeric
acceptance and result-class boundaries. `ElementLeftDivide` also accepts the
same dense double/complex/logical array and scalar combinations accepted by
`ElementDivide`; it selects the same output shape that `rhs ./ lhs` would
select and computes every element in that direction.

Single and complex-single operands stay on the existing typed binary32 path.
`ElementLeftDivide` reuses the precise real, complex scaling, mixed-double,
logical, cancellation, and output-demotion rules of `ElementDivide`, with only
the numerator and denominator roles reversed inside the arithmetic primitive.
A scalar/scalar `LeftDivide` with a single participant is supported on that
same path and returns single storage.

For double/complex/logical values, a scalar left coefficient with a non-scalar
right operand is the scalar case `rhs ./ lhs` and preserves the right operand's
shape. It is not sent to a matrix solver. A non-scalar numeric left coefficient
is a true matrix-left-division request. Dense real and complex `double` and
`single` matrices now route through the injected numerical provider; no such
request silently falls back to element-wise evaluation.

## Object vocabulary and gating

The bytecode-to-object vocabulary maps each direction and element kind
independently:

| Bytecode operator | Object operator | Conventional method |
| --- | --- | --- |
| `Divide` | `MatrixRightDivide` | `mrdivide` |
| `ElementDivide` | `ElementWiseRightDivide` | `rdivide` |
| `LeftDivide` | `MatrixLeftDivide` | `mldivide` |
| `ElementLeftDivide` | `ElementWiseLeftDivide` | `ldivide` |

This foundation does not broaden executable object dispatch. The interpreter
continues to enable only the previously accepted `Add` object path; an object
participating in any division still receives the existing structured invalid
operands result. The complete mapping is retained and tested so a later object
operator tranche cannot accidentally select a right-division method for a
left-division instruction.

## Numerical closure

The runtime integration described in `matrix-division-runtime.md` dispatches
square, overdetermined, and underdetermined-basic left division, and implements
right division through the conjugate-transpose relation. It preserves empty
shapes, structured dimension errors, cancellation, result class, and complex
storage. The ten warning-free matrix-division conformance cases pass against
the checked-in MATLAB R2022b observations.

Rank-deficient, singular, and nearly singular MATLAB-compatible successful
results remain deferred because observation v2 and the provider contract do
not yet carry warnings alongside a value.
