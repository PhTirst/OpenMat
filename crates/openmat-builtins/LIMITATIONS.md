# Release-one core built-in limitations

The registry rejects these forms with a structured `BuiltinError`; it does not
approximate them with a different operation.

- `zeros`, `ones`, and `eye` do not yet accept class-name or `like` options.
  `eye` accepts only two-dimensional sizes, matching the base identity-matrix
  form.
- A shape-changing `reshape` materializes the existing column-major buffer into
  a new exact `DenseArray`/`StringArray`. This includes char, all real/complex
  integer storages, and UTF-16 string elements. Reshaping to the same canonical
  shape keeps shared COW storage. A future array-core shape-retagging API can
  remove the extra copy without changing language-visible results.
- The two-input `complex` form supports equal shapes and scalar expansion. It
  deliberately rejects complex component inputs and other shape combinations.
- `sum` and `prod` implement the default dimension and a positive numeric
  dimension. Text options such as `all`, native/output-type selection, and
  missing-value flags are not implemented.
- `min` and `max` implement reduction forms `min(A)`, `max(A)`, and their
  `(A, [], dim)` variants. Element-wise two-input comparison, missing-value
  flags, and the second index output are not implemented.
- `dot` implements the default reduction dimension. An explicit dimension is
  not implemented.
- `norm` implements the default Euclidean norm for real or complex vectors and
  empty arrays through `ReferenceProvider`. Matrix spectral norms and explicit
  norm selectors are not implemented; logical inputs remain unsupported as in
  the R2022b oracle.
- The eight integer constructors implement exact real/complex fixed-width
  storage, component-wise half-away-from-zero rounding, saturation, and
  NaN/infinity handling. Integer arithmetic, reductions, and `real`/`imag`/`conj`
  overloads remain outside this tranche and return structured errors instead of
  promoting through double.
- `char` accepts real numeric, logical, char, and real integer inputs, preserving
  every uint16 code unit and saturating to `0..65535`. Complex-to-char forms are
  intentionally rejected until their complete R2022b conversion table is
  accepted.
- `strcmp` compares char arrays and UTF-16 string elements exactly. It supports
  string scalar expansion and char-row/string comparison; character-matrix row
  rules and cell arrays are outside this boundary. Missing string elements
  compare false, including against another missing element.
- `fileread` accepts UTF-8 text and preserves its decoded UTF-16 code units,
  including a UTF-8 BOM and original line endings. Locale-specific legacy text
  encodings are not inferred.
- `fopen`, `fclose`, `fread`, `fwrite`, `fseek`, and `ftell` use session-owned
  file identifiers and binary, column-major numeric transfers. The current
  tranche supports the fixed-width real integer and floating precisions,
  native/little/big endian conversion, and scalar or two-dimensional read
  sizes. Nonzero `skip`, repetition precision forms such as `4*uint16`, text
  transcoding, and transfers larger than 256 MiB remain explicitly rejected.
- `textscan` accepts UTF-8 file identifiers or in-memory char/string text. Its
  first tranche implements `%f`, typed floating/integer conversions, `%s`,
  `%q`, suppression, finite repeat counts, line-aware padding, and the common
  delimiter/header/empty/error/collection options. Field widths, `%c`, scansets,
  alphanumeric format literals, block comments, custom end-of-line sequences,
  and non-UTF-8 stream transcoding are rejected rather than approximated.
- `readmatrix` supports UTF-8 delimited text, automatic comma/tab/semicolon/bar
  detection, quoted fields, ragged rows, complex values, `Delimiter`,
  `NumHeaderLines`, and double/single `OutputType`. Spreadsheet files, ranges,
  import options, date/time conversion, and locale-specific numeric forms are
  rejected explicitly.
- `writematrix` supports real/complex double and single, logical, and every
  fixed-width integer matrix. It implements text overwrite with `Delimiter`
  and MATLAB-compatible general numeric formatting. Spreadsheet output,
  append mode, ranges, and import/export option objects are not implemented.
