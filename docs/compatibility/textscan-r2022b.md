# `textscan` compatibility with MATLAB R2022b

This record covers OpenMat's first `textscan` tranche. Its behavior was checked
with locally authored black-box probes against the installed MATLAB R2022b. No
MATLAB source, tests, documentation, or diagnostic messages were copied.

## Implemented surface

`textscan` accepts either an open file identifier or in-memory char/string text
and returns a `1 x N` cell row. The implemented conversions are:

- `%f`, `%f32`, and `%f64`;
- `%d`, `%d8`, `%d16`, `%d32`, and `%d64`;
- `%u`, `%u8`, `%u16`, `%u32`, and `%u64`;
- `%s` and quoted text `%q`;
- output suppression with `%*`.

Each output column retains the requested element class. Numeric columns are
dense column vectors, while text columns are cell columns of char rows. Empty
input files produce `0 x 1` columns. `CollectOutput` joins adjacent columns only
when their types and row counts agree.

The optional repeat count is supported, including zero and positive infinity.
Scanning is line-aware: missing fields at the end of a record are padded, and
extra fields repeat the format within that record. A finite file scan leaves the
stream after the last consumed field instead of forcing it to EOF.

## Name-value options

The first tranche implements `Delimiter`, `Whitespace`, `HeaderLines`,
`MultipleDelimsAsOne`, `CollectOutput`, `EmptyValue`, `ReturnOnError`,
`TreatAsEmpty`, one line marker for `CommentStyle`, and the standard newline
forms of `EndOfLine`.

With `ReturnOnError=true`, successfully converted values are returned and
columns may have different lengths. With `ReturnOnError=false`, conversion
failure uses `MATLAB:textscan:handleErrorAndShowInfo`; a file stream remains at
the first byte of the invalid token. Floating empty fields use `EmptyValue`
(`NaN` by default), while conversion to integer output maps a `NaN` empty value
to zero.

## Deliberate boundary of this tranche

- File streams must declare UTF-8 encoding. Locale-specific transcoding is not
  inferred.
- Field widths, `%c`, scansets, and other conversion families are not parsed.
- Alphanumeric literal matching inside the format is not implemented.
- `CommentStyle` supports one nonempty line marker, not paired block markers.
- Custom multi-character `EndOfLine` sequences are not implemented.
- A scan reads at most the session's 256 MiB file-transfer boundary.
