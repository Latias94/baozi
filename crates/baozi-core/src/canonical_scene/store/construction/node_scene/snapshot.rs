use baozi_operation::{CancellationToken, CapacityPlan, CapacityPlanError};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::attribute::AttributeDomain;
use crate::entity::{AuthoredTransform, AuthoredTransformKind};
use crate::geometry::TopologyKind;

use super::ValidatedNodeScene;
use super::semantic::{
    NodeSceneGeometryAttributePayload, NodeSceneGeometryAttributeSemantic, NodeSceneGeometryCursor,
    NodeSceneGeometryF32Cursor, NodeSceneGeometryNormalization, NodeSceneGeometryPayloadKind,
    NodeSceneGeometryScalarKind, NodeSceneGeometryU32Cursor, NodeSceneSemanticError,
};

const CANONICAL_SCENE_SNAPSHOT_DOMAIN: &str = "baozi.canonical-scene.snapshot";
const CANONICAL_SCENE_SNAPSHOT_SCHEMA_REVISION: u32 = 2;
const NODE_SCENE_SNAPSHOT_ALGORITHM_ID: &str = "sha256";
const NODE_SCENE_SNAPSHOT_POLL_INTERVAL: u64 = 64;
const MAX_PREVIEW_BYTES: usize = 1 << 20;
const MAX_COLLECTION_ITEMS: usize = 1 << 20;
const MAX_WORK: u64 = 100_000_000;
#[cfg(test)]
const TEST_TRANSCRIPT_MAX_BYTES: usize = 512;

const TAG_DOMAIN: u8 = 1;
const TAG_SCHEMA_REVISION: u8 = 2;
const TAG_ALGORITHM: u8 = 3;
const TAG_ROOTS: u8 = 4;
const TAG_NODES: u8 = 5;
const TAG_NODE: u8 = 6;
const TAG_CHILDREN: u8 = 7;
const TAG_ROOT_REFERENCE: u8 = 8;
const TAG_CHILD_REFERENCE: u8 = 9;
const TAG_TRANSFORM: u8 = 10;
const TAG_COLLECTION_TOTAL: u8 = 11;
const TAG_PRESENCE: u8 = 12;
const TAG_REFERENCE: u8 = 13;
const TAG_FLOAT_BITS: u8 = 14;

const TAG_GEOMETRY: u8 = 15;
const TAG_MATERIALS: u8 = 16;
const TAG_MATERIAL: u8 = 17;
const TAG_LOGICAL_MESHES: u8 = 18;
const TAG_LOGICAL_MESH: u8 = 19;
const TAG_PRIMITIVES: u8 = 20;
const TAG_PRIMITIVE: u8 = 21;
const TAG_POLYGON_BOUNDARIES: u8 = 22;
const TAG_INDICES: u8 = 23;
const TAG_MATERIAL_REFERENCE: u8 = 24;
const TAG_ATTRIBUTES: u8 = 25;
const TAG_ATTRIBUTE: u8 = 26;
const TAG_SEMANTIC: u8 = 27;
const TAG_SEMANTIC_SET: u8 = 28;
const TAG_ATTRIBUTE_DOMAIN: u8 = 29;
const TAG_SCALAR_KIND: u8 = 30;
const TAG_LANE_COUNT: u8 = 31;
const TAG_NORMALIZATION: u8 = 32;
const TAG_PAYLOAD_KIND: u8 = 33;
const TAG_ELEMENT_COUNT: u8 = 34;
const TAG_EXPLICIT_ELEMENT_COUNT: u8 = 35;
const TAG_CANONICAL_SCALAR_COUNT: u8 = 36;
const TAG_VALUES: u8 = 37;
const TAG_SPARSE_INDICES: u8 = 38;
const TAG_SPARSE_DEFAULT: u8 = 39;
const TAG_MESH_INSTANCES: u8 = 40;
const TAG_MESH_INSTANCE: u8 = 41;
const TAG_OWNER_NODE: u8 = 42;
const TAG_LOGICAL_MESH_REFERENCE: u8 = 43;
const TAG_PRIMITIVE_OFFSET: u8 = 44;
const TAG_ATTRIBUTE_OFFSET: u8 = 45;
const TAG_TOPOLOGY: u8 = 46;
const TAG_VERTEX_COUNT: u8 = 47;
const TAG_CORNER_COUNT: u8 = 48;
const TAG_FACE_COUNT: u8 = 49;
const TAG_U32_VALUE: u8 = 50;

const PRESENCE_ABSENT: u8 = 0;
const PRESENCE_PRESENT: u8 = 1;
const TRANSFORM_MATRIX: u8 = 1;
const TRANSFORM_TRS: u8 = 2;

const TOPOLOGY_POINTS: u8 = 0;
const TOPOLOGY_LINE_LIST: u8 = 1;
const TOPOLOGY_LINE_STRIP: u8 = 2;
const TOPOLOGY_LINE_LOOP: u8 = 3;
const TOPOLOGY_TRIANGLE_LIST: u8 = 4;
const TOPOLOGY_TRIANGLE_STRIP: u8 = 5;
const TOPOLOGY_TRIANGLE_FAN: u8 = 6;
const TOPOLOGY_POLYGONS: u8 = 7;

const SEMANTIC_POSITION: u8 = 0;
const SEMANTIC_NORMAL: u8 = 1;
const SEMANTIC_TANGENT: u8 = 2;
const SEMANTIC_TEXCOORD: u8 = 3;
const SEMANTIC_COLOR: u8 = 4;
const SEMANTIC_SMOOTHING_GROUP: u8 = 5;

const ATTRIBUTE_DOMAIN_VERTEX: u8 = 0;
const ATTRIBUTE_DOMAIN_CORNER: u8 = 1;
const ATTRIBUTE_DOMAIN_FACE: u8 = 2;
const SCALAR_F32: u8 = 0;
const SCALAR_U32: u8 = 1;
const NORMALIZATION_NONE: u8 = 0;
const PAYLOAD_DENSE: u8 = 0;
const PAYLOAD_SPARSE: u8 = 1;

/// Bounds a private canonical Scene snapshot capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSnapshotLimits {
    max_preview_bytes: usize,
    max_collection_items: usize,
    max_work: u64,
}

impl NodeSceneSnapshotLimits {
    /// Validates hard caps before traversal or allocation begins.
    ///
    /// Zero preview limits are valid and still produce a complete digest. A zero
    /// work limit is also valid, but capture will reject before encoding its first
    /// semantic value.
    pub(in crate::canonical_scene::store) fn try_new(
        max_preview_bytes: usize,
        max_collection_items: usize,
        max_work: u64,
    ) -> Result<Self, NodeSceneSnapshotError> {
        validate_usize_limit(
            NodeSceneSnapshotLimit::PreviewBytes,
            max_preview_bytes,
            MAX_PREVIEW_BYTES,
        )?;
        validate_usize_limit(
            NodeSceneSnapshotLimit::CollectionItems,
            max_collection_items,
            MAX_COLLECTION_ITEMS,
        )?;
        if max_work > MAX_WORK {
            return Err(NodeSceneSnapshotError::InvalidLimit {
                limit: NodeSceneSnapshotLimit::Work,
                value: max_work,
                maximum: MAX_WORK,
            });
        }
        Ok(Self {
            max_preview_bytes,
            max_collection_items,
            max_work,
        })
    }
}

/// Identifies the configured snapshot bound that rejected capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneSnapshotLimit {
    PreviewBytes,
    CollectionItems,
    Work,
}

/// Identifies the digest algorithm committed by the snapshot schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneSnapshotAlgorithm {
    Sha256,
}

/// Complete semantic totals independent from the bounded preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSnapshotTotals {
    pub(in crate::canonical_scene::store) node_count: usize,
    pub(in crate::canonical_scene::store) root_count: usize,
    pub(in crate::canonical_scene::store) edge_count: usize,
    pub(in crate::canonical_scene::store) material_count: usize,
    pub(in crate::canonical_scene::store) logical_mesh_count: usize,
    pub(in crate::canonical_scene::store) primitive_count: usize,
    pub(in crate::canonical_scene::store) attribute_count: usize,
    pub(in crate::canonical_scene::store) mesh_instance_count: usize,
    pub(in crate::canonical_scene::store) polygon_boundary_count: u64,
    pub(in crate::canonical_scene::store) primitive_index_count: u64,
    pub(in crate::canonical_scene::store) sparse_index_count: u64,
    pub(in crate::canonical_scene::store) canonical_scalar_count: u64,
    pub(in crate::canonical_scene::store) f32_scalar_count: u64,
    pub(in crate::canonical_scene::store) u32_scalar_count: u64,
    pub(in crate::canonical_scene::store) transcript_bytes: u64,
    pub(in crate::canonical_scene::store) work_units: u64,
}

/// Human-reviewable prefix produced from the same semantic events as the digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSnapshotPreview {
    pub(in crate::canonical_scene::store) text: String,
    pub(in crate::canonical_scene::store) items_omitted_by_limit: u64,
    pub(in crate::canonical_scene::store) bytes_truncated: bool,
    pub(in crate::canonical_scene::store) items_truncated: bool,
}

/// Private, complete canonical Scene digest and a review-bounded semantic preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSnapshot {
    pub(in crate::canonical_scene::store) domain: &'static str,
    pub(in crate::canonical_scene::store) schema_revision: u32,
    pub(in crate::canonical_scene::store) algorithm: NodeSceneSnapshotAlgorithm,
    pub(in crate::canonical_scene::store) digest: [u8; 32],
    pub(in crate::canonical_scene::store) preview: NodeSceneSnapshotPreview,
    pub(in crate::canonical_scene::store) totals: NodeSceneSnapshotTotals,
}

struct NodeSceneCaptureOutput {
    snapshot: NodeSceneSnapshot,
    #[cfg(test)]
    transcript: Option<TestCanonicalTranscript>,
}

#[cfg(test)]
struct TestCanonicalTranscript {
    bytes: Vec<u8>,
    complete: bool,
}

#[derive(Clone, Copy)]
struct GeometryCollectionCounts {
    materials: usize,
    logical_meshes: usize,
    primitives: usize,
    attributes: usize,
    mesh_instances: usize,
}

#[derive(Clone, Copy, Default)]
struct GeometryPayloadTotals {
    polygon_boundaries: u64,
    primitive_indices: u64,
    sparse_indices: u64,
    canonical_scalars: u64,
    f32_scalars: u64,
    u32_scalars: u64,
}

impl GeometryPayloadTotals {
    fn add_polygon_boundaries(&mut self, count: usize) -> Result<(), NodeSceneSnapshotError> {
        checked_add_count(&mut self.polygon_boundaries, count, "polygon boundaries")
    }

    fn add_primitive_indices(&mut self, count: usize) -> Result<(), NodeSceneSnapshotError> {
        checked_add_count(&mut self.primitive_indices, count, "primitive indices")
    }

    fn add_sparse_indices(&mut self, count: usize) -> Result<(), NodeSceneSnapshotError> {
        checked_add_count(&mut self.sparse_indices, count, "sparse indices")
    }

    fn add_attribute_scalars(
        &mut self,
        kind: NodeSceneGeometryScalarKind,
        count: u64,
    ) -> Result<(), NodeSceneSnapshotError> {
        self.canonical_scalars = self.canonical_scalars.checked_add(count).ok_or(
            NodeSceneSnapshotError::SemanticTotalOverflow {
                total: "canonical attribute scalars",
            },
        )?;
        let typed = match kind {
            NodeSceneGeometryScalarKind::F32 => &mut self.f32_scalars,
            NodeSceneGeometryScalarKind::U32 => &mut self.u32_scalars,
        };
        *typed = typed
            .checked_add(count)
            .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow {
                total: "typed attribute scalars",
            })?;
        Ok(())
    }
}

/// Capture failed before a complete snapshot could be returned.
#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeSceneSnapshotError {
    #[error("snapshot {limit:?} limit {value} exceeds hard maximum {maximum}")]
    InvalidLimit {
        limit: NodeSceneSnapshotLimit,
        value: u64,
        maximum: u64,
    },
    #[error("canonical Scene snapshot work {observed} exceeds limit {limit}")]
    WorkBudgetExceeded { observed: u64, limit: u64 },
    #[error("canonical Scene snapshot capture was cancelled")]
    Cancelled,
    #[error("canonical Scene view is inconsistent: {reason}")]
    InconsistentView { reason: &'static str },
    #[error("canonical Scene snapshot transcript length overflowed")]
    TranscriptLengthOverflow,
    #[error("canonical Scene snapshot preview item total overflowed")]
    PreviewItemCountOverflow,
    #[error("canonical Scene snapshot preview capacity planning failed: {source}")]
    PreviewCapacity {
        #[source]
        source: CapacityPlanError,
    },
    #[error("canonical Scene snapshot preview allocation failed")]
    PreviewAllocation,
    #[error("canonical Scene snapshot total {total} overflowed")]
    SemanticTotalOverflow { total: &'static str },
}

impl ValidatedNodeScene {
    /// Captures a complete private canonical Scene digest and bounded preview.
    pub(in crate::canonical_scene::store) fn capture_snapshot(
        &self,
        limits: NodeSceneSnapshotLimits,
        cancellation: Option<&CancellationToken>,
    ) -> Result<NodeSceneSnapshot, NodeSceneSnapshotError> {
        capture_node_scene_with_encoder(self, limits, cancellation, CanonicalDigestEncoder::new())
            .map(|output| output.snapshot)
    }
}

#[cfg(test)]
fn capture_node_scene_with_test_transcript(
    scene: &ValidatedNodeScene,
    limits: NodeSceneSnapshotLimits,
    cancellation: Option<&CancellationToken>,
) -> Result<(NodeSceneSnapshot, TestCanonicalTranscript), NodeSceneSnapshotError> {
    let output = capture_node_scene_with_encoder(
        scene,
        limits,
        cancellation,
        CanonicalDigestEncoder::with_test_transcript(),
    )?;
    let transcript = output
        .transcript
        .expect("test transcript capture must return its bounded recorder");
    Ok((output.snapshot, transcript))
}

fn capture_node_scene_with_encoder(
    scene: &ValidatedNodeScene,
    limits: NodeSceneSnapshotLimits,
    cancellation: Option<&CancellationToken>,
    mut transcript: CanonicalDigestEncoder,
) -> Result<NodeSceneCaptureOutput, NodeSceneSnapshotError> {
    let mut work = SnapshotWorkMeter::new(limits.max_work, cancellation);
    work.poll()?;
    work.observe()?;
    let source = scene.semantic_view().map_err(map_snapshot_invariant)?;
    let node_count = source.node_count();
    let root_count = source.root_count();
    let edge_count = source.edge_count();
    let geometry_counts = GeometryCollectionCounts {
        materials: source.material_count(),
        logical_meshes: source.logical_mesh_count(),
        primitives: source.primitive_count(),
        attributes: source.attribute_count(),
        mesh_instances: source.mesh_instance_count(),
    };

    if limits.max_preview_bytes != 0 {
        // Preview allocation is observable snapshot work and must be admitted
        // before capacity planning or allocator entry.
        work.observe()?;
    }
    let mut preview = PreviewSink::new(limits.max_preview_bytes, limits.max_collection_items)?;

    work.observe()?;
    transcript.tagged_bytes(TAG_DOMAIN, CANONICAL_SCENE_SNAPSHOT_DOMAIN.as_bytes())?;
    preview.header(
        CANONICAL_SCENE_SNAPSHOT_DOMAIN,
        CANONICAL_SCENE_SNAPSHOT_SCHEMA_REVISION,
        NODE_SCENE_SNAPSHOT_ALGORITHM_ID,
    );
    work.observe()?;
    transcript.tagged_u32(
        TAG_SCHEMA_REVISION,
        CANONICAL_SCENE_SNAPSHOT_SCHEMA_REVISION,
    )?;
    work.observe()?;
    transcript.tagged_bytes(TAG_ALGORITHM, NODE_SCENE_SNAPSHOT_ALGORITHM_ID.as_bytes())?;

    work.observe()?;
    transcript.write_tag(TAG_ROOTS)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_roots(root_count);
    let mut cursor = source.into_cursor().map_err(map_snapshot_invariant)?;
    let mut roots = cursor.take_roots().map_err(map_snapshot_invariant)?;
    for root_ordinal in 0..root_count {
        work.observe()?;
        let child = roots.next().map_err(map_snapshot_invariant)?.ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "root collection ended before declared total",
            },
        )?;
        transcript.write_tag(TAG_ROOT_REFERENCE)?;
        transcript.tagged_usize(TAG_REFERENCE, child)?;
        preview.reference(root_ordinal, child)?;
    }
    roots.finish().map_err(map_snapshot_invariant)?;
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, root_count)?;
    preview.end_collection();

    work.observe()?;
    transcript.write_tag(TAG_NODES)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_nodes(node_count);
    for _ in 0..node_count {
        work.observe()?;
        let node = cursor.next_node().map_err(map_snapshot_invariant)?.ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "node collection ended before declared total",
            },
        )?;
        let node_index = node.source_index();
        let transform = node.authored_transform();
        let child_count = node.child_count();
        transcript.write_tag(TAG_NODE)?;
        transcript.tagged_usize(TAG_REFERENCE, node_index)?;
        let render_node = preview.begin_node(node_index)?;
        write_transform(
            &mut transcript,
            &mut preview,
            &mut work,
            transform,
            render_node,
        )?;

        work.observe()?;
        transcript.write_tag(TAG_CHILDREN)?;
        transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
        preview.begin_children(render_node);
        let mut children = node.into_children();
        for child_ordinal in 0..child_count {
            work.observe()?;
            let child = children.next().map_err(map_snapshot_invariant)?.ok_or(
                NodeSceneSnapshotError::InconsistentView {
                    reason: "child collection ended before declared total",
                },
            )?;
            transcript.write_tag(TAG_CHILD_REFERENCE)?;
            transcript.tagged_usize(TAG_REFERENCE, child)?;
            preview.child_reference(render_node, child_ordinal, child)?;
        }
        children.finish().map_err(map_snapshot_invariant)?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, child_count)?;
        preview.end_node(render_node, child_count);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, node_count)?;

    let mut geometry = cursor.into_geometry().map_err(map_snapshot_invariant)?;
    let geometry_totals = write_geometry(
        &mut geometry,
        geometry_counts,
        &mut transcript,
        &mut preview,
        &mut work,
    )?;
    geometry.finish().map_err(map_snapshot_invariant)?;

    preview.summary(
        node_count,
        root_count,
        edge_count,
        geometry_counts,
        geometry_totals,
    );
    work.finish()?;
    let work_units = work.observed();
    let encoded = transcript.finish();
    Ok(NodeSceneCaptureOutput {
        snapshot: NodeSceneSnapshot {
            domain: CANONICAL_SCENE_SNAPSHOT_DOMAIN,
            schema_revision: CANONICAL_SCENE_SNAPSHOT_SCHEMA_REVISION,
            algorithm: NodeSceneSnapshotAlgorithm::Sha256,
            digest: encoded.digest,
            preview: preview.finish(),
            totals: NodeSceneSnapshotTotals {
                node_count,
                root_count,
                edge_count,
                material_count: geometry_counts.materials,
                logical_mesh_count: geometry_counts.logical_meshes,
                primitive_count: geometry_counts.primitives,
                attribute_count: geometry_counts.attributes,
                mesh_instance_count: geometry_counts.mesh_instances,
                polygon_boundary_count: geometry_totals.polygon_boundaries,
                primitive_index_count: geometry_totals.primitive_indices,
                sparse_index_count: geometry_totals.sparse_indices,
                canonical_scalar_count: geometry_totals.canonical_scalars,
                f32_scalar_count: geometry_totals.f32_scalars,
                u32_scalar_count: geometry_totals.u32_scalars,
                transcript_bytes: encoded.bytes,
                work_units,
            },
        },
        #[cfg(test)]
        transcript: encoded.transcript,
    })
}

fn write_geometry(
    cursor: &mut NodeSceneGeometryCursor<'_>,
    counts: GeometryCollectionCounts,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<GeometryPayloadTotals, NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_GEOMETRY)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_geometry();

    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_MATERIALS,
        "materials",
        counts.materials,
    )?;
    for _ in 0..counts.materials {
        work.observe()?;
        let material = cursor
            .next_material()
            .map_err(map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "material collection ended before declared total",
            })?;
        let source_index = material.source_index();
        transcript.write_tag(TAG_MATERIAL)?;
        transcript.tagged_usize(TAG_REFERENCE, source_index)?;
        let render = preview.begin_geometry_item("material", source_index)?;
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, counts.materials)?;

    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_LOGICAL_MESHES,
        "logical_meshes",
        counts.logical_meshes,
    )?;
    for _ in 0..counts.logical_meshes {
        work.observe()?;
        let logical_mesh = cursor
            .next_logical_mesh()
            .map_err(map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "logical-mesh collection ended before declared total",
            })?;
        let source_index = logical_mesh.source_index();
        let primitive_offset = logical_mesh.primitive_offset();
        let primitive_count = logical_mesh.primitive_count();
        transcript.write_tag(TAG_LOGICAL_MESH)?;
        transcript.tagged_usize(TAG_REFERENCE, source_index)?;
        transcript.tagged_usize(TAG_PRIMITIVE_OFFSET, primitive_offset)?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, primitive_count)?;
        let render = preview.begin_geometry_item("logical_mesh", source_index)?;
        preview.geometry_field_usize(render, "primitive_offset", primitive_offset);
        preview.geometry_field_usize(render, "primitive_count", primitive_count);
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, counts.logical_meshes)?;

    let mut totals = GeometryPayloadTotals::default();
    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_PRIMITIVES,
        "primitives",
        counts.primitives,
    )?;
    for _ in 0..counts.primitives {
        work.observe()?;
        let mut primitive = cursor
            .next_primitive()
            .map_err(map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "primitive collection ended before declared total",
            })?;
        let source_index = primitive.source_index();
        let topology = primitive.topology();
        let vertex_count = primitive.vertex_count();
        let corner_count = primitive.corner_count();
        let face_count = primitive.face_count();
        let polygon_boundaries_present = primitive.polygon_boundaries_present();
        let polygon_boundary_count = primitive.polygon_boundary_count();
        let indices_present = primitive.indices_present();
        let index_count = primitive.index_count();
        let material = primitive.material();
        let attribute_offset = primitive.attribute_offset();
        let attribute_count = primitive.attribute_count();

        totals.add_polygon_boundaries(polygon_boundary_count)?;
        totals.add_primitive_indices(index_count)?;
        transcript.write_tag(TAG_PRIMITIVE)?;
        transcript.tagged_usize(TAG_REFERENCE, source_index)?;
        transcript.tagged_u8(TAG_TOPOLOGY, encode_topology(topology))?;
        transcript.tagged_u32(TAG_VERTEX_COUNT, vertex_count)?;
        transcript.tagged_u32(TAG_CORNER_COUNT, corner_count)?;
        transcript.tagged_u32(TAG_FACE_COUNT, face_count)?;
        let render = preview.begin_geometry_item("primitive", source_index)?;
        preview.geometry_field_word(render, "topology", topology_name(topology));
        preview.geometry_field_u32(render, "vertices", vertex_count);
        preview.geometry_field_u32(render, "corners", corner_count);
        preview.geometry_field_u32(render, "faces", face_count);

        write_optional_u32_leaf(
            transcript,
            preview,
            work,
            TAG_POLYGON_BOUNDARIES,
            "polygon_boundaries",
            polygon_boundaries_present,
            polygon_boundary_count,
            primitive.take_polygon_boundaries(),
            render,
        )?;
        write_optional_u32_leaf(
            transcript,
            preview,
            work,
            TAG_INDICES,
            "indices",
            indices_present,
            index_count,
            primitive.take_indices(),
            render,
        )?;

        transcript.write_tag(TAG_MATERIAL_REFERENCE)?;
        transcript.tagged_u8(
            TAG_PRESENCE,
            if material.is_some() {
                PRESENCE_PRESENT
            } else {
                PRESENCE_ABSENT
            },
        )?;
        if let Some(material) = material {
            transcript.tagged_usize(TAG_REFERENCE, material)?;
        }
        preview.geometry_optional_reference(render, "material", material);
        transcript.write_tag(TAG_ATTRIBUTES)?;
        transcript.tagged_usize(TAG_ATTRIBUTE_OFFSET, attribute_offset)?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, attribute_count)?;
        preview.geometry_field_usize(render, "attribute_offset", attribute_offset);
        preview.geometry_field_usize(render, "attribute_count", attribute_count);
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, counts.primitives)?;

    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_ATTRIBUTES,
        "attributes",
        counts.attributes,
    )?;
    for _ in 0..counts.attributes {
        work.observe()?;
        let attribute = cursor
            .next_attribute()
            .map_err(map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "attribute collection ended before declared total",
            })?;
        let source_index = attribute.source_index();
        let semantic = attribute.semantic();
        let (semantic_kind, semantic_set) = encode_semantic(semantic);
        let domain = attribute.domain();
        let scalar_kind = attribute.scalar_kind();
        let lanes = attribute.lanes();
        let normalization = attribute.normalization();
        let payload_kind = attribute.payload_kind();
        let element_count = attribute.element_count();
        let explicit_element_count = attribute.explicit_element_count();
        let canonical_scalar_count = attribute.canonical_scalar_count();
        totals.add_attribute_scalars(scalar_kind, canonical_scalar_count)?;

        transcript.write_tag(TAG_ATTRIBUTE)?;
        transcript.tagged_usize(TAG_REFERENCE, source_index)?;
        transcript.tagged_u8(TAG_SEMANTIC, semantic_kind)?;
        transcript.tagged_u32(TAG_SEMANTIC_SET, semantic_set)?;
        transcript.tagged_u8(TAG_ATTRIBUTE_DOMAIN, encode_attribute_domain(domain)?)?;
        transcript.tagged_u8(TAG_SCALAR_KIND, encode_scalar_kind(scalar_kind))?;
        transcript.tagged_u8(TAG_LANE_COUNT, lanes)?;
        transcript.tagged_u8(TAG_NORMALIZATION, encode_normalization(normalization))?;
        transcript.tagged_u8(TAG_PAYLOAD_KIND, encode_payload_kind(payload_kind))?;
        transcript.tagged_u32(TAG_ELEMENT_COUNT, element_count)?;
        transcript.tagged_u32(TAG_EXPLICIT_ELEMENT_COUNT, explicit_element_count)?;
        transcript.tagged_u64(TAG_CANONICAL_SCALAR_COUNT, canonical_scalar_count)?;

        let render = preview.begin_geometry_item("attribute", source_index)?;
        preview.geometry_field_word(render, "semantic", semantic_name(semantic));
        preview.geometry_field_u32(render, "semantic_set", semantic_set);
        preview.geometry_field_word(render, "domain", attribute_domain_name(domain));
        preview.geometry_field_word(render, "scalar", scalar_kind_name(scalar_kind));
        preview.geometry_field_u32(render, "lanes", u32::from(lanes));
        preview.geometry_field_word(render, "normalization", "none");
        preview.geometry_field_word(render, "payload", payload_kind_name(payload_kind));
        preview.geometry_field_u32(render, "elements", element_count);
        preview.geometry_field_u32(render, "explicit", explicit_element_count);
        preview.geometry_field_u64(render, "canonical_scalars", canonical_scalar_count);

        write_attribute_payload(
            transcript,
            preview,
            work,
            attribute.into_payload(),
            scalar_kind,
            payload_kind,
            explicit_element_count,
            canonical_scalar_count,
            render,
            &mut totals,
        )?;
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, counts.attributes)?;

    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_MESH_INSTANCES,
        "mesh_instances",
        counts.mesh_instances,
    )?;
    for _ in 0..counts.mesh_instances {
        work.observe()?;
        let instance = cursor
            .next_mesh_instance()
            .map_err(map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "mesh-instance collection ended before declared total",
            })?;
        let source_index = instance.source_index();
        let owner_node = instance.owner_node();
        let logical_mesh = instance.logical_mesh();
        transcript.write_tag(TAG_MESH_INSTANCE)?;
        transcript.tagged_usize(TAG_REFERENCE, source_index)?;
        transcript.tagged_usize(TAG_OWNER_NODE, owner_node)?;
        transcript.tagged_usize(TAG_LOGICAL_MESH_REFERENCE, logical_mesh)?;
        let render = preview.begin_geometry_item("mesh_instance", source_index)?;
        preview.geometry_field_usize(render, "owner_node", owner_node);
        preview.geometry_field_usize(render, "logical_mesh", logical_mesh);
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, counts.mesh_instances)?;
    preview.end_geometry();
    Ok(totals)
}

fn begin_geometry_collection(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    name: &'static str,
    count: usize,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(tag)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_geometry_collection(name, count);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_optional_u32_leaf(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    name: &'static str,
    present: bool,
    expected_count: usize,
    cursor: Option<NodeSceneGeometryU32Cursor<'_>>,
    render: bool,
) -> Result<(), NodeSceneSnapshotError> {
    if present != cursor.is_some() {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "payload presence disagrees with its semantic cursor",
        });
    }
    match cursor {
        Some(cursor) => {
            if cursor.declared_count() != expected_count {
                return Err(NodeSceneSnapshotError::InconsistentView {
                    reason: "payload leaf count disagrees with its declared total",
                });
            }
            write_u32_leaf(transcript, preview, work, tag, name, cursor, render).map(|_| ())
        }
        None if expected_count == 0 => {
            write_absent_leaf(transcript, preview, work, tag, name, render)
        }
        None => Err(NodeSceneSnapshotError::InconsistentView {
            reason: "absent payload leaf has a nonzero declared total",
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_attribute_payload(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    payload: NodeSceneGeometryAttributePayload<'_>,
    scalar_kind: NodeSceneGeometryScalarKind,
    payload_kind: NodeSceneGeometryPayloadKind,
    explicit_element_count: u32,
    canonical_scalar_count: u64,
    render: bool,
    totals: &mut GeometryPayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    let observed_scalars = match payload {
        NodeSceneGeometryAttributePayload::DenseF32 { values }
            if scalar_kind == NodeSceneGeometryScalarKind::F32
                && payload_kind == NodeSceneGeometryPayloadKind::Dense =>
        {
            let values = write_f32_leaf(
                transcript, preview, work, TAG_VALUES, "values", values, render,
            )?;
            write_absent_leaf(
                transcript,
                preview,
                work,
                TAG_SPARSE_INDICES,
                "sparse_indices",
                render,
            )?;
            write_absent_leaf(
                transcript,
                preview,
                work,
                TAG_SPARSE_DEFAULT,
                "sparse_default",
                render,
            )?;
            values
        }
        NodeSceneGeometryAttributePayload::DenseU32 { values }
            if scalar_kind == NodeSceneGeometryScalarKind::U32
                && payload_kind == NodeSceneGeometryPayloadKind::Dense =>
        {
            let values = write_u32_leaf(
                transcript, preview, work, TAG_VALUES, "values", values, render,
            )?;
            write_absent_leaf(
                transcript,
                preview,
                work,
                TAG_SPARSE_INDICES,
                "sparse_indices",
                render,
            )?;
            write_absent_leaf(
                transcript,
                preview,
                work,
                TAG_SPARSE_DEFAULT,
                "sparse_default",
                render,
            )?;
            values
        }
        NodeSceneGeometryAttributePayload::SparseF32 {
            values,
            indices,
            default,
        } if scalar_kind == NodeSceneGeometryScalarKind::F32
            && payload_kind == NodeSceneGeometryPayloadKind::Sparse =>
        {
            let values = write_f32_leaf(
                transcript, preview, work, TAG_VALUES, "values", values, render,
            )?;
            let indices = write_u32_leaf(
                transcript,
                preview,
                work,
                TAG_SPARSE_INDICES,
                "sparse_indices",
                indices,
                render,
            )?;
            let defaults = write_f32_leaf(
                transcript,
                preview,
                work,
                TAG_SPARSE_DEFAULT,
                "sparse_default",
                default,
                render,
            )?;
            require_sparse_index_count(indices, explicit_element_count)?;
            totals.add_sparse_indices(indices)?;
            values
                .checked_add(defaults)
                .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow {
                    total: "observed f32 attribute scalars",
                })?
        }
        NodeSceneGeometryAttributePayload::SparseU32 {
            values,
            indices,
            default,
        } if scalar_kind == NodeSceneGeometryScalarKind::U32
            && payload_kind == NodeSceneGeometryPayloadKind::Sparse =>
        {
            let values = write_u32_leaf(
                transcript, preview, work, TAG_VALUES, "values", values, render,
            )?;
            let indices = write_u32_leaf(
                transcript,
                preview,
                work,
                TAG_SPARSE_INDICES,
                "sparse_indices",
                indices,
                render,
            )?;
            let defaults = write_u32_leaf(
                transcript,
                preview,
                work,
                TAG_SPARSE_DEFAULT,
                "sparse_default",
                default,
                render,
            )?;
            require_sparse_index_count(indices, explicit_element_count)?;
            totals.add_sparse_indices(indices)?;
            values
                .checked_add(defaults)
                .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow {
                    total: "observed u32 attribute scalars",
                })?
        }
        _ => {
            return Err(NodeSceneSnapshotError::InconsistentView {
                reason: "attribute payload variant disagrees with its semantic descriptor",
            });
        }
    };
    let observed_scalars = u64::try_from(observed_scalars).map_err(|_| {
        NodeSceneSnapshotError::SemanticTotalOverflow {
            total: "observed attribute scalars",
        }
    })?;
    if observed_scalars != canonical_scalar_count {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "attribute payload length disagrees with canonical scalar count",
        });
    }
    Ok(())
}

fn require_sparse_index_count(
    observed: usize,
    expected: u32,
) -> Result<(), NodeSceneSnapshotError> {
    if u64::try_from(observed).ok() != Some(u64::from(expected)) {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "sparse-index payload length disagrees with explicit element count",
        });
    }
    Ok(())
}

fn write_absent_leaf(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    name: &'static str,
    render: bool,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(tag)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?;
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, 0)?;
    preview.begin_geometry_leaf(render, name, false, 0);
    preview.end_geometry_leaf(render);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_f32_leaf(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    name: &'static str,
    mut cursor: NodeSceneGeometryF32Cursor<'_>,
    render: bool,
) -> Result<usize, NodeSceneSnapshotError> {
    let count = cursor.declared_count();
    work.observe()?;
    transcript.write_tag(tag)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_geometry_leaf(render, name, true, count);
    for ordinal in 0..count {
        work.observe()?;
        let value = cursor.next().map_err(map_snapshot_invariant)?.ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "f32 payload leaf ended before declared total",
            },
        )?;
        let bits = value.to_bits();
        transcript.tagged_u32(TAG_FLOAT_BITS, bits)?;
        preview.geometry_f32_bits(render, ordinal, bits)?;
    }
    cursor.finish().map_err(map_snapshot_invariant)?;
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    preview.end_geometry_leaf(render);
    Ok(count)
}

#[allow(clippy::too_many_arguments)]
fn write_u32_leaf(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    name: &'static str,
    mut cursor: NodeSceneGeometryU32Cursor<'_>,
    render: bool,
) -> Result<usize, NodeSceneSnapshotError> {
    let count = cursor.declared_count();
    work.observe()?;
    transcript.write_tag(tag)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_geometry_leaf(render, name, true, count);
    for ordinal in 0..count {
        work.observe()?;
        let value = cursor.next().map_err(map_snapshot_invariant)?.ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "u32 payload leaf ended before declared total",
            },
        )?;
        transcript.tagged_u32(TAG_U32_VALUE, value)?;
        preview.geometry_u32_value(render, ordinal, value)?;
    }
    cursor.finish().map_err(map_snapshot_invariant)?;
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    preview.end_geometry_leaf(render);
    Ok(count)
}

fn checked_add_count(
    total: &mut u64,
    count: usize,
    name: &'static str,
) -> Result<(), NodeSceneSnapshotError> {
    let count = u64::try_from(count)
        .map_err(|_| NodeSceneSnapshotError::SemanticTotalOverflow { total: name })?;
    *total = total
        .checked_add(count)
        .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow { total: name })?;
    Ok(())
}

const fn encode_topology(topology: TopologyKind) -> u8 {
    match topology {
        TopologyKind::Points => TOPOLOGY_POINTS,
        TopologyKind::LineList => TOPOLOGY_LINE_LIST,
        TopologyKind::LineStrip => TOPOLOGY_LINE_STRIP,
        TopologyKind::LineLoop => TOPOLOGY_LINE_LOOP,
        TopologyKind::TriangleList => TOPOLOGY_TRIANGLE_LIST,
        TopologyKind::TriangleStrip => TOPOLOGY_TRIANGLE_STRIP,
        TopologyKind::TriangleFan => TOPOLOGY_TRIANGLE_FAN,
        TopologyKind::Polygons => TOPOLOGY_POLYGONS,
    }
}

const fn topology_name(topology: TopologyKind) -> &'static str {
    match topology {
        TopologyKind::Points => "points",
        TopologyKind::LineList => "line_list",
        TopologyKind::LineStrip => "line_strip",
        TopologyKind::LineLoop => "line_loop",
        TopologyKind::TriangleList => "triangle_list",
        TopologyKind::TriangleStrip => "triangle_strip",
        TopologyKind::TriangleFan => "triangle_fan",
        TopologyKind::Polygons => "polygons",
    }
}

const fn encode_semantic(semantic: NodeSceneGeometryAttributeSemantic) -> (u8, u32) {
    match semantic {
        NodeSceneGeometryAttributeSemantic::Position => (SEMANTIC_POSITION, 0),
        NodeSceneGeometryAttributeSemantic::Normal => (SEMANTIC_NORMAL, 0),
        NodeSceneGeometryAttributeSemantic::Tangent => (SEMANTIC_TANGENT, 0),
        NodeSceneGeometryAttributeSemantic::Texcoord(set) => (SEMANTIC_TEXCOORD, set),
        NodeSceneGeometryAttributeSemantic::Color(set) => (SEMANTIC_COLOR, set),
        NodeSceneGeometryAttributeSemantic::SmoothingGroup => (SEMANTIC_SMOOTHING_GROUP, 0),
    }
}

const fn semantic_name(semantic: NodeSceneGeometryAttributeSemantic) -> &'static str {
    match semantic {
        NodeSceneGeometryAttributeSemantic::Position => "position",
        NodeSceneGeometryAttributeSemantic::Normal => "normal",
        NodeSceneGeometryAttributeSemantic::Tangent => "tangent",
        NodeSceneGeometryAttributeSemantic::Texcoord(_) => "texcoord",
        NodeSceneGeometryAttributeSemantic::Color(_) => "color",
        NodeSceneGeometryAttributeSemantic::SmoothingGroup => "smoothing_group",
    }
}

fn encode_attribute_domain(domain: AttributeDomain) -> Result<u8, NodeSceneSnapshotError> {
    match domain {
        AttributeDomain::Vertex => Ok(ATTRIBUTE_DOMAIN_VERTEX),
        AttributeDomain::Corner => Ok(ATTRIBUTE_DOMAIN_CORNER),
        AttributeDomain::Face => Ok(ATTRIBUTE_DOMAIN_FACE),
        AttributeDomain::Instance => Err(NodeSceneSnapshotError::InconsistentView {
            reason: "instance-domain attribute escaped geometry validation",
        }),
    }
}

const fn attribute_domain_name(domain: AttributeDomain) -> &'static str {
    match domain {
        AttributeDomain::Vertex => "vertex",
        AttributeDomain::Corner => "corner",
        AttributeDomain::Face => "face",
        AttributeDomain::Instance => "instance",
    }
}

const fn encode_scalar_kind(kind: NodeSceneGeometryScalarKind) -> u8 {
    match kind {
        NodeSceneGeometryScalarKind::F32 => SCALAR_F32,
        NodeSceneGeometryScalarKind::U32 => SCALAR_U32,
    }
}

const fn scalar_kind_name(kind: NodeSceneGeometryScalarKind) -> &'static str {
    match kind {
        NodeSceneGeometryScalarKind::F32 => "f32",
        NodeSceneGeometryScalarKind::U32 => "u32",
    }
}

const fn encode_normalization(normalization: NodeSceneGeometryNormalization) -> u8 {
    match normalization {
        NodeSceneGeometryNormalization::None => NORMALIZATION_NONE,
    }
}

const fn encode_payload_kind(kind: NodeSceneGeometryPayloadKind) -> u8 {
    match kind {
        NodeSceneGeometryPayloadKind::Dense => PAYLOAD_DENSE,
        NodeSceneGeometryPayloadKind::Sparse => PAYLOAD_SPARSE,
    }
}

const fn payload_kind_name(kind: NodeSceneGeometryPayloadKind) -> &'static str {
    match kind {
        NodeSceneGeometryPayloadKind::Dense => "dense",
        NodeSceneGeometryPayloadKind::Sparse => "sparse",
    }
}

fn map_snapshot_invariant(error: NodeSceneSemanticError) -> NodeSceneSnapshotError {
    NodeSceneSnapshotError::InconsistentView {
        reason: error.reason(),
    }
}

fn validate_usize_limit(
    limit: NodeSceneSnapshotLimit,
    value: usize,
    maximum: usize,
) -> Result<(), NodeSceneSnapshotError> {
    if value > maximum {
        return Err(NodeSceneSnapshotError::InvalidLimit {
            limit,
            value: u64::try_from(value).unwrap_or(u64::MAX),
            maximum: u64::try_from(maximum).unwrap_or(u64::MAX),
        });
    }
    Ok(())
}

fn write_transform(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    transform: AuthoredTransform,
    render_node: bool,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_TRANSFORM)?;
    match transform.kind() {
        AuthoredTransformKind::Matrix => {
            transcript.write_u8(TRANSFORM_MATRIX)?;
            let matrix =
                transform
                    .authored_matrix()
                    .ok_or(NodeSceneSnapshotError::InconsistentView {
                        reason: "matrix transform lost its authored representation",
                    })?;
            preview.begin_transform(render_node, "matrix", 16);
            for (ordinal, value) in matrix.cols.into_iter().flatten().enumerate() {
                work.observe()?;
                let bits = value.to_bits();
                transcript.tagged_u32(TAG_FLOAT_BITS, bits)?;
                preview.transform_bits(render_node, ordinal, bits)?;
            }
            transcript.tagged_usize(TAG_COLLECTION_TOTAL, 16)?;
            preview.end_transform(render_node, 16);
        }
        AuthoredTransformKind::Trs => {
            transcript.write_u8(TRANSFORM_TRS)?;
            let trs = transform
                .authored_trs()
                .ok_or(NodeSceneSnapshotError::InconsistentView {
                    reason: "TRS transform lost its authored representation",
                })?;
            let translation = trs.translation();
            let rotation = trs.rotation();
            let scale = trs.scale();
            preview.begin_transform(render_node, "trs", 10);
            for (ordinal, value) in [
                translation.x,
                translation.y,
                translation.z,
                rotation.x,
                rotation.y,
                rotation.z,
                rotation.w,
                scale.x,
                scale.y,
                scale.z,
            ]
            .into_iter()
            .enumerate()
            {
                work.observe()?;
                let bits = value.to_bits();
                transcript.tagged_u32(TAG_FLOAT_BITS, bits)?;
                preview.transform_bits(render_node, ordinal, bits)?;
            }
            transcript.tagged_usize(TAG_COLLECTION_TOTAL, 10)?;
            preview.end_transform(render_node, 10);
        }
    }
    Ok(())
}

struct SnapshotWorkMeter<'a> {
    limit: u64,
    observed: u64,
    since_poll: u64,
    cancellation: Option<&'a CancellationToken>,
}

impl<'a> SnapshotWorkMeter<'a> {
    const fn new(limit: u64, cancellation: Option<&'a CancellationToken>) -> Self {
        Self {
            limit,
            observed: 0,
            since_poll: 0,
            cancellation,
        }
    }

    const fn observed(&self) -> u64 {
        self.observed
    }

    fn observe(&mut self) -> Result<(), NodeSceneSnapshotError> {
        self.observed =
            self.observed
                .checked_add(1)
                .ok_or(NodeSceneSnapshotError::WorkBudgetExceeded {
                    observed: u64::MAX,
                    limit: self.limit,
                })?;
        if self.observed > self.limit {
            return Err(NodeSceneSnapshotError::WorkBudgetExceeded {
                observed: self.observed,
                limit: self.limit,
            });
        }
        self.since_poll =
            self.since_poll
                .checked_add(1)
                .ok_or(NodeSceneSnapshotError::WorkBudgetExceeded {
                    observed: self.observed,
                    limit: self.limit,
                })?;
        if self.since_poll == NODE_SCENE_SNAPSHOT_POLL_INTERVAL {
            self.poll()?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), NodeSceneSnapshotError> {
        self.poll()
    }

    fn poll(&mut self) -> Result<(), NodeSceneSnapshotError> {
        if self
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(NodeSceneSnapshotError::Cancelled);
        }
        self.since_poll = 0;
        Ok(())
    }
}

struct CanonicalDigestEncoder {
    digest: Sha256,
    bytes: u64,
    #[cfg(test)]
    transcript: Option<TestTranscriptRecorder>,
}

struct CanonicalDigestOutput {
    digest: [u8; 32],
    bytes: u64,
    #[cfg(test)]
    transcript: Option<TestCanonicalTranscript>,
}

#[cfg(test)]
struct TestTranscriptRecorder {
    bytes: Vec<u8>,
    complete: bool,
}

#[cfg(test)]
impl TestTranscriptRecorder {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            complete: true,
        }
    }

    fn observe(&mut self, bytes: &[u8]) {
        let remaining = TEST_TRANSCRIPT_MAX_BYTES.saturating_sub(self.bytes.len());
        let captured = remaining.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..captured]);
        self.complete &= captured == bytes.len();
    }

    fn finish(self) -> TestCanonicalTranscript {
        TestCanonicalTranscript {
            bytes: self.bytes,
            complete: self.complete,
        }
    }
}

impl CanonicalDigestEncoder {
    fn new() -> Self {
        Self {
            digest: Sha256::new(),
            bytes: 0,
            #[cfg(test)]
            transcript: None,
        }
    }

    #[cfg(test)]
    fn with_test_transcript() -> Self {
        Self {
            digest: Sha256::new(),
            bytes: 0,
            transcript: Some(TestTranscriptRecorder::new()),
        }
    }

    fn finish(self) -> CanonicalDigestOutput {
        CanonicalDigestOutput {
            digest: self.digest.finalize().into(),
            bytes: self.bytes,
            #[cfg(test)]
            transcript: self.transcript.map(TestTranscriptRecorder::finish),
        }
    }

    fn tagged_u8(&mut self, tag: u8, value: u8) -> Result<(), NodeSceneSnapshotError> {
        self.write_tag(tag)?;
        self.write_u8(value)
    }

    fn tagged_u32(&mut self, tag: u8, value: u32) -> Result<(), NodeSceneSnapshotError> {
        self.write_tag(tag)?;
        self.write_bytes(&value.to_le_bytes())
    }

    fn tagged_u64(&mut self, tag: u8, value: u64) -> Result<(), NodeSceneSnapshotError> {
        self.write_tag(tag)?;
        self.write_bytes(&value.to_le_bytes())
    }

    fn tagged_usize(&mut self, tag: u8, value: usize) -> Result<(), NodeSceneSnapshotError> {
        let value =
            u64::try_from(value).map_err(|_| NodeSceneSnapshotError::TranscriptLengthOverflow)?;
        self.write_tag(tag)?;
        self.write_bytes(&value.to_le_bytes())
    }

    fn tagged_bytes(&mut self, tag: u8, value: &[u8]) -> Result<(), NodeSceneSnapshotError> {
        let length = u64::try_from(value.len())
            .map_err(|_| NodeSceneSnapshotError::TranscriptLengthOverflow)?;
        self.write_tag(tag)?;
        self.write_bytes(&length.to_le_bytes())?;
        self.write_bytes(value)
    }

    fn write_tag(&mut self, tag: u8) -> Result<(), NodeSceneSnapshotError> {
        self.write_u8(tag)
    }

    fn write_u8(&mut self, value: u8) -> Result<(), NodeSceneSnapshotError> {
        self.write_bytes(&[value])
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), NodeSceneSnapshotError> {
        let length = u64::try_from(bytes.len())
            .map_err(|_| NodeSceneSnapshotError::TranscriptLengthOverflow)?;
        self.bytes = self
            .bytes
            .checked_add(length)
            .ok_or(NodeSceneSnapshotError::TranscriptLengthOverflow)?;
        self.digest.update(bytes);
        #[cfg(test)]
        if let Some(transcript) = &mut self.transcript {
            transcript.observe(bytes);
        }
        Ok(())
    }
}

struct PreviewSink {
    text: String,
    byte_limit: usize,
    item_limit: usize,
    items_omitted_by_limit: u64,
    bytes_truncated: bool,
    items_truncated: bool,
}

impl PreviewSink {
    fn new(byte_limit: usize, item_limit: usize) -> Result<Self, NodeSceneSnapshotError> {
        let text = if byte_limit == 0 {
            String::new()
        } else {
            // `byte_limit` is the exact visible-output cap. CapacityPlan separately
            // admits the allocator envelope, which remains below the hard preview
            // cap because that cap is a power of two.
            let plan = CapacityPlan::for_type::<u8>(0, 0, byte_limit)
                .map_err(|source| NodeSceneSnapshotError::PreviewCapacity { source })?;
            let mut text = String::new();
            text.try_reserve_exact(plan.allocator_additional())
                .map_err(|_| NodeSceneSnapshotError::PreviewAllocation)?;
            plan.finish(text.len(), text.capacity())
                .map_err(|source| NodeSceneSnapshotError::PreviewCapacity { source })?;
            text
        };
        Ok(Self {
            text,
            byte_limit,
            item_limit,
            items_omitted_by_limit: 0,
            bytes_truncated: false,
            items_truncated: false,
        })
    }

    fn finish(self) -> NodeSceneSnapshotPreview {
        NodeSceneSnapshotPreview {
            text: self.text,
            items_omitted_by_limit: self.items_omitted_by_limit,
            bytes_truncated: self.bytes_truncated,
            items_truncated: self.items_truncated,
        }
    }

    fn header(&mut self, domain: &str, schema_revision: u32, algorithm: &str) {
        self.push_str("snapshot(domain=");
        self.push_str(domain);
        self.push_str(",schema=");
        self.push_u64(u64::from(schema_revision));
        self.push_str(",algorithm=");
        self.push_str(algorithm);
        self.push_str(")\n");
    }

    fn begin_roots(&mut self, total: usize) {
        self.push_str("roots(present,total=");
        self.push_usize(total);
        self.push_str(")=[");
    }

    fn begin_nodes(&mut self, total: usize) {
        self.push_str("nodes(present,total=");
        self.push_usize(total);
        self.push_str(")\n");
    }

    fn begin_geometry(&mut self) {
        self.push_str("geometry(present)\n");
    }

    fn begin_geometry_collection(&mut self, name: &str, total: usize) {
        self.push_str(name);
        self.push_str("(present,total=");
        self.push_usize(total);
        self.push_str(")\n");
    }

    fn begin_geometry_item(
        &mut self,
        kind: &str,
        index: usize,
    ) -> Result<bool, NodeSceneSnapshotError> {
        if index >= self.item_limit {
            self.omit_item()?;
            return Ok(false);
        }
        self.push_str("  ");
        self.push_str(kind);
        self.push_str("[");
        self.push_usize(index);
        self.push_str("]");
        Ok(true)
    }

    fn geometry_field_word(&mut self, render: bool, name: &str, value: &str) {
        if !render {
            return;
        }
        self.push_str(" ");
        self.push_str(name);
        self.push_str("=");
        self.push_str(value);
    }

    fn geometry_field_usize(&mut self, render: bool, name: &str, value: usize) {
        if !render {
            return;
        }
        self.push_str(" ");
        self.push_str(name);
        self.push_str("=");
        self.push_usize(value);
    }

    fn geometry_field_u32(&mut self, render: bool, name: &str, value: u32) {
        self.geometry_field_u64(render, name, u64::from(value));
    }

    fn geometry_field_u64(&mut self, render: bool, name: &str, value: u64) {
        if !render {
            return;
        }
        self.push_str(" ");
        self.push_str(name);
        self.push_str("=");
        self.push_u64(value);
    }

    fn geometry_optional_reference(&mut self, render: bool, name: &str, value: Option<usize>) {
        if !render {
            return;
        }
        self.push_str(" ");
        self.push_str(name);
        self.push_str("=");
        match value {
            Some(value) => self.push_usize(value),
            None => self.push_str("absent"),
        }
    }

    fn begin_geometry_leaf(&mut self, render: bool, name: &str, present: bool, total: usize) {
        if !render {
            return;
        }
        self.push_str(" ");
        self.push_str(name);
        self.push_str("(");
        self.push_str(if present { "present" } else { "absent" });
        self.push_str(",total=");
        self.push_usize(total);
        self.push_str(")=[");
    }

    fn geometry_f32_bits(
        &mut self,
        render: bool,
        ordinal: usize,
        bits: u32,
    ) -> Result<(), NodeSceneSnapshotError> {
        if !render {
            return Ok(());
        }
        if ordinal >= self.item_limit {
            return self.omit_item();
        }
        self.delimit(ordinal);
        self.push_hex_u32(bits);
        Ok(())
    }

    fn geometry_u32_value(
        &mut self,
        render: bool,
        ordinal: usize,
        value: u32,
    ) -> Result<(), NodeSceneSnapshotError> {
        if !render {
            return Ok(());
        }
        if ordinal >= self.item_limit {
            return self.omit_item();
        }
        self.delimit(ordinal);
        self.push_u64(u64::from(value));
        Ok(())
    }

    fn end_geometry_leaf(&mut self, render: bool) {
        if render {
            self.push_str("]");
        }
    }

    fn end_geometry_item(&mut self, render: bool) {
        if render {
            self.push_str("\n");
        }
    }

    fn end_geometry(&mut self) {
        self.push_str("geometry_end\n");
    }

    fn begin_node(&mut self, index: usize) -> Result<bool, NodeSceneSnapshotError> {
        if index >= self.item_limit {
            self.omit_item()?;
            return Ok(false);
        }
        self.push_str("node[");
        self.push_usize(index);
        self.push_str("] transform=");
        Ok(true)
    }

    fn begin_transform(&mut self, render: bool, kind: &str, total: usize) {
        if !render {
            return;
        }
        self.push_str(kind);
        self.push_str("(total=");
        self.push_usize(total);
        self.push_str(")=[");
    }

    fn begin_children(&mut self, render: bool) {
        if render {
            self.push_str(" children(present)=[");
        }
    }

    fn reference(
        &mut self,
        ordinal: usize,
        reference: usize,
    ) -> Result<(), NodeSceneSnapshotError> {
        if ordinal >= self.item_limit {
            return self.omit_item();
        }
        self.delimit(ordinal);
        self.push_usize(reference);
        Ok(())
    }

    fn child_reference(
        &mut self,
        render_node: bool,
        ordinal: usize,
        reference: usize,
    ) -> Result<(), NodeSceneSnapshotError> {
        if !render_node {
            return Ok(());
        }
        self.reference(ordinal, reference)
    }

    fn transform_bits(
        &mut self,
        render_node: bool,
        ordinal: usize,
        bits: u32,
    ) -> Result<(), NodeSceneSnapshotError> {
        if !render_node {
            return Ok(());
        }
        if ordinal >= self.item_limit {
            return self.omit_item();
        }
        self.delimit(ordinal);
        self.push_hex_u32(bits);
        Ok(())
    }

    fn end_transform(&mut self, render: bool, _total: usize) {
        if render {
            self.push_str("]");
        }
    }

    fn end_collection(&mut self) {
        self.push_str("]\n");
    }

    fn end_node(&mut self, render: bool, child_total: usize) {
        if !render {
            return;
        }
        self.push_str("];total=");
        self.push_usize(child_total);
        self.push_str("\n");
    }

    fn summary(
        &mut self,
        nodes: usize,
        roots: usize,
        edges: usize,
        counts: GeometryCollectionCounts,
        payloads: GeometryPayloadTotals,
    ) {
        self.push_str("summary(nodes=");
        self.push_usize(nodes);
        self.push_str(",roots=");
        self.push_usize(roots);
        self.push_str(",edges=");
        self.push_usize(edges);
        self.push_str(",materials=");
        self.push_usize(counts.materials);
        self.push_str(",logical_meshes=");
        self.push_usize(counts.logical_meshes);
        self.push_str(",primitives=");
        self.push_usize(counts.primitives);
        self.push_str(",attributes=");
        self.push_usize(counts.attributes);
        self.push_str(",mesh_instances=");
        self.push_usize(counts.mesh_instances);
        self.push_str(",polygon_boundaries=");
        self.push_u64(payloads.polygon_boundaries);
        self.push_str(",primitive_indices=");
        self.push_u64(payloads.primitive_indices);
        self.push_str(",sparse_indices=");
        self.push_u64(payloads.sparse_indices);
        self.push_str(",canonical_scalars=");
        self.push_u64(payloads.canonical_scalars);
        self.push_str(",f32_scalars=");
        self.push_u64(payloads.f32_scalars);
        self.push_str(",u32_scalars=");
        self.push_u64(payloads.u32_scalars);
        self.push_str(")\n");
    }

    fn omit_item(&mut self) -> Result<(), NodeSceneSnapshotError> {
        self.items_truncated = true;
        self.items_omitted_by_limit = self
            .items_omitted_by_limit
            .checked_add(1)
            .ok_or(NodeSceneSnapshotError::PreviewItemCountOverflow)?;
        Ok(())
    }

    fn delimit(&mut self, ordinal: usize) {
        if ordinal != 0 {
            self.push_str(",");
        }
    }

    fn push_usize(&mut self, value: usize) {
        self.push_u64(u64::try_from(value).unwrap_or(u64::MAX));
    }

    fn push_u64(&mut self, mut value: u64) {
        let mut digits = [0_u8; 20];
        let mut cursor = digits.len();
        loop {
            cursor -= 1;
            digits[cursor] = b'0' + u8::try_from(value % 10).unwrap_or(0);
            value /= 10;
            if value == 0 {
                break;
            }
        }
        let text = std::str::from_utf8(&digits[cursor..]).unwrap_or("");
        self.push_str(text);
    }

    fn push_hex_u32(&mut self, value: u32) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut bytes = *b"0x00000000";
        for index in 0..8 {
            let shift = 28 - index * 4;
            bytes[index + 2] = HEX[((value >> shift) & 0x0f) as usize];
        }
        let text = std::str::from_utf8(&bytes).unwrap_or("");
        self.push_str(text);
    }

    fn push_str(&mut self, value: &str) {
        if self.bytes_truncated {
            return;
        }
        let remaining = self.byte_limit.saturating_sub(self.text.len());
        if value.len() > remaining {
            self.bytes_truncated = true;
            return;
        }
        self.text.push_str(value);
    }
}

#[cfg(test)]
mod tests;
