# `classdef` static methods and constant properties

This implementation closes the class-level execution path for the release-one
`classdef` core without changing the lossless frontend or resolving MATLAB's
call-versus-index ambiguity early.

## Frontend and compiler

`properties (Constant)` and `methods (Static)` remain ordinary source-backed
attributes in the CST and HIR. The compiler interprets those block attributes
when it builds class metadata:

- A constant property is emitted with `PropertyKind::Constant`, a required
  zero-input initializer function, get access, and no set access.
- An ordinary property is emitted with `PropertyKind::Stored` and explicit get
  and set access.
- A static method is emitted with `MethodKind::Static`. Its source parameters
  are its complete input list; no instance receiver is synthesized.
- An ordinary method remains `MethodKind::Instance` and must declare its
  receiver input.

Bytecode version 6.0 is the first format generation with the explicit property
kind and optional write access. The verifier checks property initializer
references and signatures, method body references, stored-versus-constant write
metadata, and constructor-only output metadata.

For a member receiver that is a bare name, the compiler emits `LoadGlobal` with
class construction disabled. Runtime name resolution can therefore return an
existing workspace object or a class reference. `GetField`, `SetField`, and
`ApplyField` retain dynamic member resolution, so the frontend does not decide
call versus indexing or instance versus class dispatch prematurely.

## Runtime behavior

Class registration evaluates each constant initializer exactly once in source
order. The resulting value is stored in persistent class metadata for the
session; it is not placed in an object slot and is not recomputed on access.
Registration and initialization observe cooperative cancellation, and member
and argument vectors retain checked allocation behavior.

A class reference supports only class-wide operations:

- Reading a constant returns its session value with normal language copy
  semantics.
- Calling a static method invokes its bytecode directly with the declared
  arguments and the declaring-class access context.
- Reading or writing a stored property through the class is rejected.
- Calling an instance method through the class is rejected by method-kind
  checking.

An instance may read an inherited or local constant, but both instance and
class assignment resolve to the same structured read-only property error.
Static method execution does not allocate an object. A static method may load
its own class reference and read a constant from the same registered class.
Every generated member instruction retains its source location for runtime
diagnostics and stack frames.

## Deferred class features

Dependent properties and dotted accessors, operator overload execution, and
homogeneous object-array execution remain outside this closure. They continue
to use the existing structured compiler or runtime rejection paths rather than
falling back to static/constant behavior.
