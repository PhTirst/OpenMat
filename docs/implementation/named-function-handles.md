# Named function handles

Milestone 6 supports named MATLAB function-handle expressions of the form
`@name`. Anonymous functions are not part of this tranche.

## Representation and bytecode

The implementation reuses `openmat-value`'s existing
`Value::Function(FunctionHandle)` representation. Bytecode version 8 adds
`LoadFunctionHandle { dst, name }`, where `name` must refer to a string in the
current function's constant pool. The verifier checks both the destination
register and string-name operand and retains the instruction's source location
on failure.

The compiler resolves a handle to a function declared in the same source unit
directly to `Constant::Function`. Other named handles lower to
`LoadFunctionHandle`. This resolution deliberately does not inspect function
locals or entry-workspace assignments: a same-named value binding cannot turn
`@name` into that value and cannot make a non-function target valid.

## File and built-in resolution

The kernel linker processes `LoadFunctionHandle` alongside ordinary global
loads while preserving their distinct semantics. Registered built-in names
remain runtime-resolved; other names search the calling file's directory first
and then configured source paths. A resolved function file is compiled, merged,
recursively linked, and the load is rewritten to a `Constant::Function`. Script
and class files do not satisfy this tranche's function-handle target contract.

If no source function is linked, the runtime resolves the name only through its
registered built-in registry. It does not read the workspace and it does not
invoke the target while constructing the handle. A remaining name produces
`RuntimeErrorKind::UnknownFunctionHandleTarget`, mapped by the kernel to
`runtime.unknownFunctionHandleTarget` and diagnostic code `OMR0018`, with the
original handle-expression source range and stack context.

## Invocation and lifetime

`handle(args...)` retains HIR's unresolved parenthesized application and lowers
to the existing `Apply` instruction. `apply_value` recognizes
`Value::Function` and delegates to `call_value`, so bytecode functions and
registered built-ins keep the call site's requested output count, argument
count, cooperative cancellation, and ordinary missing-output behavior.

Function values are ordinary copyable values. They can be passed as inputs,
returned as outputs, and stored in the persistent workspace. The interpreter
registers bytecode handles with the source module that owns their function, so
a workspace handle remains callable after the kernel installs a later request
module. Session clearing drops this retained code together with the workspace.

## Deliberate boundary

This tranche does not implement anonymous closures or capture environments. It
also does not promise class-constructor handles or script handles. Those forms
require separate compatibility and lifetime decisions rather than special-case
target names.
