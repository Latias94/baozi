#![cfg(any(
    feature = "format-obj",
    feature = "format-stl",
    feature = "format-ply",
    feature = "format-gltf"
))]

// Public boundary assurance only: Validate follows parsing; it is not a parser-loop probe.
use baozi::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenWorkError, AssetPath, AssetReadError,
    AssetReader, AttributeData, AttributeDomain, AttributeSemanticRef, AuthorityDomain,
    CancellationOrigin, CancellationToken, CauseOwner, ControlCause, ImportCause, ImportDimension,
    ImportDimensionSnapshot, ImportFailure, ImportLedgerSnapshot, ImportOperationControl,
    ImportPhase, ImportPlan, ImportProgressPhase, ImportProgressUnit, ImportReport, ImportRunError,
    ImportTerminalCause, ImportTerminalFailureClass, Importer, OpenedAsset, ProgressDecision,
    ProgressDeliveryLimits, ProgressEvent, ScopeAbortCause, TopologyKind, ValidatedScene,
};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TRIANGLES: usize = 1_025;
const CORNERS: usize = TRIANGLES * 3;
const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
const READ_CHUNK_BYTES: usize = 1_024;

type ImportEvent = ProgressEvent<ImportProgressPhase, ImportProgressUnit>;
type EventLog = Arc<Mutex<Vec<ImportEvent>>>;

#[derive(Clone, Copy)]
struct ExpectedGeometry {
    label: &'static str,
    format: &'static str,
    positions: usize,
    indexed: bool,
    normals: bool,
}

struct Fixture {
    expected: ExpectedGeometry,
    source: AssetPath,
    bytes: Arc<[u8]>,
    sidecar: Option<(AssetPath, Arc<[u8]>)>,
}

fn fixtures() -> Vec<Fixture> {
    let rows = vec![
        #[cfg(feature = "format-obj")]
        obj_fixture(false),
        #[cfg(feature = "format-stl")]
        stl_fixture(false),
        #[cfg(feature = "format-stl")]
        stl_fixture(true),
        #[cfg(feature = "format-ply")]
        ply_fixture(None),
        #[cfg(feature = "format-ply")]
        ply_fixture(Some(false)),
        #[cfg(feature = "format-ply")]
        ply_fixture(Some(true)),
        #[cfg(feature = "format-gltf")]
        gltf_fixture(false),
        #[cfg(feature = "format-gltf")]
        gltf_fixture(true),
    ];

    let expected = usize::from(cfg!(feature = "format-obj"))
        + 2 * usize::from(cfg!(feature = "format-stl"))
        + 3 * usize::from(cfg!(feature = "format-ply"))
        + 2 * usize::from(cfg!(feature = "format-gltf"));
    assert_eq!(rows.len(), expected);
    #[cfg(all(
        feature = "format-obj",
        feature = "format-stl",
        feature = "format-ply",
        feature = "format-gltf"
    ))]
    assert_eq!(
        rows.len(),
        8,
        "all-formats qualifies eight concrete encoding rows"
    );
    rows
}

#[cfg(feature = "format-obj")]
fn obj_fixture(with_material: bool) -> Fixture {
    let mut text = String::from("o assurance\nv 0 0 0\nv 1 0 0\nv 0 1 0\n");
    if with_material {
        text.push_str("mtllib material.mtl\nusemtl red\n");
    }
    for _ in 0..TRIANGLES {
        text.push_str("f 1 2 3\n");
    }
    assert!(text.len() > 8 * 1_024);
    Fixture {
        expected: ExpectedGeometry {
            label: if with_material { "OBJ + MTL" } else { "OBJ" },
            format: "org.baozi.obj",
            positions: 3,
            indexed: true,
            normals: false,
        },
        source: AssetPath::new("models/assurance.obj").unwrap(),
        bytes: text.into_bytes().into(),
        sidecar: with_material.then(|| {
            (
                AssetPath::new("models/material.mtl").unwrap(),
                Arc::<[u8]>::from(b"newmtl red\nKd 1 0 0\n".as_slice()),
            )
        }),
    }
}

#[cfg(feature = "format-stl")]
fn stl_fixture(binary: bool) -> Fixture {
    let bytes = if binary {
        let mut bytes = vec![0_u8; 80];
        bytes[..14].copy_from_slice(b"Baozi P6 facet");
        bytes.extend_from_slice(&(TRIANGLES as u32).to_le_bytes());
        for _ in 0..TRIANGLES {
            for vector in [[0.0_f32, 0.0, 1.0], TRIANGLE[0], TRIANGLE[1], TRIANGLE[2]] {
                for scalar in vector {
                    bytes.extend_from_slice(&scalar.to_le_bytes());
                }
            }
            bytes.extend_from_slice(&0_u16.to_le_bytes());
        }
        assert_eq!(bytes.len(), 84 + 50 * TRIANGLES);
        bytes
    } else {
        let mut text = String::from("solid assurance\n");
        for _ in 0..TRIANGLES {
            text.push_str(concat!(
                "facet normal 0 0 1\nouter loop\n",
                "vertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\n"
            ));
        }
        text.push_str("endsolid assurance\n");
        assert!(text.len() > 8 * 1_024);
        text.into_bytes()
    };
    Fixture {
        expected: ExpectedGeometry {
            label: if binary { "STL binary" } else { "STL ASCII" },
            format: "org.baozi.stl",
            positions: CORNERS,
            indexed: true,
            normals: true,
        },
        source: AssetPath::new("models/assurance.stl").unwrap(),
        bytes: bytes.into(),
        sidecar: None,
    }
}

#[cfg(feature = "format-ply")]
fn ply_fixture(big_endian: Option<bool>) -> Fixture {
    let (label, encoding) = match big_endian {
        None => ("PLY ASCII", "ascii"),
        Some(false) => ("PLY binary LE", "binary_little_endian"),
        Some(true) => ("PLY binary BE", "binary_big_endian"),
    };
    let header = format!(
        "ply\nformat {encoding} 1.0\nelement vertex 3\nproperty float x\n\
         property float y\nproperty float z\nelement face {TRIANGLES}\n\
         property list uchar int vertex_indices\nend_header\n"
    );
    let mut bytes = header.as_bytes().to_vec();
    if let Some(big_endian) = big_endian {
        for position in TRIANGLE {
            for scalar in position {
                let encoded = if big_endian {
                    scalar.to_be_bytes()
                } else {
                    scalar.to_le_bytes()
                };
                bytes.extend_from_slice(&encoded);
            }
        }
        let mut face = [0_u8; 13];
        face[0] = 3;
        for (ordinal, index) in [0_i32, 1, 2].into_iter().enumerate() {
            let encoded = if big_endian {
                index.to_be_bytes()
            } else {
                index.to_le_bytes()
            };
            face[1 + 4 * ordinal..5 + 4 * ordinal].copy_from_slice(&encoded);
        }
        for _ in 0..TRIANGLES {
            bytes.extend_from_slice(&face);
        }
        assert_eq!(bytes.len(), header.len() + 36 + 13 * TRIANGLES);
    } else {
        bytes.extend_from_slice(b"0 0 0\n1 0 0\n0 1 0\n");
        for _ in 0..TRIANGLES {
            bytes.extend_from_slice(b"3 0 1 2\n");
        }
        assert!(bytes.len() - header.len() > 8 * 1_024);
    }
    Fixture {
        expected: ExpectedGeometry {
            label,
            format: "org.baozi.ply",
            positions: 3,
            indexed: true,
            normals: false,
        },
        source: AssetPath::new("models/assurance.ply").unwrap(),
        bytes: bytes.into(),
        sidecar: None,
    }
}

#[cfg(feature = "format-gltf")]
fn gltf_fixture(binary: bool) -> Fixture {
    let mut buffer = Vec::with_capacity(CORNERS * 12);
    for _ in 0..TRIANGLES {
        for position in TRIANGLE {
            for scalar in position {
                buffer.extend_from_slice(&scalar.to_le_bytes());
            }
        }
    }
    assert_eq!(buffer.len(), CORNERS * 12);
    let buffer_len = buffer.len();
    let uri = if binary {
        ""
    } else {
        r#", "uri": "positions.bin""#
    };
    let mut json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "mode": 4}}]}}],
  "buffers": [{{"byteLength": {buffer_len}{uri}}}],
  "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": {buffer_len}, "target": 34962}}],
  "accessors": [{{"bufferView": 0, "componentType": 5126, "count": {CORNERS},
    "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}}]
}}"#
    )
    .into_bytes();
    json.extend(std::iter::repeat_n(b' ', 8 * 1_024));
    assert!(json.len() > 8 * 1_024);
    let (bytes, sidecar) = if binary {
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !buffer.len().is_multiple_of(4) {
            buffer.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + buffer.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&buffer);
        assert_eq!(glb.len(), total);
        (glb, None)
    } else {
        (
            json,
            Some((
                AssetPath::new("models/positions.bin").unwrap(),
                Arc::<[u8]>::from(buffer),
            )),
        )
    };
    Fixture {
        expected: ExpectedGeometry {
            label: if binary {
                "GLB with BIN"
            } else {
                "glTF external JSON"
            },
            format: "org.baozi.gltf",
            positions: CORNERS,
            indexed: false,
            normals: false,
        },
        source: AssetPath::new(if binary {
            "models/assurance.glb"
        } else {
            "models/assurance.gltf"
        })
        .unwrap(),
        bytes: bytes.into(),
        sidecar,
    }
}

#[derive(Default)]
struct IoCounts {
    bounds: AtomicUsize,
    opens: AtomicUsize,
    reads: AtomicUsize,
    bytes: AtomicUsize,
    drops: AtomicUsize,
}

struct CountedAsset {
    path: AssetPath,
    bytes: Arc<[u8]>,
    length: Option<AssetLength>,
    counts: Arc<IoCounts>,
}

impl CountedAsset {
    fn new(path: &AssetPath, bytes: &Arc<[u8]>) -> Self {
        Self {
            path: path.clone(),
            bytes: Arc::clone(bytes),
            length: Some(AssetLength::Exact(bytes.len() as u64)),
            counts: Arc::new(IoCounts::default()),
        }
    }
}

struct CountedAssetIo {
    domain: AuthorityDomain,
    primary: CountedAsset,
    sidecar: Option<CountedAsset>,
    cancel_primary_bound: Option<CancellationToken>,
    cancel_primary_open: Option<CancellationToken>,
    cancel_sidecar_bound: Option<CancellationToken>,
}

impl CountedAssetIo {
    fn new(fixture: &Fixture) -> Self {
        Self {
            domain: AuthorityDomain::new(),
            primary: CountedAsset::new(&fixture.source, &fixture.bytes),
            sidecar: fixture
                .sidecar
                .as_ref()
                .map(|(path, bytes)| CountedAsset::new(path, bytes)),
            cancel_primary_bound: None,
            cancel_primary_open: None,
            cancel_sidecar_bound: None,
        }
    }

    fn asset(&self, path: &AssetPath) -> &CountedAsset {
        if path == &self.primary.path {
            &self.primary
        } else {
            let sidecar = self
                .sidecar
                .as_ref()
                .expect("only the fixture sidecar may be requested");
            assert_eq!(path, &sidecar.path);
            sidecar
        }
    }
}

fn open_weight(path: &AssetPath) -> NonZeroU64 {
    // Two path comparisons, a path clone, and fixed Arc/counter/reader setup; no payload copy.
    NonZeroU64::new(4 * path.as_str().len() as u64 + 64).unwrap()
}

impl AssetIo for CountedAssetIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        self.asset(path)
            .counts
            .bounds
            .fetch_add(1, Ordering::Relaxed);
        if path == &self.primary.path
            && let Some(token) = &self.cancel_primary_bound
        {
            token.cancel();
        }
        if path != &self.primary.path
            && let Some(token) = &self.cancel_sidecar_bound
        {
            token.cancel();
        }
        Ok(open_weight(path))
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        let asset = self.asset(path);
        asset.counts.opens.fetch_add(1, Ordering::Relaxed);
        let opened = OpenedAsset::new(
            self.domain.clone(),
            path.clone(),
            CountedReader {
                bytes: Arc::clone(&asset.bytes),
                offset: 0,
                counts: Arc::clone(&asset.counts),
            },
            asset.length,
        );
        if path == &self.primary.path
            && let Some(token) = &self.cancel_primary_open
        {
            token.cancel();
        }
        Ok(opened)
    }
}

struct CountedReader {
    bytes: Arc<[u8]>,
    offset: usize,
    counts: Arc<IoCounts>,
}

impl AssetReader for CountedReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        self.counts.reads.fetch_add(1, Ordering::Relaxed);
        let count = buffer
            .len()
            .min(READ_CHUNK_BYTES)
            .min(self.bytes.len() - self.offset);
        buffer[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        self.counts.bytes.fetch_add(count, Ordering::Relaxed);
        Ok(count)
    }
}

impl Drop for CountedReader {
    fn drop(&mut self) {
        self.counts.drops.fetch_add(1, Ordering::Relaxed);
    }
}

fn assert_no_io(io: &CountedAssetIo, label: &str) {
    for asset in std::iter::once(&io.primary).chain(io.sidecar.iter()) {
        for counter in [
            &asset.counts.bounds,
            &asset.counts.opens,
            &asset.counts.reads,
            &asset.counts.bytes,
            &asset.counts.drops,
        ] {
            assert_eq!(
                counter.load(Ordering::Relaxed),
                0,
                "{label}: {}",
                asset.path
            );
        }
    }
}

fn dimension(ledger: &ImportLedgerSnapshot, dimension: ImportDimension) -> ImportDimensionSnapshot {
    ledger
        .dimension(dimension)
        .expect("the public ledger includes this dimension")
}

fn assert_released(ledger: &ImportLedgerSnapshot, label: &str) {
    assert_eq!(ledger.current_live_bytes(), 0, "{label}");
    assert_eq!(ledger.active_reservations(), 0, "{label}");
    assert_eq!(ledger.current_evidence_bytes(), 0, "{label}");
    for key in [
        ImportDimension::AssetOpen,
        ImportDimension::DependencyReference,
        ImportDimension::PrimaryBytes,
        ImportDimension::DependencyBytes,
        ImportDimension::TotalAssetBytes,
        ImportDimension::DetectionWork,
        ImportDimension::WorkUnits,
        ImportDimension::DecodeWork,
        ImportDimension::Tokens,
        ImportDimension::Records,
        ImportDimension::Vertices,
        ImportDimension::Faces,
        ImportDimension::TopologyCells,
        ImportDimension::AttributeCells,
        ImportDimension::ScratchBytes,
        ImportDimension::CanonicalOutputBytes,
    ] {
        let row = dimension(ledger, key);
        assert!(
            row.consumed() <= row.attempted(),
            "{label}: {key:?}: {row:?}"
        );
        assert_eq!(row.current_live_bytes(), 0, "{label}: {key:?}");
    }
}

fn assert_unadmitted_payload(ledger: &ImportLedgerSnapshot, key: ImportDimension, label: &str) {
    let row = dimension(ledger, key);
    assert_eq!(row.attempted(), 0, "{label}: {key:?}");
    assert_eq!(row.consumed(), 0, "{label}: {key:?}");
    assert_eq!(row.peak_live_bytes(), 0, "{label}: {key:?}");
}

fn assert_control_failure(
    error: ImportRunError,
    expected: ControlCause,
    phase: ImportPhase,
    fixture: &Fixture,
) -> ImportFailure {
    let label = fixture.expected.label;
    let failure = error
        .into_operation_failure()
        .expect("control must retain an operation failure");
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("{label}: expected the actual scope control cause: {failure:?}");
    };
    let ScopeAbortCause::Control(actual) = abort.cause() else {
        panic!("{label}: expected control, got {failure:?}");
    };
    assert_eq!(actual, &expected, "{label}");
    assert_eq!(failure.cause().owner(), CauseOwner::Operation, "{label}");
    if let Some(terminal) = failure.terminal_control_cause() {
        assert_eq!(terminal, &expected, "{label}");
    }
    assert_eq!(failure.last_completed().phase(), phase, "{label}");
    assert_eq!(failure.source_identity(), &fixture.source, "{label}");
    let selected = failure.selected_format().map(|format| format.as_str());
    assert_eq!(
        selected,
        (phase != ImportPhase::SessionCreated).then_some(fixture.expected.format),
        "{label}"
    );
    let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
        panic!("{label}: a control failure cannot retain a success terminal");
    };
    assert_eq!(
        terminal.class(),
        ImportTerminalFailureClass::Control,
        "{label}"
    );
    assert_released(failure.ledger(), label);
    assert_eq!(failure.ledger().committed_canonical_members(), 0, "{label}");
    assert_eq!(
        dimension(failure.ledger(), ImportDimension::CanonicalOutputBytes).committed(),
        0
    );
    failure
}

fn assert_geometry(scene: &ValidatedScene, expected: ExpectedGeometry) {
    let label = expected.label;
    assert_eq!(scene.mesh_count(), 1, "{label}");
    assert_eq!(scene.primitive_count(), 1, "{label}");
    assert_eq!(scene.mesh_instance_count(), 1, "{label}");
    let mesh = scene.meshes().next().unwrap();
    assert_eq!(mesh.primitive_count(), 1, "{label}");
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList, "{label}");
    assert_eq!(primitive.face_count() as usize, TRIANGLES, "{label}");
    assert_eq!(primitive.corner_count() as usize, CORNERS, "{label}");
    assert_eq!(
        primitive.vertex_count() as usize,
        expected.positions,
        "{label}"
    );
    assert!(primitive.polygon_face_sizes().is_none(), "{label}");
    if expected.indexed {
        let indices = primitive.indices().expect("the fixture authors indices");
        assert_eq!(indices.len(), CORNERS, "{label}");
        for (corner, index) in indices.iter().enumerate() {
            assert_eq!(
                *index as usize,
                corner % expected.positions,
                "{label}: corner {corner}"
            );
        }
    } else {
        assert!(
            primitive.indices().is_none(),
            "{label}: implicit glTF index stream"
        );
    }
    let positions = primitive.positions().expect("the fixture has positions");
    assert_eq!(
        positions.semantic(),
        AttributeSemanticRef::Position,
        "{label}"
    );
    assert_eq!(positions.domain(), AttributeDomain::Vertex, "{label}");
    let AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
        panic!("{label}: expected dense F32 VEC3 positions");
    };
    assert_eq!(values.len(), expected.positions * 3, "{label}");
    assert!(values.iter().all(|value| value.is_finite()), "{label}");
    for (row, position) in values.chunks_exact(3).enumerate() {
        assert_eq!(
            position,
            TRIANGLE[row % 3].as_slice(),
            "{label}: position {row}"
        );
    }
    let normals = primitive.normals(AttributeDomain::Vertex);
    assert_eq!(normals.is_some(), expected.normals, "{label}");
    if let Some(normals) = normals {
        let AttributeData::DenseF32 { lanes: 3, values } = normals.data() else {
            panic!("{label}: expected dense STL normals");
        };
        assert_eq!(values.len(), CORNERS * 3, "{label}");
        for normal in values.chunks_exact(3) {
            assert_eq!(normal, &[0.0, 0.0, 1.0], "{label}");
        }
    }
}

fn successful_import(plan: &ImportPlan, fixture: &Fixture) -> ImportReport {
    let io = CountedAssetIo::new(fixture);
    let report = plan
        .read_asset(
            &io,
            fixture.source.clone(),
            ImportOperationControl::default().with_cancellation(CancellationToken::new()),
        )
        .unwrap_or_else(|error| {
            panic!(
                "{}: valid fixture must import: {error}",
                fixture.expected.label
            )
        });
    assert_eq!(
        report.format().id().as_str(),
        fixture.expected.format,
        "{}",
        fixture.expected.label
    );
    assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
    assert_geometry(report.scene(), fixture.expected);
    report
}

fn assert_plan_reusable(plan: &ImportPlan, fixture: &Fixture, retained: &ImportReport) {
    assert_geometry(retained.scene(), fixture.expected);
    let recovered = successful_import(plan, fixture);
    assert_geometry(recovered.scene(), fixture.expected);
    assert!(retained.evidence().matches_scene(retained.scene()));
    assert!(recovered.evidence().matches_scene(recovered.scene()));
}

fn assert_completed_parser_work(
    ledger: &ImportLedgerSnapshot,
    baseline: &ImportLedgerSnapshot,
    fixture: &Fixture,
) {
    let label = fixture.expected.label;
    // These rows describe completed format work, independent of later validation work units.
    for key in [
        ImportDimension::PrimaryBytes,
        ImportDimension::DependencyBytes,
        ImportDimension::DecodeWork,
        ImportDimension::Records,
        ImportDimension::Vertices,
        ImportDimension::Faces,
        ImportDimension::TopologyCells,
        ImportDimension::AttributeCells,
    ] {
        let actual = dimension(ledger, key);
        let completed = dimension(baseline, key);
        assert_eq!(
            (actual.attempted(), actual.consumed()),
            (completed.attempted(), completed.consumed()),
            "{label}: completed {key:?} work must survive rollback"
        );
    }
    assert!(
        dimension(ledger, ImportDimension::Vertices).consumed()
            >= fixture.expected.positions as u64
    );
    assert!(
        dimension(ledger, ImportDimension::Faces).consumed() >= TRIANGLES as u64,
        "{label}"
    );
    assert!(
        dimension(ledger, ImportDimension::WorkUnits).consumed() > 0,
        "{label}"
    );
}

fn continuing_progress(events: &EventLog) -> ImportOperationControl {
    let events = Arc::clone(events);
    ImportOperationControl::default().with_progress_observer(move |event: ImportEvent| {
        events.lock().unwrap().push(event);
        ProgressDecision::Continue
    })
}

fn assert_monotone_progress(events: &[ImportEvent], label: &str) {
    assert!(
        !events.is_empty(),
        "{label}: delivered progress is required"
    );
    // Default delivery allows 4,096 transient events and one mandatory final event.
    assert!(events.len() <= 4_097, "{label}");
    for pair in events.windows(2) {
        assert!(pair[0].sequence() < pair[1].sequence(), "{label}: {pair:?}");
        assert!(pair[0].phase() <= pair[1].phase(), "{label}: {pair:?}");
    }
    let mut previous = BTreeMap::new();
    for event in events {
        if let Some(total) = event.total() {
            assert!(event.completed() <= total, "{label}: {event:?}");
        }
        let key = (event.phase(), event.unit());
        if let Some((completed, total)) = previous.insert(key, (event.completed(), event.total())) {
            assert!(event.completed() >= completed, "{label}: {event:?}");
            if let Some(total) = total {
                assert!(
                    event.total().is_some_and(|next| next >= total),
                    "{label}: {event:?}"
                );
            }
        }
    }
}

fn assert_primary_progress(events: &[ImportEvent], fixture: &Fixture, total: Option<u64>) {
    let primary: Vec<_> = events
        .iter()
        .filter(|event| {
            event.phase() == ImportProgressPhase::AcquirePrimary
                && event.unit() == ImportProgressUnit::Bytes
        })
        .collect();
    assert!(
        !primary.is_empty(),
        "{}: primary byte progress",
        fixture.expected.label
    );
    assert!(
        primary.iter().all(|event| event.total() == total),
        "{}",
        fixture.expected.label
    );
    assert_eq!(
        primary.last().unwrap().completed(),
        fixture.bytes.len() as u64
    );
}

fn assert_final_event(events: &[ImportEvent], label: &str) {
    let finals: Vec<_> = events
        .iter()
        .filter(|event| event.phase() == ImportProgressPhase::Finalize)
        .collect();
    assert_eq!(finals.len(), 1, "{label}: one final publication checkpoint");
    assert_eq!(
        finals[0].unit(),
        ImportProgressUnit::CanonicalItems,
        "{label}"
    );
    assert_eq!(
        (finals[0].completed(), finals[0].total()),
        (1, Some(1)),
        "{label}"
    );
}

#[test]
fn real_formats_report_monotone_progress_and_detached_output() {
    for (row, fixture) in fixtures().into_iter().enumerate() {
        let expected = fixture.expected;
        let importer = Importer::new().unwrap();
        let plan = importer.default_plan().unwrap();
        let io = CountedAssetIo::new(&fixture);
        let events = EventLog::default();
        let report = plan
            .read_asset(&io, fixture.source.clone(), continuing_progress(&events))
            .unwrap_or_else(|error| panic!("{}: {error}", expected.label));
        assert_geometry(report.scene(), expected);
        assert_eq!(
            report.format().id().as_str(),
            expected.format,
            "{}",
            expected.label
        );
        assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
        assert!(report.detection_evidence().iter().any(|entry| {
            entry.format().as_str() == expected.format
                && matches!(entry.outcome(), baozi::DetectionOutcome::Match(_))
        }));
        assert_eq!(
            report.ledger().terminal_cause(),
            ImportTerminalCause::Success
        );
        assert!(report.ledger().committed_canonical_members() > 0);
        assert_released(report.ledger(), expected.label);
        assert_eq!(
            dimension(report.ledger(), ImportDimension::PrimaryBytes).consumed(),
            fixture.bytes.len() as u64
        );
        for asset in std::iter::once(&io.primary).chain(io.sidecar.iter()) {
            assert_eq!(
                asset.counts.bounds.load(Ordering::Relaxed),
                1,
                "{}",
                expected.label
            );
            assert_eq!(
                asset.counts.opens.load(Ordering::Relaxed),
                1,
                "{}",
                expected.label
            );
            assert!(
                asset.counts.reads.load(Ordering::Relaxed) > 1,
                "{}",
                expected.label
            );
            assert_eq!(
                asset.counts.bytes.load(Ordering::Relaxed),
                asset.bytes.len()
            );
            assert_eq!(asset.counts.drops.load(Ordering::Relaxed), 1);
        }
        let delivered = events.lock().unwrap().clone();
        assert_monotone_progress(&delivered, expected.label);
        assert_primary_progress(&delivered, &fixture, Some(fixture.bytes.len() as u64));
        assert!(delivered.iter().any(|event| {
            event.phase() == ImportProgressPhase::Detect
                && event.unit() == ImportProgressUnit::Candidates
                && event.completed() > 0
        }));
        assert!(delivered.iter().any(|event| {
            event.phase() == ImportProgressPhase::Validate
                && event.unit() == ImportProgressUnit::WorkUnits
                && event.total().is_some_and(|total| total > 0)
        }));
        assert_final_event(&delivered, expected.label);
        if let Some((_, bytes)) = &fixture.sidecar {
            let dependencies: Vec<_> = delivered
                .iter()
                .filter(|event| {
                    event.phase() == ImportProgressPhase::AcquireDependency
                        && event.unit() == ImportProgressUnit::Bytes
                })
                .collect();
            assert!(
                !dependencies.is_empty(),
                "{}: real dependency progress",
                expected.label
            );
            assert!(dependencies.iter().all(|event| event.total().is_none()));
            assert_eq!(dependencies.last().unwrap().completed(), bytes.len() as u64);
            assert_eq!(
                dimension(report.ledger(), ImportDimension::DependencyBytes).consumed(),
                bytes.len() as u64
            );
        }
        let (scene, evidence) = report.into_parts();
        assert!(evidence.matches_scene(&scene));
        drop(importer);
        drop(plan);
        drop(io);
        drop(fixture);
        assert_geometry(&scene, expected);
        if row.is_multiple_of(2) {
            drop(evidence);
            assert_geometry(&scene, expected);
            drop(scene);
        } else {
            drop(scene);
            assert_eq!(evidence.format().id().as_str(), expected.format);
            assert_eq!(
                evidence.ledger().terminal_cause(),
                ImportTerminalCause::Success
            );
            assert_released(evidence.ledger(), expected.label);
            drop(evidence);
        }
    }
}

#[test]
fn pre_cancelled_real_formats_stop_before_primary_io() {
    for fixture in fixtures() {
        let plan = Importer::new().unwrap().default_plan().unwrap();
        let retained = successful_import(&plan, &fixture);
        let io = CountedAssetIo::new(&fixture);
        let token = CancellationToken::new();
        token.cancel();
        let cloned = plan.clone();
        // A cancelled token is monotonic; retrying it is not a reset operation.
        for request in [&plan, &cloned] {
            let error = request
                .read_asset(
                    &io,
                    fixture.source.clone(),
                    ImportOperationControl::default().with_cancellation(token.clone()),
                )
                .expect_err("pre-cancellation must stop before IO");
            let failure = assert_control_failure(
                error,
                ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                },
                ImportPhase::SessionCreated,
                &fixture,
            );
            assert_unadmitted_payload(
                failure.ledger(),
                ImportDimension::PrimaryBytes,
                fixture.expected.label,
            );
            assert_unadmitted_payload(
                failure.ledger(),
                ImportDimension::TotalAssetBytes,
                fixture.expected.label,
            );
            assert_no_io(&io, fixture.expected.label);
        }
        let recovered = successful_import(&cloned, &fixture);
        assert_geometry(recovered.scene(), fixture.expected);
        assert_plan_reusable(&plan, &fixture, &retained);
    }
}

#[test]
fn open_return_cancellation_precedes_snapshot_allocation_and_read() {
    for fixture in fixtures() {
        let plan = Importer::new().unwrap().default_plan().unwrap();
        let retained = successful_import(&plan, &fixture);
        // Isolate opaque-open work from shared bootstrap and protected failure work.
        let before_token = CancellationToken::new();
        let mut before_io = CountedAssetIo::new(&fixture);
        before_io.cancel_primary_bound = Some(before_token.clone());
        let before_open = assert_control_failure(
            plan.read_asset(
                &before_io,
                fixture.source.clone(),
                ImportOperationControl::default().with_cancellation(before_token),
            )
            .expect_err("cancellation from the bound query must prevent opaque open"),
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            },
            ImportPhase::SessionCreated,
            &fixture,
        );
        assert_eq!(before_io.primary.counts.bounds.load(Ordering::Relaxed), 1);
        assert_eq!(before_io.primary.counts.opens.load(Ordering::Relaxed), 0);
        assert_eq!(before_io.primary.counts.reads.load(Ordering::Relaxed), 0);
        assert_eq!(before_io.primary.counts.drops.load(Ordering::Relaxed), 0);
        let token = CancellationToken::new();
        let mut io = CountedAssetIo::new(&fixture);
        io.cancel_primary_open = Some(token.clone());
        let error = plan
            .read_asset(
                &io,
                fixture.source.clone(),
                ImportOperationControl::default().with_cancellation(token.clone()),
            )
            .expect_err("cancellation from open must precede payload admission");
        let failure = assert_control_failure(
            error,
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            },
            ImportPhase::SessionCreated,
            &fixture,
        );
        assert!(token.is_cancelled());
        let counts = &io.primary.counts;
        assert_eq!(counts.bounds.load(Ordering::Relaxed), 1);
        assert_eq!(counts.opens.load(Ordering::Relaxed), 1);
        assert_eq!(counts.reads.load(Ordering::Relaxed), 0);
        assert_eq!(counts.bytes.load(Ordering::Relaxed), 0);
        assert_eq!(counts.drops.load(Ordering::Relaxed), 1);
        let work = dimension(failure.ledger(), ImportDimension::WorkUnits);
        let before_work = dimension(before_open.ledger(), ImportDimension::WorkUnits);
        let bound = open_weight(&fixture.source).get();
        assert_eq!(work.attempted(), before_work.attempted() + bound);
        assert_eq!(
            work.consumed(),
            before_work.consumed() + bound,
            "the returned opaque open completed"
        );
        assert_unadmitted_payload(
            failure.ledger(),
            ImportDimension::PrimaryBytes,
            fixture.expected.label,
        );
        assert_unadmitted_payload(
            failure.ledger(),
            ImportDimension::TotalAssetBytes,
            fixture.expected.label,
        );
        assert_plan_reusable(&plan, &fixture, &retained);
    }
}

#[test]
fn expired_deadlines_are_preserved_as_control_failures() {
    for fixture in fixtures() {
        let plan = Importer::new().unwrap().default_plan().unwrap();
        let retained = successful_import(&plan, &fixture);
        let io = CountedAssetIo::new(&fixture);
        let deadline = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("past deadline");
        let error = plan
            .read_asset(
                &io,
                fixture.source.clone(),
                ImportOperationControl::default().with_deadline(deadline),
            )
            .expect_err("an already expired deadline must stop at entry");
        let failure = assert_control_failure(
            error,
            ControlCause::DeadlineExceeded,
            ImportPhase::SessionCreated,
            &fixture,
        );
        assert_unadmitted_payload(
            failure.ledger(),
            ImportDimension::PrimaryBytes,
            fixture.expected.label,
        );
        assert_unadmitted_payload(
            failure.ledger(),
            ImportDimension::TotalAssetBytes,
            fixture.expected.label,
        );
        assert_no_io(&io, fixture.expected.label);
        assert_plan_reusable(&plan, &fixture, &retained);
    }
}

#[test]
fn observer_cancellation_at_validation_preserves_the_completed_parser_output() {
    for fixture in fixtures() {
        let plan = Importer::new().unwrap().default_plan().unwrap();
        let retained = successful_import(&plan, &fixture);
        let io = CountedAssetIo::new(&fixture);
        let events = EventLog::default();
        let observed = Arc::clone(&events);
        let control =
            ImportOperationControl::default().with_progress_observer(move |event: ImportEvent| {
                observed.lock().unwrap().push(event);
                if event.phase() == ImportProgressPhase::Validate
                    && event.unit() == ImportProgressUnit::WorkUnits
                {
                    ProgressDecision::Cancel
                } else {
                    ProgressDecision::Continue
                }
            });
        let error = plan
            .read_asset(&io, fixture.source.clone(), control)
            .expect_err("Validate observer cancellation must roll back");
        let failure = assert_control_failure(
            error,
            ControlCause::Cancelled {
                origin: CancellationOrigin::Observer,
            },
            ImportPhase::DraftBuilt,
            &fixture,
        );
        let delivered = events.lock().unwrap().clone();
        assert_monotone_progress(&delivered, fixture.expected.label);
        assert!(delivered.iter().any(|event| {
            event.phase() == ImportProgressPhase::Validate
                && event.unit() == ImportProgressUnit::WorkUnits
        }));
        assert_completed_parser_work(failure.ledger(), retained.ledger(), &fixture);
        assert_plan_reusable(&plan, &fixture, &retained);
    }
}

#[test]
fn final_signal_cancellation_cannot_publish_real_format_output() {
    for fixture in fixtures() {
        let plan = Importer::new().unwrap().default_plan().unwrap();
        let retained = successful_import(&plan, &fixture);
        let io = CountedAssetIo::new(&fixture);
        let events = EventLog::default();
        let observed = Arc::clone(&events);
        let token = CancellationToken::new();
        let raised = token.clone();
        let control = ImportOperationControl::default()
            .with_cancellation(token.clone())
            .with_progress_limits(ProgressDeliveryLimits::new(0, 0, 1))
            .with_progress_observer(move |event: ImportEvent| {
                observed.lock().unwrap().push(event);
                if event.phase() == ImportProgressPhase::Finalize {
                    raised.cancel();
                }
                ProgressDecision::Continue
            });
        let error = plan
            .read_asset(&io, fixture.source.clone(), control)
            .expect_err("the post-observer signal poll must prevent publication");
        let failure = assert_control_failure(
            error,
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            },
            ImportPhase::Validated,
            &fixture,
        );
        assert!(token.is_cancelled());
        let delivered = events.lock().unwrap().clone();
        assert_eq!(
            delivered.len(),
            1,
            "{}: zero transient delivery",
            fixture.expected.label
        );
        assert_monotone_progress(&delivered, fixture.expected.label);
        assert_final_event(&delivered, fixture.expected.label);
        assert_completed_parser_work(failure.ledger(), retained.ledger(), &fixture);
        assert_plan_reusable(&plan, &fixture, &retained);
    }
}

#[test]
fn final_observer_cancellation_cannot_publish_real_format_output() {
    for fixture in fixtures() {
        let plan = Importer::new().unwrap().default_plan().unwrap();
        let retained = successful_import(&plan, &fixture);
        let io = CountedAssetIo::new(&fixture);
        let events = EventLog::default();
        let observed = Arc::clone(&events);
        let token = CancellationToken::new();
        let control = ImportOperationControl::default()
            .with_cancellation(token.clone())
            .with_progress_limits(ProgressDeliveryLimits::new(0, 0, 1))
            .with_progress_observer(move |event: ImportEvent| {
                observed.lock().unwrap().push(event);
                if event.phase() == ImportProgressPhase::Finalize {
                    ProgressDecision::Cancel
                } else {
                    ProgressDecision::Continue
                }
            });
        let error = plan
            .read_asset(&io, fixture.source.clone(), control)
            .expect_err("final observer cancellation must prevent publication");
        let failure = assert_control_failure(
            error,
            ControlCause::Cancelled {
                origin: CancellationOrigin::Observer,
            },
            ImportPhase::Validated,
            &fixture,
        );
        assert!(
            !token.is_cancelled(),
            "observer cancellation is distinct from the signal"
        );
        let delivered = events.lock().unwrap().clone();
        assert_eq!(
            delivered.len(),
            1,
            "{}: zero transient delivery",
            fixture.expected.label
        );
        assert_monotone_progress(&delivered, fixture.expected.label);
        assert_final_event(&delivered, fixture.expected.label);
        assert_completed_parser_work(failure.ledger(), retained.ledger(), &fixture);
        assert_plan_reusable(&plan, &fixture, &retained);
    }
}

#[cfg(feature = "format-obj")]
#[test]
fn unknown_primary_lengths_do_not_invent_progress_totals() {
    let fixture = obj_fixture(false);
    let plan = Importer::new().unwrap().default_plan().unwrap();
    for length in [
        None,
        Some(AssetLength::Hint(fixture.bytes.len() as u64 + 8_192)),
    ] {
        let mut io = CountedAssetIo::new(&fixture);
        io.primary.length = length;
        let events = EventLog::default();
        let report = plan
            .read_asset(&io, fixture.source.clone(), continuing_progress(&events))
            .unwrap();
        let delivered = events.lock().unwrap().clone();
        assert_monotone_progress(&delivered, fixture.expected.label);
        assert_primary_progress(&delivered, &fixture, None);
        assert_final_event(&delivered, fixture.expected.label);
        assert_geometry(report.scene(), fixture.expected);
        assert_eq!(
            io.primary.counts.bytes.load(Ordering::Relaxed),
            fixture.bytes.len()
        );
        assert!(io.primary.counts.reads.load(Ordering::Relaxed) > 2);
        assert_eq!(io.primary.counts.drops.load(Ordering::Relaxed), 1);
        assert_eq!(
            dimension(report.ledger(), ImportDimension::PrimaryBytes).consumed(),
            fixture.bytes.len() as u64
        );
        assert_released(report.ledger(), fixture.expected.label);
    }
}

#[cfg(any(feature = "format-obj", feature = "format-gltf"))]
#[test]
fn sidecar_bound_cancellation_precedes_open_and_preserves_real_parser_work() {
    let mut rows = Vec::new();
    #[cfg(feature = "format-obj")]
    rows.push(obj_fixture(true));
    #[cfg(feature = "format-gltf")]
    rows.push(gltf_fixture(false));
    for fixture in rows {
        let plan = Importer::new().unwrap().default_plan().unwrap();
        let retained = successful_import(&plan, &fixture);
        let mut io = CountedAssetIo::new(&fixture);
        let token = CancellationToken::new();
        io.cancel_sidecar_bound = Some(token.clone());
        let error = plan
            .read_asset(
                &io,
                fixture.source.clone(),
                ImportOperationControl::default().with_cancellation(token.clone()),
            )
            .expect_err("sidecar bound cancellation must precede sidecar open");
        let failure = assert_control_failure(
            error,
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            },
            ImportPhase::SelectionCompleted,
            &fixture,
        );
        assert!(token.is_cancelled());
        assert_eq!(io.primary.counts.bounds.load(Ordering::Relaxed), 1);
        assert_eq!(io.primary.counts.opens.load(Ordering::Relaxed), 1);
        assert!(io.primary.counts.reads.load(Ordering::Relaxed) > 0);
        assert_eq!(
            io.primary.counts.bytes.load(Ordering::Relaxed),
            fixture.bytes.len()
        );
        assert_eq!(io.primary.counts.drops.load(Ordering::Relaxed), 1);
        let counts = &io.sidecar.as_ref().unwrap().counts;
        assert_eq!(counts.bounds.load(Ordering::Relaxed), 1);
        assert_eq!(counts.opens.load(Ordering::Relaxed), 0);
        assert_eq!(counts.reads.load(Ordering::Relaxed), 0);
        assert_eq!(counts.bytes.load(Ordering::Relaxed), 0);
        assert_eq!(counts.drops.load(Ordering::Relaxed), 0);
        assert_unadmitted_payload(
            failure.ledger(),
            ImportDimension::DependencyBytes,
            fixture.expected.label,
        );
        assert_eq!(
            dimension(failure.ledger(), ImportDimension::PrimaryBytes).consumed(),
            fixture.bytes.len() as u64
        );
        assert!(dimension(failure.ledger(), ImportDimension::Records).consumed() > 0);
        if fixture.expected.format == "org.baozi.obj" {
            assert_eq!(
                dimension(failure.ledger(), ImportDimension::Faces).consumed(),
                TRIANGLES as u64
            );
            assert_eq!(
                dimension(failure.ledger(), ImportDimension::Vertices).consumed(),
                3
            );
            assert_eq!(
                dimension(failure.ledger(), ImportDimension::TopologyCells).consumed(),
                CORNERS as u64
            );
        } else {
            assert!(
                dimension(failure.ledger(), ImportDimension::DecodeWork).consumed()
                    >= fixture.bytes.len() as u64
            );
            assert!(dimension(failure.ledger(), ImportDimension::Tokens).consumed() > 0);
        }
        assert_plan_reusable(&plan, &fixture, &retained);
    }
}
