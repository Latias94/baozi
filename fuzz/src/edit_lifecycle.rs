//! One bounded import/edit/read workflow, shared with the fixed corpus regressions.

use std::{hint::black_box, sync::OnceLock};

use baozi::{
    AssetPath, AttributeDomain, AttributeElement, Axis, FrontFaceWinding, Handedness, PrimitiveId,
    PrimitiveRef, SceneGenerationId, SceneLookupError, TargetSpace, TopologyKind, ValidatedScene,
    postprocess::{
        ExistingNormals, NormalCause, NormalMode, NormalOptions, PostProcessBudgets,
        PostProcessCancellationOrigin, PostProcessCancellationToken, PostProcessControl,
        PostProcessControlCause, PostProcessReport, PostProcessRunDisposition, PostProcessRunError,
        PostProcessRunFailure, PostProcessRunOutcome, SpaceConversionCause, SpaceConversionOptions,
        TangentPolicy, TrianglePreparationOptions, TriangulationCause, convert_scene_space,
        generate_normals, prepare_triangles, triangle_vertices, triangulate_primitive,
    },
};
use baozi_format_obj::{FORMAT_ID as OBJ_FORMAT_ID, ObjImporter};
use baozi_format_ply::PlyImporter;
use baozi_import::{ImportOperationControl, ImportPlan, ResourceLimits};

use super::support;

const HEADER_BYTES: usize = 4;
const MAX_ASSET_BYTES: usize = 8 * 1024;
const MAX_TOPOLOGY_CELLS: usize = 512;
const MAX_READ_ELEMENTS: usize = 128 * 1024;

/// The original operation reports/errors are retained so seed tests can assert concrete outcomes.
pub struct LifecycleRun {
    pub input_generation: SceneGenerationId,
    pub input_digest: [u8; 32],
    pub scene: ValidatedScene,
    pub triangulation: Option<Result<PostProcessReport, PostProcessRunError>>,
    pub normals: Option<Result<PostProcessReport, PostProcessRunError>>,
    pub preparation: Option<Result<PostProcessReport, PostProcessRunError>>,
    pub conversion: Option<Result<PostProcessReport, PostProcessRunError>>,
}

pub fn run(data: &[u8]) -> Option<LifecycleRun> {
    if !(HEADER_BYTES..=HEADER_BYTES + MAX_ASSET_BYTES).contains(&data.len()) {
        return None;
    }
    static OBJ_REQUEST: OnceLock<ImportPlan> = OnceLock::new();
    static PLY_REQUEST: OnceLock<ImportPlan> = OnceLock::new();
    let (request, path) = if data[0] & 1 == 0 {
        (
            OBJ_REQUEST.get_or_init(|| {
                support::import_plan_with_limits(ObjImporter, OBJ_FORMAT_ID, import_limits())
            }),
            "lifecycle.obj",
        )
    } else {
        (
            PLY_REQUEST.get_or_init(|| {
                support::import_plan_with_limits(PlyImporter, "org.baozi.ply", import_limits())
            }),
            "lifecycle.ply",
        )
    };
    let source = AssetPath::new(path).expect("the static lifecycle path is valid");
    let report = request
        .read_bytes(
            source,
            &data[HEADER_BYTES..],
            ImportOperationControl::default(),
        )
        .ok()?;
    let (mut scene, import_evidence) = report.into_parts();
    assert!(import_evidence.matches_scene(&scene));
    let input_generation = scene.generation();
    let input_digest = read_digest(&scene);

    let selected = usize::from(data[0] >> 1) % scene.primitive_count().max(1);
    let operations = data[1];
    let policy = data[2];
    let cancellation = data[3] % 4;
    let mut triangulation = None;
    let mut normals = None;
    let mut preparation = None;
    let mut conversion = None;

    // Order is fixed, with at most one edit per operation. Only the selected primitive is edited.
    if operations & 8 != 0 {
        let mut targets = Vec::new();
        targets.try_reserve_exact(scene.primitive_count()).ok()?;
        for primitive in scene.meshes().flat_map(|mesh| mesh.primitives()) {
            if matches!(
                primitive.topology(),
                TopologyKind::Polygons
                    | TopologyKind::TriangleList
                    | TopologyKind::TriangleStrip
                    | TopologyKind::TriangleFan
            ) {
                targets.push(primitive.id());
            }
        }
        let before = SceneState::capture(&scene, selected);
        let cancelled = matches!(cancellation, 1 | 2);
        let options = TrianglePreparationOptions {
            normals: Some(normal_options(policy)),
            budgets: PostProcessBudgets {
                max_work_actions: 4_000_000,
                storage_bytes: 16 * 1024 * 1024,
            },
            ..TrianglePreparationOptions::default()
        };
        let result = prepare_triangles(scene, &targets, options, control(cancelled));
        let (next, result) = finish_edit(result, before, cancelled);
        scene = next;
        preparation = Some(result);
    }

    if operations & 8 == 0
        && operations & 1 != 0
        && let Some(primitive) = primitive_at(&scene, selected)
        && matches!(
            primitive.topology(),
            TopologyKind::Polygons | TopologyKind::TriangleList
        )
    {
        let target = primitive.id();
        let before = SceneState::capture(&scene, selected);
        let cancelled = cancellation == 1;
        let result = triangulate_primitive(scene, target, control(cancelled));
        let (next, result) = finish_edit(result, before, cancelled);
        scene = next;
        triangulation = Some(result);
    }

    // Reacquire a current-generation ID after triangulation, including failed/unchanged attempts.
    if operations & 8 == 0
        && operations & 2 != 0
        && let Some(primitive) = primitive_at(&scene, selected)
        && primitive.topology() == TopologyKind::TriangleList
    {
        let target = primitive.id();
        let before = SceneState::capture(&scene, selected);
        let cancelled = cancellation == 2;
        let options = normal_options(policy);
        let result = generate_normals(
            scene,
            target,
            options,
            TangentPolicy::Preserve,
            control(cancelled),
        );
        let (next, result) = finish_edit(result, before, cancelled);
        scene = next;
        normals = Some(result);
    }

    if operations & 4 != 0 {
        let before = SceneState::capture(&scene, selected);
        let cancelled = cancellation == 3;
        let target = if policy & 4 == 0 {
            TargetSpace::STANDARD
        } else {
            TargetSpace::try_new(
                Handedness::Left,
                Axis::PositiveY,
                Axis::NegativeZ,
                0.01,
                FrontFaceWinding::CounterClockwise,
            )
            .expect("the fixed reflected centimeter target is valid")
        };
        let options = SpaceConversionOptions {
            target,
            source_assumptions: (policy & 8 == 0).then_some(TargetSpace::STANDARD),
        };
        let result = convert_scene_space(scene, options, control(cancelled));
        let (next, result) = finish_edit(result, before, cancelled);
        scene = next;
        conversion = Some(result);
    }

    // Final reads enumerate fresh handles, never an ID retained across a publication.
    read_digest(&scene);
    Some(LifecycleRun {
        input_generation,
        input_digest,
        scene,
        triangulation,
        normals,
        preparation,
        conversion,
    })
}

fn normal_options(policy: u8) -> NormalOptions {
    NormalOptions {
        mode: if policy & 1 == 0 {
            NormalMode::Smooth
        } else {
            NormalMode::Flat
        },
        existing: if policy & 2 == 0 {
            ExistingNormals::Preserve
        } else {
            ExistingNormals::Replace
        },
    }
}

fn primitive_at(scene: &ValidatedScene, selected: usize) -> Option<PrimitiveRef<'_>> {
    scene
        .meshes()
        .flat_map(|mesh| mesh.primitives())
        .nth(selected)
}

fn control(cancelled: bool) -> PostProcessControl {
    if !cancelled {
        return PostProcessControl::default();
    }
    // Deliberately deterministic pre-execution cancellation through the existing public token.
    let token = PostProcessCancellationToken::new();
    token.cancel();
    PostProcessControl::default().with_cancellation(token)
}

struct SceneState {
    generation: SceneGenerationId,
    digest: [u8; 32],
    target: Option<PrimitiveId>,
}

impl SceneState {
    fn capture(scene: &ValidatedScene, selected: usize) -> Self {
        Self {
            generation: scene.generation(),
            digest: read_digest(scene),
            target: primitive_at(scene, selected).map(|primitive| primitive.id()),
        }
    }

    fn assert_unchanged(&self, scene: &ValidatedScene) {
        assert_eq!(scene.generation(), self.generation);
        assert_eq!(read_digest(scene), self.digest);
        if let Some(target) = self.target {
            assert_eq!(
                scene
                    .primitive(target)
                    .expect("the original ID remains readable")
                    .id(),
                target
            );
        }
    }
}

fn finish_edit(
    result: Result<PostProcessRunOutcome, PostProcessRunFailure>,
    before: SceneState,
    cancelled: bool,
) -> (
    ValidatedScene,
    Result<PostProcessReport, PostProcessRunError>,
) {
    match result {
        Ok(outcome) => {
            assert!(!cancelled, "a pre-cancelled edit must reject");
            let (scene, report) = outcome.into_parts();
            assert_eq!(report.input_generation(), before.generation);
            assert_eq!(report.output_generation(), scene.generation());
            assert!(report.matches_scene(&scene));
            assert_eq!(report.resources().current_live_bytes(), 0);
            assert_eq!(report.resources().active_reservations(), 0);
            match report.disposition() {
                PostProcessRunDisposition::Changed => {
                    assert_ne!(scene.generation(), before.generation);
                    if let Some(stale) = before.target {
                        assert!(matches!(
                            scene.primitive(stale),
                            Err(SceneLookupError::GenerationMismatch { .. })
                        ));
                    }
                    read_digest(&scene);
                }
                PostProcessRunDisposition::Unchanged => before.assert_unchanged(&scene),
                other => panic!("unhandled successful disposition: {other:?}"),
            }
            (scene, Ok(report))
        }
        Err(failure) => {
            before.assert_unchanged(failure.input());
            // Allocation/startup can fail before the first cancellation checkpoint. Fixed seed
            // regressions still require actual token cancellation under healthy resource admission.
            let startup_failure = match failure.error() {
                PostProcessRunError::ResourcePolicyInitialization => true,
                PostProcessRunError::Triangulation { failure } => {
                    matches!(failure.cause(), TriangulationCause::Start { .. })
                }
                PostProcessRunError::Normals { failure } => {
                    matches!(failure.cause(), NormalCause::Start { .. })
                }
                PostProcessRunError::SpaceConversion { failure } => {
                    matches!(failure.cause(), SpaceConversionCause::Start { .. })
                }
                PostProcessRunError::Preparation { failure } => {
                    matches!(
                        failure.cause(),
                        baozi::postprocess::PreparationCause::Start(_)
                    )
                }
                _ => false,
            };
            if cancelled && !startup_failure {
                assert_token_cancellation(failure.error());
            }
            let (scene, error) = failure.into_parts();
            (scene, Err(error))
        }
    }
}

pub fn assert_token_cancellation(error: &PostProcessRunError) {
    let cause = match error {
        PostProcessRunError::Triangulation { failure } => match failure.cause() {
            TriangulationCause::Control { cause } => cause,
            other => panic!("expected token cancellation, found {other:?}"),
        },
        PostProcessRunError::Normals { failure } => match failure.cause() {
            NormalCause::Control { cause } => cause,
            other => panic!("expected token cancellation, found {other:?}"),
        },
        PostProcessRunError::SpaceConversion { failure } => match failure.cause() {
            SpaceConversionCause::Control { cause } => cause,
            other => panic!("expected token cancellation, found {other:?}"),
        },
        PostProcessRunError::Preparation { failure } => match failure.cause() {
            baozi::postprocess::PreparationCause::Control(cause) => cause,
            other => panic!("expected token cancellation, found {other:?}"),
        },
        other => panic!("expected a started edit with token cancellation, found {other:?}"),
    };
    assert_eq!(
        *cause,
        PostProcessControlCause::Cancelled {
            origin: PostProcessCancellationOrigin::Signal
        }
    );
}

/// Check all bounded borrowed reads as well as the complete digest, not only a snapshot preview.
pub fn read_digest(scene: &ValidatedScene) -> [u8; 32] {
    assert!(scene.mesh_count() <= 4);
    assert!(scene.primitive_count() <= 8);
    assert!(scene.node_count() <= 64);
    for node in scene.nodes() {
        assert_eq!(
            scene.node(node.id()).expect("current node lookup").id(),
            node.id()
        );
        let _ = node.authored_transform().local_matrix();
        for child in node.children() {
            assert!(scene.node(child.id()).is_ok());
        }
    }
    for root in scene.roots() {
        assert!(scene.node(root.id()).is_ok());
    }
    for instance in scene.mesh_instances() {
        assert!(scene.mesh_instance(instance.id()).is_ok());
        assert!(scene.node(instance.node().id()).is_ok());
        assert!(scene.mesh(instance.mesh().id()).is_ok());
    }
    let mut elements = 0;
    for mesh in scene.meshes() {
        assert!(scene.mesh(mesh.id()).is_ok());
        for primitive in mesh.primitives() {
            assert!(scene.primitive(primitive.id()).is_ok());
            assert!(mesh.primitive(primitive.id()).is_ok());
            assert!(primitive.corner_count() as usize <= 3 * MAX_TOPOLOGY_CELLS);
            if let Some(indices) = primitive.indices() {
                assert_eq!(indices.len(), primitive.corner_count() as usize);
                assert!(
                    indices
                        .iter()
                        .all(|index| *index < primitive.vertex_count())
                );
            }
            if let Some(faces) = primitive.polygon_face_sizes() {
                assert_eq!(faces.len(), primitive.face_count() as usize);
                assert_eq!(
                    faces.iter().map(|count| u64::from(*count)).sum::<u64>(),
                    u64::from(primitive.corner_count())
                );
            }
            for attribute in primitive.attributes() {
                let count = match attribute.domain() {
                    AttributeDomain::Vertex => primitive.vertex_count(),
                    AttributeDomain::Corner => primitive.corner_count(),
                    AttributeDomain::Face => primitive.face_count(),
                    AttributeDomain::Instance => panic!("primitive cannot own Instance fields"),
                };
                assert_eq!(attribute.len(), count as usize);
                elements += attribute.len();
                assert!(elements <= MAX_READ_ELEMENTS);
                for index in 0..attribute.len() {
                    assert_element(
                        attribute.element(index).expect("validated element"),
                        attribute.lanes(),
                    );
                }
                assert!(attribute.element(attribute.len()).is_none());
            }
            if primitive.topology() == TopologyKind::TriangleList {
                let tuples = triangle_vertices(primitive).expect("triangle layout");
                assert_eq!(tuples.len(), primitive.corner_count() as usize);
                for tuple in tuples.vertices() {
                    assert_eq!(tuple.source_vertex().primitive, primitive.id());
                    assert!(tuple.source_vertex().index < primitive.vertex_count());
                    for field in tuple.attributes() {
                        elements += 1;
                        assert!(elements <= MAX_READ_ELEMENTS);
                        assert_element(field.element(), field.attribute().lanes());
                    }
                }
            }
        }
    }
    support::capture_snapshot(scene).digest()
}

fn assert_element(element: AttributeElement<'_>, lanes: u8) {
    // Force scalar loads through the public slices as well as checking their shape.
    let count = match element {
        AttributeElement::F32(values) => {
            for value in values {
                black_box(value.to_bits());
            }
            values.len()
        }
        AttributeElement::F64(values) => {
            for value in values {
                black_box(value.to_bits());
            }
            values.len()
        }
        AttributeElement::U32(values) => {
            for value in values {
                black_box(*value);
            }
            values.len()
        }
        AttributeElement::I32(values) => {
            for value in values {
                black_box(*value);
            }
            values.len()
        }
    };
    assert_eq!(count, usize::from(lanes));
}

fn import_limits() -> ResourceLimits {
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = MAX_ASSET_BYTES as u64;
    limits.max_dependency_asset_bytes = 0;
    limits.max_total_asset_bytes = MAX_ASSET_BYTES as u64;
    limits.max_open_assets = 1;
    limits.max_dependency_references = 0;
    limits.max_include_depth = 0;
    limits.max_data_uri_bytes = 0;
    limits.max_live_bytes = 16 * 1024 * 1024;
    limits.max_work_units = 1_000_000;
    limits.max_detection_work = 16_384;
    limits.max_decode_work = 65_536;
    limits.max_decoded_utf8_bytes = MAX_ASSET_BYTES as u64;
    limits.max_line_index_bytes = 64 * 1024;
    limits.max_tokens = 8_192;
    limits.max_path_length = 256;
    limits.max_string_bytes = 256;
    limits.max_canonical_string_bytes = 8 * 1024;
    limits.max_line_bytes = 1024;
    limits.max_token_bytes = 256;
    limits.max_records = 1024;
    limits.max_meshes = 4;
    limits.max_primitives = 8;
    limits.max_vertices = 128;
    limits.max_vertex_attribute_streams_per_mesh = 16;
    limits.max_total_vertex_attribute_streams = 64;
    limits.max_total_vertex_attribute_cells = 8_192;
    limits.max_total_topology_cells = MAX_TOPOLOGY_CELLS;
    limits.max_faces = 64;
    limits.max_influence_cells = 0;
    limits.max_morph_cells = 0;
    limits.max_animation_cells = 0;
    limits.max_canonical_payload_bytes = 2 * 1024 * 1024;
    limits.max_graph_nodes = 64;
    limits.max_graph_edges = 128;
    limits.max_graph_depth = 16;
    limits.max_graph_work = 4_096;
    limits.max_resource_graph_nodes = 4;
    limits.max_resource_graph_relations = 4;
    limits.max_resource_graph_depth = 4;
    limits.max_resource_graph_work = 4_096;
    limits.max_input_manifest_bytes = 16 * 1024;
    limits.max_input_manifest_work = 65_536;
    limits.max_diagnostics = 16;
    limits.max_attempted_diagnostics = 64;
    limits.max_diagnostic_bytes = 16 * 1024;
    limits.max_diagnostic_message_bytes = 1024;
    limits.max_attempted_diagnostic_bytes = 64 * 1024;
    limits.max_source_scope_items = 1024;
    limits.max_source_scope_detail_count = 1024;
    limits.max_source_scope_detail_bytes = 64 * 1024;
    limits.max_source_scope_digest_bytes = 64 * 1024;
    limits
}
