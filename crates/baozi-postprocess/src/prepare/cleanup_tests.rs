use super::*;
use crate::operation::{PostProcessProgressPhase, PostProcessProgressUnit};
use baozi_core::__scene_integration::integration::SceneVectorValues;
use baozi_core::{AttributeData, AttributeDomain};
use baozi_operation::{ProgressDecision, ProgressEvent, StoreTerminalCause};
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
        _ => panic!("the fixture uses dense floating fields"),
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
