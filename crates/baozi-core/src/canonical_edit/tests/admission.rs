use super::*;

#[test]
fn commit_expands_the_sparse_candidate_before_materializing_the_output() {
    let input = scene_with_valid_local_and_scene_bounds();
    let root = input.root().expect("fixture root");
    let mut exact_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut exact_control,
        )
        .expect("exact candidate admission should succeed")
    else {
        panic!("fixture transform must change")
    };
    assert_eq!(exact_control.candidate_reservation_bytes(), Some(0));
    assert_eq!(exact_control.candidate_resize_count(), 0);
    let retained = input
        .scene_bounds()
        .derived()
        .retained_producer_allocation_bytes()
        .expect("input scene-bounds producer footprint");
    assert!(retained > 0, "fixture must exercise footprint invalidation");
    let committed = edit
        .commit_with(&mut exact_control)
        .expect("full output admission should succeed");
    let (candidate_bytes, admitted) = exact_control
        .candidate_resize()
        .expect("commit materialization envelope");
    let output_bytes = exact_control
        .transition_required_bytes
        .expect("canonical output bytes");
    assert_eq!(candidate_bytes, 0);
    assert_eq!(exact_control.candidate_resize_count(), 1);
    assert_eq!(
        admitted, output_bytes,
        "the output plan must exclude invalidated scene-bounds producer storage"
    );
    let drops_before_commit = exact_control.reservation_drop_count();
    drop(committed);
    assert_eq!(
        exact_control.reservation_drop_count(),
        drops_before_commit + 1
    );

    let mut one_byte_short = TestControl::new();
    one_byte_short.max_reservation_bytes = Some(admitted - 1);
    let CanonicalNodeTransformEditStart::Changed(edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut one_byte_short,
        )
        .expect("the sparse candidate itself must fit")
    else {
        panic!("fixture transform must change")
    };
    assert!(matches!(
        edit.commit_with(&mut one_byte_short),
        Err(CanonicalEditFailure::Control(TestControlError::Injected))
    ));
    assert_eq!(one_byte_short.candidate_reservation_bytes(), Some(0));
    assert_eq!(one_byte_short.candidate_resize(), Some((0, admitted)));
    assert_eq!(one_byte_short.candidate_resize_count(), 1);
    assert_eq!(one_byte_short.reservation_drop_count(), 1);
}

#[test]
fn scene_boundary_enforces_output_limits_before_resize_or_materialization() {
    let input = scene_with_valid_local_and_scene_bounds();
    let root = input.root().expect("fixture root");

    let mut baseline_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) = input
        .try_edit_node_transform_with(
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut baseline_control,
        )
        .expect("scene boundary should start the transform transaction")
    else {
        panic!("fixture transform must change")
    };
    let committed = edit
        .commit_with(&mut baseline_control)
        .expect("baseline transaction should expose the exact output envelope");
    let admitted = baseline_control
        .candidate_resize()
        .expect("changed transaction must resize before materialization")
        .1;
    assert_eq!(baseline_control.transition_required_bytes, Some(admitted));
    drop(committed);

    let exact_limits = SceneValidationLimits::default().with_max_canonical_bytes(admitted);
    let mut exact_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) = input
        .try_edit_node_transform_with(root, translated(), exact_limits, &mut exact_control)
        .expect("exact output limit should admit the sparse candidate")
    else {
        panic!("fixture transform must change")
    };
    let committed = edit
        .commit_with(&mut exact_control)
        .expect("exact output limit should admit materialization");
    assert_eq!(exact_control.candidate_resize(), Some((0, admitted)));
    drop(committed);

    let below_limits = SceneValidationLimits::default().with_max_canonical_bytes(admitted - 1);
    let mut below_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) = input
        .try_edit_node_transform_with(root, translated(), below_limits, &mut below_control)
        .expect("the sparse candidate should be admitted before output planning")
    else {
        panic!("fixture transform must change")
    };
    let error = match edit.commit_with(&mut below_control) {
        Ok(_) => panic!("one byte below the output envelope must fail before resize"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        CanonicalEditFailure::Edit(CanonicalEditError::LimitExceeded {
            limit: CanonicalValidationLimit::CanonicalBytes,
            actual,
            maximum,
        }) if actual == admitted && maximum == admitted - 1
    ));
    assert_eq!(below_control.candidate_resize_count(), 0);
    assert!(!below_control.materialization_active);
    assert_eq!(below_control.transition_count, 0);
    assert_eq!(below_control.live_scratch_reservations(), 0);
    assert_eq!(below_control.reservation_drop_count(), 1);

    let node_count = input.validation_stats().canonical_counts().nodes;
    assert!(node_count > 0);
    let count_limits = SceneValidationLimits::default().with_max_nodes(node_count - 1);
    let mut count_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) = input
        .try_edit_node_transform_with(root, translated(), count_limits, &mut count_control)
        .expect("unchanged counts are checked at commit after owner validation")
    else {
        panic!("fixture transform must change")
    };
    let poll_before_commit = count_control.poll_count;
    let work_before_commit = count_control.total_work;
    let error = match edit.commit_with(&mut count_control) {
        Ok(_) => panic!("an unchanged count over the caller limit must fail before work"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        CanonicalEditFailure::Edit(CanonicalEditError::LimitExceeded {
            limit: CanonicalValidationLimit::Nodes,
            actual,
            maximum,
        }) if actual == node_count && maximum == node_count - 1
    ));
    assert_eq!(count_control.poll_count, poll_before_commit);
    assert_eq!(count_control.total_work, work_before_commit);
    assert_eq!(count_control.candidate_resize_count(), 0);
    assert_eq!(count_control.transition_count, 0);
    assert_eq!(count_control.reservation_drop_count(), 1);
}

#[test]
fn commit_admission_covers_three_primitive_rebind_growth_before_allocation() {
    let input = scene_with_three_primitives_and_absent_scene_bounds();
    let root = input.root().expect("fixture root");
    let mut exact_control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut exact_control,
        )
        .expect("three-primitive candidate admission should cover rebind growth")
    else {
        panic!("fixture transform must change")
    };
    assert_eq!(exact_control.candidate_reservation_bytes(), Some(0));
    assert_eq!(exact_control.candidate_resize_count(), 0);

    let mut planning_control = TestControl::new();
    let mut planning_context =
        CanonicalEditCloneContext::new(SceneValidationLimits::default(), &mut planning_control);
    let edit_plan = CanonicalEditPlan::try_new(
        &input,
        CanonicalInvalidations {
            scene_bounds: Some(DerivedAbsenceReason::Invalidated),
        },
        &mut planning_context,
    )
    .expect("fixture edit plan");
    let envelope_without_rebind_growth = input
        .validation_stats()
        .footprint()
        .accounted_payload_bytes();
    assert_eq!(
        edit_plan
            .output_envelope_bytes
            .checked_sub(envelope_without_rebind_growth)
            .expect("rebind output must grow for this fixture"),
        u64::try_from(std::mem::size_of::<MeshPrimitive>()).unwrap()
    );
    let committed = edit
        .commit_with(&mut exact_control)
        .expect("three-primitive output should commit");
    let (candidate_bytes, admitted) = exact_control
        .candidate_resize()
        .expect("commit materialization envelope");
    assert_eq!(candidate_bytes, 0);
    assert_eq!(
        admitted.checked_sub(envelope_without_rebind_growth),
        Some(u64::try_from(std::mem::size_of::<MeshPrimitive>()).unwrap()),
        "rebuilding a len=capacity=3 primitive Vec requires one additional slot"
    );
    assert_eq!(exact_control.transition_required_bytes, Some(admitted));
    let drops_before_commit = exact_control.reservation_drop_count();
    drop(committed);
    assert_eq!(
        exact_control.reservation_drop_count(),
        drops_before_commit + 1
    );

    let mut one_byte_short = TestControl::new();
    one_byte_short.max_reservation_bytes = Some(admitted - 1);
    let CanonicalNodeTransformEditStart::Changed(edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut one_byte_short,
        )
        .expect("the sparse candidate itself must fit")
    else {
        panic!("fixture transform must change")
    };
    assert!(matches!(
        edit.commit_with(&mut one_byte_short),
        Err(CanonicalEditFailure::Control(TestControlError::Injected))
    ));
    assert_eq!(one_byte_short.candidate_reservation_bytes(), Some(0));
    assert_eq!(one_byte_short.candidate_resize(), Some((0, admitted)));
    assert_eq!(one_byte_short.candidate_resize_count(), 1);
    assert_eq!(
        one_byte_short.clone_work, 2,
        "only the affected replacement and allocation-free rebind plan may run before admission"
    );
    assert_eq!(one_byte_short.reservation_drop_count(), 1);
}

#[test]
fn rejected_candidate_resize_preserves_the_covering_token_until_rollback() {
    let input = scene_with_valid_local_and_scene_bounds();
    let root = input.root().expect("fixture root");
    let mut control = TestControl::new();
    control.reject_resize = true;
    let CanonicalNodeTransformEditStart::Changed(mut edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("the sparse candidate should not resize")
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(control.shared_drop_state());

    assert!(matches!(
        edit.commit_with(&mut control),
        Err(CanonicalEditFailure::Control(TestControlError::Injected))
    ));
    assert_eq!(control.candidate_resize_count(), 1);
    assert!(matches!(control.candidate_resize(), Some((0, bytes)) if bytes > 0));
    assert_eq!(control.reservation_drop_count(), 1);
    assert_eq!(control.rollback_observed_state(), 0b111);
    assert_eq!(
        input.node(root).unwrap().authored_transform(),
        AuthoredTransform::identity_trs()
    );
}

#[test]
fn dropping_a_prepared_edit_drops_candidate_storage_before_its_reservation() {
    let input = scene_with_valid_bounds();
    let root = input.root().expect("fixture root");
    let mut control = TestControl::new();
    let CanonicalNodeTransformEditStart::Changed(mut edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("prepared edit")
    else {
        panic!("fixture transform must change")
    };
    edit.install_drop_probe(control.shared_drop_state());

    drop(edit);

    assert_eq!(control.reservation_drop_count(), 1);
    assert_eq!(control.rollback_observed_state(), 0b111);
}

#[test]
fn post_candidate_reserve_error_and_panic_release_the_sparse_candidate() {
    for behavior in [
        InjectedControlBehavior::Error,
        InjectedControlBehavior::Panic,
    ] {
        let input = scene_with_valid_bounds();
        let root = input.root().expect("fixture root");
        let mut control = TestControl::new();
        control.after_candidate_reserve_poll = behavior;

        match behavior {
            InjectedControlBehavior::Error => assert!(matches!(
                CanonicalNodeTransformEdit::try_begin_with(
                    &input,
                    root,
                    translated(),
                    SceneValidationLimits::default(),
                    &mut control,
                ),
                Err(CanonicalEditFailure::Control(TestControlError::Injected))
            )),
            InjectedControlBehavior::Panic => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = CanonicalNodeTransformEdit::try_begin_with(
                        &input,
                        root,
                        translated(),
                        SceneValidationLimits::default(),
                        &mut control,
                    );
                }));
                assert!(result.is_err());
            }
            InjectedControlBehavior::Continue => unreachable!(),
        }
        assert_eq!(control.reservation_drop_count(), 1);
        assert_eq!(control.candidate_resize_count(), 0);
        assert_eq!(control.transition_count, 0);
        assert_eq!(control.live_scratch_reservations(), 0);
        assert_eq!(
            input.node(root).unwrap().authored_transform(),
            AuthoredTransform::identity_trs()
        );
    }
}

#[test]
fn planning_and_materialization_charge_failures_roll_back_the_candidate() {
    for (phase, behavior) in [
        (CloneWorkPhase::Planning, InjectedControlBehavior::Error),
        (CloneWorkPhase::Planning, InjectedControlBehavior::Panic),
        (
            CloneWorkPhase::Materialization,
            InjectedControlBehavior::Error,
        ),
        (
            CloneWorkPhase::Materialization,
            InjectedControlBehavior::Panic,
        ),
    ] {
        let input = scene_with_valid_local_and_scene_bounds();
        let root = input.root().expect("fixture root");
        let mut control = TestControl::new();
        let CanonicalNodeTransformEditStart::Changed(mut edit) =
            CanonicalNodeTransformEdit::try_begin_with(
                &input,
                root,
                translated(),
                SceneValidationLimits::default(),
                &mut control,
            )
            .expect("the sparse candidate should begin")
        else {
            panic!("fixture transform must change")
        };
        edit.install_drop_probe(control.shared_drop_state());
        control.clone_work_failure = Some((phase, behavior));

        match behavior {
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
        assert_eq!(
            control.candidate_resize_count(),
            usize::from(phase == CloneWorkPhase::Materialization)
        );
        assert_eq!(control.transition_count, 0);
        assert_eq!(control.reservation_drop_count(), 1);
        assert_eq!(control.rollback_observed_state(), 0b111);
        assert_eq!(control.live_scratch_reservations(), 0);
        assert_eq!(
            input.node(root).unwrap().authored_transform(),
            AuthoredTransform::identity_trs()
        );
    }
}

#[test]
fn materialization_interval_poll_failures_roll_back_before_validation() {
    let node_count =
        usize::try_from(SCENE_VALIDATION_POLL_INTERVAL).expect("poll interval must fit usize") + 1;
    for behavior in [
        InjectedControlBehavior::Error,
        InjectedControlBehavior::Panic,
    ] {
        let input = scene_with_many_nodes(node_count);
        let root = input.root().expect("fixture root");
        let mut control = TestControl::new();
        let CanonicalNodeTransformEditStart::Changed(mut edit) =
            CanonicalNodeTransformEdit::try_begin_with(
                &input,
                root,
                translated(),
                SceneValidationLimits::default(),
                &mut control,
            )
            .expect("the sparse candidate should begin")
        else {
            panic!("fixture transform must change")
        };
        edit.install_drop_probe(control.shared_drop_state());
        control.materialization_interval_poll = behavior;

        match behavior {
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
        assert_eq!(
            control.materialization_clone_work,
            SCENE_VALIDATION_POLL_INTERVAL
        );
        assert_eq!(control.transition_count, 0);
        assert_eq!(control.reservation_drop_count(), 1);
        assert_eq!(control.rollback_observed_state(), 0b111);
        assert_eq!(control.live_scratch_reservations(), 0);
    }
}

#[test]
fn central_validation_scratch_poll_failures_release_scratch_before_candidate_rollback() {
    let node_count =
        usize::try_from(SCENE_VALIDATION_POLL_INTERVAL).expect("poll interval must fit usize") + 1;
    for behavior in [
        InjectedControlBehavior::Error,
        InjectedControlBehavior::Panic,
    ] {
        let input = scene_with_many_nodes(node_count);
        let root = input.root().expect("fixture root");
        let mut control = TestControl::new();
        let CanonicalNodeTransformEditStart::Changed(mut edit) =
            CanonicalNodeTransformEdit::try_begin_with(
                &input,
                root,
                translated(),
                SceneValidationLimits::default(),
                &mut control,
            )
            .expect("the sparse candidate should begin")
        else {
            panic!("fixture transform must change")
        };
        edit.install_drop_probe(control.shared_drop_state());
        control.scratch_poll = behavior;

        match behavior {
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
        assert_eq!(control.transition_count, 0);
        assert_eq!(control.live_scratch_reservations(), 0);
        assert_eq!(control.rollback_observed_state(), 0b111);
        assert!(control.reservation_drop_count() >= 2);
    }
}

#[test]
fn post_resize_error_and_panic_drop_candidate_before_rollback() {
    for poll_behavior in [
        InjectedControlBehavior::Error,
        InjectedControlBehavior::Panic,
    ] {
        let input = scene_with_valid_bounds();
        let root = input.root().expect("fixture root");
        let mut control = TestControl::new();
        control.after_resize_poll = poll_behavior;
        let CanonicalNodeTransformEditStart::Changed(mut edit) =
            CanonicalNodeTransformEdit::try_begin_with(
                &input,
                root,
                translated(),
                SceneValidationLimits::default(),
                &mut control,
            )
            .expect("the sparse candidate should begin")
        else {
            panic!("fixture transform must change")
        };
        edit.install_drop_probe(control.shared_drop_state());

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
        assert_eq!(control.candidate_resize_count(), 1);
        assert_eq!(control.reservation_drop_count(), 1);
        assert_eq!(control.rollback_observed_state(), 0b111);
    }
}

#[test]
fn unchanged_remaps_do_not_materialize_large_endpoint_universes() {
    const VERTEX_COUNT: usize = 512;
    let input = scene_with_many_vertices(VERTEX_COUNT);
    let root = input.root().expect("fixture root");
    let mut control = TestControl::new();

    let CanonicalNodeTransformEditStart::Changed(edit) =
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("large unchanged domains should not require materialized remaps")
    else {
        panic!("fixture transform must change")
    };
    assert_eq!(control.candidate_reservation_bytes(), Some(0));
    assert!(control.poll_count > 1);
    assert_eq!(control.total_work, 1);
    assert_eq!(control.candidate_resize_count(), 0);
    let committed = edit
        .commit_with(&mut control)
        .expect("large unchanged domains should commit");
    assert_eq!(committed.publication.remaps.retained_allocation_bytes(), 0);
    assert_eq!(control.candidate_resize_count(), 1);
    let drops_before_commit = control.reservation_drop_count();
    drop(committed);
    assert_eq!(control.reservation_drop_count(), drops_before_commit + 1);
}

#[test]
fn sibling_edits_from_one_input_publish_distinct_generations() {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let begin = || {
        let CanonicalNodeTransformEditStart::Changed(edit) = CanonicalNodeTransformEdit::try_begin(
            &input,
            root,
            translated(),
            SceneValidationLimits::default(),
        )
        .unwrap() else {
            panic!("fixture transform must change")
        };
        edit
    };

    let first = begin().commit().unwrap().into_unreserved_publication();
    let second = begin().commit().unwrap().into_unreserved_publication();
    let first_generation = first.scene().generation();
    let second_generation = second.scene().generation();

    assert_ne!(first_generation, second_generation);
    assert!(matches!(
        first.scene().node(root),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        second.scene().node(root),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    assert!(first.scene().node(root.rebind(first_generation)).is_ok());
    assert!(second.scene().node(root.rebind(second_generation)).is_ok());
}

#[test]
fn unchanged_and_invalid_handles_never_reserve_candidate_storage() {
    let input = scene_with_valid_bounds();
    let root = input.root().unwrap();
    let mut unchanged_control = TestControl::new();
    let unchanged = CanonicalNodeTransformEdit::try_begin_with(
        &input,
        root,
        AuthoredTransform::identity_trs(),
        SceneValidationLimits::default(),
        &mut unchanged_control,
    )
    .unwrap();
    assert!(matches!(
        unchanged,
        CanonicalNodeTransformEditStart::Unchanged
    ));
    assert!(unchanged_control.reserve_kinds.is_empty());

    let stale = NodeId::new(SceneGenerationId::allocate().unwrap(), 0);
    let mut stale_control = TestControl::new();
    assert!(
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            stale,
            translated(),
            SceneValidationLimits::default(),
            &mut stale_control,
        )
        .is_err()
    );
    assert!(stale_control.reserve_kinds.is_empty());

    let out_of_range = NodeId::new(input.generation(), u32::MAX);
    let mut range_control = TestControl::new();
    assert!(
        CanonicalNodeTransformEdit::try_begin_with(
            &input,
            out_of_range,
            translated(),
            SceneValidationLimits::default(),
            &mut range_control,
        )
        .is_err()
    );
    assert!(range_control.reserve_kinds.is_empty());
}
