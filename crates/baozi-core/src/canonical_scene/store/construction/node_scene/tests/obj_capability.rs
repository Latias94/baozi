use std::sync::atomic::AtomicU64;

use baozi_operation::{CancellationToken, PhysicalStorageAccount, StorePreparationFailureClass};

use crate::SceneValidationLimits;
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::construction::node_scene::semantic::{
    NodeSceneGeometryAttributePayload, NodeSceneGeometryAttributeSemantic,
    NodeSceneGeometryF32Cursor, NodeSceneGeometryU32Cursor,
};
use crate::canonical_scene::store::construction::node_scene::snapshot::{
    NodeSceneSnapshot, NodeSceneSnapshotLimits,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneConstructionRejection, NodeSceneConstructionTerminal, NodeSceneSuccessParts,
};
use crate::canonical_scene::store::construction::{STORE_DIMENSION, SceneConstructionOperation};
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AuthoringRange, DeformationAuthoringBatch,
    GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryAuthoringFailure,
    GeometryAuthoringSemanticError, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, LogicalMeshLocalId,
    LogicalMorphAuthoring, MeshInstanceDeformationAuthoring, NodeLocalId, Presence,
    PrimitiveDeformationAuthoring, SceneAuthoringBatch,
};
use crate::geometry::TopologyKind;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::{
    assert_released, rich_failure, single_trs_batch, successful_parts, terminal_failure_class,
    test_account, test_factory, test_limits, test_limits_with_work,
};

const OBJ_POSITIONS: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
const OBJ_TEXCOORDS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.5, 0.5]];
const OBJ_NORMALS: [[f32; 3]; 2] = [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
const KNOWN_OBJ_CAPABILITY_DIGEST: [u8; 32] = [
    0x79, 0x85, 0x9f, 0xac, 0x88, 0x5f, 0x80, 0x54, 0x79, 0x4d, 0x06, 0xe2, 0x7a, 0x9b, 0x67, 0x5c,
    0xd4, 0x7b, 0xc2, 0x1c, 0xc0, 0x6f, 0x50, 0x4d, 0x80, 0x7b, 0xbf, 0xc9, 0xf2, 0x0e, 0xe7, 0xce,
];
const OBJ_FACES: [ObjTriangle; 2] = [
    ObjTriangle {
        corners: [
            ObjCorner::new(0, 0, 0),
            ObjCorner::new(1, 1, 0),
            ObjCorner::new(2, 2, 0),
        ],
        smoothing: ObjSmoothing::Group(7),
    },
    ObjTriangle {
        corners: [
            ObjCorner::new(0, 3, 1),
            ObjCorner::new(2, 2, 1),
            ObjCorner::new(1, 1, 1),
        ],
        smoothing: ObjSmoothing::Off,
    },
];

#[derive(Clone, Copy)]
struct ObjCorner {
    position: u32,
    texcoord: Option<u32>,
    normal: Option<u32>,
}

impl ObjCorner {
    const fn new(position: u32, texcoord: u32, normal: u32) -> Self {
        Self {
            position,
            texcoord: Some(texcoord),
            normal: Some(normal),
        }
    }
}

#[derive(Clone, Copy)]
enum ObjSmoothing {
    Off,
    Group(u32),
}

impl ObjSmoothing {
    const fn canonical_value(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Group(group) => group,
        }
    }
}

#[derive(Clone, Copy)]
struct ObjTriangle {
    corners: [ObjCorner; 3],
    smoothing: ObjSmoothing,
}

#[derive(Clone, Copy)]
struct NormalizedObjSource<'source> {
    positions: &'source [[f32; 3]],
    texcoords: &'source [[f32; 2]],
    normals: &'source [[f32; 3]],
    faces: &'source [ObjTriangle],
}

struct ObjCanonicalProjection<'source> {
    positions: &'source [[f32; 3]],
    texcoords: &'source [[f32; 2]],
    normals: &'source [[f32; 3]],
    position_indices: Vec<u32>,
    texcoord_indices: Vec<u32>,
    normal_indices: Vec<u32>,
    smoothing_groups: Vec<u32>,
}

pub(in crate::canonical_scene::store) const OBJ_SOURCE_VECTOR_SCHEMA_ID: &str =
    "baozi.normalized-obj-capability-source.v1";

pub(in crate::canonical_scene::store) struct ObjCapabilityMutationObservation {
    pub(in crate::canonical_scene::store) field_class: &'static str,
    pub(in crate::canonical_scene::store) source_vector: Vec<u8>,
    pub(in crate::canonical_scene::store) snapshot_digest: [u8; 32],
}

pub(in crate::canonical_scene::store) struct ObjCapabilityVectorObservations {
    pub(in crate::canonical_scene::store) source_shape: ObjCapabilitySourceShapeObservation,
    pub(in crate::canonical_scene::store) source_vector: Vec<u8>,
    pub(in crate::canonical_scene::store) baseline_snapshot_digest: [u8; 32],
    pub(in crate::canonical_scene::store) equivalent_source_vector: Vec<u8>,
    pub(in crate::canonical_scene::store) equivalent_snapshot_digest: [u8; 32],
    pub(in crate::canonical_scene::store) mutations: [ObjCapabilityMutationObservation; 4],
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct ObjCapabilitySourceShapeObservation {
    pub(in crate::canonical_scene::store) position_count: usize,
    pub(in crate::canonical_scene::store) texcoord_count: usize,
    pub(in crate::canonical_scene::store) normal_count: usize,
    pub(in crate::canonical_scene::store) face_count: usize,
    pub(in crate::canonical_scene::store) corner_count: usize,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct ObjCapabilityCanonicalShapeObservation {
    pub(in crate::canonical_scene::store) topology: &'static str,
    pub(in crate::canonical_scene::store) vertex_count: u32,
    pub(in crate::canonical_scene::store) corner_count: u32,
    pub(in crate::canonical_scene::store) face_count: u32,
    pub(in crate::canonical_scene::store) vertex_position_elements: u32,
    pub(in crate::canonical_scene::store) corner_normal_elements: u32,
    pub(in crate::canonical_scene::store) corner_texcoord_elements: u32,
    pub(in crate::canonical_scene::store) face_smoothing_group_elements: u32,
}

impl ObjCanonicalProjection<'_> {
    fn with_scene_input<R>(&self, use_input: impl FnOnce(SceneAuthoringBatch<'_>) -> R) -> R {
        let attributes = [
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                self.positions.as_flattened(),
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Normal,
                AttributeDomain::Corner,
                3,
                self.normals.as_flattened(),
                Presence::Present(&self.normal_indices),
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Corner,
                2,
                self.texcoords.as_flattened(),
                Presence::Present(&self.texcoord_indices),
            ),
            GeometryAttributeAuthoring::u32(
                AttributeSemantic::SmoothingGroup,
                AttributeDomain::Face,
                1,
                &self.smoothing_groups,
            ),
        ];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&self.position_indices),
            Presence::Absent,
            AuthoringRange::new(0, 4),
        )];
        let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let instances = [GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(0),
            LogicalMeshLocalId::new(0),
        )];
        let geometry =
            GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &instances);
        let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
        let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
        let mesh_instance_deformations = [MeshInstanceDeformationAuthoring::new(
            Presence::Absent,
            Presence::Absent,
        )];
        let deformation = DeformationAuthoringBatch::new(
            &[],
            &[],
            &logical_morphs,
            &primitive_deformations,
            &mesh_instance_deformations,
        );

        use_input(SceneAuthoringBatch::new(
            single_trs_batch(),
            AppearanceAuthoringBatch::empty(),
            geometry,
            deformation,
            AnimationAuthoringBatch::empty(),
        ))
    }
}

fn obj_source<'source>(
    texcoords: &'source [[f32; 2]],
    normals: &'source [[f32; 3]],
    faces: &'source [ObjTriangle],
) -> NormalizedObjSource<'source> {
    NormalizedObjSource {
        positions: &OBJ_POSITIONS,
        texcoords,
        normals,
        faces,
    }
}

fn project_obj_source<'source>(
    source: NormalizedObjSource<'source>,
) -> ObjCanonicalProjection<'source> {
    let mut position_indices = Vec::with_capacity(source.faces.len() * 3);
    let mut texcoord_indices = Vec::with_capacity(source.faces.len() * 3);
    let mut normal_indices = Vec::with_capacity(source.faces.len() * 3);
    let mut smoothing_groups = Vec::with_capacity(source.faces.len());

    for face in source.faces {
        smoothing_groups.push(face.smoothing.canonical_value());
        for corner in face.corners {
            position_indices.push(corner.position);

            let texcoord = corner
                .texcoord
                .expect("the complete-channel OBJ fixture must retain every corner texcoord");
            texcoord_indices.push(texcoord);

            let normal = corner
                .normal
                .expect("the complete-channel OBJ fixture must retain every corner normal");
            normal_indices.push(normal);
        }
    }

    ObjCanonicalProjection {
        positions: source.positions,
        texcoords: source.texcoords,
        normals: source.normals,
        position_indices,
        texcoord_indices,
        normal_indices,
        smoothing_groups,
    }
}

fn encode_obj_source(source: NormalizedObjSource<'_>) -> Vec<u8> {
    fn push_len(encoded: &mut Vec<u8>, len: usize) {
        let len = u64::try_from(len).expect("OBJ source vector length must fit u64");
        encoded.extend_from_slice(&len.to_le_bytes());
    }

    fn push_optional_index(encoded: &mut Vec<u8>, value: Option<u32>) {
        match value {
            Some(value) => {
                encoded.push(1);
                encoded.extend_from_slice(&value.to_le_bytes());
            }
            None => encoded.push(0),
        }
    }

    let mut encoded = Vec::new();
    encoded.extend_from_slice(OBJ_SOURCE_VECTOR_SCHEMA_ID.as_bytes());
    encoded.push(0);

    encoded.push(b'p');
    push_len(&mut encoded, source.positions.len());
    for position in source.positions {
        for value in position {
            encoded.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }

    encoded.push(b't');
    push_len(&mut encoded, source.texcoords.len());
    for texcoord in source.texcoords {
        for value in texcoord {
            encoded.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }

    encoded.push(b'n');
    push_len(&mut encoded, source.normals.len());
    for normal in source.normals {
        for value in normal {
            encoded.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }

    encoded.push(b'f');
    push_len(&mut encoded, source.faces.len());
    for face in source.faces {
        encoded.extend_from_slice(&face.smoothing.canonical_value().to_le_bytes());
        for corner in face.corners {
            encoded.extend_from_slice(&corner.position.to_le_bytes());
            push_optional_index(&mut encoded, corner.texcoord);
            push_optional_index(&mut encoded, corner.normal);
        }
    }
    encoded
}

fn build_obj_scene(
    account: &PhysicalStorageAccount,
    projection: &ObjCanonicalProjection<'_>,
) -> NodeSceneSuccessParts {
    build_obj_scene_with_generation_source(account, projection, SceneGenerationId::reserve)
}

fn build_obj_scene_with_generation_source(
    account: &PhysicalStorageAccount,
    projection: &ObjCanonicalProjection<'_>,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> NodeSceneSuccessParts {
    projection.with_scene_input(|input| {
        let prepared = SceneConstructionOperation::with_account_for_scene_input(
            account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
        )
        .expect("normalized OBJ Scene input admission")
        .try_validate()
        .unwrap_or_else(|_| panic!("normalized OBJ Scene input must pass central validation"))
        .prepare_with_generation_source(reserve_generation)
        .expect("normalized OBJ Scene publication");
        successful_parts(prepared)
    })
}

pub(in crate::canonical_scene::store) fn build_obj_capability_scene_for_probe(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
) -> NodeSceneSuccessParts {
    let projection = project_obj_source(obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &OBJ_FACES));
    build_obj_scene_with_generation_source(account, &projection, || {
        SceneGenerationId::reserve_from(generation_source)
    })
}

pub(in crate::canonical_scene::store) fn collect_obj_capability_vectors_for_probe()
-> ObjCapabilityVectorObservations {
    fn observe(
        account: &PhysicalStorageAccount,
        source: NormalizedObjSource<'_>,
    ) -> (Vec<u8>, [u8; 32]) {
        let source_vector = encode_obj_source(source);
        let projection = project_obj_source(source);
        let snapshot = snapshot_projection(account, &projection);
        (source_vector, snapshot.digest)
    }

    let factory = test_factory();
    let account = test_account(&factory, 256);
    let baseline_source = obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &OBJ_FACES);
    let source_shape = ObjCapabilitySourceShapeObservation {
        position_count: baseline_source.positions.len(),
        texcoord_count: baseline_source.texcoords.len(),
        normal_count: baseline_source.normals.len(),
        face_count: baseline_source.faces.len(),
        corner_count: baseline_source
            .faces
            .iter()
            .map(|face| face.corners.len())
            .sum(),
    };
    let (source_vector, baseline_snapshot_digest) = observe(&account, baseline_source);

    let mut reordered_texcoords = OBJ_TEXCOORDS;
    reordered_texcoords.swap(0, 3);
    let mut remapped_faces = OBJ_FACES;
    for face in &mut remapped_faces {
        for corner in &mut face.corners {
            corner.texcoord = match corner.texcoord {
                Some(0) => Some(3),
                Some(3) => Some(0),
                other => other,
            };
        }
    }
    let (equivalent_source_vector, equivalent_snapshot_digest) = observe(
        &account,
        obj_source(&reordered_texcoords, &OBJ_NORMALS, &remapped_faces),
    );

    let mut position_faces = OBJ_FACES;
    position_faces[1].corners[0].position = 1;
    let (position_source, position_digest) = observe(
        &account,
        obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &position_faces),
    );

    let mut texcoord_faces = OBJ_FACES;
    texcoord_faces[1].corners[0].texcoord = Some(0);
    let (texcoord_source, texcoord_digest) = observe(
        &account,
        obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &texcoord_faces),
    );

    let mut normal_faces = OBJ_FACES;
    normal_faces[1].corners[0].normal = Some(0);
    let (normal_source, normal_digest) = observe(
        &account,
        obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &normal_faces),
    );

    let mut smoothing_faces = OBJ_FACES;
    smoothing_faces[0].smoothing = ObjSmoothing::Group(8);
    let (smoothing_source, smoothing_digest) = observe(
        &account,
        obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &smoothing_faces),
    );

    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);

    ObjCapabilityVectorObservations {
        source_shape,
        source_vector,
        baseline_snapshot_digest,
        equivalent_source_vector,
        equivalent_snapshot_digest,
        mutations: [
            ObjCapabilityMutationObservation {
                field_class: "position_index",
                source_vector: position_source,
                snapshot_digest: position_digest,
            },
            ObjCapabilityMutationObservation {
                field_class: "texcoord_index",
                source_vector: texcoord_source,
                snapshot_digest: texcoord_digest,
            },
            ObjCapabilityMutationObservation {
                field_class: "normal_index",
                source_vector: normal_source,
                snapshot_digest: normal_digest,
            },
            ObjCapabilityMutationObservation {
                field_class: "smoothing_group",
                source_vector: smoothing_source,
                snapshot_digest: smoothing_digest,
            },
        ],
    }
}

fn snapshot_limits(bytes: usize, items: usize) -> NodeSceneSnapshotLimits {
    NodeSceneSnapshotLimits::try_new(bytes, items, 1_000_000)
        .expect("OBJ capability snapshot limits")
}

fn snapshot_projection(
    account: &PhysicalStorageAccount,
    projection: &ObjCanonicalProjection<'_>,
) -> NodeSceneSnapshot {
    let parts = build_obj_scene(account, projection);
    let snapshot = parts
        .scene
        .capture_snapshot(snapshot_limits(0, 0), None)
        .expect("complete OBJ capability digest");
    drop(parts);
    snapshot
}

fn assert_f32_values(mut values: NodeSceneGeometryF32Cursor<'_>, expected: &[f32]) {
    assert_eq!(values.declared_count(), expected.len());
    for expected in expected {
        assert_eq!(
            values.next().expect("typed f32 payload read"),
            Some(*expected)
        );
    }
    assert_eq!(values.next().expect("typed f32 payload exhaustion"), None);
    values
        .finish()
        .expect("complete typed f32 payload traversal");
}

fn assert_u32_values(mut values: NodeSceneGeometryU32Cursor<'_>, expected: &[u32]) {
    assert_eq!(values.declared_count(), expected.len());
    for expected in expected {
        assert_eq!(
            values.next().expect("typed u32 payload read"),
            Some(*expected)
        );
    }
    assert_eq!(values.next().expect("typed u32 payload exhaustion"), None);
    values
        .finish()
        .expect("complete typed u32 payload traversal");
}

pub(in crate::canonical_scene::store) fn inspect_obj_capability_semantics_for_probe(
    parts: &NodeSceneSuccessParts,
) -> ObjCapabilityCanonicalShapeObservation {
    let view = parts
        .scene
        .semantic_view()
        .expect("published OBJ Scene semantic view");
    assert_eq!(view.node_count(), 1);
    assert_eq!(view.root_count(), 1);
    assert_eq!(view.logical_mesh_count(), 1);
    assert_eq!(view.primitive_count(), 1);
    assert_eq!(view.attribute_count(), 4);
    assert_eq!(view.mesh_instance_count(), 1);

    let mut cursor = view.into_cursor().expect("OBJ Scene semantic cursor");
    let mut roots = cursor.take_roots().expect("OBJ Scene roots");
    assert_eq!(roots.next().expect("OBJ root read"), Some(0));
    assert_eq!(roots.next().expect("OBJ root exhaustion"), None);
    roots.finish().expect("complete OBJ root traversal");

    let node = cursor
        .next_node()
        .expect("OBJ node read")
        .expect("OBJ source node");
    assert_eq!(node.source_index(), 0);
    let mut children = node.into_children();
    assert_eq!(children.next().expect("OBJ child exhaustion"), None);
    children.finish().expect("complete OBJ child traversal");
    assert!(cursor.next_node().expect("OBJ node exhaustion").is_none());

    {
        let appearance = cursor.appearance().expect("OBJ appearance cursor");
        assert!(
            appearance
                .next_image()
                .expect("OBJ image exhaustion")
                .is_none()
        );
        assert!(
            appearance
                .next_sampler()
                .expect("OBJ sampler exhaustion")
                .is_none()
        );
        assert!(
            appearance
                .next_texture()
                .expect("OBJ texture exhaustion")
                .is_none()
        );
        assert!(
            appearance
                .next_material()
                .expect("OBJ material exhaustion")
                .is_none()
        );
        assert!(
            appearance
                .next_binding()
                .expect("OBJ material-binding exhaustion")
                .is_none()
        );
    }

    let geometry = cursor.geometry().expect("OBJ geometry cursor");

    let logical_mesh = geometry
        .next_logical_mesh()
        .expect("OBJ logical mesh read")
        .expect("OBJ logical mesh");
    assert_eq!(logical_mesh.source_index(), 0);
    assert_eq!(logical_mesh.primitive_offset(), 0);
    assert_eq!(logical_mesh.primitive_count(), 1);
    assert!(
        geometry
            .next_logical_mesh()
            .expect("OBJ logical mesh exhaustion")
            .is_none()
    );

    let mut primitive = geometry
        .next_primitive()
        .expect("OBJ primitive read")
        .expect("OBJ primitive");
    let topology = primitive.topology();
    let vertex_count = primitive.vertex_count();
    let corner_count = primitive.corner_count();
    let face_count = primitive.face_count();
    assert_eq!(topology, TopologyKind::TriangleList);
    assert_eq!(vertex_count, 3);
    assert_eq!(corner_count, 6);
    assert_eq!(face_count, 2);
    assert_eq!(primitive.material(), None);
    assert_eq!(primitive.attribute_offset(), 0);
    assert_eq!(primitive.attribute_count(), 4);
    assert!(!primitive.polygon_boundaries_present());
    assert_u32_values(
        primitive
            .take_indices()
            .expect("OBJ position-index payload"),
        &[0, 1, 2, 0, 2, 1],
    );
    assert!(
        geometry
            .next_primitive()
            .expect("OBJ primitive exhaustion")
            .is_none()
    );

    let position = geometry
        .next_attribute()
        .expect("OBJ POSITION read")
        .expect("OBJ POSITION");
    assert_eq!(
        position.semantic(),
        NodeSceneGeometryAttributeSemantic::Position
    );
    assert_eq!(position.domain(), AttributeDomain::Vertex);
    let vertex_position_elements = position.element_count();
    assert_eq!(vertex_position_elements, 3);
    match position.into_payload() {
        NodeSceneGeometryAttributePayload::DenseF32 { values } => {
            assert_f32_values(values, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
        }
        _ => panic!("OBJ POSITION must retain a dense typed f32 payload"),
    }

    let normal = geometry
        .next_attribute()
        .expect("OBJ NORMAL read")
        .expect("OBJ NORMAL");
    assert_eq!(
        normal.semantic(),
        NodeSceneGeometryAttributeSemantic::Normal
    );
    assert_eq!(normal.domain(), AttributeDomain::Corner);
    let corner_normal_elements = normal.element_count();
    assert_eq!(corner_normal_elements, 6);
    match normal.into_payload() {
        NodeSceneGeometryAttributePayload::DenseF32 { values } => assert_f32_values(
            values,
            &[
                0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0,
                0.0, -1.0,
            ],
        ),
        _ => panic!("OBJ NORMAL must retain a dense typed f32 payload"),
    }

    let texcoord = geometry
        .next_attribute()
        .expect("OBJ TEXCOORD read")
        .expect("OBJ TEXCOORD");
    assert_eq!(
        texcoord.semantic(),
        NodeSceneGeometryAttributeSemantic::Texcoord(0)
    );
    assert_eq!(texcoord.domain(), AttributeDomain::Corner);
    let corner_texcoord_elements = texcoord.element_count();
    assert_eq!(corner_texcoord_elements, 6);
    match texcoord.into_payload() {
        NodeSceneGeometryAttributePayload::DenseF32 { values } => assert_f32_values(
            values,
            &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 0.5, 0.0, 1.0, 1.0, 0.0],
        ),
        _ => panic!("OBJ TEXCOORD must retain a dense typed f32 payload"),
    }

    let smoothing = geometry
        .next_attribute()
        .expect("OBJ smoothing read")
        .expect("OBJ smoothing");
    assert_eq!(
        smoothing.semantic(),
        NodeSceneGeometryAttributeSemantic::SmoothingGroup
    );
    assert_eq!(smoothing.domain(), AttributeDomain::Face);
    let face_smoothing_group_elements = smoothing.element_count();
    assert_eq!(face_smoothing_group_elements, 2);
    match smoothing.into_payload() {
        NodeSceneGeometryAttributePayload::DenseU32 { values } => {
            assert_u32_values(values, &[7, 0]);
        }
        _ => panic!("OBJ smoothing groups must retain a dense typed u32 payload"),
    }
    assert!(
        geometry
            .next_attribute()
            .expect("OBJ attribute exhaustion")
            .is_none()
    );

    let instance = geometry
        .next_mesh_instance()
        .expect("OBJ mesh-instance read")
        .expect("OBJ mesh instance");
    assert_eq!(instance.owner_node(), 0);
    assert_eq!(instance.logical_mesh(), 0);
    assert!(
        geometry
            .next_mesh_instance()
            .expect("OBJ mesh-instance exhaustion")
            .is_none()
    );

    let deformation = cursor.deformation().expect("OBJ deformation cursor");
    assert!(
        deformation
            .next_skin()
            .expect("OBJ skin exhaustion")
            .is_none()
    );
    let mut logical_morph = deformation
        .next_logical_morph()
        .expect("OBJ logical-morph read")
        .expect("OBJ aligned logical-morph row");
    assert_eq!(logical_morph.source_index(), 0);
    assert_eq!(logical_morph.definition_offset(), 0);
    assert_eq!(logical_morph.definition_count(), 0);
    let mut default_weights = logical_morph
        .take_default_weights()
        .expect("OBJ logical-morph default weights");
    assert_eq!(
        default_weights.next().expect("OBJ default exhaustion"),
        None
    );
    default_weights
        .finish()
        .expect("complete OBJ default-weight traversal");
    assert!(
        deformation
            .next_logical_morph()
            .expect("OBJ logical-morph exhaustion")
            .is_none()
    );
    assert!(
        deformation
            .next_morph_definition()
            .expect("OBJ morph-definition exhaustion")
            .is_none()
    );
    let mut primitive_deformation = deformation
        .next_primitive()
        .expect("OBJ primitive-deformation read")
        .expect("OBJ aligned primitive-deformation row");
    assert_eq!(primitive_deformation.source_index(), 0);
    assert!(primitive_deformation.take_influences().is_none());
    assert_eq!(primitive_deformation.morph_target_offset(), 0);
    assert_eq!(primitive_deformation.morph_target_count(), 0);
    assert!(
        deformation
            .next_primitive()
            .expect("OBJ primitive-deformation exhaustion")
            .is_none()
    );
    assert!(
        deformation
            .next_morph_target()
            .expect("OBJ morph-target exhaustion")
            .is_none()
    );
    assert!(
        deformation
            .next_morph_delta()
            .expect("OBJ morph-delta exhaustion")
            .is_none()
    );
    let mut mesh_instance_deformation = deformation
        .next_mesh_instance()
        .expect("OBJ mesh-instance deformation read")
        .expect("OBJ aligned mesh-instance deformation row");
    assert_eq!(mesh_instance_deformation.source_index(), 0);
    assert_eq!(mesh_instance_deformation.skin(), None);
    assert!(!mesh_instance_deformation.morph_weights_present());
    assert!(mesh_instance_deformation.take_morph_weights().is_none());
    assert!(
        deformation
            .next_mesh_instance()
            .expect("OBJ mesh-instance deformation exhaustion")
            .is_none()
    );

    let animation = cursor.animation().expect("OBJ animation cursor");
    assert!(
        animation
            .next_clip()
            .expect("OBJ animation-clip exhaustion")
            .is_none()
    );
    assert!(
        animation
            .next_channel()
            .expect("OBJ animation-channel exhaustion")
            .is_none()
    );
    cursor.finish().expect("complete OBJ semantic traversal");

    ObjCapabilityCanonicalShapeObservation {
        topology: "triangle_list",
        vertex_count,
        corner_count,
        face_count,
        vertex_position_elements,
        corner_normal_elements,
        corner_texcoord_elements,
        face_smoothing_group_elements,
    }
}

pub(in crate::canonical_scene::store) fn capture_obj_capability_snapshots_for_probe(
    parts: &NodeSceneSuccessParts,
) -> (NodeSceneSnapshot, NodeSceneSnapshot) {
    let full = parts
        .scene
        .capture_snapshot(snapshot_limits(8_192, 64), None)
        .expect("complete OBJ snapshot with review preview");
    let bounded = parts
        .scene
        .capture_snapshot(snapshot_limits(96, 1), None)
        .expect("complete OBJ snapshot with bounded preview");
    (full, bounded)
}

#[test]
fn normalized_obj_corner_fixture_closes_the_canonical_scene_path() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let projection = project_obj_source(obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &OBJ_FACES));
    let parts = build_obj_scene(&account, &projection);

    let canonical_shape = inspect_obj_capability_semantics_for_probe(&parts);
    assert_eq!(canonical_shape.topology, "triangle_list");
    assert_eq!(canonical_shape.vertex_count, 3);
    assert_eq!(canonical_shape.corner_count, 6);
    assert_eq!(canonical_shape.face_count, 2);
    let (full, bounded) = capture_obj_capability_snapshots_for_probe(&parts);

    assert_eq!(full.digest, KNOWN_OBJ_CAPABILITY_DIGEST);
    assert_eq!(bounded.digest, full.digest);
    assert_eq!(bounded.totals, full.totals);
    assert_eq!(full.totals.logical_mesh_count, 1);
    assert_eq!(full.totals.primitive_count, 1);
    assert_eq!(full.totals.attribute_count, 4);
    assert_eq!(full.totals.primitive_index_count, 6);
    assert_eq!(full.totals.canonical_scalar_count, 41);
    assert_eq!(full.totals.f32_scalar_count, 39);
    assert_eq!(full.totals.u32_scalar_count, 2);
    assert!(
        full.preview
            .text
            .contains("primitive[0] topology=triangle_list vertices=3 corners=6 faces=2")
    );
    assert!(full.preview.text.contains("semantic=normal"));
    assert!(full.preview.text.contains("semantic=texcoord"));
    assert!(full.preview.text.contains("semantic=smoothing_group"));
    assert!(bounded.preview.text.len() <= 96);
    assert!(bounded.preview.bytes_truncated);
    assert!(bounded.preview.items_truncated);
    assert!(bounded.preview.items_omitted_by_limit > 0);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn obj_corner_selections_and_smoothing_mutations_change_the_complete_digest() {
    let observations = collect_obj_capability_vectors_for_probe();

    for (index, mutation) in observations.mutations.iter().enumerate() {
        assert_ne!(
            mutation.snapshot_digest, observations.baseline_snapshot_digest,
            "OBJ mutation {} must change the canonical digest",
            mutation.field_class,
        );
        for earlier in &observations.mutations[..index] {
            assert_ne!(
                mutation.snapshot_digest, earlier.snapshot_digest,
                "OBJ mutations {} and {} must remain distinguishable",
                mutation.field_class, earlier.field_class,
            );
        }
    }
}

#[test]
fn equivalent_obj_texcoord_source_order_has_one_canonical_digest() {
    let observations = collect_obj_capability_vectors_for_probe();

    assert_ne!(
        observations.equivalent_source_vector,
        observations.source_vector,
    );
    assert_eq!(
        observations.equivalent_snapshot_digest,
        observations.baseline_snapshot_digest,
    );
}

#[test]
fn incomplete_obj_corner_projection_is_rejected_by_central_validation() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let mut projection = project_obj_source(obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &OBJ_FACES));
    assert_eq!(projection.texcoord_indices.pop(), Some(1));

    projection.with_scene_input(|input| {
        let rejected = match SceneConstructionOperation::with_account_for_scene_input(
            &account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
        )
        .expect("invalid OBJ projection operation admission")
        .try_validate()
        {
            Ok(_) => panic!("incomplete OBJ corner projection must not validate"),
            Err(rejected) => rejected,
        };

        match rejected.failure() {
            NodeSceneConstructionRejection::GeometryAuthoring(
                GeometryAuthoringFailure::Semantic(
                    GeometryAuthoringSemanticError::SourceProjectionCardinality {
                        attribute: 2,
                        expected: 6,
                        actual: 5,
                    },
                ),
            ) => {}
            other => panic!("unexpected incomplete OBJ projection rejection: {other:?}"),
        }

        let terminal = rejected
            .prepare_minimal_fallback()
            .expect("invalid OBJ projection fallback")
            .handoff();
        assert!(matches!(
            &terminal,
            NodeSceneConstructionTerminal::MinimalFallback(_)
        ));
        drop(terminal);
    });

    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn obj_construction_one_short_work_limit_publishes_only_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let projection = project_obj_source(obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &OBJ_FACES));

    let baseline = build_obj_scene(&account, &projection);
    let successful_work = baseline
        .report
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("successful OBJ work evidence")
        .attempted();
    assert_eq!(
        baseline
            .report
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("successful OBJ work evidence")
            .consumed(),
        successful_work,
    );
    drop(baseline);

    let work_limit = successful_work
        .checked_sub(1)
        .expect("OBJ construction must consume work");
    projection.with_scene_input(|input| {
        let validated = SceneConstructionOperation::with_account_for_scene_input(
            &account,
            test_limits_with_work(work_limit),
            input,
            SceneValidationLimits::default(),
        )
        .expect("work-bounded OBJ Scene input admission")
        .try_validate()
        .unwrap_or_else(|_| panic!("one-short total work must still admit OBJ validation"));

        let failure = rich_failure(
            validated
                .prepare()
                .expect("one-short OBJ work limit must prepare rich failure"),
        );
        let work = failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("failed OBJ work evidence");
        assert_eq!(work.attempted(), successful_work);
        assert_eq!(work.consumed(), work_limit);
        assert_eq!(
            terminal_failure_class(failure.ledger_evidence()),
            StorePreparationFailureClass::Ledger,
        );
        assert_released(failure, &account);
    });
}

#[test]
fn obj_cancellation_after_validation_publishes_only_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let projection = project_obj_source(obj_source(&OBJ_TEXCOORDS, &OBJ_NORMALS, &OBJ_FACES));
    let cancellation = CancellationToken::new();

    projection.with_scene_input(|input| {
        let validated =
            SceneConstructionOperation::with_account_and_cancellation_for_scene_input_for_test(
                &account,
                test_limits(),
                input,
                SceneValidationLimits::default(),
                cancellation.clone(),
            )
            .expect("cancellable OBJ Scene input admission")
            .try_validate()
            .unwrap_or_else(|_| panic!("OBJ input must validate before cancellation"));

        cancellation.cancel();
        let failure = rich_failure(
            validated
                .prepare()
                .expect("cancelled OBJ construction must prepare rich failure"),
        );
        assert_eq!(
            terminal_failure_class(failure.ledger_evidence()),
            StorePreparationFailureClass::Control,
        );
        assert_released(failure, &account);
    });
}
