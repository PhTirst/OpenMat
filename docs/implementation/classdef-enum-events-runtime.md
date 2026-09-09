# Classdef enumeration and event runtime

This note describes the G2 execution bridge for the bytecode-22 class feature
table. The parser, HIR, compiler, and bytecode schema are inputs to this work;
the runtime does not reinterpret class source or add an enum/event opcode.

## Class registration and linking

`Interpreter::register_class` finds the feature record keyed by the bytecode
class ID. It installs the raw enumeration base, ordered member descriptors,
events and their access metadata, abstract property slots, and sealed method
bits into `openmat-object`. Runtime-only enum member code retains the owning
module, initializer function ID, and expected argument count.

The kernel may link a class from a support file after the request module is
compiled. Its runtime-engine bridge therefore copies the linked feature record,
remaps its class ID through `MergedModule::class`, remaps every enum initializer
through `MergedModule::function`, and appends the result beside the merged
class. This keeps the version-22 metadata table aligned without changing the
general linker contract.

## Enumeration member state machine

The member cache key is `(ClassId, String)`. A separate loading set is the
recursion/transaction marker.

1. A cached value member is language-copied; a cached handle member is returned
   as the same identity.
2. An uncached lookup inserts the loading key, executes the initializer with
   its declared output arity, and invokes the class constructor through a
   privileged enum-only allocation path.
3. Only a constructed result is inserted into the cache. Every failure removes
   the loading key and rolls back incomplete object state.
4. Ordinary class construction still calls `ensure_instantiable`, so an enum
   constructor cannot be reached directly.

Enum cache values are permanent collector roots. Explicitly deleting a cached
handle member changes its object-store state to invalid but does not evict the
cache; this preserves the R2022b identity observed by later qualified lookup.

## Listener representation and ownership

Every runtime-loaded root handle class receives a private stored cell slot for
attached listener roots; subclasses inherit its property key. The first
`addlistener` lazily registers the sealed hidden implementation class
`event.listener`. A listener object stores a source edge and callback edge in
ordinary object slots, while interpreter maps retain dispatch metadata:

- listener ID to source, event name, callback, and callback-owning module;
- `(source ID, event name)` to listener IDs in registration order;
- active listener IDs for nested-dispatch suppression.

The object slots, not the maps, define GC reachability. Source and listener
therefore retain one another while either side has an external root. The maps
are removed when the listener is explicitly deleted or becomes a finalizer
candidate.

## Dispatch and teardown

`addlistener` accepts one scalar valid handle, an inherited event name, and a
function handle. It checks `ListenAccess` with the bytecode frame's class access
context and returns the newly allocated stable listener handle. `notify`
checks `NotifyAccess`, constructs default `{Source, EventName}` event data when
none was provided, then copies the current listener-ID vector.

Dispatch walks the snapshot in reverse registration order. Before each call it
verifies that the ID is still attached and alive and is not already active.
This makes deletion immediate for a listener whose turn has not arrived,
defers additions to the next notification, and prevents self-recursive nested
delivery while allowing other listeners to run. Callback errors are queued as
warnings and never abort later callbacks in the snapshot.

Explicit source deletion emits the synthetic public
`ObjectBeingDestroyed` event before the object enters `Finalizing`, then
detaches every listener and executes the existing destructor queue. Listener
handles survive source deletion but no longer dispatch. The tracing collector
marks every unreachable handle as a finalizer candidate. Candidate order is
descending object ID, so the newer listener in an unreachable source/listener
cycle detaches before the older source destruction hook. Finalizer callbacks
continue to use the existing controlled read-only receiver capability.

## Tests and known boundaries

Rust tests cover enum cache publication/rollback, event inheritance and access,
stable listener IDs, explicit listener invalidation, bidirectional roots, and
cycle reclamation. MATLAB/OpenMat differential cases cover the visible ordering
and lifecycle matrix.

The runtime currently supports scalar handle sources and function-handle
callbacks. Callback-cell syntax, source arrays, full `event.EventData` class
semantics, reflective listener properties, and MATLAB diagnostic text are
deferred. The default event-data struct is sufficient for callbacks that
consume `Source` and `EventName`, but is not claimed as a general replacement
for user-defined event-data subclasses.
