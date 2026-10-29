//! Central admission for the core's actual immutable typed tables.

use super::*;
use crate::CapacityPlan;
use crate::concrete_store::RetainedStoreObservedWork;
use crate::retained::{RetainedWriterError, SharedRetainedStorage};

/// One admitted typed-table copy. Each normally returned action completes one work unit.
pub struct CanonicalRetainedStorageBuildControl<'a, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    work: Option<RetainedStoreObservedWork<'a, O, PP, U, 1>>,
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
        if work.remaining()[0] == 0 {
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
        if let Err(cause) = work.complete([1]) {
            drop(value);
            self.sticky = Some(cause.clone());
            return Err(cause);
        }
        self.since_poll = (self.since_poll + 1) % 4096;
        Ok(value)
    }

    pub fn run_string_unit<R>(
        &mut self,
        action: impl FnOnce() -> R,
    ) -> Result<R, StoreRuntimeError> {
        if self.strings_remaining == 0 {
            let cause = StoreRuntimeError::ConstructionInvariantUnverified;
            self.sticky = Some(cause.clone());
            return Err(cause);
        }
        let result = self.run_unit(action)?;
        self.strings_remaining -= 1;
        Ok(result)
    }

    fn finish(&mut self, complete: bool) -> Result<(), StoreRuntimeError> {
        let Some(work) = self.work.take() else {
            return if self.strings_remaining == 0 && self.sticky.is_none() {
                Ok(())
            } else {
                Err(self
                    .sticky
                    .clone()
                    .unwrap_or(StoreRuntimeError::ObservedWorkIncomplete))
            };
        };
        if !complete || self.sticky.is_some() {
            work.finish_partial();
            return self.sticky.clone().map_or(Ok(()), Err);
        }
        if self.strings_remaining != 0 {
            work.finish_partial();
            return Err(StoreRuntimeError::ObservedWorkIncomplete);
        }
        work.finish()
    }
}

impl<O, PP, U> Drop for CanonicalRetainedStorageBuildControl<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            work.finish_partial();
        }
    }
}

impl<O, PP, U, L> CanonicalLane<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Admits the exact table backing capacities and one existing shared-owner header.
    /// The trusted builder retains no storage on a returned failure.
    pub fn try_retain_storage<T, const N: usize>(
        &mut self,
        capacities: [CapacityPlan; N],
        work: u64,
        semantic_bytes: u64,
        build: impl FnOnce(
            &mut CanonicalRetainedStorageBuildControl<'_, O, PP, U>,
        ) -> Result<T, StoreRuntimeError>,
    ) -> Result<SharedRetainedStorage<T>, StoreRuntimeError>
    where
        T: Send + Sync + 'static,
    {
        let result = self.retain_storage(capacities, work, semantic_bytes, build);
        preserve_first_fault(self.first_fault, result)
    }

    fn retain_storage<T, const N: usize>(
        &mut self,
        capacities: [CapacityPlan; N],
        work: u64,
        semantic_bytes: u64,
        build: impl FnOnce(
            &mut CanonicalRetainedStorageBuildControl<'_, O, PP, U>,
        ) -> Result<T, StoreRuntimeError>,
    ) -> Result<SharedRetainedStorage<T>, StoreRuntimeError>
    where
        T: Send + Sync + 'static,
    {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        self.assembly.ensure_writable()?;
        self.control_checkpoint()?.poll()?;
        if semantic_bytes > work {
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        let mut bytes = SharedRetainedStorage::<T>::header_bytes()
            .map_err(|cause| map_writer_error(&RetainedWriterError::Physical(cause)))?;
        let mut allocations = 1_u64;
        for capacity in capacities {
            if capacity.original_capacity() != 0 || capacity.original_len() != 0 {
                return Err(StoreRuntimeError::ConstructionInvariantUnverified);
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
        let mut host = operation.producer_host();
        let observed = match if work == 0 {
            Ok(None)
        } else {
            host.try_begin_observed_work([(self.materialization_work_dimension, work)])
                .map(Some)
        } {
            Ok(work) => work,
            Err(cause) => {
                if !crate::retained::release_owned_storage(physical, ()) {
                    attribution.retain_fail_closed_in_place();
                }
                return Err(super::super::map_producer_work_start_error(cause));
            }
        };
        let mut control = CanonicalRetainedStorageBuildControl {
            work: observed.map(|raw| RetainedStoreObservedWork { raw }),
            strings_remaining: semantic_bytes,
            sticky: None,
            since_poll: 0,
        };
        let built = catch_unwind(AssertUnwindSafe(|| build(&mut control)));
        let completion = control.finish(matches!(&built, Ok(Ok(_))));
        drop(control);
        let value = match built {
            Ok(Ok(value)) if completion.is_ok() => value,
            Ok(Ok(value)) => {
                if !crate::retained::release_owned_storage(physical, value) {
                    attribution.retain_fail_closed_in_place();
                }
                return Err(completion.unwrap_err());
            }
            Ok(Err(cause)) => {
                if !crate::retained::release_owned_storage(physical, ()) {
                    attribution.retain_fail_closed_in_place();
                }
                return Err(completion.err().unwrap_or(cause));
            }
            Err(payload) => {
                physical.record_unresolved();
                attribution.retain_fail_closed_in_place();
                std::mem::forget(physical);
                discard_untrusted_panic_payload(payload);
                return Err(StoreRuntimeError::ConstructionInvariantUnverified);
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

impl<O, PP, U> CanonicalLane<'_, O, PP, U, CanonicalInheritanceSource>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn try_share_storage<T: Send + Sync + 'static>(
        &mut self,
        source: &SharedRetainedStorage<T>,
    ) -> Result<SharedRetainedStorage<T>, StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        self.control_checkpoint()?.poll()?;
        let result = self
            .assembly
            .raw
            .share_typed_storage(source, self.materialization_work_dimension)
            .map_err(|cause| map_publication_error(&cause));
        preserve_first_fault(self.first_fault, result)
    }
}
