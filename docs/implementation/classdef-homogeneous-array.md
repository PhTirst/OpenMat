# Homogeneous `classdef` arrays

This note records the release-one execution representation for basic homogeneous
object arrays. It implements the contract in `spec/language/classdef-core.md`;
it does not extend that contract to heterogeneous arrays or custom indexing.

## Value representation

`openmat-value::ObjectArray` stores three pieces of session-local state:

- the exact dynamic `ClassHandle`, retained independently of the elements;
- a canonical `openmat-array::Shape` using checked `u64` dimensions;
- a `DenseArray<ObjectHandle>` in contiguous column-major order.

The dense handle buffer uses the array core's copy-on-write storage. The Rust
layout is internal and is not a process, plugin, or persistence ABI. A scalar
constructed object continues to use `Value::Object`; indexed assignment creates
`Value::ObjectArray`, including for its intermediate `1x1` result.

`Value::kind`, `dimensions`, and `numel` expose an object array as kind `object`
with its stored shape. The runtime resolves the dynamic class through the
session class registry rather than returning the coarse Value-layer name
`object`.

## Assignment and indexing

Language indices remain one-based. Resolved offsets are zero-based internal
column-major offsets and pass through the same checked selection machinery as
numeric arrays.

Assigning a scalar object to a missing indexed target creates a `1x1` object
array. Consecutive linear assignments grow a row array from `1xN` to
`1x(N+1)`; a column vector retains column orientation. Growth that would skip
elements is rejected because release one cannot synthesize intervening objects
without class-specific default-construction rules.

All inserted elements must have the exact same dynamic class, not merely a
subclass relationship. An in-bounds selection accepts either one scalar object
or an object array whose element count equals the selection count. Non-scalar
index results retain their exact class and materialize a new contiguous handle
buffer; one-element results use the scalar object representation.

Matrix literals concatenate homogeneous scalar/object-array blocks with checked
two-dimensional extents. Mixing object and non-object values, or mixing exact
dynamic classes, returns a structured runtime/object error.

## Value and handle semantics

The container does not decide element assignment semantics. Before insertion or
workspace assignment, the runtime consults each referenced object's effective
class semantics:

- value-class elements receive fresh object records with cloned property slots;
- handle-class elements retain their original object identity;
- repeated handle identities are valid, while repeated value identities are
  rejected by `ObjectArrayDescriptor` validation.

Copying property-slot arrays continues to use their existing copy-on-write
representations. If cancellation, identity exhaustion, validation, or final
array construction fails after value records were copied, the runtime removes
the fresh assignment copies before returning the error. Failed class
construction continues to use the existing incomplete-object rollback path.

## Kernel observation

Workspace summaries and inspection report the exact dynamic class, stored
dimensions, and checked byte estimate (`numel * 8` for session handles).
Inspection returns one opaque `Missing` preview entry per selected element; the
first release deliberately does not expand property payloads. Workspace deltas
treat object arrays as changed conservatively so identity/property mutations are
not hidden by handle-buffer equality.

## Current limits

- Heterogeneous object arrays are rejected.
- Growth with gaps is rejected; implicit default construction of gap elements
  is not implemented.
- Object-array concatenation is limited to two dimensions.
- Custom `subsref`, `subsasgn`, `subsindex`, and `end` behavior remains outside
  the release-one contract.
- Kernel inspection is metadata-correct but does not expand per-element
  properties.
