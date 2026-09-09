# OEX Rust SDK

The official Rust-facing bindings to `include/openmat/oex.h` (OEX ABI 1.3).
Rust 1.90 or newer, edition 2024. The currently supported native loader target
is Windows x64. SDK crates depend only on the standard library and each other.

| Package | Purpose |
| --- | --- |
| `oex-sys` | Raw C types, constants, callbacks and all 73 bridge functions |
| `oex` | Typed values, borrowed views, builders, error handling and plugin registration |
| `oex-rust-example` | Independently built `cdylib` example |

This is a separate Cargo workspace. It can be built and consumed without
linking the host's Rust `openmat-oex`, array, value or runtime crates into a
plugin. The host crate remains the implementation of the C bridge. The SDK
version and OEX ABI version are separate from kernel/client protocol versions.

## Write a plugin

In your plugin's `Cargo.toml`:

```toml
[package]
name = "my-oex-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
oex = { path = "C:/work/OpenMat/sdk/rust/oex" }
```

In `src/lib.rs`:

```rust
use oex::{Call, Result};

fn scale(call: &mut Call<'_>) -> Result<()> {
    let factor = call.input(1)?.as_f64()?;
    let mut value = call.take_input(0)?;
    for x in value.array_mut::<f64>()?.as_mut_slice() {
        *x *= factor;
    }
    call.set_output(0, value)
}

oex::export_plugin! {
    name: "My Rust plugin", version: "1.0.0",
    functions: [oex::function!("my_scale", 2..=2, 1..=1, scale)],
    classes: [],
}
```

`function!` takes the language name, input range, output range and a Rust
function. `export_plugin!` exports `OexPlugin_Init` and constructs immutable,
process-lifetime metadata. No raw pointers or `unsafe` are needed in this example.

On Windows, `oex-sys` uses Rust's `raw-dylib` linking to import
`openmat_oex.dll`. Building a plugin does not require an import library or
building the host. Running it requires the compatible bridge DLL beside the
plugin or in the distribution's configured bridge location. On other targets
the raw declarations use ordinary `openmat_oex` dynamic-library linking; native
loading there is not currently provided by OpenMat.

From the repository root:

```powershell
cargo build -p openmat-oex --locked
cargo build --manifest-path sdk/rust/Cargo.toml -p oex-rust-example --locked
Copy-Item target/debug/openmat_oex.dll sdk/rust/target/debug/
```

Load `sdk/rust/target/debug/oex_rust_example.dll` with the application's existing
`--oex-plugin` option. The [complete example](examples/rust-plugin/src/lib.rs)
also demonstrates sparse arrays, language callbacks, a native class and properties.

## Values, types and lifetimes

- `Value<'call>` owns one handle. Moving transfers it, `Drop` releases it, and
  `try_clone()` returns `Result<Value>` because the C retain operation can fail.
- `ValueRef<'value, 'call>` borrows an input slot or an owned value. Its array
  views live for `'value`; `try_to_owned()` produces an independent owned handle.
- `Call::input(&self, index)` borrows a slot; `take_input(&mut self, index)`
  requires the input borrows to have ended.
- `array::<T>()` returns a read-only view. `array_mut::<T>()` requires `&mut Value`
  and obtains the host's COW detachment before returning a mutable slice.
- `set_output` consumes a value on success and releases it on failure. Retain
  first if the plugin needs its own copy after setting an output.
- Values, views and builders cannot escape their callback. The C ABI permits
  retaining owned handles longer, but this safe SDK deliberately bounds values
  to the callback that provides their host context. Raw interoperability is
  available through `sys`, `as_raw`, `into_raw` and unsafe `Value::from_raw`.

`info()`, `kind()`, `data_type()`, `is_dense::<T>()`, `is_scalar::<T>()` and
`is_sparse::<T>()` distinguish the value category from its storage element type.
Unknown future enum codes are preserved. A failed type match returns an error
before creating a typed slice.

All 22 C element types have typed dense access: integers, floats, `Logical`,
`Char16` and `Complex<T>`. `Logical` keeps its byte private so safe code can only
write 0 or 1. `Char16` preserves exact UTF-16 code units, including surrogates.
`Element` is sealed to prevent incompatible user types from aliasing host memory.
String, Cell, Struct and Table have typed constructors and content access below.
Object classes can be queried with `class_name(call)`; native state uses the
registered class token. Other categories can still be queried and forwarded.

Arrays are column-major. `get(&[row, column])` uses zero-based coordinates;
`as_slice()` exposes the flat storage without conversion. Handles and builders
are neither `Send` nor `Sync`. Borrowed slices can be used by scoped worker
threads which join before returning. `Cancellation` explicitly supports
thread-safe polling for the duration of the callback.

## New arrays and sparse storage

Safe creation writes directly into the host's final buffer:

```rust,ignore
let value = call.array(&[2, 2], &[1.0, 2.0, 3.0, 4.0])?;
let zeros = call.zeros::<f64>(&[100, 100])?;
let generated = call.dense_builder::<f64>(&[100, 1])?
    .fill_with(|index| index as f64)?;
```

For advanced initialization, `uninit_slice()` returns `&mut [MaybeUninit<T>]`.
The consuming `unsafe assume_init()` commits the allocation and requires every
element to be initialized. Dropping a partially filled builder aborts it, also
when a fill closure panics. The SDK never creates `&mut [T]` over uninitialized
memory. `fill`, `fill_with` and `copy_from_slice` perform safe initialization
and commit in one operation.

`sparse_triplets::<T>(rows, columns, capacity)` provides `push`, `extend` and
`finish`. It tracks the initialized prefix, checks coordinates/capacity before
writing, and lets the host sort entries, combine duplicates and remove zeros.
Supported types are `Logical`, `f64` and `Complex<f64>`.

`sparse_pattern` creates persistent CSC topology; `sparse_values` creates a new
values builder for it. Patterns have `try_clone` and `Drop`, survive callbacks,
and can be kept in plugin thread-local storage. They remain neither `Send` nor
`Sync` because the current ABI does not grant cross-thread pattern operations.
The builder retains its topology, so dropping the original pattern is safe.

## Strings, cells, structs and tables

```rust,ignore
let names = call.strings_utf8(&[2, 1], &[Some("Ada"), None])?;
let scores = call.array(&[2, 1], &[90.0, 95.0])?;
let mut table = call.table(2, &[
    ("Name", names.as_value_ref()),
    ("Score", scores.as_value_ref()),
])?;
let mut column = table.table_variable(call, 0)?;
column.set_string_utf8(call, 1, "Grace")?;
table.set_table_variable(call, 0, column.as_value_ref())?;
```

`strings_utf8` accepts `Option<&str>`; `None` is missing, distinct from `Some("")`.
`string_utf8(index)` returns `Result<Option<String>>`. For exact UTF-16,
`strings_utf16` accepts `Option<&[u16]>` and `string_element` borrows code units
plus a missing flag. Unpaired surrogates are preserved by UTF-16 access; strict
UTF-8 conversion returns `ErrorKind::Encoding`. Embedded NUL is preserved.

`cell`, `structure` and `table` construct heterogeneous containers. Getters
return owned children with language copy semantics; setters borrow their source
and require `&mut Value`. Editing a child requires an explicit setter to write it
back. Handle objects retain their shared-instance semantics. Batch setters and
schema edits leave the target unchanged on failure. Indices are zero-based and
column-major, including struct element and field indices.

Table variables retain their full shape and must match the table row count in
their first dimension. `call.table(7, &[])` creates a seven-row, zero-variable
table. `table_row_names` returns `None` for absent names; an empty name list on a
zero-row table is `Some`. Find operations return `Result<Option<u64>>`.

See the [complete API and encoding guide](../../docs/implementation/oex-containers.md)
and the [tested Rust container examples](tests/plugin/src/lib.rs).

## Errors, callbacks and native objects

Use `?` for SDK failures and return `Error::new("Plugin:Identifier", "message")`
for plugin errors. `Error` implements `std::error::Error` and preserves the OEX
status category. No exceptions or Rust ABI types cross the module boundary.

Generated callbacks translate `Result<()>` and contain unwinding panics. A
panic produces `OpenMat:OEX:RustPanic`; the ordinary Rust panic hook still runs.
`panic = "abort"` aborts the process and cannot be intercepted. An opaque panic
payload is deliberately leaked on the panic path because its destructor could
panic again. The adapter does not change the process panic hook.

`Call::invoke` borrows its callable/arguments and returns `Vec<Value>`. An
original language error remains pending even if plugin code handles the Rust
`Err` or replaces it with another error. The adapter preserves the host error
instead of overwriting its identifier/message.

Use a unique `static NativeClass<T>` per class. `class!`, `method!` and
`NativeProperty::new` adapt ordinary Rust constructors/methods/properties. The
class's `create` transfers Rust state to the host; `borrow` validates its exact
type token and returns a `NativeObject<T>` retaining that state.

Native state requires `T: Send + 'static`. It is held in `Arc<Mutex<T>>` entirely
inside the plugin. `try_lock` checks exclusive access; the same object's
reentrant method calls fail instead of deadlocking or aliasing `&mut T`.
Methods receive `&mut T`; a panic poisons its mutex. Native destructors should
not panic; the C destructor adapter contains an unwind but has no error return
channel. No Rust allocation is deallocated by the host's allocator.

## Verification

Run the complete Windows verification from the repository root:

```powershell
./sdk/rust/verify.ps1
```

It builds the bridge and plugins, runs formatting and strict Clippy/rustdoc
checks, compares every public constant/structure layout/field offset/function
signature against a compiled C translation unit, runs compile-fail lifetime
tests, and loads both Rust DLLs into the current kernel. `CC` may select a
GCC/Clang-compatible C compiler for the ABI test; the default is `gcc`.

The small [test host](tests/host) alone depends on host crates. The verification
script builds it under `sdk/rust/target` so its standalone dependency graph does
not overwrite the root workspace's bridge artifacts. This keeps plugin dependencies
independent and permits full integration testing when the CLI has unrelated build
failures. The integration
script checks all dense types, COW, sparse canonicalization, pattern retention,
typed containers/encoding, errors/panics, callbacks, native properties/reentrancy
and destructor counts.
