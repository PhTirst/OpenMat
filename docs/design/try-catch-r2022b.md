# MATLAB R2022b `try`/`catch` frontend and unwind proposal

Status: proposal, not an accepted compatibility, bytecode, runtime, ABI, or
protocol contract.

This note records the first-wave handwritten frontend representation and
normalized black-box observations used to shape it. It also suggests a second-
wave compiler/VM boundary. It does not claim complete exception compatibility
and does not authorize changes to an accepted RFC or specification.

## Scope and measurement boundary

The frontend work started from OpenMat checkout
`pre-publication baseline`. On 2026-08-24, small programs
authored for OpenMat were executed with the installed
`$env:MATLAB_EXE`. Only normalized outcomes were
recorded. No MATLAB source, tests, documentation, or diagnostic prose was
copied.

These observations concern syntax and control flow. They are independent of a
physical, signal-processing, or electromagnetic time convention.

The first wave implements tokens, a lossless/error-tolerant CST, HIR, and
focused tests only. It does not implement bytecode, VM handlers, exception
objects, `throw`, `rethrow`, or full exception semantics.

## Normalized R2022b observations

The probes established the following behavior for the sampled programs:

- Both plain `catch` and `catch identifier` handle an error raised in the
  protected body.
- The bound value was an exception object and remained bound after the
  construct completed. The sampled object exposed identifier, message, stack,
  and cause properties.
- Nested constructs selected the nearest active handler. An error newly raised
  while executing an inner catch body reached an enclosing handler.
- Empty try and catch bodies parsed and completed.
- Headers separated by a line break, a comma, or a semicolon worked. Trailing
  line comments before a line break did not change the observed structure.
- A fully compact form using only spaces between `try`, its body, `catch`, and
  the catch body was rejected. The frontend therefore requires a line break,
  comma, or semicolon at the two header boundaries while retaining all trivia.
- `try ... end` without a catch clause parsed. If its protected body raised an
  error, remaining protected statements did not run, the error did not escape
  to an enclosing catch, and execution returned normally from the construct.
- Duplicate catch clauses, a missing `end`, and unmatched `catch` or `end`
  forms were rejected. Exact MATLAB identifiers and diagnostic text were not
  observed or used.

The no-catch result is a positive compatibility requirement, not parser
recovery. A no-catch `try` must remain distinguishable from both a plain catch
and a bound catch through HIR and the eventual handler representation.

The probes did not establish state rollback rules, the complete catchable-error
set, exact exception class behavior, errors crossing every callable form, the
behavior of `rethrow(caught)`, or mutation of globals and persistent values
before an error. Those require separate OpenMat-authored probes and an accepted
second-wave decision.

## First-wave syntax representation

The lexer already reserves `try`, `catch`, and `end`; all tokens, comments,
line endings, commas, semicolons, and whitespace remain in the lossless token
stream.

The CST adds these nodes:

```text
TryStmt
|- Block                         protected body, always present
|- CatchClause?                  absent for R2022b no-catch form
`- CatchClause*                  recovery-only duplicates

CatchClause
|- NameExpr?                     optional exception binding
`- Block                         catch body, always present
```

Empty blocks have zero-width ranges anchored at the next structural keyword.
A `TryStmt` covers its opening keyword through its matching `end`. A
`CatchClause` covers `catch` through the start of the matching `end` (or through
the next recovered catch boundary). This keeps comments and delimiters
reconstructable while giving HIR stable body ranges.

Parser-owned recovery diagnostics are deliberately OpenMat-specific:

| Code | Condition | Recovery boundary |
| --- | --- | --- |
| `OMP0036` | duplicate `catch` | retain another `CatchClause`, then continue to the next `catch` or `end` |
| `OMP0037` | unmatched `catch` | retain an error node and recover to the line/comma/semicolon boundary |
| `OMP0038` | missing line/comma/semicolon after `try` or a catch header | leave the following token available as recovered body syntax |
| `OMP0200` | missing matching `end` | close the `TryStmt` at EOF using the existing block diagnostic |
| `OMP0002` | unmatched `end` | retain the existing standalone error node |

There is intentionally no missing-catch diagnostic.

## First-wave HIR representation

`StmtKind::Try(TryStatement)` retains:

- the protected `Vec<Stmt>` and its `body_span`;
- an optional catch clause, distinguishing no-catch from all catch forms;
- the catch clause's optional `Name`, body statements, `body_span`, and full
  clause span;
- the enclosing statement span through the ordinary `Stmt` range.

For a duplicate-catch recovery CST, HIR selects the first catch clause. The CST
and parser diagnostic retain the rejected duplicates; downstream executable
lowering must not treat them as additional handlers.

HIR introduces no catch-local lexical scope. Based on the sampled persistence
of the binding, the second-wave proposal should treat the catch name as an
assignment in the surrounding function or script scope unless broader probes
show a conflicting case.

## Proposed bytecode handler contract

The compiler and bytecode crates should first agree on abstract handler
semantics; the concrete opcode or exception-table encoding can follow. A
minimal logical interface is:

```text
enter_handler(handler_pc, exception_destination?, protected_span)
... protected bytecode ...
leave_handler
jump continuation_pc

handler_pc:
    bind or discard the structured exception
    ... catch bytecode, or no instructions for no-catch ...
continuation_pc:
```

Required behavior:

1. The handler is active only while the protected body executes. It is removed
   before a catch body begins, so an error from a catch body reaches an outer
   handler.
2. Normal completion removes the handler and skips the catch body.
3. Plain catch discards the exception value and executes its body. Bound catch
   stores the value in the catch name's ordinary scope slot before executing
   its body.
4. No-catch still installs a handler. On error it discards the exception and
   resumes at the continuation without executing the remaining protected
   instructions.
5. An error raised in a deeper bytecode frame unwinds to the nearest active
   handler. The unwind record therefore needs at least the owning frame, target
   PC, register/temporary restoration boundary, optional exception
   destination, and protected source span.
6. Popped frames and abandoned temporaries must not remain reachable. Whether
   completed language-visible writes before the error remain visible must be
   decided from additional probes; the VM should not invent transactional
   rollback.
7. Parser and compiler diagnostics are never caught as runtime exceptions.
   The accepted contract must enumerate which runtime error categories are
   catchable.

An exception table indexed by protected PC ranges can implement the same
semantics as explicit enter/leave opcodes. Whichever encoding is chosen must be
verified for well-nested ranges, valid handler targets, valid destination
registers, and consistent frame depth. Rust types must not become a plugin or
process boundary; any external representation remains C ABI or a versioned
serialized protocol.

## Proposed minimum exception value

The runtime should unwind a structured OpenMat-owned value rather than a Rust
error string. The minimum useful internal data is:

- a stable OpenMat error category used by conformance comparison;
- an optional compatibility identifier;
- an OpenMat-authored display message (exact MATLAB prose is non-normative);
- an ordered stack of source-backed frames;
- an ordered cause list or chain;
- source information sufficient to populate the first stack frame when the
  error originates in bytecode.

The eventual language value exposed by a bound catch should provide at least
identifier, message, stack, and cause views because all four were observable in
the sampled R2022b value. Stack frames should use OpenMat source/function
identity and ranges; they must not leak Rust paths or implementation frames.
Whether the public value must report the exact MATLAB class is not decided by
this proposal.

## Second-wave coordination points

Before compiler/runtime work lands, the coordinator should confirm:

1. the concrete bytecode handler encoding and verifier invariants;
2. no-catch lowering as a swallowing handler and its continuation target;
3. the catchable runtime-error categories and conversion into the minimum
   exception value;
4. frame/register cleanup across ordinary calls, function handles, nested
   functions, and other callable values;
5. the mutation rules for arrays, outputs, globals, and persistent storage when
   an error interrupts evaluation;
6. catch-name declaration/assignment behavior in compiler scope analysis, LSP
   declarations, definitions, indexing, and semantic classification;
7. `throw`/`rethrow` behavior, cause preservation, and stack preservation;
8. any versioned protocol representation needed to observe exceptions without
   copying proprietary diagnostic prose.

The new HIR variant also requires downstream traversal arms. In particular,
compiler assigned-name collection must visit both bodies and add the catch
binding; executable lowering must not fall through to its unknown-statement
diagnostic. Kernel bare-script discovery and LSP scope/classification/indexing
walks must traverse both bodies and treat the catch name as a declaration.
Those adaptations are outside this first-wave task.

## Non-goals

This proposal does not claim complete MATLAB exception semantics, prescribe
diagnostic prose, add VM opcodes, define a public ABI, or modify the accepted
R2022b compatibility contract. It does not use a successful parse or frontend
lowering test as evidence that runtime unwinding is implemented.
