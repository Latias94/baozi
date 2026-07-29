use baozi_operation::{
    CanonicalRecordView, PublishedCanonicalStore, RecordBlock, ScalarSequence, SequenceView,
    StoredScalar, Utf8Block,
};

use crate::canonical_scene::store::schema::deformation::{
    DEFORMATION_LOGICAL_MORPH_FIELD_COUNT, DEFORMATION_MESH_INSTANCE_FIELD_COUNT,
    DEFORMATION_MORPH_DEFINITION_FIELD_COUNT, DEFORMATION_MORPH_DELTA_FIELD_COUNT,
    DEFORMATION_MORPH_DENSE, DEFORMATION_MORPH_NORMAL, DEFORMATION_MORPH_POSITION,
    DEFORMATION_MORPH_SPARSE, DEFORMATION_MORPH_TANGENT, DEFORMATION_MORPH_TARGET_FIELD_COUNT,
    DEFORMATION_PRIMITIVE_FIELD_COUNT, DEFORMATION_SKIN_FIELD_COUNT, DEFORMATION_SPACE_JOINT_LOCAL,
    DEFORMATION_SPACE_JOINT_LOCAL_BIND, DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
    DEFORMATION_SPACE_MESH_INSTANCE_LOCAL, DEFORMATION_SPACE_SCENE,
    deformation_logical_morph_field, deformation_mesh_instance_field,
    deformation_morph_definition_field, deformation_morph_delta_field,
    deformation_morph_target_field, deformation_primitive_field, deformation_skin_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, PRESENCE_ABSENT, PRESENCE_PRESENT, SceneDirectorySchema,
};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::deform::{MorphDeltaSemantic, SkinMatrixCorrection, SkinMatrixSpace, SkinMatrixSpaces};
use crate::math::{Mat4, Vec3};

use super::{NodeSceneSemanticResult, invariant, read_scalar, read_u32_as_usize};

type SceneCanonicalStore = PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>;

const MATRIX_SCALAR_COUNT: usize = 16;
const MORPH_VECTOR_WIDTH: usize = 3;

/// Proof-bound deformation tables and leaf payloads borrowed from one Scene.
pub(super) struct NodeSceneDeformationTables<'scene> {
    skins: Option<CanonicalRecordView<'scene>>,
    logical_morphs: Option<CanonicalRecordView<'scene>>,
    morph_definitions: Option<CanonicalRecordView<'scene>>,
    primitives: Option<CanonicalRecordView<'scene>>,
    morph_targets: Option<CanonicalRecordView<'scene>>,
    morph_deltas: Option<CanonicalRecordView<'scene>>,
    mesh_instances: Option<CanonicalRecordView<'scene>>,
    joint_nodes: Option<&'scene [u32]>,
    inverse_bind_matrices: Option<&'scene [f32]>,
    skin_correction_matrices: Option<&'scene [f32]>,
    skin_names: Option<&'scene str>,
    influence_row_offsets: Option<&'scene [u32]>,
    influence_joints: Option<&'scene [u32]>,
    influence_weights: Option<&'scene [f32]>,
    influence_source_orders: Option<&'scene [u32]>,
    morph_defaults: Option<&'scene [f32]>,
    morph_names: Option<&'scene str>,
    morph_values: Option<&'scene [f32]>,
    morph_sparse_indices: Option<&'scene [u32]>,
    instance_morph_weights: Option<&'scene [f32]>,
}

impl<'scene> NodeSceneDeformationTables<'scene> {
    pub(super) fn try_new(
        canonical: &'scene SceneCanonicalStore,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        Ok(Self {
            skins: resolve_table(
                canonical,
                proof.deformation_skins(),
                proof.skin_count(),
                DEFORMATION_SKIN_FIELD_COUNT,
                "validated deformation skin table is unavailable or mistyped",
                "validated deformation skin table presence disagrees with proof",
                "validated deformation skin table layout disagrees with proof",
            )?,
            logical_morphs: resolve_table(
                canonical,
                proof.deformation_logical_morphs(),
                proof.logical_morph_count(),
                DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
                "validated logical-morph table is unavailable or mistyped",
                "validated logical-morph table presence disagrees with proof",
                "validated logical-morph table layout disagrees with proof",
            )?,
            morph_definitions: resolve_table(
                canonical,
                proof.deformation_morph_definitions(),
                proof.morph_definition_count(),
                DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
                "validated morph-definition table is unavailable or mistyped",
                "validated morph-definition table presence disagrees with proof",
                "validated morph-definition table layout disagrees with proof",
            )?,
            primitives: resolve_table(
                canonical,
                proof.deformation_primitives(),
                proof.primitive_deformation_count(),
                DEFORMATION_PRIMITIVE_FIELD_COUNT,
                "validated primitive-deformation table is unavailable or mistyped",
                "validated primitive-deformation table presence disagrees with proof",
                "validated primitive-deformation table layout disagrees with proof",
            )?,
            morph_targets: resolve_table(
                canonical,
                proof.deformation_morph_targets(),
                proof.morph_target_count(),
                DEFORMATION_MORPH_TARGET_FIELD_COUNT,
                "validated morph-target table is unavailable or mistyped",
                "validated morph-target table presence disagrees with proof",
                "validated morph-target table layout disagrees with proof",
            )?,
            morph_deltas: resolve_table(
                canonical,
                proof.deformation_morph_deltas(),
                proof.morph_delta_count(),
                DEFORMATION_MORPH_DELTA_FIELD_COUNT,
                "validated morph-delta table is unavailable or mistyped",
                "validated morph-delta table presence disagrees with proof",
                "validated morph-delta table layout disagrees with proof",
            )?,
            mesh_instances: resolve_table(
                canonical,
                proof.deformation_mesh_instances(),
                proof.mesh_instance_count(),
                DEFORMATION_MESH_INSTANCE_FIELD_COUNT,
                "validated mesh-instance deformation table is unavailable or mistyped",
                "validated mesh-instance deformation table presence disagrees with proof",
                "validated mesh-instance deformation table layout disagrees with proof",
            )?,
            joint_nodes: resolve_sequence(
                canonical,
                proof.deformation_joint_nodes(),
                "validated skin joint-node payload is unavailable or mistyped",
            )?,
            inverse_bind_matrices: resolve_sequence(
                canonical,
                proof.deformation_inverse_bind_matrices(),
                "validated inverse-bind payload is unavailable or mistyped",
            )?,
            skin_correction_matrices: resolve_sequence(
                canonical,
                proof.deformation_skin_correction_matrices(),
                "validated skin-correction payload is unavailable or mistyped",
            )?,
            skin_names: resolve_utf8(
                canonical,
                proof.deformation_skin_names(),
                "validated skin-name payload is unavailable or mistyped",
            )?,
            influence_row_offsets: resolve_sequence(
                canonical,
                proof.deformation_influence_row_offsets(),
                "validated influence-row-offset payload is unavailable or mistyped",
            )?,
            influence_joints: resolve_sequence(
                canonical,
                proof.deformation_influence_joints(),
                "validated influence-joint payload is unavailable or mistyped",
            )?,
            influence_weights: resolve_sequence(
                canonical,
                proof.deformation_influence_weights(),
                "validated influence-weight payload is unavailable or mistyped",
            )?,
            influence_source_orders: resolve_sequence(
                canonical,
                proof.deformation_influence_source_orders(),
                "validated influence source-order payload is unavailable or mistyped",
            )?,
            morph_defaults: resolve_sequence(
                canonical,
                proof.deformation_morph_defaults(),
                "validated morph-default payload is unavailable or mistyped",
            )?,
            morph_names: resolve_utf8(
                canonical,
                proof.deformation_morph_names(),
                "validated morph-name payload is unavailable or mistyped",
            )?,
            morph_values: resolve_sequence(
                canonical,
                proof.deformation_morph_values(),
                "validated morph-value payload is unavailable or mistyped",
            )?,
            morph_sparse_indices: resolve_sequence(
                canonical,
                proof.deformation_morph_sparse_indices(),
                "validated morph sparse-index payload is unavailable or mistyped",
            )?,
            instance_morph_weights: resolve_sequence(
                canonical,
                proof.deformation_instance_morph_weights(),
                "validated instance morph-weight payload is unavailable or mistyped",
            )?,
        })
    }

    pub(super) const fn into_cursor(
        self,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneDeformationCursor<'scene> {
        NodeSceneDeformationCursor {
            tables: self,
            proof,
            phase: DeformationCursorPhase::Skins,
            next_skin: 0,
            next_logical_morph: 0,
            next_morph_definition: 0,
            next_primitive: 0,
            next_morph_target: 0,
            next_morph_delta: 0,
            next_mesh_instance: 0,
            joint_cursor: 0,
            inverse_bind_cursor: 0,
            correction_cursor: 0,
            skin_name_cursor: 0,
            definition_cursor: 0,
            morph_default_cursor: 0,
            morph_name_cursor: 0,
            influence_row_offset_cursor: 0,
            influence_cursor: 0,
            target_cursor: 0,
            delta_cursor: 0,
            morph_value_cursor: 0,
            sparse_index_cursor: 0,
            instance_weight_cursor: 0,
        }
    }
}

fn resolve_table<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    expected_rows: usize,
    expected_fields: usize,
    unavailable_reason: &'static str,
    presence_reason: &'static str,
    layout_reason: &'static str,
) -> NodeSceneSemanticResult<Option<CanonicalRecordView<'scene>>>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    match (expected_rows, reference) {
        (0, None) => Ok(None),
        (0, Some(_)) | (_, None) => Err(invariant(presence_reason)),
        (_, Some(reference)) => {
            let table = canonical
                .records(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))?;
            if table.rows() != expected_rows || table.fields() != expected_fields {
                return Err(invariant(layout_reason));
            }
            Ok(Some(table))
        }
    }
}

fn resolve_sequence<'scene, T, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene [T]>>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    reference
        .map(|reference| {
            canonical
                .sequence(reference.into_store())
                .map(|sequence: SequenceView<'scene, T>| sequence.as_slice())
                .ok_or_else(|| invariant(reason))
        })
        .transpose()
}

fn resolve_utf8<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene str>>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    reference
        .map(|reference| {
            canonical
                .utf8(reference.into_store())
                .ok_or_else(|| invariant(reason))
        })
        .transpose()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DeformationCursorPhase {
    Skins,
    LogicalMorphs,
    MorphDefinitions,
    Primitives,
    MorphTargets,
    MorphDeltas,
    MeshInstances,
}

/// Linear, allocation-free traversal of every proof-qualified deformation table.
#[must_use = "the deformation cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationCursor<'scene> {
    tables: NodeSceneDeformationTables<'scene>,
    proof: NodeSceneValidationProof,
    phase: DeformationCursorPhase,
    next_skin: usize,
    next_logical_morph: usize,
    next_morph_definition: usize,
    next_primitive: usize,
    next_morph_target: usize,
    next_morph_delta: usize,
    next_mesh_instance: usize,
    joint_cursor: usize,
    inverse_bind_cursor: usize,
    correction_cursor: usize,
    skin_name_cursor: usize,
    definition_cursor: usize,
    morph_default_cursor: usize,
    morph_name_cursor: usize,
    influence_row_offset_cursor: usize,
    influence_cursor: usize,
    target_cursor: usize,
    delta_cursor: usize,
    morph_value_cursor: usize,
    sparse_index_cursor: usize,
    instance_weight_cursor: usize,
}

impl<'scene> NodeSceneDeformationCursor<'scene> {
    pub(in crate::canonical_scene::store) fn skin_count(&self) -> usize {
        self.proof.skin_count()
    }

    pub(in crate::canonical_scene::store) fn logical_morph_count(&self) -> usize {
        self.proof.logical_morph_count()
    }

    pub(in crate::canonical_scene::store) fn morph_definition_count(&self) -> usize {
        self.proof.morph_definition_count()
    }

    pub(in crate::canonical_scene::store) fn primitive_count(&self) -> usize {
        self.proof.primitive_deformation_count()
    }

    pub(in crate::canonical_scene::store) fn morph_target_count(&self) -> usize {
        self.proof.morph_target_count()
    }

    pub(in crate::canonical_scene::store) fn morph_delta_count(&self) -> usize {
        self.proof.morph_delta_count()
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_count(&self) -> usize {
        self.proof.mesh_instance_count()
    }

    pub(in crate::canonical_scene::store) fn next_skin(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationSkin<'scene>>> {
        self.enter_phase(DeformationCursorPhase::Skins)?;
        let Some(row) = next_row(
            self.next_skin,
            self.proof.skin_count(),
            "skin cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .skins
            .ok_or_else(|| invariant("validated deformation skin table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            deformation_skin_field::SKIN,
            "validated skin identity is unreadable",
            "validated skin identity disagrees with its row",
        )?;

        let joint_offset = read_u32_as_usize(
            table,
            row,
            deformation_skin_field::JOINT_OFFSET,
            "validated skin joint offset is unreadable",
        )?;
        let joint_count = read_u32_as_usize(
            table,
            row,
            deformation_skin_field::JOINT_COUNT,
            "validated skin joint count is unreadable",
        )?;
        if joint_offset != self.joint_cursor {
            return Err(invariant("validated skin joint ranges are not contiguous"));
        }
        let joint_values = read_sequence_range(
            table,
            row,
            deformation_skin_field::JOINTS,
            joint_offset,
            joint_count,
            self.proof.deformation_joint_nodes(),
            self.tables.joint_nodes,
            "validated skin joint payload range or reference is inconsistent",
        )?;
        let joints =
            NodeSceneDeformationJointCursor::try_new(joint_values, self.proof.source_node_count())?;

        let inverse_offset = read_u32_as_usize(
            table,
            row,
            deformation_skin_field::INVERSE_BIND_OFFSET,
            "validated inverse-bind offset is unreadable",
        )?;
        let inverse_count = read_u32_as_usize(
            table,
            row,
            deformation_skin_field::INVERSE_BIND_COUNT,
            "validated inverse-bind count is unreadable",
        )?;
        if inverse_offset != self.inverse_bind_cursor || inverse_count != joint_count {
            return Err(invariant(
                "validated inverse-bind range disagrees with its skin palette",
            ));
        }
        let inverse_scalar_count = inverse_count
            .checked_mul(MATRIX_SCALAR_COUNT)
            .ok_or_else(|| invariant("validated inverse-bind scalar count overflows"))?;
        let inverse_values = read_sequence_range(
            table,
            row,
            deformation_skin_field::INVERSE_BINDS,
            inverse_offset,
            inverse_scalar_count,
            self.proof.deformation_inverse_bind_matrices(),
            self.tables.inverse_bind_matrices,
            "validated inverse-bind payload range or reference is inconsistent",
        )?;
        let inverse_bind_matrices = NodeSceneDeformationMatrixCursor::try_new(inverse_values)?;

        let skeleton_root = read_optional_source_node(
            table,
            row,
            deformation_skin_field::SKELETON_ROOT_PRESENCE,
            deformation_skin_field::SKELETON_ROOT,
            self.proof.source_node_count(),
            "validated skeleton-root presence or index is inconsistent",
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_skin_field::NODES,
            joint_count != 0 || skeleton_root.is_some(),
            Some(self.proof.nodes()),
            "validated skin node-table reference is inconsistent",
        )?;

        let matrix_spaces = self.read_skin_matrix_spaces(table, row)?;

        let name_presence = read_scalar::<u8>(
            table,
            row,
            deformation_skin_field::NAME_PRESENCE,
            "validated skin-name presence is unreadable",
        )?;
        let name_offset = read_u32_as_usize(
            table,
            row,
            deformation_skin_field::NAME_OFFSET,
            "validated skin-name offset is unreadable",
        )?;
        let name_count = read_u32_as_usize(
            table,
            row,
            deformation_skin_field::NAME_COUNT,
            "validated skin-name count is unreadable",
        )?;
        let name = match name_presence {
            PRESENCE_ABSENT => {
                if name_offset != 0 || name_count != 0 {
                    return Err(invariant(
                        "validated absent skin name retains a non-empty range",
                    ));
                }
                require_absent_cell(
                    table,
                    row,
                    deformation_skin_field::NAMES,
                    "validated absent skin name retains a payload reference",
                )?;
                None
            }
            PRESENCE_PRESENT => {
                if name_offset != self.skin_name_cursor {
                    return Err(invariant("validated skin-name ranges are not contiguous"));
                }
                Some(read_utf8_range(
                    table,
                    row,
                    deformation_skin_field::NAMES,
                    name_offset,
                    name_count,
                    self.proof.deformation_skin_names(),
                    self.tables.skin_names,
                    "validated skin-name payload range or reference is inconsistent",
                )?)
            }
            _ => return Err(invariant("validated skin-name presence code is unknown")),
        };

        self.joint_cursor = advance_cursor(
            self.joint_cursor,
            joint_count,
            "skin joint cursor overflows",
        )?;
        self.inverse_bind_cursor = advance_cursor(
            self.inverse_bind_cursor,
            inverse_scalar_count,
            "inverse-bind cursor overflows",
        )?;
        if name.is_some() {
            self.skin_name_cursor = advance_cursor(
                self.skin_name_cursor,
                name_count,
                "skin-name cursor overflows",
            )?;
        }
        self.next_skin = advance_cursor(self.next_skin, 1, "skin cursor index overflows")?;
        Ok(Some(NodeSceneDeformationSkin {
            source_index: row,
            name,
            skeleton_root,
            matrix_spaces,
            joints: Some(joints),
            inverse_bind_matrices: Some(inverse_bind_matrices),
        }))
    }

    pub(in crate::canonical_scene::store) fn next_logical_morph(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationLogicalMorph<'scene>>> {
        self.enter_phase(DeformationCursorPhase::LogicalMorphs)?;
        let Some(row) = next_row(
            self.next_logical_morph,
            self.proof.logical_morph_count(),
            "logical-morph cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .logical_morphs
            .ok_or_else(|| invariant("validated logical-morph table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            deformation_logical_morph_field::LOGICAL_MESH,
            "validated logical-morph identity is unreadable",
            "validated logical-morph identity disagrees with its row",
        )?;
        let definition_offset = read_u32_as_usize(
            table,
            row,
            deformation_logical_morph_field::DEFINITION_OFFSET,
            "validated logical-morph definition offset is unreadable",
        )?;
        let definition_count = read_u32_as_usize(
            table,
            row,
            deformation_logical_morph_field::DEFINITION_COUNT,
            "validated logical-morph definition count is unreadable",
        )?;
        if definition_offset != self.definition_cursor {
            return Err(invariant(
                "validated logical-morph definition ranges are not contiguous",
            ));
        }
        let definition_end = checked_end(
            definition_offset,
            definition_count,
            self.proof.morph_definition_count(),
            "validated logical-morph definition range is inconsistent",
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_logical_morph_field::DEFINITIONS,
            definition_count != 0,
            self.proof.deformation_morph_definitions(),
            "validated logical-morph definition-table reference is inconsistent",
        )?;

        let default_offset = read_u32_as_usize(
            table,
            row,
            deformation_logical_morph_field::DEFAULT_WEIGHT_OFFSET,
            "validated logical-morph default offset is unreadable",
        )?;
        let default_count = read_u32_as_usize(
            table,
            row,
            deformation_logical_morph_field::DEFAULT_WEIGHT_COUNT,
            "validated logical-morph default count is unreadable",
        )?;
        if default_offset != self.morph_default_cursor || default_count != definition_count {
            return Err(invariant(
                "validated logical-morph defaults disagree with its target definitions",
            ));
        }
        let default_values = read_sequence_range(
            table,
            row,
            deformation_logical_morph_field::DEFAULT_WEIGHTS,
            default_offset,
            default_count,
            self.proof.deformation_morph_defaults(),
            self.tables.morph_defaults,
            "validated logical-morph default payload range or reference is inconsistent",
        )?;
        if default_values.iter().any(|weight| !weight.is_finite()) {
            return Err(invariant(
                "validated logical-morph defaults contain non-finite data",
            ));
        }

        self.definition_cursor = definition_end;
        self.morph_default_cursor = advance_cursor(
            self.morph_default_cursor,
            default_count,
            "morph-default cursor overflows",
        )?;
        self.next_logical_morph = advance_cursor(
            self.next_logical_morph,
            1,
            "logical-morph cursor index overflows",
        )?;
        Ok(Some(NodeSceneDeformationLogicalMorph {
            source_index: row,
            definition_offset,
            definition_count,
            default_weights: Some(NodeSceneDeformationF32Cursor::new(default_values)),
        }))
    }

    pub(in crate::canonical_scene::store) fn next_morph_definition(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationMorphDefinition<'scene>>> {
        self.enter_phase(DeformationCursorPhase::MorphDefinitions)?;
        let Some(row) = next_row(
            self.next_morph_definition,
            self.proof.morph_definition_count(),
            "morph-definition cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .morph_definitions
            .ok_or_else(|| invariant("validated morph-definition table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            deformation_morph_definition_field::MORPH_DEFINITION,
            "validated morph-definition identity is unreadable",
            "validated morph-definition identity disagrees with its row",
        )?;
        let logical_mesh = read_u32_as_usize(
            table,
            row,
            deformation_morph_definition_field::LOGICAL_MESH,
            "validated morph-definition logical mesh is unreadable",
        )?;
        let target_ordinal = read_u32_as_usize(
            table,
            row,
            deformation_morph_definition_field::TARGET_ORDINAL,
            "validated morph-definition target ordinal is unreadable",
        )?;
        let stable_identity = read_scalar::<u32>(
            table,
            row,
            deformation_morph_definition_field::STABLE_IDENTITY,
            "validated morph-definition stable identity is unreadable",
        )?;
        let logical_table = self
            .tables
            .logical_morphs
            .ok_or_else(|| invariant("validated logical-morph table view is absent"))?;
        if logical_mesh >= self.proof.logical_morph_count() {
            return Err(invariant(
                "validated morph-definition logical mesh is outside proof-qualified space",
            ));
        }
        let owner_offset = read_u32_as_usize(
            logical_table,
            logical_mesh,
            deformation_logical_morph_field::DEFINITION_OFFSET,
            "validated owner definition offset is unreadable",
        )?;
        let owner_count = read_u32_as_usize(
            logical_table,
            logical_mesh,
            deformation_logical_morph_field::DEFINITION_COUNT,
            "validated owner definition count is unreadable",
        )?;
        let expected_row = owner_offset
            .checked_add(target_ordinal)
            .ok_or_else(|| invariant("validated morph-definition ownership overflows"))?;
        if target_ordinal >= owner_count
            || expected_row != row
            || usize::try_from(stable_identity)
                .ok()
                .is_none_or(|identity| identity >= owner_count)
        {
            return Err(invariant(
                "validated morph-definition ownership or stable identity is inconsistent",
            ));
        }
        let name_presence = read_scalar::<u8>(
            table,
            row,
            deformation_morph_definition_field::NAME_PRESENCE,
            "validated morph-name presence is unreadable",
        )?;
        let name_offset = read_u32_as_usize(
            table,
            row,
            deformation_morph_definition_field::NAME_OFFSET,
            "validated morph-name offset is unreadable",
        )?;
        let name_count = read_u32_as_usize(
            table,
            row,
            deformation_morph_definition_field::NAME_COUNT,
            "validated morph-name count is unreadable",
        )?;
        let name = match name_presence {
            PRESENCE_ABSENT => {
                if name_offset != 0 || name_count != 0 {
                    return Err(invariant(
                        "validated absent morph name retains a non-empty range",
                    ));
                }
                require_absent_cell(
                    table,
                    row,
                    deformation_morph_definition_field::NAMES,
                    "validated absent morph name retains a payload reference",
                )?;
                None
            }
            PRESENCE_PRESENT => {
                if name_offset != self.morph_name_cursor {
                    return Err(invariant("validated morph-name ranges are not contiguous"));
                }
                Some(read_utf8_range(
                    table,
                    row,
                    deformation_morph_definition_field::NAMES,
                    name_offset,
                    name_count,
                    self.proof.deformation_morph_names(),
                    self.tables.morph_names,
                    "validated morph-name payload range or reference is inconsistent",
                )?)
            }
            _ => return Err(invariant("validated morph-name presence code is unknown")),
        };
        if name.is_some() {
            self.morph_name_cursor = advance_cursor(
                self.morph_name_cursor,
                name_count,
                "morph-name cursor overflows",
            )?;
        }
        self.next_morph_definition = advance_cursor(
            self.next_morph_definition,
            1,
            "morph-definition cursor index overflows",
        )?;
        Ok(Some(NodeSceneDeformationMorphDefinition {
            source_index: row,
            logical_mesh,
            target_ordinal,
            stable_identity,
            name,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_primitive(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationPrimitive<'scene>>> {
        self.enter_phase(DeformationCursorPhase::Primitives)?;
        let Some(row) = next_row(
            self.next_primitive,
            self.proof.primitive_deformation_count(),
            "primitive-deformation cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .primitives
            .ok_or_else(|| invariant("validated primitive-deformation table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            deformation_primitive_field::PRIMITIVE,
            "validated primitive-deformation identity is unreadable",
            "validated primitive-deformation identity disagrees with its row",
        )?;
        let influence_presence = read_scalar::<u8>(
            table,
            row,
            deformation_primitive_field::INFLUENCE_PRESENCE,
            "validated influence presence is unreadable",
        )?;
        let row_offset = read_u32_as_usize(
            table,
            row,
            deformation_primitive_field::ROW_OFFSET_OFFSET,
            "validated influence-row offset is unreadable",
        )?;
        let row_count = read_u32_as_usize(
            table,
            row,
            deformation_primitive_field::ROW_OFFSET_COUNT,
            "validated influence-row count is unreadable",
        )?;
        let influence_offset = read_u32_as_usize(
            table,
            row,
            deformation_primitive_field::INFLUENCE_OFFSET,
            "validated influence offset is unreadable",
        )?;
        let influence_count = read_u32_as_usize(
            table,
            row,
            deformation_primitive_field::INFLUENCE_COUNT,
            "validated influence count is unreadable",
        )?;
        let influences = match influence_presence {
            PRESENCE_ABSENT => {
                if row_offset != 0
                    || row_count != 0
                    || influence_offset != 0
                    || influence_count != 0
                {
                    return Err(invariant(
                        "validated absent influence rows retain payload ranges",
                    ));
                }
                for field in [
                    deformation_primitive_field::ROW_OFFSETS,
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                ] {
                    require_absent_cell(
                        table,
                        row,
                        field,
                        "validated absent influence rows retain a payload reference",
                    )?;
                }
                None
            }
            PRESENCE_PRESENT => {
                if row_offset != self.influence_row_offset_cursor
                    || influence_offset != self.influence_cursor
                {
                    return Err(invariant(
                        "validated influence payload ranges are not contiguous",
                    ));
                }
                let offsets = read_sequence_range(
                    table,
                    row,
                    deformation_primitive_field::ROW_OFFSETS,
                    row_offset,
                    row_count,
                    self.proof.deformation_influence_row_offsets(),
                    self.tables.influence_row_offsets,
                    "validated influence-row payload range or reference is inconsistent",
                )?;
                let joints = read_sequence_range(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    influence_offset,
                    influence_count,
                    self.proof.deformation_influence_joints(),
                    self.tables.influence_joints,
                    "validated influence-joint payload range or reference is inconsistent",
                )?;
                let weights = read_sequence_range(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    influence_offset,
                    influence_count,
                    self.proof.deformation_influence_weights(),
                    self.tables.influence_weights,
                    "validated influence-weight payload range or reference is inconsistent",
                )?;
                let source_orders = read_sequence_range(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                    influence_offset,
                    influence_count,
                    self.proof.deformation_influence_source_orders(),
                    self.tables.influence_source_orders,
                    "validated influence source-order payload range or reference is inconsistent",
                )?;
                Some(NodeSceneDeformationInfluenceRowsCursor::try_new(
                    offsets,
                    joints,
                    weights,
                    source_orders,
                )?)
            }
            _ => return Err(invariant("validated influence presence code is unknown")),
        };

        let morph_target_offset = read_u32_as_usize(
            table,
            row,
            deformation_primitive_field::MORPH_TARGET_OFFSET,
            "validated primitive morph-target offset is unreadable",
        )?;
        let morph_target_count = read_u32_as_usize(
            table,
            row,
            deformation_primitive_field::MORPH_TARGET_COUNT,
            "validated primitive morph-target count is unreadable",
        )?;
        if morph_target_offset != self.target_cursor {
            return Err(invariant(
                "validated primitive morph-target ranges are not contiguous",
            ));
        }
        let morph_target_end = checked_end(
            morph_target_offset,
            morph_target_count,
            self.proof.morph_target_count(),
            "validated primitive morph-target range is inconsistent",
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_primitive_field::MORPH_TARGETS,
            morph_target_count != 0,
            self.proof.deformation_morph_targets(),
            "validated primitive morph-target table reference is inconsistent",
        )?;

        if influence_presence == PRESENCE_PRESENT {
            self.influence_row_offset_cursor = advance_cursor(
                self.influence_row_offset_cursor,
                row_count,
                "influence-row-offset cursor overflows",
            )?;
            self.influence_cursor = advance_cursor(
                self.influence_cursor,
                influence_count,
                "influence cursor overflows",
            )?;
        }
        self.target_cursor = morph_target_end;
        self.next_primitive = advance_cursor(
            self.next_primitive,
            1,
            "primitive-deformation cursor index overflows",
        )?;
        Ok(Some(NodeSceneDeformationPrimitive {
            source_index: row,
            influences,
            morph_target_offset,
            morph_target_count,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_morph_target(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationMorphTarget>> {
        self.enter_phase(DeformationCursorPhase::MorphTargets)?;
        let Some(row) = next_row(
            self.next_morph_target,
            self.proof.morph_target_count(),
            "morph-target cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .morph_targets
            .ok_or_else(|| invariant("validated morph-target table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            deformation_morph_target_field::MORPH_TARGET,
            "validated morph-target identity is unreadable",
            "validated morph-target identity disagrees with its row",
        )?;
        let primitive = read_u32_as_usize(
            table,
            row,
            deformation_morph_target_field::PRIMITIVE,
            "validated morph-target primitive is unreadable",
        )?;
        let target_ordinal = read_u32_as_usize(
            table,
            row,
            deformation_morph_target_field::TARGET_ORDINAL,
            "validated morph-target ordinal is unreadable",
        )?;
        if primitive >= self.proof.primitive_deformation_count() {
            return Err(invariant(
                "validated morph-target primitive is outside proof-qualified space",
            ));
        }
        let primitive_table = self
            .tables
            .primitives
            .ok_or_else(|| invariant("validated primitive-deformation table view is absent"))?;
        let owner_offset = read_u32_as_usize(
            primitive_table,
            primitive,
            deformation_primitive_field::MORPH_TARGET_OFFSET,
            "validated owner morph-target offset is unreadable",
        )?;
        let owner_count = read_u32_as_usize(
            primitive_table,
            primitive,
            deformation_primitive_field::MORPH_TARGET_COUNT,
            "validated owner morph-target count is unreadable",
        )?;
        if target_ordinal >= owner_count || owner_offset.checked_add(target_ordinal) != Some(row) {
            return Err(invariant(
                "validated morph-target ownership is inconsistent",
            ));
        }

        let delta_offset = read_u32_as_usize(
            table,
            row,
            deformation_morph_target_field::DELTA_OFFSET,
            "validated morph-target delta offset is unreadable",
        )?;
        let delta_count = read_u32_as_usize(
            table,
            row,
            deformation_morph_target_field::DELTA_COUNT,
            "validated morph-target delta count is unreadable",
        )?;
        if delta_offset != self.delta_cursor {
            return Err(invariant(
                "validated morph-target delta ranges are not contiguous",
            ));
        }
        let delta_end = checked_end(
            delta_offset,
            delta_count,
            self.proof.morph_delta_count(),
            "validated morph-target delta range is inconsistent",
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_morph_target_field::DELTAS,
            delta_count != 0,
            self.proof.deformation_morph_deltas(),
            "validated morph-target delta-table reference is inconsistent",
        )?;
        self.delta_cursor = delta_end;
        self.next_morph_target = advance_cursor(
            self.next_morph_target,
            1,
            "morph-target cursor index overflows",
        )?;
        Ok(Some(NodeSceneDeformationMorphTarget {
            source_index: row,
            primitive,
            target_ordinal,
            delta_offset,
            delta_count,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_morph_delta(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationMorphDelta<'scene>>> {
        self.enter_phase(DeformationCursorPhase::MorphDeltas)?;
        let Some(row) = next_row(
            self.next_morph_delta,
            self.proof.morph_delta_count(),
            "morph-delta cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .morph_deltas
            .ok_or_else(|| invariant("validated morph-delta table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            deformation_morph_delta_field::MORPH_DELTA,
            "validated morph-delta identity is unreadable",
            "validated morph-delta identity disagrees with its row",
        )?;

        let morph_target = read_u32_as_usize(
            table,
            row,
            deformation_morph_delta_field::MORPH_TARGET,
            "validated morph-delta target is unreadable",
        )?;
        if morph_target >= self.proof.morph_target_count() {
            return Err(invariant(
                "validated morph-delta target is outside proof-qualified space",
            ));
        }
        let target_table = self
            .tables
            .morph_targets
            .ok_or_else(|| invariant("validated morph-target table view is absent"))?;
        let owner_offset = read_u32_as_usize(
            target_table,
            morph_target,
            deformation_morph_target_field::DELTA_OFFSET,
            "validated owner morph-delta offset is unreadable",
        )?;
        let owner_count = read_u32_as_usize(
            target_table,
            morph_target,
            deformation_morph_target_field::DELTA_COUNT,
            "validated owner morph-delta count is unreadable",
        )?;
        let owner_end = checked_end(
            owner_offset,
            owner_count,
            self.proof.morph_delta_count(),
            "validated owner morph-delta range is inconsistent",
        )?;
        if row < owner_offset || row >= owner_end {
            return Err(invariant("validated morph-delta ownership is inconsistent"));
        }

        let semantic_code = read_scalar::<u8>(
            table,
            row,
            deformation_morph_delta_field::SEMANTIC,
            "validated morph-delta semantic is unreadable",
        )?;
        let semantic = decode_morph_semantic(semantic_code)?;
        let payload_kind = decode_morph_payload(read_scalar::<u8>(
            table,
            row,
            deformation_morph_delta_field::PAYLOAD_KIND,
            "validated morph-delta payload kind is unreadable",
        )?)?;
        let logical_count = read_u32_as_usize(
            table,
            row,
            deformation_morph_delta_field::LOGICAL_COUNT,
            "validated morph-delta logical count is unreadable",
        )?;
        let explicit_count = read_u32_as_usize(
            table,
            row,
            deformation_morph_delta_field::EXPLICIT_COUNT,
            "validated morph-delta explicit count is unreadable",
        )?;
        let value_offset = read_u32_as_usize(
            table,
            row,
            deformation_morph_delta_field::VALUE_OFFSET,
            "validated morph-delta value offset is unreadable",
        )?;
        let value_count = read_u32_as_usize(
            table,
            row,
            deformation_morph_delta_field::VALUE_COUNT,
            "validated morph-delta value count is unreadable",
        )?;
        let expected_value_count = explicit_count
            .checked_mul(MORPH_VECTOR_WIDTH)
            .ok_or_else(|| invariant("validated morph-delta value count overflows"))?;
        if value_offset != self.morph_value_cursor || value_count != expected_value_count {
            return Err(invariant(
                "validated morph-delta value range is not contiguous and exact",
            ));
        }
        let values = read_sequence_range(
            table,
            row,
            deformation_morph_delta_field::VALUES,
            value_offset,
            value_count,
            self.proof.deformation_morph_values(),
            self.tables.morph_values,
            "validated morph-delta value range or reference is inconsistent",
        )?;

        let sparse_offset = read_u32_as_usize(
            table,
            row,
            deformation_morph_delta_field::SPARSE_INDEX_OFFSET,
            "validated morph-delta sparse-index offset is unreadable",
        )?;
        let sparse_count = read_u32_as_usize(
            table,
            row,
            deformation_morph_delta_field::SPARSE_INDEX_COUNT,
            "validated morph-delta sparse-index count is unreadable",
        )?;
        if sparse_offset != self.sparse_index_cursor {
            return Err(invariant(
                "validated morph-delta sparse-index ranges are not contiguous",
            ));
        }
        let sparse_indices = read_sequence_range(
            table,
            row,
            deformation_morph_delta_field::SPARSE_INDICES,
            sparse_offset,
            sparse_count,
            self.proof.deformation_morph_sparse_indices(),
            self.tables.morph_sparse_indices,
            "validated morph-delta sparse-index range or reference is inconsistent",
        )?;

        let payload = match payload_kind {
            NodeSceneDeformationMorphPayloadKind::Dense => {
                if explicit_count != logical_count || sparse_count != 0 {
                    return Err(invariant(
                        "validated dense morph delta has inconsistent counts",
                    ));
                }
                NodeSceneDeformationMorphPayload::Dense {
                    logical_count,
                    values: NodeSceneDeformationVec3Cursor::try_new(values)?,
                }
            }
            NodeSceneDeformationMorphPayloadKind::Sparse => {
                if sparse_count != explicit_count {
                    return Err(invariant(
                        "validated sparse morph delta has inconsistent counts",
                    ));
                }
                NodeSceneDeformationMorphPayload::Sparse {
                    logical_count,
                    values: NodeSceneDeformationSparseDeltaCursor::try_new(
                        logical_count,
                        sparse_indices,
                        values,
                    )?,
                }
            }
        };

        self.morph_value_cursor = advance_cursor(
            self.morph_value_cursor,
            value_count,
            "morph-value cursor overflows",
        )?;
        self.sparse_index_cursor = advance_cursor(
            self.sparse_index_cursor,
            sparse_count,
            "morph sparse-index cursor overflows",
        )?;
        self.next_morph_delta = advance_cursor(
            self.next_morph_delta,
            1,
            "morph-delta cursor index overflows",
        )?;
        Ok(Some(NodeSceneDeformationMorphDelta {
            source_index: row,
            morph_target,
            semantic,
            payload,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_mesh_instance(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationMeshInstance<'scene>>> {
        self.enter_phase(DeformationCursorPhase::MeshInstances)?;
        let Some(row) = next_row(
            self.next_mesh_instance,
            self.proof.mesh_instance_count(),
            "deformation mesh-instance cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .mesh_instances
            .ok_or_else(|| invariant("validated deformation mesh-instance table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            deformation_mesh_instance_field::MESH_INSTANCE,
            "validated deformation mesh-instance identity is unreadable",
            "validated deformation mesh-instance identity disagrees with its row",
        )?;

        let skin_presence = read_scalar::<u8>(
            table,
            row,
            deformation_mesh_instance_field::SKIN_PRESENCE,
            "validated mesh-instance skin presence is unreadable",
        )?;
        let stored_skin = read_scalar::<u32>(
            table,
            row,
            deformation_mesh_instance_field::SKIN,
            "validated mesh-instance skin identity is unreadable",
        )?;
        let skin = match skin_presence {
            PRESENCE_ABSENT => {
                if stored_skin != 0 {
                    return Err(invariant(
                        "validated absent mesh-instance skin has a nonzero identity",
                    ));
                }
                require_absent_cell(
                    table,
                    row,
                    deformation_mesh_instance_field::SKINS,
                    "validated absent mesh-instance skin retains a table reference",
                )?;
                None
            }
            PRESENCE_PRESENT => {
                let skin = usize::try_from(stored_skin)
                    .map_err(|_| invariant("validated mesh-instance skin cannot address memory"))?;
                if skin >= self.proof.skin_count() {
                    return Err(invariant(
                        "validated mesh-instance skin is outside proof-qualified space",
                    ));
                }
                require_optional_record_reference(
                    table,
                    row,
                    deformation_mesh_instance_field::SKINS,
                    true,
                    self.proof.deformation_skins(),
                    "validated mesh-instance skin-table reference is inconsistent",
                )?;
                Some(skin)
            }
            _ => {
                return Err(invariant(
                    "validated mesh-instance skin presence code is unknown",
                ));
            }
        };

        let weight_presence = read_scalar::<u8>(
            table,
            row,
            deformation_mesh_instance_field::MORPH_WEIGHTS_PRESENCE,
            "validated instance morph-weight presence is unreadable",
        )?;
        let weight_offset = read_u32_as_usize(
            table,
            row,
            deformation_mesh_instance_field::MORPH_WEIGHT_OFFSET,
            "validated instance morph-weight offset is unreadable",
        )?;
        let weight_count = read_u32_as_usize(
            table,
            row,
            deformation_mesh_instance_field::MORPH_WEIGHT_COUNT,
            "validated instance morph-weight count is unreadable",
        )?;
        let morph_weights = match weight_presence {
            PRESENCE_ABSENT => {
                if weight_offset != 0 || weight_count != 0 {
                    return Err(invariant(
                        "validated absent instance morph weights retain a range",
                    ));
                }
                require_absent_cell(
                    table,
                    row,
                    deformation_mesh_instance_field::MORPH_WEIGHTS,
                    "validated absent instance morph weights retain a payload reference",
                )?;
                None
            }
            PRESENCE_PRESENT => {
                if weight_offset != self.instance_weight_cursor {
                    return Err(invariant(
                        "validated instance morph-weight ranges are not contiguous",
                    ));
                }
                let weights = read_sequence_range(
                    table,
                    row,
                    deformation_mesh_instance_field::MORPH_WEIGHTS,
                    weight_offset,
                    weight_count,
                    self.proof.deformation_instance_morph_weights(),
                    self.tables.instance_morph_weights,
                    "validated instance morph-weight range or reference is inconsistent",
                )?;
                if weights.iter().any(|weight| !weight.is_finite()) {
                    return Err(invariant(
                        "validated instance morph weights contain non-finite data",
                    ));
                }
                Some(NodeSceneDeformationF32Cursor::new(weights))
            }
            _ => {
                return Err(invariant(
                    "validated instance morph-weight presence code is unknown",
                ));
            }
        };

        if morph_weights.is_some() {
            self.instance_weight_cursor = advance_cursor(
                self.instance_weight_cursor,
                weight_count,
                "instance morph-weight cursor overflows",
            )?;
        }
        self.next_mesh_instance = advance_cursor(
            self.next_mesh_instance,
            1,
            "deformation mesh-instance cursor index overflows",
        )?;
        Ok(Some(NodeSceneDeformationMeshInstance {
            source_index: row,
            skin,
            morph_weights,
        }))
    }

    fn read_skin_matrix_spaces(
        &mut self,
        table: CanonicalRecordView<'scene>,
        row: usize,
    ) -> NodeSceneSemanticResult<SkinMatrixSpaces> {
        let canonical = SkinMatrixSpaces::canonical();
        for (field, expected) in [
            (
                deformation_skin_field::INPUT_POSITION_SPACE,
                canonical.input_position(),
            ),
            (
                deformation_skin_field::INVERSE_BIND_FROM_SPACE,
                canonical.inverse_bind_from(),
            ),
            (
                deformation_skin_field::INVERSE_BIND_TO_SPACE,
                canonical.inverse_bind_to(),
            ),
            (
                deformation_skin_field::CURRENT_JOINT_FROM_SPACE,
                canonical.current_joint_from(),
            ),
            (
                deformation_skin_field::CURRENT_JOINT_TO_SPACE,
                canonical.current_joint_to(),
            ),
            (
                deformation_skin_field::OUTPUT_POSITION_SPACE,
                canonical.output_position(),
            ),
        ] {
            let actual = decode_skin_space(read_scalar::<u8>(
                table,
                row,
                field,
                "validated skin matrix-space code is unreadable",
            )?)?;
            if actual != expected {
                return Err(invariant(
                    "validated skin matrix-space code is not canonical",
                ));
            }
        }

        let presence = read_scalar::<u8>(
            table,
            row,
            deformation_skin_field::CORRECTION_PRESENCE,
            "validated skin-correction presence is unreadable",
        )?;
        let from_code = read_scalar::<u8>(
            table,
            row,
            deformation_skin_field::CORRECTION_FROM_SPACE,
            "validated skin-correction source space is unreadable",
        )?;
        let to_code = read_scalar::<u8>(
            table,
            row,
            deformation_skin_field::CORRECTION_TO_SPACE,
            "validated skin-correction destination space is unreadable",
        )?;
        let offset = read_u32_as_usize(
            table,
            row,
            deformation_skin_field::CORRECTION_MATRIX_OFFSET,
            "validated skin-correction offset is unreadable",
        )?;
        let count = read_u32_as_usize(
            table,
            row,
            deformation_skin_field::CORRECTION_MATRIX_COUNT,
            "validated skin-correction count is unreadable",
        )?;
        match presence {
            PRESENCE_ABSENT => {
                if from_code != 0 || to_code != 0 || offset != 0 || count != 0 {
                    return Err(invariant(
                        "validated absent skin correction retains payload metadata",
                    ));
                }
                require_absent_cell(
                    table,
                    row,
                    deformation_skin_field::CORRECTION_MATRICES,
                    "validated absent skin correction retains a payload reference",
                )?;
                Ok(canonical)
            }
            PRESENCE_PRESENT => {
                if offset != self.correction_cursor || count != MATRIX_SCALAR_COUNT {
                    return Err(invariant(
                        "validated skin-correction range is not contiguous and exact",
                    ));
                }
                let from = decode_skin_space(from_code)?;
                let to = decode_skin_space(to_code)?;
                if from != canonical.input_position() || to != canonical.output_position() {
                    return Err(invariant(
                        "validated skin-correction spaces disagree with the canonical equation",
                    ));
                }
                let values = read_sequence_range(
                    table,
                    row,
                    deformation_skin_field::CORRECTION_MATRICES,
                    offset,
                    count,
                    self.proof.deformation_skin_correction_matrices(),
                    self.tables.skin_correction_matrices,
                    "validated skin-correction payload range or reference is inconsistent",
                )?;
                let matrix = matrix_from_slice(values)?;
                if !matrix.is_finite() || !matrix.is_affine() {
                    return Err(invariant(
                        "validated skin-correction matrix is not finite affine data",
                    ));
                }
                let correction = SkinMatrixCorrection::new(from, to, matrix).map_err(|_| {
                    invariant("validated skin-correction matrix cannot be reconstructed")
                })?;
                self.correction_cursor = advance_cursor(
                    self.correction_cursor,
                    count,
                    "skin-correction cursor overflows",
                )?;
                Ok(canonical.with_correction(correction))
            }
            _ => Err(invariant(
                "validated skin-correction presence code is unknown",
            )),
        }
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next_skin != self.proof.skin_count()
            || self.next_logical_morph != self.proof.logical_morph_count()
            || self.next_morph_definition != self.proof.morph_definition_count()
            || self.next_primitive != self.proof.primitive_deformation_count()
            || self.next_morph_target != self.proof.morph_target_count()
            || self.next_morph_delta != self.proof.morph_delta_count()
            || self.next_mesh_instance != self.proof.mesh_instance_count()
        {
            return Err(invariant(
                "deformation cursor ended before its proof-qualified table counts",
            ));
        }
        require_payload_exhausted(
            self.joint_cursor,
            self.tables.joint_nodes,
            "skin joint-node payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.inverse_bind_cursor,
            self.tables.inverse_bind_matrices,
            "inverse-bind payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.correction_cursor,
            self.tables.skin_correction_matrices,
            "skin-correction payload was not consumed exactly",
        )?;
        require_text_exhausted(
            self.skin_name_cursor,
            self.tables.skin_names,
            "skin-name payload was not consumed exactly",
        )?;
        if self.definition_cursor != self.proof.morph_definition_count()
            || self.target_cursor != self.proof.morph_target_count()
            || self.delta_cursor != self.proof.morph_delta_count()
        {
            return Err(invariant(
                "deformation relation ranges were not consumed exactly",
            ));
        }
        require_payload_exhausted(
            self.morph_default_cursor,
            self.tables.morph_defaults,
            "morph-default payload was not consumed exactly",
        )?;
        require_text_exhausted(
            self.morph_name_cursor,
            self.tables.morph_names,
            "morph-name payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.influence_row_offset_cursor,
            self.tables.influence_row_offsets,
            "influence-row-offset payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.influence_cursor,
            self.tables.influence_joints,
            "influence-joint payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.influence_cursor,
            self.tables.influence_weights,
            "influence-weight payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.influence_cursor,
            self.tables.influence_source_orders,
            "influence source-order payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.morph_value_cursor,
            self.tables.morph_values,
            "morph-value payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.sparse_index_cursor,
            self.tables.morph_sparse_indices,
            "morph sparse-index payload was not consumed exactly",
        )?;
        require_payload_exhausted(
            self.instance_weight_cursor,
            self.tables.instance_morph_weights,
            "instance morph-weight payload was not consumed exactly",
        )
    }

    fn enter_phase(&mut self, target: DeformationCursorPhase) -> NodeSceneSemanticResult<()> {
        if target < self.phase {
            return Err(invariant(
                "deformation cursor cannot return to an earlier table phase",
            ));
        }
        if target > DeformationCursorPhase::Skins && self.next_skin != self.proof.skin_count() {
            return Err(invariant(
                "skin table must be exhausted before later deformation tables",
            ));
        }
        if target > DeformationCursorPhase::LogicalMorphs
            && self.next_logical_morph != self.proof.logical_morph_count()
        {
            return Err(invariant(
                "logical-morph table must be exhausted before later deformation tables",
            ));
        }
        if target > DeformationCursorPhase::MorphDefinitions
            && self.next_morph_definition != self.proof.morph_definition_count()
        {
            return Err(invariant(
                "morph-definition table must be exhausted before later deformation tables",
            ));
        }
        if target > DeformationCursorPhase::Primitives
            && self.next_primitive != self.proof.primitive_deformation_count()
        {
            return Err(invariant(
                "primitive-deformation table must be exhausted before later deformation tables",
            ));
        }
        if target > DeformationCursorPhase::MorphTargets
            && self.next_morph_target != self.proof.morph_target_count()
        {
            return Err(invariant(
                "morph-target table must be exhausted before later deformation tables",
            ));
        }
        if target > DeformationCursorPhase::MorphDeltas
            && self.next_morph_delta != self.proof.morph_delta_count()
        {
            return Err(invariant(
                "morph-delta table must be exhausted before mesh instances",
            ));
        }
        self.phase = target;
        Ok(())
    }
}

/// One skin with exact palette, bind matrices, spaces, root, and optional name.
#[must_use = "the skin payload cursors must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationSkin<'scene> {
    source_index: usize,
    name: Option<&'scene str>,
    skeleton_root: Option<usize>,
    matrix_spaces: SkinMatrixSpaces,
    joints: Option<NodeSceneDeformationJointCursor<'scene>>,
    inverse_bind_matrices: Option<NodeSceneDeformationMatrixCursor<'scene>>,
}

/// Logical morph ownership and its default weight vector.
#[must_use = "the logical-morph default cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationLogicalMorph<'scene> {
    source_index: usize,
    definition_offset: usize,
    definition_count: usize,
    default_weights: Option<NodeSceneDeformationF32Cursor<'scene>>,
}

/// One primitive's compressed influence rows and logical-target range.
#[must_use = "the primitive influence cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationPrimitive<'scene> {
    source_index: usize,
    influences: Option<NodeSceneDeformationInfluenceRowsCursor<'scene>>,
    morph_target_offset: usize,
    morph_target_count: usize,
}

impl<'scene> NodeSceneDeformationPrimitive<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn influences_present(&self) -> bool {
        self.influences.is_some()
    }

    pub(in crate::canonical_scene::store) fn take_influences(
        &mut self,
    ) -> Option<NodeSceneDeformationInfluenceRowsCursor<'scene>> {
        self.influences.take()
    }

    pub(in crate::canonical_scene::store) const fn morph_target_offset(&self) -> usize {
        self.morph_target_offset
    }

    pub(in crate::canonical_scene::store) const fn morph_target_count(&self) -> usize {
        self.morph_target_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMorphTarget {
    source_index: usize,
    primitive: usize,
    target_ordinal: usize,
    delta_offset: usize,
    delta_count: usize,
}

impl NodeSceneDeformationMorphTarget {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn primitive(self) -> usize {
        self.primitive
    }

    pub(in crate::canonical_scene::store) const fn target_ordinal(self) -> usize {
        self.target_ordinal
    }

    pub(in crate::canonical_scene::store) const fn delta_offset(self) -> usize {
        self.delta_offset
    }

    pub(in crate::canonical_scene::store) const fn delta_count(self) -> usize {
        self.delta_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeSceneDeformationMorphPayloadKind {
    Dense,
    Sparse,
}

/// One relative morph stream and its proof-qualified source attachment.
#[must_use = "the morph-delta payload cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMorphDelta<'scene> {
    source_index: usize,
    morph_target: usize,
    semantic: MorphDeltaSemantic,
    payload: NodeSceneDeformationMorphPayload<'scene>,
}

impl<'scene> NodeSceneDeformationMorphDelta<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn morph_target(&self) -> usize {
        self.morph_target
    }

    pub(in crate::canonical_scene::store) const fn semantic(&self) -> MorphDeltaSemantic {
        self.semantic
    }

    pub(in crate::canonical_scene::store) fn into_payload(
        self,
    ) -> NodeSceneDeformationMorphPayload<'scene> {
        self.payload
    }
}

/// Dense or sparse finite F32x3 values for one relative morph stream.
#[must_use = "the morph payload cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) enum NodeSceneDeformationMorphPayload<'scene> {
    Dense {
        logical_count: usize,
        values: NodeSceneDeformationVec3Cursor<'scene>,
    },
    Sparse {
        logical_count: usize,
        values: NodeSceneDeformationSparseDeltaCursor<'scene>,
    },
}

impl<'scene> NodeSceneDeformationMorphPayload<'scene> {
    pub(in crate::canonical_scene::store) const fn logical_count(&self) -> usize {
        match self {
            Self::Dense { logical_count, .. } | Self::Sparse { logical_count, .. } => {
                *logical_count
            }
        }
    }

    pub(in crate::canonical_scene::store) const fn explicit_count(&self) -> usize {
        match self {
            Self::Dense { values, .. } => values.declared_count(),
            Self::Sparse { values, .. } => values.declared_count(),
        }
    }
}

/// Linear cursor over finite dense F32x3 morph vectors.
#[must_use = "the morph-vector cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationVec3Cursor<'scene> {
    scalars: &'scene [f32],
    next_vector: usize,
}

impl<'scene> NodeSceneDeformationVec3Cursor<'scene> {
    fn try_new(scalars: &'scene [f32]) -> NodeSceneSemanticResult<Self> {
        if !scalars.len().is_multiple_of(MORPH_VECTOR_WIDTH)
            || scalars.iter().any(|value| !value.is_finite())
        {
            return Err(invariant(
                "validated morph-vector payload is not finite F32x3 data",
            ));
        }
        Ok(Self {
            scalars,
            next_vector: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.scalars.len() / MORPH_VECTOR_WIDTH
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<Vec3>> {
        if self.next_vector == self.declared_count() {
            return Ok(None);
        }
        let start = self
            .next_vector
            .checked_mul(MORPH_VECTOR_WIDTH)
            .ok_or_else(|| invariant("morph-vector cursor scalar offset overflows"))?;
        let end = start
            .checked_add(MORPH_VECTOR_WIDTH)
            .ok_or_else(|| invariant("morph-vector cursor scalar end overflows"))?;
        let values = self
            .scalars
            .get(start..end)
            .ok_or_else(|| invariant("morph-vector cursor advanced beyond its range"))?;
        let component = |index| {
            values
                .get(index)
                .copied()
                .ok_or_else(|| invariant("morph-vector component is unavailable"))
        };
        let value = Vec3::new(component(0)?, component(1)?, component(2)?);
        if !value.is_finite() {
            return Err(invariant(
                "validated morph-vector payload contains non-finite data",
            ));
        }
        self.next_vector =
            advance_cursor(self.next_vector, 1, "morph-vector cursor index overflows")?;
        Ok(Some(value))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        (self.next_vector == self.declared_count())
            .then_some(())
            .ok_or_else(|| invariant("morph-vector cursor ended before its declared count"))
    }
}

/// Linear cursor over strictly ordered sparse morph vectors.
#[must_use = "the sparse morph cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationSparseDeltaCursor<'scene> {
    logical_count: usize,
    indices: &'scene [u32],
    values: NodeSceneDeformationVec3Cursor<'scene>,
    next: usize,
}

impl<'scene> NodeSceneDeformationSparseDeltaCursor<'scene> {
    fn try_new(
        logical_count: usize,
        indices: &'scene [u32],
        scalars: &'scene [f32],
    ) -> NodeSceneSemanticResult<Self> {
        let values = NodeSceneDeformationVec3Cursor::try_new(scalars)?;
        if values.declared_count() != indices.len() {
            return Err(invariant(
                "validated sparse morph indices and values have different lengths",
            ));
        }
        let mut previous = None;
        for index in indices.iter().copied() {
            let index = usize::try_from(index)
                .map_err(|_| invariant("validated sparse morph index cannot address memory"))?;
            if index >= logical_count || previous.is_some_and(|previous| previous >= index) {
                return Err(invariant(
                    "validated sparse morph indices are unordered or out of range",
                ));
            }
            previous = Some(index);
        }
        Ok(Self {
            logical_count,
            indices,
            values,
            next: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn logical_count(&self) -> usize {
        self.logical_count
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.indices.len()
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationSparseDelta>> {
        if self.next == self.indices.len() {
            return Ok(None);
        }
        let index = usize::try_from(
            *self
                .indices
                .get(self.next)
                .ok_or_else(|| invariant("sparse morph index is unavailable"))?,
        )
        .map_err(|_| invariant("sparse morph index cannot address memory"))?;
        let value = self
            .values
            .next()?
            .ok_or_else(|| invariant("sparse morph value is unavailable"))?;
        self.next = advance_cursor(self.next, 1, "sparse morph cursor index overflows")?;
        Ok(Some(NodeSceneDeformationSparseDelta { index, value }))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next != self.indices.len() {
            return Err(invariant(
                "sparse morph cursor ended before its declared count",
            ));
        }
        self.values.finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationSparseDelta {
    index: usize,
    value: Vec3,
}

impl NodeSceneDeformationSparseDelta {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn value(self) -> Vec3 {
        self.value
    }
}

/// Deformation bindings attached to one geometry mesh instance.
#[must_use = "the instance morph-weight cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMeshInstance<'scene> {
    source_index: usize,
    skin: Option<usize>,
    morph_weights: Option<NodeSceneDeformationF32Cursor<'scene>>,
}

impl<'scene> NodeSceneDeformationMeshInstance<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn skin(&self) -> Option<usize> {
        self.skin
    }

    pub(in crate::canonical_scene::store) const fn morph_weights_present(&self) -> bool {
        self.morph_weights.is_some()
    }

    pub(in crate::canonical_scene::store) fn take_morph_weights(
        &mut self,
    ) -> Option<NodeSceneDeformationF32Cursor<'scene>> {
        self.morph_weights.take()
    }
}

#[must_use = "the influence-row cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationInfluenceRowsCursor<'scene> {
    offsets: &'scene [u32],
    joints: &'scene [u32],
    weights: &'scene [f32],
    source_orders: &'scene [u32],
    next_vertex: usize,
}

impl<'scene> NodeSceneDeformationInfluenceRowsCursor<'scene> {
    fn try_new(
        offsets: &'scene [u32],
        joints: &'scene [u32],
        weights: &'scene [f32],
        source_orders: &'scene [u32],
    ) -> NodeSceneSemanticResult<Self> {
        if offsets.first().copied() != Some(0)
            || joints.len() != weights.len()
            || joints.len() != source_orders.len()
            || usize::try_from(offsets.last().copied().unwrap_or(u32::MAX)).ok()
                != Some(joints.len())
        {
            return Err(invariant(
                "validated influence rows have inconsistent payload boundaries",
            ));
        }
        for pair in offsets.windows(2) {
            if pair[0] > pair[1] || usize::try_from(pair[1]).map_or(true, |end| end > joints.len())
            {
                return Err(invariant(
                    "validated influence row offsets are unordered or out of range",
                ));
            }
        }
        if weights
            .iter()
            .any(|weight| !weight.is_finite() || *weight < 0.0)
        {
            return Err(invariant(
                "validated influence weights contain invalid data",
            ));
        }
        Ok(Self {
            offsets,
            joints,
            weights,
            source_orders,
            next_vertex: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn vertex_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub(in crate::canonical_scene::store) fn next_row(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationInfluenceRow<'scene>>> {
        if self.next_vertex == self.vertex_count() {
            return Ok(None);
        }
        let end_index = self
            .next_vertex
            .checked_add(1)
            .ok_or_else(|| invariant("influence-row cursor index overflows"))?;
        let start = usize::try_from(
            *self
                .offsets
                .get(self.next_vertex)
                .ok_or_else(|| invariant("influence-row start is unavailable"))?,
        )
        .map_err(|_| invariant("influence-row start cannot address memory"))?;
        let end = usize::try_from(
            *self
                .offsets
                .get(end_index)
                .ok_or_else(|| invariant("influence-row end is unavailable"))?,
        )
        .map_err(|_| invariant("influence-row end cannot address memory"))?;
        let row = NodeSceneDeformationInfluenceRow {
            vertex: self.next_vertex,
            influences: NodeSceneDeformationInfluenceCursor::try_new(
                self.joints
                    .get(start..end)
                    .ok_or_else(|| invariant("influence-joint row is unavailable"))?,
                self.weights
                    .get(start..end)
                    .ok_or_else(|| invariant("influence-weight row is unavailable"))?,
                self.source_orders
                    .get(start..end)
                    .ok_or_else(|| invariant("influence source-order row is unavailable"))?,
            )?,
        };
        self.next_vertex = end_index;
        Ok(Some(row))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        (self.next_vertex == self.vertex_count())
            .then_some(())
            .ok_or_else(|| invariant("influence-row cursor ended before its declared count"))
    }
}

#[must_use = "the influence row must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationInfluenceRow<'scene> {
    vertex: usize,
    influences: NodeSceneDeformationInfluenceCursor<'scene>,
}

impl<'scene> NodeSceneDeformationInfluenceRow<'scene> {
    pub(in crate::canonical_scene::store) const fn vertex(&self) -> usize {
        self.vertex
    }

    pub(in crate::canonical_scene::store) fn into_influences(
        self,
    ) -> NodeSceneDeformationInfluenceCursor<'scene> {
        self.influences
    }
}

#[must_use = "the influence cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationInfluenceCursor<'scene> {
    joints: &'scene [u32],
    weights: &'scene [f32],
    source_orders: &'scene [u32],
    next: usize,
}

impl<'scene> NodeSceneDeformationInfluenceCursor<'scene> {
    fn try_new(
        joints: &'scene [u32],
        weights: &'scene [f32],
        source_orders: &'scene [u32],
    ) -> NodeSceneSemanticResult<Self> {
        if joints.len() != weights.len() || joints.len() != source_orders.len() {
            return Err(invariant("validated influence row payload widths disagree"));
        }
        Ok(Self {
            joints,
            weights,
            source_orders,
            next: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.joints.len()
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationInfluence>> {
        if self.next == self.joints.len() {
            return Ok(None);
        }
        let source_order = *self
            .source_orders
            .get(self.next)
            .ok_or_else(|| invariant("influence source order is unavailable"))?;
        if usize::try_from(source_order)
            .ok()
            .is_none_or(|source_order| source_order >= self.joints.len())
        {
            return Err(invariant(
                "validated influence source order is outside its row",
            ));
        }
        let influence = NodeSceneDeformationInfluence {
            palette_joint: *self
                .joints
                .get(self.next)
                .ok_or_else(|| invariant("influence joint is unavailable"))?,
            weight: *self
                .weights
                .get(self.next)
                .ok_or_else(|| invariant("influence weight is unavailable"))?,
            source_order,
        };
        self.next = advance_cursor(self.next, 1, "influence cursor index overflows")?;
        Ok(Some(influence))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        (self.next == self.joints.len())
            .then_some(())
            .ok_or_else(|| invariant("influence cursor ended before its declared count"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationInfluence {
    palette_joint: u32,
    weight: f32,
    source_order: u32,
}

impl NodeSceneDeformationInfluence {
    pub(in crate::canonical_scene::store) const fn palette_joint(self) -> u32 {
        self.palette_joint
    }

    pub(in crate::canonical_scene::store) const fn weight(self) -> f32 {
        self.weight
    }

    pub(in crate::canonical_scene::store) const fn source_order(self) -> u32 {
        self.source_order
    }
}

impl<'scene> NodeSceneDeformationLogicalMorph<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn definition_offset(&self) -> usize {
        self.definition_offset
    }

    pub(in crate::canonical_scene::store) const fn definition_count(&self) -> usize {
        self.definition_count
    }

    pub(in crate::canonical_scene::store) fn take_default_weights(
        &mut self,
    ) -> Option<NodeSceneDeformationF32Cursor<'scene>> {
        self.default_weights.take()
    }
}

/// Stable logical-mesh-owned target identity and optional name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMorphDefinition<'scene> {
    source_index: usize,
    logical_mesh: usize,
    target_ordinal: usize,
    stable_identity: u32,
    name: Option<&'scene str>,
}

impl<'scene> NodeSceneDeformationMorphDefinition<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh(self) -> usize {
        self.logical_mesh
    }

    pub(in crate::canonical_scene::store) const fn target_ordinal(self) -> usize {
        self.target_ordinal
    }

    pub(in crate::canonical_scene::store) const fn stable_identity(self) -> u32 {
        self.stable_identity
    }

    pub(in crate::canonical_scene::store) const fn name(self) -> Option<&'scene str> {
        self.name
    }
}

#[must_use = "the scalar cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationLeafCursor<'scene, T: StoredScalar>
{
    values: &'scene [T],
    next: usize,
}

pub(in crate::canonical_scene::store) type NodeSceneDeformationF32Cursor<'scene> =
    NodeSceneDeformationLeafCursor<'scene, f32>;

impl<'scene, T: StoredScalar> NodeSceneDeformationLeafCursor<'scene, T> {
    const fn new(values: &'scene [T]) -> Self {
        Self { values, next: 0 }
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.values.len()
    }

    pub(in crate::canonical_scene::store) fn next(&mut self) -> NodeSceneSemanticResult<Option<T>> {
        if self.next == self.values.len() {
            return Ok(None);
        }
        let value = self
            .values
            .get(self.next)
            .copied()
            .ok_or_else(|| invariant("deformation leaf cursor advanced beyond its range"))?;
        self.next = advance_cursor(self.next, 1, "deformation leaf cursor index overflows")?;
        Ok(Some(value))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        (self.next == self.values.len())
            .then_some(())
            .ok_or_else(|| invariant("deformation leaf cursor ended before its declared count"))
    }
}

impl<'scene> NodeSceneDeformationSkin<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn name(&self) -> Option<&'scene str> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn skeleton_root(&self) -> Option<usize> {
        self.skeleton_root
    }

    pub(in crate::canonical_scene::store) const fn matrix_spaces(&self) -> SkinMatrixSpaces {
        self.matrix_spaces
    }

    pub(in crate::canonical_scene::store) fn take_joints(
        &mut self,
    ) -> Option<NodeSceneDeformationJointCursor<'scene>> {
        self.joints.take()
    }

    pub(in crate::canonical_scene::store) fn take_inverse_bind_matrices(
        &mut self,
    ) -> Option<NodeSceneDeformationMatrixCursor<'scene>> {
        self.inverse_bind_matrices.take()
    }
}

#[must_use = "the skin joint cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationJointCursor<'scene> {
    canonical_nodes: &'scene [u32],
    source_node_count: usize,
    next: usize,
}

impl<'scene> NodeSceneDeformationJointCursor<'scene> {
    fn try_new(
        canonical_nodes: &'scene [u32],
        source_node_count: usize,
    ) -> NodeSceneSemanticResult<Self> {
        for canonical in canonical_nodes {
            let source = usize::try_from(*canonical)
                .ok()
                .and_then(|value| value.checked_sub(1))
                .ok_or_else(|| {
                    invariant("validated skin joint cannot address source-node space")
                })?;
            if source >= source_node_count {
                return Err(invariant(
                    "validated skin joint is outside source-node space",
                ));
            }
        }
        Ok(Self {
            canonical_nodes,
            source_node_count,
            next: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.canonical_nodes.len()
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<usize>> {
        if self.next == self.canonical_nodes.len() {
            return Ok(None);
        }
        let canonical = usize::try_from(
            *self
                .canonical_nodes
                .get(self.next)
                .ok_or_else(|| invariant("skin joint cursor advanced beyond its range"))?,
        )
        .map_err(|_| invariant("validated skin joint cannot address memory"))?;
        let source = canonical
            .checked_sub(1)
            .ok_or_else(|| invariant("validated skin joint addresses the synthetic root"))?;
        if source >= self.source_node_count {
            return Err(invariant(
                "validated skin joint is outside source-node space",
            ));
        }
        self.next = advance_cursor(self.next, 1, "skin joint cursor index overflows")?;
        Ok(Some(source))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        (self.next == self.canonical_nodes.len())
            .then_some(())
            .ok_or_else(|| invariant("skin joint cursor ended before its declared count"))
    }
}

#[must_use = "the matrix cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationMatrixCursor<'scene> {
    scalars: &'scene [f32],
    next_matrix: usize,
}

impl<'scene> NodeSceneDeformationMatrixCursor<'scene> {
    fn try_new(scalars: &'scene [f32]) -> NodeSceneSemanticResult<Self> {
        if !scalars.len().is_multiple_of(MATRIX_SCALAR_COUNT) {
            return Err(invariant(
                "validated matrix payload is not an exact matrix sequence",
            ));
        }
        for chunk in scalars.chunks_exact(MATRIX_SCALAR_COUNT) {
            let matrix = matrix_from_slice(chunk)?;
            if !matrix.is_finite() || !matrix.is_affine() {
                return Err(invariant(
                    "validated matrix payload contains non-finite or non-affine data",
                ));
            }
        }
        Ok(Self {
            scalars,
            next_matrix: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.scalars.len() / MATRIX_SCALAR_COUNT
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<Mat4>> {
        if self.next_matrix == self.declared_count() {
            return Ok(None);
        }
        let start = self
            .next_matrix
            .checked_mul(MATRIX_SCALAR_COUNT)
            .ok_or_else(|| invariant("matrix cursor scalar offset overflows"))?;
        let end = start
            .checked_add(MATRIX_SCALAR_COUNT)
            .ok_or_else(|| invariant("matrix cursor scalar end overflows"))?;
        let matrix = matrix_from_slice(
            self.scalars
                .get(start..end)
                .ok_or_else(|| invariant("matrix cursor advanced beyond its range"))?,
        )?;
        self.next_matrix = advance_cursor(self.next_matrix, 1, "matrix cursor index overflows")?;
        Ok(Some(matrix))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        (self.next_matrix == self.declared_count())
            .then_some(())
            .ok_or_else(|| invariant("matrix cursor ended before its declared count"))
    }
}

fn next_row(
    next: usize,
    total: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<usize>> {
    if next == total {
        return Ok(None);
    }
    if next > total {
        return Err(invariant(reason));
    }
    Ok(Some(next))
}

fn advance_cursor(
    cursor: usize,
    count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<usize> {
    cursor.checked_add(count).ok_or_else(|| invariant(reason))
}

fn checked_end(
    offset: usize,
    count: usize,
    total: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<usize> {
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    if end > total {
        return Err(invariant(reason));
    }
    Ok(end)
}

fn require_row_ordinal(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    if read_u32_as_usize(table, row, field, unreadable_reason)? != row {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn require_absent_cell(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    match table.scalar::<u8>(row, field) {
        Some(0) => Ok(()),
        _ => Err(invariant(reason)),
    }
}

fn require_optional_record_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    if !present {
        return require_absent_cell(table, row, field, reason);
    }
    let expected = expected.ok_or_else(|| invariant(reason))?;
    let actual = table
        .record_ref::<Tag>(row, field)
        .ok_or_else(|| invariant(reason))?;
    if actual != expected.into_store() {
        return Err(invariant(reason));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_sequence_range<'scene, T, Tag>(
    table: CanonicalRecordView<'scene>,
    row: usize,
    field: usize,
    offset: usize,
    count: usize,
    expected: Option<CanonicalBlockRef<Tag>>,
    payload: Option<&'scene [T]>,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    if count == 0 {
        require_absent_cell(table, row, field, reason)?;
        return Ok(&[]);
    }
    let expected = expected.ok_or_else(|| invariant(reason))?;
    let actual = table
        .sequence_ref::<T, Tag>(row, field)
        .ok_or_else(|| invariant(reason))?;
    if actual != expected.into_store() {
        return Err(invariant(reason));
    }
    let payload = payload.ok_or_else(|| invariant(reason))?;
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    payload.get(offset..end).ok_or_else(|| invariant(reason))
}

#[allow(clippy::too_many_arguments)]
fn read_utf8_range<'scene, Tag>(
    table: CanonicalRecordView<'scene>,
    row: usize,
    field: usize,
    offset: usize,
    count: usize,
    expected: Option<CanonicalBlockRef<Tag>>,
    payload: Option<&'scene str>,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene str>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    if count == 0 {
        require_absent_cell(table, row, field, reason)?;
        return Ok("");
    }
    let expected = expected.ok_or_else(|| invariant(reason))?;
    let actual = table
        .utf8_ref::<Tag>(row, field)
        .ok_or_else(|| invariant(reason))?;
    if actual != expected.into_store() {
        return Err(invariant(reason));
    }
    let payload = payload.ok_or_else(|| invariant(reason))?;
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    payload.get(offset..end).ok_or_else(|| invariant(reason))
}

fn read_optional_source_node(
    table: CanonicalRecordView<'_>,
    row: usize,
    presence_field: usize,
    node_field: usize,
    source_node_count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<usize>> {
    let presence = read_scalar::<u8>(table, row, presence_field, reason)?;
    let canonical = read_scalar::<u32>(table, row, node_field, reason)?;
    match presence {
        PRESENCE_ABSENT if canonical == 0 => Ok(None),
        PRESENCE_ABSENT => Err(invariant(reason)),
        PRESENCE_PRESENT => {
            let source = usize::try_from(canonical)
                .ok()
                .and_then(|value| value.checked_sub(1))
                .ok_or_else(|| invariant(reason))?;
            if source >= source_node_count {
                return Err(invariant(reason));
            }
            Ok(Some(source))
        }
        _ => Err(invariant(reason)),
    }
}

fn decode_skin_space(value: u8) -> NodeSceneSemanticResult<SkinMatrixSpace> {
    match value {
        DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND => Ok(SkinMatrixSpace::LogicalMeshLocalBind),
        DEFORMATION_SPACE_JOINT_LOCAL_BIND => Ok(SkinMatrixSpace::JointLocalBind),
        DEFORMATION_SPACE_JOINT_LOCAL => Ok(SkinMatrixSpace::JointLocal),
        DEFORMATION_SPACE_SCENE => Ok(SkinMatrixSpace::Scene),
        DEFORMATION_SPACE_MESH_INSTANCE_LOCAL => Ok(SkinMatrixSpace::MeshInstanceLocal),
        _ => Err(invariant("validated skin matrix-space code is unknown")),
    }
}

fn decode_morph_semantic(value: u8) -> NodeSceneSemanticResult<MorphDeltaSemantic> {
    match value {
        DEFORMATION_MORPH_POSITION => Ok(MorphDeltaSemantic::Position),
        DEFORMATION_MORPH_NORMAL => Ok(MorphDeltaSemantic::Normal),
        DEFORMATION_MORPH_TANGENT => Ok(MorphDeltaSemantic::Tangent),
        _ => Err(invariant("validated morph-delta semantic code is unknown")),
    }
}

fn decode_morph_payload(
    value: u8,
) -> NodeSceneSemanticResult<NodeSceneDeformationMorphPayloadKind> {
    match value {
        DEFORMATION_MORPH_DENSE => Ok(NodeSceneDeformationMorphPayloadKind::Dense),
        DEFORMATION_MORPH_SPARSE => Ok(NodeSceneDeformationMorphPayloadKind::Sparse),
        _ => Err(invariant(
            "validated morph-delta payload-kind code is unknown",
        )),
    }
}

fn matrix_from_slice(values: &[f32]) -> NodeSceneSemanticResult<Mat4> {
    if values.len() != MATRIX_SCALAR_COUNT {
        return Err(invariant(
            "validated matrix payload does not contain exactly sixteen scalars",
        ));
    }
    let scalar = |index| {
        values
            .get(index)
            .copied()
            .ok_or_else(|| invariant("validated matrix scalar is unavailable"))
    };
    Ok(Mat4 {
        cols: [
            [scalar(0)?, scalar(1)?, scalar(2)?, scalar(3)?],
            [scalar(4)?, scalar(5)?, scalar(6)?, scalar(7)?],
            [scalar(8)?, scalar(9)?, scalar(10)?, scalar(11)?],
            [scalar(12)?, scalar(13)?, scalar(14)?, scalar(15)?],
        ],
    })
}

fn require_payload_exhausted<T>(
    cursor: usize,
    payload: Option<&[T]>,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    if cursor == payload.map_or(0, <[T]>::len) {
        Ok(())
    } else {
        Err(invariant(reason))
    }
}

fn require_text_exhausted(
    cursor: usize,
    payload: Option<&str>,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    if cursor == payload.map_or(0, str::len) {
        Ok(())
    } else {
        Err(invariant(reason))
    }
}
