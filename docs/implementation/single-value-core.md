# Single value core

This tranche adds a real MATLAB `single` value model. It does not encode
binary32 values in binary64 storage and does not make single values eligible
for double-only arithmetic dispatch.

## Representation

`openmat-array` defines `DType::{F32, ComplexF32}`, `Complex32`, and
`ArrayData::{F32, ComplexF32}`. Real elements are stored as `f32` and
complex elements as two independent `f32` components. Both variants contain a
`DenseArray`, so they retain the common canonical `Shape`, flat column-major
order, and `Arc<Vec<_>>` copy-on-write behavior. A language scalar is a `1x1`
single array; there is no widened scalar optimization.

`Value::Array` owns this storage alongside double, logical, char, and integer
arrays. For its `F32` and `ComplexF32` variants the class remains `single`; dtype,
shape, element count, and complex marker come directly from the nested storage.
`as_real_single` and `as_complex_single` expose scalar components without a
binary64 intermediate. Existing `as_real_number` and `as_complex_number`
remain double/logical accessors and deliberately return `None` for single, so a
generic double-only operation cannot silently accept single.

`ArrayElement` supports both `f32` and `Complex32`: `ArrayData::from_typed`,
`as_typed`, and `as_typed_mut` use the same checked access and COW behavior as
other dense types. `Value::as_array` now includes single storage; `as_single`
filters these two dtypes. `ValueKind::Single` remains a coarse classification
for dispatch and errors, independent of the top-level enum variants.

`Table` remains a built-in `Value::Table(TableArray)` container. Scalar, object,
and graphics representations are unchanged. OEX continues to expose single as
dense `F32` / `ComplexF32` data through its existing C ABI and C++/Rust bindings.

This Rust enum layout is not an ABI. Runtime, kernel, and protocol consumers
must dispatch the explicit dtype and use their versioned external contracts.

## Conversions and core built-ins

`single(x)` accepts implemented logical, double, complex double, char, and
fixed-width integer values while preserving the source shape and real/complex
storage marker. IEEE conversions produce binary32 values directly. NaN,
positive and negative infinity, and the sign bit of zero are retained according
to Rust's IEEE conversion semantics. Existing single input is an identity path:
the returned language value initially shares its COW buffer.

`double(single_value)` widens each binary32 component exactly to binary64.
`logical`, `real`, `imag`, and `conj` have explicit single branches and preserve
shape. `complex(single_real, single_imaginary)` requires two real single inputs,
supports equal shapes or scalar expansion, and always returns complex binary32
storage, including when every imaginary component is zero or signed zero. The
one-input form converts real single to complex single and preserves an existing
complex single value by COW clone. Mixed single/double `complex` inputs are
rejected instead of promoted silently.

`reshape` materializes the same binary32 component type in the requested shape;
an unchanged shape uses the general identity clone path. Arithmetic, indexing,
transpose, runtime assignment, and linear algebra use explicit single dispatch
rather than double views. Their existing precision,
promotion, and unsupported-operation rules are preserved by the storage merge.

## Verification boundaries

Unit coverage includes positive and negative finite values, signed zero, NaN,
infinities, `1x1`, `0xN`, and two-dimensional shapes, column-major indexing,
real and complex COW detachment, integer conversion, complex construction,
scalar expansion, metadata queries, and reshape. Protocol observation and
conformance manifests are read-only downstream acceptance work for this task.

The storage merge also exercises exact typed access, rejection of mismatched
element types, real/complex COW detachment through the common array API, and
preservation of unsupported single entry points. Validation commands:

```powershell
cargo fmt --all -- --check
cargo test -p openmat-array -p openmat-value -p openmat-sparse -p openmat-runtime -p openmat-builtins -p openmat-mat -p openmat-oex -p openmat-kernel --locked
cargo check --workspace --exclude openmat-cli --all-targets --locked
```

These checks pass. Three MAT interoperability tests remain ignored because they
require `OPENMAT_MATLAB_FIXTURE_DIR` from an installed MATLAB probe. The full
`cargo check --workspace --all-targets --locked` is blocked by pre-existing
unhandled protocol V3 variants in `openmat-cli/src/lib.rs`; the CLI and protocol
definitions are outside this storage change.
