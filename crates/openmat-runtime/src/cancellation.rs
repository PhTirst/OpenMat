use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// A shareable cooperative-cancellation flag.
///
/// The interpreter checks this flag between bytecode instructions and before
/// and after calls into built-ins. Native built-ins are expected to check the
/// token during long-running work as well.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a token in the running state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cooperative cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Clears a previous cancellation request before a new top-level run.
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::Release);
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Borrows the shared flag for crate-internal cooperative operations.
    pub(crate) fn atomic_flag(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }
}
