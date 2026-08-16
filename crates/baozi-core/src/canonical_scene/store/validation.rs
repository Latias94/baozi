mod node_scene;

use baozi_operation::__canonical_activation::{
    CanonicalValidationMember, CanonicalValidationSession, OutcomeEvidenceValidationMember,
};
use baozi_operation::{
    CanonicalRecordRef, OperationOwner, OutcomeEvidenceRecordRef, ProgressPhase, ProgressUnit,
    RecordBlock, StoreEvidenceValidationSession, StoreValidationSession, StoreWorkError,
};

use super::schema::{
    AuthoredMatrixSchema, CANONICAL_BLOCK_COUNT, DIRECTORY_FIELD_COUNT, DirectorySchema,
    EVIDENCE_FIELD_COUNT, EVIDENCE_OUTCOME_CANDIDATE, MATRIX_SCALAR_COUNT, MATRIX_SCALARS_PER_NODE,
    NODE_COUNT, NODE_FIELD_COUNT, NodeTableSchema, OutcomeEvidenceSchema, PRESENCE_ABSENT,
    PRESENCE_PRESENT, REFERENCE_COUNT, REFERENCE_FIELD_COUNT, ReferenceTableSchema,
    SCHEMA_REVISION, directory_field, evidence_field, node_field, reference_field,
};

pub(in crate::canonical_scene::store) use node_scene::{
    AnimationChannelValidationDescriptor, AnimationClipValidationDescriptor,
    AnimationMeteredCounts, AnimationValidationScratch, AnimationValidationScratchRequirements,
    AppearanceBindingValidationScratch, AppearanceCanonicalCounts,
    AppearanceMaterialValidationScratch, AppearanceValidationScratch,
    AppearanceValidationScratchRequirements, DeformationLogicalMorphValidationProjection,
    DeformationMeshInstanceValidationProjection, DeformationMorphDefinitionValidationProjection,
    DeformationMorphDeltaValidationProjection, DeformationMorphTargetValidationProjection,
    DeformationPrimitiveValidationProjection, DeformationSkinValidationProjection,
    DeformationValidationScratch, DeformationValidationScratchRequirements,
    GeometryAttributeValidationScratch, GeometryCanonicalCounts,
    GeometryLogicalMeshValidationProjection, GeometryMeshInstanceValidationProjection,
    GeometryPrimitiveValidationProjection, GeometryPrimitiveValidationScratch,
    GeometryValidationScratch, GeometryValidationScratchRequirements, NodeSceneValidationProof,
    NodeSceneValidationScratch, ProfileMeteredCounts, ProfileMeteredScratch,
    validate_node_scene_canonical_graph, validate_node_scene_canonical_graph_metered,
    validate_node_scene_evidence_graph,
};

pub(super) fn authored_matrix_scalar_is_valid(values: &[f32], index: usize) -> bool {
    let Some(value) = values.get(index).copied() else {
        return false;
    };
    value.is_finite()
        && match index {
            3 | 7 | 11 => value == 0.0,
            15 => value == 1.0,
            _ => true,
        }
}

pub(super) fn authored_matrix_is_valid(values: &[f32]) -> bool {
    values.len() == MATRIX_SCALARS_PER_NODE
        && (0..MATRIX_SCALARS_PER_NODE).fold(true, |valid, index| {
            valid & authored_matrix_scalar_is_valid(values, index)
        })
}

pub(super) fn validate_scene_graph(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<DirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
) -> bool {
    validate_scene_graph_with_work(session, directory_ref, evidence_ref, |_| Ok::<(), ()>(()))
        .expect("the no-op Scene validation work meter cannot fail")
}

pub(super) fn validate_scene_graph_with_work<E>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<DirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
    mut observe: impl FnMut(u64) -> Result<(), E>,
) -> Result<bool, E> {
    validate_scene_graph_inner_with_work(session, directory_ref, evidence_ref, &mut observe)
        .map(|result| result.is_some())
}

pub(super) fn validate_evidence_graph(
    session: &mut StoreEvidenceValidationSession<'_, '_>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
) -> bool {
    let Some(evidence) = session.records(evidence_ref) else {
        return false;
    };
    evidence.rows() == 1
        && evidence.fields() == EVIDENCE_FIELD_COUNT
        && evidence.scalar::<u32>(0, evidence_field::SCHEMA_REVISION) == Some(SCHEMA_REVISION)
        && evidence.scalar::<u32>(0, evidence_field::OUTCOME) == Some(EVIDENCE_OUTCOME_CANDIDATE)
        && evidence.scalar::<u32>(0, evidence_field::CANONICAL_BLOCK_COUNT)
            == Some(CANONICAL_BLOCK_COUNT)
        && evidence.scalar::<u32>(0, evidence_field::NODE_COUNT) == u32::try_from(NODE_COUNT).ok()
}

pub(super) fn scene_graph_validation_work() -> u64 {
    table_work(DIRECTORY_FIELD_COUNT)
        + table_work(NODE_COUNT * NODE_FIELD_COUNT)
        + table_work(REFERENCE_COUNT * REFERENCE_FIELD_COUNT)
        + work_units(MATRIX_SCALAR_COUNT)
        + table_work(EVIDENCE_FIELD_COUNT)
}

pub(super) fn validate_scene_graph_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: CanonicalValidationMember<'session, RecordBlock, DirectorySchema>,
    evidence: OutcomeEvidenceValidationMember<'session, RecordBlock, OutcomeEvidenceSchema>,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut schema_revision = None;
    let mut root_node = None;
    let mut node_count = None;
    let mut nodes = None;
    let mut reference_count = None;
    let mut references = None;
    let mut root_children_presence = None;
    let mut root_children_count = None;
    let mut child_children_presence = None;
    let mut child_children_count = None;
    let mut metadata_presence = None;
    let mut metadata_count = None;
    let mut matrix_scalar_count = None;
    let mut directory_matrix = None;
    let directory_complete = session.visit_canonical_records(
        directory,
        1,
        DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            schema_revision = row.scalar(directory_field::SCHEMA_REVISION, |value: u32| value)?;
            root_node = row.scalar(directory_field::ROOT_NODE, |value: u32| value)?;
            node_count = row.scalar(directory_field::NODE_COUNT, |value: u32| value)?;
            nodes = row.record_ref::<NodeTableSchema, _>(directory_field::NODES, |value| value)?;
            reference_count = row.scalar(directory_field::REFERENCE_COUNT, |value: u32| value)?;
            references = row
                .record_ref::<ReferenceTableSchema, _>(directory_field::REFERENCES, |value| {
                    value
                })?;
            root_children_presence =
                row.scalar(directory_field::ROOT_CHILDREN_PRESENCE, |value: u8| value)?;
            root_children_count =
                row.scalar(directory_field::ROOT_CHILDREN_COUNT, |value: u32| value)?;
            child_children_presence =
                row.scalar(directory_field::CHILD_CHILDREN_PRESENCE, |value: u8| value)?;
            child_children_count =
                row.scalar(directory_field::CHILD_CHILDREN_COUNT, |value: u32| value)?;
            metadata_presence =
                row.scalar(directory_field::METADATA_PRESENCE, |value: u8| value)?;
            metadata_count = row.scalar(directory_field::METADATA_COUNT, |value: u32| value)?;
            matrix_scalar_count =
                row.scalar(directory_field::MATRIX_SCALAR_COUNT, |value: u32| value)?;
            directory_matrix = row.sequence_ref::<f32, AuthoredMatrixSchema, _>(
                directory_field::MATRICES,
                |value| value,
            )?;
            Ok(true)
        },
    )?;
    if !directory_complete
        || require_scalar(schema_revision, SCHEMA_REVISION).is_none()
        || require_scalar(root_node, 0).is_none()
        || require_scalar(node_count, schema_u32(NODE_COUNT)).is_none()
        || require_scalar(reference_count, schema_u32(REFERENCE_COUNT)).is_none()
        || require_optional_collection(
            root_children_presence,
            root_children_count,
            PRESENCE_PRESENT,
            1,
        )
        .is_none()
        || require_optional_collection(
            child_children_presence,
            child_children_count,
            PRESENCE_PRESENT,
            0,
        )
        .is_none()
        || require_optional_collection(metadata_presence, metadata_count, PRESENCE_ABSENT, 0)
            .is_none()
        || require_scalar(matrix_scalar_count, schema_u32(MATRIX_SCALAR_COUNT)).is_none()
    {
        return Ok(false);
    }
    let (Some(nodes), Some(references), Some(directory_matrix)) =
        (nodes, references, directory_matrix)
    else {
        return Ok(false);
    };

    let mut root_id = None;
    let mut root_parent_presence = None;
    let mut root_parent_offset = None;
    let mut root_parent_count = None;
    let mut root_matrix_offset = None;
    let mut root_matrix_length = None;
    let mut root_matrix = None;
    let mut child_id = None;
    let mut child_parent_presence = None;
    let mut child_parent_offset = None;
    let mut child_parent_count = None;
    let mut child_matrix_offset = None;
    let mut child_matrix_length = None;
    let mut child_matrix = None;
    let nodes_complete = session.visit_canonical_records(
        nodes,
        NODE_COUNT,
        NODE_FIELD_COUNT,
        |row_index, row| {
            if row_index == 0 {
                root_id = row.scalar(node_field::NODE, |value: u32| value)?;
                root_parent_presence =
                    row.scalar(node_field::PARENT_PRESENCE, |value: u8| value)?;
                root_parent_offset =
                    row.scalar(node_field::PARENT_REFERENCE_OFFSET, |value: u32| value)?;
                root_parent_count =
                    row.scalar(node_field::PARENT_REFERENCE_COUNT, |value: u32| value)?;
                root_matrix_offset = row.scalar(node_field::MATRIX_OFFSET, |value: u32| value)?;
                root_matrix_length = row.scalar(node_field::MATRIX_LENGTH, |value: u32| value)?;
                root_matrix = row
                    .sequence_ref::<f32, AuthoredMatrixSchema, _>(node_field::MATRIX, |value| {
                        value
                    })?;
            } else {
                child_id = row.scalar(node_field::NODE, |value: u32| value)?;
                child_parent_presence =
                    row.scalar(node_field::PARENT_PRESENCE, |value: u8| value)?;
                child_parent_offset =
                    row.scalar(node_field::PARENT_REFERENCE_OFFSET, |value: u32| value)?;
                child_parent_count =
                    row.scalar(node_field::PARENT_REFERENCE_COUNT, |value: u32| value)?;
                child_matrix_offset = row.scalar(node_field::MATRIX_OFFSET, |value: u32| value)?;
                child_matrix_length = row.scalar(node_field::MATRIX_LENGTH, |value: u32| value)?;
                child_matrix = row
                    .sequence_ref::<f32, AuthoredMatrixSchema, _>(node_field::MATRIX, |value| {
                        value
                    })?;
            }
            Ok(true)
        },
    )?;
    if !nodes_complete
        || require_scalar(root_id, 0).is_none()
        || require_optional_collection(root_parent_presence, root_parent_count, PRESENCE_ABSENT, 0)
            .is_none()
        || require_scalar(root_parent_offset, 0).is_none()
        || require_scalar(root_matrix_offset, 0).is_none()
        || require_scalar(root_matrix_length, schema_u32(MATRIX_SCALARS_PER_NODE)).is_none()
        || require_scalar(child_id, 1).is_none()
        || require_optional_collection(
            child_parent_presence,
            child_parent_count,
            PRESENCE_PRESENT,
            1,
        )
        .is_none()
        || require_scalar(child_parent_offset, 0).is_none()
        || require_scalar(child_matrix_offset, schema_u32(MATRIX_SCALARS_PER_NODE)).is_none()
        || require_scalar(child_matrix_length, schema_u32(MATRIX_SCALARS_PER_NODE)).is_none()
    {
        return Ok(false);
    }
    let (Some(root_matrix), Some(child_matrix)) = (root_matrix, child_matrix) else {
        return Ok(false);
    };
    if root_matrix != child_matrix || root_matrix != directory_matrix {
        return Ok(false);
    }

    let mut reference_id = None;
    let mut reference_parent = None;
    let mut reference_child = None;
    let references_complete = session.visit_canonical_records(
        references,
        REFERENCE_COUNT,
        REFERENCE_FIELD_COUNT,
        |_row_index, row| {
            reference_id = row.scalar(reference_field::REFERENCE, |value: u32| value)?;
            reference_parent = row.scalar(reference_field::PARENT, |value: u32| value)?;
            reference_child = row.scalar(reference_field::CHILD, |value: u32| value)?;
            Ok(true)
        },
    )?;
    if !references_complete
        || require_scalar(reference_id, 0).is_none()
        || require_scalar(reference_parent, 0).is_none()
        || require_scalar(reference_child, 1).is_none()
    {
        return Ok(false);
    }

    let mut matrices = [0.0_f32; MATRIX_SCALAR_COUNT];
    let matrices_complete =
        session.visit_canonical_sequence(root_matrix, MATRIX_SCALAR_COUNT, |index, value| {
            matrices[index] = value;
            true
        })?;
    if !matrices_complete
        || !authored_matrix_is_valid(&matrices[..MATRIX_SCALARS_PER_NODE])
        || !authored_matrix_is_valid(&matrices[MATRIX_SCALARS_PER_NODE..])
    {
        return Ok(false);
    }

    let mut evidence_schema = None;
    let mut evidence_outcome = None;
    let mut evidence_blocks = None;
    let mut evidence_nodes = None;
    let evidence_complete = session.visit_outcome_evidence_records(
        evidence,
        1,
        EVIDENCE_FIELD_COUNT,
        |_row_index, row| {
            evidence_schema = row.scalar(evidence_field::SCHEMA_REVISION, |value: u32| value)?;
            evidence_outcome = row.scalar(evidence_field::OUTCOME, |value: u32| value)?;
            evidence_blocks =
                row.scalar(evidence_field::CANONICAL_BLOCK_COUNT, |value: u32| value)?;
            evidence_nodes = row.scalar(evidence_field::NODE_COUNT, |value: u32| value)?;
            Ok(true)
        },
    )?;
    Ok(evidence_complete
        && require_scalar(evidence_schema, SCHEMA_REVISION).is_some()
        && require_scalar(evidence_outcome, EVIDENCE_OUTCOME_CANDIDATE).is_some()
        && require_scalar(evidence_blocks, CANONICAL_BLOCK_COUNT).is_some()
        && require_scalar(evidence_nodes, schema_u32(NODE_COUNT)).is_some())
}

fn validate_scene_graph_inner_with_work<E>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<DirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
    observe: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<Option<()>, E> {
    let Some(directory) = session.canonical_records(directory_ref) else {
        return Ok(None);
    };
    if directory.rows() != 1 || directory.fields() != DIRECTORY_FIELD_COUNT {
        return Ok(None);
    }
    let schema_revision = observed(
        directory.scalar::<u32>(0, directory_field::SCHEMA_REVISION),
        observe,
    )?;
    let root_node = observed(
        directory.scalar::<u32>(0, directory_field::ROOT_NODE),
        observe,
    )?;
    let node_count = observed(
        directory.scalar::<u32>(0, directory_field::NODE_COUNT),
        observe,
    )?;
    let nodes_ref = observed(
        directory.record_ref::<NodeTableSchema>(0, directory_field::NODES),
        observe,
    )?;
    let reference_count = observed(
        directory.scalar::<u32>(0, directory_field::REFERENCE_COUNT),
        observe,
    )?;
    let references_ref = observed(
        directory.record_ref::<ReferenceTableSchema>(0, directory_field::REFERENCES),
        observe,
    )?;
    let root_children_presence = observed(
        directory.scalar::<u8>(0, directory_field::ROOT_CHILDREN_PRESENCE),
        observe,
    )?;
    let root_children_count = observed(
        directory.scalar::<u32>(0, directory_field::ROOT_CHILDREN_COUNT),
        observe,
    )?;
    let child_children_presence = observed(
        directory.scalar::<u8>(0, directory_field::CHILD_CHILDREN_PRESENCE),
        observe,
    )?;
    let child_children_count = observed(
        directory.scalar::<u32>(0, directory_field::CHILD_CHILDREN_COUNT),
        observe,
    )?;
    let metadata_presence = observed(
        directory.scalar::<u8>(0, directory_field::METADATA_PRESENCE),
        observe,
    )?;
    let metadata_count = observed(
        directory.scalar::<u32>(0, directory_field::METADATA_COUNT),
        observe,
    )?;
    let matrix_scalar_count = observed(
        directory.scalar::<u32>(0, directory_field::MATRIX_SCALAR_COUNT),
        observe,
    )?;
    let directory_matrix = observed(
        directory.sequence_ref::<f32, AuthoredMatrixSchema>(0, directory_field::MATRICES),
        observe,
    )?;
    let directory_valid = require_scalar(schema_revision, SCHEMA_REVISION).is_some()
        && require_scalar(root_node, 0).is_some()
        && require_scalar(node_count, schema_u32(NODE_COUNT)).is_some()
        && require_scalar(reference_count, schema_u32(REFERENCE_COUNT)).is_some()
        && require_optional_collection(
            root_children_presence,
            root_children_count,
            PRESENCE_PRESENT,
            1,
        )
        .is_some()
        && require_optional_collection(
            child_children_presence,
            child_children_count,
            PRESENCE_PRESENT,
            0,
        )
        .is_some()
        && require_optional_collection(metadata_presence, metadata_count, PRESENCE_ABSENT, 0)
            .is_some()
        && require_scalar(matrix_scalar_count, schema_u32(MATRIX_SCALAR_COUNT)).is_some()
        && nodes_ref.is_some()
        && references_ref.is_some()
        && directory_matrix.is_some();
    if !directory_valid {
        return Ok(None);
    }
    let nodes_ref = nodes_ref.expect("a valid directory contains its node table reference");
    let references_ref =
        references_ref.expect("a valid directory contains its reference table reference");
    let directory_matrix =
        directory_matrix.expect("a valid directory contains its authored matrix reference");

    let Some(nodes) = session.canonical_records(nodes_ref) else {
        return Ok(None);
    };
    if nodes.rows() != NODE_COUNT || nodes.fields() != NODE_FIELD_COUNT {
        return Ok(None);
    }
    let root_id = observed(nodes.scalar::<u32>(0, node_field::NODE), observe)?;
    let root_parent_presence =
        observed(nodes.scalar::<u8>(0, node_field::PARENT_PRESENCE), observe)?;
    let root_parent_offset = observed(
        nodes.scalar::<u32>(0, node_field::PARENT_REFERENCE_OFFSET),
        observe,
    )?;
    let root_parent_count = observed(
        nodes.scalar::<u32>(0, node_field::PARENT_REFERENCE_COUNT),
        observe,
    )?;
    let root_matrix_offset = observed(nodes.scalar::<u32>(0, node_field::MATRIX_OFFSET), observe)?;
    let root_matrix_length = observed(nodes.scalar::<u32>(0, node_field::MATRIX_LENGTH), observe)?;
    let root_matrix = observed(
        nodes.sequence_ref::<f32, AuthoredMatrixSchema>(0, node_field::MATRIX),
        observe,
    )?;
    let child_id = observed(nodes.scalar::<u32>(1, node_field::NODE), observe)?;
    let child_parent_presence =
        observed(nodes.scalar::<u8>(1, node_field::PARENT_PRESENCE), observe)?;
    let child_parent_offset = observed(
        nodes.scalar::<u32>(1, node_field::PARENT_REFERENCE_OFFSET),
        observe,
    )?;
    let child_parent_count = observed(
        nodes.scalar::<u32>(1, node_field::PARENT_REFERENCE_COUNT),
        observe,
    )?;
    let child_matrix_offset = observed(nodes.scalar::<u32>(1, node_field::MATRIX_OFFSET), observe)?;
    let child_matrix_length = observed(nodes.scalar::<u32>(1, node_field::MATRIX_LENGTH), observe)?;
    let child_matrix = observed(
        nodes.sequence_ref::<f32, AuthoredMatrixSchema>(1, node_field::MATRIX),
        observe,
    )?;
    let nodes_valid = require_scalar(root_id, 0).is_some()
        && require_optional_collection(root_parent_presence, root_parent_count, PRESENCE_ABSENT, 0)
            .is_some()
        && require_scalar(root_parent_offset, 0).is_some()
        && require_scalar(root_matrix_offset, 0).is_some()
        && require_scalar(root_matrix_length, schema_u32(MATRIX_SCALARS_PER_NODE)).is_some()
        && require_scalar(child_id, 1).is_some()
        && require_optional_collection(
            child_parent_presence,
            child_parent_count,
            PRESENCE_PRESENT,
            1,
        )
        .is_some()
        && require_scalar(child_parent_offset, 0).is_some()
        && require_scalar(child_matrix_offset, schema_u32(MATRIX_SCALARS_PER_NODE)).is_some()
        && require_scalar(child_matrix_length, schema_u32(MATRIX_SCALARS_PER_NODE)).is_some()
        && root_matrix.is_some()
        && child_matrix.is_some();
    if !nodes_valid {
        return Ok(None);
    }
    let root_matrix =
        root_matrix.expect("a valid root node contains its authored matrix reference");
    let child_matrix =
        child_matrix.expect("a valid child node contains its authored matrix reference");
    if root_matrix != child_matrix || root_matrix != directory_matrix {
        return Ok(None);
    }

    let Some(references) = session.canonical_records(references_ref) else {
        return Ok(None);
    };
    if references.rows() != REFERENCE_COUNT || references.fields() != REFERENCE_FIELD_COUNT {
        return Ok(None);
    }
    let reference_id = observed(
        references.scalar::<u32>(0, reference_field::REFERENCE),
        observe,
    )?;
    let reference_parent = observed(
        references.scalar::<u32>(0, reference_field::PARENT),
        observe,
    )?;
    let reference_child = observed(references.scalar::<u32>(0, reference_field::CHILD), observe)?;
    let references_valid = require_scalar(reference_id, 0).is_some()
        && require_scalar(reference_parent, 0).is_some()
        && require_scalar(reference_child, 1).is_some();
    if !references_valid {
        return Ok(None);
    }

    let Some(matrices) = session
        .canonical_sequence(root_matrix)
        .map(|value| value.as_slice())
    else {
        return Ok(None);
    };
    if matrices.len() != MATRIX_SCALAR_COUNT {
        return Ok(None);
    }
    if !authored_matrices_are_valid_complete(matrices, observe)? {
        return Ok(None);
    }

    let Some(evidence) = session.outcome_evidence_records(evidence_ref) else {
        return Ok(None);
    };
    if evidence.rows() != 1 || evidence.fields() != EVIDENCE_FIELD_COUNT {
        return Ok(None);
    }
    let evidence_schema = observed(
        evidence.scalar::<u32>(0, evidence_field::SCHEMA_REVISION),
        observe,
    )?;
    let evidence_outcome = observed(evidence.scalar::<u32>(0, evidence_field::OUTCOME), observe)?;
    let evidence_blocks = observed(
        evidence.scalar::<u32>(0, evidence_field::CANONICAL_BLOCK_COUNT),
        observe,
    )?;
    let evidence_nodes = observed(
        evidence.scalar::<u32>(0, evidence_field::NODE_COUNT),
        observe,
    )?;
    let evidence_valid = require_scalar(evidence_schema, SCHEMA_REVISION).is_some()
        && require_scalar(evidence_outcome, EVIDENCE_OUTCOME_CANDIDATE).is_some()
        && require_scalar(evidence_blocks, CANONICAL_BLOCK_COUNT).is_some()
        && require_scalar(evidence_nodes, schema_u32(NODE_COUNT)).is_some();
    Ok(evidence_valid.then_some(()))
}

fn observed<T, E>(value: T, observe: &mut impl FnMut(u64) -> Result<(), E>) -> Result<T, E> {
    observe(1)?;
    Ok(value)
}

fn authored_matrices_are_valid_complete<E>(
    values: &[f32],
    observe: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<bool, E> {
    if values.len() != MATRIX_SCALAR_COUNT {
        return Ok(false);
    }
    let mut finite = true;
    for value in values {
        finite = value.is_finite() && finite;
        observe(1)?;
    }
    let root = &values[..MATRIX_SCALARS_PER_NODE];
    let child = &values[MATRIX_SCALARS_PER_NODE..];
    Ok(finite
        && root[3] == 0.0
        && root[7] == 0.0
        && root[11] == 0.0
        && root[15] == 1.0
        && child[3] == 0.0
        && child[7] == 0.0
        && child[11] == 0.0
        && child[15] == 1.0)
}

fn work_units(value: usize) -> u64 {
    u64::try_from(value).expect("the canonical Scene validation shape fits in u64")
}

fn table_work(cells: usize) -> u64 {
    work_units(cells)
}

fn schema_u32(value: usize) -> u32 {
    u32::try_from(value).expect("the canonical Scene schema shape fits in u32")
}

fn require_scalar<T: Copy + Eq>(actual: Option<T>, expected: T) -> Option<()> {
    (actual == Some(expected)).then_some(())
}

fn require_optional_collection(
    presence: Option<u8>,
    count: Option<u32>,
    expected_presence: u8,
    expected_count: u32,
) -> Option<()> {
    require_scalar(presence, expected_presence)?;
    require_scalar(count, expected_count)
}

#[cfg(test)]
pub(super) fn validate_complex_fixture_graph(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<super::schema::ComplexDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
    expected_image_count: usize,
    expected_canonical_block_count: u32,
) -> bool {
    validate_complex_fixture_graph_inner(
        session,
        directory_ref,
        evidence_ref,
        expected_image_count,
        expected_canonical_block_count,
    )
    .is_some()
}

#[cfg(test)]
pub(super) fn validate_complex_fixture_evidence_graph(
    session: &mut StoreEvidenceValidationSession<'_, '_>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
    expected_canonical_block_count: u32,
) -> bool {
    let Some(evidence) = session.records(evidence_ref) else {
        return false;
    };
    evidence.rows() == 1
        && evidence.fields() == EVIDENCE_FIELD_COUNT
        && evidence.scalar::<u32>(0, evidence_field::SCHEMA_REVISION) == Some(SCHEMA_REVISION)
        && evidence.scalar::<u32>(0, evidence_field::OUTCOME) == Some(EVIDENCE_OUTCOME_CANDIDATE)
        && evidence.scalar::<u32>(0, evidence_field::CANONICAL_BLOCK_COUNT)
            == Some(expected_canonical_block_count)
        && evidence.scalar::<u32>(0, evidence_field::NODE_COUNT)
            == Some(super::schema::COMPLEX_NODE_COUNT)
}

#[cfg(test)]
fn require_f32(actual: Option<f32>, expected: f32) -> Option<()> {
    (actual.map(f32::to_bits) == Some(expected.to_bits())).then_some(())
}

#[cfg(test)]
fn require_f64(actual: Option<f64>, expected: f64) -> Option<()> {
    (actual.map(f64::to_bits) == Some(expected.to_bits())).then_some(())
}

#[cfg(test)]
fn validate_complex_fixture_graph_inner(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<super::schema::ComplexDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
    expected_image_count: usize,
    expected_canonical_block_count: u32,
) -> Option<()> {
    use super::schema::{
        complex_animation_field as animation, complex_directory_field as directory_field,
        complex_image_field as image, complex_influence_payload_field as influence,
        complex_instance_override_field as instance_override, complex_joint_field as joint,
        complex_logical_mesh_field as logical_mesh, complex_logical_morph_field as logical_morph,
        complex_material_binding_field as binding, complex_material_field as material,
        complex_mesh_instance_field as mesh_instance,
        complex_metadata_descriptor_field as metadata_descriptor, complex_node_field as node,
        complex_primitive_field as primitive, complex_provenance_arena_field as provenance_arena,
        complex_sampler_field as sampler, complex_skin_field as skin,
        complex_source_attachment_field as source_attachment, complex_texture_field as texture,
    };

    let directory = session.canonical_records(directory_ref)?;
    if directory.rows() != 1 || directory.fields() != super::schema::COMPLEX_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, directory_field::SCHEMA_REVISION),
        super::schema::COMPLEX_SCHEMA_REVISION,
    )?;
    let expected_image_count_u32 = u32::try_from(expected_image_count).ok()?;
    require_scalar(
        directory.scalar::<u32>(0, directory_field::IMAGE_COUNT),
        expected_image_count_u32,
    )?;
    for count_field in [
        directory_field::SAMPLER_COUNT,
        directory_field::TEXTURE_COUNT,
        directory_field::MATERIAL_BINDING_COUNT,
        directory_field::MATERIAL_COUNT,
        directory_field::PRIMITIVE_COUNT,
        directory_field::LOGICAL_MESH_COUNT,
        directory_field::NODE_COUNT,
        directory_field::MESH_INSTANCE_COUNT,
        directory_field::SKIN_COUNT,
        directory_field::JOINT_COUNT,
        directory_field::LOGICAL_MORPH_COUNT,
        directory_field::INSTANCE_OVERRIDE_COUNT,
        directory_field::ANIMATION_COUNT,
        directory_field::SOURCE_ATTACHMENT_COUNT,
    ] {
        require_scalar(directory.scalar::<u32>(0, count_field), 1)?;
    }

    let images_ref = directory
        .record_ref::<super::schema::ComplexImageTableSchema>(0, directory_field::IMAGES)?;
    let samplers_ref = directory
        .record_ref::<super::schema::ComplexSamplerTableSchema>(0, directory_field::SAMPLERS)?;
    let textures_ref = directory
        .record_ref::<super::schema::ComplexTextureTableSchema>(0, directory_field::TEXTURES)?;
    let bindings_ref = directory.record_ref::<super::schema::ComplexMaterialBindingTableSchema>(
        0,
        directory_field::MATERIAL_BINDINGS,
    )?;
    let materials_ref = directory
        .record_ref::<super::schema::ComplexMaterialTableSchema>(0, directory_field::MATERIALS)?;
    let primitives_ref = directory
        .record_ref::<super::schema::ComplexPrimitiveTableSchema>(0, directory_field::PRIMITIVES)?;
    let logical_meshes_ref = directory.record_ref::<super::schema::ComplexLogicalMeshTableSchema>(
        0,
        directory_field::LOGICAL_MESHES,
    )?;
    let nodes_ref =
        directory.record_ref::<super::schema::ComplexNodeTableSchema>(0, directory_field::NODES)?;
    let mesh_instances_ref = directory
        .record_ref::<super::schema::ComplexMeshInstanceTableSchema>(
            0,
            directory_field::MESH_INSTANCES,
        )?;
    let skins_ref =
        directory.record_ref::<super::schema::ComplexSkinTableSchema>(0, directory_field::SKINS)?;
    let joints_ref = directory
        .record_ref::<super::schema::ComplexJointTableSchema>(0, directory_field::JOINTS)?;
    let logical_morphs_ref = directory
        .record_ref::<super::schema::ComplexLogicalMorphTableSchema>(
            0,
            directory_field::LOGICAL_MORPHS,
        )?;
    let overrides_ref = directory.record_ref::<super::schema::ComplexInstanceOverrideTableSchema>(
        0,
        directory_field::INSTANCE_OVERRIDES,
    )?;
    let animations_ref = directory
        .record_ref::<super::schema::ComplexAnimationTableSchema>(0, directory_field::ANIMATIONS)?;
    let source_attachments_ref = directory
        .record_ref::<super::schema::ComplexSourceAttachmentSchema>(
            0,
            directory_field::SOURCE_ATTACHMENTS,
        )?;

    let images = session.canonical_records(images_ref)?;
    let samplers = session.canonical_records(samplers_ref)?;
    let textures = session.canonical_records(textures_ref)?;
    let bindings = session.canonical_records(bindings_ref)?;
    let materials = session.canonical_records(materials_ref)?;
    let primitives = session.canonical_records(primitives_ref)?;
    let logical_meshes = session.canonical_records(logical_meshes_ref)?;
    let nodes = session.canonical_records(nodes_ref)?;
    let mesh_instances = session.canonical_records(mesh_instances_ref)?;
    let skins = session.canonical_records(skins_ref)?;
    let joints = session.canonical_records(joints_ref)?;
    let logical_morphs = session.canonical_records(logical_morphs_ref)?;
    let overrides = session.canonical_records(overrides_ref)?;
    let animations = session.canonical_records(animations_ref)?;
    let source_attachments = session.canonical_records(source_attachments_ref)?;
    if images.rows() != expected_image_count
        || images.fields() != super::schema::COMPLEX_IMAGE_FIELD_COUNT
        || samplers.rows() != 1
        || samplers.fields() != super::schema::COMPLEX_SAMPLER_FIELD_COUNT
        || textures.rows() != 1
        || textures.fields() != super::schema::COMPLEX_TEXTURE_FIELD_COUNT
        || bindings.rows() != 1
        || bindings.fields() != super::schema::COMPLEX_MATERIAL_BINDING_FIELD_COUNT
        || materials.rows() != 1
        || materials.fields() != super::schema::COMPLEX_MATERIAL_FIELD_COUNT
        || primitives.rows() != 1
        || primitives.fields() != super::schema::COMPLEX_PRIMITIVE_FIELD_COUNT
        || logical_meshes.rows() != 1
        || logical_meshes.fields() != super::schema::COMPLEX_LOGICAL_MESH_FIELD_COUNT
        || nodes.rows() != 1
        || nodes.fields() != super::schema::COMPLEX_NODE_FIELD_COUNT
        || mesh_instances.rows() != 1
        || mesh_instances.fields() != super::schema::COMPLEX_MESH_INSTANCE_FIELD_COUNT
        || skins.rows() != 1
        || skins.fields() != super::schema::COMPLEX_SKIN_FIELD_COUNT
        || joints.rows() != 1
        || joints.fields() != super::schema::COMPLEX_JOINT_FIELD_COUNT
        || logical_morphs.rows() != 1
        || logical_morphs.fields() != super::schema::COMPLEX_LOGICAL_MORPH_FIELD_COUNT
        || overrides.rows() != 1
        || overrides.fields() != super::schema::COMPLEX_INSTANCE_OVERRIDE_FIELD_COUNT
        || animations.rows() != 1
        || animations.fields() != super::schema::COMPLEX_ANIMATION_FIELD_COUNT
        || source_attachments.rows() != 1
        || source_attachments.fields() != super::schema::COMPLEX_SOURCE_ATTACHMENT_FIELD_COUNT
    {
        return None;
    }

    for row in 0..expected_image_count {
        let expected_image = u32::try_from(row).ok()?;
        require_scalar(images.scalar::<u32>(row, image::IMAGE), expected_image)?;
        let mime_ref =
            images.utf8_ref::<super::schema::ComplexImageMimeSchema>(row, image::MIME)?;
        let payload_ref =
            images.bytes_ref::<super::schema::ComplexImagePayloadSchema>(row, image::PAYLOAD)?;
        if session.canonical_utf8(mime_ref)? != super::schema::COMPLEX_IMAGE_MIME
            || session.canonical_bytes(payload_ref)? != super::schema::COMPLEX_IMAGE_PAYLOAD
        {
            return None;
        }
    }
    require_scalar(samplers.scalar::<u32>(0, sampler::SAMPLER), 0)?;
    require_scalar(
        samplers.scalar::<u32>(0, sampler::MAG_FILTER),
        super::schema::COMPLEX_SAMPLER_MAG_FILTER,
    )?;
    require_scalar(
        samplers.scalar::<u32>(0, sampler::MIN_FILTER),
        super::schema::COMPLEX_SAMPLER_MIN_FILTER,
    )?;

    require_scalar(textures.scalar::<u32>(0, texture::TEXTURE), 0)?;
    require_scalar(textures.scalar::<u32>(0, texture::IMAGE_INDEX), 0)?;
    require_scalar(textures.scalar::<u32>(0, texture::SAMPLER_INDEX), 0)?;
    if textures.record_ref::<super::schema::ComplexImageTableSchema>(0, texture::IMAGES)?
        != images_ref
        || textures.record_ref::<super::schema::ComplexSamplerTableSchema>(0, texture::SAMPLERS)?
            != samplers_ref
    {
        return None;
    }
    require_scalar(bindings.scalar::<u32>(0, binding::MATERIAL_BINDING), 0)?;
    require_scalar(bindings.scalar::<u32>(0, binding::TEXTURE_INDEX), 0)?;
    if bindings.record_ref::<super::schema::ComplexTextureTableSchema>(0, binding::TEXTURES)?
        != textures_ref
    {
        return None;
    }
    require_scalar(materials.scalar::<u32>(0, material::MATERIAL), 0)?;
    require_scalar(
        materials.scalar::<u32>(0, material::MATERIAL_BINDING_INDEX),
        0,
    )?;
    if materials.record_ref::<super::schema::ComplexMaterialBindingTableSchema>(
        0,
        material::MATERIAL_BINDINGS,
    )? != bindings_ref
    {
        return None;
    }

    require_scalar(joints.scalar::<u32>(0, joint::JOINT), 0)?;
    let inverse_binds_ref = skins
        .sequence_ref::<f32, super::schema::ComplexInverseBindMatricesSchema>(
            0,
            skin::INVERSE_BINDS,
        )?;
    let influences_ref =
        skins.record_ref::<super::schema::ComplexInfluencePayloadSchema>(0, skin::INFLUENCES)?;
    let influences = session.canonical_records(influences_ref)?;
    if influences.rows() != super::schema::COMPLEX_INFLUENCE_ROW_COUNT
        || influences.fields() != super::schema::COMPLEX_INFLUENCE_PAYLOAD_FIELD_COUNT
    {
        return None;
    }
    for (row, weight) in super::schema::COMPLEX_INFLUENCE_WEIGHTS
        .iter()
        .copied()
        .enumerate()
    {
        require_scalar(influences.scalar::<u32>(row, influence::VERTEX), row as u32)?;
        require_scalar(influences.scalar::<u32>(row, influence::JOINT_INDEX), 0)?;
        if influences
            .record_ref::<super::schema::ComplexJointTableSchema>(row, influence::JOINTS)?
            != joints_ref
        {
            return None;
        }
        require_f32(influences.scalar::<f32>(row, influence::WEIGHT), weight)?;
    }
    require_scalar(skins.scalar::<u32>(0, skin::SKIN), 0)?;
    require_scalar(skins.scalar::<u32>(0, skin::JOINT_INDEX), 0)?;
    require_scalar(
        skins.scalar::<u32>(0, skin::INVERSE_BIND_SCALAR_COUNT),
        super::schema::COMPLEX_INVERSE_BIND_SCALAR_COUNT as u32,
    )?;
    require_scalar(
        skins.scalar::<u32>(0, skin::INFLUENCE_ROW_COUNT),
        super::schema::COMPLEX_INFLUENCE_ROW_COUNT as u32,
    )?;
    if skins.record_ref::<super::schema::ComplexJointTableSchema>(0, skin::JOINTS)? != joints_ref
        || session.canonical_sequence(inverse_binds_ref)?.as_slice()
            != super::schema::COMPLEX_INVERSE_BIND_MATRICES.as_slice()
        || skins.record_ref::<super::schema::ComplexInfluencePayloadSchema>(0, skin::INFLUENCES)?
            != influences_ref
    {
        return None;
    }

    require_scalar(primitives.scalar::<u32>(0, primitive::PRIMITIVE), 0)?;
    require_scalar(primitives.scalar::<u32>(0, primitive::MATERIAL_INDEX), 0)?;
    require_scalar(primitives.scalar::<u32>(0, primitive::SKIN_INDEX), 0)?;
    require_scalar(
        primitives.scalar::<u32>(0, primitive::INFLUENCE_ROW_COUNT),
        super::schema::COMPLEX_INFLUENCE_ROW_COUNT as u32,
    )?;
    if primitives
        .record_ref::<super::schema::ComplexMaterialTableSchema>(0, primitive::MATERIALS)?
        != materials_ref
        || primitives.record_ref::<super::schema::ComplexSkinTableSchema>(0, primitive::SKINS)?
            != skins_ref
        || primitives
            .record_ref::<super::schema::ComplexInfluencePayloadSchema>(0, primitive::INFLUENCES)?
            != influences_ref
    {
        return None;
    }
    require_scalar(
        logical_meshes.scalar::<u32>(0, logical_mesh::LOGICAL_MESH),
        0,
    )?;
    require_scalar(
        logical_meshes.scalar::<u32>(0, logical_mesh::PRIMITIVE_INDEX),
        0,
    )?;
    if logical_meshes
        .record_ref::<super::schema::ComplexPrimitiveTableSchema>(0, logical_mesh::PRIMITIVES)?
        != primitives_ref
    {
        return None;
    }

    require_scalar(nodes.scalar::<u32>(0, node::NODE), 0)?;
    require_scalar(nodes.scalar::<u32>(0, node::MESH_INSTANCE_INDEX), 0)?;
    if nodes.record_ref::<super::schema::ComplexMeshInstanceTableSchema>(0, node::MESH_INSTANCES)?
        != mesh_instances_ref
    {
        return None;
    }
    require_scalar(
        mesh_instances.scalar::<u32>(0, mesh_instance::MESH_INSTANCE),
        0,
    )?;
    require_scalar(
        mesh_instances.scalar::<u32>(0, mesh_instance::LOGICAL_MESH_INDEX),
        0,
    )?;
    require_scalar(
        mesh_instances.scalar::<u32>(0, mesh_instance::SKIN_INDEX),
        0,
    )?;
    require_scalar(
        mesh_instances.scalar::<u32>(0, mesh_instance::INSTANCE_OVERRIDE_INDEX),
        0,
    )?;
    if mesh_instances.record_ref::<super::schema::ComplexLogicalMeshTableSchema>(
        0,
        mesh_instance::LOGICAL_MESHES,
    )? != logical_meshes_ref
        || mesh_instances
            .record_ref::<super::schema::ComplexSkinTableSchema>(0, mesh_instance::SKINS)?
            != skins_ref
        || mesh_instances.record_ref::<super::schema::ComplexInstanceOverrideTableSchema>(
            0,
            mesh_instance::INSTANCE_OVERRIDES,
        )? != overrides_ref
    {
        return None;
    }

    require_scalar(
        logical_morphs.scalar::<u32>(0, logical_morph::LOGICAL_MORPH),
        0,
    )?;
    require_scalar(
        logical_morphs.scalar::<u32>(0, logical_morph::TARGET_COUNT),
        1,
    )?;
    require_f64(
        logical_morphs.scalar::<f64>(0, logical_morph::DEFAULT_WEIGHT),
        super::schema::COMPLEX_LOGICAL_MORPH_WEIGHT,
    )?;
    require_scalar(
        overrides.scalar::<u32>(0, instance_override::INSTANCE_OVERRIDE),
        0,
    )?;
    require_scalar(
        overrides.scalar::<u32>(0, instance_override::LOGICAL_MORPH_INDEX),
        0,
    )?;
    require_f64(
        overrides.scalar::<f64>(0, instance_override::WEIGHT),
        super::schema::COMPLEX_INSTANCE_OVERRIDE_WEIGHT,
    )?;
    if overrides.record_ref::<super::schema::ComplexLogicalMorphTableSchema>(
        0,
        instance_override::LOGICAL_MORPHS,
    )? != logical_morphs_ref
    {
        return None;
    }
    require_scalar(animations.scalar::<u32>(0, animation::ANIMATION), 0)?;
    require_scalar(
        animations.scalar::<u32>(0, animation::INSTANCE_OVERRIDE_INDEX),
        0,
    )?;
    require_scalar(
        animations.scalar::<u32>(0, animation::KEYFRAME_COUNT),
        super::schema::COMPLEX_ANIMATION_TIME_COUNT as u32,
    )?;
    require_scalar(
        animations.scalar::<u32>(0, animation::MORPH_WEIGHT_VALUE_WIDTH),
        super::schema::COMPLEX_MORPH_WEIGHT_VALUE_WIDTH as u32,
    )?;
    let times_ref = animations
        .sequence_ref::<f64, super::schema::ComplexAnimationTimesSchema>(0, animation::TIMES)?;
    let values_ref = animations
        .sequence_ref::<f64, super::schema::ComplexMorphWeightValuesSchema>(
            0,
            animation::MORPH_WEIGHT_VALUES,
        )?;
    if animations.record_ref::<super::schema::ComplexInstanceOverrideTableSchema>(
        0,
        animation::INSTANCE_OVERRIDES,
    )? != overrides_ref
        || session.canonical_sequence(times_ref)?.as_slice()
            != super::schema::COMPLEX_ANIMATION_TIMES.as_slice()
        || session.canonical_sequence(values_ref)?.as_slice()
            != super::schema::COMPLEX_MORPH_WEIGHT_VALUES.as_slice()
    {
        return None;
    }

    require_scalar(
        source_attachments.scalar::<u32>(0, source_attachment::SOURCE_ATTACHMENT),
        0,
    )?;
    require_scalar(
        source_attachments.scalar::<u32>(0, source_attachment::METADATA_DESCRIPTOR_INDEX),
        0,
    )?;
    require_scalar(
        source_attachments.scalar::<u32>(0, source_attachment::PROVENANCE_ARENA_INDEX),
        0,
    )?;
    let metadata_descriptors_ref = source_attachments
        .record_ref::<super::schema::ComplexMetadataDescriptorSchema>(
        0,
        source_attachment::METADATA_DESCRIPTORS,
    )?;
    let provenance_arenas_ref = source_attachments
        .record_ref::<super::schema::ComplexProvenanceArenaSchema>(
            0,
            source_attachment::PROVENANCE_ARENAS,
        )?;
    let metadata_descriptors = session.canonical_records(metadata_descriptors_ref)?;
    let provenance_arenas = session.canonical_records(provenance_arenas_ref)?;
    if metadata_descriptors.rows() != 1
        || metadata_descriptors.fields() != super::schema::COMPLEX_METADATA_DESCRIPTOR_FIELD_COUNT
        || provenance_arenas.rows() != 1
        || provenance_arenas.fields() != super::schema::COMPLEX_PROVENANCE_ARENA_FIELD_COUNT
    {
        return None;
    }
    require_scalar(
        metadata_descriptors.scalar::<u32>(0, metadata_descriptor::METADATA_DESCRIPTOR),
        0,
    )?;
    let metadata_ref = metadata_descriptors
        .utf8_ref::<super::schema::ComplexMetadataSchema>(0, metadata_descriptor::CONTENT)?;
    require_scalar(
        provenance_arenas.scalar::<u32>(0, provenance_arena::ARENA_REFERENCE),
        0,
    )?;
    let provenance_ref = provenance_arenas
        .bytes_ref::<super::schema::ComplexProvenanceSchema>(0, provenance_arena::CONTENT)?;
    if session.canonical_utf8(metadata_ref)? != super::schema::COMPLEX_METADATA
        || session.canonical_bytes(provenance_ref)? != super::schema::COMPLEX_PROVENANCE
    {
        return None;
    }

    let evidence = session.outcome_evidence_records(evidence_ref)?;
    if evidence.rows() != 1 || evidence.fields() != EVIDENCE_FIELD_COUNT {
        return None;
    }
    require_scalar(
        evidence.scalar::<u32>(0, evidence_field::SCHEMA_REVISION),
        SCHEMA_REVISION,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, evidence_field::OUTCOME),
        EVIDENCE_OUTCOME_CANDIDATE,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, evidence_field::CANONICAL_BLOCK_COUNT),
        expected_canonical_block_count,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, evidence_field::NODE_COUNT),
        super::schema::COMPLEX_NODE_COUNT,
    )?;
    Some(())
}
