# Switch bytecode and compiler lowering

This Milestone 6 tranche advances the bytecode contract from version 10.0 to
11.0 and compiles switch HIR. It does not implement VM matching semantics or
linker consumption.

## Bytecode contract

`SwitchMatch { dst, selector, case_value }` is a dedicated register
instruction. The runtime must write one scalar logical result to `dst` after
matching `selector` against `case_value` under MATLAB R2022b switch rules.

The instruction is intentionally not represented as `BinaryOperator::Equal`.
MATLAB switch matching has distinct scalar, array, character, string, and
integer behavior that must stay behind its own runtime dispatch boundary. The
bytecode verifier checks all three registers and retains the instruction's
source location in any verification error.

## Compiler control flow

The compiler evaluates the selector exactly once and retains its register for
the complete switch. Each case is then lowered in source order as:

```text
evaluate case expression
matched = SwitchMatch(selector, case value)
JumpIfFalse matched, next_case
case body
Jump end
next_case:
```

Only the expressions preceding and including the first match execute. A matched
body cannot fall through to a later case. After the last failed case, control
enters `otherwise` when present and otherwise reaches the switch end directly.

Empty case bodies still emit and patch their end jump. Empty switches only
evaluate their selector. Nested switch end patches resolve to the enclosing
instruction stream, so a nested switch at the end of an outer case remains
valid. A switch does not create a loop-control context: `break` and `continue`
inside a switch continue to target the nearest enclosing `for` or `while`, and
`return` keeps its function-level behavior.

Assignment discovery traverses every case and `otherwise` body. This makes
names assigned only inside a function or class-method switch available as
locals before instruction lowering. Case expressions continue through the
ordinary expression lowering path, preserving unresolved call-versus-index
application and source locations. Anonymous-function reservations and class
auxiliary function IDs therefore use the same allocation paths as expressions
outside switch.

Cell case lists remain outside this tranche because cell literals are not yet
executable. A cell-valued case expression produces the structured
`UnsupportedFeature::CellLiteral` diagnostic at the cell expression range; it
is not expanded into several comparisons and is not treated as equality.

## Required consumer integration

`openmat-runtime` must add a `SwitchMatch` interpreter arm. It must read the
selector and case-value registers, implement MATLAB R2022b matching for scalar
and array values including char, string, and every integer class, write a scalar
logical to `dst`, and preserve the instruction source location on errors. That
matching helper must not delegate to ordinary binary equality merely because a
subset of numeric scalar cases appears equivalent.

`openmat-kernel`'s `module_linker` must treat `dst`, `selector`, and `case_value`
as register operands wherever instructions are remapped or scanned. In
particular, inlined-script register-base remapping must offset all three, and
linker tests must cover all three independently so none can retain an
un-remapped register.

Until both consumers are integrated, workspace-wide builds are expected to
fail on their exhaustive `InstructionKind` matches. Validation for this tranche
is intentionally limited to `openmat-bytecode` and `openmat-compiler`.
