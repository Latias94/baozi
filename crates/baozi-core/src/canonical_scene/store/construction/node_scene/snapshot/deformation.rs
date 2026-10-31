use crate::deform::{MorphDeltaSemantic, SkinMatrixSpace, SkinMatrixSpaces};
use crate::math::{Mat4, Vec3};

use super::super::semantic::{
    NodeSceneDeformationCursor, NodeSceneDeformationF32Cursor, NodeSceneDeformationMorphPayload,
    NodeSceneDeformationVec3Cursor,
};
use super::{
    CanonicalDigestEncoder, NodeSceneSnapshotError, PRESENCE_ABSENT, PRESENCE_PRESENT, PreviewSink,
    SnapshotWorkMeter, TAG_COLLECTION_TOTAL, TAG_MESH_INSTANCE, TAG_MESH_INSTANCES, TAG_PRESENCE,
    TAG_REFERENCE, begin_geometry_collection,
};

const TAG_DEFORMATION: u8 = 74;
const TAG_SKINS: u8 = 75;
const TAG_SKIN: u8 = 76;
const TAG_NAME: u8 = 77;
const TAG_NAME_BYTES: u8 = 78;
const TAG_SKELETON_ROOT: u8 = 79;
const TAG_MATRIX_SPACES: u8 = 80;
const TAG_INPUT_POSITION: u8 = 81;
const TAG_INVERSE_BIND_FROM: u8 = 82;
const TAG_INVERSE_BIND_TO: u8 = 83;
const TAG_CURRENT_JOINT_FROM: u8 = 84;
const TAG_CURRENT_JOINT_TO: u8 = 85;
const TAG_OUTPUT_POSITION: u8 = 86;
const TAG_CORRECTION: u8 = 87;
const TAG_CORRECTION_FROM: u8 = 88;
const TAG_CORRECTION_TO: u8 = 89;
const TAG_CORRECTION_MATRIX: u8 = 90;
const TAG_JOINTS: u8 = 91;
const TAG_INVERSE_BINDS: u8 = 92;
const TAG_LOGICAL_MORPHS: u8 = 93;
const TAG_LOGICAL_MORPH: u8 = 94;
const TAG_DEFINITION_OFFSET: u8 = 95;
const TAG_MORPH_WEIGHTS: u8 = 96;
const TAG_MORPH_DEFINITIONS: u8 = 97;
const TAG_MORPH_DEFINITION: u8 = 98;
const TAG_LOGICAL_MESH_REFERENCE: u8 = 99;
const TAG_TARGET_ORDINAL: u8 = 100;
const TAG_STABLE_IDENTITY: u8 = 101;
const TAG_PRIMITIVES: u8 = 102;
const TAG_PRIMITIVE: u8 = 103;
const TAG_INFLUENCE_ROWS: u8 = 104;
const TAG_INFLUENCE_ROW: u8 = 105;
const TAG_INFLUENCE: u8 = 106;
const TAG_PALETTE_JOINT: u8 = 107;
const TAG_SOURCE_ORDER: u8 = 108;
const TAG_WEIGHT_BITS: u8 = 109;
const TAG_MORPH_TARGETS: u8 = 110;
const TAG_MORPH_TARGET: u8 = 111;
const TAG_DELTA_OFFSET: u8 = 112;
const TAG_MORPH_DELTAS: u8 = 113;
const TAG_MORPH_DELTA: u8 = 114;
const TAG_MORPH_TARGET_REFERENCE: u8 = 115;
const TAG_DELTA_SEMANTIC: u8 = 116;
const TAG_PAYLOAD_KIND: u8 = 117;
const TAG_LOGICAL_COUNT: u8 = 118;
const TAG_EXPLICIT_COUNT: u8 = 119;

const PAYLOAD_DENSE: u8 = 0;
const PAYLOAD_SPARSE: u8 = 1;

#[derive(Clone, Copy)]
pub(super) struct DeformationCollectionCounts {
    pub(super) skins: usize,
    pub(super) logical_morphs: usize,
    pub(super) morph_definitions: usize,
    pub(super) primitives: usize,
    pub(super) morph_targets: usize,
    pub(super) morph_deltas: usize,
    pub(super) mesh_instances: usize,
}

#[derive(Clone, Copy, Default)]
pub(super) struct DeformationPayloadTotals {
    pub(super) skin_joints: u64,
    pub(super) inverse_bind_matrices: u64,
    pub(super) skin_correction_matrices: u64,
    pub(super) skin_name_bytes: u64,
    pub(super) morph_default_weights: u64,
    pub(super) morph_name_bytes: u64,
    pub(super) influence_rows: u64,
    pub(super) influences: u64,
    pub(super) morph_value_vectors: u64,
    pub(super) morph_sparse_indices: u64,
    pub(super) instance_morph_weights: u64,
}

pub(super) fn write_deformation(
    cursor: &mut NodeSceneDeformationCursor<'_>,
    counts: DeformationCollectionCounts,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<DeformationPayloadTotals, NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_DEFORMATION)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.push_str("deformation(present)\n");

    let mut totals = DeformationPayloadTotals::default();
    write_skins(cursor, counts.skins, transcript, preview, work, &mut totals)?;
    write_logical_morphs(
        cursor,
        counts.logical_morphs,
        transcript,
        preview,
        work,
        &mut totals,
    )?;
    write_morph_definitions(
        cursor,
        counts.morph_definitions,
        transcript,
        preview,
        work,
        &mut totals,
    )?;
    write_primitives(
        cursor,
        counts.primitives,
        transcript,
        preview,
        work,
        &mut totals,
    )?;
    write_morph_targets(cursor, counts.morph_targets, transcript, preview, work)?;
    write_morph_deltas(
        cursor,
        counts.morph_deltas,
        transcript,
        preview,
        work,
        &mut totals,
    )?;
    write_mesh_instances(
        cursor,
        counts.mesh_instances,
        transcript,
        preview,
        work,
        &mut totals,
    )?;
    preview.push_str("deformation_end\n");
    Ok(totals)
}

fn write_skins(
    cursor: &mut NodeSceneDeformationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut DeformationPayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(transcript, preview, work, TAG_SKINS, "skins", count)?;
    for _ in 0..count {
        work.observe()?;
        let mut skin = cursor
            .next_skin()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "skin collection ended before declared total",
            })?;
        transcript.write_tag(TAG_SKIN)?;
        transcript.tagged_usize(TAG_REFERENCE, skin.source_index())?;
        write_optional_name(transcript, work, skin.name())?;
        totals.skin_name_bytes = add_count(
            totals.skin_name_bytes,
            skin.name().map_or(0, str::len),
            "skin name bytes",
        )?;
        write_optional_reference(transcript, TAG_SKELETON_ROOT, skin.skeleton_root())?;
        write_matrix_spaces(transcript, work, skin.matrix_spaces(), totals)?;

        let mut joints = skin
            .take_joints()
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "skin joint cursor was already consumed",
            })?;
        let joint_count = joints.declared_count();
        begin_leaf(transcript, TAG_JOINTS, true, joint_count)?;
        for _ in 0..joint_count {
            work.observe()?;
            let joint = joints
                .next()
                .map_err(super::map_snapshot_invariant)?
                .ok_or(NodeSceneSnapshotError::InconsistentView {
                    reason: "skin joint collection ended before declared total",
                })?;
            transcript.tagged_usize(TAG_REFERENCE, joint)?;
        }
        joints.finish().map_err(super::map_snapshot_invariant)?;
        totals.skin_joints = add_count(totals.skin_joints, joint_count, "skin joints")?;

        let mut matrices =
            skin.take_inverse_bind_matrices()
                .ok_or(NodeSceneSnapshotError::InconsistentView {
                    reason: "inverse-bind matrix cursor was already consumed",
                })?;
        let matrix_count = matrices.declared_count();
        begin_leaf(transcript, TAG_INVERSE_BINDS, true, matrix_count)?;
        for _ in 0..matrix_count {
            work.observe()?;
            let matrix = matrices
                .next()
                .map_err(super::map_snapshot_invariant)?
                .ok_or(NodeSceneSnapshotError::InconsistentView {
                    reason: "inverse-bind matrix collection ended before declared total",
                })?;
            write_matrix(transcript, work, matrix)?;
        }
        matrices.finish().map_err(super::map_snapshot_invariant)?;
        totals.inverse_bind_matrices = add_count(
            totals.inverse_bind_matrices,
            matrix_count,
            "inverse-bind matrices",
        )?;

        let render = preview.begin_geometry_item("skin", skin.source_index())?;
        preview.geometry_field_word(render, "name", skin.name().unwrap_or("absent"));
        preview.geometry_optional_reference(render, "skeleton_root", skin.skeleton_root());
        preview.geometry_field_usize(render, "joints", joint_count);
        preview.geometry_field_usize(render, "inverse_binds", matrix_count);
        preview.geometry_field_word(
            render,
            "correction",
            if skin.matrix_spaces().correction().is_some() {
                "present"
            } else {
                "absent"
            },
        );
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_logical_morphs(
    cursor: &mut NodeSceneDeformationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut DeformationPayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_LOGICAL_MORPHS,
        "logical_morphs",
        count,
    )?;
    for _ in 0..count {
        work.observe()?;
        let mut logical = cursor
            .next_logical_morph()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "logical-morph collection ended before declared total",
            })?;
        transcript.write_tag(TAG_LOGICAL_MORPH)?;
        transcript.tagged_usize(TAG_REFERENCE, logical.source_index())?;
        transcript.tagged_usize(TAG_DEFINITION_OFFSET, logical.definition_offset())?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, logical.definition_count())?;
        let weights =
            logical
                .take_default_weights()
                .ok_or(NodeSceneSnapshotError::InconsistentView {
                    reason: "logical-morph default cursor was already consumed",
                })?;
        let weight_count = write_f32_cursor(
            weights,
            transcript,
            preview,
            work,
            TAG_MORPH_WEIGHTS,
            false,
            "default_weights",
        )?;
        totals.morph_default_weights = add_count(
            totals.morph_default_weights,
            weight_count,
            "morph default weights",
        )?;
        let render = preview.begin_geometry_item("logical_morph", logical.source_index())?;
        preview.geometry_field_usize(render, "definition_offset", logical.definition_offset());
        preview.geometry_field_usize(render, "definition_count", logical.definition_count());
        preview.geometry_field_usize(render, "default_weights", weight_count);
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_morph_definitions(
    cursor: &mut NodeSceneDeformationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut DeformationPayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_MORPH_DEFINITIONS,
        "morph_definitions",
        count,
    )?;
    for _ in 0..count {
        work.observe()?;
        let definition = cursor
            .next_morph_definition()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "morph-definition collection ended before declared total",
            })?;
        transcript.write_tag(TAG_MORPH_DEFINITION)?;
        transcript.tagged_usize(TAG_REFERENCE, definition.source_index())?;
        transcript.tagged_usize(TAG_LOGICAL_MESH_REFERENCE, definition.logical_mesh())?;
        transcript.tagged_usize(TAG_TARGET_ORDINAL, definition.target_ordinal())?;
        transcript.tagged_u32(TAG_STABLE_IDENTITY, definition.stable_identity())?;
        write_optional_name(transcript, work, definition.name())?;
        totals.morph_name_bytes = add_count(
            totals.morph_name_bytes,
            definition.name().map_or(0, str::len),
            "morph name bytes",
        )?;
        let render = preview.begin_geometry_item("morph_definition", definition.source_index())?;
        preview.geometry_field_usize(render, "logical_mesh", definition.logical_mesh());
        preview.geometry_field_usize(render, "target_ordinal", definition.target_ordinal());
        preview.geometry_field_u32(render, "stable_identity", definition.stable_identity());
        preview.geometry_field_word(render, "name", definition.name().unwrap_or("absent"));
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_primitives(
    cursor: &mut NodeSceneDeformationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut DeformationPayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_PRIMITIVES,
        "primitive_deformations",
        count,
    )?;
    for _ in 0..count {
        work.observe()?;
        let mut primitive = cursor
            .next_primitive()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "primitive-deformation collection ended before declared total",
            })?;
        transcript.write_tag(TAG_PRIMITIVE)?;
        transcript.tagged_usize(TAG_REFERENCE, primitive.source_index())?;
        transcript.write_tag(TAG_INFLUENCE_ROWS)?;
        let influences_present = primitive.influences_present();
        let (row_count, influence_count) = match primitive.take_influences() {
            Some(mut rows) => {
                transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
                let row_count = rows.vertex_count();
                let mut primitive_influence_count = 0_usize;
                for _ in 0..row_count {
                    work.observe()?;
                    let row = rows
                        .next_row()
                        .map_err(super::map_snapshot_invariant)?
                        .ok_or(NodeSceneSnapshotError::InconsistentView {
                            reason: "influence-row collection ended before declared total",
                        })?;
                    transcript.write_tag(TAG_INFLUENCE_ROW)?;
                    transcript.tagged_usize(TAG_REFERENCE, row.vertex())?;
                    let mut influences = row.into_influences();
                    let declared = influences.declared_count();
                    for _ in 0..declared {
                        work.observe()?;
                        let influence = influences
                            .next()
                            .map_err(super::map_snapshot_invariant)?
                            .ok_or(NodeSceneSnapshotError::InconsistentView {
                            reason: "influence collection ended before declared total",
                        })?;
                        transcript.write_tag(TAG_INFLUENCE)?;
                        transcript.tagged_u32(TAG_PALETTE_JOINT, influence.palette_joint())?;
                        transcript.tagged_u32(TAG_WEIGHT_BITS, influence.weight().to_bits())?;
                        transcript.tagged_u32(TAG_SOURCE_ORDER, influence.source_order())?;
                    }
                    influences.finish().map_err(super::map_snapshot_invariant)?;
                    transcript.tagged_usize(TAG_COLLECTION_TOTAL, declared)?;
                    totals.influences = add_count(totals.influences, declared, "influences")?;
                    primitive_influence_count = primitive_influence_count
                        .checked_add(declared)
                        .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow {
                            total: "primitive influences",
                        })?;
                }
                rows.finish().map_err(super::map_snapshot_invariant)?;
                transcript.tagged_usize(TAG_COLLECTION_TOTAL, row_count)?;
                totals.influence_rows =
                    add_count(totals.influence_rows, row_count, "influence rows")?;
                (row_count, primitive_influence_count)
            }
            None => {
                transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?;
                transcript.tagged_usize(TAG_COLLECTION_TOTAL, 0)?;
                (0, 0)
            }
        };
        transcript.write_tag(TAG_MORPH_TARGETS)?;
        transcript.tagged_u8(
            TAG_PRESENCE,
            if primitive.morph_target_count() == 0 {
                PRESENCE_ABSENT
            } else {
                PRESENCE_PRESENT
            },
        )?;
        transcript.tagged_usize(TAG_REFERENCE, primitive.morph_target_offset())?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, primitive.morph_target_count())?;

        let render =
            preview.begin_geometry_item("primitive_deformation", primitive.source_index())?;
        preview.geometry_field_word(
            render,
            "influences",
            if influences_present {
                "present"
            } else {
                "absent"
            },
        );
        preview.geometry_field_usize(render, "influence_rows", row_count);
        preview.geometry_field_usize(render, "influences_observed", influence_count);
        preview.geometry_field_usize(
            render,
            "morph_target_offset",
            primitive.morph_target_offset(),
        );
        preview.geometry_field_usize(render, "morph_target_count", primitive.morph_target_count());
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_morph_targets(
    cursor: &mut NodeSceneDeformationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_MORPH_TARGETS,
        "morph_targets",
        count,
    )?;
    for _ in 0..count {
        work.observe()?;
        let target = cursor
            .next_morph_target()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "morph-target collection ended before declared total",
            })?;
        transcript.write_tag(TAG_MORPH_TARGET)?;
        transcript.tagged_usize(TAG_REFERENCE, target.source_index())?;
        transcript.tagged_usize(TAG_PRIMITIVE, target.primitive())?;
        transcript.tagged_usize(TAG_TARGET_ORDINAL, target.target_ordinal())?;
        transcript.tagged_usize(TAG_DELTA_OFFSET, target.delta_offset())?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, target.delta_count())?;
        let render = preview.begin_geometry_item("morph_target", target.source_index())?;
        preview.geometry_field_usize(render, "primitive", target.primitive());
        preview.geometry_field_usize(render, "target_ordinal", target.target_ordinal());
        preview.geometry_field_usize(render, "delta_offset", target.delta_offset());
        preview.geometry_field_usize(render, "delta_count", target.delta_count());
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_morph_deltas(
    cursor: &mut NodeSceneDeformationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut DeformationPayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_MORPH_DELTAS,
        "morph_deltas",
        count,
    )?;
    for _ in 0..count {
        work.observe()?;
        let delta = cursor
            .next_morph_delta()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "morph-delta collection ended before declared total",
            })?;
        let source_index = delta.source_index();
        let morph_target = delta.morph_target();
        let semantic = delta.semantic();
        transcript.write_tag(TAG_MORPH_DELTA)?;
        transcript.tagged_usize(TAG_REFERENCE, source_index)?;
        transcript.tagged_usize(TAG_MORPH_TARGET_REFERENCE, morph_target)?;
        transcript.tagged_u8(TAG_DELTA_SEMANTIC, encode_delta_semantic(semantic))?;
        let (kind, logical_count, explicit_count) =
            write_morph_payload(delta.into_payload(), transcript, preview, work, totals)?;
        let render = preview.begin_geometry_item("morph_delta", source_index)?;
        preview.geometry_field_usize(render, "morph_target", morph_target);
        preview.geometry_field_u32(
            render,
            "semantic",
            u32::from(encode_delta_semantic(semantic)),
        );
        preview.geometry_field_u32(render, "payload_kind", u32::from(kind));
        preview.geometry_field_usize(render, "logical_count", logical_count);
        preview.geometry_field_usize(render, "explicit_count", explicit_count);
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_morph_payload(
    payload: NodeSceneDeformationMorphPayload<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut DeformationPayloadTotals,
) -> Result<(u8, usize, usize), NodeSceneSnapshotError> {
    let logical_count = payload.logical_count();
    let explicit_count = payload.explicit_count();
    match payload {
        NodeSceneDeformationMorphPayload::Dense { values, .. } => {
            transcript.tagged_u8(TAG_PAYLOAD_KIND, PAYLOAD_DENSE)?;
            transcript.tagged_usize(TAG_LOGICAL_COUNT, logical_count)?;
            transcript.tagged_usize(TAG_EXPLICIT_COUNT, explicit_count)?;
            write_vec3_cursor(values, transcript, preview, work, false)?;
            totals.morph_value_vectors = add_count(
                totals.morph_value_vectors,
                explicit_count,
                "morph value vectors",
            )?;
            Ok((PAYLOAD_DENSE, logical_count, explicit_count))
        }
        NodeSceneDeformationMorphPayload::Sparse { mut values, .. } => {
            transcript.tagged_u8(TAG_PAYLOAD_KIND, PAYLOAD_SPARSE)?;
            transcript.tagged_usize(TAG_LOGICAL_COUNT, logical_count)?;
            transcript.tagged_usize(TAG_EXPLICIT_COUNT, explicit_count)?;
            for _ in 0..explicit_count {
                work.observe()?;
                let value = values
                    .next()
                    .map_err(super::map_snapshot_invariant)?
                    .ok_or(NodeSceneSnapshotError::InconsistentView {
                        reason: "sparse morph collection ended before declared total",
                    })?;
                transcript.tagged_usize(TAG_REFERENCE, value.index())?;
                write_vec3(transcript, work, value.value())?;
            }
            values.finish().map_err(super::map_snapshot_invariant)?;
            transcript.tagged_usize(TAG_COLLECTION_TOTAL, explicit_count)?;
            totals.morph_value_vectors = add_count(
                totals.morph_value_vectors,
                explicit_count,
                "morph value vectors",
            )?;
            totals.morph_sparse_indices = add_count(
                totals.morph_sparse_indices,
                explicit_count,
                "morph sparse indices",
            )?;
            Ok((PAYLOAD_SPARSE, logical_count, explicit_count))
        }
    }
}

fn write_mesh_instances(
    cursor: &mut NodeSceneDeformationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut DeformationPayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(
        transcript,
        preview,
        work,
        TAG_MESH_INSTANCES,
        "deformation_mesh_instances",
        count,
    )?;
    for _ in 0..count {
        work.observe()?;
        let mut instance = cursor
            .next_mesh_instance()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "deformation mesh-instance collection ended before declared total",
            })?;
        transcript.write_tag(TAG_MESH_INSTANCE)?;
        transcript.tagged_usize(TAG_REFERENCE, instance.source_index())?;
        write_optional_reference(transcript, TAG_SKIN, instance.skin())?;
        let morph_weights_present = instance.morph_weights_present();
        let weight_count = match instance.take_morph_weights() {
            Some(weights) => write_f32_cursor(
                weights,
                transcript,
                preview,
                work,
                TAG_MORPH_WEIGHTS,
                false,
                "morph_weights",
            )?,
            None => {
                begin_leaf(transcript, TAG_MORPH_WEIGHTS, false, 0)?;
                0
            }
        };
        totals.instance_morph_weights = add_count(
            totals.instance_morph_weights,
            weight_count,
            "instance morph weights",
        )?;
        let render =
            preview.begin_geometry_item("deformation_mesh_instance", instance.source_index())?;
        preview.geometry_optional_reference(render, "skin", instance.skin());
        preview.geometry_field_word(
            render,
            "morph_weights",
            if morph_weights_present {
                "present"
            } else {
                "absent"
            },
        );
        preview.geometry_field_usize(render, "morph_weight_count", weight_count);
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_matrix_spaces(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    spaces: SkinMatrixSpaces,
    totals: &mut DeformationPayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(TAG_MATRIX_SPACES)?;
    transcript.tagged_u8(
        TAG_INPUT_POSITION,
        encode_skin_space(spaces.input_position()),
    )?;
    transcript.tagged_u8(
        TAG_INVERSE_BIND_FROM,
        encode_skin_space(spaces.inverse_bind_from()),
    )?;
    transcript.tagged_u8(
        TAG_INVERSE_BIND_TO,
        encode_skin_space(spaces.inverse_bind_to()),
    )?;
    transcript.tagged_u8(
        TAG_CURRENT_JOINT_FROM,
        encode_skin_space(spaces.current_joint_from()),
    )?;
    transcript.tagged_u8(
        TAG_CURRENT_JOINT_TO,
        encode_skin_space(spaces.current_joint_to()),
    )?;
    transcript.tagged_u8(
        TAG_OUTPUT_POSITION,
        encode_skin_space(spaces.output_position()),
    )?;
    transcript.write_tag(TAG_CORRECTION)?;
    match spaces.correction() {
        Some(correction) => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
            transcript.tagged_u8(TAG_CORRECTION_FROM, encode_skin_space(correction.from()))?;
            transcript.tagged_u8(TAG_CORRECTION_TO, encode_skin_space(correction.to()))?;
            transcript.write_tag(TAG_CORRECTION_MATRIX)?;
            write_matrix(transcript, work, correction.matrix())?;
            totals.skin_correction_matrices = add_count(
                totals.skin_correction_matrices,
                1,
                "skin correction matrices",
            )?;
        }
        None => transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?,
    }
    Ok(())
}

fn write_optional_name(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    name: Option<&str>,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(TAG_NAME)?;
    match name {
        Some(name) => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
            work.observe()?;
            transcript.tagged_bytes(TAG_NAME_BYTES, name.as_bytes())?;
        }
        None => transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?,
    }
    Ok(())
}

fn write_optional_reference(
    transcript: &mut CanonicalDigestEncoder,
    tag: u8,
    reference: Option<usize>,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(tag)?;
    match reference {
        Some(reference) => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
            transcript.tagged_usize(TAG_REFERENCE, reference)?;
        }
        None => transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?,
    }
    Ok(())
}

fn begin_leaf(
    transcript: &mut CanonicalDigestEncoder,
    tag: u8,
    present: bool,
    count: usize,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(tag)?;
    transcript.tagged_u8(
        TAG_PRESENCE,
        if present {
            PRESENCE_PRESENT
        } else {
            PRESENCE_ABSENT
        },
    )?;
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)
}

fn write_f32_cursor(
    mut cursor: NodeSceneDeformationF32Cursor<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    render: bool,
    preview_name: &'static str,
) -> Result<usize, NodeSceneSnapshotError> {
    let count = cursor.declared_count();
    transcript.write_tag(tag)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_geometry_leaf(render, preview_name, true, count);
    for ordinal in 0..count {
        work.observe()?;
        let value = cursor
            .next()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "deformation scalar collection ended before declared total",
            })?;
        let bits = value.to_bits();
        transcript.tagged_u32(TAG_WEIGHT_BITS, bits)?;
        preview.geometry_f32_bits(render, ordinal, bits)?;
    }
    cursor.finish().map_err(super::map_snapshot_invariant)?;
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    preview.end_geometry_leaf(render);
    Ok(count)
}

fn write_vec3_cursor(
    mut cursor: NodeSceneDeformationVec3Cursor<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    render: bool,
) -> Result<(), NodeSceneSnapshotError> {
    let count = cursor.declared_count();
    preview.begin_geometry_leaf(render, "values", true, count);
    for _ in 0..count {
        work.observe()?;
        let value = cursor
            .next()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "dense morph collection ended before declared total",
            })?;
        write_vec3(transcript, work, value)?;
    }
    cursor.finish().map_err(super::map_snapshot_invariant)?;
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    preview.end_geometry_leaf(render);
    Ok(())
}

fn write_vec3(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    value: Vec3,
) -> Result<(), NodeSceneSnapshotError> {
    for component in [value.x, value.y, value.z] {
        work.observe()?;
        transcript.tagged_u32(TAG_WEIGHT_BITS, component.to_bits())?;
    }
    Ok(())
}

fn write_matrix(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    matrix: Mat4,
) -> Result<(), NodeSceneSnapshotError> {
    for column in matrix.cols {
        for scalar in column {
            work.observe()?;
            transcript.tagged_u32(TAG_WEIGHT_BITS, scalar.to_bits())?;
        }
    }
    Ok(())
}

fn add_count(total: u64, count: usize, name: &'static str) -> Result<u64, NodeSceneSnapshotError> {
    total
        .checked_add(
            u64::try_from(count)
                .map_err(|_| NodeSceneSnapshotError::SemanticTotalOverflow { total: name })?,
        )
        .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow { total: name })
}

const fn encode_skin_space(space: SkinMatrixSpace) -> u8 {
    match space {
        SkinMatrixSpace::LogicalMeshLocalBind => 0,
        SkinMatrixSpace::JointLocalBind => 1,
        SkinMatrixSpace::JointLocal => 2,
        SkinMatrixSpace::Scene => 3,
        SkinMatrixSpace::MeshInstanceLocal => 4,
    }
}

const fn encode_delta_semantic(semantic: MorphDeltaSemantic) -> u8 {
    match semantic {
        MorphDeltaSemantic::Position => 0,
        MorphDeltaSemantic::Normal => 1,
        MorphDeltaSemantic::Tangent => 2,
    }
}
