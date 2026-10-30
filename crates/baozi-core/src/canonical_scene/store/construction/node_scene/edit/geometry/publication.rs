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

/// Published remap evidence retained beside the edited Scene.
#[must_use = "published geometry evidence owns the rooted remap storage"]
pub struct SceneGeometryEditEvidence {
    input_target: PrimitiveId,
    output_target: PrimitiveId,
    corner_origins: baozi_operation::OutcomeEvidenceSequenceRef<u32, SceneGeometryCornerOriginsTag>,
    face_offsets: baozi_operation::OutcomeEvidenceSequenceRef<u32, SceneGeometryFaceOffsetsTag>,
    store: PublishedOutcomeEvidenceStore<RecordBlock, SceneGeometryEditEvidenceTag>,
}

impl SceneGeometryEditEvidence {
    pub const fn input_target(&self) -> PrimitiveId {
        self.input_target
    }

    pub const fn output_target(&self) -> PrimitiveId {
        self.output_target
    }

    /// Returns the published flattened source-corner relation without copying it.
    pub fn source_corner_origins(&self) -> &[u32] {
        self.store
            .sequence(self.corner_origins)
            .map(|view| view.as_slice())
            .unwrap_or_else(|| panic!("published geometry corner-origin evidence is missing"))
    }

    /// Returns the published source-face triangle-prefix relation without copying it.
    pub fn source_face_triangle_offsets(&self) -> &[u32] {
        self.store
            .sequence(self.face_offsets)
            .map(|view| view.as_slice())
            .unwrap_or_else(|| panic!("published geometry face-offset evidence is missing"))
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
        let (corner_origins, face_offsets) = {
            let root = published.outcome_evidence.root();
            if root.rows() != 1 || root.fields() != super::schema::ROOT_FIELDS {
                return Err(SceneGeometryEditHandoffPairingError);
            }
            let Some(corners) = root.sequence_ref::<u32, SceneGeometryCornerOriginsTag>(
                0,
                super::schema::field::CORNER_ORIGINS,
            ) else {
                return Err(SceneGeometryEditHandoffPairingError);
            };
            let Some(faces) = root.sequence_ref::<u32, SceneGeometryFaceOffsetsTag>(
                0,
                super::schema::field::FACE_OFFSETS,
            ) else {
                return Err(SceneGeometryEditHandoffPairingError);
            };
            (corners, faces)
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
            evidence: SceneGeometryEditEvidence {
                input_target,
                output_target,
                corner_origins,
                face_offsets,
                store: published.outcome_evidence,
            },
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
