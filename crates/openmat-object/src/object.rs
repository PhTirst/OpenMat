use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
};

use openmat_value::ObjectHandle;

use crate::{
    ClassId, ClassRegistry, ClassSemantics, ObjectError, ObjectId, ObjectResult,
    PropertyDescriptor, PropertyKey,
};

/// Lifecycle of an object's allocated property storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstructionState {
    Allocated,
    Constructing,
    Constructed,
    Failed,
}

/// Shared lifecycle state for one handle identity.
///
/// `Finalizing` is already invalid to language code. The record remains in the
/// store only so trusted runtime finalization can read its stored properties.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandleState {
    Alive,
    Finalizing,
    Invalid,
}

/// An unforgeable store-issued capability for one finalizing handle.
///
/// The runtime uses this value to select and invoke the destructor. It does not
/// prescribe user-code execution or ordering beyond the order supplied by a
/// [`GcPlan`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalizationCandidate {
    id: ObjectId,
    class_id: ClassId,
    construction_state: ConstructionState,
}

impl FinalizationCandidate {
    #[must_use]
    pub const fn id(&self) -> ObjectId {
        self.id
    }

    #[must_use]
    pub const fn class_id(&self) -> ClassId {
        self.class_id
    }

    #[must_use]
    pub const fn semantics(&self) -> ClassSemantics {
        ClassSemantics::Handle
    }

    /// Reports whether finalization follows complete or partial construction.
    #[must_use]
    pub const fn construction_state(&self) -> ConstructionState {
        self.construction_state
    }
}

/// First phase of a stop-the-world collection.
///
/// All listed records have been made language-invalid, but none of the
/// unreachable slots in this plan have been reclaimed. The runtime may execute
/// destructors for `candidates()` and must then pass the plan to
/// [`ObjectStore::finish_collection`].
#[derive(Debug, Eq, PartialEq)]
pub struct GcPlan {
    candidates: Vec<FinalizationCandidate>,
    unreachable: Vec<ObjectId>,
    planned_objects: u64,
    planned_slots: u64,
}

impl GcPlan {
    /// Finalizer candidates in deterministic descending `ObjectId` order.
    ///
    /// This is a scheduling primitive, not a general language rule for
    /// arbitrary object graphs.
    #[must_use]
    pub fn candidates(&self) -> &[FinalizationCandidate] {
        &self.candidates
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unreachable.is_empty()
    }
}

/// Read-only statistics for one object store's tracing collector.
///
/// Allocation debt is measured in object-slot units: one unit for the record
/// plus one for each stored property. This deliberately stable proxy avoids
/// pretending that shared copy-on-write `Value` buffers have a unique byte
/// owner while still scaling collection pressure with object complexity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GcMetrics {
    pub live_objects: u64,
    pub live_slots: u64,
    pub allocation_debt: u64,
    pub collections: u64,
    pub total_allocated_objects: u64,
    pub total_reclaimed_objects: u64,
    pub total_reclaimed_slots: u64,
    pub last_reclaimed_objects: u64,
    pub last_reclaimed_slots: u64,
}

/// Result of one completed stop-the-world mark-sweep collection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GcCollection {
    pub reclaimed_objects: u64,
    pub reclaimed_slots: u64,
    pub remaining_objects: u64,
    pub remaining_slots: u64,
}

/// A language-level reference to object state owned by an [`ObjectStore`].
///
/// This type intentionally does not implement `Clone` or `Copy`. Language
/// assignment must call [`ObjectStore::assignment_copy`] so value objects are
/// copied while handle objects alias their existing identity.
#[derive(Debug, Eq, PartialEq)]
pub struct ObjectRef {
    id: ObjectId,
    class_id: ClassId,
    semantics: ClassSemantics,
}

impl ObjectRef {
    const fn new(id: ObjectId, class_id: ClassId, semantics: ClassSemantics) -> Self {
        Self {
            id,
            class_id,
            semantics,
        }
    }

    #[must_use]
    pub const fn id(&self) -> ObjectId {
        self.id
    }

    #[must_use]
    pub const fn class_id(&self) -> ClassId {
        self.class_id
    }

    #[must_use]
    pub const fn semantics(&self) -> ClassSemantics {
        self.semantics
    }

    /// Persistent identity equality. Value assignment normally makes this
    /// false; handle assignment makes it true.
    #[must_use]
    pub fn aliases(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Debug)]
struct ObjectRecord<S> {
    class_id: ClassId,
    semantics: ClassSemantics,
    state: ConstructionState,
    handle_state: HandleState,
    slots: BTreeMap<PropertyKey, S>,
}

/// Owns object property state while keeping the slot type independent of the
/// dynamic runtime `Value` implementation.
#[derive(Debug)]
pub struct ObjectStore<S> {
    objects: BTreeMap<ObjectId, ObjectRecord<S>>,
    invalid_handles: BTreeMap<ObjectId, ClassId>,
    next_id: u64,
    metrics: GcMetrics,
}

impl<S> Default for ObjectStore<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> ObjectStore<S> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            objects: BTreeMap::new(),
            invalid_handles: BTreeMap::new(),
            next_id: 1,
            metrics: GcMetrics {
                live_objects: 0,
                live_slots: 0,
                allocation_debt: 0,
                collections: 0,
                total_allocated_objects: 0,
                total_reclaimed_objects: 0,
                total_reclaimed_slots: 0,
                last_reclaimed_objects: 0,
                last_reclaimed_slots: 0,
            },
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Returns the identifier the next successful ordinary allocation will use.
    ///
    /// Runtime transactions may use this as a prediction, but must still perform the checked
    /// allocation before publishing the corresponding object value.
    #[must_use]
    pub const fn next_allocation_id(&self) -> ObjectId {
        ObjectId::new(self.next_id)
    }

    /// Returns a stable snapshot of allocation and collection counters.
    #[must_use]
    pub const fn gc_metrics(&self) -> GcMetrics {
        self.metrics
    }

    /// Returns whether allocation pressure has reached a caller-selected
    /// object-slot-unit threshold.
    #[must_use]
    pub const fn collection_due(&self, allocation_threshold: u64) -> bool {
        self.metrics.allocation_debt >= allocation_threshold
    }

    /// # Errors
    ///
    /// Returns an error when the reference is absent or does not match its record.
    pub fn state(&self, object: &ObjectRef) -> ObjectResult<ConstructionState> {
        Ok(self.record(object)?.state)
    }

    /// Returns the shared lifecycle state of a handle identity, including an
    /// `Invalid` tombstone after its property slots have been reclaimed.
    ///
    /// # Errors
    ///
    /// Returns an error for a value object or a reference from another store.
    pub fn handle_state(&self, object: &ObjectRef) -> ObjectResult<HandleState> {
        if object.semantics != ClassSemantics::Handle {
            return Err(ObjectError::ObjectSemanticsMismatch {
                id: object.id,
                expected: ClassSemantics::Handle,
                actual: object.semantics,
            });
        }
        if let Some(record) = self.objects.get(&object.id) {
            Self::validate_reference(object, record)?;
            return Ok(record.handle_state);
        }
        match self.invalid_handles.get(&object.id) {
            Some(class_id) if *class_id == object.class_id => Ok(HandleState::Invalid),
            Some(_) => Err(ObjectError::ObjectReferenceMismatch { id: object.id }),
            None => Err(ObjectError::UnknownObject { id: object.id }),
        }
    }

    /// Atomically makes one handle identity invalid to all language aliases.
    ///
    /// `requires_destructor` is supplied by runtime method lookup. When true,
    /// the returned capability preserves controlled property access until
    /// [`Self::finish_finalization`] is called. A handle without a destructor
    /// is reclaimed immediately. Repeated starts return `Ok(None)` and never
    /// enqueue the destructor twice.
    ///
    /// Partial and failed construction states are accepted so constructor
    /// unwind can use the same protocol.
    ///
    /// # Errors
    ///
    /// Returns an error for a value object or a mismatched/unknown reference.
    pub fn begin_finalization(
        &mut self,
        object: &ObjectRef,
        requires_destructor: bool,
    ) -> ObjectResult<Option<FinalizationCandidate>> {
        if object.semantics != ClassSemantics::Handle {
            return Err(ObjectError::ObjectSemanticsMismatch {
                id: object.id,
                expected: ClassSemantics::Handle,
                actual: object.semantics,
            });
        }
        if let Some(class_id) = self.invalid_handles.get(&object.id) {
            return if *class_id == object.class_id {
                Ok(None)
            } else {
                Err(ObjectError::ObjectReferenceMismatch { id: object.id })
            };
        }

        let candidate = {
            let record = self.record_raw_mut(object)?;
            if record.handle_state != HandleState::Alive {
                return Ok(None);
            }
            record.handle_state = HandleState::Finalizing;
            FinalizationCandidate {
                id: object.id,
                class_id: record.class_id,
                construction_state: record.state,
            }
        };
        if requires_destructor {
            Ok(Some(candidate))
        } else {
            if let Some(record) = self.remove_record(object.id) {
                self.record_reclamation(&record);
            }
            Ok(None)
        }
    }

    /// Reads a stored property while a destructor is running.
    ///
    /// This is deliberately read-only and requires a store-issued capability;
    /// ordinary [`Self::slot`] access rejects the same identity as invalid.
    ///
    /// # Errors
    ///
    /// Returns an error if the capability does not identify the matching
    /// `Finalizing` record or if the property slot is absent.
    pub fn finalizer_slot(
        &self,
        candidate: &FinalizationCandidate,
        key: &PropertyKey,
    ) -> ObjectResult<&S> {
        let record = self.finalizing_record(candidate)?;
        record
            .slots
            .get(key)
            .ok_or_else(|| ObjectError::PropertySlotNotFound {
                id: candidate.id,
                property: key.name().to_owned(),
            })
    }

    /// Reclaims a record after the runtime has attempted its destructor.
    ///
    /// The operation is idempotent for an already-completed matching
    /// capability, which lets error containment still complete invalidation.
    ///
    /// # Errors
    ///
    /// Returns an error when the capability does not match the stored identity
    /// or the handle has not entered `Finalizing`.
    pub fn finish_finalization(&mut self, candidate: &FinalizationCandidate) -> ObjectResult<bool> {
        if let Some(class_id) = self.invalid_handles.get(&candidate.id) {
            return if *class_id == candidate.class_id {
                Ok(false)
            } else {
                Err(ObjectError::FinalizationCandidateMismatch { id: candidate.id })
            };
        }
        self.finalizing_record(candidate)?;
        if let Some(record) = self.remove_record(candidate.id) {
            self.record_reclamation(&record);
        }
        Ok(true)
    }

    /// # Errors
    ///
    /// Returns an error for an unknown reference or a state other than `Allocated`.
    pub fn begin_construction(&mut self, object: &ObjectRef) -> ObjectResult<()> {
        self.transition(
            object,
            ConstructionState::Allocated,
            ConstructionState::Constructing,
        )
    }

    /// # Errors
    ///
    /// Returns an error for an unknown reference or a state other than `Constructing`.
    pub fn finish_construction(&mut self, object: &ObjectRef) -> ObjectResult<()> {
        self.transition(
            object,
            ConstructionState::Constructing,
            ConstructionState::Constructed,
        )
    }

    /// # Errors
    ///
    /// Returns an error for an unknown reference or a terminal construction state.
    pub fn fail_construction(&mut self, object: &ObjectRef) -> ObjectResult<()> {
        let record = self.record_mut(object)?;
        match record.state {
            ConstructionState::Allocated | ConstructionState::Constructing => {
                record.state = ConstructionState::Failed;
                Ok(())
            }
            from => Err(ObjectError::InvalidConstructionTransition {
                id: object.id,
                from,
                to: ConstructionState::Failed,
            }),
        }
    }

    /// Removes storage for an object whose construction did not complete.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or mismatched reference, or when asked
    /// to discard an already-constructed language object.
    pub fn discard_incomplete(&mut self, object: &ObjectRef) -> ObjectResult<()> {
        let record = self.record(object)?;
        if record.state == ConstructionState::Constructed {
            return Err(ObjectError::ObjectNotConstructed {
                id: object.id,
                state: record.state,
            });
        }
        self.remove_record(object.id);
        Ok(())
    }

    /// Removes a freshly created assignment copy of a constructed value object.
    ///
    /// This is a narrow rollback primitive for callers that are assembling a
    /// larger value, such as a homogeneous object array. Handle records cannot
    /// be removed through this operation because an assignment copy of a handle
    /// is an alias to pre-existing identity.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown/mismatched reference, a handle object,
    /// or a record that is not fully constructed.
    pub fn discard_value_assignment_copy(&mut self, object: &ObjectRef) -> ObjectResult<()> {
        let record = self.record(object)?;
        if record.semantics != ClassSemantics::Value {
            return Err(ObjectError::ObjectSemanticsMismatch {
                id: object.id,
                expected: ClassSemantics::Value,
                actual: record.semantics,
            });
        }
        if record.state != ConstructionState::Constructed {
            return Err(ObjectError::ObjectNotConstructed {
                id: object.id,
                state: record.state,
            });
        }
        self.remove_record(object.id);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when the object or requested stored-property slot is absent.
    pub fn slot(&self, object: &ObjectRef, key: &PropertyKey) -> ObjectResult<&S> {
        let record = self.record(object)?;
        record
            .slots
            .get(key)
            .ok_or_else(|| ObjectError::PropertySlotNotFound {
                id: object.id,
                property: key.name().to_owned(),
            })
    }

    /// # Errors
    ///
    /// Returns an error when the object or requested stored-property slot is absent.
    pub fn slot_mut(&mut self, object: &ObjectRef, key: &PropertyKey) -> ObjectResult<&mut S> {
        let object_id = object.id;
        let record = self.record_mut(object)?;
        record
            .slots
            .get_mut(key)
            .ok_or_else(|| ObjectError::PropertySlotNotFound {
                id: object_id,
                property: key.name().to_owned(),
            })
    }

    fn transition(
        &mut self,
        object: &ObjectRef,
        expected: ConstructionState,
        next: ConstructionState,
    ) -> ObjectResult<()> {
        let record = self.record_mut(object)?;
        if record.state != expected {
            return Err(ObjectError::InvalidConstructionTransition {
                id: object.id,
                from: record.state,
                to: next,
            });
        }
        record.state = next;
        Ok(())
    }

    fn record(&self, object: &ObjectRef) -> ObjectResult<&ObjectRecord<S>> {
        let record = self.record_raw(object)?;
        if record.semantics == ClassSemantics::Handle && record.handle_state != HandleState::Alive {
            return Err(ObjectError::HandleNotAlive {
                id: object.id,
                state: record.handle_state,
            });
        }
        Ok(record)
    }

    fn record_raw(&self, object: &ObjectRef) -> ObjectResult<&ObjectRecord<S>> {
        let Some(record) = self.objects.get(&object.id) else {
            return match self.invalid_handles.get(&object.id) {
                Some(class_id)
                    if *class_id == object.class_id
                        && object.semantics == ClassSemantics::Handle =>
                {
                    Err(ObjectError::HandleNotAlive {
                        id: object.id,
                        state: HandleState::Invalid,
                    })
                }
                Some(_) => Err(ObjectError::ObjectReferenceMismatch { id: object.id }),
                None => Err(ObjectError::UnknownObject { id: object.id }),
            };
        };
        Self::validate_reference(object, record)?;
        Ok(record)
    }

    fn record_mut(&mut self, object: &ObjectRef) -> ObjectResult<&mut ObjectRecord<S>> {
        let record = self.record_raw_mut(object)?;
        if record.semantics == ClassSemantics::Handle && record.handle_state != HandleState::Alive {
            return Err(ObjectError::HandleNotAlive {
                id: object.id,
                state: record.handle_state,
            });
        }
        Ok(record)
    }

    fn record_raw_mut(&mut self, object: &ObjectRef) -> ObjectResult<&mut ObjectRecord<S>> {
        if !self.objects.contains_key(&object.id) {
            return match self.invalid_handles.get(&object.id) {
                Some(class_id)
                    if *class_id == object.class_id
                        && object.semantics == ClassSemantics::Handle =>
                {
                    Err(ObjectError::HandleNotAlive {
                        id: object.id,
                        state: HandleState::Invalid,
                    })
                }
                Some(_) => Err(ObjectError::ObjectReferenceMismatch { id: object.id }),
                None => Err(ObjectError::UnknownObject { id: object.id }),
            };
        }
        let Some(record) = self.objects.get_mut(&object.id) else {
            return Err(ObjectError::UnknownObject { id: object.id });
        };
        Self::validate_reference(object, record)?;
        Ok(record)
    }

    fn finalizing_record(
        &self,
        candidate: &FinalizationCandidate,
    ) -> ObjectResult<&ObjectRecord<S>> {
        let Some(record) = self.objects.get(&candidate.id) else {
            return if self.invalid_handles.get(&candidate.id) == Some(&candidate.class_id) {
                Err(ObjectError::HandleNotAlive {
                    id: candidate.id,
                    state: HandleState::Invalid,
                })
            } else {
                Err(ObjectError::FinalizationCandidateMismatch { id: candidate.id })
            };
        };
        if record.class_id != candidate.class_id
            || record.semantics != ClassSemantics::Handle
            || record.state != candidate.construction_state
            || record.handle_state != HandleState::Finalizing
        {
            return Err(ObjectError::FinalizationCandidateMismatch { id: candidate.id });
        }
        Ok(record)
    }

    fn validate_reference(object: &ObjectRef, record: &ObjectRecord<S>) -> ObjectResult<()> {
        if object.class_id != record.class_id || object.semantics != record.semantics {
            return Err(ObjectError::ObjectReferenceMismatch { id: object.id });
        }
        Ok(())
    }

    fn reserve_id(&self) -> ObjectResult<(ObjectId, u64)> {
        let id = ObjectId::new(self.next_id);
        let next = self
            .next_id
            .checked_add(1)
            .ok_or(ObjectError::IdExhausted { kind: "object" })?;
        Ok((id, next))
    }

    fn record_allocation(&mut self, slot_count: usize) {
        let slots = u64::try_from(slot_count).unwrap_or(u64::MAX);
        let debt = slots.saturating_add(1);
        self.metrics.live_objects = self.metrics.live_objects.saturating_add(1);
        self.metrics.live_slots = self.metrics.live_slots.saturating_add(slots);
        self.metrics.allocation_debt = self.metrics.allocation_debt.saturating_add(debt);
        self.metrics.total_allocated_objects =
            self.metrics.total_allocated_objects.saturating_add(1);
    }

    fn remove_record(&mut self, id: ObjectId) -> Option<ObjectRecord<S>> {
        let mut record = self.objects.remove(&id)?;
        if record.semantics == ClassSemantics::Handle {
            if record.handle_state == HandleState::Alive {
                record.handle_state = HandleState::Finalizing;
            }
            self.invalid_handles.insert(id, record.class_id);
            record.handle_state = HandleState::Invalid;
        }
        let slots = u64::try_from(record.slots.len()).unwrap_or(u64::MAX);
        self.metrics.live_objects = self.metrics.live_objects.saturating_sub(1);
        self.metrics.live_slots = self.metrics.live_slots.saturating_sub(slots);
        Some(record)
    }

    fn record_reclamation(&mut self, record: &ObjectRecord<S>) {
        let slots = u64::try_from(record.slots.len()).unwrap_or(u64::MAX);
        self.metrics.total_reclaimed_objects =
            self.metrics.total_reclaimed_objects.saturating_add(1);
        self.metrics.total_reclaimed_slots =
            self.metrics.total_reclaimed_slots.saturating_add(slots);
    }

    /// Begins a non-moving, exact two-phase collection.
    ///
    /// `roots` are store-local object handles. `trace_slot` must report every
    /// object handle reachable through one stored slot. `needs_finalization`
    /// performs runtime-owned destructor lookup for unreachable handle classes.
    /// Unknown handles are ignored. No user code is executed here.
    ///
    /// Every unreachable handle is made invalid to language access before this
    /// method returns. Its slots, and all unreachable value-object slots, stay
    /// allocated until [`Self::finish_collection`] consumes the returned plan.
    /// Finalizer candidates use deterministic descending `ObjectId` order.
    #[must_use]
    pub fn begin_collection_with<I, F, P>(
        &mut self,
        roots: I,
        mut trace_slot: F,
        mut needs_finalization: P,
    ) -> GcPlan
    where
        I: IntoIterator<Item = ObjectHandle>,
        F: FnMut(&S, &mut dyn FnMut(ObjectHandle)),
        P: FnMut(ClassId) -> bool,
    {
        match self.try_begin_collection_with(
            roots,
            |slot, visit| {
                trace_slot(slot, visit);
                Ok::<(), Infallible>(())
            },
            |class_id| Ok::<bool, Infallible>(needs_finalization(class_id)),
        ) {
            Ok(plan) => plan,
            Err(error) => match error {},
        }
    }

    /// Fallible form of [`Self::begin_collection_with`].
    ///
    /// Runtime cancellation or checked graph-expansion failures can be returned
    /// from either callback. Callback failure leaves every lifecycle unchanged;
    /// invalidation is committed only after traversal and destructor lookup
    /// have both completed successfully.
    ///
    /// # Errors
    ///
    /// Returns the first error reported by `trace_slot` or
    /// `needs_finalization`.
    pub fn try_begin_collection_with<I, F, P, E>(
        &mut self,
        roots: I,
        mut trace_slot: F,
        mut needs_finalization: P,
    ) -> Result<GcPlan, E>
    where
        I: IntoIterator<Item = ObjectHandle>,
        F: FnMut(&S, &mut dyn FnMut(ObjectHandle)) -> Result<(), E>,
        P: FnMut(ClassId) -> Result<bool, E>,
    {
        let mut pending = roots
            .into_iter()
            .map(|handle| ObjectId::new(handle.identifier()))
            .collect::<Vec<_>>();
        let mut marked = BTreeSet::new();

        while let Some(id) = pending.pop() {
            if marked.contains(&id) {
                continue;
            }
            let Some(record) = self.objects.get(&id) else {
                continue;
            };
            if record.semantics == ClassSemantics::Handle
                && record.handle_state != HandleState::Alive
            {
                continue;
            }
            marked.insert(id);

            let mut outgoing = Vec::new();
            for slot in record.slots.values() {
                trace_slot(slot, &mut |handle| outgoing.push(handle))?;
            }
            pending.extend(
                outgoing
                    .into_iter()
                    .map(|handle| ObjectId::new(handle.identifier())),
            );
        }

        let mut unreachable = Vec::new();
        let mut candidates = Vec::new();
        for (id, record) in self.objects.iter().rev() {
            if marked.contains(id)
                || (record.semantics == ClassSemantics::Handle
                    && record.handle_state != HandleState::Alive)
            {
                continue;
            }
            let needs_destructor =
                record.semantics == ClassSemantics::Handle && needs_finalization(record.class_id)?;
            unreachable.push(*id);
            if needs_destructor {
                candidates.push(FinalizationCandidate {
                    id: *id,
                    class_id: record.class_id,
                    construction_state: record.state,
                });
            }
        }

        for id in &unreachable {
            let Some(record) = self.objects.get_mut(id) else {
                continue;
            };
            if record.semantics == ClassSemantics::Handle {
                record.handle_state = HandleState::Finalizing;
            }
        }

        let planned_objects = u64::try_from(unreachable.len()).unwrap_or(u64::MAX);
        let planned_slots = unreachable.iter().fold(0_u64, |slots, id| {
            slots.saturating_add(self.objects.get(id).map_or(0, |record| {
                u64::try_from(record.slots.len()).unwrap_or(u64::MAX)
            }))
        });
        Ok(GcPlan {
            candidates,
            unreachable,
            planned_objects,
            planned_slots,
        })
    }

    /// Completes a prepared collection after the runtime has attempted every
    /// requested destructor.
    ///
    /// This consumes the plan and reclaims all unreachable slots together, so
    /// destructor state is not discarded during candidate discovery.
    #[must_use]
    pub fn finish_collection(&mut self, plan: GcPlan) -> GcCollection {
        let reclaimed_objects = plan.planned_objects;
        let reclaimed_slots = plan.planned_slots;
        for id in plan.unreachable {
            let Some(record) = self.remove_record(id) else {
                continue;
            };
            self.record_reclamation(&record);
        }

        self.metrics.allocation_debt = 0;
        self.metrics.collections = self.metrics.collections.saturating_add(1);
        self.metrics.last_reclaimed_objects = reclaimed_objects;
        self.metrics.last_reclaimed_slots = reclaimed_slots;

        GcCollection {
            reclaimed_objects,
            reclaimed_slots,
            remaining_objects: self.metrics.live_objects,
            remaining_slots: self.metrics.live_slots,
        }
    }

    /// Runs a complete collection for classes that require no destructor.
    ///
    /// Runtime finalizer integration should instead use
    /// [`Self::begin_collection_with`] followed by [`Self::finish_collection`].
    pub fn collect_with<I, F>(&mut self, roots: I, trace_slot: F) -> GcCollection
    where
        I: IntoIterator<Item = ObjectHandle>,
        F: FnMut(&S, &mut dyn FnMut(ObjectHandle)),
    {
        let plan = self.begin_collection_with(roots, trace_slot, |_| false);
        self.finish_collection(plan)
    }

    /// Applies language assignment semantics using a runtime-provided slot
    /// copy operation. The callback is not invoked for handle objects.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or incompletely constructed object,
    /// exhausted identity space, or an error reported by `copy_slot`.
    pub fn assignment_copy_with<F>(
        &mut self,
        source: &ObjectRef,
        mut copy_slot: F,
    ) -> ObjectResult<ObjectRef>
    where
        F: FnMut(&S) -> ObjectResult<S>,
    {
        let source_record = self.record(source)?;
        if source_record.state != ConstructionState::Constructed {
            return Err(ObjectError::ObjectNotConstructed {
                id: source.id,
                state: source_record.state,
            });
        }
        if source_record.semantics == ClassSemantics::Handle {
            return Ok(ObjectRef::new(source.id, source.class_id, source.semantics));
        }

        let mut slots = BTreeMap::new();
        for (key, slot) in &source_record.slots {
            slots.insert(key.clone(), copy_slot(slot)?);
        }
        let copied_record = ObjectRecord {
            class_id: source_record.class_id,
            semantics: source_record.semantics,
            state: source_record.state,
            handle_state: HandleState::Alive,
            slots,
        };
        let (id, next_id) = self.reserve_id()?;
        let slot_count = copied_record.slots.len();
        self.objects.insert(id, copied_record);
        self.next_id = next_id;
        self.record_allocation(slot_count);
        Ok(ObjectRef::new(id, source.class_id, source.semantics))
    }
}

impl<S: Clone> ObjectStore<S> {
    /// Allocates all inherited stored-property slots. Explicit class defaults
    /// are cloned; the callback supplies the language's implicit default for a
    /// declaration that has none.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown class, exhausted identity space, or an
    /// error reported by `implicit_default`.
    pub fn allocate_with<F>(
        &mut self,
        registry: &ClassRegistry<S>,
        class_id: ClassId,
        mut implicit_default: F,
    ) -> ObjectResult<ObjectRef>
    where
        F: FnMut(&PropertyDescriptor<S>) -> ObjectResult<S>,
    {
        let semantics = registry.semantics(class_id)?;
        let mut slots = BTreeMap::new();
        for (key, property) in registry.stored_properties(class_id)? {
            let value = match property.default_value() {
                Some(value) => value.clone(),
                None => implicit_default(property)?,
            };
            slots.insert(key, value);
        }
        let (id, next_id) = self.reserve_id()?;
        let slot_count = slots.len();
        self.objects.insert(
            id,
            ObjectRecord {
                class_id,
                semantics,
                state: ConstructionState::Allocated,
                handle_state: HandleState::Alive,
                slots,
            },
        );
        self.next_id = next_id;
        self.record_allocation(slot_count);
        Ok(ObjectRef::new(id, class_id, semantics))
    }

    /// Applies language assignment semantics to an already-constructed object.
    /// Handle objects receive another reference to the same record; value
    /// objects receive independent cloned storage under a fresh identity.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or incompletely constructed object, or
    /// when allocating a copied value identity fails.
    pub fn assignment_copy(&mut self, source: &ObjectRef) -> ObjectResult<ObjectRef> {
        self.assignment_copy_with(source, |slot| Ok(slot.clone()))
    }
}

impl<S: Clone + Default> ObjectStore<S> {
    /// Allocates using `S::default()` for stored properties without an explicit
    /// class default.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown class or exhausted object identity space.
    pub fn allocate(
        &mut self,
        registry: &ClassRegistry<S>,
        class_id: ClassId,
    ) -> ObjectResult<ObjectRef> {
        self.allocate_with(registry, class_id, |_| Ok(S::default()))
    }
}

/// Metadata-only model for a basic homogeneous object array.
///
/// Exact class equality is required. Handle identities may repeat (alias),
/// while repeated value identities are rejected so each value element denotes
/// independent object state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectArrayDescriptor {
    class_id: ClassId,
    semantics: ClassSemantics,
    shape: Vec<u64>,
    element_ids: Vec<ObjectId>,
}

impl ObjectArrayDescriptor {
    /// Validates exact class, effective semantics, shape, and value identity
    /// independence for a prospective homogeneous object array.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown class metadata, shape overflow or mismatch,
    /// heterogeneous elements, inconsistent semantics, or aliased value objects.
    pub fn new<S>(
        registry: &ClassRegistry<S>,
        class_id: ClassId,
        shape: Vec<u64>,
        elements: &[&ObjectRef],
    ) -> ObjectResult<Self> {
        let semantics = registry.semantics(class_id)?;
        let expected = shape
            .iter()
            .try_fold(1_u64, |product, dimension| product.checked_mul(*dimension))
            .ok_or(ObjectError::ShapeOverflow)?;
        let actual = u64::try_from(elements.len()).map_err(|_| ObjectError::ShapeOverflow)?;
        if expected != actual {
            return Err(ObjectError::ArrayElementCountMismatch {
                expected,
                actual: elements.len(),
            });
        }

        let mut value_ids = BTreeSet::new();
        let mut element_ids = Vec::new();
        element_ids.try_reserve_exact(elements.len()).map_err(|_| {
            ObjectError::CapacityExceeded {
                resource: "object-array elements",
            }
        })?;
        for (index, element) in elements.iter().enumerate() {
            if element.class_id != class_id {
                return Err(ObjectError::HeterogeneousArrayElement {
                    index,
                    expected: class_id,
                    actual: element.class_id,
                });
            }
            if element.semantics != semantics {
                return Err(ObjectError::ObjectSemanticsMismatch {
                    id: element.id,
                    expected: semantics,
                    actual: element.semantics,
                });
            }
            if semantics == ClassSemantics::Value && !value_ids.insert(element.id) {
                return Err(ObjectError::ValueIdentityAliased { id: element.id });
            }
            element_ids.push(element.id);
        }

        Ok(Self {
            class_id,
            semantics,
            shape,
            element_ids,
        })
    }

    #[must_use]
    pub const fn class_id(&self) -> ClassId {
        self.class_id
    }

    #[must_use]
    pub const fn semantics(&self) -> ClassSemantics {
        self.semantics
    }

    #[must_use]
    pub fn shape(&self) -> &[u64] {
        &self.shape
    }

    #[must_use]
    pub fn element_ids(&self) -> &[ObjectId] {
        &self.element_ids
    }
}
