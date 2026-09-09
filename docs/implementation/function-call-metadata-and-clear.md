# Function call metadata and command-form `clear`

This tranche advances the register-bytecode contract from version 4.0 to 5.0.
The version change preserves the v4 class tables and access metadata and adds
three explicit instructions:

- `LoadCallInputCount` reads the actual argument count stored in the current
  execution frame.
- `LoadCallOutputCount` reads the output count requested by the current call
  site.
- `ClearGlobal` removes an ordered, non-empty list of workspace names encoded
  as string constants in the containing function.

The interpreter creates call metadata for every bytecode invocation. Ordinary
functions, instance methods, and constructors receive the caller's requested
output count. Property-default evaluators request one output. The synthetic
module entry requests zero outputs. Counts are checked at the bytecode `u32`
boundary when a frame is created and are represented as MATLAB scalar doubles
when loaded.

The compiler recognizes bare `nargin` and `nargout` only inside a function and
only when the function's deterministic local analysis has not bound the same
name as a parameter, output, or local. Parenthesized `nargin(...)` and
`nargout(...)` remain unresolved call-versus-index applications. Entry
workspace names and the existing bare `true`/`false` rules are unchanged.

The parser recognizes command-form `clear name1 name2` as a lossless
`ClearStmt`; the HIR preserves the names in source order and records whether
the command used the supported identifier-list form. The compiler emits
`ClearGlobal` only for a top-level entry statement. A linked support script's
entry instructions are spliced at the original call site, so its clear effects
remain ordered with the surrounding script. The linker remaps every encoded
name constant when it copies those instructions.

Release one deliberately rejects `clear` without names, option forms such as
`clear all`, wildcard or pattern forms, and function-form arguments. A valid
identifier-list `clear` inside a function is also rejected until frame-local
clear semantics are implemented; it never falls back to clearing the shared
workspace. These forms produce structured parser or compiler diagnostics.
