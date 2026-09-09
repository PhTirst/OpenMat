# Sparse linear algebra compatibility boundary (MATLAB R2022b)

This note records OpenMat-authored black-box probes executed by the locally
installed MATLAB R2022b 9.13. It does not copy MATLAB source, tests,
documentation, identifiers, messages, or diagnostic prose. Warning categories
below are OpenMat-owned names derived from the observable input condition.

## Matrix multiplication

For non-scalar operands, both `sparse * full` and `full * sparse` return full
double storage. The measured real and complex cases preserve ordinary matrix
dimensions, column-major values, multiple output columns, and compatible empty
inner dimensions. In particular, `2x0 sparse * 0x3 full` returns a full `2x3`
zero matrix, while `3x0 full * 0x2 sparse` returns a full `3x2` zero matrix.

The checked-in `sparse_matrix_multiply_full` case covers these observations.
The runtime applies sparse-by-full products one full column at a time through
the provider SpMV boundary. Full-by-sparse uses `(S' * F')'`, including both
complex conjugations, so the same provider path owns validation and execution.

## Full-row-rank underdetermined left division

Sparse underdetermined `A\B` is observably different from the existing dense
underdetermined operator. With

```matlab
A = sparse([1, 0, 1; 0, 1, 1]);
B = [1, 2; 2, 3];
```

R2022b returns the full `3x2` basic solution
`[1,2; 2,3; 0,0]` and no warning was observed. The dense operator selects a
different basic column set for the same numerical matrix. A shifted pattern
`sparse([0,1,0; 0,0,1])` selects columns two and three, confirming that the
sparse result is not a hard-coded trailing-zero rule. A complex multi-RHS probe
also selected the first numerically independent columns in left-to-right CSC
order and left every unselected result row exactly zero.

`SparseProvider::underdetermined_basic_f64` and its complex counterpart encode
that distinct contract. The provider-neutral selector uses reorthogonalized
independence tests without densifying the complete wide matrix, then the chosen
square CSC system is factored and solved by the selected provider. Deficient
row rank remains a structured `RankDeficient` status. The checked-in
`sparse_underdetermined_basic` case covers real, shifted-pattern, complex, and
multiple-RHS results plus the absence of a warning.

## Warning-bearing square and tall policies

The same isolated probe measured these two successful warning-bearing
boundaries:

- For `A = sparse([1,2; 2,4])` and
  `B = [1,2; 2,5]`, square `A\B` returned a full `2x2` result whose first column
  was `[NaN; NaN]` and second column was `[Inf; -Inf]`. A warning was observed.
  OpenMat names the normalized condition `matrix-singular`.
- For `A = sparse([1,2; 2,4; 3,6])` and
  `B = [1,0; 2,1; 3,0]`, tall `A\B` returned the full basic least-squares result
  `[1,1/7; 0,0]` (the measured first component was
  `1.0000000000000002` for the first RHS and `0.14285714285714288` for the
  second). Its Frobenius residual was `0.84515425472851646`, and a warning was
  observed. OpenMat names the normalized condition `matrix-rank-deficient`.

The provider already reports `Singular` and `RankDeficient` structurally. It
must continue to do so: warning state, visible warning text, and the successful
non-finite/basic result policy are runtime concerns.

## Runtime warning and fallback ownership

The provider continues to return typed `Singular` and `RankDeficient` states.
`linalg_ops.rs` consumes only those categories and constructs the user-visible
fallback value:

- square singular fallback continues partial-pivot elimination past exact zero
  pivots, then permits IEEE triangular division to produce the observed
  `NaN`/signed-`Inf` payload while preserving structurally solvable pivots; and
- tall rank-deficient fallback selects the first numerically independent CSC
  columns, solves the reduced least-squares problem by reorthogonalized QR, and
  leaves unselected result rows exactly zero.

Sparse binary dispatch returns a typed success outcome containing the `Value`
and an optional `SparseSolveWarning`. The central interpreter records the
OpenMat-owned identifier in session `WarningState`, so `lastwarn` is updated even
when display is disabled, and emits through the ordinary ordered
`OutputEvent::CommandText` path only when enabled. It neither parses provider
text nor writes to stderr. The current identifiers are
`OpenMat:Sparse:SingularMatrix` and
`OpenMat:Sparse:RankDeficientMatrix`.

Additional locally authored probes confirmed that an entirely zero square
matrix still yields `NaN` for a zero RHS and signed infinities for a nonzero RHS,
whereas an all-zero tall matrix returns an all-zero basic least-squares result.
Complex singular fallback was observed as complex `NaN` values. The checked-in
`sparse_warning_solve_fallbacks` differential case asserts both real numerical
payloads and that `lastwarn` becomes nonempty; runtime tests separately assert
the OpenMat-owned identifiers and emitted warning events.
