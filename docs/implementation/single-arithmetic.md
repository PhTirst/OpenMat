# Exact single element arithmetic

This tranche adds the first closed runtime path for MATLAB `single` element
arithmetic. It consumes and produces `ArrayData::{F32, ComplexF32}`
directly; a single-only operation never materializes a binary64 array or uses a
binary64 value as temporary storage.

## Supported operators

The runtime implements `+`, `-`, `.*`, `./`, and `.^`, plus `==`, `~=`, `<`,
`<=`, `>`, and `>=`. Addition and subtraction are element operations in the
existing bytecode model. Only the explicitly dotted multiply, divide, and power
operators enter this path. Matrix `*`, matrix `/`, and matrix `^` remain
structured `UnsupportedArrayOperator` results for the numerical-backend
tranche.

Both operands may be real or complex single. Double and logical scalars or
dense arrays are also accepted when the other operand is single. Other dynamic
classes remain invalid operands rather than being converted implicitly.

## Precision and result class

Locally authored black-box probes against the installed MATLAB R2022b establish
the following rules used by this implementation:

- single with single performs component arithmetic in binary32 and returns
  single;
- single with logical maps false and true to binary32 zero and one, performs
  binary32 arithmetic, and returns single;
- single with double preserves the double operand's precision for the operation
  and rounds each final component to binary32; the result class is still single;
- comparison always returns logical and preserves double precision when a
  double operand participates;
- a complex arithmetic result whose stored imaginary components all become
  zero is returned as real single, including an empty complex input; otherwise
  it remains complex single;
- a negative real base raised element-wise to a finite non-integral exponent
  promotes the complete result array to complex single. Integral and positive
  real powers stay real when every resulting imaginary component is zero.

The mixed-double rule does not widen or replace single storage. A binary32 input
component is exactly representable in binary64, is used with the actual double
operand, and only the result is converted into the required binary32 output
buffer. Complex single-only add, subtract, multiply, divide, and finite power
operate on `Complex32` components and binary32 elementary functions. Complex
multiply and divide retain the direct binary32 path for ordinary magnitudes and
use a binary32 scaling fallback only when an intermediate product or squared
magnitude would overflow or underflow. This preserves R2022b results such as
equal extreme operands dividing to one and extreme conjugates dividing to the
unit imaginary value.

## Shape, comparison, and interruption rules

Operands must have equal canonical shapes unless either operand has one
element. One-element scalar expansion preserves the other operand's complete
shape, including `0x0`, `0xN`, and N-D shapes. Two differently shaped empty
arrays do not expand and produce structured `ShapeMismatch`, matching the
already accepted runtime shape rule.

Equality and inequality accept real or complex operands and use IEEE equality:
NaN is unequal and signed zeros compare equal. Ordered comparisons accept real
storage only. Complex storage, even when its current imaginary components are
all zero, produces structured `InvalidOperands` for `<`, `<=`, `>`, and `>=` as
required by this milestone.

Every operation allocates a typed output buffer, so the result does not share
storage with either input; an ordinary clone of the result continues to share
its COW buffer. Allocation and element loops use the runtime cancellation token
and the common periodic check interval.

## Explicit boundaries

This tranche does not guess unmeasured complex exceptional behavior. Arithmetic
with a non-finite component in a complex operand remains structured
`UnsupportedArrayOperator`. Complex `.^` is supported for finite components and
a nonzero base. A zero complex-storage base with a real exponent uses the real
power rule, while zero raised to an exponent with a nonzero imaginary component
remains unsupported. Real single NaN, infinities, division by zero, and signed
zero use IEEE binary32 behavior. Finite complex division by a signed complex
zero follows the measured R2022b component signs; complex zero divided by zero
returns real NaN. These cases are covered by tests. Complex equality and
inequality continue to accept non-finite components because their IEEE
comparison behavior is defined without complex arithmetic.

## Verification

The focused runtime test records R2022b-derived binary32 bit patterns for all
five arithmetic operators, finite complex arithmetic, mixed-double rounding,
logical mixing, scalar expansion, real-to-complex power promotion, all six
comparisons, NaN/infinity/signed-zero behavior, empty shapes, COW separation,
shape mismatch, cancellation, extreme finite complex scaling, complex zero
division, and each unsupported boundary. The probe source and raw proprietary
output are intentionally not part of the repository.
