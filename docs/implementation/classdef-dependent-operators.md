# Dependent properties and basic operator dispatch

This tranche completes the executable path for conventional dependent-property
accessors and the `plus` instance operator described by the R2022b class core
contract. It extends bytecode to version 7.0; older bytecode remains rejected by
the existing exact-version verifier boundary.

## Dependent-property representation

`properties (Dependent)` lowers to `PropertyKind::Dependent`. A dependent
property has no initializer and never contributes an instance storage slot. Its
optional get and set access values encode whether `get.Name` and `set.Name` are
present, so getter-only and setter-only declarations remain distinguishable.

The compiler and verifier require conventional accessors to be instance methods
with these signatures:

- `function value = get.Name(object)`
- `function object = set.Name(object, value)`

A dependent declaration without either accessor, an initializer, a dotted
method targeting a non-dependent property, a static accessor, or a wrong
input/output arity is rejected with structured compiler or bytecode-verifier
diagnostics. Reading a setter-only property and writing a getter-only property
produce structured object errors.

At runtime, property resolution selects the accessor virtually through the
class registry. Getter execution returns the requested value. Setter execution
must return an object of the receiver class; that value becomes the assignment
result, preserving value-class copy/update behavior while handle classes retain
their shared identity. Accessor bodies execute with the declaring class as the
access context, which permits an accessor to read or write its class's private
backing property without granting that access to external callers.
An accessor override in a subclass is accepted without redeclaring the inherited
dependent property; the class registry validates and selects it at runtime.

## `plus` operator dispatch

The interpreter leaves the existing scalar and array arithmetic path unchanged
when neither operand is an object. If an object participates in binary `Add`, it
requires compatible object classes, resolves `Operator::Add` through
`ClassRegistry::resolve_operator`, and invokes the selected instance method as
`plus(left, right)` with one requested output. Method selection therefore uses
the same access and virtual-dispatch rules as ordinary class methods.

Missing `plus` methods, heterogeneous object operands, or object participation
in an operator not enabled by this tranche return structured invalid-operands
errors. The bytecode-to-object operator mapping is centralized for later
subtraction, multiplication, division, power, and comparison work, but only
`Add`/`plus` is executable in this tranche.

## Verification coverage

Parser and HIR tests assert lossless preservation of `get.Name` and `set.Name`.
Compiler and bytecode tests cover emitted metadata and malformed signatures.
Object tests cover slot-free dependent properties and read/write modes. Runtime
tests execute accessors and operator dispatch directly, including missing and
heterogeneous operator failures. Kernel tests load real class files and execute
the R2022b-shaped dependent-property and `plus` programs end to end.
