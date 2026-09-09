# MATLAB R2022b compatibility contract

## Normative target

The target is the MATLAB R2022b base language, excluding Simulink and proprietary
toolboxes. OpenMat specifications and OpenMat-owned conformance tests are the
normative project artifacts; the local MATLAB installation is a black-box oracle
used to discover and verify observable behavior.

## Required semantic foundations

- Dynamic typing with `double` as the default numeric class.
- Dense real and complex arrays, logical arrays, and common integer classes.
- Column-major storage behavior and one-based language indexing.
- MATLAB-compatible shape, `size`, `ndims`, `numel`, empty-array, and trailing
  singleton-dimension behavior for implemented types.
- Linear, multidimensional, colon, range, and logical indexing.
- Copy-on-write value semantics for arrays and value-class objects.
- Conjugate transpose (`'`) and non-conjugate transpose (`.'`).
- Character arrays from single quotes and strings from double quotes.
- Scripts, functions, nested scopes as introduced by milestones, multiple input
  and output arguments, and `nargin`/`nargout` behavior.
- `if`, `switch`, `for`, `while`, `try`/`catch`, `break`, `continue`, and
  `return`, staged according to milestone acceptance tests.

## Comparison policy

- Exact comparison for classes, shapes, integers, logical values, strings, and
  deterministic structural results.
- Tolerance or residual comparison for floating-point and linear algebra.
- Match whether an error occurs and its OpenMat error category; exact MATLAB
  identifier or diagnostic prose is not required unless a test explicitly says
  otherwise.
- Display formatting and execution speed are non-normative in release one.

