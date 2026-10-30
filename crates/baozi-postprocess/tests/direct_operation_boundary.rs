mod common;

use baozi_core::{PrimitiveId, TopologyKind, ValidatedScene};
use baozi_postprocess::{
    PostProcessCancellationOrigin, PostProcessCancellationToken, PostProcessControl,
    PostProcessControlCause, PostProcessOperation, PostProcessReport, PostProcessResourceEvidence,
    PostProcessRunDisposition, PostProcessRunError, PostProcessRunFailure, PostProcessRunOutcome,
    TriangulationCause, TriangulationFailure, TriangulationRemap, triangulate_primitive,
};
use common::{canonical_quad_import, canonical_triangle_import};
use std::error::Error;
use std::time::{Duration, Instant};

#[test]
fn unchanged_operation_preserves_input_identity_and_import_evidence() {
    let (scene, evidence) = canonical_triangle_import().into_parts();
    let generation = scene.generation();
    let blocks = scene.canonical_block_count();
    let target = primitive_id(&scene);

    let success = triangulate_primitive(scene, target, PostProcessControl::default())
        .expect("an existing triangle list should be unchanged");
    assert_eq!(success.disposition(), PostProcessRunDisposition::Unchanged);
    let (scene, report) = success.into_parts();

    assert_eq!(scene.generation(), generation);
    assert_eq!(scene.canonical_block_count(), blocks);
    assert!(evidence.matches_scene(&scene));
    assert_eq!(evidence.scene_generation(), generation);
    assert_eq!(
        evidence.ledger().committed_canonical_members(),
        u64::try_from(blocks).expect("canonical block count should fit the ledger scalar")
    );
    assert_eq!(
        report.operation(),
        PostProcessOperation::Triangulate { target }
    );
    assert_eq!(report.input_generation(), generation);
    assert_eq!(report.output_generation(), generation);
    assert!(report.matches_scene(&scene));
    assert!(report.triangulation_remap().is_none());
    assert_no_canonical_write(report.resources());
    assert!(report.resources().consumed_work_items() > 0);

    drop(scene);
    drop(evidence);
    assert_eq!(report.input_generation(), generation);
    assert!(report.resources().peak_evidence_bytes() > 0);
    assert_eq!(report.resources().published_outcome_evidence_bytes(), 0);
    assert_eq!(report.resources().published_terminal_evidence_records(), 1);
}

#[test]
fn expired_deadline_returns_the_original_scene_and_terminal_control_evidence() {
    let (scene, evidence) = canonical_quad_import().into_parts();
    let generation = scene.generation();
    let target = primitive_id(&scene);
    let deadline = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .expect("the monotonic clock should have at least one second of history");

    let failure = triangulate_primitive(
        scene,
        target,
        PostProcessControl::default().with_deadline(deadline),
    )
    .expect_err("an expired deadline must stop the direct operation");
    assert_eq!(failure.input().generation(), generation);
    let (scene, error) = failure.into_parts();

    assert_eq!(scene.generation(), generation);
    assert!(evidence.matches_scene(&scene));
    assert_eq!(
        scene.primitive(target).unwrap().topology(),
        TopologyKind::Polygons
    );
    let PostProcessRunError::Triangulation { failure } = error else {
        panic!("a started operation must retain its triangulation failure");
    };
    assert!(matches!(
        failure.cause(),
        TriangulationCause::Control {
            cause: PostProcessControlCause::DeadlineExceeded
        }
    ));
    assert_no_canonical_write(
        failure
            .resources()
            .expect("deadline rejection must retain real terminal evidence"),
    );
}

#[test]
fn reports_and_import_evidence_reject_other_generations() {
    let (scene_a, evidence_a) = canonical_triangle_import().into_parts();
    let (scene_b, evidence_b) = canonical_triangle_import().into_parts();
    let target_a = primitive_id(&scene_a);
    let target_b = primitive_id(&scene_b);
    assert_ne!(scene_a.generation(), scene_b.generation());
    assert!(!evidence_a.matches_scene(&scene_b));
    assert!(!evidence_b.matches_scene(&scene_a));

    let success_a = triangulate_primitive(scene_a, target_a, PostProcessControl::default())
        .expect("the first triangle list should be unchanged");
    let success_b = triangulate_primitive(scene_b, target_b, PostProcessControl::default())
        .expect("the second triangle list should be unchanged");

    assert_ne!(
        success_a.report().input_generation(),
        success_b.report().input_generation()
    );
    assert!(evidence_a.matches_scene(success_a.scene()));
    assert!(evidence_b.matches_scene(success_b.scene()));
    assert!(success_a.report().matches_scene(success_a.scene()));
    assert!(success_b.report().matches_scene(success_b.scene()));
    assert!(!success_a.report().matches_scene(success_b.scene()));
    assert!(!success_b.report().matches_scene(success_a.scene()));
}

#[test]
fn public_direct_operation_types_preserve_thread_transfer_contracts() {
    fn assert_send<T: Send>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<PostProcessOperation>();
    assert_send_sync::<PostProcessCancellationToken>();
    assert_send_sync::<PostProcessControlCause>();
    assert_send_sync::<PostProcessReport>();
    assert_send_sync::<PostProcessResourceEvidence>();
    assert_send_sync::<PostProcessRunError>();
    assert_send_sync::<TriangulationCause>();
    assert_send_sync::<TriangulationFailure>();
    assert_send_sync::<TriangulationRemap>();
    assert_send_sync::<baozi_postprocess::NormalMode>();
    assert_send_sync::<baozi_postprocess::NormalOptions>();
    assert_send_sync::<baozi_postprocess::NormalCause>();
    assert_send_sync::<baozi_postprocess::NormalFailure>();
    assert_send_sync::<baozi_postprocess::NormalGenerationReport>();
    assert_send::<PostProcessControl>();
    assert_send::<PostProcessRunFailure>();
    assert_send::<PostProcessRunOutcome>();
}

#[test]
#[allow(
    clippy::result_large_err,
    reason = "thread transfer exercises the public inline original-Scene failure owner"
)]
fn direct_operation_and_detached_report_can_move_between_threads() {
    let (scene, evidence) = canonical_quad_import().into_parts();
    let target = primitive_id(&scene);
    let control = PostProcessControl::default();

    let success = std::thread::spawn(move || triangulate_primitive(scene, target, control))
        .join()
        .expect("the direct-operation thread must not panic")
        .expect("the quad should triangulate on another thread");
    assert_eq!(success.disposition(), PostProcessRunDisposition::Changed);
    assert!(!evidence.matches_scene(success.scene()));
    let (scene, report) = success.into_parts();
    let output_generation = scene.generation();
    drop(scene);
    drop(evidence);

    std::thread::spawn(move || {
        assert_eq!(
            report.operation(),
            PostProcessOperation::Triangulate { target }
        );
        assert_eq!(report.input_generation(), target.generation());
        assert_eq!(report.output_generation(), output_generation);
        let remap = report
            .triangulation_remap()
            .expect("the detached report must retain actual origins");
        assert_eq!(remap.input_target(), target);
        assert_eq!(remap.output_target().generation(), output_generation);
        assert_eq!(remap.source_corner_origins().len(), 6);
        assert_eq!(remap.source_face_triangle_offsets(), &[0, 2]);
        assert!(report.resources().published_outcome_evidence_bytes() > 0);
        assert_eq!(report.resources().current_live_bytes(), 0);
        assert_eq!(report.resources().active_reservations(), 0);
    })
    .join()
    .expect("the detached-evidence thread must not panic");
}

#[test]
fn failure_source_chain_retains_the_direct_cause_and_terminal_evidence() {
    let (scene, evidence) = canonical_quad_import().into_parts();
    let target = primitive_id(&scene);
    let cancellation = PostProcessCancellationToken::new();
    cancellation.cancel();
    let failure = triangulate_primitive(
        scene,
        target,
        PostProcessControl::default().with_cancellation(cancellation),
    )
    .expect_err("pre-cancelled triangulation must fail");

    assert!(evidence.matches_scene(failure.input()));
    let run_error = Error::source(&failure)
        .and_then(|source| source.downcast_ref::<PostProcessRunError>())
        .expect("the input owner must retain the typed invocation error");
    let operation = Error::source(run_error)
        .and_then(|source| source.downcast_ref::<TriangulationFailure>())
        .expect("the invocation error must retain direct-operation evidence");
    let cause = Error::source(operation)
        .and_then(|source| source.downcast_ref::<TriangulationCause>())
        .expect("direct-operation evidence must retain the typed cause");
    assert!(matches!(
        cause,
        TriangulationCause::Control {
            cause: PostProcessControlCause::Cancelled {
                origin: PostProcessCancellationOrigin::Signal
            }
        }
    ));
    assert_no_canonical_write(
        operation
            .resources()
            .expect("ordinary cancellation must retain terminal evidence"),
    );
}

fn primitive_id(scene: &ValidatedScene) -> PrimitiveId {
    scene
        .meshes()
        .next()
        .expect("the fixture must contain a mesh")
        .primitives()
        .next()
        .expect("the fixture must contain a primitive")
        .id()
}

fn assert_no_canonical_write(resources: &PostProcessResourceEvidence) {
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert_eq!(resources.committed_canonical_bytes(), 0);
    assert!(resources.peak_evidence_bytes() > 0);
    assert_eq!(resources.published_outcome_evidence_bytes(), 0);
    assert!(resources.published_terminal_evidence_records() > 0);
    assert!(resources.peak_evidence_bytes() >= resources.current_evidence_bytes());
}
