use crate::{FORMAT_ID, GltfImporter};
use baozi_core::{AttributeElement, TopologyKind, ValidatedScene};
use baozi_import::authoring::{MorphDeltaSemantic, Vec3};
use baozi_import::{
    CauseOwner, ImportCause, ImportDimension, ImportFailure, ImportOperationControl, ImportPhase,
    ImportPlan, ImportReport, ImportRequest, ImportTerminalCause, ImportTerminalFailureClass,
    OptionCompileLimits, RegistryBuildLimits, RegistryBuilder, ScopeAbortCause,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use baozi_operation::{CancellationOrigin, CancellationToken, ControlCause};
use serde_json::json;
use std::{cell::RefCell, error::Error, marker::PhantomData, rc::Rc, sync::Arc};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const ROW_COUNT: usize = 8_193;
const CANCEL_AFTER: usize = 257;
const MAX_FURTHER_ROWS: usize = 256;
// One definition, one default weight, three scalars per row, and one target row.
const MORPH_CELLS: u64 = 3 * ROW_COUNT as u64 + 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewedSite {
    SemanticPair,
    ExtensionPair,
    ResourceSource,
}

struct ReviewedProbe {
    site: ReviewedSite,
    completed: usize,
    token: Option<CancellationToken>,
}

thread_local! {
    static REVIEWED_PROBE: RefCell<Option<ReviewedProbe>> = const { RefCell::new(None) };
}

struct ReviewedProbeGuard(PhantomData<Rc<()>>);

impl ReviewedProbeGuard {
    fn new(site: ReviewedSite, token: CancellationToken) -> Self {
        REVIEWED_PROBE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none());
            *slot = Some(ReviewedProbe {
                site,
                completed: 0,
                token: Some(token),
            });
        });
        Self(PhantomData)
    }

    fn finish(self) -> ReviewedProbe {
        REVIEWED_PROBE.with(|slot| slot.borrow_mut().take().expect("active review probe"))
    }
}

impl Drop for ReviewedProbeGuard {
    fn drop(&mut self) {
        let probe = REVIEWED_PROBE.with(|slot| slot.borrow_mut().take());
        drop(probe);
    }
}

fn reviewed_completion(site: ReviewedSite, completed: usize) {
    let token = REVIEWED_PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        if probe.site != site {
            return None;
        }
        probe.completed = completed;
        if completed == 1 {
            probe.token.take()
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
    }
}

pub(crate) fn semantic_pair_completed(completed: usize) {
    reviewed_completion(ReviewedSite::SemanticPair, completed);
}

pub(crate) fn extension_pair_completed(completed: usize) {
    reviewed_completion(ReviewedSite::ExtensionPair, completed);
}

pub(crate) fn resource_source_completed(completed: usize) {
    reviewed_completion(ReviewedSite::ResourceSource, completed);
}

fn assert_review_signal(failure: &ImportFailure) {
    assert!(
        matches!(
            failure.cause(),
            ImportCause::Scope(abort) if matches!(
                abort.cause(),
                ScopeAbortCause::Control(ControlCause::Cancelled { origin: CancellationOrigin::Signal })
            )
        ),
        "expected review-site Signal cancellation: {:?}",
        failure.cause()
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
}

fn assert_format_code(failure: &ImportFailure, code: crate::GltfFailureCode) {
    let ImportCause::Format(actual) = failure.cause() else {
        panic!("expected glTF format rejection: {:?}", failure.cause());
    };
    assert_eq!(actual.code(), code.as_str());
}

fn wide_schema(duplicate: bool) -> serde_json::Value {
    let mut attributes = serde_json::Map::new();
    attributes.insert("POSITION".into(), json!(0));
    for set in 0..512 {
        attributes.insert(format!("JOINTS_{set}"), json!(1));
        attributes.insert(format!("WEIGHTS_{set}"), json!(2));
    }
    if duplicate {
        attributes.insert("WEIGHTS_0511".into(), json!(2));
    }
    // The mesh is unselected: global schema checks must still visit its full width.
    json!({
        "asset":{"version":"2.0"}, "scene":0, "scenes":[{}],
        "accessors":[
            {"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[0,0,0]},
            {"componentType":5123,"count":1,"type":"VEC4"},
            {"componentType":5126,"count":1,"type":"VEC4"}
        ],
        "meshes":[{"primitives":[{"attributes":attributes,"mode":0}]}]
    })
}

#[test]
fn wide_unselected_attribute_schema_has_bounded_duplicate_checks() -> TestResult {
    let plan = import_plan()?;
    let source = AssetPath::new("wide-schema.gltf")?;
    let bytes = serde_json::to_vec(&wide_schema(false))?;
    let baseline = plan.read_bytes(source.clone(), &bytes, ImportOperationControl::default())?;
    assert_eq!(baseline.scene().mesh_count(), 0);
    let token = CancellationToken::new();
    let probe = ReviewedProbeGuard::new(ReviewedSite::SemanticPair, token.clone());
    let failure = plan
        .read_bytes(
            source.clone(),
            &bytes,
            ImportOperationControl::default().with_cancellation(token),
        )
        .expect_err("the actual semantic-pair comparison raises cancellation")
        .into_operation_failure()
        .expect("ordinary operation failure");
    let probe = probe.finish();
    assert_review_signal(&failure);
    assert!((1..=baozi_import::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS + 1).contains(&probe.completed));
    assert!(probe.completed < 1024);
    let duplicate = serde_json::to_vec(&wide_schema(true))?;
    let failure = plan
        .read_bytes(source, &duplicate, ImportOperationControl::default())
        .expect_err("the late alias is a duplicate semantic")
        .into_operation_failure()
        .expect("ordinary format failure");
    assert_format_code(&failure, crate::GltfFailureCode::InvalidDocument);
    Ok(())
}

#[test]
fn required_extension_membership_checks_long_shared_prefixes() -> TestResult {
    let prefix = "X".repeat(65_536);
    let used = format!("{prefix}a");
    let required = format!("{prefix}b");
    let bytes = serde_json::to_vec(&json!({
        "asset":{"version":"2.0"}, "scene":0,"scenes":[{}],
        "extensionsUsed":[used], "extensionsRequired":[required]
    }))?;
    let source = AssetPath::new("long-extensions.gltf")?;
    let plan = import_plan()?;
    let missing = plan
        .read_bytes(source.clone(), &bytes, ImportOperationControl::default())
        .expect_err("missing-used must precede unsupported-required")
        .into_operation_failure()
        .expect("ordinary format failure");
    assert_format_code(&missing, crate::GltfFailureCode::InvalidDocument);
    let token = CancellationToken::new();
    let probe = ReviewedProbeGuard::new(ReviewedSite::ExtensionPair, token.clone());
    let failure = plan
        .read_bytes(
            source,
            &bytes,
            ImportOperationControl::default().with_cancellation(token),
        )
        .expect_err("the real membership comparison raises cancellation")
        .into_operation_failure()
        .expect("ordinary operation failure");
    let probe = probe.finish();
    assert_review_signal(&failure);
    // Each pair examines two raw bytes; neither list's length-one sort invokes this hook.
    assert!((1..=crate::parser::BYTE_POLL_INTERVAL / 2 + 1).contains(&probe.completed));
    assert!(probe.completed < prefix.len());
    Ok(())
}

#[test]
fn many_resource_source_identities_have_bounded_construction() -> TestResult {
    const RESOURCES: usize = 512;
    let io = MemoryAssetIo::new();
    let mut buffers = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    let mut binary = Vec::new();
    for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    for index in 0..RESOURCES {
        let name = format!("resource-{index}.bin");
        io.try_insert(AssetPath::new(&name)?, Arc::<[u8]>::from(binary.as_slice()))?;
        buffers.push(json!({"byteLength":binary.len(),"uri":name}));
        views.push(json!({"buffer":index,"byteLength":binary.len()}));
        accessors.push(json!({"bufferView":index,"componentType":5126,"count":3,
            "type":"VEC3","min":[0,0,0],"max":[1,1,0]}));
        primitives.push(json!({"attributes":{"POSITION":index},"mode":4}));
    }
    let document = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"meshes":[{"primitives":primitives}],
        "buffers":buffers,"bufferViews":views,"accessors":accessors});
    let source = AssetPath::new("many-resources.gltf")?;
    io.try_insert(
        source.clone(),
        Arc::<[u8]>::from(serde_json::to_vec(&document)?),
    )?;
    let plan = import_plan()?;
    let baseline = plan.read_asset(&io, source.clone(), ImportOperationControl::default())?;
    assert_eq!(baseline.scene().mesh_count(), 1);
    assert_eq!(baseline.scene().primitive_count(), RESOURCES);
    let token = CancellationToken::new();
    let probe = ReviewedProbeGuard::new(ReviewedSite::ResourceSource, token.clone());
    let failure = plan
        .read_asset(
            &io,
            source.clone(),
            ImportOperationControl::default().with_cancellation(token),
        )
        .expect_err("the first completed source-identity row raises cancellation")
        .into_operation_failure()
        .expect("ordinary operation failure");
    let probe = probe.finish();
    assert_review_signal(&failure);
    assert!((1..=baozi_import::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS + 1).contains(&probe.completed));
    assert!(probe.completed < RESOURCES + 1);
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::Vertices)
            .unwrap()
            .consumed(),
        3 * RESOURCES as u64
    );
    assert_eq!(baseline.scene().primitive_count(), RESOURCES);
    let recovered = plan.read_asset(&io, source, ImportOperationControl::default())?;
    assert_eq!(recovered.scene().primitive_count(), RESOURCES);
    Ok(())
}

struct MorphProbe {
    cancel_after: usize,
    token: Option<CancellationToken>,
    cancelled_at: Option<usize>,
    completed_rows: usize,
    last_row: [f32; 3],
}

thread_local! {
    static MORPH_PROBE: RefCell<Option<MorphProbe>> = const { RefCell::new(None) };
}

struct MorphProbeGuard {
    _same_thread: PhantomData<Rc<()>>,
}

impl MorphProbeGuard {
    fn new(cancel_after: usize, token: CancellationToken) -> Self {
        MORPH_PROBE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none(), "morph completion probes must not overlap");
            *slot = Some(MorphProbe {
                cancel_after,
                token: Some(token),
                cancelled_at: None,
                completed_rows: 0,
                last_row: [0.0; 3],
            });
        });
        Self {
            _same_thread: PhantomData,
        }
    }

    fn finish(self) -> MorphProbe {
        MORPH_PROBE.with(|slot| slot.borrow_mut().take().expect("active morph probe"))
    }
}

impl Drop for MorphProbeGuard {
    fn drop(&mut self) {
        let _probe = MORPH_PROBE.with(|slot| slot.borrow_mut().take());
    }
}

// Observe the actual append only after its existing work completion succeeds.
pub(crate) fn morph_vec3_completed(completed_rows: usize, last_row: [f32; 3]) {
    let token = MORPH_PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        probe.completed_rows = completed_rows;
        probe.last_row = last_row;
        if completed_rows == probe.cancel_after {
            probe.cancelled_at = Some(completed_rows);
            probe.token.take()
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
    }
}

#[derive(Clone, Copy)]
enum Container {
    Json,
    Glb,
}

fn fixture(container: Container) -> TestResult<(MemoryAssetIo, AssetPath)> {
    let stream_bytes = ROW_COUNT * 3 * size_of::<f32>();
    let mut binary = Vec::with_capacity(2 * stream_bytes);
    for row in 0..ROW_COUNT {
        for lane in [row as f32, 1.0, -1.0] {
            binary.extend_from_slice(&lane.to_le_bytes());
        }
    }
    binary.resize(2 * stream_bytes, 0);
    let mut document = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{
            "weights": [0.25],
            "primitives": [{
                "mode": 0, "attributes": {"POSITION": 0}, "targets": [{"POSITION": 1}]
            }]
        }],
        "buffers": [{"byteLength": binary.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": stream_bytes},
            {"buffer": 0, "byteOffset": stream_bytes, "byteLength": stream_bytes}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": ROW_COUNT, "type": "VEC3",
             "min": [0.0, 1.0, -1.0], "max": [(ROW_COUNT - 1) as f32, 1.0, -1.0]},
            {"bufferView": 1, "componentType": 5126, "count": ROW_COUNT, "type": "VEC3",
             "min": [0.0, 0.0, 0.0], "max": [0.0, 0.0, 0.0]}
        ]
    });
    let source = match container {
        Container::Json => {
            document["buffers"][0]["uri"] = json!("morph.bin");
            AssetPath::new("models/control.gltf")?
        }
        Container::Glb => AssetPath::new("models/control.glb")?,
    };
    let mut json = serde_json::to_vec(&document)?;
    json.resize(json.len() + 8_192, b' ');
    let io = MemoryAssetIo::new();
    let bytes = match container {
        Container::Json => {
            let _ = io.try_insert(
                AssetPath::new("models/morph.bin")?,
                Arc::<[u8]>::from(binary),
            )?;
            json
        }
        Container::Glb => {
            while !json.len().is_multiple_of(4) {
                json.push(b' ');
            }
            assert!(binary.len().is_multiple_of(4));
            let total = 12 + 8 + json.len() + 8 + binary.len();
            let mut glb = Vec::with_capacity(total);
            glb.extend_from_slice(b"glTF");
            glb.extend_from_slice(&2_u32.to_le_bytes());
            glb.extend_from_slice(&u32::try_from(total)?.to_le_bytes());
            for (kind, payload) in [(b"JSON", json.as_slice()), (b"BIN\0", binary.as_slice())] {
                glb.extend_from_slice(&u32::try_from(payload.len())?.to_le_bytes());
                glb.extend_from_slice(kind);
                glb.extend_from_slice(payload);
            }
            assert_eq!(glb.len(), total);
            glb
        }
    };
    let _ = io.try_insert(source.clone(), Arc::<[u8]>::from(bytes))?;
    Ok((io, source))
}

fn import_plan() -> TestResult<ImportPlan> {
    let mut registry = RegistryBuilder::new();
    registry.register(GltfImporter)?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    Ok(snapshot.plan(&ImportRequest::default(), OptionCompileLimits::default())?)
}

fn assert_scene(scene: &ValidatedScene) {
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    assert_eq!(scene.mesh_instance_count(), 1);
    let mesh = scene.meshes().next().unwrap();
    assert_eq!(mesh.morph_target_count(), 1);
    assert_eq!(mesh.default_morph_weights(), &[0.25]);
    let definition = mesh.morph_target_definitions().next().unwrap();
    assert_eq!(definition.stable_identity(), 0);
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::Points);
    assert_eq!(
        (
            primitive.vertex_count(),
            primitive.corner_count(),
            primitive.face_count()
        ),
        (ROW_COUNT as u32, ROW_COUNT as u32, ROW_COUNT as u32)
    );
    assert_eq!(primitive.indices(), None);
    assert_eq!(primitive.morph_target_count(), 1);
    let target = primitive.morph_targets().next().unwrap();
    assert_eq!(target.id(), definition.id());
    assert_eq!(target.delta_count(), 1);
    let positions = primitive.positions().unwrap();
    assert_eq!((positions.lanes(), positions.len()), (3, ROW_COUNT));
    let deltas = target.delta(MorphDeltaSemantic::Position).unwrap().data();
    assert_eq!(deltas.len(), ROW_COUNT);
    for row in 0..ROW_COUNT {
        assert_eq!(
            positions.element(row),
            Some(AttributeElement::F32(&[row as f32, 1.0, -1.0]))
        );
        assert_eq!(deltas.value(row), Some(Vec3::ZERO));
    }
    assert_eq!(positions.element(ROW_COUNT), None);
    assert_eq!(deltas.value(ROW_COUNT), None);
    assert_eq!(scene.mesh_instances().next().unwrap().morph_weights(), None);
}

fn assert_success(report: &ImportReport, container: Container) {
    assert_eq!(report.format().id().as_str(), FORMAT_ID);
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
    assert!(report.evidence().matches_scene(report.scene()));
    assert_scene(report.scene());
    let ledger = report.ledger();
    let cells = ledger.dimension(ImportDimension::MorphCells).unwrap();
    assert_eq!(
        (cells.attempted(), cells.consumed()),
        (MORPH_CELLS, MORPH_CELLS)
    );
    let dependency = ledger.dimension(ImportDimension::DependencyBytes).unwrap();
    let expected_bytes = match container {
        Container::Json => (2 * ROW_COUNT * 3 * size_of::<f32>()) as u64,
        Container::Glb => 0,
    };
    assert_eq!(dependency.consumed(), expected_bytes);
    assert!(ledger.committed_canonical_members() > 0);
    assert_eq!(ledger.terminal_cause(), ImportTerminalCause::Success);
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_evidence_bytes(), 0);
}

fn assert_cancelled(failure: &ImportFailure, completed_rows: usize) {
    assert_eq!(failure.cause().owner(), CauseOwner::Operation);
    assert!(
        matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled {
                    origin: CancellationOrigin::Signal,
                }))
        ),
        "expected signal cancellation inside morph materialization, got {:?}",
        failure.cause()
    );
    assert_eq!(failure.selected_format().unwrap().as_str(), FORMAT_ID);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    let ledger = failure.ledger();
    assert!(matches!(
        ledger.terminal_cause(),
        ImportTerminalCause::Failure(cause) if cause.class() == ImportTerminalFailureClass::Control
    ));
    let cells = ledger.dimension(ImportDimension::MorphCells).unwrap();
    assert_eq!(cells.attempted(), MORPH_CELLS);
    assert_eq!(cells.consumed(), 2 + 3 * completed_rows as u64);
    assert!(cells.consumed() < cells.attempted());
    assert_eq!(ledger.committed_canonical_members(), 0);
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_evidence_bytes(), 0);
    for dimension in [
        ImportDimension::MorphCells,
        ImportDimension::WorkUnits,
        ImportDimension::PrimaryBytes,
        ImportDimension::DependencyBytes,
        ImportDimension::CanonicalOutputBytes,
    ] {
        let row = ledger.dimension(dimension).unwrap();
        assert!(row.consumed() <= row.attempted(), "{dimension:?}");
        assert_eq!(row.current_live_bytes(), 0, "{dimension:?}");
    }
    assert_eq!(
        ledger
            .dimension(ImportDimension::CanonicalOutputBytes)
            .unwrap()
            .committed(),
        0
    );
}

fn check_morph_cancellation(container: Container) -> TestResult {
    let (io, source) = fixture(container)?;
    let plan = import_plan()?;
    let baseline = plan.read_asset(&io, source.clone(), ImportOperationControl::default())?;
    assert_success(&baseline, container);
    let (retained_scene, retained_evidence) = baseline.into_parts();

    let token = CancellationToken::new();
    let probe = MorphProbeGuard::new(CANCEL_AFTER, token.clone());
    let result = plan.read_asset(
        &io,
        source.clone(),
        ImportOperationControl::default().with_cancellation(token.clone()),
    );
    let physical = probe.finish();
    let failure = result
        .expect_err("the morph completion marker must interrupt the real parser")
        .into_operation_failure()?;
    assert_eq!(physical.cancelled_at, Some(CANCEL_AFTER));
    assert!(physical.token.is_none());
    assert!(token.is_cancelled());
    let completed_rows = physical.completed_rows;
    assert!((CANCEL_AFTER..ROW_COUNT).contains(&completed_rows));
    assert_eq!(physical.last_row, [0.0; 3]);
    assert_cancelled(&failure, completed_rows);
    assert!(retained_evidence.matches_scene(&retained_scene));
    assert_scene(&retained_scene);

    // Check reuse and ownership before the responsiveness assertion.
    let fresh = ImportOperationControl::default().with_cancellation(CancellationToken::new());
    let reused = plan.read_asset(&io, source.clone(), fresh)?;
    assert_success(&reused, container);
    let cloned_plan = plan.clone();
    let fresh = ImportOperationControl::default().with_cancellation(CancellationToken::new());
    let cloned = cloned_plan.read_asset(&io, source, fresh)?;
    assert_success(&cloned, container);
    drop(cloned);
    drop(reused);
    drop(failure);
    drop(cloned_plan);
    drop(plan);
    drop(io);
    assert!(retained_evidence.matches_scene(&retained_scene));
    drop(retained_evidence);
    assert_scene(&retained_scene);

    let further_rows = completed_rows - CANCEL_AFTER;
    assert!(
        further_rows <= MAX_FURTHER_ROWS,
        "real morph VEC3 loop appended {further_rows} further rows after cancellation at \
         row {CANCEL_AFTER}; completed={completed_rows}, bound={MAX_FURTHER_ROWS}, \
         MorphCells={}",
        2 + 3 * completed_rows as u64
    );
    Ok(())
}

#[test]
fn json_morph_cancellation_has_bounded_physical_prefix() -> TestResult {
    check_morph_cancellation(Container::Json)
}

#[test]
fn glb_morph_cancellation_has_bounded_physical_prefix() -> TestResult {
    check_morph_cancellation(Container::Glb)
}

#[test]
fn morph_completion_probe_is_reset_on_unwind() {
    let unwind = std::panic::catch_unwind(|| {
        let _probe = MorphProbeGuard::new(CANCEL_AFTER, CancellationToken::new());
        panic!("exercise scoped morph probe cleanup");
    });
    assert!(unwind.is_err());
    MORPH_PROBE.with(|slot| assert!(slot.borrow().is_none()));
    let probe = MorphProbeGuard::new(CANCEL_AFTER, CancellationToken::new());
    assert_eq!(probe.finish().completed_rows, 0);
}
