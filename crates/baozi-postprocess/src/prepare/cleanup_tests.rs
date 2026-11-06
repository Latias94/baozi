#![allow(
    clippy::result_large_err,
    reason = "test hooks retain the original inline runtime failure without allocating another owner"
)]

use super::*;
use crate::operation::runtime::{ActiveRuntime, RuntimeKeys};
use crate::operation::{PostProcessProgressPhase, PostProcessProgressUnit};
use baozi_core::__scene_integration::integration::SceneVectorValues;
use baozi_core::{AttributeData, AttributeDomain};
use baozi_operation::{
    CapacityPlan, CounterClass, DimensionPolicy, LedgerCause, ProgressDecision, ProgressEvent,
    StoreRuntimeError, StoreTerminalCause,
};
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Branch {
    Candidate,
    Rejected,
    NeverActivated,
    CentralRejected,
    Interrupted,
    MinimalFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    Planned,
    Materializing,
    BuildingEvidence,
    Validating,
    ValidationProgress,
    PublicationProgress,
    PreparedEditDropped,
    ScratchDropped {
        address: usize,
        len: usize,
        capacity: usize,
    },
    Preparing(Branch),
    Frozen(StoreTerminalCause),
    BeforePreparedOutput,
    PreparedPrefixReady(usize),
    PreparationReturned(bool),
    PreparationCancellationObserved,
    FinalProgressCancelled,
    ObserverDropped,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Frontier {
    OutputStorage,
    BackfillAppend,
    PreparationReturn,
    FinalProgress,
}

struct Buffer {
    name: &'static str,
    address: usize,
    len: usize,
}

#[derive(Default)]
struct Trace {
    buffers: Vec<Buffer>,
    events: Vec<Event>,
    corner_origins: Vec<u32>,
    face_offsets: Vec<u32>,
    normal_bits: Vec<u64>,
    frontier: Option<Frontier>,
    keys: Option<RuntimeKeys>,
    work_before: Option<(u64, u64, u64)>,
    work_after_tightening: Option<(u64, u64, u64)>,
    live_capacity_bytes: u64,
    output_reservation_bytes: u64,
    pending_target: Option<PrimitiveId>,
    prefix_targets: Vec<PrimitiveId>,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

// Record only scalar facts about the real owners. No callback or substitute owner runs here.
pub(crate) fn record(event: Event) {
    TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace {
            trace.events.push(event);
        }
    });
}

pub(super) fn before_prepared_output(
    operation: &mut ActiveRuntime<'_>,
    keys: RuntimeKeys,
    prepared: &input::PreparedPrimitive,
    sorted: &Vec<PrimitiveId>,
    selected: &Vec<baozi_core::PrimitiveRef<'_>>,
) -> Result<(), StoreRuntimeError> {
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace.as_mut().filter(|trace| trace.frontier.is_some()) else {
            return Ok(());
        };
        assert!(trace.buffers.is_empty());
        trace.events.clear();
        trace.events.push(Event::BeforePreparedOutput);
        trace.keys = Some(keys);
        trace.pending_target = Some(prepared.target);
        trace.buffers.push(Buffer {
            name: "sorted",
            address: sorted.as_ptr() as usize,
            len: sorted.len(),
        });
        trace.buffers.push(Buffer {
            name: "selected",
            address: selected.as_ptr() as usize,
            len: selected.len(),
        });
        assert!(prepared.normals.is_none());
        assert!(prepared.tangents.is_none());
        assert!(!prepared.remove_tangents);
        let origins = prepared.origins.as_ref().expect("pending lowered payload");
        trace.buffers.push(Buffer {
            name: "triangles",
            address: origins.triangles.values().as_ptr() as usize,
            len: origins.triangles.values().len(),
        });
        trace.buffers.push(Buffer {
            name: "face_offsets",
            address: origins.face_offsets.values().as_ptr() as usize,
            len: origins.face_offsets.values().len(),
        });
        trace
            .corner_origins
            .extend(origins.triangles.values().iter().flatten());
        trace.face_offsets.extend(origins.face_offsets.values());
        trace.live_capacity_bytes = u64::try_from(
            sorted.capacity() * size_of::<PrimitiveId>()
                + selected.capacity() * size_of::<baozi_core::PrimitiveRef<'_>>()
                + origins.triangles.values().capacity() * size_of::<[u32; 3]>()
                + origins.face_offsets.values().capacity() * size_of::<u32>(),
        )
        .unwrap();
        trace.output_reservation_bytes =
            CapacityPlan::for_type::<input::PreparedPrimitive>(0, 0, sorted.len())
                .unwrap()
                .envelope_delta_bytes()
                .unwrap();
        if trace.frontier == Some(Frontier::OutputStorage) {
            trace.work_before = Some(operation.work_counters(keys.work)?);
            operation
                .tighten_dimension_policy(
                    keys.scratch,
                    DimensionPolicy::reservable_bytes(0, 0, 0, trace.live_capacity_bytes),
                )
                .expect("the actual currently live capacities must fit before output admission");
            trace.work_after_tightening = Some(operation.work_counters(keys.work)?);
            assert_eq!(trace.work_before, trace.work_after_tightening);
        }
        Ok(())
    })
}

pub(super) fn prepared_prefix_ready(
    operation: &mut ActiveRuntime<'_>,
    keys: RuntimeKeys,
    prepared: &input::PreparedPrimitive,
    output: &ScratchVec<input::PreparedPrimitive>,
) -> Result<(), StoreRuntimeError> {
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace.as_mut().filter(|trace| trace.frontier.is_some()) else {
            return Ok(());
        };
        let prefix = output.values();
        trace.events.push(Event::PreparedPrefixReady(prefix.len()));
        assert_eq!(trace.pending_target, Some(prepared.target));
        for previous in prefix {
            assert!(previous.origins.is_none());
            assert!(previous.normals.is_none());
            assert!(previous.tangents.is_none());
            assert!(!previous.remove_tangents);
        }
        if trace.frontier == Some(Frontier::BackfillAppend) {
            assert_eq!(prefix.len(), 1, "deny the second physical backfill append");
            trace.buffers.push(Buffer {
                name: "prepared",
                address: prefix.as_ptr() as usize,
                len: prefix.len(),
            });
            trace
                .prefix_targets
                .extend(prefix.iter().map(|row| row.target));
            let before = operation.work_counters(keys.work)?;
            assert_eq!(before.2, 0);
            trace.work_before = Some(before);
            operation
                .tighten_dimension_policy(
                    keys.work,
                    DimensionPolicy::counter(before.0 + 2, before.1 + 1, 0),
                )
                .expect("tightening must retain the literal completed prefix");
            trace.work_after_tightening = Some(operation.work_counters(keys.work)?);
            assert_eq!(trace.work_after_tightening, Some(before));
        }
        Ok(())
    })
}

pub(super) fn preparation_returned(prepared: &input::PreparedSelection) {
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace.as_mut().filter(|trace| trace.frontier.is_some()) else {
            return;
        };
        let changed = match prepared {
            input::PreparedSelection::Unchanged => false,
            input::PreparedSelection::Prepared(output) => {
                trace.buffers.push(Buffer {
                    name: "prepared",
                    address: output.values().as_ptr() as usize,
                    len: output.values().len(),
                });
                trace
                    .prefix_targets
                    .extend(output.values().iter().map(|row| row.target));
                true
            }
        };
        trace.events.push(Event::PreparationReturned(changed));
    });
}

pub(super) fn selection_ready(
    prepared: &ScratchVec<input::PreparedPrimitive>,
    replacements: &[SceneGeometryReplacement<'_>],
) {
    TRACE.with_borrow_mut(|trace| {
        let Some(trace) = trace else { return };
        // Earlier preparation temporaries may have reused the same allocator addresses.
        // Track destruction only after these selected buffers are all live together.
        trace.events.clear();
        trace.buffers.push(Buffer {
            name: "prepared",
            address: prepared.values().as_ptr() as usize,
            len: prepared.values().len(),
        });
        trace.buffers.push(Buffer {
            name: "replacements",
            address: replacements.as_ptr() as usize,
            len: replacements.len(),
        });
        for primitive in prepared.values() {
            if let Some(origins) = &primitive.origins {
                trace.buffers.push(Buffer {
                    name: "triangles",
                    address: origins.triangles.values().as_ptr() as usize,
                    len: origins.triangles.values().len(),
                });
                trace.buffers.push(Buffer {
                    name: "face_offsets",
                    address: origins.face_offsets.values().as_ptr() as usize,
                    len: origins.face_offsets.values().len(),
                });
                trace
                    .corner_origins
                    .extend(origins.triangles.values().iter().flatten());
                trace.face_offsets.extend(origins.face_offsets.values());
            }
            if let Some(normals) = &primitive.normals {
                let (address, len) = match normals.values() {
                    SceneVectorValues::F32(values) => {
                        trace
                            .normal_bits
                            .extend(values.iter().flatten().map(|v| u64::from(v.to_bits())));
                        (values.as_ptr() as usize, values.len())
                    }
                    SceneVectorValues::F64(values) => {
                        trace
                            .normal_bits
                            .extend(values.iter().flatten().map(|v| v.to_bits()));
                        (values.as_ptr() as usize, values.len())
                    }
                };
                trace.buffers.push(Buffer {
                    name: "normals",
                    address,
                    len,
                });
            }
        }
    });
}

struct Recording;

impl Recording {
    fn start() -> Self {
        TRACE.with_borrow_mut(|trace| {
            assert!(trace.replace(Trace::default()).is_none());
        });
        Self
    }

    fn start_at(frontier: Frontier) -> Self {
        let recording = Self::start();
        TRACE.with_borrow_mut(|trace| trace.as_mut().unwrap().frontier = Some(frontier));
        recording
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

struct CancelAt(Event);

impl baozi_operation::CancellationSignal for CancelAt {
    fn is_cancelled(&self) -> bool {
        TRACE.with_borrow(|trace| {
            trace
                .as_ref()
                .is_some_and(|trace| trace.events.contains(&self.0))
        })
    }
}

fn progress(
    cancel_validation: bool,
) -> impl baozi_operation::ProgressObserver<PostProcessProgressPhase, PostProcessProgressUnit> {
    move |event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>| {
        match event.phase() {
            PostProcessProgressPhase::Validation => {
                record(Event::ValidationProgress);
                if cancel_validation {
                    return ProgressDecision::Cancel;
                }
            }
            PostProcessProgressPhase::Publication => record(Event::PublicationProgress),
        }
        ProgressDecision::Continue
    }
}

fn options() -> TrianglePreparationOptions {
    TrianglePreparationOptions {
        normals: Some(crate::NormalOptions {
            mode: crate::NormalMode::Flat,
            existing: crate::ExistingNormals::Replace,
        }),
        ..Default::default()
    }
}

fn target(scene: &ValidatedScene) -> PrimitiveId {
    scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id()
}

fn bits(attribute: baozi_core::AttributeRef<'_>) -> Vec<u64> {
    match attribute.data() {
        AttributeData::DenseF32 { values, .. } => {
            values.iter().map(|v| u64::from(v.to_bits())).collect()
        }
        AttributeData::DenseF64 { values, .. } => values.iter().map(|v| v.to_bits()).collect(),
        AttributeData::DenseU32 { values, .. } => values.iter().copied().map(u64::from).collect(),
        _ => panic!("the fixture uses dense floating or unsigned fields"),
    }
}

impl Trace {
    fn at(&self, event: Event) -> usize {
        self.events
            .iter()
            .position(|seen| *seen == event)
            .unwrap_or_else(|| panic!("missing {event:?}: {:?}", self.events))
    }

    fn dropped(&self, name: &str) -> usize {
        let buffer = self
            .buffers
            .iter()
            .find(|buffer| buffer.name == name)
            .unwrap();
        assert!(buffer.len > 0, "{name} must own actual nonempty storage");
        let mut matches =
            self.events
                .iter()
                .enumerate()
                .filter_map(|(index, event)| match *event {
                    Event::ScratchDropped {
                        address,
                        len,
                        capacity,
                    } if address == buffer.address => {
                        assert_eq!(len, buffer.len, "{name} length at actual destruction");
                        assert!(capacity >= len);
                        Some(index)
                    }
                    _ => None,
                });
        let dropped = matches
            .next()
            .unwrap_or_else(|| panic!("{name} was not released"));
        assert!(matches.next().is_none(), "{name} must release exactly once");
        dropped
    }

    fn assert_cleanup(&self, branch: Branch) -> StoreTerminalCause {
        assert_eq!(
            self.buffers.len(),
            5,
            "prepared, replacements, both origins and generated normals"
        );
        assert_eq!(self.corner_origins.len(), 6);
        assert_eq!(self.face_offsets, [0, 2]);
        assert_eq!(self.normal_bits.len(), 18);
        let preparing = self.at(Event::Preparing(branch));
        assert_eq!(
            self.events
                .iter()
                .filter(|event| matches!(event, Event::Preparing(_)))
                .count(),
            1
        );
        let mapping_dropped = self.at(Event::PreparedEditDropped);
        // Planned follows successful selected preflight: core initialized the nonempty mapping
        // rows. PreparedEdit::Selection owns that real C storage; its Drop releases the mapping
        // Vec and reservation before the immediately following PreparedEditDropped observation.
        assert!(self.at(Event::Planned) < mapping_dropped);
        let replacements = self.dropped("replacements");
        let prepared = self.dropped("prepared");
        assert!(mapping_dropped < replacements);
        for name in ["triangles", "face_offsets", "normals"] {
            let dropped = self.dropped(name);
            assert!(
                replacements < dropped && dropped < prepared,
                "{name} cleanup order"
            );
        }
        assert!(
            prepared < preparing,
            "real scratch must be gone before terminal preparation"
        );
        let mut frozen = self.events.iter().enumerate().filter_map(|(index, event)| {
            if let Event::Frozen(cause) = event {
                Some((index, *cause))
            } else {
                None
            }
        });
        let (index, cause) = frozen.next().expect("actual frozen terminal ledger");
        assert!(preparing < index);
        assert!(frozen.next().is_none());
        cause
    }
}

#[test]
fn changed_selection_drops_real_scratch_before_candidate_publication_and_freeze() {
    let (scene, import) = crate::test_fixtures::canonical_quad_import().into_parts();
    let input_target = target(&scene);
    let input_bits = bits(scene.primitive(input_target).unwrap().positions().unwrap());
    let input_indices = scene
        .primitive(input_target)
        .unwrap()
        .indices()
        .unwrap()
        .to_vec();
    let recording = Recording::start();
    let output = prepare_triangles(
        scene,
        &[input_target],
        options(),
        PostProcessControl::default().with_progress_observer(progress(false)),
    )
    .unwrap();
    let trace = recording.finish();
    assert_eq!(
        trace.assert_cleanup(Branch::Candidate),
        StoreTerminalCause::Success
    );
    assert!(trace.at(Event::ValidationProgress) < trace.at(Event::PreparedEditDropped));
    assert!(trace.at(Event::Preparing(Branch::Candidate)) < trace.at(Event::PublicationProgress));
    assert!(
        trace.at(Event::PublicationProgress) < trace.at(Event::Frozen(StoreTerminalCause::Success))
    );
    let (scene, report) = output.into_parts();
    assert_ne!(scene.generation(), input_target.generation());
    assert!(report.matches_scene(&scene));
    assert_eq!(report.input_generation(), input_target.generation());
    let evidence = report.geometry_preparation().unwrap();
    assert_eq!(evidence.geometry_reconstruction_count(), 1);
    let selected = evidence.target(0).unwrap();
    assert_eq!(
        selected.disposition(),
        crate::PostProcessRunDisposition::Changed
    );
    assert_eq!(selected.input_target(), input_target);
    assert_eq!(selected.output_target().generation(), scene.generation());
    assert_eq!(
        selected.source_corner_origins().unwrap(),
        trace.corner_origins
    );
    assert_eq!(selected.source_face_triangle_offsets().unwrap(), &[0, 2]);
    let primitive = scene.primitive(selected.output_target()).unwrap();
    assert_eq!(bits(primitive.positions().unwrap()), input_bits);
    assert_eq!(
        bits(primitive.normals(AttributeDomain::Corner).unwrap()),
        trace.normal_bits
    );
    let expected_indices: Vec<_> = trace
        .corner_origins
        .iter()
        .map(|&corner| input_indices[corner as usize])
        .collect();
    assert_eq!(primitive.indices().unwrap(), expected_indices);
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
    drop(scene);
    drop(import);
    std::thread::spawn(move || {
        let selected = report.geometry_preparation().unwrap().target(0).unwrap();
        assert_eq!(selected.input_target(), input_target);
        assert_eq!(
            selected.source_corner_origins().unwrap(),
            trace.corner_origins
        );
        assert_eq!(selected.source_face_triangle_offsets().unwrap(), &[0, 2]);
        assert!(report.resources().published_outcome_evidence_bytes() > 0);
    })
    .join()
    .unwrap();
}

enum ExpectedFailure {
    Explicit(crate::PostProcessCancellationOrigin),
    FinalControl,
}

fn assert_failure(control: PostProcessControl, branch: Branch, expected: ExpectedFailure) -> Trace {
    let (scene, import) = crate::test_fixtures::canonical_quad_import().into_parts();
    let input_target = target(&scene);
    let input_bits = bits(scene.primitive(input_target).unwrap().positions().unwrap());
    let input_indices = scene
        .primitive(input_target)
        .unwrap()
        .indices()
        .unwrap()
        .to_vec();
    let recording = Recording::start();
    let failure = prepare_triangles(scene, &[input_target], options(), control).unwrap_err();
    let trace = recording.finish();
    let StoreTerminalCause::Failure(terminal) = trace.assert_cleanup(branch) else {
        panic!("a failure must freeze failure evidence")
    };
    assert_eq!(
        terminal.class(),
        baozi_operation::StorePreparationFailureClass::Control
    );
    // The production terminal vocabulary assigns code 1 to cancellation (both origins).
    assert_eq!(terminal.code(), 1);
    assert!(!trace.events.contains(&Event::PublicationProgress));
    assert_eq!(failure.input().generation(), input_target.generation());
    assert!(import.matches_scene(failure.input()));
    let primitive = failure.input().primitive(input_target).unwrap();
    assert_eq!(bits(primitive.positions().unwrap()), input_bits);
    assert_eq!(primitive.indices().unwrap(), input_indices);
    assert!(primitive.normals(AttributeDomain::Corner).is_none());
    let (input, error) = failure.into_parts();
    drop(input);
    drop(import);
    let PostProcessRunError::Preparation { failure } = error else {
        panic!("typed preparation failure")
    };
    match expected {
        ExpectedFailure::Explicit(origin) => assert!(
            matches!(failure.cause(), PreparationCause::Control(crate::PostProcessControlCause::Cancelled { origin: selected }) if *selected == origin),
            "expected explicit {origin:?} cancellation, got {:?}",
            failure.cause()
        ),
        ExpectedFailure::FinalControl => {
            assert_eq!(branch, Branch::CentralRejected);
            // Observer cancellation precedes validator entry, so no callback carries an explicit
            // Control cause. The existing terminal projection uses the frozen Control header.
            assert!(
                matches!(failure.cause(), PreparationCause::FinalControl),
                "expected FinalControl for pre-validator observer cancellation, got {:?}",
                failure.cause()
            );
        }
    }
    assert!(matches!(&failure.owner, FailureOwner::Fallback { .. }));
    let resources = failure
        .resources()
        .expect("the real fallback retains its ledger");
    assert_eq!(resources.committed_canonical_bytes(), 0);
    assert_eq!(resources.published_outcome_evidence_bytes(), 0);
    assert_eq!(resources.published_terminal_evidence_records(), 2);
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    trace
}

#[test]
fn changed_selection_drops_real_scratch_before_activation_rejection_preparation() {
    let trace = assert_failure(
        PostProcessControl::default()
            .with_cancellation_signal(CancelAt(Event::Materializing))
            .with_progress_observer(progress(false)),
        Branch::Rejected,
        ExpectedFailure::Explicit(crate::PostProcessCancellationOrigin::Signal),
    );
    trace.at(Event::Materializing);
    assert!(!trace.events.contains(&Event::BuildingEvidence));
    assert!(!trace.events.contains(&Event::ValidationProgress));
}

#[test]
fn changed_selection_drops_real_scratch_before_never_activated_preparation() {
    let trace = assert_failure(
        PostProcessControl::default()
            .with_cancellation_signal(CancelAt(Event::Planned))
            .with_progress_observer(progress(false)),
        Branch::NeverActivated,
        ExpectedFailure::Explicit(crate::PostProcessCancellationOrigin::Signal),
    );
    assert!(!trace.events.contains(&Event::Materializing));
    assert!(!trace.events.contains(&Event::BuildingEvidence));
    assert!(!trace.events.contains(&Event::ValidationProgress));
}

#[test]
fn changed_selection_drops_real_scratch_before_central_rejection_preparation() {
    let trace = assert_failure(
        PostProcessControl::default().with_progress_observer(progress(true)),
        Branch::CentralRejected,
        ExpectedFailure::FinalControl,
    );
    assert!(trace.at(Event::BuildingEvidence) < trace.at(Event::ValidationProgress));
    assert!(trace.at(Event::ValidationProgress) < trace.at(Event::PreparedEditDropped));
    assert!(
        !trace.events.contains(&Event::Validating),
        "observer cancellation must reject before the validation callback"
    );
}

#[test]
fn changed_selection_drops_real_scratch_before_central_interruption_preparation() {
    let trace = assert_failure(
        PostProcessControl::default()
            .with_cancellation_signal(CancelAt(Event::Validating))
            .with_progress_observer(progress(false)),
        Branch::Interrupted,
        ExpectedFailure::Explicit(crate::PostProcessCancellationOrigin::Signal),
    );
    assert!(trace.at(Event::BuildingEvidence) < trace.at(Event::ValidationProgress));
    assert!(trace.at(Event::ValidationProgress) < trace.at(Event::Validating));
    assert!(trace.at(Event::Validating) < trace.at(Event::PreparedEditDropped));
}

#[test]
fn changed_selection_drops_real_scratch_before_pending_minimal_fallback_preparation() {
    let trace = assert_failure(
        PostProcessControl::default()
            .with_cancellation_signal(CancelAt(Event::BuildingEvidence))
            .with_progress_observer(progress(false)),
        Branch::MinimalFallback,
        ExpectedFailure::Explicit(crate::PostProcessCancellationOrigin::Signal),
    );
    assert!(trace.at(Event::Materializing) < trace.at(Event::BuildingEvidence));
    assert!(
        !trace.events.contains(&Event::ValidationProgress),
        "evidence failure returns genuine PendingMinimalFallback before central validation"
    );
}

fn delayed_output_fixture() -> baozi_import::ImportReport {
    crate::test_fixtures::import_obj(
        b"v -0.0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
          vt -0.0 0.25\nvt 1 0.25\nvt 1 1\nvt 0 1\nvn -0.0 0 1\n\
          o prefix-first\nf 1/1/1 2/2/1 3/3/1\n\
          o prefix-second\nf 1/1/1 3/3/1 4/4/1\n\
          o pending-quad\nf 1/1/1 2/2/1 3/3/1 4/4/1\n\
          o unselected\nf 2/2/1 3/3/1 4/4/1\n",
    )
}

#[derive(Debug, PartialEq, Eq)]
struct PrimitiveSnapshot {
    target: PrimitiveId,
    topology: baozi_core::TopologyKind,
    indices: Vec<u32>,
    face_sizes: Option<Vec<u32>>,
    attribute_layout: Vec<String>,
    attribute_bits: Vec<Vec<u64>>,
}

fn primitive_snapshots(scene: &ValidatedScene) -> Vec<PrimitiveSnapshot> {
    scene
        .meshes()
        .flat_map(|mesh| mesh.primitives())
        .map(|primitive| PrimitiveSnapshot {
            target: primitive.id(),
            topology: primitive.topology(),
            indices: primitive.indices().unwrap().to_vec(),
            face_sizes: primitive.polygon_face_sizes().map(<[u32]>::to_vec),
            attribute_layout: primitive
                .attributes()
                .map(|attribute| {
                    format!(
                        "{:?}/{:?}/{}",
                        attribute.semantic(),
                        attribute.domain(),
                        attribute.lanes()
                    )
                })
                .collect(),
            attribute_bits: primitive.attributes().map(bits).collect(),
        })
        .collect()
}

fn unattached_failure_after_source_drop(
    failure: PostProcessRunFailure,
    import: baozi_import::ImportEvidence,
    original: &[PrimitiveSnapshot],
) -> PreparationFailure {
    assert_eq!(
        failure.input().generation(),
        original[0].target.generation()
    );
    assert!(import.matches_scene(failure.input()));
    assert_eq!(primitive_snapshots(failure.input()), original);
    let (scene, error) = failure.into_parts();
    drop(scene);
    drop(import);
    let PostProcessRunError::Preparation { failure } = error else {
        panic!("typed preparation failure");
    };
    assert!(matches!(&failure.owner, FailureOwner::None));
    let resources = failure.resources().expect("the original unattached ledger");
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert_eq!(resources.committed_canonical_bytes(), 0);
    assert_eq!(resources.published_outcome_evidence_bytes(), 0);
    assert_eq!(resources.published_terminal_evidence_records(), 0);
    assert!(resources.peak_staging_bytes() > 0);
    failure
}

impl Trace {
    fn assert_unattached(&self) {
        assert!(!self.events.iter().any(|event| matches!(
            event,
            Event::Planned
                | Event::Materializing
                | Event::BuildingEvidence
                | Event::Validating
                | Event::ValidationProgress
                | Event::PublicationProgress
                | Event::PreparedEditDropped
                | Event::Preparing(_)
        )));
        assert!(
            !self
                .buffers
                .iter()
                .any(|buffer| buffer.name == "replacements")
        );
    }

    fn assert_unchanged_selection_dropped(&self) {
        let returned = self.at(Event::PreparationReturned(false));
        let drops: Vec<_> = self
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                Event::ScratchDropped {
                    address,
                    len,
                    capacity,
                } => {
                    assert!(
                        index < returned,
                        "selection cleanup precedes producer return"
                    );
                    assert_eq!(*len, 2);
                    assert!(*capacity >= *len);
                    Some(*address)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            drops.len(),
            2,
            "only real sorted and selected scratch owners"
        );
        assert_ne!(drops[0], drops[1]);
        assert!(self.buffers.is_empty());
        self.assert_unattached();
    }
}

#[test]
fn first_lowered_payload_drops_when_actual_delayed_output_storage_is_denied() {
    let (scene, import) = delayed_output_fixture().into_parts();
    let original = primitive_snapshots(&scene);
    assert_eq!(original.len(), 4);
    assert_eq!(original[0].topology, baozi_core::TopologyKind::TriangleList);
    assert_eq!(original[1].topology, baozi_core::TopologyKind::TriangleList);
    assert_eq!(original[2].topology, baozi_core::TopologyKind::Polygons);
    let targets = [original[2].target, original[0].target, original[1].target];
    let recording = Recording::start_at(Frontier::OutputStorage);
    let failure = prepare_triangles(
        scene,
        &targets,
        TrianglePreparationOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap_err();
    let trace = recording.finish();
    trace.assert_unattached();
    assert_eq!(trace.pending_target, Some(original[2].target));
    assert_eq!(trace.buffers.len(), 4);
    assert_eq!(trace.corner_origins.len(), 6);
    assert_eq!(trace.face_offsets, [0, 2]);
    assert!(!trace.events.iter().any(|event| matches!(
        event,
        Event::PreparedPrefixReady(_) | Event::PreparationReturned(_)
    )));
    for name in ["triangles", "face_offsets"] {
        assert!(trace.at(Event::BeforePreparedOutput) < trace.dropped(name));
        assert!(trace.dropped(name) < trace.dropped("selected"));
    }
    assert!(trace.dropped("selected") < trace.dropped("sorted"));
    let failure = unattached_failure_after_source_drop(failure, import, &original);
    let PreparationCause::Runtime(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
        dimension,
        class,
        value,
        limit,
    })) = failure.cause()
    else {
        panic!("the real output storage admission must fail: {failure:?}");
    };
    assert_eq!(*dimension, trace.keys.unwrap().scratch);
    assert_eq!(*class, CounterClass::Live);
    assert_eq!(*limit, trace.live_capacity_bytes);
    assert_eq!(*value, *limit + trace.output_reservation_bytes);
    let before = trace.work_before.unwrap();
    assert_eq!(before.2, 0);
    assert_eq!(trace.work_after_tightening, Some(before));
    let resources = failure.resources().unwrap();
    assert_eq!(resources.attempted_work_actions(), before.0);
    assert_eq!(resources.consumed_work_actions(), before.1);
}

#[test]
fn denied_second_backfill_append_drops_real_initialized_prefix_and_pending_lowered_payload() {
    let (scene, import) = delayed_output_fixture().into_parts();
    let original = primitive_snapshots(&scene);
    let targets = [original[2].target, original[1].target, original[0].target];
    let recording = Recording::start_at(Frontier::BackfillAppend);
    let failure = prepare_triangles(
        scene,
        &targets,
        TrianglePreparationOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap_err();
    let trace = recording.finish();
    trace.assert_unattached();
    assert_eq!(trace.pending_target, Some(original[2].target));
    assert_eq!(trace.prefix_targets, [original[0].target]);
    assert_eq!(trace.buffers.len(), 5);
    assert_eq!(trace.corner_origins.len(), 6);
    assert_eq!(trace.face_offsets, [0, 2]);
    assert!(trace.at(Event::BeforePreparedOutput) < trace.at(Event::PreparedPrefixReady(1)));
    assert!(trace.at(Event::PreparedPrefixReady(1)) < trace.dropped("prepared"));
    for name in ["triangles", "face_offsets"] {
        assert!(trace.dropped("prepared") < trace.dropped(name));
        assert!(trace.dropped(name) < trace.dropped("selected"));
    }
    assert!(trace.dropped("selected") < trace.dropped("sorted"));
    assert!(!trace.events.iter().any(|event| matches!(
        event,
        Event::PreparedPrefixReady(2..) | Event::PreparationReturned(_)
    )));
    let failure = unattached_failure_after_source_drop(failure, import, &original);
    let PreparationCause::Runtime(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
        dimension,
        class,
        value,
        limit,
    })) = failure.cause()
    else {
        panic!("the second backfill append admission must fail: {failure:?}");
    };
    let before = trace.work_before.unwrap();
    assert_eq!(before.2, 0);
    assert_eq!(trace.work_after_tightening, Some(before));
    assert_eq!(*dimension, trace.keys.unwrap().work);
    assert_eq!(*class, CounterClass::Consumed);
    assert_eq!((*value, *limit), (before.1 + 2, before.1 + 1));
    let resources = failure.resources().unwrap();
    assert_eq!(resources.attempted_work_actions(), before.0 + 2);
    assert_eq!(resources.consumed_work_actions(), before.1 + 1);
}

struct CancelAfterPreparation;

impl baozi_operation::CancellationSignal for CancelAfterPreparation {
    fn is_cancelled(&self) -> bool {
        TRACE.with_borrow_mut(|trace| {
            let Some(trace) = trace else { return false };
            if !trace
                .events
                .iter()
                .any(|event| matches!(event, Event::PreparationReturned(_)))
            {
                return false;
            }
            trace.events.push(Event::PreparationCancellationObserved);
            true
        })
    }
}

#[test]
fn successful_preparation_exit_cancels_both_real_results_before_attachment() {
    for changed in [false, true] {
        let (scene, import) = delayed_output_fixture().into_parts();
        let original = primitive_snapshots(&scene);
        let count = if changed { 3 } else { 2 };
        let targets: Vec<_> = original[..count]
            .iter()
            .rev()
            .map(|primitive| primitive.target)
            .collect();
        let recording = Recording::start_at(Frontier::PreparationReturn);
        let failure = prepare_triangles(
            scene,
            &targets,
            TrianglePreparationOptions::default(),
            PostProcessControl::default().with_cancellation_signal(CancelAfterPreparation),
        )
        .unwrap_err();
        let trace = recording.finish();
        trace.assert_unattached();
        let returned = trace.at(Event::PreparationReturned(changed));
        let cancelled = trace.at(Event::PreparationCancellationObserved);
        assert!(returned < cancelled);
        assert_eq!(
            trace
                .events
                .iter()
                .filter(|event| **event == Event::PreparationCancellationObserved)
                .count(),
            1,
            "known-error terminal cleanup must not poll again"
        );
        if changed {
            assert_eq!(
                trace.prefix_targets,
                original[..3]
                    .iter()
                    .map(|primitive| primitive.target)
                    .collect::<Vec<_>>()
            );
            assert_eq!(trace.pending_target, Some(original[2].target));
            assert_eq!(trace.corner_origins.len(), 6);
            assert_eq!(trace.face_offsets, [0, 2]);
            assert!(trace.dropped("selected") < returned);
            assert!(trace.dropped("sorted") < returned);
            for name in ["triangles", "face_offsets"] {
                assert!(cancelled < trace.dropped(name));
                assert!(trace.dropped(name) < trace.dropped("prepared"));
            }
        } else {
            trace.assert_unchanged_selection_dropped();
        }
        let failure = unattached_failure_after_source_drop(failure, import, &original);
        assert!(matches!(
            failure.cause(),
            PreparationCause::Control(crate::PostProcessControlCause::Cancelled {
                origin: crate::PostProcessCancellationOrigin::Signal,
            })
        ));
    }
}

struct CancelFinalProgress;

impl baozi_operation::ProgressObserver<PostProcessProgressPhase, PostProcessProgressUnit>
    for CancelFinalProgress
{
    fn observe(
        &mut self,
        event: ProgressEvent<PostProcessProgressPhase, PostProcessProgressUnit>,
    ) -> ProgressDecision {
        assert_eq!(event.phase(), PostProcessProgressPhase::Publication);
        assert_eq!(event.unit(), PostProcessProgressUnit::Scene);
        assert_eq!((event.completed(), event.total()), (1, Some(1)));
        TRACE.with_borrow(|trace| {
            let trace = trace.as_ref().expect("explicit final-progress recording");
            assert!(trace.frontier == Some(Frontier::FinalProgress));
            trace.assert_unchanged_selection_dropped();
        });
        record(Event::FinalProgressCancelled);
        ProgressDecision::Cancel
    }
}

impl Drop for CancelFinalProgress {
    fn drop(&mut self) {
        record(Event::ObserverDropped);
    }
}

#[test]
fn unattached_final_observer_cancels_after_selection_cleanup_and_keeps_owned_final_control() {
    let (scene, import) = delayed_output_fixture().into_parts();
    let original = primitive_snapshots(&scene);
    let targets = [original[1].target, original[0].target];
    let recording = Recording::start_at(Frontier::FinalProgress);
    let failure = prepare_triangles(
        scene,
        &targets,
        TrianglePreparationOptions::default(),
        PostProcessControl::default().with_progress_observer(CancelFinalProgress),
    )
    .unwrap_err();
    let trace = recording.finish();
    trace.assert_unchanged_selection_dropped();
    assert!(trace.at(Event::PreparationReturned(false)) < trace.at(Event::FinalProgressCancelled));
    assert!(trace.at(Event::FinalProgressCancelled) < trace.at(Event::ObserverDropped));
    assert_eq!(
        trace
            .events
            .iter()
            .filter(|event| **event == Event::ObserverDropped)
            .count(),
        1
    );
    let failure = unattached_failure_after_source_drop(failure, import, &original);
    assert!(matches!(failure.cause(), PreparationCause::FinalControl));
    let resources = failure.resources().unwrap();
    assert!(resources.consumed_work_actions() > 0);
    assert_eq!(
        resources.attempted_work_actions(),
        resources.consumed_work_actions()
    );
}
