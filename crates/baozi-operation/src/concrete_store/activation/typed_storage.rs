//! Central admission for the core's actual immutable typed tables.

use std::convert::Infallible;

use super::*;
use crate::CapacityPlan;
use crate::concrete_store::execution::map_execution_error;
use crate::ledger::{WorkFrame, WorkLane, WorkLayout};
use crate::retained::{
    ExclusiveOperationParts, ORDINARY_WORK_PERMIT_ABANDONED_CODE, RetainedWriterError,
    SharedRetainedStorage, record_terminal_cause_with_precedence,
};

const STORAGE_BUILD_POLL_INTERVAL: usize = 256;

/// A typed-storage factory failure, carrying only destructor-free rejection facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalStorageBuildError<R: Copy> {
    /// An observed operation, resource, ownership, or construction failure.
    Runtime(StoreRuntimeError),
    /// A caller-owned rejection that permits only cleanup and terminal lane rejection.
    Rejected(R),
}

impl<R: Copy> From<StoreRuntimeError> for CanonicalStorageBuildError<R> {
    fn from(cause: StoreRuntimeError) -> Self {
        Self::Runtime(cause)
    }
}

impl CanonicalStorageBuildError<Infallible> {
    /// Recovers the runtime failure of a factory that cannot reject caller input.
    pub fn into_runtime(self) -> StoreRuntimeError {
        match self {
            Self::Runtime(cause) => cause,
            Self::Rejected(cause) => match cause {},
        }
    }
}

struct RetainedStorageWorkHost<'a, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    parts: ExclusiveOperationParts<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>,
    layout: WorkLayout<1>,
    frame: WorkFrame<1>,
}

impl<'a, O, PP, U> RetainedStorageWorkHost<'a, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn try_admit(
        mut parts: ExclusiveOperationParts<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>,
        dimension: StoreDimensionKey,
        work: u64,
    ) -> Result<Self, StoreRuntimeError> {
        let layout = parts.work.bind([dimension]).map_err(|cause| {
            record_terminal_cause_with_precedence(
                parts.ordinary.failure,
                PreparationFailureCode::ledger(),
            );
            StoreRuntimeError::Ledger(cause)
        })?;
        parts.ordinary.poll.poll().map_err(|cause| {
            record_terminal_cause_with_precedence(
                parts.ordinary.failure,
                PreparationFailureCode::from_control(&cause),
            );
            StoreRuntimeError::Control(cause)
        })?;
        let frame = layout
            .admit_observed(&mut parts.work.work, [work], WorkLane::Producer)
            .map_err(|cause| {
                record_terminal_cause_with_precedence(
                    parts.ordinary.failure,
                    PreparationFailureCode::ledger(),
                );
                StoreRuntimeError::Ledger(cause)
            })?;
        Ok(Self {
            parts,
            layout,
            frame,
        })
    }

    fn poll(&mut self) -> Result<(), StoreRuntimeError> {
        self.parts.ordinary.poll.poll().map_err(|cause| {
            record_terminal_cause_with_precedence(
                self.parts.ordinary.failure,
                PreparationFailureCode::from_control(&cause),
            );
            StoreRuntimeError::Control(cause)
        })
    }

    fn complete_unit(&mut self) -> Result<(), StoreRuntimeError> {
        self.frame
            .complete_batch_with_outcome(&self.layout, &mut self.parts.work.work, [1])
            .into_result()
            .map_err(|cause| {
                record_terminal_cause_with_precedence(
                    self.parts.ordinary.failure,
                    PreparationFailureCode::ledger(),
                );
                StoreRuntimeError::Ledger(cause)
            })
    }

    fn resolve(&mut self) -> Result<(), StoreRuntimeError> {
        self.frame
            .resolve(&self.layout, &mut self.parts.work.work, false)
            .map_err(|cause| {
                record_terminal_cause_with_precedence(
                    self.parts.ordinary.failure,
                    PreparationFailureCode::ledger(),
                );
                StoreRuntimeError::Ledger(cause)
            })
    }
}

impl<O, PP, U> Drop for RetainedStorageWorkHost<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.frame.active {
            let _ = self.resolve();
        }
    }
}

/// One admitted typed-table copy. Each normally returned action completes one work unit.
pub struct CanonicalRetainedStorageBuildControl<'a, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    work: Option<RetainedStorageWorkHost<'a, O, PP, U>>,
    strings_remaining: u64,
    sticky: Option<StoreRuntimeError>,
    since_poll: usize,
}

impl<O, PP, U> CanonicalRetainedStorageBuildControl<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, StoreRuntimeError> {
        if let Some(cause) = &self.sticky {
            return Err(cause.clone());
        }
        let Some(work) = self.work.as_mut() else {
            let cause = StoreRuntimeError::ObservedWorkExceedsAdmitted;
            self.sticky = Some(cause.clone());
            return Err(cause);
        };
        if work.frame.remaining[0] == 0 {
            let cause = StoreRuntimeError::ObservedWorkExceedsAdmitted;
            self.sticky = Some(cause.clone());
            return Err(cause);
        }
        if self.since_poll == 0
            && let Err(cause) = work.poll()
        {
            self.sticky = Some(cause.clone());
            return Err(cause);
        }
        let value = action();
        if let Err(cause) = work.complete_unit() {
            self.sticky = Some(cause.clone());
            drop(value);
            return Err(cause);
        }
        self.since_poll = (self.since_poll + 1) % STORAGE_BUILD_POLL_INTERVAL;
        Ok(value)
    }

    pub fn run_string_unit<R>(
        &mut self,
        action: impl FnOnce() -> R,
    ) -> Result<R, StoreRuntimeError> {
        if let Some(cause) = &self.sticky {
            return Err(cause.clone());
        }
        if self.strings_remaining == 0 {
            let cause = StoreRuntimeError::ConstructionInvariantUnverified;
            self.sticky = Some(cause.clone());
            return Err(cause);
        }
        let result = self.run_unit(action)?;
        self.strings_remaining -= 1;
        Ok(result)
    }

    fn finish(
        &mut self,
        complete: bool,
        first_fault: &mut Option<ActivationFirstFault>,
    ) -> Result<(), StoreRuntimeError> {
        if complete
            && first_fault.is_none()
            && (self.strings_remaining != 0
                || self
                    .work
                    .as_ref()
                    .is_some_and(|work| work.frame.remaining != [0]))
        {
            if let Some(work) = &self.work {
                record_terminal_cause_with_precedence(
                    work.parts.ordinary.failure,
                    PreparationFailureCode::product_graph(ORDINARY_WORK_PERMIT_ABANDONED_CODE),
                );
            }
            note_later_contract::<()>(first_fault, Err(StoreRuntimeError::ObservedWorkIncomplete));
        }
        if let Some(mut work) = self.work.take() {
            if let Some(code) = first_fault
                .as_ref()
                .and_then(ActivationFirstFault::recoverable_code)
            {
                record_terminal_cause_with_precedence(work.parts.ordinary.failure, code);
            }
            note_later_contract(first_fault, work.resolve());
        }
        preserved_fault(first_fault).map_or(Ok(()), Err)
    }
}

impl<O, PP, U, L> CanonicalLane<'_, '_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Admits the exact table backing capacities and one existing shared-owner header.
    /// The trusted builder retains no storage on a returned failure. A domain rejection makes
    /// the lane terminal: its caller must finish cleanup and return `CanonicalLane::reject`.
    pub fn try_retain_storage<T, R, const N: usize>(
        &mut self,
        capacities: [CapacityPlan; N],
        work: u64,
        semantic_bytes: u64,
        build: impl FnOnce(
            &mut CanonicalRetainedStorageBuildControl<'_, O, PP, U>,
        ) -> Result<T, CanonicalStorageBuildError<R>>,
    ) -> Result<SharedRetainedStorage<T>, CanonicalStorageBuildError<R>>
    where
        T: Send + Sync + 'static,
        R: Copy,
    {
        self.try_retain_storage_prepared(
            capacities,
            work,
            semantic_bytes,
            |_| Ok(()),
            |(), control| build(control),
        )
    }

    /// Admits one aggregate before preparing its shared children and building its own tables.
    ///
    /// Preparation may use the existing lane sharing operations and allocate only capacities
    /// included in this admission. Its returned storage must move into the final value. Separate
    /// retained child allocations keep their own existing owners and are not charged again here.
    /// A returned failure destroys preparation data before releasing this aggregate's admission.
    pub fn try_retain_storage_prepared<P, T, R, const N: usize>(
        &mut self,
        capacities: [CapacityPlan; N],
        work: u64,
        semantic_bytes: u64,
        prepare: impl FnOnce(&mut Self) -> Result<P, StoreRuntimeError>,
        build: impl FnOnce(
            P,
            &mut CanonicalRetainedStorageBuildControl<'_, O, PP, U>,
        ) -> Result<T, CanonicalStorageBuildError<R>>,
    ) -> Result<SharedRetainedStorage<T>, CanonicalStorageBuildError<R>>
    where
        T: Send + Sync + 'static,
        R: Copy,
    {
        let result = self.retain_storage(capacities, work, semantic_bytes, prepare, build);
        match result {
            Err(CanonicalStorageBuildError::Runtime(cause)) => {
                note_later_contract::<()>(self.first_fault, Err(cause.clone()));
                Err(preserved_fault(self.first_fault).unwrap_or(cause).into())
            }
            Err(CanonicalStorageBuildError::Rejected(cause)) => {
                self.pending_rejection = true;
                Err(CanonicalStorageBuildError::Rejected(cause))
            }
            Ok(storage) => Ok(storage),
        }
    }

    fn retain_storage<P, T, R, const N: usize>(
        &mut self,
        capacities: [CapacityPlan; N],
        work: u64,
        semantic_bytes: u64,
        prepare: impl FnOnce(&mut Self) -> Result<P, StoreRuntimeError>,
        build: impl FnOnce(
            P,
            &mut CanonicalRetainedStorageBuildControl<'_, O, PP, U>,
        ) -> Result<T, CanonicalStorageBuildError<R>>,
    ) -> Result<SharedRetainedStorage<T>, CanonicalStorageBuildError<R>>
    where
        T: Send + Sync + 'static,
        R: Copy,
    {
        if let Some(cause) = self.entry_failure() {
            return Err(cause.into());
        }
        self.assembly.ensure_writable()?;
        self.control_checkpoint()?.poll()?;
        if semantic_bytes > work {
            return Err(StoreRuntimeError::ConstructionInvariantUnverified.into());
        }
        let mut bytes = SharedRetainedStorage::<T>::header_bytes()
            .map_err(|cause| map_writer_error(&RetainedWriterError::Physical(cause)))?;
        let mut allocations = 1_u64;
        for capacity in capacities {
            if capacity.original_capacity() != 0 || capacity.original_len() != 0 {
                return Err(StoreRuntimeError::ConstructionInvariantUnverified.into());
            }
            let allocation = capacity
                .envelope_delta_bytes()
                .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
            bytes = bytes
                .checked_add(allocation)
                .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
            allocations += u64::from(allocation != 0);
        }
        let (phase, dimension) = self
            .assembly
            .raw
            .storage_destination()
            .map_err(|cause| map_publication_error(&cause))?;
        let operation = self
            .assembly
            .raw
            .storage_operation()
            .map_err(|cause| map_publication_error(&cause))?;
        let (mut attribution, physical) = operation
            .reserve_retained_storage(phase, dimension, bytes, allocations)
            .map_err(|cause| map_writer_error(&cause))?;
        let prepared = match catch_unwind(AssertUnwindSafe(|| prepare(self))) {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(cause)) => {
                note_later_contract::<()>(self.first_fault, Err(cause.clone()));
                if !crate::retained::release_owned_storage(physical, ()) {
                    attribution.retain_fail_closed_in_place();
                    note_later_contract::<()>(
                        self.first_fault,
                        Err(StoreRuntimeError::ConstructionInvariantUnverified),
                    );
                }
                return Err(cause.into());
            }
            Err(payload) => {
                note_later_contract::<()>(
                    self.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
                physical.record_unresolved();
                attribution.retain_fail_closed_in_place();
                std::mem::forget(physical);
                discard_untrusted_panic_payload(payload);
                return Err(StoreRuntimeError::ConstructionInvariantUnverified.into());
            }
        };
        if let Some(cause) = self.entry_failure() {
            if !crate::retained::release_owned_storage(physical, prepared) {
                attribution.retain_fail_closed_in_place();
                note_later_contract::<()>(
                    self.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
            }
            return Err(cause.into());
        }
        let operation = match self.assembly.raw.storage_operation() {
            Ok(operation) => operation,
            Err(cause) => {
                let cause = map_publication_error(&cause);
                note_later_contract::<()>(self.first_fault, Err(cause.clone()));
                if !crate::retained::release_owned_storage(physical, prepared) {
                    attribution.retain_fail_closed_in_place();
                    note_later_contract::<()>(
                        self.first_fault,
                        Err(StoreRuntimeError::ConstructionInvariantUnverified),
                    );
                }
                return Err(cause.into());
            }
        };
        let observed = match if work == 0 {
            Ok(None)
        } else {
            operation
                .exclusive_parts()
                .map_err(map_execution_error)
                .and_then(|parts| {
                    RetainedStorageWorkHost::try_admit(
                        parts,
                        self.materialization_work_dimension,
                        work,
                    )
                })
                .map(Some)
        } {
            Ok(work) => work,
            Err(cause) => {
                note_later_contract::<()>(self.first_fault, Err(cause.clone()));
                if !crate::retained::release_owned_storage(physical, prepared) {
                    attribution.retain_fail_closed_in_place();
                    note_later_contract::<()>(
                        self.first_fault,
                        Err(StoreRuntimeError::ConstructionInvariantUnverified),
                    );
                }
                return Err(cause.into());
            }
        };
        let mut control = CanonicalRetainedStorageBuildControl {
            work: observed,
            strings_remaining: semantic_bytes,
            sticky: None,
            since_poll: 0,
        };
        let built = catch_unwind(AssertUnwindSafe(|| build(prepared, &mut control)));
        if let Some(cause) = &control.sticky {
            note_later_contract::<()>(self.first_fault, Err(cause.clone()));
        }
        if let Ok(Err(CanonicalStorageBuildError::Runtime(cause))) = &built {
            note_later_contract::<()>(self.first_fault, Err(cause.clone()));
        }
        if built.is_err() {
            note_later_contract::<()>(
                self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        let completion = control.finish(matches!(&built, Ok(Ok(_))), self.first_fault);
        drop(control);
        if completion.is_ok() {
            let observed = self
                .control_checkpoint()
                .and_then(|mut checkpoint| checkpoint.poll());
            note_later_contract(self.first_fault, observed);
        }
        let value = match built {
            Ok(Ok(value)) => {
                if let Some(cause) = preserved_fault(self.first_fault) {
                    if !crate::retained::release_owned_storage(physical, value) {
                        attribution.retain_fail_closed_in_place();
                        note_later_contract::<()>(
                            self.first_fault,
                            Err(StoreRuntimeError::ConstructionInvariantUnverified),
                        );
                    }
                    return Err(cause.into());
                }
                value
            }
            Ok(Err(cause)) => {
                if !crate::retained::release_owned_storage(physical, ()) {
                    attribution.retain_fail_closed_in_place();
                    note_later_contract::<()>(
                        self.first_fault,
                        Err(StoreRuntimeError::ConstructionInvariantUnverified),
                    );
                }
                return Err(preserved_fault(self.first_fault).map_or(cause, Into::into));
            }
            Err(payload) => {
                physical.record_unresolved();
                attribution.retain_fail_closed_in_place();
                std::mem::forget(physical);
                discard_untrusted_panic_payload(payload);
                return Err(StoreRuntimeError::ConstructionInvariantUnverified.into());
            }
        };
        let storage = physical.try_bind_shared(value).map_err(|cause| {
            if cause.charge_retained() {
                attribution.retain_fail_closed_in_place();
            }
            map_writer_error(&RetainedWriterError::Physical(cause))
        })?;
        attribution
            .retain_for_output()
            .map_err(StoreRuntimeError::Ledger)?;
        Ok(storage.with_attribution(attribution.take_storage_owner()))
    }
}

impl<O, PP, U> CanonicalLane<'_, '_, O, PP, U, CanonicalInheritanceSource>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn try_share_storage<T: Send + Sync + 'static>(
        &mut self,
        source: &SharedRetainedStorage<T>,
    ) -> Result<SharedRetainedStorage<T>, StoreRuntimeError> {
        if let Some(cause) = self.entry_failure() {
            return Err(cause);
        }
        self.control_checkpoint()?.poll()?;
        let result = self
            .assembly
            .raw
            .share_typed_storage(source)
            .map_err(|cause| map_publication_error(&cause));
        preserve_first_fault(self.first_fault, result)
    }
}

#[cfg(test)]
mod tests {
    use super::super::scoped_transition_tests::{
        TestActivationDescription, TestInvocationHost, TestOutcome, TestProgress, TestRejection,
        TestUnit, account, operation_with_control_and_evidence_limit, scoped_shape,
    };
    use super::*;
    use crate::{
        CancellationOrigin, CancellationSignal, OperationControl, PhysicalStorageAccount,
        PhysicalStorageAccountSnapshot, ProgressDecision, ProgressEvent,
    };
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct PollProbe {
        calls: Arc<AtomicUsize>,
        cancelled: Arc<AtomicBool>,
    }

    impl CancellationSignal for PollProbe {
        fn is_cancelled(&self) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    fn observed_control(
        polls: &Arc<AtomicUsize>,
        observations: &Arc<AtomicUsize>,
        cancelled: &Arc<AtomicBool>,
    ) -> OperationControl<TestProgress, TestUnit> {
        let observations = Arc::clone(observations);
        OperationControl::default()
            .with_cancellation(PollProbe {
                calls: Arc::clone(polls),
                cancelled: Arc::clone(cancelled),
            })
            .with_progress_observer(move |_: ProgressEvent<TestProgress, TestUnit>| {
                observations.fetch_add(1, Ordering::SeqCst);
                ProgressDecision::Continue
            })
    }

    struct DropProbe {
        account: Arc<PhysicalStorageAccount>,
        snapshot: Arc<Mutex<Option<PhysicalStorageAccountSnapshot>>>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            assert!(!std::thread::panicking());
            let snapshot = self.account.snapshot();
            assert!(!snapshot.poisoned());
            assert!(snapshot.live_retained_allocations() > 0);
            {
                let mut dropped = self.snapshot.lock().unwrap();
                assert!(dropped.is_none());
                *dropped = Some(snapshot);
            }
            panic!("injected typed-storage owner release failure");
        }
    }

    #[test]
    fn failed_suffix_resolution_preserves_first_runtime_and_contains_actual_integrity_fault() {
        for prior_runtime in [false, true] {
            let account = Arc::new(account());
            let polls = Arc::new(AtomicUsize::new(0));
            let observations = Arc::new(AtomicUsize::new(0));
            let cancelled = Arc::new(AtomicBool::new(false));
            let completed = Cell::new(0);
            let frozen_polls = Cell::new(0);
            let frozen_observations = Cell::new(0);
            let mut host = TestInvocationHost::new();
            let body = host.run_body(|parts, _disposition| {
                let CanonicalBodyParts {
                    startup,
                    output,
                    activation,
                    ..
                } = parts;
                let (mut operation, keys) = operation_with_control_and_evidence_limit(
                    startup,
                    &account,
                    observed_control(&polls, &observations, &cancelled),
                    u64::MAX,
                );
                operation
                    .producer_host()
                    .report_progress(TestProgress::Build, TestUnit::Items, 1, Some(1))
                    .unwrap();
                let (policy, limits, failure, success) = scoped_shape(keys);
                let owner = operation
                    .into_session_outcome_owner(policy, limits, failure)
                    .unwrap();
                let mut description = Some(TestActivationDescription::new(success, ()));
                let outcome: Result<TestOutcome<'_>, StoreActivationContainment> = owner
                    .plan_and_activate_scoped(output, activation, |mut planning| {
                        let ((), completion) = planning
                            .with_semantic_work(|semantic| semantic.run_unit(|| ()))
                            .unwrap();
                        let permit = match planning
                            .seal_activation(description.take().unwrap(), completion)
                        {
                            CanonicalActivationSeal::Sealed(permit) => permit,
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                        permit.activate(|mut lane, _plan| {
                            assert!(lane.test_dimension_evidence(keys.dimension).is_some());
                            let capacity = CapacityPlan::for_type::<u8>(0, 0, 2).unwrap();
                            let cause = lane
                                .try_retain_storage::<Vec<u8>, (usize, usize), 1>(
                                    [capacity],
                                    2,
                                    0,
                                    |control| {
                                        let mut values =
                                            Vec::with_capacity(capacity.envelope_capacity());
                                        control.run_unit(|| {
                                            values.push(7);
                                            completed.set(completed.get() + 1);
                                        })?;
                                        let work = control.work.as_mut().unwrap();
                                        assert_eq!(work.frame.remaining, [1]);
                                        // The real ledger has one pending unit; resolving two must fail.
                                        work.frame.remaining[0] += 1;
                                        frozen_polls.set(polls.load(Ordering::SeqCst));
                                        frozen_observations
                                            .set(observations.load(Ordering::SeqCst));
                                        if prior_runtime {
                                            Err(StoreRuntimeError::Control(
                                                ControlCause::DeadlineExceeded,
                                            )
                                            .into())
                                        } else {
                                            Err(CanonicalStorageBuildError::Rejected((3, 1)))
                                        }
                                    },
                                )
                                .err()
                                .expect("failed resolution cannot return shared storage");
                            let expected = if prior_runtime {
                                StoreRuntimeError::Control(ControlCause::DeadlineExceeded)
                            } else {
                                StoreRuntimeError::Ledger(
                                    crate::LedgerCause::FinalSnapshotShapeMismatch,
                                )
                            };
                            assert_eq!(
                                cause,
                                CanonicalStorageBuildError::Runtime(expected.clone())
                            );
                            let first = lane.first_fault.as_ref().unwrap();
                            assert_eq!(first.cause, expected);
                            // Ledger is recoverable here; actual integrity/quiescence contains it.
                            assert!(!first.contract_seen);
                            assert!(lane.test_dimension_evidence(keys.dimension).is_none());
                            assert_eq!(completed.get(), 1);
                            assert!(frozen_polls.get() > 0);
                            assert!(frozen_observations.get() > 0);
                            assert_eq!(polls.load(Ordering::SeqCst), frozen_polls.get());
                            assert_eq!(
                                observations.load(Ordering::SeqCst),
                                frozen_observations.get()
                            );
                            lane.reject(TestRejection::Requested, 7)
                        })
                    });
                assert!(
                    outcome.is_err(),
                    "corrupt work cannot become a clean rejected session"
                );
            });
            assert!(body.is_err());
            assert_eq!(completed.get(), 1);
            assert!(account.snapshot().poisoned());
        }
    }

    #[test]
    fn nonclean_prepared_release_keeps_observed_control_and_admission_live() {
        let account = Arc::new(account());
        let polls = Arc::new(AtomicUsize::new(0));
        let observations = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(Mutex::new(None));
        let builder_called = Cell::new(false);
        let frozen_polls = Cell::new(0);
        let frozen_observations = Cell::new(0);
        let mut host = TestInvocationHost::new();
        let body = host.run_body(|parts, _disposition| {
            let CanonicalBodyParts {
                startup,
                output,
                activation,
                ..
            } = parts;
            let (mut operation, keys) = operation_with_control_and_evidence_limit(
                startup,
                &account,
                observed_control(&polls, &observations, &cancelled),
                u64::MAX,
            );
            operation
                .producer_host()
                .report_progress(TestProgress::Build, TestUnit::Items, 1, Some(1))
                .unwrap();
            let (policy, limits, failure, success) = scoped_shape(keys);
            let owner = operation
                .into_session_outcome_owner(policy, limits, failure)
                .unwrap();
            let mut description = Some(TestActivationDescription::new(success, ()));
            let outcome: Result<TestOutcome<'_>, StoreActivationContainment> = owner
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let ((), completion) = planning
                        .with_semantic_work(|semantic| semantic.run_unit(|| ()))
                        .unwrap();
                    let permit =
                        match planning.seal_activation(description.take().unwrap(), completion) {
                            CanonicalActivationSeal::Sealed(permit) => permit,
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                    permit.activate(|mut lane, _plan| {
                        let baseline = account.snapshot();
                        let baseline_live = lane
                            .test_dimension_evidence(keys.dimension)
                            .unwrap()
                            .current_live_bytes();
                        let cause = lane
                            .try_retain_storage_prepared::<_, (), Infallible, 0>(
                                [],
                                2,
                                0,
                                |lane| {
                                    cancelled.store(true, Ordering::SeqCst);
                                    let cause =
                                        lane.control_checkpoint().unwrap().poll().unwrap_err();
                                    assert_eq!(
                                        cause,
                                        StoreRuntimeError::Control(ControlCause::Cancelled {
                                            origin: CancellationOrigin::Signal,
                                        })
                                    );
                                    frozen_polls.set(polls.load(Ordering::SeqCst));
                                    frozen_observations.set(observations.load(Ordering::SeqCst));
                                    Ok(DropProbe {
                                        account: Arc::clone(&account),
                                        snapshot: Arc::clone(&dropped),
                                    })
                                },
                                |_prepared, _control| {
                                    builder_called.set(true);
                                    Ok(())
                                },
                            )
                            .err()
                            .expect("failed prepared release cannot bind output")
                            .into_runtime();
                        assert_eq!(
                            cause,
                            StoreRuntimeError::Control(ControlCause::Cancelled {
                                origin: CancellationOrigin::Signal,
                            })
                        );
                        assert!(!builder_called.get());
                        let first = lane.first_fault.as_ref().unwrap();
                        assert_eq!(first.cause, cause);
                        assert!(first.contract_seen);
                        let at_drop = dropped.lock().unwrap().unwrap();
                        assert!(at_drop.live_retained_bytes() > baseline.live_retained_bytes());
                        assert_eq!(
                            at_drop.live_retained_allocations(),
                            baseline.live_retained_allocations() + 1
                        );
                        let held = account.snapshot();
                        assert!(held.poisoned());
                        assert_eq!(held.live_retained_bytes(), at_drop.live_retained_bytes());
                        assert_eq!(
                            held.live_retained_allocations(),
                            at_drop.live_retained_allocations()
                        );
                        assert!(
                            lane.test_dimension_evidence(keys.dimension)
                                .unwrap()
                                .current_live_bytes()
                                > baseline_live
                        );
                        assert!(frozen_observations.get() > 0);
                        assert_eq!(polls.load(Ordering::SeqCst), frozen_polls.get());
                        assert_eq!(
                            observations.load(Ordering::SeqCst),
                            frozen_observations.get()
                        );
                        lane.reject(TestRejection::Requested, 7)
                    })
                });
            assert!(outcome.is_err());
        });
        assert!(body.is_err());
        assert!(account.snapshot().poisoned());
    }

    #[test]
    fn nonclean_output_release_keeps_final_control_failure_and_completed_prefix() {
        let account = Arc::new(account());
        let polls = Arc::new(AtomicUsize::new(0));
        let observations = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(Mutex::new(None));
        let completed = Cell::new(0);
        let frozen_polls = Cell::new(0);
        let frozen_observations = Cell::new(0);
        let mut host = TestInvocationHost::new();
        let body = host.run_body(|parts, _disposition| {
            let CanonicalBodyParts {
                startup,
                output,
                activation,
                ..
            } = parts;
            let (mut operation, keys) = operation_with_control_and_evidence_limit(
                startup,
                &account,
                observed_control(&polls, &observations, &cancelled),
                u64::MAX,
            );
            operation
                .producer_host()
                .report_progress(TestProgress::Build, TestUnit::Items, 1, Some(1))
                .unwrap();
            let (policy, limits, failure, success) = scoped_shape(keys);
            let owner = operation
                .into_session_outcome_owner(policy, limits, failure)
                .unwrap();
            let mut description = Some(TestActivationDescription::new(success, ()));
            let outcome: Result<TestOutcome<'_>, StoreActivationContainment> = owner
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let ((), completion) = planning
                        .with_semantic_work(|semantic| semantic.run_unit(|| ()))
                        .unwrap();
                    let permit =
                        match planning.seal_activation(description.take().unwrap(), completion) {
                            CanonicalActivationSeal::Sealed(permit) => permit,
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                    permit.activate(|mut lane, _plan| {
                        let baseline = account.snapshot();
                        let baseline_work = lane.test_dimension_evidence(keys.dimension).unwrap();
                        let cause = lane
                            .try_retain_storage::<DropProbe, Infallible, 0>([], 1, 0, |control| {
                                control.run_unit(|| completed.set(completed.get() + 1))?;
                                assert_eq!(control.work.as_ref().unwrap().frame.remaining, [0]);
                                cancelled.store(true, Ordering::SeqCst);
                                frozen_polls.set(polls.load(Ordering::SeqCst));
                                frozen_observations.set(observations.load(Ordering::SeqCst));
                                Ok(DropProbe {
                                    account: Arc::clone(&account),
                                    snapshot: Arc::clone(&dropped),
                                })
                            })
                            .err()
                            .expect("failed output release cannot bind shared storage")
                            .into_runtime();
                        assert_eq!(
                            cause,
                            StoreRuntimeError::Control(ControlCause::Cancelled {
                                origin: CancellationOrigin::Signal,
                            })
                        );
                        let first = lane.first_fault.as_ref().unwrap();
                        assert_eq!(first.cause, cause);
                        assert!(first.contract_seen);
                        assert_eq!(completed.get(), 1);
                        let at_drop = dropped.lock().unwrap().unwrap();
                        assert!(at_drop.live_retained_bytes() > baseline.live_retained_bytes());
                        assert_eq!(
                            at_drop.live_retained_allocations(),
                            baseline.live_retained_allocations() + 1
                        );
                        let held = account.snapshot();
                        assert!(held.poisoned());
                        assert_eq!(held.live_retained_bytes(), at_drop.live_retained_bytes());
                        assert_eq!(
                            held.live_retained_allocations(),
                            at_drop.live_retained_allocations()
                        );
                        let held_work = lane.test_dimension_evidence(keys.dimension).unwrap();
                        assert!(
                            held_work.current_live_bytes() > baseline_work.current_live_bytes()
                        );
                        assert!(held_work.consumed() > baseline_work.consumed());
                        // Only the normal final checkpoint observes cancellation; cleanup adds none.
                        assert_eq!(polls.load(Ordering::SeqCst), frozen_polls.get() + 1);
                        assert!(frozen_observations.get() > 0);
                        assert_eq!(
                            observations.load(Ordering::SeqCst),
                            frozen_observations.get()
                        );
                        lane.reject(TestRejection::Requested, 7)
                    })
                });
            assert!(outcome.is_err());
        });
        assert!(body.is_err());
        assert_eq!(completed.get(), 1);
        assert!(account.snapshot().poisoned());
    }

    #[test]
    fn builder_panic_keeps_completed_prefix_without_post_panic_control_callbacks() {
        let account = Arc::new(account());
        let polls = Arc::new(AtomicUsize::new(0));
        let observations = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let completed = Cell::new(0);
        let frozen_polls = Cell::new(0);
        let frozen_observations = Cell::new(0);
        let mut host = TestInvocationHost::new();
        let body = host.run_body(|parts, _disposition| {
            let CanonicalBodyParts {
                startup,
                output,
                activation,
                ..
            } = parts;
            let (mut operation, keys) = operation_with_control_and_evidence_limit(
                startup,
                &account,
                observed_control(&polls, &observations, &cancelled),
                u64::MAX,
            );
            operation
                .producer_host()
                .report_progress(TestProgress::Build, TestUnit::Items, 1, Some(1))
                .unwrap();
            let (policy, limits, failure, success) = scoped_shape(keys);
            let owner = operation
                .into_session_outcome_owner(policy, limits, failure)
                .unwrap();
            let mut description = Some(TestActivationDescription::new(success, ()));
            let outcome: Result<TestOutcome<'_>, StoreActivationContainment> = owner
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let ((), completion) = planning
                        .with_semantic_work(|semantic| semantic.run_unit(|| ()))
                        .unwrap();
                    let permit =
                        match planning.seal_activation(description.take().unwrap(), completion) {
                            CanonicalActivationSeal::Sealed(permit) => permit,
                            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
                        };
                    permit.activate(|mut lane, _plan| {
                        let capacity = CapacityPlan::for_type::<u8>(0, 0, 2).unwrap();
                        let cause = lane
                            .try_retain_storage::<Vec<u8>, Infallible, 1>(
                                [capacity],
                                2,
                                0,
                                |control| {
                                    let mut values =
                                        Vec::with_capacity(capacity.envelope_capacity());
                                    control.run_unit(|| {
                                        values.push(7);
                                        completed.set(completed.get() + 1);
                                    })?;
                                    assert_eq!(control.work.as_ref().unwrap().frame.remaining, [1]);
                                    frozen_polls.set(polls.load(Ordering::SeqCst));
                                    frozen_observations.set(observations.load(Ordering::SeqCst));
                                    panic!(
                                        "injected builder panic after one completed physical action"
                                    );
                                },
                            )
                            .err()
                            .expect("builder panic cannot return shared storage")
                            .into_runtime();
                        assert_eq!(cause, StoreRuntimeError::ConstructionInvariantUnverified);
                        let first = lane.first_fault.as_ref().unwrap();
                        assert_eq!(first.cause, cause);
                        assert!(first.contract_seen);
                        assert_eq!(completed.get(), 1);
                        assert!(frozen_polls.get() > 0);
                        assert!(frozen_observations.get() > 0);
                        assert_eq!(polls.load(Ordering::SeqCst), frozen_polls.get());
                        assert_eq!(
                            observations.load(Ordering::SeqCst),
                            frozen_observations.get()
                        );
                        assert!(account.snapshot().poisoned());
                        lane.reject(TestRejection::Requested, 7)
                    })
                });
            assert!(outcome.is_err());
        });
        assert!(body.is_err());
        assert_eq!(completed.get(), 1);
        assert!(account.snapshot().poisoned());
    }
}
