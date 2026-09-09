# MAT-file load/save implementation boundary

Status: implemented on the bytecode-v20 runtime path and measured against the
installed MATLAB R2022b.

## Implemented

- save(filename) and save(filename, names...)
- command forms such as save data a b and load data a
- default compressed Level-5/v7 output and explicit -v6, -v7, or -v7.3
- automatic Level-5/v7.3 input detection
- -append for an existing supported MAT-file, preserving v7.3 unless a format
  option is explicit
- automatic .mat suffix when the filename has no extension
- exact-name, *, and ? variable selection
- S = load(filename, names...) as an ordered scalar structure
- no-output load as one staged current-scope update
- script, ordinary function, global, persistent, and dynamically introduced
  function-workspace bindings
- clear and clearvars removal of dynamically loaded function variables
- little- and big-endian Level-5 reads, small data elements, MATLAB's compact
  numeric payloads, and bounded zlib decompression
- dense double, single, complex, logical, all fixed-width integer classes,
  exact UTF-16 char arrays, cells, and structures
- MATLAB-schema-compatible v7.3/HDF5 N-D values and shaped empties

The filesystem still enforces the session's 256 MiB transfer limit. The MAT
decoder separately bounds Level-5 decompression and v7.3 file images,
allocations, uncompressed datasets, objects, references, elements, and nesting.
The v7.3 security, dependency, and MATLAB-schema details are recorded in
`mat-v73-hdf5.md`.

## Explicitly unsupported

- the matfile partial-I/O/hyperslab write API
- sparse arrays
- MATLAB string, table, categorical, datetime, object, function-handle, and
  graphics-handle persistence
- -ascii, -struct, -regexp, and -nocompression
- creating a missing destination with -append
- warning-event parity for a load filter that matches no variable

These boundaries fail with OpenMat-owned structured diagnostics. They are not
silently converted to a private format or a different MATLAB class.

## Verification

Normal Rust tests cover v6/v7/v7.3 round trips, nesting, decompression limits,
HDF5 resource limits, cancellation, reference cycles,
script and function scope, command form, append, dynamic clear, and rollback
after a valid decoded prefix followed by malformed bytes.

An ignored codec test reads fixtures from OPENMAT_MATLAB_FIXTURE_DIR. The
release gate used R2022b-authored v6/v7 files containing double, complex
single, logical, char, cell, and struct values, then loaded OpenMat-authored
v6/v7 files back into R2022b. A separate exact-code-unit check covered BMP and
surrogate UTF-16 char data. The v7.3 gate additionally used MATLAB-authored
HDF5 schema fixtures and loaded OpenMat-authored v7.3 output back into R2022b;
it did not substitute generic HDF5 readability for MATLAB compatibility.
