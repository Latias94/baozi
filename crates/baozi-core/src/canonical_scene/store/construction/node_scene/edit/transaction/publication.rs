//! Generation admission and rooted handoff for a validated Scene edit.
//!
//! The operation-owned freeze permit and the core-owned generation reservation meet only in the
//! scoped callback below.  The callback cannot retain either branded joint capability after it
//! returns, and the published evidence owner keeps the remap leaves alive with the new Scene.

use baozi_operation::__retained_store_integration::activation::{
    CanonicalFreezePermit, CanonicalJointCommit, CanonicalJointSeal,
    CentrallyRejectedCanonicalStoreSession, FrozenFailureState, StoreActivationContainment,
};
use baozi_operation::__retained_store_integration::authority::PreparedStoreSuccess;
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, PublishedOutcomeEvidenceStore, RecordBlock,
    StoreLedgerEvidence,
};
use thiserror::Error;

use crate::canonical_scene::store::construction::GENERATION_EXHAUSTED_CODE;
use crate::canonical_scene::store::construction::node_scene::SceneStorage;
use crate::canonical_scene::store::schema::SceneDirectorySchema;
use crate::identity::{
    PrimitiveId, ReservedSceneGeneration, SceneGenerationError, SceneGenerationId,
};
use crate::scene_space::{Axis, FrontFaceWinding, Handedness, SceneSpace, SceneSpaceField};
use crate::space_conversion::{SpaceConversion, TargetSpace};

use super::SceneEditOutput;
use super::schema::{
    SceneEditEvidenceTag, SceneGeometryCornerOriginsTag, SceneGeometryFaceOffsetsTag,
};
use super::validation::SceneEditValidationProof;

const SCENE_EDIT_GENERATION_REJECTION_CODE: u32 = GENERATION_EXHAUSTED_CODE;

/// Core's final generation-admission rejection for a Scene edit.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("Scene edit Scene generation admission was exhausted")]
pub struct SceneEditGenerationRejection;

impl SceneEditGenerationRejection {
    pub const fn stable_code(self) -> u32 {
        SCENE_EDIT_GENERATION_REJECTION_CODE
    }
}

/// The operation/core paired freeze permit for one validated Scene edit.
pub type SceneEditFreezePermit<O, PP, U> = CanonicalFreezePermit<
    O,
    PP,
    U,
    SceneEditOutput,
    SceneEditValidationProof,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    SceneEditEvidenceTag,
>;

/// The scoped operation contribution paired with a Scene edit generation token.
pub type SceneEditJointCommit<'joint, O, PP, U> = CanonicalJointCommit<
    'joint,
    O,
    PP,
    U,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    SceneEditEvidenceTag,
>;

/// Rich failure typestate produced when the edit cannot consume its final generation permit.
pub type SceneEditFrozenGenerationRejection<O, PP, U> = CentrallyRejectedCanonicalStoreSession<
    O,
    PP,
    U,
    SceneEditGenerationRejection,
    RecordBlock,
    SceneEditEvidenceTag,
    FrozenFailureState,
>;

/// Core-owned generation/proof half of a successful Scene edit publication.
#[must_use = "the Scene edit handoff token must consume the matching prepared owner"]
pub struct SceneEditHandoffToken {
    generation: ReservedSceneGeneration,
    proof: SceneEditValidationProof,
    tables: crate::canonical_scene::store::typed_storage::SceneTables,
}

/// Published effects for exactly one concrete Scene operation.
#[must_use = "published Scene edit evidence owns its rooted storage"]
pub enum SceneEditEvidence {
    Triangulation(SceneTriangulationEvidence),
    Normals(SceneNormalGenerationEvidence),
    SpaceConversion(SceneSpaceConversionEvidence),
}

struct GeometryEvidenceOwner {
    input_target: PrimitiveId,
    output_target: PrimitiveId,
    store: PublishedOutcomeEvidenceStore<RecordBlock, SceneEditEvidenceTag>,
}

/// Centrally proved whole-scene effects with the actual published evidence root owner.
/// Primitive-local remaps are deliberately absent; triangle reversal uses `[0, 2, 1]`.
pub struct SceneSpaceConversionEvidence {
    store: PublishedOutcomeEvidenceStore<RecordBlock, SceneEditEvidenceTag>,
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
}

impl SceneSpaceConversionEvidence {
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.input_generation
    }
    pub const fn output_generation(&self) -> SceneGenerationId {
        self.output_generation
    }
    pub fn source(&self) -> TargetSpace {
        let root = self.store.root();
        decode_space(
            |field| root.scalar::<u32>(0, field),
            root.scalar::<u64>(0, super::schema::field::SOURCE_UNIT_BITS),
            [
                super::schema::space_field::SOURCE_HANDEDNESS,
                super::schema::space_field::SOURCE_UP,
                super::schema::space_field::SOURCE_FRONT,
                super::schema::space_field::SOURCE_WINDING,
            ],
        )
        .expect("published Scene conversion source facts were validated before handoff")
    }
    pub fn target(&self) -> TargetSpace {
        let root = self.store.root();
        decode_space(
            |field| root.scalar::<u32>(0, field),
            root.scalar::<u64>(0, super::schema::field::TARGET_UNIT_BITS),
            [
                super::schema::space_field::TARGET_HANDEDNESS,
                super::schema::space_field::TARGET_UP,
                super::schema::space_field::TARGET_FRONT,
                super::schema::space_field::TARGET_WINDING,
            ],
        )
        .expect("published Scene conversion target facts were validated before handoff")
    }
    pub fn reverses_triangle_winding(&self) -> bool {
        self.store
            .root()
            .scalar::<u32>(0, super::schema::space_field::REVERSE_WINDING)
            .expect("published Scene conversion winding effect was validated before handoff")
            != 0
    }
    pub fn triangle_corner_order(&self) -> [u32; 3] {
        if self.reverses_triangle_winding() {
            [0, 2, 1]
        } else {
            [0, 1, 2]
        }
    }
    pub fn assumed_source_field(&self, field: SceneSpaceField) -> bool {
        let bit = match field {
            SceneSpaceField::Handedness => 1,
            SceneSpaceField::UpAxis => 2,
            SceneSpaceField::FrontAxis => 4,
            SceneSpaceField::UnitScaleToMeters => 8,
            SceneSpaceField::FrontFaceWinding => 16,
            _ => 0,
        };
        self.store
            .root()
            .scalar::<u32>(0, super::schema::space_field::ASSUMED_MASK)
            .expect("published Scene conversion assumption mask was validated before handoff")
            & bit
            != 0
    }
}

/// Triangulation evidence with both centrally validated origin relations present.
pub struct SceneTriangulationEvidence {
    owner: GeometryEvidenceOwner,
    corner_origins: baozi_operation::OutcomeEvidenceSequenceRef<u32, SceneGeometryCornerOriginsTag>,
    face_offsets: baozi_operation::OutcomeEvidenceSequenceRef<u32, SceneGeometryFaceOffsetsTag>,
    source_corners: u32,
    vertices: u32,
}

impl SceneTriangulationEvidence {
    pub const fn input_target(&self) -> PrimitiveId {
        self.owner.input_target
    }

    pub const fn output_target(&self) -> PrimitiveId {
        self.owner.output_target
    }

    pub const fn source_corner_count(&self) -> u32 {
        self.source_corners
    }

    pub const fn vertex_count(&self) -> u32 {
        self.vertices
    }

    /// Returns the published flattened source-corner relation without copying it.
    pub fn source_corner_origins(&self) -> &[u32] {
        self.owner
            .store
            .sequence(self.corner_origins)
            .map(|view| view.as_slice())
            .unwrap_or_else(|| panic!("published geometry corner-origin evidence is missing"))
    }

    /// Returns the published source-face triangle-prefix relation without copying it.
    pub fn source_face_triangle_offsets(&self) -> &[u32] {
        self.owner
            .store
            .sequence(self.face_offsets)
            .map(|view| view.as_slice())
            .unwrap_or_else(|| panic!("published geometry face-offset evidence is missing"))
    }
}

/// Normal-generation effects decoded once from the centrally validated root.
/// The retained root owner remains alive independently of the Scene.
pub struct SceneNormalGenerationEvidence {
    owner: GeometryEvidenceOwner,
    effects: u32,
    removed_metadata: u32,
    normal_attribute: u32,
}

impl SceneNormalGenerationEvidence {
    pub const fn input_target(&self) -> PrimitiveId {
        self.owner.input_target
    }

    pub const fn output_target(&self) -> PrimitiveId {
        self.owner.output_target
    }

    pub const fn replaced_normal(&self) -> bool {
        self.effects & super::schema::normal_field::REPLACED_NORMAL != 0
    }

    pub const fn removed_tangent(&self) -> bool {
        self.effects & super::schema::normal_field::REMOVED_TANGENT != 0
    }

    pub const fn smooth(&self) -> bool {
        self.effects & super::schema::normal_field::SMOOTH != 0
    }

    pub const fn removed_metadata_entries(&self) -> u32 {
        self.removed_metadata
    }

    pub const fn normal_attribute_index(&self) -> u32 {
        self.normal_attribute
    }
}

/// Exact published Scene, rooted remap evidence, and operation ledger evidence.
#[must_use = "the published Scene edit must be transferred to its host"]
pub struct SceneEditPublishedSuccess {
    ledger_evidence: StoreLedgerEvidence,
    evidence: SceneEditEvidence,
    scene: SceneStorage,
}

impl SceneEditPublishedSuccess {
    pub fn into_parts(
        self,
    ) -> (
        crate::ValidatedScene,
        SceneEditEvidence,
        StoreLedgerEvidence,
    ) {
        (
            crate::ValidatedScene::from_node_scene(self.scene),
            self.evidence,
            self.ledger_evidence,
        )
    }
}

impl SceneEditHandoffToken {
    /// Verifies the canonical root and publishes the matching Scene/evidence owners.
    pub fn handoff(
        self,
        store: PreparedStoreSuccess<
            RecordBlock,
            SceneDirectorySchema,
            RecordBlock,
            SceneEditEvidenceTag,
        >,
    ) -> Result<SceneEditPublishedSuccess, SceneEditHandoffPairingError> {
        let target_generation = match self.proof.input_target {
            super::SceneEditTarget::Primitive(target) => target.generation(),
            super::SceneEditTarget::WholeScene(generation) => generation,
        };
        if target_generation != self.proof.source_generation {
            drop(self);
            return Err(SceneEditHandoffPairingError);
        }
        let expected_root = self.proof.scene.directory().into_store();
        let published = store.handoff();
        if published.canonical.root_ref() != expected_root {
            drop(published);
            drop(self);
            return Err(SceneEditHandoffPairingError);
        }

        let evidence = {
            let root = published.outcome_evidence.root();
            if root.rows() != 1 || root.fields() != super::schema::ROOT_FIELDS {
                return Err(SceneEditHandoffPairingError);
            }
            let invalid = SceneEditHandoffPairingError;
            if root.scalar::<u32>(0, super::schema::field::REVISION)
                != Some(super::schema::REVISION)
                || root.scalar::<u32>(0, super::schema::field::OUTCOME)
                    != Some(super::schema::OUTCOME_CHANGED)
                || root.scalar::<u32>(0, super::schema::field::SEMANTIC_VERSION)
                    != Some(super::schema::OPERATION_SEMANTIC_VERSION)
            {
                return Err(invalid);
            }
            match root.scalar::<u32>(0, super::schema::field::OPERATION_KIND) {
                Some(super::schema::OPERATION_NORMALS) => {
                    let super::SceneEditTarget::Primitive(input_target) = self.proof.input_target
                    else {
                        return Err(invalid);
                    };
                    let output_target = input_target.rebind(self.generation.id());
                    if root.scalar::<u64>(0, super::schema::field::SOURCE_UNIT_BITS) != Some(0)
                        || root.scalar::<u64>(0, super::schema::field::TARGET_UNIT_BITS) != Some(0)
                    {
                        return Err(invalid);
                    }
                    if root.scalar::<u8>(0, super::schema::field::CORNER_ORIGINS) != Some(0)
                        || root.scalar::<u8>(0, super::schema::field::FACE_OFFSETS) != Some(0)
                    {
                        return Err(SceneEditHandoffPairingError);
                    }
                    let effects = root
                        .scalar::<u32>(0, super::schema::normal_field::EFFECTS)
                        .ok_or(invalid)?;
                    let removed_metadata = root
                        .scalar::<u32>(0, super::schema::normal_field::REMOVED_METADATA)
                        .ok_or(invalid)?;
                    let normal_attribute = root
                        .scalar::<u32>(0, super::schema::normal_field::OUTPUT_NORMAL_ATTRIBUTE)
                        .ok_or(invalid)?;
                    let attributes = root
                        .scalar::<u32>(0, super::schema::normal_field::OUTPUT_ATTRIBUTE_COUNT)
                        .ok_or(invalid)?;
                    if normal_attribute >= attributes {
                        return Err(invalid);
                    }
                    SceneEditEvidence::Normals(SceneNormalGenerationEvidence {
                        owner: GeometryEvidenceOwner {
                            input_target,
                            output_target,
                            store: published.outcome_evidence,
                        },
                        effects,
                        removed_metadata,
                        normal_attribute,
                    })
                }
                Some(super::schema::OPERATION_TRIANGULATE) => {
                    let super::SceneEditTarget::Primitive(input_target) = self.proof.input_target
                    else {
                        return Err(invalid);
                    };
                    let output_target = input_target.rebind(self.generation.id());
                    if root.scalar::<u64>(0, super::schema::field::SOURCE_UNIT_BITS) != Some(0)
                        || root.scalar::<u64>(0, super::schema::field::TARGET_UNIT_BITS) != Some(0)
                    {
                        return Err(invalid);
                    }
                    let corner_origins = root
                        .sequence_ref::<u32, SceneGeometryCornerOriginsTag>(
                            0,
                            super::schema::field::CORNER_ORIGINS,
                        )
                        .ok_or(invalid)?;
                    let face_offsets = root
                        .sequence_ref::<u32, SceneGeometryFaceOffsetsTag>(
                            0,
                            super::schema::field::FACE_OFFSETS,
                        )
                        .ok_or(invalid)?;
                    let source_corners = root
                        .scalar::<u32>(0, super::schema::field::SOURCE_CORNER_COUNT)
                        .ok_or(invalid)?;
                    let vertices = root
                        .scalar::<u32>(0, super::schema::field::VERTEX_COUNT)
                        .ok_or(invalid)?;
                    SceneEditEvidence::Triangulation(SceneTriangulationEvidence {
                        owner: GeometryEvidenceOwner {
                            input_target,
                            output_target,
                            store: published.outcome_evidence,
                        },
                        corner_origins,
                        face_offsets,
                        source_corners,
                        vertices,
                    })
                }
                Some(super::schema::OPERATION_SPACE_CONVERSION) => {
                    let super::SceneEditTarget::WholeScene(input_generation) =
                        self.proof.input_target
                    else {
                        return Err(invalid);
                    };
                    if root.scalar::<u8>(0, super::schema::field::CORNER_ORIGINS) != Some(0)
                        || root.scalar::<u8>(0, super::schema::field::FACE_OFFSETS) != Some(0)
                        || root.scalar::<u32>(0, super::schema::field::OPTIONS_ALGORITHM) != Some(0)
                    {
                        return Err(invalid);
                    }
                    let source = decode_space(
                        |field| root.scalar::<u32>(0, field),
                        root.scalar::<u64>(0, super::schema::field::SOURCE_UNIT_BITS),
                        [
                            super::schema::space_field::SOURCE_HANDEDNESS,
                            super::schema::space_field::SOURCE_UP,
                            super::schema::space_field::SOURCE_FRONT,
                            super::schema::space_field::SOURCE_WINDING,
                        ],
                    )
                    .ok_or(invalid)?;
                    let target = decode_space(
                        |field| root.scalar::<u32>(0, field),
                        root.scalar::<u64>(0, super::schema::field::TARGET_UNIT_BITS),
                        [
                            super::schema::space_field::TARGET_HANDEDNESS,
                            super::schema::space_field::TARGET_UP,
                            super::schema::space_field::TARGET_FRONT,
                            super::schema::space_field::TARGET_WINDING,
                        ],
                    )
                    .ok_or(invalid)?;
                    root.scalar::<u32>(0, super::schema::space_field::ASSUMED_MASK)
                        .filter(|mask| *mask <= 31)
                        .ok_or(invalid)?;
                    let reverse = root
                        .scalar::<u32>(0, super::schema::space_field::REVERSE_WINDING)
                        .filter(|value| *value <= 1)
                        .ok_or(invalid)?
                        != 0;
                    let changes = root
                        .scalar::<u32>(0, super::schema::space_field::CHANGES_COORDINATES)
                        .filter(|value| *value <= 1)
                        .ok_or(invalid)?
                        != 0;
                    let source_space = SceneSpace {
                        handedness: source.handedness(),
                        up_axis: source.up_axis(),
                        front_axis: source.front_axis(),
                        unit_scale_to_meters: Some(source.unit_scale_to_meters()),
                        front_face_winding: source.front_face_winding(),
                        ..SceneSpace::UNKNOWN
                    };
                    let conversion = SpaceConversion::try_new(&source_space, target, None)
                        .map_err(|_| invalid)?;
                    if conversion.reverse_winding() != reverse
                        || conversion.changes_coordinates() != changes
                    {
                        return Err(invalid);
                    }
                    SceneEditEvidence::SpaceConversion(SceneSpaceConversionEvidence {
                        store: published.outcome_evidence,
                        input_generation,
                        output_generation: self.generation.id(),
                    })
                }
                _ => return Err(invalid),
            }
        };

        Ok(SceneEditPublishedSuccess {
            scene: SceneStorage {
                generation: self.generation.into_id(),
                tables: self.tables,
                canonical: published.canonical,
                proof: self.proof.scene,
                #[cfg(test)]
                semantic_read_fault: None,
            },
            evidence,
            ledger_evidence: published.ledger_evidence,
        })
    }
}

fn decode_space(
    mut read: impl FnMut(usize) -> Option<u32>,
    unit: Option<u64>,
    fields: [usize; 4],
) -> Option<TargetSpace> {
    let handedness = match read(fields[0])? {
        1 => Handedness::Right,
        2 => Handedness::Left,
        _ => return None,
    };
    let axis = |value| match value {
        1 => Some(Axis::PositiveX),
        2 => Some(Axis::NegativeX),
        3 => Some(Axis::PositiveY),
        4 => Some(Axis::NegativeY),
        5 => Some(Axis::PositiveZ),
        6 => Some(Axis::NegativeZ),
        _ => None,
    };
    let up = axis(read(fields[1])?)?;
    let front = axis(read(fields[2])?)?;
    let winding = match read(fields[3])? {
        1 => FrontFaceWinding::Clockwise,
        2 => FrontFaceWinding::CounterClockwise,
        _ => return None,
    };
    TargetSpace::try_new(handedness, up, front, f64::from_bits(unit?), winding).ok()
}

/// A prepared canonical owner did not match the core proof root or evidence shape.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("prepared Scene edit owner does not match the core validation proof")]
pub struct SceneEditHandoffPairingError;

/// Consumes the final Scene edit freeze permit and reserves its destination generation.
///
/// The paired joint and seal are scoped to the callback, so the operation owner can prepare the
/// exact store while core retains the only generation/proof token.
#[allow(clippy::type_complexity)]
pub fn consume_scene_edit_freeze_permit<O, PP, U, R>(
    permit: SceneEditFreezePermit<O, PP, U>,
    reject: impl FnOnce(SceneEditFrozenGenerationRejection<O, PP, U>) -> R,
    prepare: impl for<'joint> FnOnce(
        SceneEditHandoffToken,
        SceneEditJointCommit<'joint, O, PP, U>,
        CanonicalJointSeal<'joint>,
    ) -> R,
) -> Result<R, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let generation = match crate::identity::SceneGenerationId::reserve() {
        Ok(generation) => generation,
        Err(SceneGenerationError::Exhausted) => {
            return permit
                .reject_generation(
                    SceneEditGenerationRejection,
                    SCENE_EDIT_GENERATION_REJECTION_CODE,
                )
                .map(reject);
        }
    };

    Ok(permit.bind_joint_commit(|output, proof, joint, seal| {
        prepare(
            SceneEditHandoffToken {
                generation,
                proof,
                tables: output.scene.into_tables(),
            },
            joint,
            seal,
        )
    }))
}
