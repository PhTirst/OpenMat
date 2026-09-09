# Sparse CSC first tranche (MATLAB R2022b)

This tranche defines sparse language values as OpenMat-owned normalized
compressed-sparse-column (CSC) storage. It does not expose `faer`, SuperLU, or
another provider container as a language value.

## Representation contract

- Shapes are exactly two-dimensional and retain `0x0`, `0xN`, and `Nx0`.
- Rows, columns, and internal indices use checked `u64` arithmetic.
- `col_offsets` and `row_indices` are zero based. `col_offsets` is monotone;
  the row indices in each column are strictly increasing.
- COO coordinates cross the language boundary as one based, are validated,
  converted to zero based, sorted by `(column,row)`, and combined at duplicate
  coordinates.
- Duplicate double and complex-double values are summed in original COO input
  order. Duplicate logical values use the equivalent boolean result. A combined numeric `+0` or `-0`
  and logical `false` are removed; `NaN`, `+Inf`, and `-Inf` are retained.
- Clones share the complete CSC allocation. No unchecked mutation API or
  third-party sparse type crosses the dynamic-value boundary.

The public Rust surface is `CscMatrix<T>`, `CooEntry<T>`, and
`SparseArrayData::{Logical,F64,ComplexF64}`. Checked constructors are
`try_empty`, `try_from_coo`, and `try_from_canonical_parts`. Observation and
basic display adapters use `stored_entry` or `element_at_offset`; dense
materialization is explicit through `try_to_dense`.

`payload_bytes` accounts for the language CSC payload as one 64-bit word per
column offset and row index plus the reserved typed value payload. Rust
`Arc`/`Vec` headers and allocator over-allocation are deliberately excluded.
`allocated_buffer_bytes` reports actual buffer capacities when host-side
diagnostics need them. Every variable-size allocation is preceded by checked
conversion and `try_reserve_exact`; long loops poll the supplied cancellation
flag.

## R2022b probes

The behavior was checked with locally authored programs executed by the
installed MATLAB R2022b, not inferred from Octave. The focused oracle case is
`builtin_sparse_core_semantics`.

Observed behavior used by the implementation:

- `sparse(logical(...))` is sparse logical; `full` and `nonzeros` preserve the
  logical class, while `spones` returns sparse double.
- Numeric duplicate COO coordinates sum before zero cleanup. `Inf + -Inf`
  becomes a stored `NaN`; duplicates which cancel to zero are not stored.
- A complex COO result whose surviving imaginary components are all zero is
  canonical real double. A genuinely complex transpose remains sparse;
  apostrophe conjugates and dot-apostrophe does not.
- Scalar indexing of a sparse matrix returns sparse `1x1` storage. An implicit
  zero selected from a complex sparse input is canonical real sparse zero.
- `find` walks CSC column-major order, returns full indices/values, preserves
  row-vector and `0x0` orientation, accepts `Inf` as an unlimited count, and
  honors `first`/`last` limits.
- `sparse([],[],[],0,N)` and `sparse([],[],[],N,0)` preserve their rectangular
  empty shapes. `speye` does the same. `spalloc` may reserve more entries than
  `numel`; R2022b reports a minimum `nzmax` of one.

## Implemented language surface

The standalone sparse built-in module implements `sparse`, `full`, `issparse`,
`nnz`, `nonzeros`, `spones`, `speye`, and `spalloc`. It exports a sparse-only
`find` delegation hook because the shared registry already owns the `find`
name. Runtime array operations implement sparse transpose, conjugate
transpose, and one- or two-subscript scalar reads.

The coordinator must add the module/registry call and delegate sparse `find`
from the existing implementation. Other consumers of the new `Value::Sparse`
variant must add explicit display, preview, persistence-rejection, and memory
summary branches at their existing shared boundaries.

## Deliberate boundary

This tranche does not implement sparse solve (`A\b`), general sparse
arithmetic, complex indexed assignment, `eigs`, `svds`, MAT v7.3 persistence,
or a provider bridge to `faer`/SuperLU. General non-scalar sparse indexing and
sparse assignment remain outside this first closure.
