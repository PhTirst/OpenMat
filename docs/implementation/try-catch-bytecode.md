# Try/catch bytecode contract

This tranche advances the logical bytecode format from version `13.0` to
`14.0`. It defines and verifies function-level exception tables. It does not
implement compiler lowering, VM unwinding, an exception language object, a
module serializer, a linker, or a disassembler.

The contract is deliberately a versioned data model, not a Rust ABI. A process,
plugin, cache, or protocol boundary must encode every field explicitly.

## Public API

Each `Function` owns `exception_handlers: Vec<ExceptionHandler>`. Table order is
preserved but has no selection priority. `Function::new` creates an empty table,
and `Function::with_exception_handlers` installs a complete table.

An `ExceptionHandler` contains:

- `protected_start` and `protected_end`, defining a non-empty half-open
  instruction interval;
- `kind`, either `Catch { handler, error_local }` or `Swallow`;
- `exit`, the first instruction after the complete construct;
- an optional `SourceLocation` for the complete source `try` statement.

`ExceptionHandler::catch`, `ExceptionHandler::swallow`, and `with_location` are
the intended construction API. `error_local: None` represents plain `catch`;
`Some(local)` represents `catch name`. `Swallow` is a distinct format variant,
not an empty catch, because R2022b `try ... end` without `catch` is a positive
language form. An empty protected source body emits no exception record because
there is no protected instruction that can raise an error.

The runtime-provided caught value is opaque to this crate. The VM must write an
OpenMat-owned structured error value or handle to `error_local` before it
executes the first catch instruction. The slot is an ordinary surrounding-scope
local, not a catch-only lexical local. No Rust error type or object layout is
part of the bytecode contract.

## Compiler emission layout

For `try body; catch [name]; catch_body; end`, lowering must emit:

```text
protected_start:
    body
protected_end:
    Jump exit
handler:                         # exactly protected_end + 1
    catch_body                   # error_local was assigned by the VM
exit:                            # handler == exit for an empty catch body
    next instruction
```

The exception record is
`Catch { protected_start, protected_end, handler, error_local, exit }`. The
normal-path jump is outside the protected range, so successful execution always
skips the catch. The catch body is also outside its own protected range, so an
error raised there can reach an enclosing protected range.

For no-catch `try body; end`, lowering emits:

```text
protected_start:
    body
protected_end == exit:
    next instruction
```

The record kind is `Swallow`. Successful execution falls through to `exit`; an
error abandons the remaining protected instructions and resumes at the same
`exit`. No catch body or catch-local assignment exists.

The compiler must ensure `exit` names a real instruction, inserting an ordinary
continuation/return instruction when the construct ends a function. Empty
protected bodies omit the record; for a catch form, their unreachable catch
body may be omitted as well.

Nested source constructs emit nested protected ranges. If an inner construct is
inside an outer protected body, the inner skip jump, catch body, and continuation
remain inside the outer protected interval. Thus an error from the inner catch
is not handled again by the inner record but is still eligible for the outer
record.

## Runtime selection and unwinding

When an instruction raises a catchable runtime error, the VM selects the unique
innermost protected interval containing that instruction PC. Equal intervals
and crossing overlaps are invalid bytecode, so table order cannot change the
answer.

For an error originating in the current frame, the tested PC is the failing
instruction. For an error propagated by a called bytecode function or built-in,
the VM first searches the callee in the same way. If it finds no handler, it
pops that frame and tests the caller's suspended call/apply instruction PC.
Consequently a call error is caught precisely when that caller instruction is
inside a protected interval. This rule applies to every instruction that can
invoke user or built-in code, not only `InstructionKind::Call`.

On `Catch`, the VM keeps the owning frame, removes all popped callee frames and
abandoned temporaries, assigns the caught value when requested, and sets the PC
to `handler`. On `Swallow`, it discards the caught value and sets the PC to
`exit`. Installing the catch-local value must be infallible after verification.
This contract does not add transactional rollback: language-visible writes
completed before an error remain subject to the later runtime compatibility
decision.

Parser/compiler diagnostics are not runtime errors and never enter this table.
The bytecode does not enumerate catchable runtime categories; the runtime/error
value tranche must define that closed conversion boundary without leaking Rust
diagnostics.

## Structural verification

`verify` checks every exception record before instruction operands:

- `protected_start < protected_end`, and both endpoints name instructions;
- `handler` and `exit` name instructions;
- a catch handler is exactly `protected_end + 1`, does not follow `exit`, and
  `protected_end` contains `Jump { target: exit }`;
- a no-catch record has `protected_end == exit`;
- `error_local`, when present, is inside the owning function's local array;
- equal protected ranges are rejected as ambiguous;
- overlapping protected ranges must be strictly nested; crossing ranges are
  rejected;
- a nested record's catch/continuation region must finish no later than the
  enclosing protected end, keeping errors from the nested catch eligible for
  the enclosing handler.

Metadata errors carry the owning function and the record's optional source
location, with no misleading instruction provenance. Ordinary verification of
the skip `Jump` and all body instructions still runs after table validation.

## Serialization and compatibility

There is no module serializer/deserializer in the repository at this tranche.
When one is added, the v14 function record must encode the exception table after
the instruction stream as a `u32` count followed in vector order by:

```text
protected_start: u32
protected_end:   u32
kind:            u8     # 0 = Catch, 1 = Swallow
if Catch:
    handler:     u32
    local_tag:   u8     # 0 = absent, 1 = present
    if present:
        error_local: u32
exit:            u32
location_tag:    u8     # 0 = absent, 1 = present
if present:
    source_id:   u32
    start:       u32
    end:         u32
```

Integers use the containing bytecode artifact's eventual fixed endianness; the
Rust struct layout and enum discriminants must never be copied. A decoder must
reject unknown tags, truncated/extra record fields, and resource-limit
violations, then run `verify` before exposing executable bytecode. A round trip
must preserve record order and optional source locations exactly.

The current verifier continues the repository's exact-version rule. A `13.0`
artifact has no exception-table field and is rejected with
`UnsupportedVersion { found: 13.0, supported: 14.0 }`; it is not silently read
as v14 with empty tables. Future compatibility conversion, if accepted, must be
an explicit artifact migration outside the verifier.

## Linker and disassembler requirements

A whole-function linker copy preserves the table unchanged when instruction and
local numbering are unchanged. Any instruction-splicing linker must remap
`protected_start`, `protected_end`, catch `handler`, and `exit` by the same
instruction mapping used for the copied body; it must remap `error_local` by the
destination local-slot mapping and remap source IDs in `location`. It may copy a
record only when its protected body, skip/catch region, and exit are all
representable in the destination. Partial copies must be rejected, and the
linked module must be verified again.

A future disassembler should print one stable line per record before its
function instructions, for example:

```text
handler 0: protect pc0..pc1 catch pc2 error_local local0 exit pc3 @ source7:11..29
handler 1: protect pc4..pc5 catch pc6 error_local _ exit pc8
handler 2: protect pc9..pc11 swallow exit pc11
```

`pcA..pcB` remains half-open. Reassembly, once implemented, must preserve the
explicit `Swallow` versus `Catch` distinction and pass the same verifier.

## Required consumer work

Compiler, runtime, and kernel/linker changes are intentionally outside this
commit. Those consumers must add the new `Function` field or construction
builder, implement the emission/unwind/remapping rules above, and preserve the
v14 exact-version diagnostic. Until they do, workspace-wide exhaustive
construction or matching can fail even though `openmat-bytecode` itself is
fully verified.
