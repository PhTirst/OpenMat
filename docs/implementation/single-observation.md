# Exact single observation

The kernel observes single arrays through `Value::Array` with `ArrayData::F32`
or `ArrayData::ComplexF32` storage. Summaries, display formatting, byte
counts, previews, and workspace change detection do not materialize a double
array. Shapes and flat values retain the stored column-major order, and change
detection compares binary32 component bits so an unchanged NaN is stable while
signed-zero changes remain observable.

Kernel v0 and v1 already define numeric preview components as JSON numbers.
Every finite binary32 component is exactly representable in that existing
binary64 Rust field, so the kernel widens one component only at the wire-model
boundary and keeps `class: "single"`. Real NaN and infinities use the existing
special spellings. As required by the accepted v1 contract, a complex value
with any non-finite component returns `workspace.unsupportedValue`; v0 retains
its legacy `missing` degradation.

Kernel v2 exact numeric values already admit class `single`. Exact leaf payloads
are generated directly from each `f32` component as normalized number strings,
including `NaN`, `+Inf`, `-Inf`, and signed zero. The accepted v2 model cannot
represent complex storage when every imaginary component is numerically zero,
including an empty complex array, because `complex: true` requires at least one
nonzero imaginary string. The kernel reports `workspace.unsupportedValue` at
that exact-value boundary instead of silently relabeling the storage as real.

The conformance CLI accepts a `single` number from v0/v1 only when widening the
value back from binary32 reproduces the wire component bit-for-bit. It then
formats that exact widening with the oracle's `%.9g` rule, so `single(0.1)` is
observed as `"0.100000001"` and `single(1/3)` as `"0.333333343"`. Formatting
uses nine significant digits, switches to scientific notation for exponents
below -4 or at least 9, removes insignificant trailing fractional zeroes,
normalizes scientific exponents to a sign and at least two digits, and preserves
`-0`. Double formatting is unchanged. Schema-v2 exact strings pass through
without numeric conversion.

The kernel maps every structured `ArrayRuntimeError::InvalidIndex` reason to
`runtime.indexOutOfBounds` with diagnostic `OMR0016`, alongside structured
out-of-bounds and assignment-cardinality failures. The classification inspects
the nested error variant rather than message text; an ordinary internal invalid
execution state remains `runtime.invalidState` with `OMR0013`.

Unknown named function-handle targets remain the kernel error
`runtime.unknownFunctionHandleTarget` with diagnostic `OMR0018`. At the
conformance observation boundary, that category now maps to the R2022b oracle's
project-owned `undefined-name` category; no MATLAB message text is introduced.
