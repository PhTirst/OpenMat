//! Heap-backed interpreter continuations.
//!
//! `ExecutionFrame` owns the m registers/locals and the futures below retain the
//! continuation around each call (including constructor cleanup, catch handlers,
//! property access, and finalization). `Stk::run` suspends the caller and pushes
//! the callee onto an explicit heap stack; `finish` drives only its top frame.
//! In particular, do NOT replace these calls with direct recursive `.await`:
//! that would reintroduce a native recursive `poll` chain.
//!
//! This is synchronous execution, not an async I/O runtime. Only host entry
//! points and synchronous native-to-m callbacks start a driver. No thread is
//! created and no plugin or kernel interface changes.

pub(super) use reblessive::Stk as FrameStack;

// Adapt the lending AsyncFnOnce bound to reblessive's FnOnce -> Future bound.
#[allow(clippy::redundant_closure)]
pub(super) fn run<R>(entry: impl AsyncFnOnce(&mut FrameStack) -> R) -> R {
    reblessive::Stack::new()
        .enter(|stack| entry(stack))
        .finish()
}
