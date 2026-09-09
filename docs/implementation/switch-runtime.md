# Switch runtime baseline

This note records the non-cell `SwitchMatch` consumer implemented for bytecode
version 11. Cell-valued case lists remain a compiler diagnostic and are not part
of this runtime baseline.

## VM contract

`SwitchMatch { dst, selector, case_value }` reads the two source registers once,
compares their already-evaluated values, and writes one logical scalar to
`dst`. The helper is separate from ordinary binary equality. It only borrows
array storage, checks cancellation during long text scans, and does not detach
copy-on-write buffers.

The module linker offsets all three registers when a script entry is inlined.
Merging support-module entry, named-function, and anonymous-function bodies
retains all three fields unchanged because those functions keep independent
register files.

## R2022b observations and implemented rules

Minimal clean-room probes against the installed MATLAB R2022b established the
following behavior:

- Numeric and logical selectors must be scalar. A non-scalar or empty numeric
  selector errors, while a non-scalar numeric case value does not match a
  scalar selector.
- Numeric class alone does not prevent a match: equal double, single, logical,
  signed integer, and unsigned integer scalars can match. Real values also
  match complex values with an exact zero imaginary component.
- Floating NaN never matches, including itself. Signed zero matches zero.
- Integer comparisons are exact across signedness, widths, and floating-point
  values. In particular, `uint64` maximum and `int64` maximum do not match their
  upward-rounded binary64 conversions, while `int64` minimum matches its exact
  binary64 representation. Exact integers are never converted to `f64`.
- Character arrays and nonempty string arrays require matching shape and exact
  contents. UTF-16 code units are compared directly, including isolated
  surrogates. A character row vector and scalar string compare by exact UTF-16
  payload; empty char and an empty string scalar match.
- A missing string element prevents a match, including missing against missing.
  A `0 x 0` string array does not match. Numeric/string mixtures error.
- Unsupported internal markers, callable handles, and unresolved object values
  return a structured runtime operand error rather than panicking.

The current `openmat-value`/`openmat-array` interface has no `single` storage
variant. The probe confirms the intended cross-class scalar behavior, but this
runtime cannot preserve or test a distinct single payload until that shared
value interface is extended by its owning tranche.

This baseline does not complete the Milestone 6 switch tranche: executable
cell-valued case lists are still absent by design.
