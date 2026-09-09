# Global and persistent compiler lowering

Status: the original compiler tranche was implemented on OpenMat bytecode
version `15.0` and integrated with the interpreter-owned session storage
described in `docs/implementation/global-persistent-runtime.md`. Bytecode v17
extends the same binding analysis with shared lexical captures and ordinary
local/capture clearing.

This implementation follows `docs/design/global-persistent-r2022b.md` and
`docs/implementation/global-persistent-bytecode.md`. The HIR declaration is
kept as a declaration throughout compilation; it is never converted to an
assignment.

## Static binding analysis

Each executable lexical function is analysed before instruction lowering. The
analysis walks statements and nested control-flow bodies in source order.
Nested function definitions are discovered recursively and analysed as their
own executable lexical functions; names shared with an enclosing function are
represented as captures instead of being folded into the parent's body. Each
function produces one binding table with the following internal storage
alternatives:

```text
BindingStorage =
    Local(LocalSlot)
  | Captured
  | WorkspaceGlobal
  | Persistent(PersistentSlot)
```

Persistent slots are allocated in declaration/name order and are owned by the
containing bytecode function. The final count is copied to
`Function.persistent_slot_count`. A declared global or persistent name is
excluded from ordinary assigned-name local allocation. Function input ABI
positions still reserve the bytecode parameter slots required by
`parameter_count`; if an input is later declared global, that physical ABI slot
is not the name's binding storage. Global outputs do not allocate an ordinary
output local and are loaded from the workspace when returning.

Scripts use the same explicit table, but ordinary assigned script names remain
`WorkspaceGlobal`, preserving the existing workspace behavior. A script
`global` declaration selects the same storage and additionally emits the
declaration instruction at runtime.

Anonymous-function capture behavior remains value based. Local and persistent
free names are loaded from their selected storage into the capture operand;
workspace globals continue through the existing workspace/capture resolution
path. Nested function statements retain the compiler's existing unsupported
boundary and are not included in an enclosing function's declaration scan.

## Declaration validation and emission

Compiler diagnostic `OMC0011` validates declaration form and lexical context
before bytecode can be returned. It also reports:

- repeated persistent declarations;
- global/persistent collisions in either declaration order;
- persistent names which are inputs or outputs;
- persistent declarations after an earlier assignment or use of the name.

Repeated global declarations remain valid. A global declaration after earlier
use or assignment also compiles. MATLAB R2022b warns for the sampled
global-after-use case, but OpenMat has no compiler warning channel in this
tranche, so the compiler deliberately does not synthesize an error or a fake
warning.

`StmtKind::Declaration` has its own lowering arm. For every valid recovered
name it emits `DeclareGlobal` or `DeclarePersistent` at the statement's actual
control-flow position. The emitted `Instruction` carries the complete
declaration statement range as its `SourceLocation`. Declarations in branches,
loops, and handlers are therefore neither hoisted nor deduplicated.

## Storage-directed lowering

The following paths resolve the binding table rather than assuming a function
name is a `LocalSlot`:

| Compiler path | Local | Workspace global | Persistent |
| --- | --- | --- | --- |
| name read | `LoadLocal` | `LoadGlobal` | `LoadPersistent` |
| simple/multiple assignment | `StoreLocal` | `StoreGlobal` | `StorePersistent` |
| aggregate assignment root read | `LoadLocal` | `LoadGlobalOrNothing` | `LoadPersistent` |
| aggregate result write | `StoreLocal` | `StoreGlobal` | `StorePersistent` |
| function output | `LoadLocal` | `LoadGlobal` | rejected by the persistent/output rule |
| catch binding | direct handler local | scratch local then `StoreGlobal` | scratch local then `StorePersistent` |
| `for` variable | `StoreLocal` | `StoreGlobal` | `StorePersistent` |
| statement `ans` | `StatementResultTarget::Local` | `StatementResultTarget::Global` | `OMC0012` |
| named `clear` | existing function-local unsupported boundary | `ClearGlobal` | `OMC0012` |

`LoadGlobalOrNothing` remains intentional for a workspace aggregate root: it
is the bytecode operation reserved for growing a name-rooted value when the
workspace binding is absent. The write still uses `StoreGlobal`.

Automatic named display remains a `Display` instruction after the
storage-directed store. It labels the already evaluated register and does not
perform an independent binding read or write.

Catch metadata can encode only a `LocalSlot`. A global or persistent catch name
therefore uses a compiler-owned scratch local as the exception-handler operand,
then copies that value to the selected binding at handler entry. This scratch
slot is not a language-visible local binding.

## Explicit unsupported boundaries

Compiler diagnostic `OMC0012` is used when a name has been resolved to a
non-local storage class but the accepted bytecode cannot express the operation
safely:

- `StatementResultTarget` has no persistent alternative, so an expression
  statement whose statically declared `ans` is persistent is rejected;
- v15 defines no persistent clear/reset instruction or lifetime contract, so
  named `clear` of a persistent binding is rejected;
- class-constructor object seeding is defined in terms of a `LocalSlot`, so a
  non-local constructor output is rejected rather than producing invalid class
  metadata or falling back to a local binding.

Named clear of ordinary function locals emits `ClearLocal`; named clear of a
shared lexical variable emits `ClearCapture`; named clear of a declared global
inside a function emits `ClearGlobal`. Bare `clear` uses those storage-specific
operations for ordinary bindings while preserving persistent bindings. Clear
patterns, function-form arguments, and resource-unloading forms remain outside
this tranche.

## Runtime and compatibility limitations

The integrated runtime owns declaration initialization, persistent function
identity, lifetime/reset behavior, unwind and cancellation behavior, workspace
interaction, module replacement, and session teardown. Bytecode v17 still has
no canonical source-path identity, so same-named functions in the same class
context can share persistent slots when loaded from different files; see the
runtime implementation note for that exact boundary.

No MATLAB source, tests, documentation, or diagnostic text is incorporated by
this implementation. The compatibility decisions above are limited to the
OpenMat design observations and accepted bytecode contract.

## Verification coverage

`crates/openmat-compiler/tests/compiler.rs` covers cross-function global
instruction shape, script globals, global-after-use compilation, stable
persistent slots, conditional declaration locations, simple and aggregate
storage-directed assignments, outputs, catch bindings, `for` variables,
named clear, statement `ans`, closure capture, declaration form/context, and
all listed conflict diagnostics. The full pre-existing compiler suite remains
part of the required regression run.
