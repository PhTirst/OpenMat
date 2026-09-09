# Low-level file I/O compatibility with MATLAB R2022b

This record covers the first OpenMat low-level file-I/O tranche. Its behavior
was checked with locally authored black-box probes against the installed MATLAB
R2022b. No MATLAB source, tests, documentation, or diagnostic messages were
copied.

## Session model

- File identifiers are session-local doubles at the language boundary.
- Allocation starts at 3 and reuses the lowest closed identifier.
- `fopen('all')` returns the open identifiers as a double row vector.
- `fclose('all')` and interpreter session clearing close every owned stream.
- Cloned `LocalFileSystem` values share the session working directory but do
  not share operating-system file handles.

An open failure such as a missing read-only file produces `fid == -1` and a
nonempty message. Operations on an invalid numeric identifier use the observed
`MATLAB:badfid_mx` identifier. Querying an invalid identifier with `fopen(fid)`
instead produces empty metadata outputs.

## Implemented surface

The minimal registry now includes:

- `fopen(filename[, permission[, machinefmt[, encoding]]])`, `fopen(fid)`, and
  `fopen('all')`;
- `fclose(fid)` and `fclose('all')`;
- `ftell(fid)` and `fseek(fid, offset, origin)` with `bof`, `cof`, `eof`, or
  their numeric equivalents;
- `fread(fid[, size[, precision[, skip[, machinefmt]]]])`;
- `fwrite(fid, A[, precision[, skip[, machinefmt]]])`.

Binary conversion supports signed and unsigned 8/16/32/64-bit integers,
single, and double. `fread` accepts the ordinary source precision, `*type`, and
`source=>destination` forms. Native, `ieee-le`, and `ieee-be` byte orders are
implemented. Dense numeric transfers retain column-major order and return the
observed partial-final-column zero padding for a two-element size.

The mode query uses the observed Windows canonical spellings (`rb`, `wb`,
`ab`, `rb+`, `wb+`, `ab+`, and explicit text variants). New append streams
start with byte position zero, while each append write still targets EOF.

## Deliberate boundary of this tranche

- A nonzero `skip` is rejected; no silent approximation is made.
- Repetition precision syntax such as `4*uint16` is not parsed yet.
- The `encoding` value is retained as stream metadata for future text
  consumers. `fread` and `fwrite` remain byte-oriented and do not transcode.
- A single read or write is capped at the runtime's 256 MiB transfer boundary.
- `textscan` is covered by its own compatibility tranche. Formatted text output
  and `eval` remain separate later tranches.
