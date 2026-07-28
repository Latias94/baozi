use baozi_operation::{
    CanonicalRecordRef, OutcomeEvidenceRecordRef, StoreEvidenceValidationSession,
    StoreValidationSession,
};

mod geometry;

use super::super::construction::node_scene::SceneEvidenceCounts;
use super::super::node_scene::{authored_transform_scalar_count, decode_authored_transform};
use super::super::production::{Presence, ValidatedGeometryBatch, ValidatedNodeBatch};
use super::super::schema::geometry::{
    GeometryAttributeTableSchema, GeometryLogicalMeshTableSchema, GeometryMaterialTableSchema,
    GeometryMeshInstanceTableSchema, GeometryPrimitiveTableSchema,
};
use super::super::schema::{
    CanonicalBlockRef, NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE, NODE_SCENE_NODE_FIELD_COUNT,
    NODE_SCENE_TRANSFORM_SYNTHETIC, NodeSceneAuthoredTransformSchema,
    NodeSceneChildReferenceTableSchema, NodeSceneNodeTableSchema, NodeSceneOutcomeEvidenceSchema,
    PRESENCE_ABSENT, PRESENCE_PRESENT, SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION,
    SceneDirectorySchema, SceneGeometryDirectorySchema, node_scene_child_reference_field,
    node_scene_evidence_field, node_scene_node_field, scene_directory_field,
};
use super::{require_optional_collection, require_scalar};
use geometry::validate_geometry_graph;

/// Immutable semantic authority minted only by central node-scene validation.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneValidationProof {
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    references: Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>>,
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
    materials: Option<CanonicalBlockRef<GeometryMaterialTableSchema>>,
    logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    attributes: Option<CanonicalBlockRef<GeometryAttributeTableSchema>>,
    mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
    source_node_count: usize,
    source_root_count: usize,
    source_edge_count: usize,
    material_count: usize,
    logical_mesh_count: usize,
    primitive_count: usize,
    attribute_count: usize,
    mesh_instance_count: usize,
}

impl NodeSceneValidationProof {
    pub(in crate::canonical_scene::store) const fn nodes(
        self,
    ) -> CanonicalBlockRef<NodeSceneNodeTableSchema> {
        self.nodes
    }

    pub(in crate::canonical_scene::store) const fn references(
        self,
    ) -> Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>> {
        self.references
    }

    pub(in crate::canonical_scene::store) const fn transforms(
        self,
    ) -> Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>> {
        self.transforms
    }

    pub(in crate::canonical_scene::store) const fn materials(
        self,
    ) -> Option<CanonicalBlockRef<GeometryMaterialTableSchema>> {
        self.materials
    }

    pub(in crate::canonical_scene::store) const fn logical_meshes(
        self,
    ) -> Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>> {
        self.logical_meshes
    }

    pub(in crate::canonical_scene::store) const fn primitives(
        self,
    ) -> Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>> {
        self.primitives
    }

    pub(in crate::canonical_scene::store) const fn attributes(
        self,
    ) -> Option<CanonicalBlockRef<GeometryAttributeTableSchema>> {
        self.attributes
    }

    pub(in crate::canonical_scene::store) const fn mesh_instances(
        self,
    ) -> Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>> {
        self.mesh_instances
    }

    pub(in crate::canonical_scene::store) const fn source_node_count(self) -> usize {
        self.source_node_count
    }

    pub(in crate::canonical_scene::store) const fn source_root_count(self) -> usize {
        self.source_root_count
    }

    pub(in crate::canonical_scene::store) const fn source_edge_count(self) -> usize {
        self.source_edge_count
    }

    pub(in crate::canonical_scene::store) const fn material_count(self) -> usize {
        self.material_count
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh_count(self) -> usize {
        self.logical_mesh_count
    }

    pub(in crate::canonical_scene::store) const fn primitive_count(self) -> usize {
        self.primitive_count
    }

    pub(in crate::canonical_scene::store) const fn attribute_count(self) -> usize {
        self.attribute_count
    }

    pub(in crate::canonical_scene::store) const fn mesh_instance_count(self) -> usize {
        self.mesh_instance_count
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_layout_fault_for_test(
        &mut self,
        source_node_count: usize,
        source_root_count: usize,
    ) -> bool {
        let Some(source_edge_count) = source_node_count.checked_sub(source_root_count) else {
            return false;
        };
        self.source_node_count = source_node_count;
        self.source_root_count = source_root_count;
        self.source_edge_count = source_edge_count;
        true
    }
}

pub(in crate::canonical_scene::store) fn validate_node_scene_graph<R, E>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    expected_nodes: &ValidatedNodeBatch<'_, R>,
    expected_geometry: &ValidatedGeometryBatch<'_, impl Sized>,
    mut observe_work: impl FnMut(u64) -> Result<(), E>,
) -> Result<Option<NodeSceneValidationProof>, E> {
    let mut work_failure = None;
    let proof = validate_node_scene_graph_inner(
        session,
        directory_ref,
        evidence_ref,
        expected_nodes,
        expected_geometry,
        &mut |amount| match observe_work(amount) {
            Ok(()) => Some(()),
            Err(cause) => {
                work_failure = Some(cause);
                None
            }
        },
    );
    match work_failure {
        Some(cause) => Err(cause),
        None => Ok(proof),
    }
}

pub(in crate::canonical_scene::store) fn validate_node_scene_evidence_graph(
    session: &mut StoreEvidenceValidationSession<'_, '_>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    counts: SceneEvidenceCounts,
) -> bool {
    let Some(evidence) = session.records(evidence_ref) else {
        return false;
    };
    evidence.rows() == 1
        && evidence.fields() == NODE_SCENE_EVIDENCE_FIELD_COUNT
        && evidence.scalar::<u32>(0, node_scene_evidence_field::SCHEMA_REVISION)
            == Some(SCENE_SCHEMA_REVISION)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::OUTCOME)
            == Some(NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::CANONICAL_BLOCK_COUNT)
            == Some(counts.canonical_block_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::NODE_COUNT)
            == Some(counts.node_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::MATERIAL_COUNT)
            == Some(counts.material_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::LOGICAL_MESH_COUNT)
            == Some(counts.logical_mesh_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PRIMITIVE_COUNT)
            == Some(counts.primitive_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::ATTRIBUTE_COUNT)
            == Some(counts.attribute_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::MESH_INSTANCE_COUNT)
            == Some(counts.mesh_instance_count)
        && evidence.scalar::<u32>(0, node_scene_evidence_field::PAYLOAD_BLOCK_COUNT)
            == Some(counts.payload_block_count)
}

fn validate_node_scene_graph_inner<NR, GR>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SceneDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    expected: &ValidatedNodeBatch<'_, NR>,
    expected_geometry: &ValidatedGeometryBatch<'_, GR>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<NodeSceneValidationProof> {
    observe_work(1)?;
    let directory = session.canonical_records(directory_ref)?;
    if directory.rows() != 1 || directory.fields() != SCENE_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, scene_directory_field::SCHEMA_REVISION),
        SCENE_SCHEMA_REVISION,
    )?;
    require_scalar(
        directory.scalar::<u32>(0, scene_directory_field::ROOT_NODE),
        0,
    )?;

    let node_count_u32 = directory.scalar::<u32>(0, scene_directory_field::NODE_COUNT)?;
    require_scalar(
        Some(node_count_u32),
        u32::try_from(expected.canonical_node_count()).ok()?,
    )?;
    let node_count = usize::try_from(node_count_u32).ok()?;
    let source_node_count = node_count.checked_sub(1)?;
    let input = expected.input();
    let input_nodes = input.nodes();
    if source_node_count != input_nodes.len()
        || source_node_count != expected.canonical_order().len()
        || source_node_count != expected.canonical_index_by_local().len()
    {
        return None;
    }
    let source_node_count_u32 = u32::try_from(source_node_count).ok()?;
    require_scalar(
        Some(source_node_count_u32),
        u32::try_from(expected.reference_count()).ok()?,
    )?;
    let expected_transform_scalar_count = expected
        .matrix_scalar_count()
        .checked_add(expected.trs_scalar_count())?;
    let expected_transform_scalar_count_u32 =
        u32::try_from(expected_transform_scalar_count).ok()?;
    let has_source_nodes = source_node_count != 0;
    let expected_presence = if has_source_nodes {
        PRESENCE_PRESENT
    } else {
        PRESENCE_ABSENT
    };
    require_optional_collection(
        directory.scalar::<u8>(0, scene_directory_field::REFERENCE_PRESENCE),
        directory.scalar::<u32>(0, scene_directory_field::REFERENCE_COUNT),
        expected_presence,
        source_node_count_u32,
    )?;
    require_scalar(
        directory.scalar::<u8>(0, scene_directory_field::TRANSFORM_PRESENCE),
        expected_presence,
    )?;

    let nodes_ref =
        directory.record_ref::<NodeSceneNodeTableSchema>(0, scene_directory_field::NODES)?;
    let nodes = session.canonical_records(nodes_ref)?;
    if nodes.rows() != node_count || nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT {
        return None;
    }

    let (references_ref, references, transforms_ref, transforms) = if has_source_nodes {
        let references_ref = directory.record_ref::<NodeSceneChildReferenceTableSchema>(
            0,
            scene_directory_field::REFERENCES,
        )?;
        let references = session.canonical_records(references_ref)?;
        if references.rows() != source_node_count
            || references.fields() != NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT
        {
            return None;
        }
        let transforms_ref = directory.sequence_ref::<f32, NodeSceneAuthoredTransformSchema>(
            0,
            scene_directory_field::TRANSFORMS,
        )?;
        let transforms = session.canonical_sequence(transforms_ref)?.as_slice();
        require_scalar(
            directory.scalar::<u32>(0, scene_directory_field::TRANSFORM_SCALAR_COUNT),
            expected_transform_scalar_count_u32,
        )?;
        if transforms.len() != usize::try_from(expected_transform_scalar_count).ok()? {
            return None;
        }
        (
            Some(references_ref),
            Some(references),
            Some(transforms_ref),
            Some(transforms),
        )
    } else {
        require_scalar(
            directory.scalar::<u8>(0, scene_directory_field::REFERENCES),
            0,
        )?;
        require_scalar(
            directory.scalar::<u32>(0, scene_directory_field::TRANSFORM_SCALAR_COUNT),
            0,
        )?;
        require_scalar(Some(expected_transform_scalar_count_u32), 0)?;
        require_scalar(
            directory.scalar::<u8>(0, scene_directory_field::TRANSFORMS),
            0,
        )?;
        (None, None, None, None)
    };

    let source_root_count = match input.roots() {
        Presence::Absent => return None,
        Presence::Present(roots) => roots.len(),
    };
    let expected_root_count = u32::try_from(source_root_count).ok()?;

    observe_work(1)?;
    require_scalar(nodes.scalar::<u32>(0, node_scene_node_field::NODE), 0)?;
    require_optional_collection(
        nodes.scalar::<u8>(0, node_scene_node_field::CHILDREN_PRESENCE),
        nodes.scalar::<u32>(0, node_scene_node_field::CHILD_REFERENCE_COUNT),
        PRESENCE_PRESENT,
        expected_root_count,
    )?;
    require_scalar(
        nodes.scalar::<u32>(0, node_scene_node_field::CHILD_REFERENCE_OFFSET),
        0,
    )?;
    require_scalar(
        nodes.scalar::<u8>(0, node_scene_node_field::TRANSFORM_KIND),
        NODE_SCENE_TRANSFORM_SYNTHETIC,
    )?;
    require_scalar(
        nodes.scalar::<u32>(0, node_scene_node_field::TRANSFORM_OFFSET),
        0,
    )?;
    require_scalar(
        nodes.scalar::<u32>(0, node_scene_node_field::TRANSFORM_LENGTH),
        0,
    )?;
    require_scalar(nodes.scalar::<u8>(0, node_scene_node_field::TRANSFORMS), 0)?;

    if has_source_nodes {
        let references = references.as_ref()?;
        for source_row in 0..source_node_count {
            observe_work(1)?;
            let reference = u32::try_from(source_row).ok()?;
            require_scalar(
                references.scalar::<u32>(source_row, node_scene_child_reference_field::REFERENCE),
                reference,
            )?;
            require_scalar(
                references.scalar::<u32>(source_row, node_scene_child_reference_field::CHILD),
                reference.checked_add(1)?,
            )?;
        }
    }

    let mut transform_offset = 0_usize;
    let mut child_reference_offset = expected_root_count;
    for row in 1..node_count {
        observe_work(1)?;
        let source_row = row - 1;
        let local = *expected.canonical_order().get(source_row)?;
        let expected_node = *input_nodes.get(local)?;
        let expected_child_count = expected.child_count_by_local(local)?;
        let row_u32 = u32::try_from(row).ok()?;
        require_scalar(
            expected.canonical_index_by_local().get(local).copied(),
            row_u32,
        )?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::NODE),
            row_u32,
        )?;
        require_optional_collection(
            nodes.scalar::<u8>(row, node_scene_node_field::CHILDREN_PRESENCE),
            nodes.scalar::<u32>(row, node_scene_node_field::CHILD_REFERENCE_COUNT),
            PRESENCE_PRESENT,
            expected_child_count,
        )?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::CHILD_REFERENCE_OFFSET),
            child_reference_offset,
        )?;
        if expected_child_count != 0 && child_reference_offset.checked_add(1)? <= row_u32 {
            return None;
        }
        child_reference_offset = child_reference_offset.checked_add(expected_child_count)?;

        let kind = nodes.scalar::<u8>(row, node_scene_node_field::TRANSFORM_KIND)?;
        let transform_len = authored_transform_scalar_count(kind)?;
        observe_work(u64::try_from(transform_len).ok()?)?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::TRANSFORM_OFFSET),
            u32::try_from(transform_offset).ok()?,
        )?;
        require_scalar(
            nodes.scalar::<u32>(row, node_scene_node_field::TRANSFORM_LENGTH),
            u32::try_from(transform_len).ok()?,
        )?;
        let row_transforms = nodes.sequence_ref::<f32, NodeSceneAuthoredTransformSchema>(
            row,
            node_scene_node_field::TRANSFORMS,
        )?;
        if Some(row_transforms) != transforms_ref {
            return None;
        }
        let next_transform_offset = transform_offset.checked_add(transform_len)?;
        let values = transforms
            .as_ref()?
            .get(transform_offset..next_transform_offset)?;
        let stored_transform = decode_authored_transform(kind, values)?;
        if !stored_transform.has_same_authored_bits(expected_node.authored_transform()) {
            return None;
        }
        transform_offset = next_transform_offset;
    }

    require_scalar(Some(child_reference_offset), source_node_count_u32)?;
    require_scalar(
        directory.scalar::<u32>(0, scene_directory_field::SOURCE_ROOT_COUNT),
        expected_root_count,
    )?;
    if has_source_nodes && transform_offset != transforms?.len() {
        return None;
    }

    let geometry_ref =
        directory.record_ref::<SceneGeometryDirectorySchema>(0, scene_directory_field::GEOMETRY)?;
    let geometry_refs = validate_geometry_graph(
        session,
        geometry_ref,
        nodes_ref,
        expected,
        expected_geometry,
        observe_work,
    )?;

    let expected_canonical_blocks =
        expected_scene_canonical_block_count(has_source_nodes, expected_geometry)?;
    observe_work(1)?;
    let evidence = session.outcome_evidence_records(evidence_ref)?;
    if evidence.rows() != 1 || evidence.fields() != NODE_SCENE_EVIDENCE_FIELD_COUNT {
        return None;
    }
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::SCHEMA_REVISION),
        SCENE_SCHEMA_REVISION,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::OUTCOME),
        NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::CANONICAL_BLOCK_COUNT),
        expected_canonical_blocks,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::NODE_COUNT),
        node_count_u32,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::MATERIAL_COUNT),
        expected_geometry.material_count(),
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::LOGICAL_MESH_COUNT),
        expected_geometry.logical_mesh_count(),
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::PRIMITIVE_COUNT),
        expected_geometry.primitive_count(),
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::ATTRIBUTE_COUNT),
        expected_geometry.attribute_count(),
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::MESH_INSTANCE_COUNT),
        expected_geometry.mesh_instance_count(),
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, node_scene_evidence_field::PAYLOAD_BLOCK_COUNT),
        expected_geometry.payload_block_count(),
    )?;
    let source_edge_count = source_node_count.checked_sub(source_root_count)?;
    Some(NodeSceneValidationProof {
        nodes: CanonicalBlockRef::from_store(nodes_ref),
        references: references_ref.map(CanonicalBlockRef::from_store),
        transforms: transforms_ref.map(CanonicalBlockRef::from_store),
        materials: geometry_refs.materials,
        logical_meshes: geometry_refs.logical_meshes,
        primitives: geometry_refs.primitives,
        attributes: geometry_refs.attributes,
        mesh_instances: geometry_refs.mesh_instances,
        source_node_count,
        source_root_count,
        source_edge_count,
        material_count: usize::try_from(expected_geometry.material_count()).ok()?,
        logical_mesh_count: usize::try_from(expected_geometry.logical_mesh_count()).ok()?,
        primitive_count: usize::try_from(expected_geometry.primitive_count()).ok()?,
        attribute_count: usize::try_from(expected_geometry.attribute_count()).ok()?,
        mesh_instance_count: usize::try_from(expected_geometry.mesh_instance_count()).ok()?,
    })
}

fn expected_scene_canonical_block_count<R>(
    has_source_nodes: bool,
    geometry: &ValidatedGeometryBatch<'_, R>,
) -> Option<u32> {
    let node_optional_blocks = if has_source_nodes { 2_u32 } else { 0 };
    let geometry_table_blocks = [
        geometry.material_count(),
        geometry.logical_mesh_count(),
        geometry.primitive_count(),
        geometry.attribute_count(),
        geometry.mesh_instance_count(),
    ]
    .into_iter()
    .try_fold(0_u32, |count, rows| count.checked_add(u32::from(rows != 0)))?;
    3_u32
        .checked_add(node_optional_blocks)?
        .checked_add(geometry_table_blocks)?
        .checked_add(geometry.payload_block_count())
}
