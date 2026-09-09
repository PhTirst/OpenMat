# Global and persistent runtime storage

This note records the interpreter semantics for the bytecode-v15 global and
persistent instructions. Compiler lowering is integrated separately as
documented in `docs/implementation/global-persistent-compiler.md`; kernel
protocols and workspace serialization are unchanged.

## Session storage

An `Interpreter` owns both stores. `DeclareGlobal` resolves its string constant
and ensures that the session workspace contains that name. An absent binding is
created as the canonical `0 x 0` double value, while an existing binding is
never overwritten. The workspace is shared by every function executed in the
same interpreter, so equal global names refer to the same binding.

Persistent values live in a separate interpreter-owned map rather than in an
`ExecutionFrame`. Its key is:

```text
(Function diagnostic name, caller ClassId if any, PersistentSlot)
```

The diagnostic function name is stable across `replace_module`, allowing a
replacement module with the same named function to retain its slots. The
caller class identity separates same-named method bodies belonging to different
registered classes. Ordinary differently named functions are also isolated.

Bytecode v15 does not carry a canonical source-path or definition identity.
Consequently, unrelated functions with the same diagnostic name and the same
class context still collide, including same-named functions loaded from
different files. This tranche deliberately documents that limitation instead
of claiming complete reload or path identity.

## Instruction behavior

- `DeclarePersistent` creates an absent slot as canonical `0 x 0` double and
  leaves an existing value unchanged.
- `LoadPersistent` clones the stored `Value`. If verified control flow reaches
  a valid slot before its declaration executed, the load first creates the
  canonical empty value.
- `StorePersistent` applies the runtime's existing language-copy operation
  before publishing the value. Arrays therefore retain copy-on-write behavior,
  value objects are assignment-copied, and handle objects preserve identity.
  A store to a valid but not-yet-declared slot creates that slot.

The verifier remains responsible for rejecting a `PersistentSlot` outside the
containing function's `persistent_slot_count` and non-string global declaration
constants. Runtime handling of verified operands does not add a second bounds
contract.

## Lifetime, unwind, and cancellation

Completed global and persistent stores are session writes. They survive normal
return, a caught error, and stack unwind; frame cleanup does not roll them back.
`replace_module` retains both the workspace and persistent map. `clear_session`
resets both stores together with class, object, closure, and function-handle
state. Neither store is written to disk or added to a kernel protocol.

The interpreter checks cooperative cancellation before dispatching every
instruction. A cancellation already requested before a declaration, load, or
store therefore prevents that instruction from creating or changing storage.
Errors continue to use the instruction's existing source location and call
stack metadata.

`Interpreter::persistent_binding_count` is read-only in-process introspection
for tests and hosts. It does not expose values and is not a Rust ABI boundary
for plugins or other processes.
