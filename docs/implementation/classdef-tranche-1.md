# `classdef` execution tranche 1

This note records the first executable `classdef` tranche. The accepted
language specifications and RFCs remain authoritative; this document describes
the implemented subset and the explicit boundary to later work.

## Executable path

A class file now follows the normal source pipeline:

```text
lossless parser -> HIR -> bytecode v3 class table -> RegisterClass
                -> ClassRegistry<Value> + ObjectStore<Value>
```

Bytecode v3 carries source-located class, stored-property, default-expression,
and method descriptions. The verifier checks class-table indices, referenced
default/method functions, zero-input default signatures, and constructor output
slots. `GetField`, `SetField`, and `ApplyField` retain source provenance.

`ApplyField` preserves the frontend's unresolved application rule. At runtime,
`obj.member(args)` first performs instance-method lookup. If no method exists,
the runtime reads `member` as a property and applies the arguments as indices to
that value. The parser and HIR therefore do not prematurely turn all
parenthesized member applications into calls.

## Included semantics

- A class file request atomically registers one default value class or one
  direct `< handle` class in the runtime session.
- Stored public properties support an implicit empty-double default or an
  executable scalar, array, character/string expression default.
- An absent constructor is an implicit public zero-input constructor. Passing
  inputs to it is an input-arity error.
- A same-name constructor receives a preallocated object in its first output
  slot. Constructor body failure removes the incomplete record and never
  creates the requested workspace binding.
- Public property read/write and public instance method dispatch work both in
  scripts and method bodies.
- A value receiver is copied on assignment and argument transfer. Mutation in
  a value method is observable when the method returns the updated object and
  the caller assigns that result.
- A handle receiver retains identity across assignment, argument transfer, and
  requests, so mutation is visible through every alias.
- `class`, `isa`, and `isobject` understand tranche-one objects. A direct
  handle class also satisfies `isa(obj, "handle")`.
- Classes, executable method bodies, objects, and workspace bindings persist
  across sequential `ExecuteRequest` messages. Shutdown clears all four.
- Workspace summaries report the actual class name and scalar `1x1` shape.
  Inspecting a scalar object returns its class and a bounded `Missing` payload,
  because kernel protocol v0 has no property-structure preview form.

Runtime errors retain the failing instruction range and bytecode stack. The
kernel keeps a session source-id table, so a method invoked from a later request
still reports its original class-file source name and range, with stack ranges
as related diagnostics. Cancellation remains checked between every bytecode
instruction and around built-ins; class default, constructor, and method
functions use the same execution path.

## Structured deferred features

The following forms remain parseable but do not execute in tranche 1:

| Feature | Tranche-one result |
| --- | --- |
| Class attributes | compiler `OMC0007` with the attribute name |
| `Static` methods | compiler `OMC0007` method-attribute diagnostic |
| `Constant` properties | compiler `OMC0007` property-attribute diagnostic |
| `Dependent` properties | compiler `OMC0007` property-attribute diagnostic |
| `get.Name` / `set.Name` accessors | lossless CST and one HIR method name, then compiler `OMC0007` |
| protected/private access | compiler `OMC0007` for non-public `Access` |
| User superclass and override | compiler `OMC0007` identifying the superclass |
| Operator overload methods | compiler `OMC0007` identifying the method |
| Homogeneous object arrays | runtime `OMR0015` / `runtime.unsupportedClassFeature` |

This is deliberate rejection, not partial execution with guessed semantics.

## Verification

The repository tests cover parser/HIR member syntax and dotted accessors;
positive and negative class bytecode verification; compiler emission and
deferred diagnostics; direct runtime value/handle copying, properties, methods,
and failed-constructor rollback; and kernel multi-request registration,
persistence, summaries, inspection, source ranges, stack ranges, cancellation,
and shutdown.

Three locally authored programs were also executed against the installed MATLAB
R2022b 9.13 black-box oracle. Their observed results were:

- value assignment plus returned update: mutated value `7`, assigned copy `5`;
- handle assignment plus in-place update: both aliases `7`;
- constructor, array/string defaults, and method: second value `60`, label
  `ready`.

All three reported the authored class name and true results for the relevant
`isa` and `isobject` predicates. No MATLAB implementation source, tests,
documentation, or diagnostic prose was read or copied.

## Tranche 2 interface work

Later execution work should extend the versioned bytecode description rather
than expose Rust layout or replace the object core. It needs:

1. serialized property kind and get/set access metadata, plus executable
   dependent accessor links;
2. serialized method access and executable static dispatch through a class
   receiver;
3. a versioned superclass reference and inherited executable-code lookup,
   including override validation;
4. operator instructions that consult `ClassRegistry` dispatch before ordinary
   numeric evaluation;
5. a language-level homogeneous object-array value representation backed by
   `ObjectArrayDescriptor`, including indexing and copy behavior;
6. an explicit class invalidation/redefinition policy for file changes and
   `clear classes`-style lifecycle work.

Multiple inheritance, enumeration, events/listeners, dynamic properties,
heterogeneous arrays, complete `meta.*`, and custom indexing remain outside the
release-one core contract as specified.
