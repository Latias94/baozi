mod common;

use baozi_postprocess::{
    CompiledPostProcessPlan, CompiledPostProcessRun, PostProcessCancellationOrigin,
    PostProcessCancellationToken, PostProcessCompileFailure, PostProcessControl,
    PostProcessControlCause, PostProcessError, PostProcessExecutionFailure, PostProcessFailure,
    PostProcessReport, PostProcessRequests, PostProcessResourceEvidence, PostProcessRunDisposition,
    PostProcessRunError, PostProcessRunOutcome,
};
use common::{canonical_triangle_import, canonical_triangle_scene};
use std::error::Error;
use std::time::{Duration, Instant};

#[test]
fn empty_requested_run_returns_the_original_owner_and_generation_bound_evidence() {
    let scene = canonical_triangle_scene();
    let generation = scene.generation();

    let compiled = PostProcessRequests::empty().compile(scene).unwrap();
    assert_eq!(compiled.plan().input_generation(), generation);
    assert!(compiled.plan().is_empty());
    assert!(compiled.plan().steps().is_empty());
    assert_eq!(compiled.plan().step_count(), 0);
    assert!(!compiled.plan().requires_canonical_write());

    let success = compiled.execute().unwrap();
    let report = success.report();

    assert_eq!(success.disposition(), PostProcessRunDisposition::Unchanged);
    assert_eq!(success.scene().generation(), generation);
    assert_eq!(report.preserved_generation(), generation);
    assert!(report.matches_scene(success.scene()));
    assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
    assert!(report.plan().is_empty());
    assert!(report.step_outcomes().is_empty());
    assert!(report.diagnostics().is_empty());
    assert!(report.losses().is_empty());
    assert_quiescent_unchanged_resources(report.resources());
}

#[test]
fn success_parts_keep_the_scene_and_report_from_one_invocation() {
    let scene = canonical_triangle_scene();
    let generation = scene.generation();

    let success = PostProcessRequests::empty()
        .compile(scene)
        .unwrap()
        .execute()
        .unwrap();
    let (scene, report) = success.into_parts();

    assert_eq!(scene.generation(), generation);
    assert!(report.matches_scene(&scene));
    assert_eq!(report.preserved_generation(), generation);
    assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
    assert!(report.plan().is_empty());
    assert_quiescent_unchanged_resources(report.resources());
}

#[test]
fn pre_cancelled_compile_returns_the_original_scene_and_terminal_control_evidence() {
    let scene = canonical_triangle_scene();
    let generation = scene.generation();
    let cancellation = PostProcessCancellationToken::new();
    cancellation.cancel();
    let control = PostProcessControl::default().with_cancellation(cancellation);

    let failure = PostProcessRequests::empty()
        .compile_with_control(scene, control)
        .unwrap_err();
    let (scene, error) = failure.into_parts();
    let failure = error
        .into_operation_failure()
        .expect("coordinator should initialize before control polling");

    assert_eq!(scene.generation(), generation);
    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            cause: PostProcessControlCause::Cancelled {
                origin: PostProcessCancellationOrigin::Signal
            }
        }
    ));
    assert_quiescent_unchanged_resources(failure.resources());
}

#[test]
fn cancellation_between_compile_and_execute_returns_the_original_scene() {
    let (scene, evidence) = canonical_triangle_import().into_parts();
    let generation = scene.generation();
    let cancellation = PostProcessCancellationToken::new();
    let control = PostProcessControl::default().with_cancellation(cancellation.clone());
    let compiled = PostProcessRequests::empty()
        .compile_with_control(scene, control)
        .unwrap();

    cancellation.cancel();
    let failure = compiled.execute().unwrap_err();
    let (scene, failure) = failure.into_parts();

    assert_eq!(scene.generation(), generation);
    assert!(evidence.matches_scene(&scene));
    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            cause: PostProcessControlCause::Cancelled {
                origin: PostProcessCancellationOrigin::Signal
            }
        }
    ));
    assert_quiescent_unchanged_resources(failure.resources());
}

#[test]
fn expired_deadline_returns_the_original_scene_and_terminal_control_evidence() {
    let (scene, evidence) = canonical_triangle_import().into_parts();
    let generation = scene.generation();
    let deadline = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .expect("the monotonic clock should have at least one second of history");

    let failure = PostProcessRequests::empty()
        .compile_with_control(scene, PostProcessControl::default().with_deadline(deadline))
        .unwrap_err();
    let (scene, error) = failure.into_parts();
    let failure = error
        .into_operation_failure()
        .expect("coordinator should initialize before deadline polling");

    assert_eq!(scene.generation(), generation);
    assert!(evidence.matches_scene(&scene));
    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            cause: PostProcessControlCause::DeadlineExceeded
        }
    ));
    assert_quiescent_unchanged_resources(failure.resources());
}

#[test]
fn import_evidence_remains_bound_after_scene_moves_through_postprocess() {
    let import = canonical_triangle_import();
    let (scene, evidence) = import.into_parts();
    let generation = scene.generation();

    assert_eq!(evidence.scene_generation(), generation);
    assert!(evidence.matches_scene(&scene));

    let success = PostProcessRequests::empty()
        .compile(scene)
        .unwrap()
        .execute()
        .unwrap();

    assert!(evidence.matches_scene(success.scene()));
    assert_eq!(success.report().preserved_generation(), generation);
    assert!(success.report().matches_scene(success.scene()));
    assert_eq!(
        evidence.ledger().committed_canonical_members(),
        u64::try_from(success.scene().canonical_block_count())
            .expect("canonical block count should fit the ledger scalar")
    );
}

#[test]
fn reports_and_import_evidence_cannot_be_reinterpreted_across_generations() {
    let (scene_a, evidence_a) = canonical_triangle_import().into_parts();
    let (scene_b, evidence_b) = canonical_triangle_import().into_parts();
    assert_ne!(scene_a.generation(), scene_b.generation());
    assert!(evidence_a.matches_scene(&scene_a));
    assert!(evidence_b.matches_scene(&scene_b));
    assert!(!evidence_a.matches_scene(&scene_b));
    assert!(!evidence_b.matches_scene(&scene_a));

    let success_a = PostProcessRequests::empty()
        .compile(scene_a)
        .unwrap()
        .execute()
        .unwrap();
    let success_b = PostProcessRequests::empty()
        .compile(scene_b)
        .unwrap()
        .execute()
        .unwrap();

    assert_ne!(
        success_a.report().preserved_generation(),
        success_b.report().preserved_generation()
    );
    assert!(success_a.report().matches_scene(success_a.scene()));
    assert!(success_b.report().matches_scene(success_b.scene()));
    assert!(!success_a.report().matches_scene(success_b.scene()));
    assert!(!success_b.report().matches_scene(success_a.scene()));
}

#[test]
fn public_result_types_preserve_the_required_thread_transfer_contracts() {
    fn assert_send<T: Send>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<CompiledPostProcessPlan>();
    assert_send_sync::<PostProcessCancellationToken>();
    assert_send_sync::<PostProcessControlCause>();
    assert_send_sync::<PostProcessError>();
    assert_send_sync::<PostProcessFailure>();
    assert_send_sync::<PostProcessReport>();
    assert_send_sync::<PostProcessRequests>();
    assert_send_sync::<PostProcessResourceEvidence>();
    assert_send_sync::<PostProcessRunError>();
    assert_send::<PostProcessControl>();
    assert_send::<CompiledPostProcessRun>();
    assert_send::<PostProcessCompileFailure>();
    assert_send::<PostProcessExecutionFailure>();
    assert_send::<PostProcessRunOutcome>();
}

#[test]
fn failure_source_chain_retains_terminal_operation_evidence() {
    let cancellation = PostProcessCancellationToken::new();
    cancellation.cancel();
    let failure = PostProcessRequests::empty()
        .compile_with_control(
            canonical_triangle_scene(),
            PostProcessControl::default().with_cancellation(cancellation),
        )
        .unwrap_err();

    let run_error = Error::source(&failure)
        .and_then(|source| source.downcast_ref::<PostProcessRunError>())
        .expect("compile failure should retain the typed run error");
    let operation = Error::source(run_error)
        .and_then(|source| source.downcast_ref::<PostProcessFailure>())
        .expect("run error should retain terminal operation evidence");
    assert!(matches!(
        operation.error(),
        PostProcessError::Control { .. }
    ));
}

fn assert_quiescent_unchanged_resources(resources: &PostProcessResourceEvidence) {
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.peak_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert_eq!(resources.attempted_work_items(), 0);
    assert_eq!(resources.consumed_work_items(), 0);
    assert_eq!(resources.peak_staging_bytes(), 0);
    assert_eq!(resources.committed_canonical_bytes(), 0);
}
