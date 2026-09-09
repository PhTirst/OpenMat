# MATLAB R2022b matrix division clean-room boundary

This document records black-box behavior measured with locally installed,
licensed MATLAB R2022b. It is a differential target for OpenMat, not a runtime
implementation or an inference from Octave. All programs and expected values
are OpenMat-authored. No MATLAB source, tests, documentation, or diagnostic
prose is copied into this repository.

The original measurements apply to the baseline
`pre-publication baseline`. Temporary exploratory probes lived
under the operating-system temporary directory. Checked-in observations were
captured through the existing bounded oracle and observation-v2 schema.

## 1. Observable operator contract

For `A` with shape `m x n` and `B` with shape `m x p`, `A\B` has shape
`n x p` when the operands are compatible:

- `m == n`: solve `A X = B`;
- `m > n`: return a least-squares solution minimizing each residual
  `||A x - b||_2`;
- `m < n`: R2022b matrix left division returns a **basic solution**, not the
  minimum-norm solution promised by the current provider-only `gels` contract.

The last distinction is observable even for a full-row-rank matrix. For

```matlab
A = [1, 0, 1; 0, 1, 1];
b = [1; 2];
```

R2022b returns `[-1; 0; 2]`. The minimum-norm solution would be `[0; 1; 1]`.
Both satisfy `A*x == b`, so residual-only tests cannot distinguish the
semantics.

For `L` with shape `p x n` and `R` with shape `m x n`, `L/R` has shape
`p x m`. The language-level relation to implement and test for complex data is

```text
L / R = (R' \ L')'
```

where each apostrophe is the conjugate transpose. The checked-in rectangular
complex case stores both the quotient and its difference from this relation.
The measured difference is zero to the case tolerance.

Scalar and `1x1` operands are observably the scalar division boundary: `8\2`,
`[8]\[2]`, `2/8`, and `[2]/[8]` all produce real `1x1 double` value `0.25`.

## 2. Measured R2022b boundary

All payloads use column-major linear order. `Warning` below is an OpenMat-owned
normalized category, never MATLAB diagnostic text. A dash means that no warning
was observed in an isolated fresh-process probe.

| Operation | Input boundary | Output | Warning | Corpus |
| --- | --- | --- | --- | --- |
| `A\B` | full-rank real square, two RHS | `2x2 double`, real | - | `matrix_left_square_real_multi_rhs` |
| `A\B` | full-column-rank real `4x2`, two RHS | `2x2 double`, real least-squares result | - | `matrix_left_overdetermined_real_multi_rhs` |
| `A\B` | full-row-rank real `2x3`, two RHS | `3x2 double`, real basic solutions | - | `matrix_left_underdetermined_basic` |
| `A\B` | full-rank complex square, two RHS | `2x2 double`, complex | - | `matrix_left_complex_multi_rhs` |
| `A\B` | full-rank complex `single` square | `2x1 single`, complex | - | `matrix_left_single_complex` |
| scalar and `1x1` `\` and `/` | four equivalent expressions | `1x4 double`, real | - | `matrix_scalar_and_one_by_one` |
| `L/R` | complex `1x2 / 3x2` | `1x3 double`, complex | - | `matrix_right_complex_relation` |
| `A\B` | rank-deficient real `3x2` | `2x1 double` basic solution `[0; 0.5]` | `matrix-rank-deficient` | not representable faithfully |
| `A\B` | rank-deficient real `2x3` | `3x1 double` basic solution `[0; 0; 1/3]` | `matrix-rank-deficient` | not representable faithfully |
| `A\B` | singular real `2x2` | `2x1 double` containing two `NaN` values | `matrix-singular` | not representable faithfully |
| `A\B` | nearly singular real `2x2` | finite `2x1 double` result `[2; 0]` | `matrix-nearly-singular` | not representable faithfully |
| `A\B` | coefficient/RHS row mismatch | error `dimension-mismatch` | - | `matrix_left_shape_mismatch` |
| `L/R` | incompatible column counts | error `dimension-mismatch` | - | `matrix_right_shape_mismatch` |

The three warning categories above are project names derived only from the
observable condition. They do not preserve vendor identifiers or localized
messages.

### 2.1 Empty shapes

R2022b treats compatible zero dimensions as successful solves with exact output
shapes and no observed warning:

| Expression | Result |
| --- | --- |
| `eye(2)\zeros(2,0)` | real `2x0 double` |
| `zeros(3,0)\zeros(3,2)` | real `0x2 double` |
| `zeros(0,3)\zeros(0,2)` | real `3x2 double` zeros |
| `[]\[]` | real `0x0 double` |
| `zeros(0,2)/eye(2)` | real `0x2 double` |
| `zeros(2,0)/zeros(3,0)` | real `2x3 double` zeros |

`matrix_empty_shapes` stores these six recursively normalized values in a cell
so each child retains its own class, size, `numel`, and `complex` flag.

## 3. Numerical comparison policy

Class, shape, `ndims`, `numel`, and `complex` are exact requirements. Reference
components remain the oracle's full-precision decimal strings. Differential
comparison uses:

- real/complex `double` solve results: absolute `1e-12`, relative `1e-12`;
- real/complex `single` solve results: absolute `5e-6`, relative `5e-6`;
- scalar `0.25`, zeros, empty payloads, and error categories: exact comparison.

The tolerances cover harmless provider-dependent rounding for these small,
well-conditioned systems. They are not rank thresholds. A candidate cannot use
a loose numeric tolerance to substitute a minimum-norm answer for the measured
underdetermined basic solution because the vectors differ by order one.

## 4. Provider coverage and language integration

The provider contract in `docs/implementation/rectangular-solve.md` began as a
full-rank binary64 QR/LQ boundary. The language runtime now wires it to `\` and
`/`, adds a deterministic column-pivoted underdetermined basic-solution helper,
and bridges same-class `single` inputs through binary64. Its coverage remains
narrower than the complete R2022b operator:

| Boundary | Current implementation | R2022b compatibility consequence |
| --- | --- | --- |
| full-rank square `double`/complex `double` | language dispatch to provider `gesv` unique solve | covers the checked-in square numerical core |
| full-column-rank overdetermined `double`/complex `double` | language dispatch to provider `gels` least squares, including multiple RHS | covers the checked-in overdetermined numerical core |
| full-row-rank underdetermined `double`/complex `double` | deterministic column-pivoted basic-solution helper plus provider square solve | covers the checked-in R2022b basic-solution case |
| zero required rank and compatible empty shapes | runtime short-circuits with exact result shape before provider dispatch | covers all six measured empty-shape results |
| rank-deficient rectangular | non-rank-revealing `gels`; provider may return structured rank deficiency only for a zero triangular diagonal | lacks R2022b rank selection, returned basic solution, and warning |
| singular/nearly singular square | `gesv` reports exact singularity; no condition-aware warning report | lacks R2022b successful value plus singular/nearly-singular warning behavior |
| `single` | explicit binary32-to-binary64 provider bridge with rounded binary32 result | covers the checked-in complex-single case, but is not native single LAPACK |
| right division | runtime applies the conjugate-transposed left-division relation | covers the checked-in complex rectangular relation |

In particular, wiring underdetermined `mldivide` directly to `gels` would pass a
residual check but fail `matrix_left_underdetermined_basic`. Rank-deficient
compatibility needs a rank-revealing QR/SVD policy and a MATLAB-compatible basic
solution choice, not just a different error mapping.

## 5. Schema boundary

Observation v2 precisely represents the checked-in successful values and the
two `dimension-mismatch` errors. It has no warning channel. Consequently a
rank-deficient, singular, or nearly singular observation cannot simultaneously
assert its successful payload and required warning. Those probes are design
evidence only and are intentionally absent from the corpus.

A future shared schema change must add an ordered warning collection to a
successful observation and expectation. The minimum semantic shape required by
this boundary is:

```json
{
  "warnings": [
    {"category": "matrix-rank-deficient"}
  ]
}
```

The initial category set needed here is `matrix-rank-deficient`,
`matrix-singular`, and `matrix-nearly-singular`. Only the category participates
in conformance comparison; prose, locale, provider names, estimated rank, and
condition-number formatting do not. This task does not modify the schema,
oracle, runner, or comparator.

## 6. Integration status and remaining interface requirements

Items 1, 2, 3 (for full-rank inputs), 5, and 6 below are implemented. Items 4
and 7 remain the shared boundary for warning-bearing successful results:

1. Preserve matrix left and right division as distinct language operations.
   Swapping `A\B` into generic scalar `B/A` is valid only for the scalar/`1x1`
   fast path and cannot represent matrix solve dispatch.
2. Dispatch `mldivide(A,B)` by two-dimensional shape and numeric class. Preserve
   multiple RHS, column-major layout, output class (`double` or `single`), output
   complexness, and the exact `n x p` result shape.
3. Use unique solve for full-rank square systems and least squares for
   full-column-rank overdetermined systems. Add a rank-revealing basic-solution
   path for underdetermined and rank-deficient systems; do not expose the
   existing minimum-norm `gels` result as R2022b `\`.
4. Return a successful value together with zero or more normalized warning
   categories. Provider `SingularMatrix` or `RankDeficient` errors alone cannot
   model the measured R2022b boundary.
5. Implement `mrdivide(L,R)` through `(R'\L')'` for matrix operands, including
   complex conjugation, then preserve the resulting `p x m` shape and class.
6. Handle the six measured empty-shape combinations before native calls where
   necessary. Reject incompatible rows for `\` and incompatible columns for
   `/` as `dimension-mismatch`.
7. Extend observation schema/oracle/runner/comparator atomically before adding
   warning-bearing conformance cases. Until then, do not claim the warning
   boundary is tested by the checked-in runner.

At the original measured baseline, the compiler/runtime only implemented
generic scalar division for these operators and rejected matrix division before
reaching the provider bridge. The conformance cases therefore define
development targets; their MATLAB oracle success is not evidence of OpenMat
runtime support.

An exact-ID OpenMat runner snapshot over the ten new cases passed only
`matrix_scalar_and_one_by_one`. The other nine produced OpenMat `type-error`:
seven differed from the successful MATLAB outcome, while the two shape cases
differed from MATLAB's `dimension-mismatch` category. The snapshot therefore
has `1 passed / 9 failed / 0 unsupported / 0 internal`; the failures are
intentional evidence of the missing language dispatch, not accepted test
successes.

After bytecode v16 and the runtime numerical integration, the same exact ten
matrix-division case IDs pass `10 / 10` against their checked-in MATLAB R2022b
observations. A broader `matrix_*` run passes `11 / 11` because it also includes
the pre-existing `matrix_shape` case. This closes the warning-free corpus only;
the warning-bearing boundaries in sections 2 and 5 remain deferred.

## 7. Corpus and reproduction

The first tranche contains ten cases:

- six direct numeric success cases for square, overdetermined,
  underdetermined-basic, complex, complex `single`, and right-division relation;
- one scalar/`1x1` equivalence case;
- one recursive empty-shape case;
- two normalized dimension-mismatch cases.

Capture only this tranche from the repository root with:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath '<MATLAB R2022b matlab.exe>' `
    -Tag matrix-division `
    -ResultDirectory tests/conformance/reference/matlab-r2022b
```

The oracle validates each generated observation against its manifest and the
selected v2 schemas. `run-summary.json` remains transient.
