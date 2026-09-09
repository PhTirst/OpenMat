# MATLAB R2022b enumeration, events, and abstract-member model

This compatibility note records the independently testable front-end and
metadata boundary added for enumeration blocks, events blocks, abstract
properties, and sealed methods. Observations were made on 2026-08-31 with the
locally installed MATLAB R2022b executable and OpenMat-authored black-box
classes. The probes retained only Boolean load outcomes and public metadata
values; no MATLAB source, tests, documentation, diagnostic identifiers, or
diagnostic messages are copied into the repository.

## R2022b observations

`enumeration ... end` contains named members with optional constructor
arguments. A class with no explicit base can use an ordinary class constructor,
and explicit `uint8`, `double`, and `handle` bases all loaded in the sampled
probes. The arguments belong to the member declaration and are evaluated for
member construction rather than becoming ordinary property defaults.

An enumeration class must declare at least one member. Duplicate member names
and collisions between a member and a property are rejected. Enumeration
classes cannot be abstract and cannot be subclassed. That inheritance rule is
independent of the public `Sealed` flag: a sampled enumeration class reported
`Sealed == false`, an explicit `Sealed = false` class loaded and still reported
false, and its attempted subclass was nevertheless rejected.

Events require handle-class semantics in the sampled R2022b behavior. A direct
value class containing an events block did not load, while a direct handle
class did. `ListenAccess = protected`, `NotifyAccess = private`, and
`Hidden = true` were retained by event metadata. Effective reflection includes
inherited events, so OpenMat keeps source-local event declarations separate and
leaves inheritance expansion to class linking. Duplicate event names and an
event/property name collision were rejected.

Successful qualified enumeration lookup evaluates the member initializer and
constructor once, then reuses the member. A failed construction does not
publish a partial singleton and a later lookup retries. For a handle-valued
enumeration, explicitly deleting the member invalidates it; later qualified
lookup returns that same invalid handle rather than constructing a replacement.

Event listeners run newest-first in stable reverse registration order. Each
`notify` takes a listener-ID snapshot: listeners added by a callback wait for a
later notification, while a listener deleted before its turn is skipped. A
currently executing listener is suppressed from a notification nested inside
that same callback. A callback error was not thrown back to the notifier and
did not prevent the remaining snapshotted listener from running.

The source retains an attached listener even after its last local listener
variable is cleared, and a live listener handle keeps its source alive.
Explicitly deleting a listener detaches and invalidates it. Explicitly deleting
a source emits `ObjectBeingDestroyed` before its user `delete` method, detaches
the callbacks, invalidates the source, and leaves the listener handle itself
valid. When the last external roots of a source/listener cycle are cleared,
R2022b detaches the unreachable listener before source finalization, so that
listener does not receive `ObjectBeingDestroyed` during cycle collection.

A property block marked `Abstract` makes its class effectively abstract even
when the class header omits `Abstract`. A concrete subclass discharges the slot
by declaring a property with the same name and exactly matching get/set access;
the sampled access mismatch was rejected. `Abstract` combined successfully
with both `Dependent` and `Constant`. A sampled dependent abstract property was
implemented by a stored property, so abstract-property completion is based on
name and access rather than identical storage kind. The sampled abstract
constant property reported public get access and no set access.

`methods (Abstract, Sealed)` is legal in R2022b. The abstract slot remains
visible as sealed metadata, and an attempted override was rejected. This can
intentionally leave an abstract contract with no concrete subclass
implementation. It is therefore not valid to simplify `Abstract + Sealed` to
an attribute conflict.

## OpenMat front-end and metadata contract

The lexer keeps `events` contextual: it remains an identifier outside a class
body. The parser produces dedicated lossless `EnumerationBlock`,
`EnumMemberDecl`, `EventsBlock`, and `EventDecl` nodes and retains malformed
members in an error-tolerant tree. HIR preserves block attributes, declaration
order, enum constructor expressions, event names, and source ranges.

The compiler performs local duplicate/conflict checks, rejects empty
enumerations and direct value-class events, recognizes event visibility and
hidden metadata, and rejects inheritance from an enumeration declared in the
same compilation unit. Cross-module enforcement belongs to the object linker.
Every enum member receives a zero-input bytecode function that evaluates its
constructor arguments and returns them in declaration order. This delays all
runtime object lifecycle policy without erasing source evaluation semantics.

The object model stores enumeration base/member metadata and event access
metadata, rejects direct ordinary construction of an enumeration, treats an
enumeration as non-inheritable independently of its reflected sealed flag, and
requires effective handle semantics for events. Its abstract-slot linker now
includes properties and exact get/set access. Method descriptors carry a
sealed bit, and override validation rejects both concrete and abstract sealed
method overrides.

## Bytecode compatibility

The logical bytecode schema advances from 21.0 to 22.0. The legacy
`ClassDefinition`, `PropertyDefinition`, and `MethodDefinition` records remain
source-compatible; version-22 additions live in a parallel
`ClassFeatureDefinition` table keyed by `ClassDefinitionId`. A feature record
contains:

- ordinary versus enumeration class kind and the raw enumeration base name;
- ordered enum member names, argument-initializer function IDs, and arities;
- ordered event names with listen access, notify access, and hidden state;
- abstract property names, source property kinds, and get/set access;
- names of local sealed methods, including legal abstract sealed slots.

The verifier accepts current 22.0 modules and featureless 21.0 modules. It
rejects a 21.0 module that carries the new table rather than guessing a schema,
and verifies feature/class indices, initializer function arity, empty and
duplicate names, namespace conflicts, abstract-property access, and sealed
method targets. The repository still has no serialized encoder/decoder; a
future decoder must materialize an empty feature table when decoding 21.0 and
must decode the table explicitly for 22.0.

## OpenMat G2 runtime contract

Module registration now consumes `ClassFeatureDefinition` and installs enum,
event, abstract-property, and sealed-method metadata on the corresponding
runtime class. Linked support modules remap enum initializer function IDs and
merge their feature records with the linked class IDs.

Enum members use an interpreter-owned `(ClassId, member name)` cache. The
privileged member path evaluates the zero-input initializer, invokes the normal
class constructor, and publishes only a fully constructed object. Recursive
construction is rejected; any initializer or constructor failure rolls back
the object and loading marker so a future lookup can retry. Value members cross
the ordinary language-copy boundary on every lookup. Handle members return the
interned identity, including its invalid state after explicit deletion. Direct
calls to an enumeration constructor still fail independently of member lookup.

`addlistener` and `notify` are runtime callables rather than bytecode opcodes.
An attached listener is a real sealed handle object with class name
`event.listener`, a stable object ID, stored callback, and a traced source
edge. The source contains a private runtime-owned listener-root slot. These two
edges reproduce R2022b retention while allowing the tracing collector to
reclaim an unreachable cycle. Event lookup is inherited and checks
`ListenAccess` or `NotifyAccess` against the declaring class.

Dispatch snapshots listener IDs, walks them newest-first, rechecks liveness and
attachment before each callback, defers additions, honors removals, and skips
the active listener in nested notification. Callback failures are contained,
the snapshot continues, and OpenMat emits a warning after the outermost
dispatch. Explicit listener deletion detaches first. Explicit source deletion
emits `ObjectBeingDestroyed` while the source is still readable, then detaches
listeners before normal two-phase handle finalization. GC finalizer candidates
are processed newest-ID-first; unreachable listener objects therefore detach
before their older source in a collected cycle.

The checked-in execution references are the `enum_*` and `event_*` conformance
cases under `tests/conformance`. They were generated by the clean-room MATLAB
R2022b oracle and then passed by OpenMat. They cover successful and failed enum
construction, handle-enum deletion, event access, dispatch ordering and
mutation, nested notification, callback errors, listener/source deletion,
destruction ordering, and source/listener GC retention.

Current narrow boundaries are explicit. Event sources are scalar handle
objects and callbacks are function handles; MATLAB callback-cell forms and
source arrays are not implemented. Default event data is an OpenMat struct
containing `Source` and `EventName`, not an `event.EventData` subclass. Warning
identifiers and messages are OpenMat-owned rather than copies of MATLAB text.
