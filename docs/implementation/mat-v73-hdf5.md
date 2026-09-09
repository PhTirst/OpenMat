# MAT v7.3/HDF5 implementation boundary

Status: second interoperable tranche, measured bidirectionally against the
installed MATLAB R2022b (9.13.0.1967605).

## Backend boundary and dispatch

`openmat-mat` keeps the existing Level-5 v6/v7 codec and adds a narrow
`Mat73Backend<P: Hdf5Provider>` boundary. The provider exchanges owned MAT-file
images and OpenMat values; HDF5 handles and the Rust ABI do not cross a plugin,
process, or crate API boundary. Automatic decoding selects v7.3 only when both
MATLAB's `MATLAB 7.3 MAT-file` user-block header and the HDF5 signature at byte
512 are present. Ordinary HDF5 is not treated as a MATLAB file.

`save(..., '-v7.3')` selects this backend. `load` auto-detects it. `-append`
preserves an existing v7.3 file unless an explicit `-v6`, `-v7`, `-mat`, or
`-v7.3` option requests another output format. Level-5 remains the default.

## MATLAB schema implemented

The writer emits a 512-byte HDF5 user block and MATLAB schema metadata rather
than merely producing generic HDF5:

- HDF5 dataset dimensions are the reverse of MATLAB dimensions; OpenMat values
  remain column-major and retain arbitrary N-D shapes.
- real numeric arrays use their exact primitive HDF5 type; complex numeric and
  integer arrays use `real`/`imag` compound members.
- logical and UTF-16 char arrays use `MATLAB_class` plus the measured
  `MATLAB_int_decode` attributes.
- shaped empties store MATLAB dimensions as a `uint64` payload with
  `MATLAB_empty = 1`.
- cells store object-reference datasets. Nonempty structs are groups whose
  field datasets contain object references; `MATLAB_fields` uses the measured
  variable-length array of one-byte fixed ASCII strings. Referenced values live
  below `/#refs#/`.
- datasets at least 1024 bytes use built-in deflate level 3.

The backend covers dense double and single, all fixed-width signed and unsigned
integers, logical, real and complex storage, exact UTF-16 char code units, N-D
shape, shaped empties, cells, structs, and MATLAB-compatible sparse CSC values.

## Sparse CSC persistence

Sparse values are HDF5 groups rather than dense datasets. `MATLAB_sparse` is
the `uint64` row count, `jc` is the zero-based `uint64` column-offset vector,
`ir` is the zero-based `uint64` row-index vector, and `data` uses double,
`real`/`imag` compound double, or `uint8` logical storage. Sparse logical adds
`MATLAB_int_decode = 1`. A zero-NNZ value retains both dimensions through its
row-count attribute and `columns + 1` offsets; matching MATLAB R2022b, the
writer omits `ir` and `data` in that case.

Decode validates the complete CSC boundary before constructing a runtime
value: `jc` must have `columns + 1` entries, start at zero, be monotone, and end
at the stored count; `ir` and `data` lengths must agree; every row must be in
bounds and strictly increasing within a column; stored explicit zero values are
rejected. Counts and index conversion use checked arithmetic. Sparse values can
appear at the root or beneath cell and struct object references. The MAT v7.3
schema does not persist `nzmax` reserve beyond stored entries, so a read value's
reserve normalizes to `nnz` (or the runtime minimum of one for empty sparse).

## Provider-neutral partial dense I/O

`Mat73Hyperslab` describes zero-based contiguous `start` and `count` vectors in
MATLAB dimension order. `Hdf5Provider::read_dense_hyperslab` and
`write_dense_hyperslab` keep HDF5 handles behind the provider boundary; the
default methods return a structured `Unsupported` error for providers that have
not implemented this optional capability. `Mat73Backend` and the top-level
`read_v73_dense_hyperslab` / `write_v73_dense_hyperslab` functions expose the
same owned-image API.

The official provider reverses selection dimensions only at the HDF5 boundary,
so returned and replacement buffers remain column-major. It supports the dense
primitive classes already handled by the whole-file codec, including complex,
logical, char, and fixed-width integer storage. Bounds, dimensionality, result
allocation, replacement shape, and exact storage class are checked. A partial
write updates the selected dataset in a writable in-memory HDF5 image and
returns a new owned MAT image; it does not expose a file or dataset handle.

Language-level `matfile` registration is not part of this lane because the
existing `load`/`save` ownership is `openmat-builtins/src/file_io.rs`, which was
not an authorized path. Integration needs a coordinator-approved edit there to
parse language subscripts into `Mat73Hyperslab` and pass filesystem bytes to
the public core API.

## Object-backed families

MATLAB R2022b stores string arrays as MCOS/object-decoded records linked to
`#subsystem#`, not as a standalone UTF-16 dataset. Table, datetime,
categorical, and classdef values use the same object-decoding family. The
second-stage reader skips the subsystem as metadata and deterministically
returns `MatErrorKind::Unsupported` when a user variable or nested reference
has `MATLAB_object_decode`; it does not reinterpret the opaque record as char,
cell, or struct. The writer likewise returns `Unsupported` for runtime string,
table, and object values. Ordinary char arrays continue to preserve every
UTF-16 code unit, including NUL and unpaired surrogates.

## HDF5 security and resource boundary

The complete link tree, including `/#refs#/` and struct groups, is traversed
with an object bound before decoding. Only single hard links are accepted;
soft/external links, hard-link aliases/cycles, external dataset storage, virtual
datasets, and filters other than built-in deflate are rejected. Dynamic HDF5
filter/plugin loading is disabled process-wide with `H5PLset_loading_state(0)`.

File-image bytes, decoded allocations, uncompressed HDF5 dataset bytes, array
elements, object count, followed references, and recursion depth have separate
limits. Active `/#refs#/` paths detect reference cycles. Allocation growth uses
checked arithmetic and fallible reservation, and encode/decode observe the
runtime cancellation flag. Errors retain an OpenMat-owned category:
`InvalidFormat`, `Unsupported`, `LimitExceeded`, `Cancelled`, `Hdf5`, or
`InvalidValue`.

## Reproducible native dependency

The Windows target builds the official HDF5 C sources and zlib statically; it
does not download a prebuilt HDF5 binary. `Cargo.lock` pins crates.io source
archives and SHA-256 checksums:

| package | version | crates.io archive SHA-256 |
| --- | --- | --- |
| `hdf5-metno` | 0.14.1 | `f72d6ab4f6d6d79bd350c23f6fe17c99d86d8dd05c0e86f6291a366d47e1fece` |
| `hdf5-metno-sys` | 0.12.3 | `8139abe2218e47a40bdc7822a4365b620e23dce17e6b4735bbe6629410cdfedb` |
| `hdf5-metno-src` | 0.10.4 | `69c883c565498492954e344c482f8622525e58c7a71100dd9498c4940da82885` |
| `libz-sys` | 1.1.29 | `85bc9657773828b90eeb625adff10eeac83cc21bbfd8e23a03eaa8a33c9e28d9` |

This resolves to official HDF5 2.2.0 and bundled zlib 1.3.2. The Rust bindings
are MIT OR Apache-2.0, official HDF5 uses its 3-clause BSD license, and zlib uses
the zlib license. The dependency features disable shared HDF5 and enable static
HDF5 plus static zlib. `tools/mat73/Verify-Hdf5Static.ps1` checks the locked
checksums, resolved feature graph, and PE import table; a passing release binary
has no HDF5 or zlib DLL import.

## MATLAB R2022b black-box gate

The MATLAB-authored fixture included dense 2-D and 3-D values, complex single,
signed/unsigned integers, logical, UTF-16 including a surrogate pair and NUL,
0-by-3 and 2-by-0-by-4 empties, a cell array, and a struct array. OpenMat decoded
that file and re-encoded it. MATLAB R2022b then loaded the OpenMat-authored file
and asserted class, size, values, code units, cells, and struct fields. A second
probe measured empty cell/struct and zero-field struct metadata. These are
schema-compatibility checks, not generic `h5read` checks.

The second locally-authored probe adds real, complex, logical, 0-by-N, N-by-0,
reserved-empty, cell-nested, and struct-nested sparse values. OpenMat reads the
MATLAB-authored sparse file, writes its own sparse file, and MATLAB asserts
class, `issparse`, dimensions, stored values, logical class, complexity, and
nested placement. Separate object-backed fixtures cover string, table,
datetime, categorical, and a locally-authored classdef solely to verify the
stable unsupported category. Hyperslab tests read and update a nontrivial 2-D
block and verify column-major positions plus bounds and type failures.

## Deferred surface

MCOS string/table/datetime/categorical/classdef persistence, arbitrary MATLAB
objects, partial sparse updates, dataset growth, strided hyperslabs, external
links, and non-deflate/custom filters remain unsupported. The core hyperslab
API is not yet registered as a language `matfile` builtin. A whole-file
`-append` still rewrites supported values; callers must explicitly use the new
dense hyperslab API for partial in-image updates.
