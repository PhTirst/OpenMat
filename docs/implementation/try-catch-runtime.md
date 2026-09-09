# Try/catch and MException runtime

OpenMat consumes bytecode exception tables in `openmat-runtime` and exposes
runtime failures as the built-in value class `MException`. Parser, compiler,
verifier, cancellation, and internal VM failures remain outside the catchable
language boundary.

## Unwinding

`run_frame_with_handlers` offers a failed instruction and its structured
`RuntimeError` to `unwind_frame_error`. The shortest containing protected range
is the active handler. A callee without a matching handler drops its frame and
propagates the same error to the caller's suspended call instruction.

Entering a handler clears expression registers and packs while preserving
language locals and completed workspace/object writes. A bound catch receives
an `MException`; a plain catch discards it; a no-catch handler resumes at its
continuation. Catch bodies lie outside their inner protected range, so their
errors can reach an outer handler.

## MException

`MException` is installed when an interpreter session starts. It is a value
class with public-get/private-set properties:

- `identifier`: a char value containing the structured error identifier;
- `message`: OpenMat-owned char diagnostic text;
- `stack`: a failure-first column struct with `file`, `name`, and `line`;
- `cause`: a column cell array of `MException` values.

`MException(identifier, message, ...)` validates an empty or colon-separated
identifier and uses the standard `sprintf` formatter when format arguments are
present. A newly constructed exception has an empty stack and cause list.

`addCause` returns a value-copy with one appended cause. `getReport` supports
`basic` and `extended` reports plus the accepted `hyperlinks` option; report
text is authored by OpenMat. Basic reports omit stack and causes, while
extended reports recursively include both.

## Throwing and stack preservation

`throw` copies its input, records the current source-backed language stack in
the copy, and raises it. The source object remains unchanged. `throwAsCaller`
does the same after removing the current language frame. `rethrow` accepts a
previously caught exception and propagates its original runtime error and stack
without replacing the origin.

The in-flight exception copy is a temporary GC root until a bound handler
receives it or an unbound handler discards it. Exception metadata follows
ordinary value-class copies but does not independently keep unreachable
objects alive.

Every caught or unhandled catchable error also updates the session's
`lasterr`/`lasterror` state using the same `file`, `name`, and `line` stack
projection.

## Catchable boundary

Catchability is an exhaustive conversion on `RuntimeErrorKind`; display text
is never inspected. Language errors include undefined names, call/arity and
operand failures, built-in failures, object/access/class failures, call-depth
errors, and structured array failures. Cancellation, invalid instruction
pointers, and internal execution-state failures are deliberately not
catchable.

Rust types remain internal implementation details. No Rust ABI is introduced
at the plugin or process boundary.
