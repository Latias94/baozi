//! Generation admission and rooted handoff for a validated geometry edit.
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
use crate::identity::{PrimitiveId, ReservedSceneGeneration, SceneGenerationError};

use super::SceneGeometryEditOutput;
use super::schema::{
    SceneGeometryCornerOriginsTag, SceneGeometryEditEvidenceTag, SceneGeometryFaceOffsetsTag,
};
use super::validation::SceneGeometryEditValidationProof;

const GEOMETRY_EDIT_GENERATION_REJECTION_CODE: u32 = GENERATION_EXHAUSTED_CODE;

/// Core's final generation-admission rejection for a geometry edit.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("geometry-edit Scene generation admission was exhausted")]
pub struct SceneGeometryEditGenerationRejection;

impl SceneGeometryEditGenerationRejection {
    pub const fn stable_code(self) -> u32 {
        GEOMETRY_EDIT_GENERATION_REJECTION_CODE
    }
}

/// The operation/core paired freeze permit for one validated geometry edit.
pub type SceneGeometryEditFreezePermit<O, PP, U> = CanonicalFreezePermit<
    O,
    PP,
    U,
    SceneGeometryEditOutput,
    SceneGeometryEditValidationProof,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    SceneGeometryEditEvidenceTag,
>;

/// The scoped operation contribution paired with a geometry-edit generation token.
pub type SceneGeometryEditJointCommit<'joint, O, PP, U> = CanonicalJointCommit<
    'joint,
    O,
    PP,
    U,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    SceneGeometryEditEvidenceTag,
>;

/// Rich failure typestate produced when the edit cannot consume its final generation permit.
pub type SceneGeometryEditFrozenGenerationRejection<O, PP, U> =
    CentrallyRejectedCanonicalStoreSession<
        O,
        PP,
        U,
        SceneGeometryEditGenerationRejection,
        RecordBlock,
        SceneGeometryEditEvidenceTag,
        FrozenFailureState,
    >;

/// Core-owned generation/proof half of a successful geometry-edit publication.
#[must_use = "the geometry-edit handoff token must consume the matching prepared owner"]
pub struct SceneGeometryEditHandoffToken {
    generation: ReservedSceneGeneration,
    proof: SceneGeometryEditValidationProof,
    tables: crate::canonical_scene::store::typed_storage::SceneTables,
}

/// Published effects for exactly one concrete geometry operation.
#[must_use = "published geometry evidence owns its rooted storage"]
pub enum SceneGeometryEditEvidence {
    Triangulation(SceneTriangulationEvidence),
    Normals(SceneNormalGenerationEvidence),
}

struct GeometryEvidenceOwner {
    input_target: PrimitiveId,
    output_target: PrimitiveId,
    store: PublishedOutcomeEvidenceStore<RecordBlock, SceneGeometryEditEvidenceTag>,
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
#[must_use = "the published geometry edit must be transferred to its host"]
pub struct SceneGeometryEditPublishedSuccess {
    ledger_evidence: StoreLedgerEvidence,
    evidence: SceneGeometryEditEvidence,
    scene: SceneStorage,
}

impl SceneGeometryEditPublishedSuccess {
    pub fn into_parts(
        self,
    ) -> (
        crate::ValidatedScene,
        SceneGeometryEditEvidence,
        StoreLedgerEvidence,
    ) {
        (
            crate::ValidatedScene::from_node_scene(self.scene),
            self.evidence,
            self.ledger_evidence,
        )
    }
}

impl SceneGeometryEditHandoffToken {
    /// Verifies the canonical root and publishes the matching Scene/evidence owners.
    pub fn handoff(
        self,
        store: PreparedStoreSuccess<
            RecordBlock,
            SceneDirectorySchema,
            RecordBlock,
            SceneGeometryEditEvidenceTag,
        >,
    ) -> Result<SceneGeometryEditPublishedSuccess, SceneGeometryEditHandoffPairingError> {
        if self.proof.input_target.generation() != self.proof.source_generation {
            drop(self);
            return Err(SceneGeometryEditHandoffPairingError);
        }
        let expected_root = self.proof.scene.directory().into_store();
        let published = store.handoff();
        if published.canonical.root_ref() != expected_root {
            drop(published);
            drop(self);
            return Err(SceneGeometryEditHandoffPairingError);
        }

        let input_target = self.proof.input_target;
        let output_target = input_target.rebind(self.generation.id());
        let evidence = {
            let root = published.outcome_evidence.root();
            if root.rows() != 1 || root.fields() != super::schema::ROOT_FIELDS {
                return Err(SceneGeometryEditHandoffPairingError);
            }
            let invalid = SceneGeometryEditHandoffPairingError;
            match root.scalar::<u32>(0, super::schema::field::OPERATION_KIND) {
                Some(super::schema::OPERATION_NORMALS) => {
                    if root.scalar::<u8>(0, super::schema::field::CORNER_ORIGINS) != Some(0)
                        || root.scalar::<u8>(0, super::schema::field::FACE_OFFSETS) != Some(0)
                    {
                        return Err(SceneGeometryEditHandoffPairingError);
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
                    SceneGeometryEditEvidence::Normals(SceneNormalGenerationEvidence {
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
                    SceneGeometryEditEvidence::Triangulation(SceneTriangulationEvidence {
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
                _ => return Err(invalid),
            }
        };

        Ok(SceneGeometryEditPublishedSuccess {
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

/// A prepared canonical owner did not match the core proof root or evidence shape.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("prepared geometry-edit owner does not match the core validation proof")]
pub struct SceneGeometryEditHandoffPairingError;

/// Consumes the final geometry-edit freeze permit and reserves its destination generation.
///
/// The paired joint and seal are scoped to the callback, so the operation owner can prepare the
/// exact store while core retains the only generation/proof token.
#[allow(clippy::type_complexity)]
pub fn consume_scene_edit_freeze_permit<O, PP, U, R>(
    permit: SceneGeometryEditFreezePermit<O, PP, U>,
    reject: impl FnOnce(SceneGeometryEditFrozenGenerationRejection<O, PP, U>) -> R,
    prepare: impl for<'joint> FnOnce(
        SceneGeometryEditHandoffToken,
        SceneGeometryEditJointCommit<'joint, O, PP, U>,
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
                    SceneGeometryEditGenerationRejection,
                    GEOMETRY_EDIT_GENERATION_REJECTION_CODE,
                )
                .map(reject);
        }
    };

    Ok(permit.bind_joint_commit(|output, proof, joint, seal| {
        prepare(
            SceneGeometryEditHandoffToken {
                generation,
                proof,
                tables: output.scene.into_tables(),
            },
            joint,
            seal,
        )
    }))
}
