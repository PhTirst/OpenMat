use std::collections::{BTreeSet, VecDeque};

/// Why the interpreter must inspect object reachability at the next language
/// boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SafepointReason {
    RootRemoval,
    ScopeExit,
    CommandBoundary,
    AllocationDebt,
    ExplicitDelete,
    ConstructionFailure,
}

/// Pending finalizer work in deterministic discovery order.
///
/// The queue deliberately has no clock or timer integration. Work may be
/// discovered while a destructor runs, so membership remains recorded until
/// each item is popped rather than being cleared at drain entry.
#[derive(Debug)]
pub(super) struct FinalizerQueue<T> {
    pending: VecDeque<(u64, T)>,
    members: BTreeSet<u64>,
}

impl<T> Default for FinalizerQueue<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
            members: BTreeSet::new(),
        }
    }
}

impl<T> FinalizerQueue<T> {
    pub(super) fn enqueue(&mut self, identity: u64, item: T) {
        if self.members.insert(identity) {
            self.pending.push_back((identity, item));
        }
    }

    pub(super) fn enqueue_all(
        &mut self,
        items: impl IntoIterator<Item = T>,
        identity: impl Fn(&T) -> u64,
    ) {
        for item in items {
            self.enqueue(identity(&item), item);
        }
    }

    pub(super) fn pop_front(&mut self) -> Option<T> {
        let (identity, item) = self.pending.pop_front()?;
        self.members.remove(&identity);
        Some(item)
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Scheduling state kept separately from the object collector.
///
/// Root removal is coalesced until the next bytecode boundary. Scope exit and
/// command completion are invoked directly because their callers already own
/// the corresponding language boundary.
#[derive(Debug, Default)]
pub(super) struct FinalizationSafepoints {
    root_removal_pending: bool,
    draining: bool,
}

impl FinalizationSafepoints {
    pub(super) const fn note_root_removal(&mut self) {
        self.root_removal_pending = true;
    }

    pub(super) fn take_root_removal(&mut self) -> bool {
        std::mem::take(&mut self.root_removal_pending)
    }

    pub(super) const fn root_removal_pending(&self) -> bool {
        self.root_removal_pending
    }

    pub(super) const fn is_draining(&self) -> bool {
        self.draining
    }

    pub(super) const fn begin_drain(&mut self) -> bool {
        if self.draining {
            return false;
        }
        self.draining = true;
        true
    }

    pub(super) const fn end_drain(&mut self) {
        self.draining = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{FinalizationSafepoints, FinalizerQueue};

    #[test]
    fn queue_preserves_discovery_order_and_coalesces_duplicates() {
        let mut queue = FinalizerQueue::default();
        queue.enqueue_all([3_u64, 1, 3, 2, 1], |item| *item);

        assert_eq!(queue.pop_front(), Some(3));
        assert_eq!(queue.pop_front(), Some(1));
        assert_eq!(queue.pop_front(), Some(2));
        assert_eq!(queue.pop_front(), None);
        assert!(queue.is_empty());
    }

    #[test]
    fn popped_item_can_be_queued_again_for_a_new_lifecycle_generation() {
        let mut queue = FinalizerQueue::default();
        queue.enqueue(7, 7_u64);
        assert_eq!(queue.pop_front(), Some(7));
        queue.enqueue(7, 7);
        assert_eq!(queue.pop_front(), Some(7));
    }

    #[test]
    fn root_removals_coalesce_and_nested_drains_are_rejected() {
        let mut safepoints = FinalizationSafepoints::default();
        safepoints.note_root_removal();
        safepoints.note_root_removal();
        assert!(safepoints.take_root_removal());
        assert!(!safepoints.take_root_removal());

        assert!(safepoints.begin_drain());
        assert!(!safepoints.begin_drain());
        safepoints.end_drain();
        assert!(safepoints.begin_drain());
    }
}
