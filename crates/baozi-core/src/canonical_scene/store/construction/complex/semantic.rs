use std::fmt::Write as _;

use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, PublishedCanonicalStore, RecordBlock,
};

use super::super::super::schema::*;
use super::super::super::schema::{
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

pub(in crate::canonical_scene::store) const COMPLEX_SEMANTIC_TRANSCRIPT_SCHEMA_ID: &str =
    "baozi.complex-fixture.semantic-transcript.v1";

fn transcript_record(
    transcript: &mut String,
    name: &str,
    view: CanonicalRecordView<'_>,
    rows: usize,
    fields: usize,
) -> Option<()> {
    if view.rows() != rows || view.fields() != fields {
        return None;
    }
    writeln!(transcript, "record:{name}:rows={rows}:fields={fields}").ok()
}

fn transcript_u32(transcript: &mut String, name: &str, value: Option<u32>) -> Option<()> {
    writeln!(transcript, "u32:{name}={}", value?).ok()
}

fn transcript_f32(transcript: &mut String, name: &str, value: Option<f32>) -> Option<()> {
    writeln!(transcript, "f32-bits:{name}={:08x}", value?.to_bits()).ok()
}

fn transcript_f64(transcript: &mut String, name: &str, value: Option<f64>) -> Option<()> {
    writeln!(transcript, "f64-bits:{name}={:016x}", value?.to_bits()).ok()
}

fn transcript_reference(transcript: &mut String, name: &str, target: &str) -> Option<()> {
    writeln!(transcript, "ref:{name}={target}").ok()
}

fn require_reference<Tag: 'static>(
    actual: Option<CanonicalRecordRef<Tag>>,
    expected: CanonicalRecordRef<Tag>,
) -> Option<()> {
    (actual? == expected).then_some(())
}

fn transcript_bytes(transcript: &mut String, kind: &str, name: &str, bytes: &[u8]) -> Option<()> {
    write!(transcript, "{kind}:{name}:len={}:", bytes.len()).ok()?;
    for byte in bytes {
        write!(transcript, "{byte:02x}").ok()?;
    }
    transcript.push('\n');
    Some(())
}

fn transcript_f32_sequence(transcript: &mut String, name: &str, values: &[f32]) -> Option<()> {
    write!(transcript, "seq-f32-bits:{name}:len={}:", values.len()).ok()?;
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            transcript.push(',');
        }
        write!(transcript, "{:08x}", value.to_bits()).ok()?;
    }
    transcript.push('\n');
    Some(())
}

fn transcript_f64_sequence(transcript: &mut String, name: &str, values: &[f64]) -> Option<()> {
    write!(transcript, "seq-f64-bits:{name}:len={}:", values.len()).ok()?;
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            transcript.push(',');
        }
        write!(transcript, "{:016x}", value.to_bits()).ok()?;
    }
    transcript.push('\n');
    Some(())
}

pub(super) fn inspect_fixed_image_views(
    canonical: &PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    directory_ref: CanonicalBlockRef<ComplexDirectorySchema>,
    expected_image_count: usize,
) -> Option<()> {
    let directory = canonical.records(directory_ref.into_store())?;
    let image_count =
        usize::try_from(directory.scalar::<u32>(0, directory_field::IMAGE_COUNT)?).ok()?;
    if image_count != expected_image_count || image_count == 0 {
        return None;
    }
    let images_ref = directory.record_ref::<ComplexImageTableSchema>(0, directory_field::IMAGES)?;
    let images = canonical.records(images_ref)?;
    if images.rows() != image_count || images.fields() != COMPLEX_IMAGE_FIELD_COUNT {
        return None;
    }
    for row in [0, image_count - 1] {
        if images.scalar::<u32>(row, image::IMAGE) != u32::try_from(row).ok() {
            return None;
        }
        let mime_ref = images.utf8_ref::<ComplexImageMimeSchema>(row, image::MIME)?;
        let payload_ref = images.bytes_ref::<ComplexImagePayloadSchema>(row, image::PAYLOAD)?;
        if canonical.utf8(mime_ref)? != COMPLEX_IMAGE_MIME
            || canonical.bytes(payload_ref)? != COMPLEX_IMAGE_PAYLOAD
        {
            return None;
        }
    }
    Some(())
}

fn image_leaf_name(base: &str, row: usize, image_count: usize) -> String {
    if image_count == 1 {
        base.to_owned()
    } else {
        format!("{base}[{row}]")
    }
}

struct AssetTableRefs {
    images: CanonicalRecordRef<ComplexImageTableSchema>,
    samplers: CanonicalRecordRef<ComplexSamplerTableSchema>,
    textures: CanonicalRecordRef<ComplexTextureTableSchema>,
    bindings: CanonicalRecordRef<ComplexMaterialBindingTableSchema>,
    materials: CanonicalRecordRef<ComplexMaterialTableSchema>,
}

struct InstanceTableRefs {
    nodes: CanonicalRecordRef<ComplexNodeTableSchema>,
    mesh_instances: CanonicalRecordRef<ComplexMeshInstanceTableSchema>,
    logical_meshes: CanonicalRecordRef<ComplexLogicalMeshTableSchema>,
    skins: CanonicalRecordRef<ComplexSkinTableSchema>,
    overrides: CanonicalRecordRef<ComplexInstanceOverrideTableSchema>,
    logical_morphs: CanonicalRecordRef<ComplexLogicalMorphTableSchema>,
}

pub(super) fn semantic_transcript(
    canonical: &PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    directory_ref: CanonicalBlockRef<ComplexDirectorySchema>,
) -> Option<String> {
    let mut transcript = String::new();
    writeln!(transcript, "schema:{COMPLEX_SEMANTIC_TRANSCRIPT_SCHEMA_ID}").ok()?;
    let directory = canonical.records(directory_ref.into_store())?;
    transcript_record(
        &mut transcript,
        "directory",
        directory,
        1,
        COMPLEX_DIRECTORY_FIELD_COUNT,
    )?;
    transcript_u32(
        &mut transcript,
        "directory.schema_revision",
        directory.scalar(0, directory_field::SCHEMA_REVISION),
    )?;

    macro_rules! directory_target {
        ($count_name:literal, $count_field:expr, $ref_name:literal, $schema:ty, $ref_field:expr) => {{
            transcript_u32(
                &mut transcript,
                concat!("directory.", $count_name),
                directory.scalar(0, $count_field),
            )?;
            let reference = directory.record_ref::<$schema>(0, $ref_field)?;
            transcript_reference(&mut transcript, concat!("directory.", $ref_name), $ref_name)?;
            reference
        }};
    }

    let images_ref = directory_target!(
        "image_count",
        directory_field::IMAGE_COUNT,
        "images",
        ComplexImageTableSchema,
        directory_field::IMAGES
    );
    let image_count =
        usize::try_from(directory.scalar::<u32>(0, directory_field::IMAGE_COUNT)?).ok()?;
    let samplers_ref = directory_target!(
        "sampler_count",
        directory_field::SAMPLER_COUNT,
        "samplers",
        ComplexSamplerTableSchema,
        directory_field::SAMPLERS
    );
    let textures_ref = directory_target!(
        "texture_count",
        directory_field::TEXTURE_COUNT,
        "textures",
        ComplexTextureTableSchema,
        directory_field::TEXTURES
    );
    let bindings_ref = directory_target!(
        "material_binding_count",
        directory_field::MATERIAL_BINDING_COUNT,
        "material_bindings",
        ComplexMaterialBindingTableSchema,
        directory_field::MATERIAL_BINDINGS
    );
    let materials_ref = directory_target!(
        "material_count",
        directory_field::MATERIAL_COUNT,
        "materials",
        ComplexMaterialTableSchema,
        directory_field::MATERIALS
    );
    let primitives_ref = directory_target!(
        "primitive_count",
        directory_field::PRIMITIVE_COUNT,
        "primitives",
        ComplexPrimitiveTableSchema,
        directory_field::PRIMITIVES
    );
    let logical_meshes_ref = directory_target!(
        "logical_mesh_count",
        directory_field::LOGICAL_MESH_COUNT,
        "logical_meshes",
        ComplexLogicalMeshTableSchema,
        directory_field::LOGICAL_MESHES
    );
    let nodes_ref = directory_target!(
        "node_count",
        directory_field::NODE_COUNT,
        "nodes",
        ComplexNodeTableSchema,
        directory_field::NODES
    );
    let mesh_instances_ref = directory_target!(
        "mesh_instance_count",
        directory_field::MESH_INSTANCE_COUNT,
        "mesh_instances",
        ComplexMeshInstanceTableSchema,
        directory_field::MESH_INSTANCES
    );
    let skins_ref = directory_target!(
        "skin_count",
        directory_field::SKIN_COUNT,
        "skins",
        ComplexSkinTableSchema,
        directory_field::SKINS
    );
    let joints_ref = directory_target!(
        "joint_count",
        directory_field::JOINT_COUNT,
        "joints",
        ComplexJointTableSchema,
        directory_field::JOINTS
    );
    let logical_morphs_ref = directory_target!(
        "logical_morph_count",
        directory_field::LOGICAL_MORPH_COUNT,
        "logical_morphs",
        ComplexLogicalMorphTableSchema,
        directory_field::LOGICAL_MORPHS
    );
    let overrides_ref = directory_target!(
        "instance_override_count",
        directory_field::INSTANCE_OVERRIDE_COUNT,
        "instance_overrides",
        ComplexInstanceOverrideTableSchema,
        directory_field::INSTANCE_OVERRIDES
    );
    let animations_ref = directory_target!(
        "animation_count",
        directory_field::ANIMATION_COUNT,
        "animations",
        ComplexAnimationTableSchema,
        directory_field::ANIMATIONS
    );
    let source_attachments_ref = directory_target!(
        "source_attachment_count",
        directory_field::SOURCE_ATTACHMENT_COUNT,
        "source_attachments",
        ComplexSourceAttachmentSchema,
        directory_field::SOURCE_ATTACHMENTS
    );

    transcript_assets(
        canonical,
        &mut transcript,
        image_count,
        AssetTableRefs {
            images: images_ref,
            samplers: samplers_ref,
            textures: textures_ref,
            bindings: bindings_ref,
            materials: materials_ref,
        },
    )?;
    let influences_ref = transcript_skinning(canonical, &mut transcript, joints_ref, skins_ref)?;
    transcript_geometry(
        canonical,
        &mut transcript,
        primitives_ref,
        materials_ref,
        skins_ref,
        influences_ref,
        logical_meshes_ref,
    )?;
    transcript_instances(
        canonical,
        &mut transcript,
        InstanceTableRefs {
            nodes: nodes_ref,
            mesh_instances: mesh_instances_ref,
            logical_meshes: logical_meshes_ref,
            skins: skins_ref,
            overrides: overrides_ref,
            logical_morphs: logical_morphs_ref,
        },
    )?;
    transcript_animations(canonical, &mut transcript, animations_ref, overrides_ref)?;
    transcript_source_attachments(canonical, &mut transcript, source_attachments_ref)?;

    Some(transcript)
}

fn transcript_assets(
    canonical: &PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    transcript: &mut String,
    image_count: usize,
    refs: AssetTableRefs,
) -> Option<()> {
    let AssetTableRefs {
        images: images_ref,
        samplers: samplers_ref,
        textures: textures_ref,
        bindings: bindings_ref,
        materials: materials_ref,
    } = refs;
    let images = canonical.records(images_ref)?;
    transcript_record(
        transcript,
        "images",
        images,
        image_count,
        COMPLEX_IMAGE_FIELD_COUNT,
    )?;
    for row in 0..image_count {
        let mime_name = image_leaf_name("image_mime", row, image_count);
        let payload_name = image_leaf_name("image_payload", row, image_count);
        transcript_u32(
            transcript,
            &format!("images[{row}].image"),
            images.scalar(row, image::IMAGE),
        )?;
        let mime_ref = images.utf8_ref::<ComplexImageMimeSchema>(row, image::MIME)?;
        transcript_reference(transcript, &format!("images[{row}].mime"), &mime_name)?;
        let payload_ref = images.bytes_ref::<ComplexImagePayloadSchema>(row, image::PAYLOAD)?;
        transcript_reference(transcript, &format!("images[{row}].payload"), &payload_name)?;
        transcript_bytes(
            transcript,
            "utf8",
            &mime_name,
            canonical.utf8(mime_ref)?.as_bytes(),
        )?;
        transcript_bytes(
            transcript,
            "bytes",
            &payload_name,
            canonical.bytes(payload_ref)?,
        )?;
    }

    let samplers = canonical.records(samplers_ref)?;
    transcript_record(
        transcript,
        "samplers",
        samplers,
        1,
        COMPLEX_SAMPLER_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "samplers[0].sampler",
        samplers.scalar(0, sampler::SAMPLER),
    )?;
    transcript_u32(
        transcript,
        "samplers[0].mag_filter",
        samplers.scalar(0, sampler::MAG_FILTER),
    )?;
    transcript_u32(
        transcript,
        "samplers[0].min_filter",
        samplers.scalar(0, sampler::MIN_FILTER),
    )?;

    let textures = canonical.records(textures_ref)?;
    transcript_record(
        transcript,
        "textures",
        textures,
        1,
        COMPLEX_TEXTURE_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "textures[0].texture",
        textures.scalar(0, texture::TEXTURE),
    )?;
    transcript_u32(
        transcript,
        "textures[0].image_index",
        textures.scalar(0, texture::IMAGE_INDEX),
    )?;
    require_reference(
        textures.record_ref::<ComplexImageTableSchema>(0, texture::IMAGES),
        images_ref,
    )?;
    transcript_reference(transcript, "textures[0].images", "images")?;
    transcript_u32(
        transcript,
        "textures[0].sampler_index",
        textures.scalar(0, texture::SAMPLER_INDEX),
    )?;
    require_reference(
        textures.record_ref::<ComplexSamplerTableSchema>(0, texture::SAMPLERS),
        samplers_ref,
    )?;
    transcript_reference(transcript, "textures[0].samplers", "samplers")?;

    let bindings = canonical.records(bindings_ref)?;
    transcript_record(
        transcript,
        "material_bindings",
        bindings,
        1,
        COMPLEX_MATERIAL_BINDING_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "material_bindings[0].material_binding",
        bindings.scalar(0, binding::MATERIAL_BINDING),
    )?;
    transcript_u32(
        transcript,
        "material_bindings[0].texture_index",
        bindings.scalar(0, binding::TEXTURE_INDEX),
    )?;
    require_reference(
        bindings.record_ref::<ComplexTextureTableSchema>(0, binding::TEXTURES),
        textures_ref,
    )?;
    transcript_reference(transcript, "material_bindings[0].textures", "textures")?;

    let materials = canonical.records(materials_ref)?;
    transcript_record(
        transcript,
        "materials",
        materials,
        1,
        COMPLEX_MATERIAL_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "materials[0].material",
        materials.scalar(0, material::MATERIAL),
    )?;
    transcript_u32(
        transcript,
        "materials[0].material_binding_index",
        materials.scalar(0, material::MATERIAL_BINDING_INDEX),
    )?;
    require_reference(
        materials.record_ref::<ComplexMaterialBindingTableSchema>(0, material::MATERIAL_BINDINGS),
        bindings_ref,
    )?;
    transcript_reference(
        transcript,
        "materials[0].material_bindings",
        "material_bindings",
    )?;

    Some(())
}

fn transcript_skinning(
    canonical: &PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    transcript: &mut String,
    joints_ref: CanonicalRecordRef<ComplexJointTableSchema>,
    skins_ref: CanonicalRecordRef<ComplexSkinTableSchema>,
) -> Option<CanonicalRecordRef<ComplexInfluencePayloadSchema>> {
    let joints = canonical.records(joints_ref)?;
    transcript_record(transcript, "joints", joints, 1, COMPLEX_JOINT_FIELD_COUNT)?;
    transcript_u32(
        transcript,
        "joints[0].joint",
        joints.scalar(0, joint::JOINT),
    )?;

    let skins = canonical.records(skins_ref)?;
    transcript_record(transcript, "skins", skins, 1, COMPLEX_SKIN_FIELD_COUNT)?;
    transcript_u32(transcript, "skins[0].skin", skins.scalar(0, skin::SKIN))?;
    transcript_u32(
        transcript,
        "skins[0].joint_index",
        skins.scalar(0, skin::JOINT_INDEX),
    )?;
    require_reference(
        skins.record_ref::<ComplexJointTableSchema>(0, skin::JOINTS),
        joints_ref,
    )?;
    transcript_reference(transcript, "skins[0].joints", "joints")?;
    transcript_u32(
        transcript,
        "skins[0].inverse_bind_scalar_count",
        skins.scalar(0, skin::INVERSE_BIND_SCALAR_COUNT),
    )?;
    let inverse_binds_ref =
        skins.sequence_ref::<f32, ComplexInverseBindMatricesSchema>(0, skin::INVERSE_BINDS)?;
    transcript_reference(transcript, "skins[0].inverse_binds", "inverse_binds")?;
    transcript_u32(
        transcript,
        "skins[0].influence_row_count",
        skins.scalar(0, skin::INFLUENCE_ROW_COUNT),
    )?;
    let influences_ref = skins.record_ref::<ComplexInfluencePayloadSchema>(0, skin::INFLUENCES)?;
    transcript_reference(transcript, "skins[0].influences", "influences")?;
    transcript_f32_sequence(
        transcript,
        "inverse_binds",
        canonical.sequence(inverse_binds_ref)?.as_slice(),
    )?;

    let influences = canonical.records(influences_ref)?;
    transcript_record(
        transcript,
        "influences",
        influences,
        COMPLEX_INFLUENCE_ROW_COUNT,
        COMPLEX_INFLUENCE_PAYLOAD_FIELD_COUNT,
    )?;
    for row in 0..COMPLEX_INFLUENCE_ROW_COUNT {
        transcript_u32(
            transcript,
            &format!("influences[{row}].vertex"),
            influences.scalar(row, influence::VERTEX),
        )?;
        transcript_u32(
            transcript,
            &format!("influences[{row}].joint_index"),
            influences.scalar(row, influence::JOINT_INDEX),
        )?;
        require_reference(
            influences.record_ref::<ComplexJointTableSchema>(row, influence::JOINTS),
            joints_ref,
        )?;
        transcript_reference(transcript, &format!("influences[{row}].joints"), "joints")?;
        transcript_f32(
            transcript,
            &format!("influences[{row}].weight"),
            influences.scalar(row, influence::WEIGHT),
        )?;
    }

    Some(influences_ref)
}

fn transcript_geometry(
    canonical: &PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    transcript: &mut String,
    primitives_ref: CanonicalRecordRef<ComplexPrimitiveTableSchema>,
    materials_ref: CanonicalRecordRef<ComplexMaterialTableSchema>,
    skins_ref: CanonicalRecordRef<ComplexSkinTableSchema>,
    influences_ref: CanonicalRecordRef<ComplexInfluencePayloadSchema>,
    logical_meshes_ref: CanonicalRecordRef<ComplexLogicalMeshTableSchema>,
) -> Option<()> {
    let primitives = canonical.records(primitives_ref)?;
    transcript_record(
        transcript,
        "primitives",
        primitives,
        1,
        COMPLEX_PRIMITIVE_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "primitives[0].primitive",
        primitives.scalar(0, primitive::PRIMITIVE),
    )?;
    transcript_u32(
        transcript,
        "primitives[0].material_index",
        primitives.scalar(0, primitive::MATERIAL_INDEX),
    )?;
    require_reference(
        primitives.record_ref::<ComplexMaterialTableSchema>(0, primitive::MATERIALS),
        materials_ref,
    )?;
    transcript_reference(transcript, "primitives[0].materials", "materials")?;
    transcript_u32(
        transcript,
        "primitives[0].skin_index",
        primitives.scalar(0, primitive::SKIN_INDEX),
    )?;
    require_reference(
        primitives.record_ref::<ComplexSkinTableSchema>(0, primitive::SKINS),
        skins_ref,
    )?;
    transcript_reference(transcript, "primitives[0].skins", "skins")?;
    transcript_u32(
        transcript,
        "primitives[0].influence_row_count",
        primitives.scalar(0, primitive::INFLUENCE_ROW_COUNT),
    )?;
    require_reference(
        primitives.record_ref::<ComplexInfluencePayloadSchema>(0, primitive::INFLUENCES),
        influences_ref,
    )?;
    transcript_reference(transcript, "primitives[0].influences", "influences")?;

    let logical_meshes = canonical.records(logical_meshes_ref)?;
    transcript_record(
        transcript,
        "logical_meshes",
        logical_meshes,
        1,
        COMPLEX_LOGICAL_MESH_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "logical_meshes[0].logical_mesh",
        logical_meshes.scalar(0, logical_mesh::LOGICAL_MESH),
    )?;
    transcript_u32(
        transcript,
        "logical_meshes[0].primitive_index",
        logical_meshes.scalar(0, logical_mesh::PRIMITIVE_INDEX),
    )?;
    require_reference(
        logical_meshes.record_ref::<ComplexPrimitiveTableSchema>(0, logical_mesh::PRIMITIVES),
        primitives_ref,
    )?;
    transcript_reference(transcript, "logical_meshes[0].primitives", "primitives")?;

    Some(())
}

fn transcript_instances(
    canonical: &PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    transcript: &mut String,
    refs: InstanceTableRefs,
) -> Option<()> {
    let InstanceTableRefs {
        nodes: nodes_ref,
        mesh_instances: mesh_instances_ref,
        logical_meshes: logical_meshes_ref,
        skins: skins_ref,
        overrides: overrides_ref,
        logical_morphs: logical_morphs_ref,
    } = refs;
    let nodes = canonical.records(nodes_ref)?;
    transcript_record(transcript, "nodes", nodes, 1, COMPLEX_NODE_FIELD_COUNT)?;
    transcript_u32(transcript, "nodes[0].node", nodes.scalar(0, node::NODE))?;
    transcript_u32(
        transcript,
        "nodes[0].mesh_instance_index",
        nodes.scalar(0, node::MESH_INSTANCE_INDEX),
    )?;
    require_reference(
        nodes.record_ref::<ComplexMeshInstanceTableSchema>(0, node::MESH_INSTANCES),
        mesh_instances_ref,
    )?;
    transcript_reference(transcript, "nodes[0].mesh_instances", "mesh_instances")?;

    let mesh_instances = canonical.records(mesh_instances_ref)?;
    transcript_record(
        transcript,
        "mesh_instances",
        mesh_instances,
        1,
        COMPLEX_MESH_INSTANCE_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "mesh_instances[0].mesh_instance",
        mesh_instances.scalar(0, mesh_instance::MESH_INSTANCE),
    )?;
    transcript_u32(
        transcript,
        "mesh_instances[0].logical_mesh_index",
        mesh_instances.scalar(0, mesh_instance::LOGICAL_MESH_INDEX),
    )?;
    require_reference(
        mesh_instances
            .record_ref::<ComplexLogicalMeshTableSchema>(0, mesh_instance::LOGICAL_MESHES),
        logical_meshes_ref,
    )?;
    transcript_reference(
        transcript,
        "mesh_instances[0].logical_meshes",
        "logical_meshes",
    )?;
    transcript_u32(
        transcript,
        "mesh_instances[0].skin_index",
        mesh_instances.scalar(0, mesh_instance::SKIN_INDEX),
    )?;
    require_reference(
        mesh_instances.record_ref::<ComplexSkinTableSchema>(0, mesh_instance::SKINS),
        skins_ref,
    )?;
    transcript_reference(transcript, "mesh_instances[0].skins", "skins")?;
    transcript_u32(
        transcript,
        "mesh_instances[0].instance_override_index",
        mesh_instances.scalar(0, mesh_instance::INSTANCE_OVERRIDE_INDEX),
    )?;
    require_reference(
        mesh_instances
            .record_ref::<ComplexInstanceOverrideTableSchema>(0, mesh_instance::INSTANCE_OVERRIDES),
        overrides_ref,
    )?;
    transcript_reference(
        transcript,
        "mesh_instances[0].instance_overrides",
        "instance_overrides",
    )?;

    let logical_morphs = canonical.records(logical_morphs_ref)?;
    transcript_record(
        transcript,
        "logical_morphs",
        logical_morphs,
        1,
        COMPLEX_LOGICAL_MORPH_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "logical_morphs[0].logical_morph",
        logical_morphs.scalar(0, logical_morph::LOGICAL_MORPH),
    )?;
    transcript_u32(
        transcript,
        "logical_morphs[0].target_count",
        logical_morphs.scalar(0, logical_morph::TARGET_COUNT),
    )?;
    transcript_f64(
        transcript,
        "logical_morphs[0].default_weight",
        logical_morphs.scalar(0, logical_morph::DEFAULT_WEIGHT),
    )?;

    let overrides = canonical.records(overrides_ref)?;
    transcript_record(
        transcript,
        "instance_overrides",
        overrides,
        1,
        COMPLEX_INSTANCE_OVERRIDE_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "instance_overrides[0].instance_override",
        overrides.scalar(0, instance_override::INSTANCE_OVERRIDE),
    )?;
    transcript_u32(
        transcript,
        "instance_overrides[0].logical_morph_index",
        overrides.scalar(0, instance_override::LOGICAL_MORPH_INDEX),
    )?;
    require_reference(
        overrides
            .record_ref::<ComplexLogicalMorphTableSchema>(0, instance_override::LOGICAL_MORPHS),
        logical_morphs_ref,
    )?;
    transcript_reference(
        transcript,
        "instance_overrides[0].logical_morphs",
        "logical_morphs",
    )?;
    transcript_f64(
        transcript,
        "instance_overrides[0].weight",
        overrides.scalar(0, instance_override::WEIGHT),
    )?;

    Some(())
}

fn transcript_animations(
    canonical: &PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    transcript: &mut String,
    animations_ref: CanonicalRecordRef<ComplexAnimationTableSchema>,
    overrides_ref: CanonicalRecordRef<ComplexInstanceOverrideTableSchema>,
) -> Option<()> {
    let animations = canonical.records(animations_ref)?;
    transcript_record(
        transcript,
        "animations",
        animations,
        1,
        COMPLEX_ANIMATION_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "animations[0].animation",
        animations.scalar(0, animation::ANIMATION),
    )?;
    transcript_u32(
        transcript,
        "animations[0].instance_override_index",
        animations.scalar(0, animation::INSTANCE_OVERRIDE_INDEX),
    )?;
    require_reference(
        animations
            .record_ref::<ComplexInstanceOverrideTableSchema>(0, animation::INSTANCE_OVERRIDES),
        overrides_ref,
    )?;
    transcript_reference(
        transcript,
        "animations[0].instance_overrides",
        "instance_overrides",
    )?;
    transcript_u32(
        transcript,
        "animations[0].keyframe_count",
        animations.scalar(0, animation::KEYFRAME_COUNT),
    )?;
    let times_ref =
        animations.sequence_ref::<f64, ComplexAnimationTimesSchema>(0, animation::TIMES)?;
    transcript_reference(transcript, "animations[0].times", "animation_times")?;
    transcript_u32(
        transcript,
        "animations[0].morph_weight_value_width",
        animations.scalar(0, animation::MORPH_WEIGHT_VALUE_WIDTH),
    )?;
    let morph_weight_values_ref = animations
        .sequence_ref::<f64, ComplexMorphWeightValuesSchema>(0, animation::MORPH_WEIGHT_VALUES)?;
    transcript_reference(
        transcript,
        "animations[0].morph_weight_values",
        "morph_weight_values",
    )?;
    transcript_f64_sequence(
        transcript,
        "animation_times",
        canonical.sequence(times_ref)?.as_slice(),
    )?;
    transcript_f64_sequence(
        transcript,
        "morph_weight_values",
        canonical.sequence(morph_weight_values_ref)?.as_slice(),
    )?;

    Some(())
}

fn transcript_source_attachments(
    canonical: &PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    transcript: &mut String,
    source_attachments_ref: CanonicalRecordRef<ComplexSourceAttachmentSchema>,
) -> Option<()> {
    let source_attachments = canonical.records(source_attachments_ref)?;
    transcript_record(
        transcript,
        "source_attachments",
        source_attachments,
        1,
        COMPLEX_SOURCE_ATTACHMENT_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "source_attachments[0].source_attachment",
        source_attachments.scalar(0, source_attachment::SOURCE_ATTACHMENT),
    )?;
    transcript_u32(
        transcript,
        "source_attachments[0].metadata_descriptor_index",
        source_attachments.scalar(0, source_attachment::METADATA_DESCRIPTOR_INDEX),
    )?;
    let metadata_descriptors_ref = source_attachments
        .record_ref::<ComplexMetadataDescriptorSchema>(
            0,
            source_attachment::METADATA_DESCRIPTORS,
        )?;
    transcript_reference(
        transcript,
        "source_attachments[0].metadata_descriptors",
        "metadata_descriptors",
    )?;
    transcript_u32(
        transcript,
        "source_attachments[0].provenance_arena_index",
        source_attachments.scalar(0, source_attachment::PROVENANCE_ARENA_INDEX),
    )?;
    let provenance_arenas_ref = source_attachments
        .record_ref::<ComplexProvenanceArenaSchema>(0, source_attachment::PROVENANCE_ARENAS)?;
    transcript_reference(
        transcript,
        "source_attachments[0].provenance_arenas",
        "provenance_arenas",
    )?;

    let metadata_descriptors = canonical.records(metadata_descriptors_ref)?;
    transcript_record(
        transcript,
        "metadata_descriptors",
        metadata_descriptors,
        1,
        COMPLEX_METADATA_DESCRIPTOR_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "metadata_descriptors[0].metadata_descriptor",
        metadata_descriptors.scalar(0, metadata_descriptor::METADATA_DESCRIPTOR),
    )?;
    let metadata_ref =
        metadata_descriptors.utf8_ref::<ComplexMetadataSchema>(0, metadata_descriptor::CONTENT)?;
    transcript_reference(transcript, "metadata_descriptors[0].content", "metadata")?;
    transcript_bytes(
        transcript,
        "utf8",
        "metadata",
        canonical.utf8(metadata_ref)?.as_bytes(),
    )?;

    let provenance_arenas = canonical.records(provenance_arenas_ref)?;
    transcript_record(
        transcript,
        "provenance_arenas",
        provenance_arenas,
        1,
        COMPLEX_PROVENANCE_ARENA_FIELD_COUNT,
    )?;
    transcript_u32(
        transcript,
        "provenance_arenas[0].arena_reference",
        provenance_arenas.scalar(0, provenance_arena::ARENA_REFERENCE),
    )?;
    let provenance_ref =
        provenance_arenas.bytes_ref::<ComplexProvenanceSchema>(0, provenance_arena::CONTENT)?;
    transcript_reference(transcript, "provenance_arenas[0].content", "provenance")?;
    transcript_bytes(
        transcript,
        "bytes",
        "provenance",
        canonical.bytes(provenance_ref)?,
    )?;

    Some(())
}
