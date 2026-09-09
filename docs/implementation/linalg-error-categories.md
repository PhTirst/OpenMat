# Structured linear algebra error categories

The runtime-to-kernel boundary classifies linear algebra failures from nested
enums only. Diagnostic display text is non-normative and is never parsed to
choose a kernel or conformance category.

| Runtime structure | Kernel category | Diagnostic code | Conformance category |
| --- | --- | --- | --- |
| `RuntimeErrorKind::Cancelled` | `runtime.cancelled` | `OMR0001` | `other` |
| `RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch { .. })` | `runtime.dimensionMismatch` | `OMR0019` | `dimension-mismatch` |
| Structured indexing failures already recognized by the kernel | `runtime.indexOutOfBounds` | `OMR0016` | `index-out-of-bounds` |
| Other `InvalidExecutionState` failures | `runtime.invalidState` | `OMR0013` | `other` |

The `runtime.dimensionMismatch` row is deliberately narrow. It does not include
`SingularMatrix`, `RankDeficient`, `SquareMatrixRequired`, provider failures,
other linear algebra variants, or `ArrayRuntimeError::ShapeMismatch`. Those
failures retain the existing `runtime.invalidState` kernel category and reach
the conformance schema as `other`.

Cancellation observed by a linear algebra operation is converted by the
runtime to the top-level `RuntimeErrorKind::Cancelled` variant before the
kernel boundary, so it continues to use `runtime.cancelled` rather than either
invalid-state category. Existing structured indexing classification is also
unchanged.
