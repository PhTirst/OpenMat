# Char and integer runtime implementation notes

This note records the Milestone 6 runtime/builtins boundary implemented against
value-interface base `pre-publication baseline`.

## Exact runtime paths

- `Constant::Char([])` materializes a `0x0` `ArrayData::Char`; nonempty payloads
  materialize a `1xN` UTF-16 code-unit row without UTF-8 conversion.
- Char and integer concatenation, indexing, indexed assignment, transpose, and
  iteration use typed dense buffers. Integer indices are range-checked in their
  signed/unsigned domain, including `uint64`, before conversion to a host offset.
- Same-class integer assignment preserves component width and signedness. A
  real target promotes to the corresponding complex storage only when assigned
  a complex value of the same integer class. Other mixed forms return a
  structured error.
- String concatenation, indexing, assignment, and `strcmp` use
  `StringElement` code units plus the independent missing bit. Lossy UTF-16
  conversion is not used for value truth.

## Constructors

The eight integer constructors share one checked conversion core. Floating
components use half-away-from-zero rounding, then target-range saturation;
NaN maps to zero and infinities saturate. Integer-to-integer conversion stays
in exact signed/unsigned widened domains. Real and imaginary components are
converted independently, and a result uses real storage when every converted
imaginary component is zero.

`char` has a separate conversion boundary. Exact `uint16` input preserves every
code unit, including isolated surrogates. Other real integer inputs saturate to
`0..65535` without passing through floating point.

## R2022b clean-room probes

On 2026-08-24, a locally authored temporary script was executed with the
installed MATLAB R2022b. It printed only class and numeric component results;
no MATLAB source, tests, documentation, or diagnostic text was retained. The
temporary script was removed after the probe.

The probe closed two cases not specified by the accepted design:

- nonconjugating transpose of complex `int8`/`uint8` preserves the exact
  components and class, while conjugating transpose is rejected;
- `char(double)` truncates finite fractional values toward zero, maps NaN to
  zero, and saturates infinities/out-of-range values to `0..65535`.

The runtime therefore rejects complex-integer conjugating transpose with a
structured exact-type error and applies truncation only in the char conversion
path. Integer constructors continue to use half-away-from-zero rounding.
