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

#[cfg(test)]
pub(super) fn validate_complex_fixture_graph(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<super::schema::ComplexDirectorySchema>,
    evidence_ref: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
) -> bool {
    validate_complex_fixture_graph_inner(session, directory_ref, evidence_ref).is_some()
}

#[cfg(test)]
pub(super) fn validate_complex_fixture_evidence_graph(
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
            == Some(super::schema::COMPLEX_CANONICAL_BLOCK_COUNT)
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
    for count_field in [
        directory_field::IMAGE_COUNT,
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
    if images.rows() != 1
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

    require_scalar(images.scalar::<u32>(0, image::IMAGE), 0)?;
    let mime_ref = images.utf8_ref::<super::schema::ComplexImageMimeSchema>(0, image::MIME)?;
    let payload_ref =
        images.bytes_ref::<super::schema::ComplexImagePayloadSchema>(0, image::PAYLOAD)?;
    if session.canonical_utf8(mime_ref)? != super::schema::COMPLEX_IMAGE_MIME
        || session.canonical_bytes(payload_ref)? != super::schema::COMPLEX_IMAGE_PAYLOAD
    {
        return None;
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
        super::schema::COMPLEX_CANONICAL_BLOCK_COUNT,
    )?;
    require_scalar(
        evidence.scalar::<u32>(0, evidence_field::NODE_COUNT),
        super::schema::COMPLEX_NODE_COUNT,
    )?;
    Some(())
}
