#![allow(
    clippy::result_large_err,
    reason = "qualification retains the real inline normal adapter failure vocabulary"
)]

use baozi_core::__scene_integration::integration::{
    SceneEditBootstrap, SceneVectorValues, try_start_scene_edit_operation,
};
use baozi_core::{AttributeData, AttributeDomain, ValidatedScene};
use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;
use baozi_operation::{
    CancellationOrigin, ControlCause, CounterClass, DimensionPolicy, LedgerCause,
    StoreLedgerEvidence, StorePreparationFailureClass, StoreRuntimeError, StoreTerminalCause,
};

use super::{Arc, AtomicUsize, Ordering, StopAtPoll, common};
use crate::normals::{NormalMode, input, kernel};
use crate::operation::{PostProcessOwner, runtime};
use crate::{PostProcessBudgets, PostProcessControl};

// Four startup account owners each complete one clone and one typed handle.
const ENTRY_WORK: u64 = 8;

fn start(
    scene: &ValidatedScene,
    control: PostProcessControl,
) -> (runtime::EditOperation, runtime::RuntimeKeys) {
    let (writer, _, _) = SceneEditBootstrap::try_new().unwrap().into_parts();
    let (keys, ledger, plan) =
        runtime::plan_selected_runtime(writer, PostProcessBudgets::default()).unwrap();
    let mut operation = try_start_scene_edit_operation::<PostProcessOwner, _, _>(
        scene,
        control.into_runtime(),
        ledger,
        plan,
    )
    .unwrap();
    let initial = operation
        .producer_host()
        .dimension_state(keys.work)
        .unwrap();
    assert_eq!(
        (initial.attempted(), initial.consumed()),
        (ENTRY_WORK, ENTRY_WORK)
    );
    assert!(!operation.account().snapshot().poisoned());
    (operation, keys)
}

fn live_storage(operation: &runtime::EditOperation) -> (u64, u64, bool) {
    let physical = operation.account().snapshot();
    (
        physical.live_retained_bytes(),
        physical.live_retained_allocations(),
        physical.poisoned(),
    )
}

fn finish(
    operation: runtime::EditOperation,
    keys: runtime::RuntimeKeys,
    failure: Option<StorePreparationFailureClass>,
    attempted: u64,
    consumed: u64,
) {
    let prepared = operation
        .prepare_evidence_only_success(runtime::final_progress())
        .expect("resolved normal work must retain its terminal owner");
    let evidence: StoreLedgerEvidence = match (prepared, failure) {
        (PreparedStoreEvidenceOnlyOutcome::Success(success), None) => {
            success.handoff().ledger_evidence
        }
        (PreparedStoreEvidenceOnlyOutcome::MinimalFallback(fallback), Some(_)) => {
            fallback.handoff().ledger_evidence
        }
        _ => panic!("an ignored normal execution failure must prevent success"),
    };
    match (evidence.header().terminal_cause(), failure) {
        (StoreTerminalCause::Success, None) => {}
        (StoreTerminalCause::Failure(cause), Some(expected)) => assert_eq!(cause.class(), expected),
        _ => panic!("normal execution must keep the selected terminal cause"),
    }
    let work = evidence.dimension(keys.work).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (attempted, consumed));
    assert_eq!(evidence.header().current_live_bytes(), 0);
    assert_eq!(evidence.header().active_reservations(), 0);
    let scratch = evidence.dimension(keys.scratch).unwrap();
    assert_eq!(scratch.current_live_bytes(), 0);
    assert_eq!(scratch.pending_publication(), 0);
    assert_eq!(scratch.committed_canonical_output(), 0);
    let canonical = evidence.dimension(keys.canonical).unwrap();
    assert_eq!(canonical.pending_publication(), 0);
    assert_eq!(canonical.committed_canonical_output(), 0);
}

fn assert_cancelled(result: Result<(), input::InputError>) {
    let cause = match result {
        Err(input::InputError::Runtime(cause))
        | Err(input::InputError::Kernel(kernel::Error::Control(cause))) => cause,
        other => panic!("expected a normal adapter cancellation, got {other:?}"),
    };
    assert!(matches!(
        cause,
        StoreRuntimeError::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        })
    ));
}

fn assert_budget_rejection(
    result: Result<(), input::InputError>,
    keys: runtime::RuntimeKeys,
    class: CounterClass,
    allowed: u64,
) {
    let Err(input::InputError::Kernel(kernel::Error::Control(StoreRuntimeError::Ledger(
        LedgerCause::LimitExceeded {
            dimension,
            class: actual,
            value,
            limit,
        },
    )))) = result
    else {
        panic!("the next kernel leaf must be rejected before physical work: {result:?}");
    };
    assert_eq!(dimension, keys.work);
    assert_eq!(actual, class);
    assert_eq!((value, limit), (allowed + 1, allowed));
}

fn restrict_work(
    operation: &mut runtime::EditOperation,
    keys: runtime::RuntimeKeys,
    class: CounterClass,
    allowed: u64,
) {
    let attempted = if class == CounterClass::Attempted {
        allowed
    } else {
        runtime::MAX_WORK
    };
    operation
        .producer_host()
        .tighten_dimension_policy(keys.work, DimensionPolicy::counter(attempted, allowed, 0))
        .unwrap();
}

#[test]
fn projected_flat_cancellation_keeps_periodic_and_final_short_region_prefixes() {
    // 198 output rows form seven chunks. Three validations then precede the face phase.
    // Each face returns one evaluation and three output actions, with control costs 9+1+1+1.
    // Twenty-one faces fit 252 credits; the next face polls before its reads.
    const FACES: usize = 66;
    const CORNERS: u64 = 198;
    const BEFORE_FACES: u64 = 20; // startup8 + share2 + init7 + validation3
    const COMPLETE: u64 = 284; // BEFORE_FACES + 66*4 semantic actions
    let mut obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\n".to_vec();
    for _ in 0..FACES {
        obj.extend_from_slice(b"f 1 2 3\n");
    }
    let (scene, import) = common::import_obj(&obj).into_parts();
    let generation = scene.generation();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let target = primitive.id();
    let positions = primitive.positions().unwrap();
    let indices: Vec<_> = (0..FACES).flat_map(|_| [0_u32, 1, 2]).collect();
    assert_eq!(primitive.indices(), Some(indices.as_slice()));
    assert!(matches!(
        positions.data(),
        AttributeData::DenseF32 { lanes: 3, .. }
    ));

    // Region entry, allocation, kernel entry, validation boundary, 3 weighted frontiers,
    // kernel final and execution final. Prefixes are literal action outcomes, not cost credits.
    for (stop, completed) in [
        (0, ENTRY_WORK),
        (1, ENTRY_WORK),
        (2, 17),
        (3, BEFORE_FACES),
        (4, 104),
        (5, 188),
        (6, 272),
        (7, COMPLETE),
        (8, COMPLETE),
        (usize::MAX, COMPLETE),
    ] {
        let polls = Arc::new(AtomicUsize::new(0));
        let (mut operation, keys) = start(
            &scene,
            PostProcessControl::default().with_cancellation_signal(StopAtPoll {
                polls: polls.clone(),
                stop,
                panic: false,
            }),
        );
        polls.store(0, Ordering::Relaxed);
        let physical = live_storage(&operation);
        let mut returned_output = false;
        let result = runtime::with_execution(&mut operation, keys, |active| {
            let generated = input::generate(
                input::NormalInput::projected(
                    positions,
                    primitive.indices(),
                    None,
                    CORNERS as u32,
                    NormalMode::Flat,
                    false,
                ),
                active,
                keys,
            )?;
            let SceneVectorValues::F32(values) = generated.values() else {
                panic!("F32 positions must keep F32 normal output");
            };
            assert_eq!(values.len(), CORNERS as usize);
            assert!(values.iter().all(|value| *value == [0.0, 0.0, 1.0]));
            returned_output = true;
            Ok::<_, input::InputError>(())
        });
        assert_eq!(returned_output, stop >= 8);
        assert_eq!(
            polls.load(Ordering::Relaxed),
            if stop == usize::MAX { 9 } else { stop + 1 }
        );
        let failure = if stop == usize::MAX {
            result.unwrap();
            None
        } else {
            assert_cancelled(result);
            Some(StorePreparationFailureClass::Control)
        };
        assert_eq!(live_storage(&operation), physical);
        finish(operation, keys, failure, completed, completed);
        assert_eq!(scene.generation(), generation);
        assert!(import.matches_scene(&scene));
        let original = scene.primitive(target).unwrap();
        assert_eq!(original.indices(), Some(indices.as_slice()));
        assert_eq!(
            original.positions().unwrap().data(),
            AttributeData::DenseF32 {
                lanes: 3,
                values: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            }
        );
        assert!(original.normals(AttributeDomain::Corner).is_none());
    }
}

#[test]
fn projected_smooth_cancellation_keeps_ordering_and_final_short_prefixes() {
    const FACES: usize = 90;
    const CORNERS: u64 = 3 * FACES as u64;
    // 9 output +9 order/tail +3 cache chunks, three shares, three Position validations,
    // and90 face evaluations: 8+6+21+3+90=128 before ordering.
    const BEFORE_ORDER: u64 = 128;
    // Ordering adds3 group +1 reset +9 histogram +1 prefix +9 scatter chunks =23 actions.
    // 272 candidate visits, 270 outputs, 3 starts and3 finishes:548 actions.
    const COMPLETE: u64 = 699;
    let mut obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\ns 1\n".to_vec();
    for _ in 0..FACES {
        obj.extend_from_slice(b"f 1 2 3\n");
    }
    let (scene, import) = common::import_obj(&obj).into_parts();
    let generation = scene.generation();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let positions = primitive.positions().unwrap();
    let groups = [1_u32; FACES];
    assert_eq!(positions.len(), 3);
    assert_eq!(primitive.indices().unwrap().len(), CORNERS as usize);

    // Poll11 is ordering entry. Eight32-credit chunks fit before poll12;
    // another eight fit before poll13; poll14 begins accumulation.
    for (stop, completed) in [
        (11, BEFORE_ORDER),
        (12, 136),
        (13, 144),
        (14, 151),
        (19, COMPLETE),
        (20, COMPLETE),
        (usize::MAX, COMPLETE),
    ] {
        let polls = Arc::new(AtomicUsize::new(0));
        let (mut operation, keys) = start(
            &scene,
            PostProcessControl::default().with_cancellation_signal(StopAtPoll {
                polls: polls.clone(),
                stop,
                panic: false,
            }),
        );
        polls.store(0, Ordering::Relaxed);
        let physical = live_storage(&operation);
        let mut returned_output = false;
        let result = runtime::with_execution(&mut operation, keys, |active| {
            let generated = input::generate(
                input::NormalInput::projected(
                    positions,
                    primitive.indices(),
                    Some(&groups),
                    CORNERS as u32,
                    NormalMode::Smooth,
                    false,
                ),
                active,
                keys,
            )?;
            let SceneVectorValues::F32(values) = generated.values() else {
                panic!("F32 positions must keep F32 normal output");
            };
            assert_eq!(values.len(), CORNERS as usize);
            assert!(values.iter().all(|value| *value == [0.0, 0.0, 1.0]));
            returned_output = true;
            Ok::<_, input::InputError>(())
        });
        assert_eq!(returned_output, stop >= 20);
        assert_eq!(
            polls.load(Ordering::Relaxed),
            if stop == usize::MAX { 21 } else { stop + 1 }
        );
        let failure = if stop == usize::MAX {
            result.unwrap();
            None
        } else {
            assert_cancelled(result);
            Some(StorePreparationFailureClass::Control)
        };
        assert_eq!(live_storage(&operation), physical);
        finish(operation, keys, failure, completed, completed);
        assert_eq!(scene.generation(), generation);
        assert!(import.matches_scene(&scene));
        assert!(primitive.normals(AttributeDomain::Corner).is_none());
    }
}

#[test]
fn enlarged_order_admission_rejects_without_retrying_the_general_algorithm() {
    let (scene, import) = common::canonical_triangle_import().into_parts();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let positions = primitive.positions().unwrap();
    let generation = scene.generation();
    let ids = [0_u32, 1, 2, 0, 1, 2];
    // Six output rows admit an eight-row envelope. C=6 used eight order slots;
    // C+V=9 now requires sixteen. Even the smallest output backing rejects the latter.
    let limit =
        8 * std::mem::size_of::<[f32; 3]>() as u64 + 8 * std::mem::size_of::<usize>() as u64;
    let (mut operation, keys) = start(&scene, PostProcessControl::default());
    operation
        .producer_host()
        .tighten_dimension_policy(
            keys.scratch,
            DimensionPolicy::reservable_bytes(0, 0, 0, limit),
        )
        .unwrap();
    let physical = live_storage(&operation);
    let mut returned_output = false;
    let result: Result<(), input::InputError> =
        runtime::with_execution(&mut operation, keys, |active| {
            input::generate(
                input::NormalInput::projected(
                    positions,
                    Some(&ids),
                    None,
                    6,
                    NormalMode::Smooth,
                    false,
                ),
                active,
                keys,
            )?;
            returned_output = true;
            Ok(())
        });
    assert!(
        !returned_output,
        "storage rejection must not retry the general workspace"
    );
    let Err(input::InputError::Runtime(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
        dimension,
        class,
        value,
        limit: actual_limit,
    }))) = result
    else {
        panic!("expected the enlarged scratch admission error, got {result:?}");
    };
    assert_eq!(dimension, keys.scratch);
    assert_eq!(class, CounterClass::Live);
    assert_eq!(actual_limit, limit);
    assert!(value > limit);
    assert_eq!(live_storage(&operation), physical);
    // Only output storage's two share units and its one six-row chunk completed.
    let completed = 11;
    finish(
        operation,
        keys,
        Some(StorePreparationFailureClass::Ledger),
        completed,
        completed,
    );
    assert_eq!(scene.generation(), generation);
    assert!(import.matches_scene(&scene));
    assert!(primitive.normals(AttributeDomain::Corner).is_none());
}

#[test]
fn projected_dense_action_denial_keeps_sort_and_output_prefixes() {
    let (scene, import) = common::canonical_triangle_import().into_parts();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let positions = primitive.positions().unwrap();
    let generation = scene.generation();
    // Flat: startup8+share2+one output chunk=11; validation12..14, face15, output16..18.
    // Smooth: startup/share/init=17; validation18..20, face21, ordering22..25;
    // first bucket output30, next35, final39. A hard group instead uses one proof,
    // one identity chunk, and bounded heap levels/extractions before the same candidate protocol.
    let hard_groups = [0_u32];
    for (mode, groups, allowed, complete) in [
        (NormalMode::Flat, None, 10, 18), // Denied initialization.
        (NormalMode::Flat, None, 11, 18), // Denied first logical Position.
        (NormalMode::Flat, None, 14, 18), // Denied face before any face tuple.
        (NormalMode::Flat, None, 15, 18), // Denied first output.
        (NormalMode::Flat, None, 16, 18),
        (NormalMode::Flat, None, 17, 18),
        (NormalMode::Flat, None, 18, 18),
        (NormalMode::Smooth, None, 21, 39), // Denied reset chunk.
        (NormalMode::Smooth, None, 22, 39), // Denied histogram chunk.
        (NormalMode::Smooth, None, 23, 39), // Denied prefix chunk.
        (NormalMode::Smooth, None, 24, 39), // Denied scatter chunk.
        (NormalMode::Smooth, None, 29, 39), // Denied first bucket output.
        (NormalMode::Smooth, None, 38, 39),
        (NormalMode::Smooth, None, 39, 39),
        (NormalMode::Smooth, Some(hard_groups.as_slice()), 21, 41),
        (NormalMode::Smooth, Some(hard_groups.as_slice()), 22, 41),
        (NormalMode::Smooth, Some(hard_groups.as_slice()), 23, 41),
        (NormalMode::Smooth, Some(hard_groups.as_slice()), 40, 41),
        (NormalMode::Smooth, Some(hard_groups.as_slice()), 41, 41),
    ] {
        for class in [CounterClass::Attempted, CounterClass::Consumed] {
            let polls = Arc::new(AtomicUsize::new(0));
            let (mut operation, keys) = start(
                &scene,
                PostProcessControl::default().with_cancellation_signal(StopAtPoll {
                    polls: polls.clone(),
                    stop: usize::MAX,
                    panic: false,
                }),
            );
            polls.store(0, Ordering::Relaxed);
            restrict_work(&mut operation, keys, class, allowed);
            let physical = live_storage(&operation);
            let mut returned_output = false;
            let result = runtime::with_execution(&mut operation, keys, |active| {
                let generated = input::generate(
                    input::NormalInput::projected(
                        positions,
                        primitive.indices(),
                        groups,
                        3,
                        mode,
                        false,
                    ),
                    active,
                    keys,
                )?;
                let SceneVectorValues::F32(values) = generated.values() else {
                    panic!("F32 positions must keep F32 output");
                };
                assert_eq!(values, [[0.0, 0.0, 1.0]; 3].as_slice());
                returned_output = true;
                Ok::<_, input::InputError>(())
            });
            assert_eq!(returned_output, allowed == complete);
            let failure = if allowed == complete {
                result.unwrap();
                None
            } else {
                assert_budget_rejection(result, keys, class, allowed);
                Some(StorePreparationFailureClass::Ledger)
            };
            assert_eq!(live_storage(&operation), physical);
            finish(
                operation,
                keys,
                failure,
                allowed + u64::from(failure.is_some()),
                allowed,
            );
            assert_eq!(scene.generation(), generation);
            assert!(import.matches_scene(&scene));
            assert_eq!(primitive.indices(), Some([0, 1, 2].as_slice()));
            assert_eq!(
                primitive.positions().unwrap().data(),
                AttributeData::DenseF32 {
                    lanes: 3,
                    values: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                }
            );
            assert!(primitive.normals(AttributeDomain::Corner).is_none());
        }
    }
}

#[test]
fn real_runtime_kernel_output_rejection_preserves_the_written_prefix() {
    let scene = common::canonical_triangle_scene();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    // Without adapter scratch, three Position validations and one face action precede output.
    // Inspect real destination storage, independently of ledger totals.
    for written in 0..=3 {
        let (mut operation, keys) = start(&scene, PostProcessControl::default());
        let physical = live_storage(&operation);
        let allowed = ENTRY_WORK + 4 + written;
        if written < 3 {
            restrict_work(&mut operation, keys, CounterClass::Attempted, allowed);
        }
        let mut output = [[73.0; 3]; 3];
        let result = runtime::with_execution(&mut operation, keys, |active| {
            kernel::generate_into(
                kernel::Input {
                    positions: primitive.positions().unwrap(),
                    vertex_ids: kernel::VertexIds::Indexed(primitive.indices().unwrap()),
                    smoothing_groups: None,
                },
                kernel::Mode::Flat,
                kernel::Output::F32(&mut output),
                kernel::Workspace {
                    corner_order: &mut [],
                    vertex_cursors: None,
                    face_crosses: None,
                },
                |event| match event {
                    kernel::WorkEvent::Begin(kind) => active.begin_action(kind.control_cost()),
                    kernel::WorkEvent::Complete => active.complete_action(),
                    kernel::WorkEvent::Checkpoint => active.checkpoint(),
                },
            )
            .map_err(input::InputError::Kernel)
        });
        assert!(
            output[..written as usize]
                .iter()
                .all(|value| *value == [0.0, 0.0, 1.0])
        );
        assert!(
            output[written as usize..]
                .iter()
                .all(|value| *value == [73.0; 3])
        );
        assert_eq!(live_storage(&operation), physical);
        if written < 3 {
            assert_budget_rejection(result, keys, CounterClass::Attempted, allowed);
            finish(
                operation,
                keys,
                Some(StorePreparationFailureClass::Ledger),
                allowed + 1,
                allowed,
            );
        } else {
            result.unwrap();
            finish(operation, keys, None, allowed, allowed);
        }
        assert!(primitive.normals(AttributeDomain::Corner).is_none());
    }
}

struct SparseTriangle;

impl baozi_import::Format for SparseTriangle {
    type Options = baozi_import::NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut baozi_import::RegistryBuildScope<'_>,
    ) -> Result<baozi_import::CompiledOptions<Self::Options>, baozi_import::OptionCompileError>
    {
        baozi_import::NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> baozi_import::FormatDescriptor {
        baozi_import::FormatDescriptor::builder(
            baozi_core::FormatId::new("dev.baozi.test.normal-runtime-sparse").unwrap(),
            "Sparse normal runtime triangle",
        )
        .encoding(baozi_import::FormatEncoding::Binary)
        .capabilities([(
            baozi_import::FormatCapability::Geometry,
            baozi_import::CapabilityStatus::Supported,
        )])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _: &baozi_import::Probe<'_>,
        _: &baozi_import::ReadHint,
        _: &baozi_import::DetectionScope<'_>,
    ) -> baozi_import::DetectionResult<baozi_import::DetectionOutcome> {
        Ok(baozi_import::DetectionOutcome::Match(
            baozi_import::ReadConfidence::Certain,
        ))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut baozi_import::ParseContext<'_>,
    ) -> baozi_import::ParseResult<()> {
        use baozi_import::authoring::{
            AttributeSemantic, AuthoringRange, GeometryAttributeAuthoring,
            GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
            Presence, StaticSceneBuilder,
        };
        let positions = [GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            3,
            &[1, 2],
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            &[0.0; 3],
        )];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&[0, 1, 2]),
            baozi_core::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 1),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        context.submit(
            StaticSceneBuilder::new(Presence::Present(&[]), &[])
                .with_scene_space(&baozi_core::SceneSpace::STANDARD_TARGET)
                .with_geometry(&meshes, &primitives, &positions, &[]),
        )
    }
}

#[test]
fn projected_sparse_action_denial_keeps_logical_read_face_and_output_prefixes() {
    use baozi_import::{
        FormatSelection, ImportConfigOverlay, ImportOperationControl, ImportRequest,
        ImportRequestLayer, OptionCompileLimits, RegistryBuildLimits, RegistryBuilder,
    };
    let mut registry = RegistryBuilder::new();
    registry.register(SparseTriangle).unwrap();
    let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let request = registry
        .plan(
            &ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
                ImportConfigOverlay::default().with_format_selection(FormatSelection::Forced(
                    baozi_core::FormatId::new("dev.baozi.test.normal-runtime-sparse").unwrap(),
                )),
            )),
            OptionCompileLimits::default(),
        )
        .unwrap();
    let (scene, import) = request
        .read_bytes(
            baozi_io::AssetPath::new("normals.fixture").unwrap(),
            b"sparse",
            ImportOperationControl::default(),
        )
        .unwrap()
        .into_parts();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let positions = primitive.positions().unwrap();
    assert!(kernel::PositionSource::is_sparse(&positions));
    assert!(!kernel::PositionSource::is_f64(&positions));
    let generation = scene.generation();
    // Sparse comparisons are pure internal reads. The semantic sequence equals the dense
    // triangle: startup8+share2+init1, three logical validations, face1, three outputs.
    for allowed in [10, 11, 12, 13, 14, 15, 16, 17, 18] {
        for class in [CounterClass::Attempted, CounterClass::Consumed] {
            let polls = Arc::new(AtomicUsize::new(0));
            let (mut operation, keys) = start(
                &scene,
                PostProcessControl::default().with_cancellation_signal(StopAtPoll {
                    polls: polls.clone(),
                    stop: usize::MAX,
                    panic: false,
                }),
            );
            polls.store(0, Ordering::Relaxed);
            restrict_work(&mut operation, keys, class, allowed);
            let physical = live_storage(&operation);
            let mut returned_output = false;
            let result = runtime::with_execution(&mut operation, keys, |active| {
                let generated = input::generate(
                    input::NormalInput::projected(
                        positions,
                        primitive.indices(),
                        None,
                        3,
                        NormalMode::Flat,
                        false,
                    ),
                    active,
                    keys,
                )?;
                let SceneVectorValues::F32(values) = generated.values() else {
                    panic!("sparse F32 positions must keep F32 output");
                };
                assert_eq!(values, [[0.0, 0.0, 1.0]; 3]);
                returned_output = true;
                Ok::<_, input::InputError>(())
            });
            assert_eq!(returned_output, allowed == 18);
            let failure = if allowed == 18 {
                result.unwrap();
                None
            } else {
                assert_budget_rejection(result, keys, class, allowed);
                Some(StorePreparationFailureClass::Ledger)
            };
            assert_eq!(live_storage(&operation), physical);
            finish(
                operation,
                keys,
                failure,
                allowed + u64::from(failure.is_some()),
                allowed,
            );
            assert_eq!(scene.generation(), generation);
            assert!(import.matches_scene(&scene));
            assert_eq!(primitive.indices(), Some([0, 1, 2].as_slice()));
            assert_eq!(
                primitive.positions().unwrap().data(),
                AttributeData::SparseF32 {
                    lanes: 3,
                    logical_count: 3,
                    indices: &[1, 2],
                    values: &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                    default: &[0.0; 3],
                }
            );
            assert!(primitive.normals(AttributeDomain::Corner).is_none());
        }
    }
}
