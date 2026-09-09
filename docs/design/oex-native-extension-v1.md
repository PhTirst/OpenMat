# OEX native extension ABI 1.1

Status: sealed implementation baseline for ABI 1.1; this document is not an
accepted RFC. Plugins built against the formative ABI 1.0 class descriptor must
be recompiled against the 1.1 header.

OEX is OpenMat's trusted in-process C ABI. It is intentionally low level, is
not MEX compatible, and never exposes a Rust ABI. Untrusted extensions require
a future out-of-process protocol.

## Public surface

A plugin includes the single public header `include/openmat/oex.h`, links to
`openmat_oex.dll`, and calls ordinary exported `Oex*` functions. Function
tables are not part of the public source API.

Opaque objects privately carry enough routing information for the bridge DLL
to reach the host that created them. The bridge owns no `OexValue`, call state,
array memory, or error memory. This permits the runtime to use the same C ABI
from a statically linked Rust host while plugins see a conventional import
library.

All public structs start with `structSize`. Static descriptor structs may grow
by appending fields; descriptor arrays are walked using each entry's declared
size. Output/view structures are frozen and a future incompatible shape uses a
new type and function. ABI 1.x may add functions, but existing fields,
constants, signatures, layouts, and semantics do not change. The ABI uses
`cdecl`, fixed-width integers, UTF-8 pointer/length metadata, and explicit
structure layouts. C `enum`, `bool`, `long`, `wchar_t`, exceptions, unwinding,
and cross-module allocators do not cross the boundary.

## Loading and static registration

Every plugin exports one ordinary entrypoint:

```c
OEX_PLUGIN_EXPORT const OexPluginDefinition *OEX_CALL OexPlugin_Init(void);
```

It returns process-lifetime static data containing plugin metadata and arrays
of function and class descriptors. OpenMat validates and copies the entire
known descriptor before registering anything. A bad descriptor therefore does
not leave a partially registered plugin. Loaded modules are pinned for the
kernel lifetime; hot unload is not supported.

Each function descriptor contains its complete language-visible name, input
range, output range, and one callback of type `OexStatus (*)(OexCall *)`.
A native handle-class descriptor contains its constructor, destructor, method
descriptors, and dependent-property descriptors. Instances use the ordinary
OpenMat object store, class identity, method/property dispatch, `clear`, and
garbage-collection lifecycle. The DLL remains pinned until all copied callbacks
and live instances are gone. Native value-class semantics are not yet supported
and are rejected at load time.
Every native class also supplies a unique non-null process-lifetime
`typeToken`; OpenMat compares the address as opaque identity and never
dereferences it.

## Calls and ownership

`OexCall` is valid only during one synchronous callback. It owns the input
slots, output slots, current error, cancellation handle, and unfinished
builders.

- `OexCall_BorrowInput` returns an immutable borrowed `OexValue`.
- `OexCall_TakeInput` consumes that input slot and returns an owned value.
- `OexValue_Retain` creates another owned handle using OpenMat copy-on-write
  storage.
- `OexCall_SetOutput` consumes an owned handle only when it succeeds.
- `OexValue_Release` releases an owned handle that was not consumed.
- Every requested output must be set exactly once before successful return.

A callback may call `OexCall_SetError` to copy an identifier and message into
host storage. Returning a nonzero status discards host-owned input slots,
staged outputs, and unfinished builders. It does not release an owned
`OexValue` previously taken, retained, created, or committed but not transferred
with `OexCall_SetOutput`; the plugin must release such handles on every failure
path.

## Native object bridge

Language object values contain only an `ObjectHandle`; the plugin instance
pointer remains in the interpreter's native-instance table. A receiver method
gets its own instance directly. To resolve a native object passed as an
ordinary argument, the plugin calls `OexNativeObject_BorrowInstance` with the
active call and expected class token. The host verifies an exact registered
class and a live scalar object before returning the instance. The borrow lasts
only for the callback.

`OexNativeObject_Create` performs the reverse operation for factories and
methods that allocate objects. On success it consumes the plugin instance and
returns an owned `OexValue`; the runtime transaction installs the ordinary
`ObjectHandle` and native-instance-table entry only if the callback succeeds.
Failure leaves ownership with the plugin. If a later callback failure rolls
back a successful creation, OpenMat calls the registered destructor exactly
once.

Each `OexNativePropertyDefinition` may provide a getter, a setter, or both. A
getter is invoked with zero inputs and one requested output; a setter is invoked
with the assigned value as its single input and zero requested outputs. The
receiver is always passed separately as the native instance pointer. A missing
getter makes the property write-only, and a missing setter makes it read-only.
Property callbacks use the same `OexCall` ownership, error, cancellation, and
native-object transaction rules as functions and methods.

## Dense arrays and zero-copy mutation

All OEX dense arrays are contiguous, column-major, and have a canonical rank
of at least two. `OexDenseView` aliases the actual OpenMat element buffer; the
host does not create logical or complex conversion buffers.

The core storage representations are also the C representations:

- logical: one `uint8_t`, restricted to 0 or 1;
- char: one exact `uint16_t` UTF-16 code unit;
- real integers and floats: their fixed-width C types;
- complex values: an interleaved `{ re, im }` C structure.

Mutable access requires ownership. The normal mutation sequence is
`OexCall_TakeInput`, then `OexDense_GetMutableView`. OpenMat performs any
required copy-on-write detachment while obtaining the mutable view, after
which the returned pointer directly aliases the taken value's storage. The
interpreter donates dead argument registers and packs into the owned builtin
call path, so a uniquely owned last-use array retains its exact buffer. If the
bytecode reads an argument again, or a host caller uses the borrowed invocation
API, normal COW cloning preserves language value semantics.

New output arrays use `OexDenseBuilder_CreateUninitialized`. It allocates the
final typed OpenMat buffer, exposes it through `OexMutableDenseView`, and does
not initialize the elements. The plugin must initialize every element before
`OexDenseBuilder_Commit`. Commit consumes the builder and returns an owned
value without copying the element buffer. Abort consumes the builder;
unfinished builders are automatically aborted at callback return.

Views remain valid until their value is released or consumed, the callback
returns, or another operation invalidates that value's storage. A plugin may
let worker threads read a view or write disjoint mutable elements while the
callback is active, but must join them before returning and must not
concurrently call APIs that mutate or retain the same value.

## Sparse arrays

`OexSparse_GetCscView` directly exposes current canonical logical, `double`,
and complex-`double` CSC storage. Offsets and row indices are zero-based
`uint64_t`; offsets have `columns + 1` entries and values have `storedCount`
entries.

`OexSparseTripletBuilder_CreateUninitialized` exposes writable row, column,
and value buffers. Commit consumes the initialized prefix, sorts it into CSC,
combines duplicate coordinates, removes explicit zeros, and returns the final
OpenMat sparse allocation.

For repeated assembly, `OexSparsePattern_Create` validates and owns canonical
offsets and row indices once. The persistent, reference-counted pattern may be
kept in a native object. Each `OexSparsePatternBuilder_CreateUninitialized`
then allocates only a writable value buffer. Commit preserves canonical order,
removes explicit zeros, and retains the pattern entry count as sparse reserve
metadata. Builders are call-scoped and auto-abort on callback return; patterns
must be explicitly released. These construction APIs currently accept only
logical, `double`, and complex-`double` values.

## Cancellation and failure boundary

The callback runs synchronously. `OexCancellation_IsRequested` performs a
thread-safe atomic read and can be polled by worker threads. Other handle and
call APIs are call-thread operations unless a future function explicitly says
otherwise.

OEX catches neither access violations nor foreign allocator bugs. A plugin
that keeps call-scoped pointers, commits uninitialized elements, writes invalid
logical bytes, races a mutable view, or unwinds across C has violated the
trusted ABI and may corrupt the kernel.
