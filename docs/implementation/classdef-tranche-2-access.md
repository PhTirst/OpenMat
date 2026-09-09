# `classdef` tranche 2: access and direct inheritance

This note describes the focused access-control and direct-inheritance tranche.
The accepted language specification remains authoritative.

## Versioned representation

Bytecode v4 adds explicit `Public`, `Protected`, and `Private` access values.
Stored properties serialize independent get and set access, methods serialize
call access, and class definitions serialize an optional direct user-defined
superclass name. Direct `< handle` remains encoded as declared handle semantics;
a user subclass inherits effective handle semantics from the object registry.

A syntactically bare unresolved name also records whether runtime class
resolution should perform implicit zero-input construction. Workspace bindings
still win name resolution, and explicit `ClassName()` continues through ordinary
runtime application.

The verifier rejects empty and immediately self-referential superclass names in
addition to the existing class function, member name, signature, and register
checks. The serialized fields are a format contract and do not expose Rust
layout as an ABI.

## Access execution

The compiler accepts `Access`, `GetAccess`, and `SetAccess` values of `public`,
`protected`, or `private`. Other property or method attributes keep their
structured unsupported diagnostics.

Each executing class method carries the class that declared the selected method
as its access context:

- public members are visible to all callers;
- private members are visible only when that declaring class is the current
  method class;
- protected members are visible when the current method class is the declaring
  class or one of its subclasses;
- entry scripts and ordinary functions use an external context.

Inherited methods execute with their declaring class context. Consequently a
base method invoked on a derived object can read its base-private members, while
a derived method cannot. Access is checked before a value receiver is copied for
mutation, so a rejected write creates no detached object record.

`ObjectError::AccessDenied` maps to the dedicated runtime error variant and the
stable kernel code/category `OMR0017` / `access-violation`; classification does
not inspect display text.

## Direct superclass linking and storage

The real-file linker follows the serialized superclass name through the same
canonical MATLAB source resolver used for functions and classes. It links the
base class before the derived class, detects superclass cycles, retains source
origins for linked method functions, and reuses classes already registered in
the session.

Registration continues to use `ClassRegistry<Value>` and `ObjectStore<Value>`.
The registry supplies inherited base-to-derived stored-property layout,
effective value/handle semantics, virtual method selection, and declaring-class
metadata. Runtime method code is selected from the registry's declaring class,
so the existing override foundation works without a parallel dispatch table.

## Covered clean-room cases

Real temporary-file `RuntimeEngine::execute_file` tests cover the four focused
R2022b shapes:

- public property access plus a public method reading a private property returns
  `[7, 11]`;
- external private property access returns `access-violation`;
- a derived value object reads/writes inherited protected state while an
  inherited base method uses base-private state, returning
  `[10, 20, 40, 3, 6]`;
- external protected property access returns `access-violation`.

The same test also rejects external calls to protected and private methods and
checks canonical source ranges on the focused access error.

## Deliberate boundary

This tranche does not add multiple inheritance, access lists naming selected
classes, explicit `obj@Base` construction or base-method calls, inherited
constructor chaining with user code, abstract/sealed behavior, static methods,
constant/dependent execution, operator dispatch, or object arrays. In
particular, a derived class with no constructor supports the implicit zero-input
path used by the focused cases; executing a user-defined base constructor as
part of derived construction requires a later constructor-chain contract.

Class invalidation, file-change redefinition, and `clear classes` lifecycle also
remain later work. They should extend the versioned class/link metadata rather
than expose registry or object-store Rust layout.
