use super::*;
use baozi_operation::__retained_store_integration::authority::RetainedStorage;
use baozi_operation::CapacityPlan;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn payloads(
    operation: &mut TestOperation,
    keys: TestKeys,
    count: usize,
) -> (Vec<RetainedStorage<Vec<u8>>>, u64) {
    let mut outputs = Vec::new();
    let mut charged_bytes = 0;
    for index in 0..count {
        let plan = CapacityPlan::for_type::<u8>(0, 0, 31 + index).unwrap();
        let bytes = plan.envelope_delta_bytes().unwrap();
        let reservation = operation
            .reserve_retained_storage(keys.phase, keys.dimension, bytes, 1)
            .unwrap();
        let mut value = Vec::new();
        value
            .try_reserve_exact(plan.allocator_additional())
            .unwrap();
        plan.finish(value.len(), value.capacity()).unwrap();
        value.resize(plan.desired_capacity(), index as u8);
        let mut output = reservation.bind_storage(value).unwrap();
        output.prepare_retained_output().unwrap();
        charged_bytes += bytes;
        outputs.push(output);
    }
    (outputs, charged_bytes)
}

#[test]
fn many_retained_outputs_do_not_consume_per_allocation_terminal_slots() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    let count = TEST_TERMINAL_ATTRIBUTION_SLOTS * 4;
    let (outputs, bytes) = payloads(&mut operation, keys, count);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .unwrap();
    let canonical = assembly
        .try_retain_canonical_bytes(BytesPlan::new(1).unwrap(), BytesTag, |writer| {
            writer.write_bytes(&[7])
        })
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_bytes(BytesPlan::new(1).unwrap(), EvidenceBytesTag, |writer| {
            writer.write_bytes(&[9])
        })
        .unwrap();
    let validated = match assembly.validate_success(canonical, evidence, |session, c, e| {
        session.canonical_bytes(c) == Some(&[7][..])
            && session.outcome_evidence_bytes(e) == Some(&[9][..])
    }) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("retained output must not prevent central validation"),
    };
    let prepared = match validated.try_prepare(final_progress()).unwrap() {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("retained output attribution must publish with success"),
    };
    let published = prepared.handoff();
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    assert!(published.ledger_evidence.header().peak_live_bytes() >= bytes);
    assert_eq!(outputs[17].as_ref(), &vec![17; 48]);
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), bytes);
    assert_eq!(account.snapshot().live_retained_allocations(), count as u64);
    drop(outputs);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn rich_failure_keeps_real_output_storage_charged_until_its_owner_drops() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    let (outputs, bytes) = payloads(&mut operation, keys, 4);
    let mut assembly = operation
        .try_publication_assembly(rich_failure_plan(keys, 1))
        .unwrap();
    let evidence = assembly
        .try_retain_outcome_evidence_bytes(BytesPlan::new(1).unwrap(), EvidenceBytesTag, |writer| {
            writer.write_bytes(&[3])
        })
        .unwrap();
    let validated = match assembly
        .validate_rich_failure(evidence, |session, e| session.bytes(e) == Some(&[3][..]))
    {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("rich failure evidence must validate"),
    };
    let published = match validated.try_prepare(71, final_progress()).unwrap() {
        PreparedStoreFailureOutcome::RichFailure(prepared) => prepared.handoff(),
        _ => panic!("retained output must permit rich failure publication"),
    };
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), bytes);
    drop(outputs);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn minimal_fallback_and_host_unwind_preserve_storage_first_release() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    let (outputs, bytes) = payloads(&mut operation, keys, 4);
    let published = operation.prepare_minimal_fallback(73).unwrap().handoff();
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), bytes);
    let unwind = catch_unwind(AssertUnwindSafe(move || {
        let _outputs = outputs;
        panic!("unrelated host unwind after minimal publication");
    }));
    assert!(unwind.is_err());
    drop(unwind);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn dropping_a_candidate_before_publication_removes_only_its_own_attribution() {
    let factory = factory(1);
    let account = account(&factory);
    let (mut operation, keys) = operation(&account);
    let baseline = account.snapshot();
    let (outputs, _) = payloads(&mut operation, keys, 4);
    drop(outputs);
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        baseline.live_retained_bytes()
    );
    assert_eq!(
        account.snapshot().live_retained_allocations(),
        baseline.live_retained_allocations()
    );
    let published = operation.prepare_minimal_fallback(73).unwrap().handoff();
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(!account.snapshot().poisoned());
}
