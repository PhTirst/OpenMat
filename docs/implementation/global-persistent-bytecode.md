# Global and persistent bytecode v15

This implementation contract defines the bytecode boundary accepted for the
compiler and runtime follow-up tranches. Bytecode version `15.0` adds executable
declaration instructions and function-owned persistent-slot operands. It does
not implement compiler binding decisions or runtime storage.

## Execution and ownership model

`DeclareGlobal` and `DeclarePersistent` are ordinary instructions. The compiler
must emit them at the declaration's source position, attach the declaration's
`SourceLocation` to the surrounding `Instruction`, and leave them in the
control-flow path on which the source declaration occurs. A declaration in a
branch which is not taken therefore does not execute. Optimizers must not hoist,
deduplicate, or remove either instruction without a later semantic contract
which proves that transformation valid.

A global name identifies one binding in the workspace owned by an
`Interpreter` session. Every function executed in that session observes the
same binding for the same string name. This bytecode contract carries the name;
the runtime follow-up owns creation, value, reset, and lifetime behavior.

A persistent operand is a `PersistentSlot`, a `u32` format index bounded by the
containing function's `persistent_slot_count`. Slots are function-owned, not
frame-local and not module-global. The runtime follow-up must define stable
runtime function identity and use `(runtime function identity, PersistentSlot)`
as the storage key. That task also owns storage lifetime and reset behavior.

## Versioned function record

The v15 logical function record adds:

```text
persistent_slot_count: u32
```

`Function::new` initializes the count to zero. The
`with_persistent_slot_count` builder sets it explicitly. Whole-function clones
and module linking preserve the value. When the existing kernel linker splices
an entry body into another function, it checked-adds both counts and offsets
every spliced persistent operand by the target's original count.

## Instruction discriminants

The four v15 instructions have the following explicit logical serialization
discriminants and operands:

| Discriminant | Instruction | Operands |
| ---: | --- | --- |
| `41` | `DeclareGlobal` | `name: ConstantId` |
| `42` | `DeclarePersistent` | `slot: PersistentSlot` |
| `43` | `LoadPersistent` | `dst: Register`, `slot: PersistentSlot` |
| `44` | `StorePersistent` | `slot: PersistentSlot`, `src: Register` |

These integers are format-level opcode tags. A future encoder must write and
decode them explicitly. Their values do not come from Rust declaration order,
`mem::discriminant`, or any other property of the in-memory enum layout. This
contract assigns only the four v15 additions; it does not retroactively define
the physical encoding of pre-v15 variants.

## Verification

The verifier accepts only exact version `15.0` and rejects v14 artifacts with
an `UnsupportedVersion` error. For each containing function it verifies:

- `DeclareGlobal.name` exists and is a `Constant::String`;
- every `PersistentSlot` is strictly less than
  `Function.persistent_slot_count`;
- `LoadPersistent.dst` and `StorePersistent.src` are within
  `Function.register_count`;
- all pre-existing operands and metadata retain their existing checks.

Source locations remain diagnostic metadata on `Instruction`; verifier errors
for the new operands report that location through the existing error surface.

## Follow-up interfaces

The compiler task must introduce a binding table whose storage alternatives
are equivalent to:

```text
Local(LocalSlot)
WorkspaceGlobal(ConstantId)
Persistent(PersistentSlot)
```

It must allocate persistent slots per function, set
`persistent_slot_count`, emit located declaration instructions on the actual
control-flow path, and route every affected name read and write through the
selected storage kind. It must not allocate an ordinary local slot for a global
or persistent declaration name.

The runtime task must implement `DeclareGlobal` against the interpreter-session
workspace and implement the three persistent operations against storage keyed
by runtime function identity plus slot. Declaration must establish absent
storage without overwriting existing storage. The canonical initial value,
function identity across linking/reload/closures/methods, clear behavior,
unwind and cancellation behavior, and session teardown remain runtime
decisions. Until that task lands, the interpreter has an explicit invalid-state
placeholder for all four instructions; it never treats them as no-ops.

No kernel protocol, serialized workspace snapshot, compiler lowering, binding
diagnostic, or persistent/global storage behavior is added by this tranche.
