use crate::{FORMAT_ID, StlImporter};
use baozi_core::{
    AttributeData, AttributeDomain, AttributeSemanticRef, TopologyKind, ValidatedScene,
};
use baozi_import::{
    CauseOwner, ImportCause, ImportDimension, ImportDimensionSnapshot, ImportFailure,
    ImportOperationControl, ImportPhase, ImportPlan, ImportReport, ImportRequest,
    ImportTerminalCause, ImportTerminalFailureClass, OptionCompileLimits, RegistryBuildLimits,
    RegistryBuilder, ScopeAbortCause,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationOrigin, CancellationToken, ControlCause};
use std::cell::RefCell;
use std::fmt::Write;

const FACET_COUNT: usize = 1025;
const COLORED_FACET_COUNT: usize = 8193;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompletionSite {
    AsciiFacet,
    BinaryFacet,
    BinaryColorRow,
    TopologyIndex,
}

#[derive(Clone, Copy, Debug, Default)]
struct Completion {
    rows: usize,
    leaves: usize,
    positions: usize,
    normals: usize,
    colors: usize,
    indices: usize,
    last_position: Option<[f32; 3]>,
    last_normal: Option<[f32; 3]>,
    last_color: Option<[f32; 4]>,
    last_index: Option<u32>,
}

struct Probe {
    site: CompletionSite,
    target: usize,
    token: Option<CancellationToken>,
    cancelled_at: Option<Completion>,
    last: Option<Completion>,
    observations: usize,
    cancel_requests: usize,
}

thread_local! {
    static COMPLETION_PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
}

struct ScopedProbe;

impl ScopedProbe {
    fn install(site: CompletionSite, target: usize, token: CancellationToken) -> Self {
        COMPLETION_PROBE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(
                slot.is_none(),
                "a completion probe must not replace an active probe"
            );
            *slot = Some(Probe {
                site,
                target,
                token: Some(token),
                cancelled_at: None,
                last: None,
                observations: 0,
                cancel_requests: 0,
            });
        });
        Self
    }

    fn finish(self) -> Probe {
        COMPLETION_PROBE.with(|slot| slot.borrow_mut().take().expect("active completion probe"))
    }
}

impl Drop for ScopedProbe {
    fn drop(&mut self) {
        let probe = COMPLETION_PROBE.with(|slot| slot.borrow_mut().take());
        drop(probe);
    }
}

pub(super) fn facet_completed(
    site: CompletionSite,
    facets: usize,
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
) {
    record_completion(
        site,
        Completion {
            rows: facets,
            // ASCII has three separate work.run leaves; binary has one fixed 50-byte leaf.
            leaves: if site == CompletionSite::AsciiFacet {
                positions.len() / 3 + normals.len() / 3 + facets
            } else {
                facets
            },
            positions: positions.len(),
            normals: normals.len(),
            last_position: positions.last().copied(),
            last_normal: normals.last().copied(),
            ..Completion::default()
        },
    );
}

pub(super) fn color_row_completed(colors: &[[f32; 4]]) {
    record_completion(
        CompletionSite::BinaryColorRow,
        Completion {
            rows: colors.len() / 3,
            leaves: colors.len() / 3,
            colors: colors.len(),
            last_color: colors.last().copied(),
            ..Completion::default()
        },
    );
}

pub(super) fn index_completed(indices: &[u32]) {
    record_completion(
        CompletionSite::TopologyIndex,
        Completion {
            rows: indices.len(),
            leaves: indices.len(),
            indices: indices.len(),
            last_index: indices.last().copied(),
            ..Completion::default()
        },
    );
}

fn record_completion(site: CompletionSite, completion: Completion) {
    let token = COMPLETION_PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        if probe.site != site {
            return None;
        }
        probe.last = Some(completion);
        probe.observations += 1;
        let complete_row = match site {
            CompletionSite::BinaryColorRow | CompletionSite::TopologyIndex => true,
            CompletionSite::AsciiFacet | CompletionSite::BinaryFacet => {
                completion.positions == 3 * completion.rows
                    && completion.normals == 3 * completion.rows
            }
        };
        if complete_row && completion.rows == probe.target && probe.cancelled_at.is_none() {
            probe.cancelled_at = Some(completion);
            probe.cancel_requests += 1;
            probe.token.take()
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
    }
}

fn import_plan() -> ImportPlan {
    let mut registry = RegistryBuilder::new();
    registry
        .register(StlImporter)
        .expect("real STL registration");
    registry
        .freeze(RegistryBuildLimits::default())
        .expect("STL registry freezes")
        .plan(&ImportRequest::default(), OptionCompileLimits::default())
        .expect("STL plan compiles")
}

fn ascii_fixture() -> Vec<u8> {
    let mut document = String::from("solid p6\n");
    for face in 0..FACET_COUNT {
        writeln!(
            document,
            "facet normal 0 0 1\nouter loop\nvertex {face} 0 0\nvertex {} 0 0\nvertex {face} 1 0\nendloop\nendfacet",
            face + 1,
        )
        .expect("writing an in-memory STL fixture");
    }
    document.push_str("endsolid p6\n");
    assert!(document.len() > 8192);
    assert_eq!(document.lines().count(), 2 + 7 * FACET_COUNT);
    document.into_bytes()
}

fn binary_fixture(facets: usize, colored: bool) -> Vec<u8> {
    let mut bytes = vec![0; 80];
    bytes[..6].copy_from_slice(b"P6 STL");
    bytes.extend_from_slice(&u32::try_from(facets).unwrap().to_le_bytes());
    for face in 0..facets {
        let x = face as f32;
        for value in [0.0, 0.0, 1.0, x, 0.0, 0.0, x + 1.0, 0.0, 0.0, x, 1.0, 0.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let attribute: u16 = if colored {
            [0xfc00, 0x83e0, 0x801f][face % 3]
        } else {
            0
        };
        bytes.extend_from_slice(&attribute.to_le_bytes());
    }
    assert_eq!(bytes.len(), 84 + 50 * facets);
    bytes
}

fn facet_color(face: usize) -> [f32; 4] {
    match face % 3 {
        0 => [1.0, 0.0, 0.0, 1.0],
        1 => [0.0, 1.0, 0.0, 1.0],
        _ => [0.0, 0.0, 1.0, 1.0],
    }
}

fn assert_scene(scene: &ValidatedScene, facets: usize, colored: bool) {
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    assert_eq!(scene.attribute_count(), if colored { 3 } else { 2 });
    let mesh = scene.meshes().next().expect("one retained STL mesh");
    assert_eq!(mesh.primitive_count(), 1);
    let primitive = mesh
        .primitives()
        .next()
        .expect("one retained STL primitive");
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.face_count() as usize, facets);
    assert_eq!(primitive.vertex_count() as usize, 3 * facets);
    assert_eq!(primitive.corner_count() as usize, 3 * facets);
    let indices = primitive.indices().expect("authored triangle indices");
    assert_eq!(indices.len(), 3 * facets);
    for (index, value) in indices.iter().copied().enumerate() {
        assert_eq!(value as usize, index);
    }
    let positions = primitive.positions().expect("retained STL positions");
    assert_eq!(positions.len(), 3 * facets);
    let AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
        panic!("STL positions must be dense XYZ");
    };
    assert_eq!(values.len(), 9 * facets);
    for (face, actual) in values.chunks_exact(9).enumerate() {
        let x = face as f32;
        assert_eq!(actual, [x, 0.0, 0.0, x + 1.0, 0.0, 0.0, x, 1.0, 0.0]);
    }
    let normals = primitive
        .normals(AttributeDomain::Vertex)
        .expect("retained STL normals");
    assert_eq!(normals.len(), 3 * facets);
    let AttributeData::DenseF32 { lanes: 3, values } = normals.data() else {
        panic!("STL normals must be dense XYZ");
    };
    assert_eq!(values.len(), 9 * facets);
    for actual in values.chunks_exact(3) {
        assert_eq!(actual, [0.0, 0.0, 1.0]);
    }
    let colors = primitive.attribute(AttributeSemanticRef::Color(0), AttributeDomain::Vertex);
    assert_eq!(colors.is_some(), colored);
    if let Some(colors) = colors {
        assert_eq!(colors.len(), 3 * facets);
        let AttributeData::DenseF32 { lanes: 4, values } = colors.data() else {
            panic!("STL colors must be dense RGBA");
        };
        assert_eq!(values.len(), 12 * facets);
        for (vertex, actual) in values.chunks_exact(4).enumerate() {
            assert_eq!(actual, facet_color(vertex / 3));
        }
    }
}

fn retain_scene(report: ImportReport, facets: usize, colored: bool) -> ValidatedScene {
    assert_eq!(report.format().id().as_str(), FORMAT_ID);
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    let (scene, evidence) = report.into_parts();
    assert!(evidence.matches_scene(&scene));
    drop(evidence);
    assert_scene(&scene, facets, colored);
    scene
}

fn row(failure: &ImportFailure, dimension: ImportDimension) -> ImportDimensionSnapshot {
    failure
        .ledger()
        .dimension(dimension)
        .expect("configured STL dimension")
}

fn assert_signal_failure(failure: &ImportFailure, source: &AssetPath) {
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert!(
        matches!(
            failure.cause(),
            ImportCause::Scope(abort) if matches!(
                abort.cause(),
                ScopeAbortCause::Control(ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                })
            )
        ),
        "expected the exact Signal control cause: {failure:?}"
    );
    assert_eq!(
        failure.selected_format().map(|format| format.as_str()),
        Some(FORMAT_ID)
    );
    assert_eq!(failure.source_identity(), source);
    assert!(failure.terminal_control_cause().is_none());
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    assert!(failure.diagnostics().is_empty());
    assert!(failure.loss_evidence().is_empty());
    let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
        panic!("cancelled STL must have terminal failure evidence");
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Control);
    // The terminal ledger projects Cancelled to control code 1; the scope preserves its origin.
    assert_eq!(terminal.code(), 1);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert!(
        failure.ledger().current_administrative_bytes()
            <= failure.ledger().peak_administrative_bytes()
    );
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    for dimension in [
        ImportDimension::Faces,
        ImportDimension::Vertices,
        ImportDimension::AttributeCells,
        ImportDimension::Records,
        ImportDimension::Tokens,
        ImportDimension::DecodeWork,
        ImportDimension::Meshes,
        ImportDimension::TopologyCells,
        ImportDimension::Primitives,
        ImportDimension::AttributeStreams,
        ImportDimension::GraphNodes,
        ImportDimension::GraphEdges,
        ImportDimension::CanonicalOutputBytes,
    ] {
        let row = row(failure, dimension);
        assert!(row.consumed() <= row.attempted(), "{dimension:?}: {row:?}");
        assert_eq!(row.current_live_bytes(), 0, "{dimension:?}");
        assert_eq!(row.committed(), 0, "{dimension:?}");
    }
}

fn assert_before_mesh_construction(failure: &ImportFailure) {
    assert_eq!(row(failure, ImportDimension::Meshes).attempted(), 0);
    assert_eq!(row(failure, ImportDimension::TopologyCells).attempted(), 0);
}

fn assert_probe(probe: &Probe, token: &CancellationToken, total_rows: usize) -> Completion {
    assert!(token.is_cancelled());
    assert_eq!(probe.cancel_requests, 1);
    assert!(probe.token.is_none());
    let raised = probe
        .cancelled_at
        .expect("the requested physical prefix must be reached");
    assert_eq!(raised.rows, probe.target);
    assert_eq!(
        raised.leaves,
        if probe.site == CompletionSite::AsciiFacet {
            3 * probe.target
        } else {
            probe.target
        }
    );
    let last = probe.last.expect("actual completed geometry leaves");
    assert!(
        last.rows >= raised.rows && last.rows < total_rows,
        "{last:?}"
    );
    assert!(last.leaves >= raised.leaves);
    assert_eq!(probe.observations, last.leaves);
    if probe.site == CompletionSite::BinaryColorRow {
        assert_eq!(raised.colors, 3 * probe.target);
        assert_eq!(last.colors, 3 * last.rows);
        assert_eq!(raised.last_color, Some(facet_color(probe.target - 1)));
        assert_eq!(last.last_color, Some(facet_color(last.rows - 1)));
    } else if probe.site == CompletionSite::TopologyIndex {
        assert_eq!(raised.indices, probe.target);
        assert_eq!(last.indices, last.rows);
        assert_eq!(raised.last_index, Some((probe.target - 1) as u32));
        assert_eq!(last.last_index, Some((last.rows - 1) as u32));
    } else {
        assert_eq!(raised.positions, 3 * probe.target);
        assert_eq!(raised.normals, 3 * probe.target);
        assert_eq!(
            raised.last_position,
            Some([(probe.target - 1) as f32, 1.0, 0.0])
        );
        assert_eq!(
            last.last_position,
            Some([(last.positions / 3 - 1) as f32, 1.0, 0.0])
        );
        assert_eq!(last.last_normal, Some([0.0, 0.0, 1.0]));
    }
    last
}

fn assert_geometry_bound(probe: &Probe) {
    let raised = probe.cancelled_at.unwrap();
    let last = probe.last.unwrap();
    let subsequent = last.leaves - raised.leaves;
    assert!(
        subsequent <= 256,
        "{:?}: {subsequent} subsequent fixed geometry leaves after row {}, stopped at row {}; bound is 256",
        probe.site,
        raised.rows,
        last.rows,
    );
}

#[test]
fn ascii_facet_257_cancellation_retains_prefix_and_scene() {
    let plan = import_plan();
    let source = AssetPath::new("p6-ascii.stl").unwrap();
    let bytes = ascii_fixture();
    let baseline = plan
        .read_bytes(source.clone(), &bytes, ImportOperationControl::default())
        .expect("valid ASCII baseline");
    let retained = retain_scene(baseline, FACET_COUNT, false);
    let token = CancellationToken::new();
    let probe = ScopedProbe::install(CompletionSite::AsciiFacet, 257, token.clone());
    let result = plan.read_bytes(
        source.clone(),
        &bytes,
        ImportOperationControl::default().with_cancellation(token.clone()),
    );
    let probe = probe.finish();
    let failure = result
        .expect_err("ASCII facet cancellation")
        .into_operation_failure()
        .expect("operation failure");
    assert_signal_failure(&failure, &source);
    assert_before_mesh_construction(&failure);
    let last = assert_probe(&probe, &token, FACET_COUNT);
    for (dimension, consumed) in [
        (ImportDimension::Faces, last.rows),
        (ImportDimension::Vertices, last.positions),
        (
            ImportDimension::AttributeCells,
            last.positions + last.normals,
        ),
        (ImportDimension::Records, 1 + 7 * last.rows),
        (ImportDimension::Tokens, 2 + 21 * last.rows),
    ] {
        assert_eq!(
            row(&failure, dimension).consumed(),
            consumed as u64,
            "{dimension:?}"
        );
    }
    assert_scene(&retained, FACET_COUNT, false);
    let report = plan
        .clone()
        .read_bytes(
            source,
            &bytes,
            ImportOperationControl::default().with_cancellation(CancellationToken::new()),
        )
        .expect("fresh ASCII control reuses the same plan");
    let reused = retain_scene(report, FACET_COUNT, false);
    drop(plan);
    drop(bytes);
    assert_scene(&retained, FACET_COUNT, false);
    assert_scene(&reused, FACET_COUNT, false);
    assert_geometry_bound(&probe);
}

#[test]
fn binary_facet_257_cancellation_retains_prefix_and_scene() {
    let plan = import_plan();
    let source = AssetPath::new("p6-binary.stl").unwrap();
    let bytes = binary_fixture(FACET_COUNT, false);
    let baseline = plan
        .read_bytes(source.clone(), &bytes, ImportOperationControl::default())
        .expect("valid binary baseline");
    let retained = retain_scene(baseline, FACET_COUNT, false);
    let token = CancellationToken::new();
    let probe = ScopedProbe::install(CompletionSite::BinaryFacet, 257, token.clone());
    let result = plan.read_bytes(
        source.clone(),
        &bytes,
        ImportOperationControl::default().with_cancellation(token.clone()),
    );
    let probe = probe.finish();
    let failure = result
        .expect_err("binary facet cancellation")
        .into_operation_failure()
        .expect("operation failure");
    assert_signal_failure(&failure, &source);
    assert_before_mesh_construction(&failure);
    let last = assert_probe(&probe, &token, FACET_COUNT);
    let p = last.rows as u64;
    for (dimension, attempted, consumed) in [
        (ImportDimension::DecodeWork, bytes.len() as u64, 84 + 50 * p),
        (ImportDimension::Faces, FACET_COUNT as u64, p),
        (ImportDimension::Vertices, 3 * FACET_COUNT as u64, 3 * p),
        (
            ImportDimension::AttributeCells,
            6 * FACET_COUNT as u64,
            6 * p,
        ),
        (ImportDimension::Records, FACET_COUNT as u64, p),
    ] {
        assert_eq!(
            row(&failure, dimension).attempted(),
            attempted,
            "{dimension:?}"
        );
        assert_eq!(
            row(&failure, dimension).consumed(),
            consumed,
            "{dimension:?}"
        );
    }
    assert_eq!(last.positions, 3 * last.rows);
    assert_eq!(last.normals, 3 * last.rows);
    assert_scene(&retained, FACET_COUNT, false);
    let report = plan
        .clone()
        .read_bytes(
            source,
            &bytes,
            ImportOperationControl::default().with_cancellation(CancellationToken::new()),
        )
        .expect("fresh binary control reuses the same plan");
    let reused = retain_scene(report, FACET_COUNT, false);
    drop(plan);
    drop(bytes);
    assert_scene(&retained, FACET_COUNT, false);
    assert_scene(&reused, FACET_COUNT, false);
    assert_geometry_bound(&probe);
}

#[test]
fn binary_index_1_cancellation_retains_prefix_and_scene() {
    let plan = import_plan();
    let source = AssetPath::new("p6-binary-index.stl").unwrap();
    let bytes = binary_fixture(FACET_COUNT, false);
    let baseline = plan
        .read_bytes(source.clone(), &bytes, ImportOperationControl::default())
        .expect("valid binary index baseline");
    let retained = retain_scene(baseline, FACET_COUNT, false);
    let token = CancellationToken::new();
    let probe = ScopedProbe::install(CompletionSite::TopologyIndex, 1, token.clone());
    let result = plan.read_bytes(
        source.clone(),
        &bytes,
        ImportOperationControl::default().with_cancellation(token.clone()),
    );
    let probe = probe.finish();
    let failure = result
        .expect_err("binary index projection cancellation")
        .into_operation_failure()
        .expect("operation failure");
    assert_signal_failure(&failure, &source);
    let last = assert_probe(&probe, &token, 3 * FACET_COUNT);
    let n = FACET_COUNT as u64;
    for (dimension, attempted, consumed) in [
        (
            ImportDimension::DecodeWork,
            bytes.len() as u64,
            bytes.len() as u64,
        ),
        (ImportDimension::Faces, n, n),
        (ImportDimension::Vertices, 3 * n, 3 * n),
        (ImportDimension::AttributeCells, 6 * n, 6 * n),
        (ImportDimension::Records, n, n),
        (ImportDimension::Meshes, 1, 1),
        (ImportDimension::TopologyCells, 3 * n, last.indices as u64),
        (ImportDimension::Primitives, 1, 0),
        (ImportDimension::AttributeStreams, 2, 0),
        (ImportDimension::GraphNodes, 1, 0),
        (ImportDimension::GraphEdges, 1, 0),
    ] {
        assert_eq!(
            row(&failure, dimension).attempted(),
            attempted,
            "{dimension:?}"
        );
        assert_eq!(
            row(&failure, dimension).consumed(),
            consumed,
            "{dimension:?}"
        );
    }
    assert_scene(&retained, FACET_COUNT, false);
    let report = plan
        .clone()
        .read_bytes(
            source,
            &bytes,
            ImportOperationControl::default().with_cancellation(CancellationToken::new()),
        )
        .expect("fresh binary index control reuses the same plan");
    let reused = retain_scene(report, FACET_COUNT, false);
    drop(plan);
    drop(bytes);
    assert_scene(&retained, FACET_COUNT, false);
    assert_scene(&reused, FACET_COUNT, false);
    assert_geometry_bound(&probe);
}

#[test]
fn binary_color_row_1_cancellation_respects_geometry_bound() {
    let plan = import_plan();
    let source = AssetPath::new("p6-binary-color.stl").unwrap();
    let bytes = binary_fixture(COLORED_FACET_COUNT, true);
    let baseline = plan
        .read_bytes(source.clone(), &bytes, ImportOperationControl::default())
        .expect("valid colored binary baseline");
    let retained = retain_scene(baseline, COLORED_FACET_COUNT, true);
    let token = CancellationToken::new();
    let probe = ScopedProbe::install(CompletionSite::BinaryColorRow, 1, token.clone());
    let result = plan.read_bytes(
        source.clone(),
        &bytes,
        ImportOperationControl::default().with_cancellation(token.clone()),
    );
    let probe = probe.finish();
    let failure = result
        .expect_err("binary color expansion cancellation")
        .into_operation_failure()
        .expect("operation failure");
    assert_signal_failure(&failure, &source);
    assert_before_mesh_construction(&failure);
    let last = assert_probe(&probe, &token, COLORED_FACET_COUNT);
    let n = COLORED_FACET_COUNT as u64;
    for (dimension, attempted, consumed) in [
        (
            ImportDimension::DecodeWork,
            bytes.len() as u64,
            bytes.len() as u64,
        ),
        (ImportDimension::Faces, n, n),
        (ImportDimension::Vertices, 3 * n, 3 * n),
        (ImportDimension::Records, n, n),
        (
            ImportDimension::AttributeCells,
            9 * n,
            6 * n + last.colors as u64,
        ),
    ] {
        assert_eq!(
            row(&failure, dimension).attempted(),
            attempted,
            "{dimension:?}"
        );
        assert_eq!(
            row(&failure, dimension).consumed(),
            consumed,
            "{dimension:?}"
        );
    }
    assert_scene(&retained, COLORED_FACET_COUNT, true);
    let report = plan
        .clone()
        .read_bytes(
            source,
            &bytes,
            ImportOperationControl::default().with_cancellation(CancellationToken::new()),
        )
        .expect("fresh colored binary control reuses the same plan");
    let reused = retain_scene(report, COLORED_FACET_COUNT, true);
    drop(plan);
    drop(bytes);
    assert_scene(&retained, COLORED_FACET_COUNT, true);
    assert_scene(&reused, COLORED_FACET_COUNT, true);
    // Keep this assertion last so the first failing run also exercises cleanup and reuse.
    assert_geometry_bound(&probe);
}

#[test]
fn binary_two_facets_cancel_after_one_with_literal_work_prefix() {
    let plan = import_plan();
    let source = AssetPath::new("binary-two-facet-prefix.stl").unwrap();
    let bytes = binary_fixture(2, false);
    assert_eq!(bytes.len(), 184);
    let token = CancellationToken::new();
    let probe = ScopedProbe::install(CompletionSite::BinaryFacet, 1, token.clone());
    let result = plan.read_bytes(
        source.clone(),
        &bytes,
        ImportOperationControl::default().with_cancellation(token.clone()),
    );
    let probe = probe.finish();
    let failure = result
        .expect_err("the second facet must not run")
        .into_operation_failure()
        .unwrap();
    assert_signal_failure(&failure, &source);
    assert_before_mesh_construction(&failure);
    let last = assert_probe(&probe, &token, 2);
    assert_eq!((last.rows, last.positions, last.normals), (1, 3, 3));
    for (dimension, attempted, consumed) in [
        (ImportDimension::DecodeWork, 184, 134),
        (ImportDimension::Faces, 2, 1),
        (ImportDimension::Vertices, 6, 3),
        (ImportDimension::AttributeCells, 12, 6),
        (ImportDimension::Records, 2, 1),
    ] {
        let row = row(&failure, dimension);
        assert_eq!(
            (row.attempted(), row.consumed()),
            (attempted, consumed),
            "{dimension:?}"
        );
    }
    assert_geometry_bound(&probe);
}
