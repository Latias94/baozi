use super::*;
use crate::ledger::{DimensionPolicy, DynamicStorage, Ledger, LedgerLimits};

struct Owner;

impl OperationOwner for Owner {}

fn ledger() -> Ledger<Owner, u8, u8> {
    let limits = LedgerLimits::new(
        u64::MAX,
        [(
            0,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
        )],
    )
    .expect("detachment policy");
    Ledger::try_new(limits, &[0]).expect("detachment ledger")
}

fn assert_empty_owner(
    batch: &AttributionReservationBatch<Owner, u8, u8, CanonicalOutput>,
    identity: LedgerIdentity,
) {
    assert!(matches!(
        &batch.owner,
        AttributionBatchOwner::Empty(actual) if *actual == identity
    ));
    assert_eq!(
        (
            batch.attributions.len(),
            batch.attributions.capacity(),
            batch.attribution_limit,
        ),
        (0, 0, 0)
    );
    assert_eq!(
        (
            batch.commitments.len(),
            batch.commitments.capacity(),
            batch.commitment_limit,
        ),
        (0, 0, 0)
    );
    assert!(batch.capacity_cover.is_none());
}

#[test]
fn actual_empty_batch_detaches_without_shared_storage() {
    let mut ledger = ledger();
    let identity = ledger.identity;
    let mut batch = ledger
        .try_attribution_batch::<CanonicalOutput>(0, 0, 0, 0)
        .expect("actual empty batch");
    assert_empty_owner(&batch, identity);
    assert!(ledger.state.shared().is_none());

    let detached = batch.detach_in_place(None, identity);

    assert_empty_owner(&batch, identity);
    assert!(detached.attributions.is_empty());
    assert_eq!(detached.attributions.capacity(), 0);
    assert!(detached.commitments.is_empty());
    assert_eq!(detached.commitments.capacity(), 0);
    assert!(detached.capacity_cover.is_none());
    assert_eq!(detached.active_count(), 0);
    assert_eq!(detached.live_bytes(), 0);
    assert!(ledger.state.shared().is_none());
    let state = ledger.state.read();
    assert_eq!(state.active_reservations, 0);
    assert_eq!(state.current_live_bytes, 0);
}

#[test]
fn shared_batch_moves_actual_backings_and_keeps_nonzero_and_zero_byte_tokens_live() {
    let mut ledger = ledger();
    let identity = ledger.identity;
    let mut batch = ledger
        .try_attribution_batch::<CanonicalOutput>(0, 0, 2, 1)
        .expect("actual shared batch");
    for bytes in [7, 0] {
        let token = ledger
            .reserve_operation_attribution::<CanonicalOutput>(0, 0, bytes)
            .expect("actual attribution token");
        assert!(batch.try_push_attribution(token).is_ok());
    }
    assert!(
        ledger
            .try_push_commitment(&mut batch, OutputCommitment::new(0, 5))
            .is_ok()
    );
    let attribution_backing = (batch.attributions.as_ptr(), batch.attributions.capacity());
    let commitment_backing = (batch.commitments.as_ptr(), batch.commitments.capacity());
    let cover_bytes = batch.account_capacity_bytes();
    assert!(cover_bytes > 0);
    let shared = ledger.state.shared().expect("actual shared storage");
    assert_eq!(Arc::strong_count(shared), 5);
    {
        let state = lock(shared);
        assert_eq!(state.active_reservations, 3);
        assert_eq!(state.current_live_bytes, cover_bytes + 7);
    }

    let mut detached = batch.detach_in_place(Some(shared), identity);

    assert_empty_owner(&batch, identity);
    assert_eq!(Arc::strong_count(shared), 1);
    assert_eq!(
        (
            detached.attributions.as_ptr(),
            detached.attributions.capacity()
        ),
        attribution_backing
    );
    assert_eq!(
        (
            detached.commitments.as_ptr(),
            detached.commitments.capacity()
        ),
        commitment_backing
    );
    assert_eq!(detached.active_count(), 3);
    assert_eq!(detached.live_bytes(), cover_bytes + 7);
    assert_eq!(detached.attributions[0].bytes(), 7);
    assert_eq!(detached.attributions[1].bytes(), 0);
    assert!(
        detached
            .attributions
            .iter()
            .all(|token| token.core.owner.is_none() && token.core.active)
    );
    let cover = detached
        .capacity_cover
        .as_ref()
        .expect("moved capacity cover");
    assert!(cover.core.owner.is_none());
    assert!(cover.core.active);
    assert_eq!(cover.core.bytes, cover_bytes);

    let mut state = lock(shared);
    assert_eq!(state.active_reservations, 3);
    assert_eq!(state.current_live_bytes, cover_bytes + 7);
    assert_eq!(state.dimensions[0].state.committed, 0);
    detached.apply_commitments(&mut state.dimensions);
    assert_eq!(state.dimensions[0].state.committed, 5);
    assert_eq!(state.dimensions[0].state.committed_canonical_output, 5);
    assert_eq!(state.active_reservations, 3);
    assert_eq!(state.current_live_bytes, cover_bytes + 7);
    detached.release_attributions(&mut state.view_mut());
    assert_eq!(state.active_reservations, 1);
    assert_eq!(state.current_live_bytes, cover_bytes);
    detached.destroy_storage_then_release_cover(&mut state.view_mut());
    assert_eq!(state.active_reservations, 0);
    assert_eq!(state.current_live_bytes, 0);
    assert_eq!(state.dimensions[0].state.current_live_bytes, 0);
    assert_eq!(state.phase_states[0].current_live_bytes, 0);
}

#[test]
fn destroyed_discard_or_deferred_batch_retires_its_actual_shared_owner_before_unwrap() {
    for discard_payload in [false, true] {
        let mut ledger = ledger();
        let identity = ledger.identity;
        let mut batch = ledger
            .try_attribution_batch::<CanonicalOutput>(0, 0, 1, 1)
            .expect("actual discard or deferred batch");
        if discard_payload {
            let token = ledger
                .reserve_operation_attribution::<CanonicalOutput>(0, 0, 7)
                .expect("discard attribution");
            assert!(batch.try_push_attribution(token).is_ok());
        }
        assert!(
            ledger
                .try_push_commitment(&mut batch, OutputCommitment::new(0, 5))
                .is_ok()
        );
        let shared = ledger.state.shared().expect("actual shared storage");
        assert_eq!(
            Arc::strong_count(shared),
            if discard_payload { 4 } else { 3 }
        );
        {
            let mut state = lock(shared);
            batch.release_bound_attributions(&mut state.view_mut());
        }

        batch.destroy_storage_then_release_bound_cover();

        assert!(batch.attributions.is_empty());
        assert_eq!(batch.attributions.capacity(), 0);
        assert!(batch.commitments.is_empty());
        assert_eq!(batch.commitments.capacity(), 0);
        assert!(batch.capacity_cover.is_none());
        assert_eq!(Arc::strong_count(shared), 2);
        {
            let state = lock(shared);
            assert_eq!(state.active_reservations, 0);
            assert_eq!(state.current_live_bytes, 0);
            assert_eq!(state.dimensions[0].state.committed, 0);
        }

        batch.retire_empty_owner(identity);

        assert_empty_owner(&batch, identity);
        assert_eq!(Arc::strong_count(shared), 1);
        let DynamicStorage::Shared(shared) = ledger.state else {
            panic!("the actual storage remains shared until unwrapped");
        };
        assert!(Arc::try_unwrap(shared).is_ok());
    }
}
