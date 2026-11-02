use crate::{FORMAT_ID, PlyImporter};
use baozi_core::{AttributeData, AttributeDomain, TopologyKind, ValidatedScene};
use baozi_import::{
    CauseOwner, ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportPhase,
    ImportRequest, ImportTerminalCause, ImportTerminalFailureClass, OptionCompileLimits,
    RegistryBuildLimits, RegistryBuilder, ScopeAbortCause,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationOrigin, CancellationToken, ControlCause};
use std::cell::RefCell;

const FACE_ROWS: usize = 1_025;
const FACE_TRIGGER: usize = 257;
const WIDE_INDICES: usize = 8_193;
const INDEX_TRIGGER: usize = 1;
const MAX_FURTHER_LEAVES: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PhysicalPrefix {
    face_rows: usize,
    indices: usize,
    last_index: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Observation {
    physical: PhysicalPrefix,
    raised_at: Option<PhysicalPrefix>,
    raises: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    FaceRow,
    Index,
}

struct Probe {
    target: Target,
    token: CancellationToken,
    observation: Observation,
}

thread_local! {
    static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
}

enum Completion {
    FaceRow(usize),
    Index { count: usize, value: u32 },
}

pub(crate) fn face_row_completed(count: usize) {
    completed(Completion::FaceRow(count));
}

pub(crate) fn face_index_completed(count: usize, value: u32) {
    completed(Completion::Index { count, value });
}

fn completed(completion: Completion) {
    let token = PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        let observation = &mut probe.observation;
        let at_target = match completion {
            Completion::FaceRow(count) => {
                observation.physical.face_rows = count;
                probe.target == Target::FaceRow && count == FACE_TRIGGER
            }
            Completion::Index { count, value } => {
                observation.physical.indices = count;
                observation.physical.last_index = Some(value);
                probe.target == Target::Index && count == INDEX_TRIGGER
            }
        };
        if at_target && observation.raised_at.is_none() {
            observation.raised_at = Some(observation.physical);
            observation.raises += 1;
            Some(probe.token.clone())
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
    }
}

fn with_probe<T>(target: Target, run: impl FnOnce(CancellationToken) -> T) -> (T, Observation) {
    struct Reset;

    impl Drop for Reset {
        fn drop(&mut self) {
            PROBE.with(|slot| {
                *slot.borrow_mut() = None;
            });
        }
    }

    let token = CancellationToken::new();
    PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(
            slot.is_none(),
            "PLY completion probe must be scoped to one import"
        );
        *slot = Some(Probe {
            target,
            token: token.clone(),
            observation: Observation::default(),
        });
    });
    let _reset = Reset;
    let result = run(token);
    let observation = PROBE.with(|slot| {
        slot.borrow()
            .as_ref()
            .expect("scoped PLY completion probe")
            .observation
    });
    (result, observation)
}

fn fixture(encoding: &str, wide_list: bool) -> Vec<u8> {
    assert!(matches!(
        encoding,
        "ascii" | "binary_little_endian" | "binary_big_endian"
    ));
    assert!(!wide_list || encoding != "ascii");
    const {
        assert!(FACE_ROWS > FACE_TRIGGER + MAX_FURTHER_LEAVES);
        assert!(WIDE_INDICES > INDEX_TRIGGER + MAX_FURTHER_LEAVES);
    }
    let face_count = if wide_list { 1 } else { FACE_ROWS };
    let count_type = if wide_list { "uint" } else { "uchar" };
    let mut bytes = format!(
        "ply\nformat {encoding} 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face {face_count}\nproperty list {count_type} int vertex_indices\nend_header\n"
    )
    .into_bytes();
    let body_start = bytes.len();
    if encoding == "ascii" {
        bytes.extend_from_slice(b"0 0 0\n1 0 0\n0 1 0\n");
        for _ in 0..face_count {
            bytes.extend_from_slice(b"3 0 1 2\n");
        }
        assert!(bytes.len() - body_start > 2 * 4_096);
        return bytes;
    }
    let big_endian = encoding == "binary_big_endian";
    for component in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bytes.extend_from_slice(&if big_endian {
            component.to_be_bytes()
        } else {
            component.to_le_bytes()
        });
    }
    let face_size = if wide_list { WIDE_INDICES } else { 3 };
    let count_bytes = if wide_list { 4 } else { 1 };
    for _ in 0..face_count {
        if wide_list {
            let count = u32::try_from(face_size).expect("bounded wide PLY list count");
            bytes.extend_from_slice(&if big_endian {
                count.to_be_bytes()
            } else {
                count.to_le_bytes()
            });
        } else {
            bytes.push(3);
        }
        // Canonical polygons admit repeated in-range indices; no triangulation is requested.
        for corner in 0..face_size {
            let index = (corner % 3) as i32;
            bytes.extend_from_slice(&if big_endian {
                index.to_be_bytes()
            } else {
                index.to_le_bytes()
            });
        }
    }
    assert_eq!(
        bytes.len() - body_start,
        3 * 3 * 4 + face_count * (count_bytes + face_size * 4)
    );
    bytes
}

fn assert_scene(scene: &ValidatedScene, wide_list: bool) {
    assert_eq!(scene.node_count(), 1);
    assert_eq!(scene.root_count(), 1);
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    assert_eq!(scene.mesh_instance_count(), 1);
    assert_eq!(scene.attribute_count(), 1);
    let mesh = scene.meshes().next().expect("one PLY mesh");
    let primitive = mesh.primitives().next().expect("one PLY primitive");
    let index_count = if wide_list {
        WIDE_INDICES
    } else {
        3 * FACE_ROWS
    };
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(primitive.corner_count() as usize, index_count);
    assert_eq!(
        primitive.face_count() as usize,
        if wide_list { 1 } else { FACE_ROWS }
    );
    assert_eq!(
        primitive.topology(),
        if wide_list {
            TopologyKind::Polygons
        } else {
            TopologyKind::TriangleList
        }
    );
    let indices = primitive.indices().expect("explicit PLY face indices");
    assert_eq!(indices.len(), index_count);
    for (corner, &index) in indices.iter().enumerate() {
        assert_eq!(index, (corner % 3) as u32);
    }
    if wide_list {
        assert_eq!(
            primitive.polygon_face_sizes(),
            Some(&[WIDE_INDICES as u32][..])
        );
    } else {
        assert!(primitive.polygon_face_sizes().is_none());
    }
    let positions = primitive.positions().expect("PLY vertex positions");
    assert_eq!(positions.domain(), AttributeDomain::Vertex);
    assert_eq!(positions.lanes(), 3);
    assert_eq!(positions.len(), 3);
    let AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
        panic!(
            "expected dense F32 PLY positions, got {:?}",
            positions.data()
        );
    };
    assert_eq!(values, &[0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    assert!(values.iter().all(|value| value.is_finite()));
}

fn assert_signal_failure(failure: &ImportFailure) {
    assert!(
        matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Control(ControlCause::Cancelled {
                        origin: CancellationOrigin::Signal,
                    })
                )
        ),
        "expected typed signal cancellation, got {:?}",
        failure.cause()
    );
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert_eq!(
        failure.selected_format().map(|id| id.as_str()),
        Some(FORMAT_ID)
    );
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    assert!(failure.diagnostics().is_empty());
    assert!(failure.loss_evidence().is_empty());
    let ledger = failure.ledger();
    assert!(matches!(
        ledger.terminal_cause(),
        ImportTerminalCause::Failure(terminal)
            if terminal.class() == ImportTerminalFailureClass::Control
    ));
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_evidence_bytes(), 0);
    assert_eq!(ledger.committed_canonical_members(), 0);
    for dimension in [
        ImportDimension::Records,
        ImportDimension::Vertices,
        ImportDimension::Faces,
        ImportDimension::TopologyCells,
        ImportDimension::CanonicalOutputBytes,
    ] {
        let row = ledger
            .dimension(dimension)
            .expect("configured PLY work/storage dimension");
        assert!(row.consumed() <= row.attempted(), "{dimension:?}: {row:?}");
        assert_eq!(row.current_live_bytes(), 0, "{dimension:?}");
        assert_eq!(row.committed(), 0, "{dimension:?}");
    }
}

fn assert_parser_cancellation(encoding: &str, wide_list: bool) {
    let bytes = fixture(encoding, wide_list);
    let source = AssetPath::new(format!("control-{encoding}-{wide_list}.ply"))
        .expect("valid PLY fixture path");
    let mut registry = RegistryBuilder::new();
    registry
        .register(PlyImporter)
        .expect("real PLY registration");
    let snapshot = registry
        .freeze(RegistryBuildLimits::default())
        .expect("PLY registry snapshot");
    let plan = snapshot
        .plan(&ImportRequest::default(), OptionCompileLimits::default())
        .expect("real PLY import plan");
    let baseline = plan
        .read_bytes(source.clone(), &bytes, ImportOperationControl::default())
        .expect("finite PLY fixture must succeed before cancellation is armed");
    assert_eq!(baseline.format().id().as_str(), FORMAT_ID);
    assert!(baseline.diagnostics().is_empty());
    assert!(baseline.loss_evidence().is_empty());
    let (retained_scene, retained_evidence) = baseline.into_parts();
    assert!(retained_evidence.matches_scene(&retained_scene));
    assert_scene(&retained_scene, wide_list);

    let target = if wide_list {
        Target::Index
    } else {
        Target::FaceRow
    };
    let (failure, observed) = with_probe(target, |token| {
        plan.read_bytes(
            source.clone(),
            &bytes,
            ImportOperationControl::default().with_cancellation(token),
        )
        .expect_err("completed parser prefix must trigger cancellation")
        .into_operation_failure()
        .expect("initialized operation failure")
    });
    assert_signal_failure(&failure);
    assert_eq!(observed.raises, 1, "completion trigger must fire once");
    let raised_at = observed.raised_at.expect("real parser completion trigger");
    let expected_trigger = if wide_list {
        PhysicalPrefix {
            face_rows: 0,
            indices: INDEX_TRIGGER,
            last_index: Some(0),
        }
    } else {
        PhysicalPrefix {
            face_rows: FACE_TRIGGER,
            indices: 3 * FACE_TRIGGER,
            last_index: Some(2),
        }
    };
    assert_eq!(raised_at, expected_trigger);
    let physical = observed.physical;
    assert!(physical.indices >= raised_at.indices);
    let total_indices = if wide_list {
        WIDE_INDICES
    } else {
        3 * FACE_ROWS
    };
    assert!(physical.indices < total_indices);
    assert_eq!(
        physical.last_index,
        Some(((physical.indices - 1) % 3) as u32)
    );
    let ledger = failure.ledger();
    for (dimension, consumed) in [
        (ImportDimension::Vertices, 3),
        (ImportDimension::Faces, physical.face_rows as u64),
        (ImportDimension::Records, 3 + physical.face_rows as u64),
        // An unfinished wide face has no completed polygon-count cell.
        (ImportDimension::TopologyCells, physical.indices as u64),
    ] {
        assert_eq!(
            ledger
                .dimension(dimension)
                .expect("PLY work prefix")
                .consumed(),
            consumed,
            "{encoding}, wide_list={wide_list}: {dimension:?}, {observed:?}"
        );
    }
    if wide_list {
        assert_eq!(physical.face_rows, 0);
        assert_eq!(
            ledger
                .dimension(ImportDimension::Records)
                .unwrap()
                .attempted(),
            4
        );
        assert_eq!(
            ledger
                .dimension(ImportDimension::Faces)
                .unwrap()
                .attempted(),
            1
        );
        assert_eq!(
            ledger
                .dimension(ImportDimension::TopologyCells)
                .unwrap()
                .attempted(),
            WIDE_INDICES as u64 + 1
        );
    }
    assert_scene(&retained_scene, wide_list);
    let fresh_token = CancellationToken::new();
    let fresh = plan
        .read_bytes(
            source.clone(),
            &bytes,
            ImportOperationControl::default().with_cancellation(fresh_token),
        )
        .expect("the same plan must succeed with fresh control after parser cancellation");
    assert_eq!(fresh.format().id().as_str(), FORMAT_ID);
    assert_scene(fresh.scene(), wide_list);
    if encoding == "ascii" {
        let cloned = plan
            .clone()
            .read_bytes(source, &bytes, ImportOperationControl::default())
            .expect("a cloned plan must remain usable after cancellation");
        assert_scene(cloned.scene(), wide_list);
    }
    drop(plan);
    drop(snapshot);
    drop(registry);
    drop(bytes);
    assert!(retained_evidence.matches_scene(&retained_scene));
    assert_scene(&retained_scene, wide_list);

    // Count completed index leaves, not parent Records/Faces or a whole unbounded list.
    assert!(
        physical.indices - raised_at.indices <= MAX_FURTHER_LEAVES,
        "{encoding}, wide_list={wide_list}: more than {MAX_FURTHER_LEAVES} further fixed index leaves: {observed:?}"
    );
}

#[test]
fn ascii_face_rows_cancel_after_completed_face_257() {
    assert_parser_cancellation("ascii", false);
}

#[test]
fn little_endian_face_rows_cancel_after_completed_face_257() {
    assert_parser_cancellation("binary_little_endian", false);
}

#[test]
fn big_endian_face_rows_cancel_after_completed_face_257() {
    assert_parser_cancellation("binary_big_endian", false);
}

#[test]
fn little_endian_wide_list_cancels_within_256_further_indices() {
    assert_parser_cancellation("binary_little_endian", true);
}

#[test]
fn big_endian_wide_list_cancels_within_256_further_indices() {
    assert_parser_cancellation("binary_big_endian", true);
}
