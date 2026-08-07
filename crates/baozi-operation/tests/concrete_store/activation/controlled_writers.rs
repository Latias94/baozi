use super::*;

#[test]
fn activation_reservation_survives_reborrows_for_multiple_canonical_writes() {
    let factory = factory(1);
    let account = account(&factory);
    let (operation, keys) = operation(&account);

    let outcome = operation
        .into_session_outcome_owner(evidence_plan(keys, 1))
        .plan_and_activate(
            |preflight| preflight.seal::<()>(canonical_plan(keys, 2)),
            |mut lane| {
                let mut reservation = {
                    let mut checkpoint = lane.control_checkpoint().unwrap();
                    checkpoint.reserve(keys.phase, keys.dimension, 2).unwrap()
                };

                let _first = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(1)?;
                            Ok(())
                        },
                    )
                    .unwrap();

                {
                    let mut checkpoint = lane.control_checkpoint().unwrap();
                    checkpoint.validate_live_owner(&reservation).unwrap();
                    checkpoint.resize(&mut reservation, 4).unwrap();
                }

                let second = lane
                    .try_retain_sequence(
                        SequencePlan::<u8>::new(1).unwrap(),
                        SequenceTag,
                        |writer, _control| {
                            writer.push(2)?;
                            Ok(())
                        },
                    )
                    .unwrap();

                {
                    let mut checkpoint = lane.control_checkpoint().unwrap();
                    checkpoint.resize(&mut reservation, 0).unwrap();
                }
                drop(reservation);
                lane.complete::<_, (), _, _>(second, second)
            },
        )
        .unwrap();

    assert!(matches!(outcome, CanonicalActivationOutcome::Activated(_)));
    drop(outcome);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn controlled_sequence_writer_preserves_its_first_fault() {
    assert_ignored_controlled_writer_fault_is_contained(ControlledWriterKind::Sequence);
}

#[test]
fn controlled_bytes_writer_preserves_its_first_fault() {
    assert_ignored_controlled_writer_fault_is_contained(ControlledWriterKind::Bytes);
}

#[test]
fn controlled_utf8_writer_preserves_its_first_fault() {
    assert_ignored_controlled_writer_fault_is_contained(ControlledWriterKind::Utf8);
}

#[test]
fn controlled_record_writer_preserves_its_first_fault() {
    assert_ignored_controlled_writer_fault_is_contained(ControlledWriterKind::Records);
}

#[test]
fn controlled_sequence_builder_body_and_drop_panics_are_isolated() {
    assert_controlled_writer_body_and_drop_panics_are_isolated(ControlledWriterKind::Sequence);
}

#[test]
fn controlled_bytes_builder_body_and_drop_panics_are_isolated() {
    assert_controlled_writer_body_and_drop_panics_are_isolated(ControlledWriterKind::Bytes);
}

#[test]
fn controlled_utf8_builder_body_and_drop_panics_are_isolated() {
    assert_controlled_writer_body_and_drop_panics_are_isolated(ControlledWriterKind::Utf8);
}

#[test]
fn controlled_record_builder_body_and_drop_panics_are_isolated() {
    assert_controlled_writer_body_and_drop_panics_are_isolated(ControlledWriterKind::Records);
}
