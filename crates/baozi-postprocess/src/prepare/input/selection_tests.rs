use super::super::cleanup_tests::Event as CleanupEvent;
use super::*;
use crate::operation::runtime::{MAX_WORK, with_execution, with_test_operation};
use baozi_operation::__retained_store_integration::activation::{
    FiniteCanonicalOperationPublication, StoreInvocationError,
};
use baozi_operation::__retained_store_integration::authority::PreparedFiniteStoreEvidenceOnlyOutcome;
use baozi_operation::{
    CancellationSignal, CapacityPlan, CounterClass, DimensionPolicy, LedgerCause,
    MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS, StoreRuntimeError,
};
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Leaf {
    Lookup,
    Append,
    Comparison,
    Swap,
    Duplicate,
}

#[derive(Clone, Copy)]
enum Fault {
    Deny(Leaf, usize),
    CancelLookup(usize),
    CancelLookupError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Before(Leaf),
    Entered(Leaf),
    Returned(Leaf, bool),
    Validation,
    Cleanup(CleanupEvent),
}

struct Snapshot {
    readers: Vec<PrimitiveId>,
    address: usize,
    capacity: usize,
    work: (u64, u64, u64),
    polls: usize,
    event_index: usize,
}

struct Trace {
    fault: Fault,
    keys: Option<RuntimeKeys>,
    storage: Option<Snapshot>,
    frontier: Option<Snapshot>,
    rejected: Option<Snapshot>,
    semantic_error: Option<baozi_core::SceneLookupError>,
    current: Option<Leaf>,
    events: Vec<Event>,
    polls: usize,
    cancelled: bool,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

impl Trace {
    fn snapshot(&self, operation: &ActiveRuntime<'_>, values: &[PrimitiveRef<'_>]) -> Snapshot {
        Snapshot {
            readers: values.iter().map(|primitive| primitive.id()).collect(),
            address: values.as_ptr() as usize,
            capacity: self.storage.as_ref().map_or(0, |storage| storage.capacity),
            work: operation.work_counters(self.keys.unwrap().work).unwrap(),
            polls: self.polls,
            event_index: self.events.len(),
        }
    }

    fn assert_storage_released(&self, expected: &[PrimitiveId]) {
        let storage = self.storage.as_ref().expect("actual Many allocation");
        let frontier = self.frontier.as_ref().expect("the designated frontier ran");
        assert_eq!(frontier.readers, expected);
        assert_eq!(frontier.address, storage.address);
        assert_eq!(frontier.capacity, storage.capacity);
        assert!(storage.capacity >= expected.len());
        let drops: Vec<_> = self
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Cleanup(CleanupEvent::ScratchDropped {
                    address,
                    len,
                    capacity,
                }) => Some((*address, *len, *capacity)),
                _ => None,
            })
            .collect();
        assert_eq!(drops, [(storage.address, expected.len(), storage.capacity)]);
        assert!(!self.events.iter().any(|event| matches!(
            event,
            Event::Validation
                | Event::Cleanup(
                    CleanupEvent::Planned
                        | CleanupEvent::Materializing
                        | CleanupEvent::BuildingEvidence
                        | CleanupEvent::Validating
                        | CleanupEvent::ValidationProgress
                        | CleanupEvent::PreparedEditDropped
                        | CleanupEvent::Preparing(_)
                )
        )));
    }

    fn assert_stopped(&self, leaf: Leaf, expected: &[PrimitiveId]) {
        self.assert_storage_released(expected);
        let frontier = self.frontier.as_ref().unwrap();
        let storage = self.storage.as_ref().unwrap();
        let completed = self.events[..frontier.event_index]
            .iter()
            .filter(|event| matches!(event, Event::Returned(_, true)))
            .count() as u64;
        assert_eq!(
            frontier.work,
            (storage.work.0 + completed, storage.work.1 + completed, 0)
        );
        let later: Vec<_> = self.events[frontier.event_index..]
            .iter()
            .copied()
            .filter(|event| !matches!(event, Event::Cleanup(_)))
            .collect();
        assert_eq!(later, [Event::Before(leaf), Event::Returned(leaf, false)]);
        let rejected = self.rejected.as_ref().expect("actual leaf rejection");
        assert_eq!(rejected.readers, frontier.readers);
        assert_eq!(rejected.address, frontier.address);
        assert_eq!(rejected.capacity, frontier.capacity);
        assert!(self.current.is_none());
    }
}

pub(super) fn storage_ready(
    operation: &ActiveRuntime<'_>,
    keys: RuntimeKeys,
    values: &Vec<PrimitiveRef<'_>>,
) {
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace else { return };
        assert!(trace.storage.is_none());
        assert!(values.is_empty());
        trace.keys = Some(keys);
        let mut storage = trace.snapshot(operation, values);
        storage.capacity = values.capacity();
        trace.storage = Some(storage);
    });
}

pub(super) fn before_leaf(
    operation: &mut ActiveRuntime<'_>,
    leaf: Leaf,
    values: &[PrimitiveRef<'_>],
) {
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace else { return };
        assert!(trace.current.replace(leaf).is_none());
        trace.events.push(Event::Before(leaf));
        let targeted = match trace.fault {
            Fault::Deny(target, occurrence) => {
                leaf == target
                    && trace
                        .events
                        .iter()
                        .filter(|event| **event == Event::Before(leaf))
                        .count()
                        == occurrence
            }
            Fault::CancelLookup(prefix) => leaf == Leaf::Lookup && values.len() == prefix,
            Fault::CancelLookupError => false,
        };
        if targeted {
            assert!(trace.frontier.is_none());
            let mut frontier = trace.snapshot(operation, values);
            frontier.event_index -= 1;
            assert_eq!(frontier.work.0, frontier.work.1);
            assert_eq!(frontier.work.2, 0);
            if matches!(trace.fault, Fault::Deny(..)) {
                operation
                    .tighten_dimension_policy(
                        trace.keys.unwrap().work,
                        DimensionPolicy::counter(MAX_WORK, frontier.work.1, 0),
                    )
                    .expect("retain the actual completed prefix and deny the next admission");
                assert_eq!(
                    operation.work_counters(trace.keys.unwrap().work).unwrap(),
                    frontier.work
                );
            } else {
                trace.cancelled = true;
            }
            trace.frontier = Some(frontier);
        }
    });
}

pub(super) fn entered_leaf() {
    TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace {
            trace.events.push(Event::Entered(trace.current.unwrap()));
        }
    });
}

pub(super) fn returned_leaf(
    operation: &ActiveRuntime<'_>,
    values: &[PrimitiveRef<'_>],
    returned: bool,
) {
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace else { return };
        let leaf = trace.current.take().unwrap();
        trace.events.push(Event::Returned(leaf, returned));
        if !returned {
            trace.rejected = Some(trace.snapshot(operation, values));
        }
    });
}

pub(super) fn lookup_returned(
    operation: &ActiveRuntime<'_>,
    values: &[PrimitiveRef<'_>],
    result: &Result<Result<PrimitiveRef<'_>, baozi_core::SceneLookupError>, StoreRuntimeError>,
) {
    returned_leaf(operation, values, result.is_ok());
    let Ok(Err(cause)) = result else { return };
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace else { return };
        if matches!(trace.fault, Fault::CancelLookupError) {
            assert!(trace.frontier.is_none());
            let mut frontier = trace.snapshot(operation, values);
            frontier.event_index = trace.events.len() - 3;
            trace.semantic_error = Some(*cause);
            trace.frontier = Some(frontier);
            trace.cancelled = true;
        }
    });
}

pub(super) fn validation_started() {
    TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace {
            trace.events.push(Event::Validation);
        }
    });
}

pub(in crate::prepare) fn record_cleanup(event: CleanupEvent) {
    TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace {
            trace.events.push(Event::Cleanup(event));
        }
    });
}

struct Recording;

impl Recording {
    fn start(fault: Fault) -> Self {
        TRACE.with_borrow_mut(|trace| {
            assert!(
                trace
                    .replace(Trace {
                        fault,
                        keys: None,
                        storage: None,
                        frontier: None,
                        rejected: None,
                        semantic_error: None,
                        current: None,
                        events: Vec::new(),
                        polls: 0,
                        cancelled: false,
                    })
                    .is_none()
            );
        });
        Self
    }

    fn finish(self) -> Trace {
        TRACE.with_borrow_mut(|trace| trace.take().unwrap())
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        TRACE.with_borrow_mut(|trace| {
            let _ = trace.take();
        });
    }
}

struct SelectionCancellation;

impl CancellationSignal for SelectionCancellation {
    fn is_cancelled(&self) -> bool {
        TRACE.with_borrow_mut(|trace| {
            let Some(trace) = trace else { return false };
            trace.polls += 1;
            trace.cancelled
        })
    }
}

#[test]
fn denied_reader_sort_swap_keeps_the_completed_comparison_and_original_order() {
    let (scene, _import) = crate::test_fixtures::import_obj(
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\no first\nf 1 2 3\no second\nf 1 3 2\n",
    )
    .into_parts();
    let primitives: Vec<_> = scene.meshes().flat_map(|mesh| mesh.primitives()).collect();
    assert_eq!(primitives.len(), 2);
    let original = [primitives[0].id(), primitives[1].id()];
    assert!(original[0] < original[1]);
    let (publication, (keys, before)) = with_test_operation(
        &scene,
        crate::PostProcessControl::default(),
        |operation, keys| {
            let mut before = (0, 0, 0);
            let result = with_execution(operation, keys, |active| {
                before = active.work_counters(keys.work)?;
                assert_eq!(before.0, before.1);
                assert_eq!(before.2, 0);
                active.tighten_dimension_policy(
                    keys.work,
                    DimensionPolicy::counter(MAX_WORK, before.1 + 1, 0),
                )?;
                let mut selected = [primitives[0], primitives[1]];
                let result = sort_selected(&mut selected, active);
                assert_eq!(selected.map(|primitive| primitive.id()), original);
                assert!(matches!(
                    &result,
                    Err(PreparationCause::Runtime(StoreRuntimeError::Ledger(
                        LedgerCause::LimitExceeded {
                            dimension,
                            class: CounterClass::Consumed,
                            value,
                            limit,
                        }
                    ))) if *dimension == keys.work
                        && (*value, *limit) == (before.1 + 2, before.1 + 1)
                ));
                assert_eq!(
                    active.work_counters(keys.work).unwrap(),
                    (before.0 + 2, before.1 + 1, 0)
                );
                result
            });
            assert!(matches!(result, Err(PreparationCause::Runtime(_))));
            (keys, before)
        },
    );
    let FiniteCanonicalOperationPublication::EvidenceOnly(
        PreparedFiniteStoreEvidenceOnlyOutcome::MinimalFallback(fallback),
    ) = publication
    else {
        panic!("the original numerical rejection must prevent successful publication");
    };
    let facts = fallback.handoff().facts;
    let work = facts.dimension(keys.work).unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (before.0 + 2, before.1 + 1)
    );
    assert_eq!(facts.header().current_live_bytes(), 0);
    assert_eq!(facts.header().active_reservations(), 0);
}

#[test]
fn denied_many_reader_storage_precedes_the_first_invalid_lookup() {
    let scene = crate::test_fixtures::canonical_triangle_scene();
    let foreign = crate::test_fixtures::canonical_triangle_scene();
    let valid = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let invalid = foreign
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    assert_ne!(valid.generation(), invalid.generation());
    let (publication, keys) = with_test_operation(
        &scene,
        crate::PostProcessControl::default(),
        |operation, keys| {
            let live = operation
                .with_exclusive_execution::<_, StoreInvocationError>(|context| {
                    context.dimension_state(keys.scratch)
                })
                .unwrap()
                .live_bytes();
            let result = with_execution(operation, keys, |active| {
                let before = active.work_counters(keys.work)?;
                active.tighten_dimension_policy(
                    keys.scratch,
                    DimensionPolicy::reservable_bytes(0, 0, 0, live),
                )?;
                let result = SelectedPrimitives::resolve(&scene, &[invalid, valid], active, keys);
                assert!(matches!(
                    &result,
                    Err(PreparationCause::Runtime(StoreRuntimeError::Ledger(
                        LedgerCause::LimitExceeded {
                            dimension,
                            class: CounterClass::Live,
                            value,
                            limit,
                        }
                    ))) if *dimension == keys.scratch && *limit == live && *value > live
                ));
                assert_eq!(active.work_counters(keys.work).unwrap(), before);
                result.map(|_| ())
            });
            assert!(matches!(result, Err(PreparationCause::Runtime(_))));
            keys
        },
    );
    let FiniteCanonicalOperationPublication::EvidenceOnly(
        PreparedFiniteStoreEvidenceOnlyOutcome::MinimalFallback(fallback),
    ) = publication
    else {
        panic!("the original denied storage admission must retain its own failure");
    };
    let facts = fallback.handoff().facts;
    assert_eq!(facts.header().current_live_bytes(), 0);
    assert_eq!(facts.header().active_reservations(), 0);
    let scratch = facts.dimension(keys.scratch).unwrap();
    assert_eq!(scratch.current_live_bytes(), 0);
}

fn many_fixture() -> baozi_import::ImportReport {
    crate::test_fixtures::import_obj(
        b"v -0.0 0 0\nv 1 0 0\nv 0 1 0\no first\nf 1 2 3\no second\nf 1 3 2\no third\nf 2 1 3\n",
    )
}

#[derive(Debug, PartialEq, Eq)]
struct SourcePrimitive {
    target: PrimitiveId,
    topology: TopologyKind,
    indices: Option<Vec<u32>>,
    attributes: Vec<(String, Vec<u64>)>,
}

fn source_snapshot(scene: &ValidatedScene) -> Vec<SourcePrimitive> {
    scene
        .meshes()
        .flat_map(|mesh| mesh.primitives())
        .map(|primitive| SourcePrimitive {
            target: primitive.id(),
            topology: primitive.topology(),
            indices: primitive.indices().map(<[u32]>::to_vec),
            attributes: primitive
                .attributes()
                .map(|attribute| {
                    let bits = match attribute.data() {
                        AttributeData::DenseF32 { values, .. } => values
                            .iter()
                            .map(|value| u64::from(value.to_bits()))
                            .collect(),
                        AttributeData::DenseF64 { values, .. } => {
                            values.iter().map(|value| value.to_bits()).collect()
                        }
                        AttributeData::DenseU32 { values, .. } => {
                            values.iter().copied().map(u64::from).collect()
                        }
                        _ => panic!("the OBJ fixture has only dense floating or unsigned fields"),
                    };
                    (
                        format!(
                            "{:?}/{:?}/{}",
                            attribute.semantic(),
                            attribute.domain(),
                            attribute.lanes()
                        ),
                        bits,
                    )
                })
                .collect(),
        })
        .collect()
}

fn owned_unattached_failure(
    failure: crate::PostProcessRunFailure,
    import: baozi_import::ImportEvidence,
    original: &[SourcePrimitive],
) -> crate::PreparationFailure {
    assert_eq!(
        failure.input().generation(),
        original[0].target.generation()
    );
    assert!(import.matches_scene(failure.input()));
    assert_eq!(source_snapshot(failure.input()), original);
    let (scene, error) = failure.into_parts();
    drop(scene);
    drop(import);
    let crate::PostProcessRunError::Preparation { failure } = error else {
        panic!("selection rejection retains its typed preparation error");
    };
    assert!(matches!(
        &failure.owner,
        crate::direct_error::FailureOwner::None
    ));
    let resources = failure.resources().expect("owned original operation facts");
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert_eq!(resources.committed_canonical_bytes(), 0);
    assert_eq!(resources.published_outcome_evidence_bytes(), 0);
    assert_eq!(resources.published_terminal_evidence_records(), 0);
    failure
}

#[test]
fn actual_many_selection_denies_each_remaining_leaf_before_its_closure() {
    let foreign = crate::test_fixtures::canonical_triangle_scene();
    let invalid = foreign
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    for leaf in [
        Leaf::Lookup,
        Leaf::Append,
        Leaf::Comparison,
        Leaf::Duplicate,
    ] {
        let (scene, import) = many_fixture().into_parts();
        let original = source_snapshot(&scene);
        assert_eq!(original.len(), 3);
        let authored = [original[0].target, original[1].target, original[2].target];
        assert!(authored.windows(2).all(|pair| pair[0] < pair[1]));
        assert_ne!(scene.generation(), invalid.generation());
        let (targets, occurrence, expected) = match leaf {
            Leaf::Lookup => ([authored[0], invalid, authored[2]], 2, vec![authored[0]]),
            Leaf::Append => (authored, 2, vec![authored[0]]),
            Leaf::Comparison => (authored, 2, authored.to_vec()),
            Leaf::Duplicate => (
                [authored[2], authored[0], authored[0]],
                1,
                vec![authored[0], authored[0], authored[2]],
            ),
            Leaf::Swap => unreachable!("the retained direct-sort test covers swap admission"),
        };
        let recording = Recording::start(Fault::Deny(leaf, occurrence));
        let failure = crate::prepare_triangles(
            scene,
            &targets,
            TrianglePreparationOptions::default(),
            crate::PostProcessControl::default(),
        )
        .expect_err("the designated real leaf must reject before validation");
        let trace = recording.finish();
        trace.assert_stopped(leaf, &expected);
        let before = trace.frontier.as_ref().unwrap().work;
        assert_eq!(
            trace.rejected.as_ref().unwrap().work,
            (before.0 + 1, before.1, 0)
        );
        if leaf == Leaf::Append {
            assert_eq!(
                trace.events[trace.frontier.as_ref().unwrap().event_index - 1],
                Event::Returned(Leaf::Lookup, true)
            );
        }
        if leaf == Leaf::Comparison {
            assert_eq!(
                trace
                    .events
                    .iter()
                    .filter(|event| **event == Event::Returned(Leaf::Comparison, true))
                    .count(),
                1
            );
        }
        if leaf == Leaf::Duplicate {
            assert!(
                trace
                    .events
                    .contains(&Event::Returned(Leaf::Comparison, true))
            );
            assert!(trace.events.contains(&Event::Returned(Leaf::Swap, true)));
        }
        let failure = owned_unattached_failure(failure, import, &original);
        let PreparationCause::Runtime(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
            dimension,
            class,
            value,
            limit,
        })) = failure.cause()
        else {
            panic!("{leaf:?} must retain the original numerical rejection: {failure:?}");
        };
        assert_eq!(*dimension, trace.keys.unwrap().work);
        assert_eq!(*class, CounterClass::Consumed);
        assert_eq!((*value, *limit), (before.1 + 1, before.1));
        let resources = failure.resources().unwrap();
        assert_eq!(resources.attempted_work_actions(), before.0 + 1);
        assert_eq!(resources.consumed_work_actions(), before.1);
        let requested = CapacityPlan::for_type::<PrimitiveRef<'_>>(0, 0, targets.len())
            .unwrap()
            .envelope_delta_bytes()
            .unwrap();
        let actual =
            (trace.storage.as_ref().unwrap().capacity * size_of::<PrimitiveRef<'_>>()) as u64;
        assert_eq!(resources.peak_staging_bytes(), requested.max(actual));
    }
}

#[test]
fn actual_many_lookup_cancellation_preserves_the_completed_reader_prefix() {
    let (scene, import) = many_fixture().into_parts();
    let original = source_snapshot(&scene);
    assert_eq!(original.len(), 3);
    let authored = [original[0].target, original[1].target, original[2].target];
    assert_eq!(MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS % 2, 0);
    let prefix = MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS / 2;
    let targets: Vec<_> = authored.into_iter().cycle().take(prefix + 2).collect();
    let recording = Recording::start(Fault::CancelLookup(prefix));
    let failure = crate::prepare_triangles(
        scene,
        &targets,
        TrianglePreparationOptions::default(),
        crate::PostProcessControl::default().with_cancellation_signal(SelectionCancellation),
    )
    .expect_err("the next existing control poll must stop the Many resolver");
    let trace = recording.finish();
    trace.assert_stopped(Leaf::Lookup, &targets[..prefix]);
    let frontier = trace.frontier.as_ref().unwrap();
    let storage = trace.storage.as_ref().unwrap();
    assert!(!frontier.readers.is_empty());
    assert_eq!(
        frontier.work,
        (
            storage.work.0 + 2 * prefix as u64,
            storage.work.1 + 2 * prefix as u64,
            0,
        )
    );
    let rejected = trace.rejected.as_ref().unwrap();
    assert_eq!(rejected.work, frontier.work);
    assert_eq!(rejected.polls, frontier.polls + 1);
    let failure = owned_unattached_failure(failure, import, &original);
    assert!(matches!(
        failure.cause(),
        PreparationCause::Control(crate::PostProcessControlCause::Cancelled {
            origin: crate::PostProcessCancellationOrigin::Signal,
        })
    ));
    let resources = failure.resources().unwrap();
    assert_eq!(resources.attempted_work_actions(), frontier.work.0);
    assert_eq!(resources.consumed_work_actions(), frontier.work.1);
    let requested = CapacityPlan::for_type::<PrimitiveRef<'_>>(0, 0, targets.len())
        .unwrap()
        .envelope_delta_bytes()
        .unwrap();
    let actual = (storage.capacity * size_of::<PrimitiveRef<'_>>()) as u64;
    assert_eq!(resources.peak_staging_bytes(), requested.max(actual));
}

#[test]
fn actual_many_semantic_lookup_error_completes_before_projection_without_cleanup_poll() {
    let (scene, import) = many_fixture().into_parts();
    let original = source_snapshot(&scene);
    assert_eq!(original.len(), 3);
    let foreign = crate::test_fixtures::canonical_triangle_scene();
    let invalid = foreign
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let expected_error = baozi_core::SceneLookupError::GenerationMismatch {
        kind: baozi_core::SceneEntityKind::Primitive,
        expected: scene.generation(),
        actual: invalid.generation(),
    };
    assert_ne!(scene.generation(), invalid.generation());
    let targets = [original[0].target, invalid, original[2].target];
    let recording = Recording::start(Fault::CancelLookupError);
    let (publication, (keys, completed)) = with_test_operation(
        &scene,
        crate::PostProcessControl::default().with_cancellation_signal(SelectionCancellation),
        |operation, keys| {
            let mut completed = (0, 0, 0);
            let mut polls_at_error = 0;
            let result = with_execution(operation, keys, |active| {
                let result = SelectedPrimitives::resolve(&scene, &targets, active, keys);
                assert!(matches!(
                    &result,
                    Err(PreparationCause::Target(cause)) if *cause == expected_error
                ));
                TRACE.with_borrow(|trace| {
                    let trace = trace.as_ref().unwrap();
                    let frontier = trace.frontier.as_ref().unwrap();
                    assert_eq!(trace.semantic_error, Some(expected_error));
                    assert!(trace.cancelled);
                    completed = frontier.work;
                    polls_at_error = frontier.polls;
                    assert_eq!(trace.polls, polls_at_error);
                });
                assert_eq!(active.work_counters(keys.work).unwrap(), completed);
                result.map(|_| ())
            });
            assert!(matches!(
                result,
                Err(PreparationCause::Target(cause)) if cause == expected_error
            ));
            TRACE.with_borrow(|trace| {
                assert_eq!(
                    trace.as_ref().unwrap().polls,
                    polls_at_error,
                    "known-error region cleanup must not poll"
                );
            });
            (keys, completed)
        },
    );
    let trace = recording.finish();
    trace.assert_storage_released(&[original[0].target]);
    assert!(trace.rejected.is_none());
    assert!(trace.current.is_none());
    let frontier = trace.frontier.as_ref().unwrap();
    let storage = trace.storage.as_ref().unwrap();
    assert_eq!(completed, (storage.work.0 + 3, storage.work.1 + 3, 0));
    let later: Vec<_> = trace.events[frontier.event_index..]
        .iter()
        .copied()
        .filter(|event| !matches!(event, Event::Cleanup(_)))
        .collect();
    assert_eq!(
        later,
        [
            Event::Before(Leaf::Lookup),
            Event::Entered(Leaf::Lookup),
            Event::Returned(Leaf::Lookup, true),
        ]
    );
    assert_eq!(source_snapshot(&scene), original);
    assert!(import.matches_scene(&scene));
    drop(scene);
    drop(import);
    drop(foreign);
    let FiniteCanonicalOperationPublication::EvidenceOnly(
        PreparedFiniteStoreEvidenceOnlyOutcome::MinimalFallback(fallback),
    ) = publication
    else {
        panic!("the original failed execution cannot publish success");
    };
    let facts = fallback.handoff().facts;
    let work = facts.dimension(keys.work).unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (completed.0, completed.1)
    );
    assert_eq!(facts.header().current_live_bytes(), 0);
    assert_eq!(facts.header().active_reservations(), 0);
    assert_eq!(
        facts.dimension(keys.scratch).unwrap().current_live_bytes(),
        0
    );
}
