use super::*;

#[test]
fn controlled_commit_transitions_one_candidate_without_canonical_reserve() {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let mut control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .unwrap()
    else {
        panic!("fixture transform must change")
    };
    assert_eq!(
        control
            .reserve_kinds
            .iter()
            .filter(|kind| **kind == SceneReservationKind::EditCandidate)
            .count(),
        1
    );

    let committed = edit.commit_with(&mut control).unwrap();
    let expected_bytes = committed
        .publication
        .scene
        .validation_stats()
        .footprint()
        .accounted_payload_bytes()
        + committed.publication.remaps.retained_allocation_bytes();
    assert_eq!(control.transition_count, 1);
    assert_eq!(control.transition_required_bytes, Some(expected_bytes));
    assert!(
        !control
            .reserve_kinds
            .contains(&SceneReservationKind::CanonicalOutput)
    );

    let publication = committed
        .try_publish(|reservation| {
            reservation.published = true;
            Ok::<(), TestControlError>(())
        })
        .unwrap();
    assert_ne!(publication.scene().generation(), input.generation());
    assert_eq!(control.rollback_observed_state(), 0);
}

#[test]
fn wrong_owner_commit_is_rejected_before_foreign_work_and_preserves_rollback() {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let mut owner = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(mut edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut owner,
        )
        .unwrap()
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(owner.shared_drop_state());
    let mut foreign = TestControl::new();

    assert!(matches!(
        edit.commit_with(&mut foreign),
        Err(CanonicalEditFailure::Control(TestControlError::WrongOwner))
    ));
    assert_eq!(foreign.poll_count, 0);
    assert_eq!(foreign.total_work, 0);
    assert_eq!(foreign.candidate_resize_count(), 0);
    assert_eq!(foreign.transition_count, 0);
    assert_eq!(owner.rollback_observed_state(), 0b111);
    assert_eq!(
        input.node(root).unwrap().authored_transform(),
        AuthoredTransform::identity_trs()
    );
}

#[test]
fn wrong_kind_commit_is_rejected_before_owner_work_and_preserves_rollback() {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let mut control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(mut edit) = input
        .try_edit_node_transform_with(
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .unwrap()
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(control.shared_drop_state());
    edit.reservation.kind = SceneReservationKind::CanonicalOutput;
    let polls_before_commit = control.poll_count;
    let work_before_commit = control.total_work;

    assert!(matches!(
        edit.commit_with(&mut control),
        Err(CanonicalEditFailure::Control(TestControlError::WrongKind))
    ));
    assert_eq!(control.poll_count, polls_before_commit);
    assert_eq!(control.total_work, work_before_commit);
    assert_eq!(control.candidate_resize_count(), 0);
    assert_eq!(control.transition_count, 0);
    assert_eq!(control.rollback_observed_state(), 0b111);
    assert_eq!(
        input.node(root).unwrap().authored_transform(),
        AuthoredTransform::identity_trs()
    );
}

#[test]
fn rejected_transition_preserves_bundle_until_candidate_rollback() {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let mut control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(mut edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .unwrap()
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(control.shared_drop_state());
    control.reject_transition = true;

    assert!(matches!(
        edit.commit_with(&mut control),
        Err(CanonicalEditFailure::Control(TestControlError::Injected))
    ));
    assert_eq!(control.candidate_resize_count(), 1);
    assert_eq!(control.transition_count, 1);
    assert_eq!(control.transition_required_bytes, None);
    assert_eq!(control.rollback_observed_state(), 0b111);
    assert_eq!(
        input.node(root).unwrap().authored_transform(),
        AuthoredTransform::identity_trs()
    );
}

#[test]
fn validation_failure_drops_bundle_before_candidate_rollback() {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let mut control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(mut edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .unwrap()
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(control.shared_drop_state());
    edit.override_root_for_test(Some(NodeIndex::from_u32(u32::MAX)));

    assert!(matches!(
        edit.commit_with(&mut control),
        Err(CanonicalEditFailure::Edit(CanonicalEditError::Validation(
            _
        )))
    ));
    assert_eq!(control.rollback_observed_state(), 0b111);
    assert_eq!(control.transition_count, 0);
}

#[test]
fn post_transition_error_and_panic_drop_bundle_before_rollback() {
    for poll_behavior in [
        InjectedControlBehavior::Error,
        InjectedControlBehavior::Panic,
    ] {
        let input = scene_with_valid_bounds();
        let root = input.root().unwrap();
        let mut control = TestControl::new();
        let CanonicalNodeTransformEditStart::Changed(mut edit) =
            CanonicalNodeTransformEdit::try_begin_with(
                &input,
                root,
                translated(),
                SceneValidationLimits::default(),
                &mut control,
            )
            .unwrap()
        else {
            panic!("fixture transform must change")
        };
        edit.install_drop_probe(control.shared_drop_state());
        control.after_transition_poll = poll_behavior;

        match poll_behavior {
            InjectedControlBehavior::Error => assert!(matches!(
                edit.commit_with(&mut control),
                Err(CanonicalEditFailure::Control(TestControlError::Injected))
            )),
            InjectedControlBehavior::Panic => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = edit.commit_with(&mut control);
                }));
                assert!(result.is_err());
            }
            InjectedControlBehavior::Continue => unreachable!(),
        }
        assert_eq!(control.transition_count, 1);
        assert_eq!(control.rollback_observed_state(), 0b111);
    }
}

#[test]
fn transition_panic_drops_bundle_before_candidate_rollback() {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let mut control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(mut edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .unwrap()
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(control.shared_drop_state());
    control.panic_during_transition = true;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = edit.commit_with(&mut control);
    }));
    assert!(result.is_err());
    assert_eq!(control.rollback_observed_state(), 0b111);
}

fn committed_with_drop_probe(
    control: &mut TestControl,
) -> super::CanonicalEditCommit<TestReservation> {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let CanonicalNodeTransformEditStart::Changed(mut edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            control,
        )
        .unwrap()
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(control.shared_drop_state());
    edit.commit_with(control).unwrap()
}

#[test]
fn dropping_a_commit_drops_publication_storage_before_its_reservation() {
    let mut control = TestControl::new();
    let committed = committed_with_drop_probe(&mut control);
    let drops_before_commit = control.reservation_drop_count();

    drop(committed);

    assert_eq!(control.reservation_drop_count(), drops_before_commit + 1);
    assert_eq!(control.rollback_observed_state(), 0b111);
}

#[test]
fn publication_error_and_panic_drop_bundle_before_reservation_rollback() {
    let mut error_control = TestControl::new();
    let committed = committed_with_drop_probe(&mut error_control);
    let result: Result<CanonicalEditPublication, TestControlError> =
        committed.try_publish(|_| Err(TestControlError::Injected));
    assert!(matches!(result, Err(TestControlError::Injected)));
    assert_eq!(error_control.rollback_observed_state(), 0b111);

    let mut panic_control = TestControl::new();
    let committed = committed_with_drop_probe(&mut panic_control);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = committed.try_publish(|_| -> Result<(), TestControlError> {
            panic!("injected publication panic")
        });
    }));
    assert!(result.is_err());
    assert_eq!(panic_control.rollback_observed_state(), 0b111);
}
