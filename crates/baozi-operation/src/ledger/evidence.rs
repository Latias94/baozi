use super::storage::DynamicStorage;
use super::{LedgerEvidenceError, LedgerEvidencePurpose, capacity_bytes, release_evidence};
use crate::{CapacityPlan, CapacityPlanError, ReplacementCapacityPlan};

pub(super) fn try_grow_selected_evidence_vec<P, D, T>(
    storage: &mut DynamicStorage<P, D>,
    values: &mut Vec<T>,
    desired_capacity: usize,
    retained_bytes: &mut u64,
    purpose: LedgerEvidencePurpose,
) -> Result<(), LedgerEvidenceError> {
    let original_bytes =
        capacity_bytes(values).map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    if original_bytes != *retained_bytes {
        return Err(LedgerEvidenceError::TrackedStorageCoverageMismatch {
            purpose,
            retained_bytes: *retained_bytes,
            capacity_bytes: original_bytes,
        });
    }
    let source_plan =
        CapacityPlan::for_type::<T>(values.len(), values.capacity(), desired_capacity)
            .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    if source_plan.desired_capacity() <= values.capacity() {
        source_plan
            .finish(values.len(), values.capacity())
            .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
        return Ok(());
    }
    storage
        .read()
        .current_administrative_bytes
        .checked_sub(original_bytes)
        .ok_or(LedgerEvidenceError::Capacity {
            purpose,
            cause: CapacityPlanError::CapacityOverflow,
        })?;
    let replacement_plan = ReplacementCapacityPlan::for_type::<T>(
        values.len(),
        values.capacity(),
        source_plan.desired_capacity(),
    )
    .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
    let mut candidate = StorageCandidate::try_with_capacity(
        storage,
        replacement_plan.candidate().desired_capacity(),
        purpose,
    )?;
    candidate.values.append(values);
    std::mem::swap(values, &mut candidate.values);
    drop(std::mem::take(&mut candidate.values));
    release_evidence(&mut candidate.admission.storage.write(), original_bytes);
    let (_, bytes) = candidate.into_parts();
    *retained_bytes = bytes;
    Ok(())
}

struct StorageAdmission<'a, P, D> {
    storage: &'a mut DynamicStorage<P, D>,
    bytes: u64,
}

impl<'a, P, D> StorageAdmission<'a, P, D> {
    fn new(
        storage: &'a mut DynamicStorage<P, D>,
        bytes: u64,
        purpose: LedgerEvidencePurpose,
    ) -> Result<Self, LedgerEvidenceError> {
        if bytes != 0 {
            let mut state = storage.write();
            let admitted = state
                .current_administrative_bytes
                .checked_add(bytes)
                .ok_or(LedgerEvidenceError::Capacity {
                    purpose,
                    cause: CapacityPlanError::CapacityOverflow,
                })?;
            if admitted > state.evidence_byte_limit {
                return Err(LedgerEvidenceError::LimitExceeded {
                    purpose,
                    value: admitted,
                    limit: state.evidence_byte_limit,
                });
            }
            state.current_administrative_bytes = admitted;
            state.peak_administrative_bytes = state.peak_administrative_bytes.max(admitted);
        }
        Ok(Self { storage, bytes })
    }

    fn retain_actual_bytes(&mut self, actual_bytes: u64) {
        let unused_bytes = self.bytes - actual_bytes;
        if unused_bytes != 0 {
            release_evidence(&mut self.storage.write(), unused_bytes);
        }
        self.bytes = actual_bytes;
    }

    fn into_bytes(mut self) -> u64 {
        std::mem::take(&mut self.bytes)
    }
}

impl<P, D> Drop for StorageAdmission<'_, P, D> {
    fn drop(&mut self) {
        if self.bytes != 0 {
            release_evidence(&mut self.storage.write(), self.bytes);
        }
    }
}

pub(super) struct StorageCandidate<'a, P, D, T> {
    // Fields drop in declaration order, including unwind: backing precedes its admission.
    pub(super) values: Vec<T>,
    admission: StorageAdmission<'a, P, D>,
}

impl<'a, P, D, T> StorageCandidate<'a, P, D, T> {
    pub(super) fn try_with_capacity(
        storage: &'a mut DynamicStorage<P, D>,
        desired_capacity: usize,
        purpose: LedgerEvidencePurpose,
    ) -> Result<Self, LedgerEvidenceError> {
        let (mut candidate, plan) = Self::admit(storage, desired_capacity, purpose)?;
        candidate.allocate(plan, purpose)?;
        Ok(candidate)
    }

    fn admit(
        storage: &'a mut DynamicStorage<P, D>,
        desired_capacity: usize,
        purpose: LedgerEvidencePurpose,
    ) -> Result<(Self, CapacityPlan), LedgerEvidenceError> {
        let values = Vec::new();
        let requirement = CapacityPlan::for_type::<T>(0, values.capacity(), desired_capacity)
            .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
        let plan =
            CapacityPlan::for_type::<T>(0, values.capacity(), requirement.envelope_capacity())
                .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
        let bytes = plan
            .envelope_delta_bytes()
            .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
        let admission = StorageAdmission::new(storage, bytes, purpose)?;
        Ok((Self { values, admission }, plan))
    }

    fn allocate(
        &mut self,
        plan: CapacityPlan,
        purpose: LedgerEvidencePurpose,
    ) -> Result<(), LedgerEvidenceError> {
        plan.verify_preflight(self.values.len(), self.values.capacity())
            .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
        if plan.desired_capacity() <= self.values.capacity() {
            plan.finish(self.values.len(), self.values.capacity())
                .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
            return Ok(());
        }
        let requested_bytes = self.admission.bytes;
        #[cfg(test)]
        if super::test_take_evidence_allocation_failure() {
            return Err(LedgerEvidenceError::AllocationFailed {
                purpose,
                requested_bytes,
            });
        }
        if self
            .values
            .try_reserve_exact(plan.allocator_additional())
            .is_err()
        {
            return Err(LedgerEvidenceError::AllocationFailed {
                purpose,
                requested_bytes,
            });
        }
        #[cfg(test)]
        let observed_capacity = if super::test_take_evidence_over_envelope_failure() {
            plan.envelope_capacity().saturating_add(1)
        } else {
            self.values.capacity()
        };
        #[cfg(not(test))]
        let observed_capacity = self.values.capacity();
        let actual_bytes = plan
            .finish(self.values.len(), observed_capacity)
            .map_err(|cause| LedgerEvidenceError::Capacity { purpose, cause })?;
        self.admission.retain_actual_bytes(actual_bytes);
        Ok(())
    }

    pub(super) fn into_parts(self) -> (Vec<T>, u64) {
        (self.values, self.admission.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{LedgerState, lock};
    use super::super::{
        PublicationState, test_evidence_allocation_guard, test_fail_next_evidence_allocation,
        test_fail_next_evidence_over_envelope,
    };
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    const BASE_EVIDENCE: u64 = 128;

    fn storage() -> Arc<Mutex<Option<LedgerState<(), ()>>>> {
        Arc::new(Mutex::new(Some(LedgerState {
            dimensions: Vec::new(),
            phases: Vec::new(),
            phase_states: Vec::new(),
            dimension_snapshot_staging: Vec::new(),
            phase_snapshot_staging: Vec::new(),
            header: super::super::LedgerHeader {
                current_live_bytes: 0,
                peak_live_bytes: 0,
                active_reservations: 0,
                retained_output_bytes: 0,
                retained_output_reservations: 0,
                evidence_byte_limit: u64::MAX,
                current_administrative_bytes: BASE_EVIDENCE,
                peak_administrative_bytes: BASE_EVIDENCE,
                publication: PublicationState::Open,
                last_completed_phase: None,
            },
        })))
    }

    struct DropProbe {
        storage: Arc<Mutex<Option<LedgerState<(), ()>>>>,
        observed_bytes: Arc<AtomicU64>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            let state = self
                .storage
                .try_lock()
                .expect("destructor runs without the storage guard");
            self.observed_bytes.store(
                state
                    .as_ref()
                    .expect("installed test storage")
                    .current_administrative_bytes,
                Ordering::SeqCst,
            );
        }
    }

    #[test]
    fn backing_drops_before_owned_charge_after_concurrent_counter_changes() {
        let storage = storage();
        let observed_bytes = Arc::new(AtomicU64::new(0));
        let mut selected = DynamicStorage::Shared(Arc::clone(&storage));
        let mut evidence = StorageCandidate::try_with_capacity(
            &mut selected,
            1,
            LedgerEvidencePurpose::Publication,
        )
        .expect("evidence admission");
        evidence.values.push(DropProbe {
            storage: Arc::clone(&storage),
            observed_bytes: Arc::clone(&observed_bytes),
        });
        let owned_bytes = evidence.admission.bytes;
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let mut state = lock(&storage);
                    state.current_administrative_bytes += 29;
                    state.peak_administrative_bytes = state
                        .peak_administrative_bytes
                        .max(state.current_administrative_bytes);
                    release_evidence(&mut state, 17);
                })
                .join()
                .expect("concurrent evidence owner");
        });
        let peak = lock(&storage).peak_administrative_bytes;
        drop(evidence);
        assert_eq!(
            observed_bytes.load(Ordering::SeqCst),
            BASE_EVIDENCE + owned_bytes + 12
        );
        let state = lock(&storage);
        assert_eq!(state.current_administrative_bytes, BASE_EVIDENCE + 12);
        assert_eq!(state.peak_administrative_bytes, peak);
    }

    #[test]
    fn failed_allocation_releases_only_its_admission_and_keeps_peak() {
        let _guard = test_evidence_allocation_guard();
        for over_envelope in [false, true] {
            let storage = storage();
            let mut selected = DynamicStorage::Shared(Arc::clone(&storage));
            let (mut evidence, plan) = StorageCandidate::<(), (), u64>::admit(
                &mut selected,
                3,
                LedgerEvidencePurpose::Publication,
            )
            .expect("evidence envelope admission");
            {
                let mut state = lock(&storage);
                state.current_administrative_bytes += 29;
                state.peak_administrative_bytes = state
                    .peak_administrative_bytes
                    .max(state.current_administrative_bytes);
                release_evidence(&mut state, 17);
            }
            let peak = lock(&storage).peak_administrative_bytes;
            if over_envelope {
                test_fail_next_evidence_over_envelope();
            } else {
                test_fail_next_evidence_allocation();
            }
            let error = evidence
                .allocate(plan, LedgerEvidencePurpose::Publication)
                .expect_err("injected allocation or capacity failure");
            if over_envelope {
                assert!(matches!(
                    error,
                    LedgerEvidenceError::Capacity {
                        purpose: LedgerEvidencePurpose::Publication,
                        cause: CapacityPlanError::CapacityEnvelopeViolated { .. },
                    }
                ));
            } else {
                assert!(matches!(
                    error,
                    LedgerEvidenceError::AllocationFailed {
                        purpose: LedgerEvidencePurpose::Publication,
                        requested_bytes: 32,
                    }
                ));
            }
            drop(evidence);
            let state = lock(&storage);
            assert_eq!(state.current_administrative_bytes, BASE_EVIDENCE + 12);
            assert_eq!(state.peak_administrative_bytes, peak);
        }
    }

    #[test]
    fn selected_spare_capacity_checks_owned_credit_without_entering_the_allocator() {
        let _guard = test_evidence_allocation_guard();
        for shared in [false, true] {
            let state = Arc::try_unwrap(storage()).unwrap().into_inner().unwrap();
            let mut storage = DynamicStorage::Inline(state);
            if shared {
                storage.ensure_shared().unwrap();
            }
            let baseline = storage.read().current_administrative_bytes;
            let (mut values, mut retained_bytes) =
                StorageCandidate::<_, _, u64>::try_with_capacity(
                    &mut storage,
                    4,
                    LedgerEvidencePurpose::Progress,
                )
                .unwrap()
                .into_parts();
            values.extend([11, 22]);
            let original_pointer = values.as_ptr();
            let original_capacity = values.capacity();
            let original_bytes = retained_bytes;
            let before = {
                let state = storage.read();
                (
                    state.current_administrative_bytes,
                    state.peak_administrative_bytes,
                )
            };
            super::super::test_set_evidence_allocation_failure(true);
            for corrupt_credit in [false, true] {
                if corrupt_credit {
                    retained_bytes = 0;
                }
                let result = try_grow_selected_evidence_vec(
                    &mut storage,
                    &mut values,
                    3,
                    &mut retained_bytes,
                    LedgerEvidencePurpose::Progress,
                );
                if corrupt_credit {
                    assert_eq!(
                        result,
                        Err(LedgerEvidenceError::TrackedStorageCoverageMismatch {
                            purpose: LedgerEvidencePurpose::Progress,
                            retained_bytes: 0,
                            capacity_bytes: original_bytes,
                        })
                    );
                } else {
                    result.unwrap();
                    assert_eq!(retained_bytes, original_bytes);
                }
                assert!(super::super::test_evidence_allocation_failure_is_armed());
                assert_eq!(values, [11, 22]);
                assert_eq!(values.as_ptr(), original_pointer);
                assert_eq!(values.capacity(), original_capacity);
                let state = storage.read();
                assert_eq!(
                    (
                        state.current_administrative_bytes,
                        state.peak_administrative_bytes
                    ),
                    before
                );
            }
            super::super::test_set_evidence_allocation_failure(false);
            drop(values);
            release_evidence(&mut storage.write(), original_bytes);
            assert_eq!(storage.read().current_administrative_bytes, baseline);
        }
    }

    #[test]
    fn selected_replacement_keeps_original_owner_on_failure_and_charges_actual_capacity() {
        let _guard = test_evidence_allocation_guard();
        for shared in [false, true] {
            let state = Arc::try_unwrap(storage()).unwrap().into_inner().unwrap();
            let mut storage = DynamicStorage::Inline(state);
            if shared {
                storage.ensure_shared().unwrap();
            }
            let baseline = storage.read().current_administrative_bytes;
            let mut values = vec![11_u64, 22];
            let original_capacity = values.capacity();
            let original_pointer = values.as_ptr();
            let original_bytes = capacity_bytes(&values).expect("source bytes");
            let mut retained_bytes = original_bytes;
            {
                let mut state = storage.write();
                state.current_administrative_bytes += original_bytes;
                state.peak_administrative_bytes = state.current_administrative_bytes;
            }
            let desired_capacity = original_capacity + 1;
            let envelope =
                ReplacementCapacityPlan::for_type::<u64>(2, original_capacity, desired_capacity)
                    .expect("replacement plan")
                    .candidate_envelope_bytes()
                    .expect("replacement envelope");
            for over_envelope in [false, true] {
                if over_envelope {
                    test_fail_next_evidence_over_envelope();
                } else {
                    test_fail_next_evidence_allocation();
                }
                assert!(
                    try_grow_selected_evidence_vec(
                        &mut storage,
                        &mut values,
                        desired_capacity,
                        &mut retained_bytes,
                        LedgerEvidencePurpose::Progress,
                    )
                    .is_err()
                );
                assert_eq!(values, [11, 22]);
                assert_eq!(values.as_ptr(), original_pointer);
                assert_eq!(values.capacity(), original_capacity);
                assert_eq!(retained_bytes, original_bytes);
                let state = storage.read();
                assert_eq!(
                    state.current_administrative_bytes,
                    baseline + original_bytes
                );
                assert_eq!(
                    state.peak_administrative_bytes,
                    baseline + original_bytes + envelope
                );
            }
            try_grow_selected_evidence_vec(
                &mut storage,
                &mut values,
                desired_capacity,
                &mut retained_bytes,
                LedgerEvidencePurpose::Progress,
            )
            .expect("qualified replacement");
            assert_eq!(values, [11, 22]);
            assert_eq!(
                retained_bytes,
                capacity_bytes(&values).expect("replacement bytes")
            );
            assert_eq!(
                storage.read().current_administrative_bytes,
                baseline + retained_bytes
            );
            drop(values);
            release_evidence(&mut storage.write(), retained_bytes);
            assert_eq!(storage.read().current_administrative_bytes, baseline);
        }
    }
}
