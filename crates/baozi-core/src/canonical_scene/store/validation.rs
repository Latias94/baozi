use baozi_operation::{
    CanonicalRecordRef, OutcomeEvidenceRecordRef, StoreEvidenceValidationSession,
    StoreValidationSession,
};

use super::schema::{
    AuthoredMatrixSchema, CANONICAL_BLOCK_COUNT, DIRECTORY_FIELD_COUNT, DirectorySchema,
    EVIDENCE_FIELD_COUNT, EVIDENCE_OUTCOME_CANDIDATE, MATRIX_SCALAR_COUNT, MATRIX_SCALARS_PER_NODE,
    NODE_COUNT, NODE_FIELD_COUNT, NodeTableSchema, OutcomeEvidenceSchema, PRESENCE_ABSENT,
    PRESENCE_PRESENT, REFERENCE_COUNT, REFERENCE_FIELD_COUNT, ReferenceTableSchema,
    SCHEMA_REVISION, directory_field, evidence_field, node_field, reference_field,
};

pub(super) fn authored_matrix_is_valid(values: &[f32]) -> bool {
    values.len() == MATRIX_SCALARS_PER_NODE
        && values.iter().all(|value| value.is_finite())
        && values[3] == 0.0
        && values[7] == 0.0
        && values[11] == 0.0
        && values[15] == 1.0
}

pub(super) fn validate_scene_graph(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<DirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
) -> bool {
    validate_scene_graph_inner(session, directory_ref, evidence_ref).is_some()
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

fn validate_scene_graph_inner(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<DirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
) -> Option<()> {
    let directory = session.canonical_records(directory_ref)?;
    if directory.rows() != 1 || directory.fields() != DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, directory_field::SCHEMA_REVISION),
        SCHEMA_REVISION,
    )?;
    require_scalar(directory.scalar::<u32>(0, directory_field::ROOT_NODE), 0)?;
    require_scalar(
        directory.scalar::<u32>(0, directory_field::NODE_COUNT),
        u32::try_from(NODE_COUNT).ok()?,
    )?;
    require_scalar(
        directory.scalar::<u32>(0, directory_field::REFERENCE_COUNT),
        u32::try_from(REFERENCE_COUNT).ok()?,
    )?;
    require_scalar(
        directory.scalar::<u32>(0, directory_field::MATRIX_SCALAR_COUNT),
        u32::try_from(MATRIX_SCALAR_COUNT).ok()?,
    )?;

    require_optional_collection(
        directory.scalar::<u8>(0, directory_field::ROOT_CHILDREN_PRESENCE),
        directory.scalar::<u32>(0, directory_field::ROOT_CHILDREN_COUNT),
        PRESENCE_PRESENT,
        1,
    )?;
    require_optional_collection(
        directory.scalar::<u8>(0, directory_field::CHILD_CHILDREN_PRESENCE),
        directory.scalar::<u32>(0, directory_field::CHILD_CHILDREN_COUNT),
        PRESENCE_PRESENT,
        0,
    )?;
    require_optional_collection(
        directory.scalar::<u8>(0, directory_field::METADATA_PRESENCE),
        directory.scalar::<u32>(0, directory_field::METADATA_COUNT),
        PRESENCE_ABSENT,
        0,
    )?;

    let nodes_ref = directory.record_ref::<NodeTableSchema>(0, directory_field::NODES)?;
    let references_ref =
        directory.record_ref::<ReferenceTableSchema>(0, directory_field::REFERENCES)?;
    let directory_matrix =
        directory.sequence_ref::<f32, AuthoredMatrixSchema>(0, directory_field::MATRICES)?;
    let nodes = session.canonical_records(nodes_ref)?;
    let references = session.canonical_records(references_ref)?;
    if nodes.rows() != NODE_COUNT || nodes.fields() != NODE_FIELD_COUNT {
        return None;
    }
    if references.rows() != REFERENCE_COUNT || references.fields() != REFERENCE_FIELD_COUNT {
        return None;
    }

    require_scalar(nodes.scalar::<u32>(0, node_field::NODE), 0)?;
    require_optional_collection(
        nodes.scalar::<u8>(0, node_field::PARENT_PRESENCE),
        nodes.scalar::<u32>(0, node_field::PARENT_REFERENCE_COUNT),
        PRESENCE_ABSENT,
        0,
    )?;
    require_scalar(
        nodes.scalar::<u32>(0, node_field::PARENT_REFERENCE_OFFSET),
        0,
    )?;
    require_scalar(nodes.scalar::<u32>(0, node_field::MATRIX_OFFSET), 0)?;
    require_scalar(
        nodes.scalar::<u32>(0, node_field::MATRIX_LENGTH),
        u32::try_from(MATRIX_SCALARS_PER_NODE).ok()?,
    )?;

    require_scalar(nodes.scalar::<u32>(1, node_field::NODE), 1)?;
    require_optional_collection(
        nodes.scalar::<u8>(1, node_field::PARENT_PRESENCE),
        nodes.scalar::<u32>(1, node_field::PARENT_REFERENCE_COUNT),
        PRESENCE_PRESENT,
        1,
    )?;
    require_scalar(
        nodes.scalar::<u32>(1, node_field::PARENT_REFERENCE_OFFSET),
        0,
    )?;
    require_scalar(
        nodes.scalar::<u32>(1, node_field::MATRIX_OFFSET),
        u32::try_from(MATRIX_SCALARS_PER_NODE).ok()?,
    )?;
    require_scalar(
        nodes.scalar::<u32>(1, node_field::MATRIX_LENGTH),
        u32::try_from(MATRIX_SCALARS_PER_NODE).ok()?,
    )?;

    let root_matrix = nodes.sequence_ref::<f32, AuthoredMatrixSchema>(0, node_field::MATRIX)?;
    let child_matrix = nodes.sequence_ref::<f32, AuthoredMatrixSchema>(1, node_field::MATRIX)?;
    if root_matrix != child_matrix || root_matrix != directory_matrix {
        return None;
    }
    let matrices = session.canonical_sequence(root_matrix)?.as_slice();
    if matrices.len() != MATRIX_SCALAR_COUNT
        || !authored_matrix_is_valid(&matrices[..MATRIX_SCALARS_PER_NODE])
        || !authored_matrix_is_valid(&matrices[MATRIX_SCALARS_PER_NODE..])
    {
        return None;
    }

    require_scalar(references.scalar::<u32>(0, reference_field::REFERENCE), 0)?;
    require_scalar(references.scalar::<u32>(0, reference_field::PARENT), 0)?;
    require_scalar(references.scalar::<u32>(0, reference_field::CHILD), 1)?;

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
        CANONICAL_BLOCK_COUNT,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, evidence_field::NODE_COUNT),
        u32::try_from(NODE_COUNT).ok()?,
    )?;
    Some(())
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
