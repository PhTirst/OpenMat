# Heap-backed m call frames

The interpreter executes m calls without recursively entering the Rust VM loop.
`ExecutionFrame` still contains the instruction pointer, registers, locals,
captures, arguments, and scope information. Its owner is now a suspended,
heap-allocated execution state, not an active native call frame.

## Driver and continuations

`interpreter/frames.rs` supplies the synchronous driver and `FrameStack` context.
The driver uses [reblessive 0.4.3](https://docs.rs/reblessive/0.4.3/reblessive/struct.Stack.html)
(MIT, no enabled optional features or transitive dependencies). The runtime
crate continues to forbid unsafe code.

The allocator is a chunked heap stack: continuation allocations inside each
block carry a back-pointer, and blocks are linked when the stack grows. This
avoids a separate system allocation for every linked-list node while keeping
live frame addresses stable and supporting last-in-first-out reclamation.

Internal `async fn` bodies encode continuations: which instruction resumes,
where outputs are written, and which cleanup must run on return or failure.
This is not async I/O, threading, or an async public/plugin API.

At a potentially recursive call, `stack.run(|stack| ...).await` suspends the
caller and schedules the child on the explicit heap stack. The driver polls
only the top continuation. Returning an error resumes the parent's existing
error-handling path; it does not replay the instruction or its side effects.
Do not substitute direct recursive `.await` calls: recursive future polling
would bring back native-stack growth.

The same mechanism covers ordinary calls, function handles, class constructors,
methods, accessors, operator dispatch, eval, events, and finalizers. Existing
language stack traces and GC-root snapshots remain separate and are pushed and
popped with the corresponding execution frame.

## Synchronous host boundaries

`execute_entry` and the existing host hooks remain synchronous. Built-ins and
OEX callbacks also retain their existing synchronous contracts. A native-to-m
callback enters a nested heap driver, so its m descendants use heap frames,
but the native caller itself necessarily remains on the native stack.

`maximum_call_depth` is 2048 by default and counts active **m frames**,
not internal continuation tasks. It can be raised independently of the native
stack size. Repeated synchronous native-to-m reentry is additionally capped at
16 (or the lower configured limit), returning `CallDepthExceeded`. This cap
does not limit a pure m call chain to 16 frames. Arbitrary recursion entirely
inside plugin code is still the plugin's responsibility.

The UI presentation helper's synchronous property/deletion bridges remain
available without changing that concurrently developed module's interface.
New interpreter-internal call paths should receive `&mut FrameStack` and use
the existing driver rather than create another one.

## Verification

```text
cargo build -p openmat-oex -p openmat-server
cargo test -p openmat-runtime -p openmat-kernel
cargo clippy -p openmat-runtime --all-targets --no-deps -- -D warnings
```

Use `--target-dir` to avoid replacing a running development server.
Use the same target directory for all commands: the kernel's C-plugin fixture
expects the built `openmat_oex.dll` beside the test profile directory.
`tests/heap_frames.rs` explicitly runs the VM in a 1 MiB thread: exactly 2048
active m frames under the default budget, mutual recursion and multiple
outputs, deep class construction and accessors, error traces and catch,
cancellation, native callback reentry, and
recovery of the same session after failures. A native probe also checks that
its stack address does not drift with m call depth. No linker `/STACK` increase
or `RUST_MIN_STACK` override is required for these tests.
