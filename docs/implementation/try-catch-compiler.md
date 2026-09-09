# Try/catch compiler lowering

The compiler emits the bytecode v14 function-owned exception table described
in `try-catch-bytecode.md`. This note records the compiler-side boundary; it
does not define runtime error conversion or unwinding.

## Name analysis

Assigned-name traversal visits the protected body and catch body recursively.
A bound catch name is collected between those bodies in source order and is an
ordinary assignment in the surrounding scope.

- Functions and methods use the ordinary preallocated local for `catch name`.
- The synthetic script entry allocates an internal local for each emitted bound
  handler. The VM writes the caught value there, and the handler prologue uses
  `LoadLocal` followed by `StoreGlobal` to publish the same-named workspace
  binding before lowering the catch body.
- Plain catch has no error local. A try without catch uses `Swallow`.

The assigned-name result also keeps unresolved name and anonymous-function
capture analysis consistent inside both bodies. Named function-handle lowering
continues to resolve function identities directly.

## Instruction layout

For a catch form with at least one emitted protected instruction, the compiler
records the instruction index before lowering the protected body, then emits:

```text
protected_start:
    protected body
protected_end:
    Jump exit
handler = protected_end + 1:
    optional script workspace-binding prologue
    catch body
exit:
    continuation
```

The normal path therefore skips the catch region, and the catch region is
outside its own protected half-open interval. If the construct is inside an
outer protected body, the inner skip, handler, and continuation region remain
within or end at the outer protected boundary.

For a no-catch form, `protected_end == exit`. The compiler places an ordinary
jump-to-next continuation instruction at that index. Besides ensuring the exit
always names a real instruction, this makes nested no-catch source constructs
produce strictly nested rather than duplicate protected ranges.

If lowering the protected source body emits no instruction, the compiler emits
no exception record and omits an unreachable catch body. The function's final
return or a following construct supplies any continuation instruction that was
forward-referenced while lowering.

## Diagnostic boundary

Parser and compiler diagnostics still prevent module construction. They are
not bytecode runtime errors and are never represented by an exception-table
entry. Runtime catchable categories and the structured error value remain
outside the compiler boundary.
