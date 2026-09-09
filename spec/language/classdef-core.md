# Release-one core `classdef` contract

## Included

- Class definitions in class files.
- Value classes and handle-semantics classes derived from `handle`.
- Constructors, instance methods, and static methods.
- Properties with default values.
- Public, protected, and private get/set or method access.
- Constant and dependent properties, including conventional accessor methods.
- One user-defined superclass. Handle semantics are inherited through that
  superclass.
- Ordinary method dispatch and basic arithmetic/comparison operator overloads.
- Basic homogeneous object arrays.
- Core reflection predicates: `class`, `isa`, and `isobject`.

## Parsed or diagnosed but not required to execute in release one

Unsupported class, property, or method attributes should produce a structured
diagnostic rather than a parser crash.

## Excluded from release one

- Multiple inheritance.
- Enumeration classes.
- Events and listeners.
- Dynamic properties and `dynamicprops`.
- The complete `meta.*` reflection system.
- `matlab.mixin.*` compatibility.
- Heterogeneous object arrays.
- Full property type/size validation syntax and semantics.
- User-defined serialization and complete custom display behavior.
- Complete custom indexing through `subsref`, `subsasgn`, `subsindex`, or `end`.

The runtime representation must leave room for later additions without exposing
Rust layout as a stable ABI.

