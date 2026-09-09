# OpenMat low-level FFI

The `ffi` namespace calls C-compatible functions exported by DLLs without
parsing C headers. M code declares every type and the calling convention. The
implementation intentionally does not verify that a declaration matches the
real function: a wrong type, invalid pointer, use-after-free, or incorrect
function address may corrupt memory or terminate the kernel process.

The first implementation targets 64-bit Windows. `ffi.win64` is the native
calling convention. `ffi.cdecl` and `ffi.stdcall` are accepted aliases because
those spellings use the same calling convention on Windows x64; they do not
claim Win32 x86 support.

## DLL functions

An exported symbol is a normal callable object. Its signature is configured
through properties and is retained when the symbol is read from the library
again, including across REPL requests.

```matlab
kernel32 = ffi.Library("kernel32.dll");

kernel32.GetCurrentProcessId.argtypes = {};
kernel32.GetCurrentProcessId.restype = ffi.u32;
kernel32.GetCurrentProcessId.abi = ffi.win64;

pid = kernel32.GetCurrentProcessId();
address = kernel32.GetCurrentProcessId.address;
```

Symbols are resolved lazily and case-sensitively. A new symbol initially has
`argtypes = {}`, `restype = ffi.void`, and `abi = ffi.win64`; callers should set
all three explicitly.

## C types and structures

Primitive types are `ffi.i8`, `ffi.u8`, `ffi.i16`, `ffi.u16`, `ffi.i32`,
`ffi.u32`, `ffi.i64`, `ffi.u64`, `ffi.f32`, `ffi.f64`, and `ffi.void`.
`ffi.usize`/`ffi.isize`, `ffi.bool` (one byte), and `ffi.bool32` (four bytes)
are aliases. Integer results remain exact fixed-width M integer scalars; float
results are M doubles.

`ffi.Structure` receives a name, an `N x 2` cell array, and an optional packing
ceiling. The first column contains field names and the second contains FFI
types. With no third argument, fields use natural C alignment on Windows x64.
The value is stored in native memory, not in a Rust structure or an ordinary M
struct.

```matlab
SYSTEMTIME = ffi.Structure("SYSTEMTIME", {
    "year",         ffi.u16;
    "month",        ffi.u16;
    "dayOfWeek",    ffi.u16;
    "day",          ffi.u16;
    "hour",         ffi.u16;
    "minute",       ffi.u16;
    "second",       ffi.u16;
    "milliseconds", ffi.u16
});

now = SYSTEMTIME();       % zeroed native allocation
now.year = 2026;          % writes directly to the C field
n = now.year;             % reads directly from the C field

bytes = ffi.sizeof(SYSTEMTIME);
alignment = ffi.alignof(SYSTEMTIME);
address = ffi.addressof(now);
```

Nested structures and structures passed or returned by value use the same
declared layout. A positive power-of-two third argument implements the usual C
packing ceiling:

```matlab
PACKED = ffi.Structure("PACKED", {
    "tag",   ffi.u8;
    "value", ffi.u32
}, 1);

ffi.sizeof(PACKED)   % 5
PACKED.pack          % 1; natural layout reports 0
```

`ffi.Array(T, count)` is a fixed-size C array type. An array value owns native
memory; `at` and `set` use zero-based C element offsets and deliberately do not
check the declared length.

```matlab
WORDS = ffi.Array(ffi.u16, 3);
words = WORDS();
words.set(0, 11);
words.set(1, 17);
second = words.at(1);

PACKET = ffi.Structure("PACKET", {"values", WORDS});
packet = PACKET();
packet.values.set(2, 23);  % writes the array field in packet's native memory
```

`ffi.Union` has the same field-table and optional-packing arguments as
`ffi.Structure`. Every member starts at offset zero, so assigning one member
immediately changes the bytes observed through the others.

```matlab
NUMBER = ffi.Union("NUMBER", {
    "bits", ffi.u32;
    "real", ffi.f32
});
n = NUMBER();
n.bits = uint32(1065353216);  % n.real now reads 1.0 on IEEE-754 hosts
```

Bitfields and flexible-array members are not implemented. Declare their storage
manually or use pointer arithmetic when exact compiler-specific layout is
required.

## Pointers and raw memory

`ffi.Ptr(T)` constructs a pointer type. A structure value is accepted for a
pointer argument and contributes its address. Pointer objects expose the raw
address and a typed `contents` property; accessing it immediately reads or
writes the pointed-to process memory.

```matlab
kernel32.GetSystemTime.argtypes = {ffi.Ptr(SYSTEMTIME)};
kernel32.GetSystemTime.restype = ffi.void;
kernel32.GetSystemTime.abi = ffi.win64;
kernel32.GetSystemTime(now);

p = ffi.cast(ffi.addressof(now), ffi.Ptr(SYSTEMTIME));
year = p.contents.year;
p.contents.month = 8;

next = p.offset(1);       % C-style element offset, not byte offset
raw = ffi.alloc(SYSTEMTIME, 4);
```

`ffi.cast(address, pointerType)` accepts an exact integer address or another
native object. `ffi.alloc(T, count)` owns a zeroed native block and returns a
typed pointer. `pointer.offset(count)` retains that allocation when possible.
Raw pointers returned by a DLL or made with `ffi.cast` do not acquire ownership
of external memory.

There are deliberately no pointer bounds, lifetime, null, or access-right
checks. `pointer.contents` on address zero is not converted to a friendly M
error.

## Character buffers

String helpers are explicit allocation/decoding operations, not automatic
argument conversion:

```matlab
utf8 = ffi.cstr("OpenMat");      % owned, NUL-terminated UTF-8; ffi.Ptr(ffi.u8)
wide = ffi.wstr("OpenMat");     % owned, NUL-terminated UTF-16; ffi.Ptr(ffi.u16)

text1 = ffi.read_cstr(utf8);
text2 = ffi.read_wstr(wide);
text3 = ffi.read_cstr(address, 64); % optional maximum code-unit count
```

The read operations scan arbitrary process memory. A bad address, missing
terminator without a maximum, or undersized buffer can terminate the kernel.
Invalid UTF-8/UTF-16 produces an M runtime error after the bytes have been read.

## Function pointers

Function-pointer types include their complete signature:

```matlab
GETPID = ffi.FunctionType(ffi.u32, {}, ffi.win64);
getPid = ffi.cast(kernel32.GetCurrentProcessId.address, GETPID);
pid = getPid();
```

The arguments are `restype`, an `argtypes` cell array, and an optional ABI
(default `ffi.win64`). A function pointer read from a structure field becomes
the same callable `ffi.Function` object. Function values can also be passed to
arguments declared with a function-pointer type.

Windows error state can be captured immediately after selected calls:

```matlab
kernel32.CreateFileW.use_last_error = true;
handle = kernel32.CreateFileW(...);
code = kernel32.CreateFileW.last_error;
```

When `use_last_error` is false, `last_error` is zero and no capture occurs.
This is Windows `GetLastError`, not `errno`; it does not interpret whether the
native function considers its result successful.

## Current boundary

This phase supports DLL loading, primitive values, naturally aligned and packed
structures, unions, fixed C arrays, aggregate fields, pointers and dereference,
owned raw and string allocations, fixed-signature calls, aggregates by value,
function-pointer calls, and optional `GetLastError` capture on Windows x64.

Callbacks from C into M, variadic calls, bitfields, flexible arrays, `errno`,
and automatic per-argument string encoding are intentionally deferred. None of
these omissions are replaced by header parsing or a high-level safety wrapper.
