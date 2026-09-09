# Try/catch kernel linking

This tranche closes the kernel-side bytecode v14 linker and script-discovery
work. It is based on integration commit
`pre-publication baseline` and does not implement compiler
lowering or runtime unwinding.

## Whole-function merge

`merge_support_module` clones support functions before remapping their
function and class references. Because instruction and local numbering do not
change in this path, each function's `exception_handlers` vector is preserved
in order and without field changes. This applies to both the detached support
entry returned in `MergedModule` and non-entry functions appended to the target
module.

## Entry splicing

Both class-entry prepend and script-entry inline use `splice_entry`. The copied
entry receives destination bases for constants, ordinary registers, pack
registers, and locals. `local_count` grows by the copied entry's complete local
count, and all local operands are rebased, including:

- `LoadLocal` and `StoreLocal`;
- the constructor object local in `InvokeSuperclassConstructor`;
- local `StatementResultTarget` values on all statement-result instructions;
- `Catch.error_local` when a catch binding is present.

For an entry PC below the copied instruction count, the linked PC is the splice
index plus the entry PC. A verified entry's omitted terminal `Return` may be
used by `protected_end`, catch `handler`, or `exit`; that PC maps to the first
caller instruction after the splice. A protected start must name a copied
instruction. Any other partial or out-of-body handler reference is rejected as
a link error.

The same mapping is applied to every copied handler field:
`protected_start`, `protected_end`, catch `handler`, and `exit`. Handler vector
order is retained when the mapped handlers are appended after the target's
existing handlers.

Existing target jumps and exception metadata are adjusted together. Prepending
shifts a target at the insertion point so that it continues to name the
original instruction. Replacing a script load keeps a target at the replaced
PC on the first inserted instruction and shifts later PCs by the net splice
length. If deletion collapses an existing protected interval to empty, its
record is removed; the remaining catch code is unreachable ordinary bytecode.

Instruction locations and handler locations are copied unchanged. Kernel
source IDs are allocated centrally while compiling the primary and support
units, so preserving the source ID and byte range is the existing provenance
mapping; the splice does not renumber source IDs.

## Bare-script discovery

The HIR walk used to distinguish callable functions from bare workspace
scripts now descends into both `TryStatement.body` and the optional catch body.
Both sides inherit the existing `workspace_scope` flag. A catch binding is not
an expression statement and is therefore not recorded as a bare-script site.

## Verification coverage

Kernel unit tests construct bytecode v14 functions directly and verify linked
modules after:

- whole-function merge of entry and non-entry handler tables;
- inline splice with plain catch, bound catch, swallow, nested handlers, an
  existing outer target handler, local rebasing, and preserved source
  locations;
- prepend splice with both an inserted swallow and an existing catch handler.

A parser-to-HIR test also proves that bare script statements are collected from
both protected and catch bodies at top-level workspace scope and at function
scope, while catch bindings are excluded.

Real-source try/catch execution remains dependent on the separately coordinated
compiler-lowering and runtime-unwinding tranches. This kernel work deliberately
does not cross those crate boundaries.
