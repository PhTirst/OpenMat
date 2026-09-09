# MATLAB R2022b abstract and sealed classdef baseline

This phase extends the OpenMat class model from exact baseline
`pre-publication baseline`. The MATLAB observations were
made on 2026-08-30 with a locally installed MATLAB R2022b executable and
OpenMat-authored black-box probes. No MATLAB source, tests, documentation,
diagnostic identifiers, or diagnostic messages are retained.

## R2022b probe results

Class attributes are case-sensitive and require their complete spelling.
`Abstract` and `Sealed` are accepted; lowercase spellings and abbreviations are
rejected. Repeating either attribute is rejected. `(Abstract, Sealed)` is a
legal combination rather than a conflict. The explicit values `true` and
`false` are accepted, so `Abstract = false` and `Sealed = false` disable the
corresponding attribute. A class that explicitly sets `Abstract = false` while
declaring abstract slots is rejected.

An abstract method declaration is a bodyless signature inside a
`methods (Abstract)` block:

```matlab
methods (Abstract)
    value = transform(object, input)
    [left, right] = split(object, input)
    notify(object, input)
end
```

The declaration does not use the `function` keyword and has no method-level
`end`. Multiple outputs, no outputs, and ordinary input lists are accepted.
`Static` and `Access` can be combined with `Abstract`, including
`methods (Abstract, Static, Access = protected)`. A concrete function body in
an abstract block and a bodyless signature outside one are rejected.

Abstractness is effective, not merely textual. A class containing an abstract
method block is abstract even if its class header omits `(Abstract)`. An
otherwise concrete-looking subclass also remains abstract while any inherited
slot is unresolved. Metadata can be loaded for such a class, but construction
fails when instantiation is attempted.

Across multiple inheritance levels, a concrete method discharges an abstract
slot by exact method name. R2022b does not require matching input/output counts
for this link decision, and a static method can implement an inherited instance
abstract slot by name. For that cross-kind case, object-qualified dynamic
dispatch invokes the static implementation without adding the receiver.
The implementing method must preserve the abstract declaration's access level;
widening protected to public and narrowing it to private are both rejected.
A call that resolves to an unimplemented static abstract slot fails when
dispatch is attempted.

A sealed class can be constructed and used normally, but loading a direct
subclass of it fails. Abstract and sealed flags are visible through both
`metaclass(object)` and `meta.class.fromName(name)`. Effective method metadata
exposes `Name`, `Abstract`, `Static`, and `Access`. OpenMat phase one represents
this metadata as read-only struct-shaped values with those fields; it does not
yet claim the complete MATLAB `meta.class` object API.

## OpenMat implementation contract

The parser retains bodyless abstract declarations as dedicated lossless CST
nodes and recovers malformed signatures without discarding source text. HIR
keeps the name, output list, input list, and source span separately from
ordinary function bodies. The compiler emits a signature stub plus an
`is_abstract` method marker; the runtime never installs that stub as executable
class code.

The class linker computes the unresolved slot set over the complete
base-to-derived chain. It infers effective abstractness, rejects sealed direct
superclasses, validates exact access on abstract overrides, and keeps the
abstract slot's declaring class as the access owner while separately selecting
the concrete override's declaring class as the executable-code owner.
Instantiation checks effective abstractness before allocation. Virtual lookup
rejects an unresolved slot before looking for executable code.

Class reflection is compiler-recognized only when `metaclass` is not shadowed
and for the exact `meta.class.fromName` form. It lowers through an existing
field-read instruction using an internal field name that cannot be expressed as
a MATLAB identifier. This avoids widening the kernel module-linking boundary.

## Bytecode compatibility

The logical bytecode format advances from version 20.0 to 21.0 because class
and method table records gain required semantic fields:

- `ClassDefinition.declared_abstract: Option<bool>` distinguishes omitted,
  explicit true, and explicit false;
- `ClassDefinition.sealed: bool` records the direct-inheritance constraint;
- `MethodDefinition.is_abstract: bool` distinguishes signature stubs from
  executable methods.

No new opcode is introduced. Version 20.0 modules are rejected by the exact
version verifier instead of being guessed into the new schema. Version 21.0
verification covers abstract constructor rejection, constructor-output
metadata on abstract slots, and explicit-concrete/local-abstract conflicts.
The repository has no serialized bytecode encoder/decoder yet; the relevant
roundtrip guarantee is therefore an in-memory clone followed by verification,
covered alongside the new metadata verifier cases.

## Differential coverage

Eight new `class_abstract_*` and `class_sealed_*` cases have checked-in R2022b
observations:

- multi-level abstract-slot completion, protected dispatch, and concrete
  dynamic dispatch;
- class and method reflection, including effective flags and abstract-slot
  count;
- static implementation of an inherited instance abstract slot;
- direct abstract-class instantiation failure;
- effective abstractness from a missing inherited implementation;
- dispatch-time failure for an unimplemented static abstract slot;
- positive sealed-class construction, reflection, and method dispatch;
- rejection of inheritance from a sealed class.

The negative references retain only OpenMat-owned normalized categories:
abstract construction and sealed inheritance are `type-error`; unresolved
abstract static dispatch is `undefined-name`.

## Deferred features

This phase does not implement enumeration classes, events or listeners,
`parfor`, `spmd`, multiple inheritance, complete `meta.*` objects, property
validation attributes, mixins, dynamic properties, or MATLAB's full class
loading lifecycle. It also does not claim general concrete-method signature
compatibility; the name-only rule documented above is specifically the R2022b
abstract-slot link decision observed by these probes.
