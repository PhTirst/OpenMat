# `switch` frontend closure

This milestone closes the handwritten frontend path for
`switch`/`case`/`otherwise`. It deliberately stops before bytecode emission or
VM execution. Accepted specifications and RFCs remain authoritative.

## Lossless CST

The parser produces a `SwitchStmt` whose ordered children are:

1. the selector expression, when present;
2. zero or more `CaseClause` nodes in source order; and
3. an optional `OtherwiseClause`.

Each `CaseClause` retains its expression and `Block`. An `OtherwiseClause`
retains its `Block`, including an empty one. The containing `SwitchStmt` token
range includes the matching `end` and all trivia remains in the shared token
stream, so `Cst::text()` reconstructs the source exactly.

A switch body stops only at `case`, `otherwise`, or the `end` belonging to that
switch parser invocation. Nested `if`, `for`, `while`, and `switch` statements
consume their own clauses and `end` before control returns to the enclosing
block. Consequently, an inner switch clause cannot terminate an outer switch
case body.

## Error recovery

Malformed switches still return a root CST and preserve all tokens. Stable
parser diagnostics cover the required structural failures:

| Code | Condition |
| --- | --- |
| `OMP0031` | no switch selector begins on the logical header line |
| `OMP0032` | no case expression begins on the logical header line |
| `OMP0033` | repeated `otherwise` |
| `OMP0034` | `case` after `otherwise` |
| `OMP0035` | an unlabeled statement or token directly in the switch body |
| `OMP0200` | missing matching `end` |

Repeated or out-of-order clauses remain in the CST after the ordering
diagnostic. Recovery therefore does not discard the later source and does not
panic. A selector or case expression that begins but is malformed retains the
ordinary stable expression diagnostic, such as `OMP0103` for a missing closing
parenthesis.

## HIR boundary

HIR represents the construct as one selector expression, an ordered
`Vec<SwitchCase>`, and an `Option<OtherwiseBranch>`. Every switch statement,
case clause, otherwise clause, selector, case expression, and contained
statement retains a UTF-8 byte range. The frontend does not select a comparison
operator, expand cell cases, or otherwise decide matching and execution
semantics.

Malformed CST nodes lower to ordinary error expressions plus structured HIR
diagnostics, following the existing error-tolerant lowering contract.

## Compiler boundary and later bytecode work

A structurally valid `StmtKind::Switch` currently produces exactly one
source-located `OMC0007` diagnostic with
`UnsupportedFeature::SwitchStatement`. Compiler lowering does not visit the
selector, cases, or otherwise body after reporting that boundary, so the
construct cannot degrade into malformed-HIR or internal-compiler diagnostics.

The execution tranche should evaluate the selector exactly once and evaluate
case expressions in source order. It should first lock the MATLAB R2022b
matching rules, then either add a versioned `SwitchMatch` bytecode operation or
an equivalently explicit runtime helper before using conditional jumps. A
dedicated matching boundary is preferable to prematurely encoding switch as
ordinary binary equality, because container and type-sensitive matching belong
to execution semantics rather than this frontend milestone.

`try`/`catch` remains outside this implementation.
