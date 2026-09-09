# Aggregate C3 growth, rollback, and error classification

Baseline: integration commit `90341d4`.

## Implemented boundary

For a name-rooted `AssignPlace` whose path continues after a parenthesized cell
or struct selection, a scalar out-of-bounds selector may grow an offline working
root before the remaining path is evaluated. Existing records retain their
column-major positions. Every cell gap and every field of every struct gap is
filled with a real `0x0` double. The completed path is written back once; a
failed tail discards the grown working root.

Deletion does not use this growth path. It continues to validate the original
shape and supports only the accepted whole-dimension or linear-vector forms.

The runtime still resolves all selector values before `AssignPlace`, reads the
RHS before mutation, builds the result away from the source root, and writes the
destination register only after success. The persistent workspace store emitted
by the compiler therefore remains unreachable on a failed assignment.

## Rollback evidence

`openmat-runtime` host-side tests execute bytecode directly and inspect the
persistent workspace after `execute_entry` returns `Err`. They cover:

- cell paren cardinality mismatch (`2` selected, `3` supplied);
- struct paren field-schema mismatch;
- multi-element brace assignment without an expanded RHS pack.

For each covered root, both the root binding and a pre-existing copy-on-write
alias retain their exact value and continue sharing storage with the original.
This is the rollback gate for Aggregate C3 before language-level exception
handling exists.

## Error classification

The runtime preserves structured `ArrayRuntimeError::AssignmentSizeMismatch`
detail. The kernel classifies both that detail and a direct
`IndexOutOfBounds` detail as `runtime.indexOutOfBounds` with diagnostic code
`OMR0016`. The CLI then normalizes the stable kernel category to the
schema-v2 conformance category `index-out-of-bounds` without inspecting error
text.

## Remaining boundary

This work does not implement `try`/`catch`. Consequently
`aggregate_c3_transaction_rollback` remains deferred as an end-to-end
differential program. Its two single-root rollback conditions are covered by
the host-side runtime tests above; this does not claim statement-wide atomicity
for multiple independent LHS places.
