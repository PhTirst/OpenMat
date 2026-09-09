# `classdef` direct-inheritance construction and dispatch

This note records the executable single-inheritance completion built on the
accepted `classdef` and R2022b compatibility contracts. Those contracts remain
authoritative.

## Explicit superclass construction

The lossless syntax layer has a dedicated node for
`object@DirectSuperclass(arguments)`. HIR retains four independently located
parts: the constructor output object, the written superclass name, the
arguments, and the full call range. It is not lowered as an unresolved ordinary
application or as a function handle.

The compiler accepts this form only as a standalone statement in the
constructor declared by the current class. The receiver name must be that
constructor's first output, and the named class must equal the class
definition's direct user superclass. Calls from ordinary methods, nested
expressions, classes without a user superclass, or calls naming a non-direct
ancestor are diagnosed before bytecode is produced.

Bytecode v6 adds `InvokeSuperclassConstructor`. The instruction contains:

- a string constant naming the direct superclass;
- the current constructor output local, not an arbitrary object register;
- evaluated argument registers.

The verifier independently finds the unique class constructor that owns the
instruction. It rejects a non-constructor function, an incorrect superclass
name, an incorrect constructor-output local, non-string metadata, and invalid
frame operands. This preserves the constructor-only and direct-ancestor rules
when bytecode did not originate from the normal compiler.

## Runtime object and constructor behavior

The object store allocates one record for the dynamic class before user
constructor code runs. Its stored-property map already contains inherited
properties in base-to-derived declaration order, keyed by their declaring
class. A direct superclass call therefore seeds the base constructor's output
local with the same incomplete object handle; it never allocates a separate
base object and never changes the object's dynamic class.

Runtime validation repeats the security-relevant bytecode assumptions. The
current frame must carry a declaring-class access context, the named class must
be that class's registered direct superclass, the output object must belong to
the current hierarchy, and its construction state must still be
`Constructing`. Constructor access is checked with the derived declaring class
as caller. The base constructor body itself executes with the base declaring
class as its access context.

The outermost dynamic-class construction owns the transaction. Base and
derived constructor bodies mutate the same incomplete record. If argument
preparation, access checking, cancellation, a base constructor, a derived
constructor, or final object validation fails, the record is marked failed and
discarded. The caller's workspace store instruction is never reached. Only the
outermost constructor changes the record from `Constructing` to `Constructed`.

Implicit zero-input base constructors remain available when the direct base has
no declared constructor. Supplying arguments to such an implicit constructor
produces the normal input-arity error.

## Dispatch and reflection

Method lookup always begins at the object's dynamic class. Consequently an
inherited base method such as `describe` can call `object.score()` and select a
derived override. Access checking at that call site uses the class that declared
the currently executing base method; the selected override body then executes
with its own declaring class context. This keeps virtual selection separate
from private/protected access identity.

Core reflection uses the same dynamic object metadata:

- `class(object)` returns the dynamic class name;
- `isa(object, name)` walks the complete registered superclass chain;
- `isa(object, "handle")` is true only for effective handle semantics,
  including semantics inherited through a user superclass;
- `isobject` is true only for a live classdef object value.

An unrelated handle class therefore does not make a value-class hierarchy
satisfy `isa(object, "handle")`.

## Linking and diagnostics

The kernel still resolves superclass files through the canonical MATLAB source
resolver and links base class registration before derived registration. Missing
superclasses, dependency-name mismatches, and cycles keep their existing source
errors. Linked constructor functions retain their original source identifiers,
and `InvokeSuperclassConstructor` carries the written call range for runtime
diagnostics and stack traces.

The new instruction participates in checked constant/register remapping during
module linking. It uses the normal per-instruction cancellation check and the
existing checked `u32` table/register allocation paths; no unchecked platform
index conversion or Windows-specific behavior was added.

## Verification coverage and boundary

Tests cover lossless parser and HIR representation, legal and illegal compiler
contexts, bytecode verifier rejection, transitive object ancestry, same-record
base initialization, failed-base rollback, and real temporary class files. The
two focused conformance programs produce `[12, 4, 12]` for override dispatch and
`[true, true, true, false, true, false]` for reflection.

This inheritance tranche composes with the existing bytecode-v6 static-method
and constant-property execution without changing those semantics. It does not
add multiple inheritance, explicit base-method dispatch, constructor call-order
analysis, automatic insertion of omitted user-base constructor calls, dependent
property execution, operator overloads, object arrays, class invalidation, or
the full `meta.*` system.
