use baozi_core::{
    Aabb, Mesh, MeshBinding, Node, PrimitiveTopology, ValidatedScene, Vec3, validate_scene,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, CancellationToken, ControlCause, ProgressDecision,
    ProgressDeliveryLimits, ProgressEvent,
};
use baozi_postprocess::{
    PostProcessControl, PostProcessDimension, PostProcessError, PostProcessFailure,
    PostProcessLedgerPhase, PostProcessLedgerSnapshot, PostProcessLimits, PostProcessPipeline,
    PostProcessProgressPhase, PostProcessProgressUnit, PostProcessReport,
    PostProcessRunDisposition, PostProcessRunError, PostProcessRunOutcome, PostProcessStep,
};
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct CancelOnSecondPoll {
    polls: Arc<AtomicUsize>,
}

impl CancelOnSecondPoll {
    fn new() -> Self {
        Self {
            polls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn polls(&self) -> usize {
        self.polls.load(Ordering::Acquire)
    }
}

impl CancellationSignal for CancelOnSecondPoll {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::AcqRel) >= 1
    }
}

#[test]
fn partial_staging_is_rolled_back_on_step_failure() {
    let scene = scene_with_meshes([
        triangle_mesh(),
        Mesh {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
            ],
            indices: vec![0, 1, 2],
            ..Mesh::default()
        },
    ]);
    let generation = scene.generation();

    let failure = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run(&scene)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert_eq!(scene.generation(), generation);
    assert!(scene.meshes()[0].normals.is_empty());
    assert!(scene.meshes()[1].normals.is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(failure.ledger().peak_live_bytes() > 0);
}

#[test]
fn pre_cancelled_operation_returns_a_typed_control_source() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let control = PostProcessControl::default().with_cancellation(cancellation);

    let run_error = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap_err();
    assert!(
        Error::source(&run_error)
            .and_then(|source| source.downcast_ref::<PostProcessFailure>())
            .is_some()
    );
    let failure = run_error
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            source: ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        }
    ));
    let domain_source = Error::source(&failure)
        .and_then(|source| source.downcast_ref::<PostProcessError>())
        .expect("failure source should retain the post-process error");
    assert!(
        Error::source(domain_source)
            .and_then(|source| source.downcast_ref::<ControlCause>())
            .is_some()
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(scene.meshes()[0].normals.is_empty());
}

#[test]
fn observer_panic_discards_live_staging() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let control = PostProcessControl::default().with_progress_observer(
        |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
            if event.phase() == PostProcessProgressPhase::Transform && event.completed() >= 2 {
                panic!("observer failure");
            }
            ProgressDecision::Continue
        },
    );

    let failure = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            source: ControlCause::ObserverPanicked
        }
    ));
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(failure.ledger().peak_live_bytes() > 0);
    assert!(scene.meshes()[0].normals.is_empty());
}

#[test]
fn observer_cancellation_stops_transform_work_and_rolls_back() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let generation = scene.generation();
    let control = PostProcessControl::default().with_progress_observer(
        |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
            if event.phase() == PostProcessProgressPhase::Transform && event.completed() >= 2 {
                ProgressDecision::Cancel
            } else {
                ProgressDecision::Continue
            }
        },
    );

    let failure = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            source: ControlCause::Cancelled {
                origin: CancellationOrigin::Observer
            }
        }
    ));
    assert_eq!(scene.generation(), generation);
    assert!(scene.meshes()[0].normals.is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(failure.ledger().peak_live_bytes() > 0);
}

#[test]
fn cancellation_after_validation_does_not_publish_the_candidate() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let generation = scene.generation();
    let control = PostProcessControl::default().with_progress_observer(
        |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
            if event.phase() == PostProcessProgressPhase::Validate && event.completed() == 1 {
                ProgressDecision::Cancel
            } else {
                ProgressDecision::Continue
            }
        },
    );

    let failure = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            source: ControlCause::Cancelled {
                origin: CancellationOrigin::Observer
            }
        }
    ));
    assert_eq!(scene.generation(), generation);
    assert!(scene.meshes()[0].normals.is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(
        failure
            .ledger()
            .phase(PostProcessLedgerPhase::ValidationCommit)
            .is_some_and(|phase| phase.peak_live_bytes() > 0)
    );
}

#[test]
fn final_observer_cancellation_bypasses_exhausted_transient_quota_and_rolls_back() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let generation = scene.generation();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&events);
    let control = PostProcessControl::default()
        .with_progress_limits(ProgressDeliveryLimits::new(1, usize::MAX, 0))
        .with_progress_observer(
            move |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
                observed
                    .lock()
                    .unwrap()
                    .push((event.phase(), event.completed()));
                if event.phase() == PostProcessProgressPhase::Validate && event.completed() == 1 {
                    ProgressDecision::Cancel
                } else {
                    ProgressDecision::Continue
                }
            },
        );

    let failure = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            source: ControlCause::Cancelled {
                origin: CancellationOrigin::Observer
            }
        }
    ));
    assert_eq!(scene.generation(), generation);
    assert!(scene.meshes()[0].normals.is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(
        failure
            .ledger()
            .dimension(PostProcessDimension::CanonicalBytes)
            .expect("canonical dimension should be frozen")
            .committed(),
        0
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            (PostProcessProgressPhase::Transform, 0),
            (PostProcessProgressPhase::Validate, 1),
        ]
    );
}

#[test]
fn cancellation_is_polled_inside_central_validation() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let cancellation = CancellationToken::new();
    let observer_cancellation = cancellation.clone();
    let control = PostProcessControl::default()
        .with_cancellation(cancellation)
        .with_progress_observer(
            move |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
                if event.phase() == PostProcessProgressPhase::Validate && event.completed() == 0 {
                    observer_cancellation.cancel();
                }
                ProgressDecision::Continue
            },
        );

    let failure = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            source: ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        }
    ));
    assert!(scene.meshes()[0].normals.is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}

#[test]
fn progress_is_monotonic_across_distinct_owner_phases() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&events);
    let control = PostProcessControl::default().with_progress_observer(
        move |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
            observed.lock().unwrap().push((
                event.phase(),
                event.completed(),
                event.total(),
                event.sequence(),
            ));
            ProgressDecision::Continue
        },
    );

    let success = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap();
    let ledger = success.report().ledger();

    let committed = ledger
        .dimension(PostProcessDimension::CanonicalBytes)
        .expect("canonical dimension should be frozen")
        .committed();
    assert_eq!(
        committed,
        success.scene().footprint().accounted_payload_bytes()
    );
    assert!(ledger.peak_live_bytes() > committed);
    assert_eq!(ledger.current_live_bytes(), 0);

    let events = events.lock().unwrap();
    assert!(!events.is_empty());
    for pair in events.windows(2) {
        let (left_phase, left_completed, _, left_sequence) = pair[0];
        let (right_phase, right_completed, _, right_sequence) = pair[1];
        assert!(right_sequence > left_sequence);
        assert!(phase_order(right_phase) >= phase_order(left_phase));
        if right_phase == left_phase {
            assert!(right_completed >= left_completed);
        }
    }
}

#[test]
fn success_snapshot_retains_the_complete_accounting_phase_catalog() {
    let scene = scene_with_meshes([triangle_mesh()]);

    let success = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run(&scene)
        .unwrap();
    let ledger = success.report().ledger();

    assert_eq!(
        ledger.phases().map(|(phase, _)| phase).collect::<Vec<_>>(),
        PostProcessLedgerPhase::ALL
    );
    let plan_preflight = ledger
        .phase(PostProcessLedgerPhase::PlanPreflight)
        .expect("the frozen plan-preflight phase must remain queryable");
    assert_eq!(plan_preflight.current_live_bytes(), 0);
    assert_eq!(plan_preflight.peak_live_bytes(), 0);
    assert!(
        ledger
            .phase(PostProcessLedgerPhase::TransactionStaging)
            .is_some_and(|phase| phase.peak_live_bytes() > 0)
    );
    assert!(
        ledger
            .phase(PostProcessLedgerPhase::ValidationCommit)
            .is_some_and(|phase| phase.peak_live_bytes() > 0)
    );
    assert!(
        ledger
            .phase(PostProcessLedgerPhase::OutcomeFinalization)
            .is_some()
    );
}

#[test]
fn empty_plan_preserves_the_input_generation() {
    let scene = scene_with_meshes([triangle_mesh()]);

    let outcome = PostProcessPipeline::new([]).unwrap().run(&scene).unwrap();
    let ledger = outcome.report().ledger();

    assert_eq!(outcome.disposition(), PostProcessRunDisposition::Unchanged);
    assert_eq!(outcome.report().disposition(), outcome.disposition());
    assert!(std::ptr::eq(outcome.scene(), &scene));
    assert_eq!(outcome.scene().generation(), scene.generation());
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(ledger.peak_live_bytes(), 0);
    assert_eq!(
        ledger
            .dimension(PostProcessDimension::CanonicalBytes)
            .expect("canonical dimension should be frozen")
            .committed(),
        0
    );
    assert_eq!(
        ledger
            .phase(PostProcessLedgerPhase::TransactionStaging)
            .expect("transaction-staging phase should be frozen")
            .peak_live_bytes(),
        0
    );
    assert_eq!(
        ledger
            .phase(PostProcessLedgerPhase::ValidationCommit)
            .expect("validation-commit phase should be frozen")
            .peak_live_bytes(),
        0
    );
}

#[test]
fn unchanged_outcome_rejects_owned_extraction_without_losing_evidence() {
    let scene = scene_with_meshes([triangle_mesh()]);

    let outcome = PostProcessPipeline::new([])
        .unwrap()
        .run(&scene)
        .unwrap()
        .into_changed()
        .expect_err("an unchanged invocation cannot yield an owned generation");

    assert_eq!(outcome.disposition(), PostProcessRunDisposition::Unchanged);
    assert_eq!(outcome.report().disposition(), outcome.disposition());
    assert!(std::ptr::eq(outcome.scene(), &scene));
    assert_eq!(outcome.report().ledger().current_live_bytes(), 0);
}

#[test]
fn unchanged_report_can_outlive_the_run_borrow() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let generation = scene.generation();

    let report = PostProcessPipeline::new([])
        .unwrap()
        .run(&scene)
        .unwrap()
        .into_unchanged_report()
        .expect("an unchanged run should yield its owned report");
    let moved_scene = scene;

    assert_eq!(moved_scene.generation(), generation);
    assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn changed_outcome_releases_the_input_borrow_and_retains_evidence() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let input_generation = scene.generation();

    let outcome = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run(&scene)
        .unwrap();
    assert_eq!(outcome.disposition(), PostProcessRunDisposition::Changed);
    let (changed, report) = outcome
        .into_changed()
        .expect("a changed invocation should yield its owned generation");
    drop(scene);

    assert_ne!(changed.generation(), input_generation);
    assert!(!changed.meshes()[0].normals.is_empty());
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(
        report
            .ledger()
            .dimension(PostProcessDimension::CanonicalBytes)
            .expect("canonical dimension should be frozen")
            .committed(),
        changed.footprint().accounted_payload_bytes()
    );
}

#[test]
fn empty_plan_emits_no_progress_and_returns_unchanged() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let generation = scene.generation();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&events);
    let control = PostProcessControl::default().with_progress_observer(
        move |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
            observed.lock().unwrap().push(event.phase());
            if event.phase() == PostProcessProgressPhase::Validate && event.completed() == 1 {
                ProgressDecision::Cancel
            } else {
                ProgressDecision::Continue
            }
        },
    );

    let outcome = PostProcessPipeline::new([])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap();
    let ledger = outcome.report().ledger();

    assert_eq!(outcome.disposition(), PostProcessRunDisposition::Unchanged);
    assert_eq!(scene.generation(), generation);
    assert!(std::ptr::eq(outcome.scene(), &scene));
    assert!(events.lock().unwrap().is_empty());
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(
        ledger
            .dimension(PostProcessDimension::CanonicalBytes)
            .expect("canonical dimension should be frozen")
            .committed(),
        0
    );
}

#[test]
fn empty_plan_honors_the_final_control_poll() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let generation = scene.generation();
    let cancellation = CancelOnSecondPoll::new();
    let observed = cancellation.clone();
    let control = PostProcessControl::default().with_cancellation(cancellation);

    let failure = PostProcessPipeline::new([])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            source: ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        }
    ));
    assert_eq!(observed.polls(), 2);
    assert_eq!(scene.generation(), generation);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(
        failure
            .ledger()
            .dimension(PostProcessDimension::CanonicalBytes)
            .expect("canonical dimension should be frozen")
            .committed(),
        0
    );
}

#[test]
fn unchanged_transform_skips_validation_and_returns_unchanged() {
    let mut mesh = triangle_mesh();
    mesh.normals = vec![Vec3::new(0.0, 0.0, 1.0); mesh.positions.len()];
    let scene = scene_with_meshes([mesh]);
    let generation = scene.generation();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&events);
    let control = PostProcessControl::default().with_progress_observer(
        move |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
            observed.lock().unwrap().push(event.phase());
            if event.phase() == PostProcessProgressPhase::Validate && event.completed() == 1 {
                ProgressDecision::Cancel
            } else {
                ProgressDecision::Continue
            }
        },
    );

    let outcome = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap();
    let ledger = outcome.report().ledger();

    assert_eq!(outcome.disposition(), PostProcessRunDisposition::Unchanged);
    assert_eq!(scene.generation(), generation);
    assert!(std::ptr::eq(outcome.scene(), &scene));
    assert!(!events.lock().unwrap().is_empty());
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .all(|phase| *phase == PostProcessProgressPhase::Transform)
    );
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(
        ledger
            .phase(PostProcessLedgerPhase::TransactionStaging)
            .expect("transaction-staging phase should be frozen")
            .peak_live_bytes(),
        0
    );
    assert_eq!(
        ledger
            .dimension(PostProcessDimension::CanonicalBytes)
            .expect("canonical dimension should be frozen")
            .committed(),
        0
    );
}

#[test]
fn fully_satisfied_plan_skips_edit_staging() {
    let mut mesh = triangle_mesh();
    mesh.normals = vec![Vec3::new(0.0, 0.0, 1.0); mesh.positions.len()];
    mesh.bounds = Some(Aabb {
        min: Vec3::new(0.0, 0.0, 0.0),
        max: Vec3::new(1.0, 1.0, 0.0),
    });
    let scene = scene_with_meshes([mesh]);

    let outcome = PostProcessPipeline::new([
        PostProcessStep::Triangulate,
        PostProcessStep::GenerateNormals,
        PostProcessStep::GenerateBoundingBoxes,
    ])
    .unwrap()
    .run(&scene)
    .unwrap();
    let ledger = outcome.report().ledger();

    assert_eq!(outcome.disposition(), PostProcessRunDisposition::Unchanged);
    assert!(std::ptr::eq(outcome.scene(), &scene));
    assert_eq!(
        ledger
            .phase(PostProcessLedgerPhase::TransactionStaging)
            .expect("transaction-staging phase should be frozen")
            .peak_live_bytes(),
        0
    );
    assert_eq!(
        ledger
            .dimension(PostProcessDimension::CanonicalBytes)
            .expect("canonical dimension should be frozen")
            .committed(),
        0
    );
}

#[test]
fn unchanged_transform_honors_signal_cancelled_by_the_last_action() {
    let mut mesh = triangle_mesh();
    mesh.normals = vec![Vec3::new(0.0, 0.0, 1.0); mesh.positions.len()];
    let scene = scene_with_meshes([mesh]);
    let generation = scene.generation();
    let cancellation = CancellationToken::new();
    let observer_cancellation = cancellation.clone();
    let control = PostProcessControl::default()
        .with_cancellation(cancellation)
        .with_progress_observer(
            move |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
                if event.phase() == PostProcessProgressPhase::Transform && event.completed() == 1 {
                    observer_cancellation.cancel();
                }
                ProgressDecision::Continue
            },
        );

    let failure = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .run_with_control(&scene, control)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Control {
            source: ControlCause::Cancelled {
                origin: CancellationOrigin::Signal
            }
        }
    ));
    assert_eq!(scene.generation(), generation);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(
        failure
            .ledger()
            .dimension(PostProcessDimension::CanonicalBytes)
            .expect("canonical dimension should be frozen")
            .committed(),
        0
    );
}

#[test]
fn work_limit_failure_releases_staging() {
    let scene = scene_with_meshes([triangle_mesh()]);
    let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateNormals])
        .unwrap()
        .with_limits(PostProcessLimits::new(2, 1024 * 1024));

    let failure = pipeline
        .run(&scene)
        .unwrap_err()
        .into_operation_failure()
        .expect("operation should initialize");

    assert!(matches!(
        failure.error(),
        PostProcessError::Accounting { .. }
    ));
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert!(scene.meshes()[0].normals.is_empty());
}

#[test]
fn public_result_types_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<PostProcessError>();
    assert_send_sync::<PostProcessFailure>();
    assert_send_sync::<PostProcessLedgerSnapshot>();
    assert_send_sync::<PostProcessRunOutcome<'static>>();
    assert_send_sync::<PostProcessPipeline>();
    assert_send_sync::<PostProcessReport>();
    assert_send_sync::<PostProcessRunError>();
}

fn triangle_mesh() -> Mesh {
    Mesh {
        topology: PrimitiveTopology::Triangles,
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ],
        indices: vec![0, 1, 2],
        ..Mesh::default()
    }
}

fn phase_order(phase: PostProcessProgressPhase) -> u8 {
    match phase {
        PostProcessProgressPhase::Transform => 0,
        PostProcessProgressPhase::Validate => 1,
    }
}

fn scene_with_meshes<const N: usize>(meshes: [Mesh; N]) -> ValidatedScene {
    let mut builder = baozi_core::SceneBuilder::try_new().expect("test scene builder allocation");
    for mesh in meshes {
        let mesh = builder.add_mesh(mesh).expect("test mesh id should fit");
        builder
            .add_child_node(
                builder.root(),
                Node {
                    mesh_bindings: vec![MeshBinding::new(mesh)],
                    ..Node::default()
                },
            )
            .expect("test node should be valid");
    }
    validate_scene(builder.finish_draft()).expect("test scene should be valid")
}
