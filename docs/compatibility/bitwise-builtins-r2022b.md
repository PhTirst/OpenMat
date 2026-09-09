# MATLAB R2022b bitwise built-ins

Status: the clean-room implementation and focused tests are complete in
`crates/openmat-builtins/src/core_bitwise.rs`. Repository-level registration is
pending because this task is not permitted to modify
`crates/openmat-builtins/src/lib.rs`.

## Covered functions

The module implements `bitand`, `bitor`, `bitxor`, `bitshift`, `bitget`, and
`bitset`, including the legacy assumed-integer-type forms. The behavior below
was measured by executing OpenMat-authored probes in the installed MATLAB
R2022b (9.13.0) and is captured by the `builtin_bitwise_*` conformance cases.

## Input and output classes

- `bitand`, `bitor`, and `bitxor` accept real `double`, `logical`, and all eight
  fixed-width integer classes. Two logical operands produce logical output;
  mixing logical and double produces double output.
- Fixed-width integer binary operands normally have the same class. MATLAB's
  asymmetric exception permits the second operand to be a scalar double that
  is an exact integer in the first operand's class range. The output retains
  the first operand's integer class.
- `bitshift`, `bitget`, and `bitset` accept real double or a real fixed-width
  integer array as their first input. Their control inputs may be real double,
  single, or fixed-width integer arrays. Logical, character, and complex
  controls are rejected.
- All six functions preserve the primary numeric class. In particular,
  `bitget` returns zero or one in the class of its first input rather than
  returning logical.
- A double input uses an unsigned 64-bit word by default and must be a finite,
  integral value in `[0, 2^64)`. An assumed signed type permits negative double
  inputs in that signed type's range. Computed words are converted back to
  double using normal binary64 rounding.
- The assumed type can be one of `int8`, `uint8`, `int16`, `uint16`, `int32`,
  `uint32`, `int64`, or `uint64`, supplied as a character row or string scalar.
  With an exact integer first input it must match that input's class.

## Shape rules

`bitand`, `bitor`, and `bitxor` use R2022b implicit expansion independently in
every dimension. Singleton dimensions expand, including against a zero extent,
so `0x1` combined with `1xN` produces `0xN`.

`bitshift`, `bitget`, and `bitset` use a narrower rule: every participating
array is either scalar or has exactly the same canonical shape as every other
nonscalar input. They do not perform row-versus-column implicit expansion.
The first nonscalar shape is retained. Empty `0xN` shapes are retained, and
invalid scalar controls are still diagnosed when the result would be empty.

## Word operations

- Positive `bitshift` counts shift left and discard bits beyond the selected
  word width. Negative counts shift right. Signed right shifts copy the sign
  bit; unsigned and default-double right shifts insert zero bits.
- A shift whose magnitude is at least the word width produces zero, except
  that a sufficiently large right shift of a negative signed value produces
  all one bits (numeric `-1`).
- `bitget` and `bitset` positions are one-based and must lie in `1..=width`.
  The sign bit is an ordinary addressable bit for signed classes.
- Omitting `bitset`'s setting value means one. A supplied real numeric value is
  interpreted as clear only when it is zero; any nonzero value, including
  negative values, infinities, and NaN, sets the bit.

## Structured failures

The module uses the runtime's broad error categories without reproducing MATLAB
diagnostic prose:

- wrong argument or output counts: `ArgumentCount`;
- unsupported, complex, logical-control, or mixed integer classes: `Type`;
- nonintegral/out-of-range values, invalid bit positions, unknown/mismatched
  assumed types, incompatible shapes, and allocation/offset overflow: `Domain`;
- cooperative cancellation: `Cancelled`.

The current conformance normalizer maps MATLAB's position and size identifiers
to its broad `other` category, while mixed-class inputs normalize to
`type-error`. Focused Rust tests retain the more precise internal OpenMat
categories above.

## Allocation and cancellation

Output shapes and column-major broadcast offsets use checked arithmetic. Every
materialized vector reserves capacity with `try_reserve_exact`, host-length
conversion is checked, and dense construction errors remain structured. Input
validation and output mapping poll the invocation cancellation token at entry,
at exit, and every 4096 visited elements.

## Registration boundary

The module exposes one crate-private integration point:
`core_bitwise::register_bitwise(&mut BuiltinRegistry)`. Completing repository
integration requires exactly two edits in `crates/openmat-builtins/src/lib.rs`:
declare `mod core_bitwise;` with the other core modules, then call
`core_bitwise::register_bitwise(registry)?;` from `register_minimal`. Those edits
are intentionally absent from this task's commit because `lib.rs` is outside
its authorized paths.
