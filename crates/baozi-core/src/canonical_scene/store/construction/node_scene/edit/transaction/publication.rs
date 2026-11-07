//! Generation admission and rooted handoff for a validated Scene edit.
//!
//! Core admits the actual proven roots before reserving a generation and owns every terminal
//! branch. Published evidence keeps the remap leaves alive independently of the new Scene.

use baozi_operation::__retained_store_integration::activation::{
    CanonicalCauseSlot, CanonicalFreezePermit, CanonicalJointSuccess, StoreActivationContainment,
    StoreCanonicalRejection,
};
use baozi_operation::__retained_store_integration::authority::PreparedStoreSuccess;
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, PublishedCanonicalStore,
    PublishedOutcomeEvidenceStore, PublishedStoreFallbackParts, RecordBlock, StoreLedgerEvidence,
    StorePrepareError,
};
use thiserror::Error;

#[cfg(test)]
use crate::attribute::AttributeDomain;
use crate::canonical_scene::store::construction::GENERATION_EXHAUSTED_CODE;
use crate::canonical_scene::store::construction::node_scene::SceneStorage;
use crate::canonical_scene::store::schema::SceneDirectorySchema;
use crate::identity::{
    PrimitiveId, ReservedSceneGeneration, SceneGenerationError, SceneGenerationId,
};
use crate::scene_space::SceneSpaceField;
use crate::space_conversion::TargetSpace;

use super::SceneEditOutput;
use super::schema::{
    SceneEditEvidenceTag, SceneGeometryCornerOriginsTag, SceneGeometryFaceOffsetsTag,
};
use super::validation::{SceneEditHandoffFacts, SceneEditValidationProof};

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
pub type SceneEditFreezePermit<'inv, O, PP, U> = CanonicalFreezePermit<
    'inv,
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

/// Core-owned generation/proof half of a successful Scene edit publication.
#[must_use = "the Scene edit handoff token must consume the matching prepared owner"]
struct SceneEditHandoffToken {
    generation: ReservedSceneGeneration,
    proof: SceneEditValidationProof,
    tables: crate::canonical_scene::store::typed_storage::SceneTables,
}

/// Published effects for exactly one concrete Scene operation.
#[must_use = "published Scene edit evidence owns its rooted storage"]
pub enum SceneEditEvidence {
    #[cfg(test)]
    Triangulation(SceneTriangulationEvidence),
    #[cfg(test)]
    Normals(SceneNormalGenerationEvidence),
    #[cfg(test)]
    Tangents(SceneTangentGenerationEvidence),
    SpaceConversion(SceneSpaceConversionEvidence),
    Selection(SceneGeometrySelectionEvidence),
}

#[cfg(test)]
struct GeometryEvidenceOwner {
    input_target: PrimitiveId,
    output_target: PrimitiveId,
    store: PublishedOutcomeEvidenceStore<RecordBlock, SceneEditEvidenceTag>,
}

/// Centrally proved whole-scene effects with the actual published evidence root owner.
/// Primitive-local remaps are deliberately absent; triangle reversal uses `[0, 2, 1]`.
pub struct SceneSpaceConversionEvidence {
    _store: PublishedOutcomeEvidenceStore<RecordBlock, SceneEditEvidenceTag>,
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
    source: TargetSpace,
    target: TargetSpace,
    assumed_mask: u32,
    reverse_winding: bool,
}

impl SceneSpaceConversionEvidence {
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.input_generation
    }
    pub const fn output_generation(&self) -> SceneGenerationId {
        self.output_generation
    }
    pub const fn source(&self) -> TargetSpace {
        self.source
    }
    pub const fn target(&self) -> TargetSpace {
        self.target
    }
    pub const fn reverses_triangle_winding(&self) -> bool {
        self.reverse_winding
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
        self.assumed_mask & bit != 0
    }
}

/// Triangulation evidence with both centrally validated origin relations present.
#[cfg(test)]
pub struct SceneTriangulationEvidence {
    owner: GeometryEvidenceOwner,
    corner_origins: baozi_operation::OutcomeEvidenceSequenceRef<u32, SceneGeometryCornerOriginsTag>,
    face_offsets: baozi_operation::OutcomeEvidenceSequenceRef<u32, SceneGeometryFaceOffsetsTag>,
    source_corners: u32,
    vertices: u32,
}

#[cfg(test)]
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
#[cfg(test)]
pub struct SceneNormalGenerationEvidence {
    owner: GeometryEvidenceOwner,
    effects: u32,
    removed_metadata: u32,
    normal_attribute: u32,
}

#[cfg(test)]
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

/// Tangent-generation facts retained independently of the published Scene.
#[cfg(test)]
pub struct SceneTangentGenerationEvidence {
    owner: GeometryEvidenceOwner,
    replaced: bool,
    tangent_attribute: u32,
    corners: u32,
    normal_domain: AttributeDomain,
    uv_set: u32,
    uv_domain: AttributeDomain,
}

#[cfg(test)]
impl SceneTangentGenerationEvidence {
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.owner.input_target.generation()
    }

    pub const fn output_generation(&self) -> SceneGenerationId {
        self.owner.output_target.generation()
    }

    pub const fn input_target(&self) -> PrimitiveId {
        self.owner.input_target
    }

    pub const fn output_target(&self) -> PrimitiveId {
        self.owner.output_target
    }

    pub const fn replaced_tangent(&self) -> bool {
        self.replaced
    }

    pub const fn tangent_attribute_index(&self) -> u32 {
        self.tangent_attribute
    }

    pub const fn corner_count(&self) -> u32 {
        self.corners
    }

    pub const fn normal_domain(&self) -> AttributeDomain {
        self.normal_domain
    }

    pub const fn uv_set(&self) -> u32 {
        self.uv_set
    }

    pub const fn uv_domain(&self) -> AttributeDomain {
        self.uv_domain
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
    fn handoff(
        self,
        store: PreparedStoreSuccess<
            RecordBlock,
            SceneDirectorySchema,
            RecordBlock,
            SceneEditEvidenceTag,
        >,
    ) -> SceneEditPublishedSuccess {
        let published = store.handoff();
        let (scene, evidence) =
            self.handoff_physical(published.canonical, published.outcome_evidence);
        SceneEditPublishedSuccess {
            scene,
            evidence,
            ledger_evidence: published.ledger_evidence,
        }
    }

    fn handoff_physical(
        self,
        canonical: PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>,
        outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, SceneEditEvidenceTag>,
    ) -> (SceneStorage, SceneEditEvidence) {
        let evidence = match self.proof.handoff {
            SceneEditHandoffFacts::Selection {
                input_generation,
                targets,
                reconstruction_work_items,
                geometry_reconstruction_count,
                reconstructed_primitive_count,
                removed_metadata,
                origins,
            } => SceneEditEvidence::Selection(SceneGeometrySelectionEvidence {
                store: outcome_evidence,
                input_generation,
                output_generation: self.generation.id(),
                targets,
                reconstruction_work_items,
                geometry_reconstruction_count,
                reconstructed_primitive_count,
                removed_metadata,
                corners: origins.corners,
                faces: origins.faces,
            }),
            SceneEditHandoffFacts::SpaceConversion {
                input_generation,
                source,
                target,
                assumed_mask,
                reverse_winding,
            } => SceneEditEvidence::SpaceConversion(SceneSpaceConversionEvidence {
                _store: outcome_evidence,
                input_generation,
                output_generation: self.generation.id(),
                source,
                target,
                assumed_mask,
                reverse_winding,
            }),
            #[cfg(test)]
            SceneEditHandoffFacts::Triangulation {
                input_target,
                origins,
                source_corners,
                vertices,
            } => SceneEditEvidence::Triangulation(SceneTriangulationEvidence {
                owner: GeometryEvidenceOwner {
                    input_target,
                    output_target: input_target.rebind(self.generation.id()),
                    store: outcome_evidence,
                },
                corner_origins: origins.corners,
                face_offsets: origins.faces,
                source_corners,
                vertices,
            }),
            #[cfg(test)]
            SceneEditHandoffFacts::Normals {
                input_target,
                effects,
                removed_metadata,
                normal_attribute,
            } => SceneEditEvidence::Normals(SceneNormalGenerationEvidence {
                owner: GeometryEvidenceOwner {
                    input_target,
                    output_target: input_target.rebind(self.generation.id()),
                    store: outcome_evidence,
                },
                effects,
                removed_metadata,
                normal_attribute,
            }),
            #[cfg(test)]
            SceneEditHandoffFacts::Tangents {
                input_target,
                replaced,
                tangent_attribute,
                corners,
                normal_domain,
                uv_set,
                uv_domain,
            } => SceneEditEvidence::Tangents(SceneTangentGenerationEvidence {
                owner: GeometryEvidenceOwner {
                    input_target,
                    output_target: input_target.rebind(self.generation.id()),
                    store: outcome_evidence,
                },
                replaced,
                tangent_attribute,
                corners,
                normal_domain,
                uv_set,
                uv_domain,
            }),
        };

        (
            SceneStorage {
                generation: self.generation.into_id(),
                tables: self.tables,
                canonical,
                proof: self.proof.scene,
                #[cfg(test)]
                semantic_read_fault: None,
            },
            evidence,
        )
    }
}

/// The exact owned terminal publication from a centrally validated Scene edit.
#[must_use = "the Scene edit publication owns its terminal storage and ledger evidence"]
#[allow(
    clippy::large_enum_variant,
    reason = "publication transfers the exact Scene and physical owners without a new allocation after commit"
)]
pub enum SceneEditPublication {
    Success(SceneEditPublishedSuccess),
    MinimalFallback {
        published: PublishedStoreFallbackParts,
    },
}

/// A precommit failure to admit or prepare the Scene edit's terminal owners.
#[derive(Debug, Error)]
pub enum SceneEditPublicationError {
    #[error(transparent)]
    Containment(StoreActivationContainment),
    #[error(transparent)]
    Preparation(StorePrepareError),
}

/// Admits recovery and both proven roots before reserving the destination generation.
///
/// Core prepares the exact joint outcome and transfers its owners without a caller finalizer.
/// Every evidence predicate is proved by central validation before this permit is issued.
pub fn consume_scene_edit_freeze_permit<'inv, O, PP, U>(
    permit: SceneEditFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<'inv, StoreCanonicalRejection<SceneEditGenerationRejection>>,
) -> Result<SceneEditPublication, SceneEditPublicationError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    consume_scene_edit_freeze_permit_with_generation_source(
        permit,
        rejection_slot,
        SceneGenerationId::reserve,
    )
}

fn consume_scene_edit_freeze_permit_with_generation_source<'inv, O, PP, U>(
    permit: SceneEditFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<'inv, StoreCanonicalRejection<SceneEditGenerationRejection>>,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<SceneEditPublication, SceneEditPublicationError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (permit, (reserve_generation, (), ())) = permit
        .admit_generation_rejection(&rejection_slot, (reserve_generation, (), ()))
        .map_err(SceneEditPublicationError::Containment)?;
    let canonical_root = permit.proof().scene.directory().into_store();
    let evidence_root = permit.proof().evidence_root;
    let (permit, reserve_generation) = permit
        .admit_generation_pairing(canonical_root, evidence_root, reserve_generation)
        .map_err(SceneEditPublicationError::Containment)?;
    let generation = match reserve_generation() {
        Ok(generation) => generation,
        Err(SceneGenerationError::Exhausted) => {
            let rejected = permit
                .reject_generation(
                    rejection_slot,
                    SceneEditGenerationRejection,
                    SCENE_EDIT_GENERATION_REJECTION_CODE,
                )
                .map_err(SceneEditPublicationError::Containment)?;
            let (prepared, _) = rejected
                .prepare_minimal_fallback()
                .map_err(SceneEditPublicationError::Preparation)?
                .into_parts();
            return Ok(SceneEditPublication::MinimalFallback {
                published: prepared.handoff(),
            });
        }
    };

    permit.bind_joint_commit(|joint, seal| {
        let CanonicalJointSuccess {
            output,
            proof,
            store,
        } = joint
            .prepare(seal)
            .map_err(SceneEditPublicationError::Containment)?;
        let token = SceneEditHandoffToken {
            generation,
            proof,
            tables: output.scene.into_tables(),
        };
        Ok(SceneEditPublication::Success(token.handoff(store)))
    })
}

/// The rooted selected-target descriptors and origins from one atomic publication.
pub struct SceneGeometrySelectionEvidence {
    store: PublishedOutcomeEvidenceStore<RecordBlock, SceneEditEvidenceTag>,
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
    targets: u32,
    reconstruction_work_items: u64,
    geometry_reconstruction_count: u32,
    reconstructed_primitive_count: u32,
    removed_metadata: u32,
    corners: baozi_operation::OutcomeEvidenceSequenceRef<u32, SceneGeometryCornerOriginsTag>,
    faces: baozi_operation::OutcomeEvidenceSequenceRef<u32, SceneGeometryFaceOffsetsTag>,
}

impl SceneGeometrySelectionEvidence {
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.input_generation
    }
    pub const fn output_generation(&self) -> SceneGenerationId {
        self.output_generation
    }
    pub const fn reconstruction_work_items(&self) -> u64 {
        self.reconstruction_work_items
    }
    pub const fn geometry_reconstruction_count(&self) -> u32 {
        self.geometry_reconstruction_count
    }
    pub const fn reconstructed_primitive_count(&self) -> u32 {
        self.reconstructed_primitive_count
    }
    pub const fn removed_metadata_entries(&self) -> u32 {
        self.removed_metadata
    }
    pub const fn len(&self) -> usize {
        self.targets as usize
    }
    pub const fn is_empty(&self) -> bool {
        self.targets == 0
    }
    pub fn target(&self, index: usize) -> Option<SceneGeometryTargetEvidence<'_>> {
        if index >= self.len() {
            return None;
        }
        let all = self.store.sequence(self.faces)?.as_slice();
        let words = all.get(
            index * super::selected::TARGET_WORDS..(index + 1) * super::selected::TARGET_WORDS,
        )?;
        Some(SceneGeometryTargetEvidence { owner: self, words })
    }
}

#[derive(Clone, Copy)]
pub struct SceneGeometryTargetEvidence<'a> {
    owner: &'a SceneGeometrySelectionEvidence,
    words: &'a [u32],
}
impl SceneGeometryTargetEvidence<'_> {
    /// Whether this target actually lowered topology or generated/removed a field.
    pub fn changed(self) -> bool {
        self.words[10] & 0x0f != 0
    }
    pub fn input_target(self) -> PrimitiveId {
        PrimitiveId::new(
            crate::identity::LogicalMeshId::new(self.owner.input_generation, self.words[0]),
            self.words[1],
        )
    }
    pub fn output_target(self) -> PrimitiveId {
        self.input_target().rebind(self.owner.output_generation)
    }
    pub fn source_corner_count(self) -> u32 {
        self.words[4]
    }
    pub fn vertex_count(self) -> u32 {
        self.words[3]
    }
    pub fn output_corner_count(self) -> u32 {
        self.words[6]
    }
    pub fn output_face_count(self) -> u32 {
        self.words[7]
    }
    pub fn normal_attribute_index(self) -> Option<u32> {
        (self.words[8] != u32::MAX).then_some(self.words[8])
    }
    pub fn tangent_attribute_index(self) -> Option<u32> {
        (self.words[9] != u32::MAX).then_some(self.words[9])
    }
    pub fn lowered(self) -> bool {
        self.words[10] & 1 != 0
    }
    pub fn removed_tangent(self) -> bool {
        self.words[10] & 8 != 0
    }
    pub fn replaced_normal(self) -> bool {
        self.words[10] & 16 != 0
    }
    pub fn replaced_tangent(self) -> bool {
        self.words[10] & 32 != 0
    }
    pub fn smooth(self) -> bool {
        self.words[10] & 64 != 0
    }
}
impl<'a> SceneGeometryTargetEvidence<'a> {
    pub fn corner_origins(self) -> Option<&'a [u32]> {
        if !self.lowered() {
            return None;
        }
        self.owner
            .store
            .sequence(self.owner.corners)?
            .as_slice()
            .get(self.words[11] as usize..self.words[11] as usize + self.words[6] as usize)
    }
    pub fn face_offsets(self) -> Option<&'a [u32]> {
        if !self.lowered() {
            return None;
        }
        self.owner
            .store
            .sequence(self.owner.faces)?
            .as_slice()
            .get(self.words[12] as usize..self.words[12] as usize + self.words[13] as usize)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    use baozi_operation::__retained_store_integration::activation::{
        CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
        CanonicalFreezePermitIssue, CanonicalValidationDecision, CanonicalValidationProgress,
    };
    use baozi_operation::__retained_store_integration::authority::{
        FiniteRetainedStoreStartPlan, FiniteStoreLedgerPlan,
    };
    use baozi_operation::{
        AccountPoisonReason, OperationControl, StoreDimensionKey, StorePhaseKey,
        StorePreparationFailureClass, StoreTerminalCause,
    };

    use crate::SceneValidationLimits;
    use crate::canonical_scene::store::construction::node_scene::edit::tests::try_build_single_trs_scene;
    use crate::canonical_scene::store::construction::node_scene::materialization::node_scene_activation_roles;
    use crate::canonical_scene::store::construction::node_scene::scratch::ScopedNodeSceneValidationScratchOwner;
    use crate::canonical_scene::store::construction::node_scene::tests::{
        test_account, test_factory, test_limits,
    };
    use crate::canonical_scene::store::construction::{
        STORE_DIMENSION, SceneCanonicalInvocationHost, SceneConstructionPhase,
        SceneConstructionUnit, SceneFailureEvidenceWork, SceneStoreOperation, final_progress,
        try_plan_scene_operation,
    };

    use super::super::{SceneEditBootstrap, SceneEditRejection, resolve_scene_space_conversion};
    use super::*;

    struct GenerationCapture<'a> {
        drops: &'a Cell<usize>,
        account: &'a baozi_operation::PhysicalStorageAccount,
        retained_before: u64,
    }

    impl Drop for GenerationCapture<'_> {
        fn drop(&mut self) {
            let snapshot = self.account.snapshot();
            assert!(!snapshot.poisoned());
            assert!(snapshot.live_retained_bytes() > self.retained_before);
            self.drops.set(self.drops.get() + 1);
        }
    }

    fn finite_scene_edit_plans() -> (
        FiniteStoreLedgerPlan,
        FiniteRetainedStoreStartPlan,
        [StoreDimensionKey; 6],
        [StorePhaseKey; 3],
    ) {
        use baozi_operation::__retained_store_integration::authority::{
            RetainedStoreSharePlan, RetainedStoreWriterPlan,
        };
        use baozi_operation::{
            DimensionPolicy, RetainedStoreBatchLimits, RetainedStoreFailureTerminalWorkPlan,
            StoreDimensionSemanticId, StoreFallbackEnvelope, StoreKeyDomain, StorePhaseSemanticId,
            StoreSemanticCatalogId,
        };
        use std::num::{NonZeroU32, NonZeroU64};

        let catalog = StoreSemanticCatalogId::new(*b"baozi-unit-store", NonZeroU32::MIN);
        let domain = StoreKeyDomain::try_new(catalog).unwrap();
        let dimensions = [1, 2, 3, 4, 5, 6].map(|local| {
            domain
                .try_admit_dimension(StoreDimensionSemanticId::new(
                    catalog,
                    NonZeroU64::new(local).unwrap(),
                ))
                .unwrap()
        });
        let phases = [1, 2, 3].map(|local| {
            domain
                .try_admit_phase(StorePhaseSemanticId::new(
                    catalog,
                    NonZeroU64::new(local).unwrap(),
                ))
                .unwrap()
        });
        let requirement =
            RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_members(1).unwrap();
        let failure = RetainedStoreFailureTerminalWorkPlan::new(
            dimensions[5],
            1,
            1280 + requirement.attempted_work_limit(),
            1280 + requirement.consumed_work_limit(),
        )
        .unwrap();
        let mut policies = dimensions.map(|dimension| {
            (
                dimension,
                DimensionPolicy::reservable_bytes(256, 256, 256, 1 << 20)
                    .with_published_outcome_evidence_limit(256),
            )
        });
        policies[5].1 = requirement.dimension_policy();
        let ledger =
            FiniteStoreLedgerPlan::try_new(domain, 8 << 20, 8 << 20, policies, phases).unwrap();
        let plan = FiniteRetainedStoreStartPlan::new(
            RetainedStoreWriterPlan::new(
                phases[2],
                dimensions[0],
                dimensions[1],
                RetainedStoreBatchLimits::new(4, 2).unwrap(),
            ),
            RetainedStoreSharePlan::new(dimensions[2], dimensions[3]),
            StoreFallbackEnvelope::new(23),
            dimensions[4],
            1,
            failure,
        );
        (ledger, plan, dimensions, phases)
    }

    #[test]
    fn finite_scene_edit_entry_keeps_original_facts_after_source_and_host_end() {
        use baozi_operation::__retained_store_integration::activation::{
            FiniteCanonicalInvocationHost, FiniteCanonicalOperationPublication,
        };
        use baozi_operation::__retained_store_integration::authority::PreparedFiniteStoreEvidenceOnlyOutcome;
        use baozi_operation::StoreRuntimeError;

        for attached in [false, true] {
            let (facts, work_dimension) = {
                let factory = test_factory();
                let account = test_account(&factory, 1_024);
                let generations = AtomicU64::new(0);
                let source = try_build_single_trs_scene(&account, || {
                    SceneGenerationId::reserve_from(&generations)
                })
                .unwrap();
                let scene = crate::ValidatedScene::from_node_scene(source.scene);
                let (ledger, plan, dimensions, _) = finite_scene_edit_plans();
                let mut host = FiniteCanonicalInvocationHost::<
                    crate::canonical_scene::store::construction::SceneConstructionOwner,
                    SceneConstructionPhase,
                    SceneConstructionUnit,
                    (),
                    (),
                >::new();
                let finish = super::super::run_finite_scene_edit_body(
                    &mut host,
                    &scene,
                    OperationControl::default(),
                    ledger,
                    plan,
                    |mut parts, disposition| {
                        parts
                            .startup
                            .with_action_execution(dimensions[2], |actions| {
                                actions.begin_action(1)?;
                                actions.complete_action()?;
                                Ok::<_, StoreRuntimeError>(())
                            })
                            .unwrap();
                        if attached {
                            parts
                                .startup
                                .attach()
                                .unwrap()
                                .request_unchanged(disposition)
                                .unwrap();
                        } else {
                            parts.startup.request_unchanged(disposition).unwrap();
                        }
                        Ok(())
                    },
                )
                .unwrap();
                let facts = if attached {
                    match finish.finish_attached(final_progress()).unwrap() {
                        FiniteCanonicalOperationPublication::EvidenceOnly(
                            PreparedFiniteStoreEvidenceOnlyOutcome::Success(prepared),
                        ) => prepared.handoff().facts,
                        _ => panic!("an unchanged finite Scene edit must publish its fixed facts"),
                    }
                } else {
                    finish.finish_unattached(final_progress()).unwrap()
                };
                assert!(!account.snapshot().poisoned());
                (facts, dimensions[2])
            };
            assert_eq!(facts.header().dimension_count(), 6);
            assert_eq!(facts.header().phase_count(), 3);
            assert_eq!(facts.header().terminal_cause(), StoreTerminalCause::Success);
            let work = facts.dimension(work_dimension).unwrap();
            assert!(work.consumed() >= 1);
            assert_eq!(work.attempted(), work.consumed());
        }
    }

    #[test]
    fn finite_scene_edit_entry_returns_owned_session_error_after_source_and_host_end() {
        use baozi_operation::__retained_store_integration::activation::{
            CanonicalActivationSessionPolicy, CanonicalFailureRequirements,
            CanonicalMaterializationRole, CanonicalValidationRole, FiniteCanonicalInvocationHost,
            RetainedStoreWriterRole,
        };
        use baozi_operation::__retained_store_integration::authority::{
            FiniteStoreExitCause, FiniteStoreFinalizeCause,
        };
        use baozi_operation::__retained_store_integration::producer::CanonicalTableShape;
        use baozi_operation::{RetainedStoreBatchLimits, StoreKeyDomain, StoreRuntimeError};

        let original_containment = Cell::new(None);
        let (error, dimensions, phases, foreign_dimension) = {
            let factory = test_factory();
            let account = test_account(&factory, 1_024);
            let generations = AtomicU64::new(0);
            let source = try_build_single_trs_scene(&account, || {
                SceneGenerationId::reserve_from(&generations)
            })
            .unwrap();
            let scene = crate::ValidatedScene::from_node_scene(source.scene);
            let (ledger, plan, dimensions, phases) = finite_scene_edit_plans();
            let foreign_domain =
                StoreKeyDomain::try_new(dimensions[2].semantic_id().catalog()).unwrap();
            let foreign_dimension = foreign_domain
                .try_admit_dimension(dimensions[2].semantic_id())
                .unwrap();
            let mut host = FiniteCanonicalInvocationHost::<
                crate::canonical_scene::store::construction::SceneConstructionOwner,
                SceneConstructionPhase,
                SceneConstructionUnit,
                (),
                (),
            >::new();
            let error = super::super::run_finite_scene_edit_body(
                &mut host,
                &scene,
                OperationControl::default(),
                ledger,
                plan,
                |mut parts, _| {
                    parts
                        .startup
                        .with_action_execution(dimensions[2], |actions| {
                            actions.begin_action(1)?;
                            actions.complete_action()?;
                            Ok::<_, StoreRuntimeError>(())
                        })
                        .unwrap();
                    let operation = parts.startup.attach().unwrap();
                    let _owner = operation
                        .into_session_outcome_owner(
                            CanonicalActivationSessionPolicy::new(
                                RetainedStoreWriterRole::new(
                                    phases[1],
                                    dimensions[0],
                                    dimensions[1],
                                ),
                                RetainedStoreWriterRole::new(
                                    phases[1],
                                    dimensions[0],
                                    dimensions[1],
                                ),
                                CanonicalMaterializationRole::new(dimensions[2], dimensions[3]),
                                CanonicalValidationRole::new(
                                    phases[2],
                                    dimensions[2],
                                    dimensions[3],
                                ),
                                dimensions[2],
                                dimensions[4],
                            )
                            .with_semantic_bytes_dimension(foreign_dimension),
                            RetainedStoreBatchLimits::new(2, 2).unwrap(),
                            CanonicalFailureRequirements::new(
                                CanonicalTableShape::new(1, 1).unwrap(),
                            ),
                        )
                        .inspect_err(|error| {
                            assert!(error.has_residue());
                            let FiniteStoreExitCause::Finalize(
                                FiniteStoreFinalizeCause::Publication(
                                    StorePrepareError::FinalizationContainment(cause),
                                ),
                            ) = error.cause()
                            else {
                                panic!("session rejection must own its original containment cause")
                            };
                            original_containment.set(Some(*cause));
                        })?;
                    panic!("a foreign session key must reject before activation");
                },
            )
            .err()
            .expect("the finite Scene adapter must return the already-owned session error");
            (error, dimensions, phases, foreign_dimension)
        };

        let FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Publication(
            StorePrepareError::FinalizationContainment(cause),
        )) = error.cause()
        else {
            panic!("the adapter must preserve the session error after source and Host exit")
        };
        assert_eq!(Some(*cause), original_containment.get());
        assert_eq!(
            cause.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        assert!(error.has_residue());
        let domain = error
            .domain()
            .expect("the error must own the original domain");
        for dimension in dimensions {
            assert_eq!(
                domain.try_admit_dimension(dimension.semantic_id()),
                Some(dimension)
            );
        }
        for phase in phases {
            assert_eq!(domain.try_admit_phase(phase.semantic_id()), Some(phase));
        }
        assert_ne!(
            domain.try_admit_dimension(foreign_dimension.semantic_id()),
            Some(foreign_dimension)
        );
    }

    #[test]
    fn scene_edit_publication_admits_recovery_and_proof_roots_before_source() {
        let _: fn(
            SceneEditHandoffToken,
            PreparedStoreSuccess<
                RecordBlock,
                SceneDirectorySchema,
                RecordBlock,
                SceneEditEvidenceTag,
            >,
        ) -> SceneEditPublishedSuccess = SceneEditHandoffToken::handoff;
        for (foreign_slot, foreign_canonical) in [(false, false), (true, false), (false, true)] {
            for exhausted in [false, true] {
                let factory = test_factory();
                let account = test_account(&factory, 1_024);
                let generation_source = AtomicU64::new(0);
                let source = try_build_single_trs_scene(&account, || {
                    SceneGenerationId::reserve_from(&generation_source)
                })
                .expect("single-node edit fixture must publish success");
                let scene = crate::ValidatedScene::from_node_scene(source.scene);
                let input_generation = scene.generation();
                let retained = account.snapshot();
                if exhausted {
                    generation_source.store(u64::MAX, Ordering::Relaxed);
                }
                let generation_before = generation_source.load(Ordering::Relaxed);
                let generation_calls = Cell::new(0_usize);
                let generation_drops = Cell::new(0_usize);
                let (writer, failure, admission) =
                    SceneEditBootstrap::try_new().unwrap().into_parts();
                let start = try_plan_scene_operation(
                    test_limits(),
                    OperationControl::default(),
                    256,
                    SceneFailureEvidenceWork {
                        max_evidence_members: SceneEditBootstrap::success_evidence_members(),
                        construction_work_limit: SceneEditBootstrap::failure_field_work(),
                        validation_work_limit: SceneEditBootstrap::failure_field_work(),
                    },
                )
                .unwrap();
                let mut invocation_host = SceneCanonicalInvocationHost::<
                    SceneEditOutput,
                    SceneEditValidationProof,
                    SceneEditRejection,
                    SceneEditRejection,
                    SceneEditGenerationRejection,
                    SceneEditEvidenceTag,
                >::new();
                let mut foreign_host = SceneCanonicalInvocationHost::<
                    (),
                    (),
                    (),
                    (),
                    SceneEditGenerationRejection,
                    SceneEditEvidenceTag,
                >::new();
                {
                    let source_fields = [
                        SceneSpaceField::Handedness,
                        SceneSpaceField::UpAxis,
                        SceneSpaceField::FrontAxis,
                        SceneSpaceField::UnitScaleToMeters,
                        SceneSpaceField::FrontFaceWinding,
                    ];
                    let mut assumed_source_fields = None;
                    let mut inputs = Some((start, writer, failure, admission));
                    let finish = invocation_host
                        .run_body(|parts, disposition| {
                            let (start, writer, failure, admission) = inputs.take().unwrap();
                            let operation = SceneStoreOperation::try_start_derived(
                                parts.startup,
                                start.control,
                                start.ledger_plan,
                                &scene.as_node_scene().canonical,
                                start.start_plan,
                            )
                            .unwrap();
                            let owner = operation
                                .into_session_outcome_owner(
                                    node_scene_activation_roles(start.keys).session_policy(),
                                    writer,
                                    failure,
                                )
                                .unwrap();
                            let resolved = resolve_scene_space_conversion(
                                &scene,
                                TargetSpace::STANDARD,
                                Some(TargetSpace::STANDARD),
                            )
                            .unwrap();
                            assert!(!resolved.is_unchanged());
                            let mut transaction = admission
                                .into_space_transaction(resolved, SceneValidationLimits::default());
                            let activated = owner
                                .plan_and_activate_scoped(
                                    parts.output,
                                    parts.activation,
                                    |mut planning| {
                                        let (sealed, completion) = planning
                                            .with_semantic_work(|semantic| {
                                                transaction.preflight(semantic)
                                            })
                                            .unwrap();
                                        match planning.seal_activation(sealed, completion) {
                                            CanonicalActivationSeal::Sealed(permit) => {
                                                permit.activate(|lane, plan| plan.materialize(lane))
                                            }
                                            CanonicalActivationSeal::Unsealed(failure) => {
                                                failure.finish()
                                            }
                                        }
                                    },
                                )
                                .unwrap();
                            let postflight = transaction.finish();
                            let CanonicalActivationOutcome::Activated(activated) = activated else {
                                panic!("valid space edit must materialize")
                            };
                            let central = activated
                                .validate_centrally(
                                    parts.proof,
                                    parts.validation,
                                    CanonicalValidationProgress::new(
                                        SceneConstructionPhase::Validation,
                                        SceneConstructionUnit::WorkUnits,
                                    ),
                                    |lane, output| postflight.build_evidence(lane, output),
                                    |planning, output| {
                                        if !foreign_canonical {
                                            return postflight.validate_candidate(planning, output);
                                        }
                                        let scratch_plan =
                                            output.scene.inner().central_validation.scratch_plan();
                                        planning.validate_with(
                                            |checkpoint| {
                                                ScopedNodeSceneValidationScratchOwner::try_allocate(
                                                    checkpoint,
                                                    scratch_plan,
                                                )
                                            },
                                            |session, directory, evidence, scratch| {
                                                let mut proof = postflight
                                            .prove_candidate(
                                                session, directory, evidence, scratch, output,
                                            )?
                                            .expect("the actual metered candidate must be valid");
                                                proof.scene = scene.as_node_scene().proof;
                                                Ok(CanonicalValidationDecision::accept(proof))
                                            },
                                        )
                                    },
                                )
                                .unwrap();
                            let CanonicalCentralValidationOutcome::Candidate(candidate) = central
                            else {
                                panic!("valid space edit must pass central validation")
                            };
                            let SceneEditHandoffFacts::SpaceConversion {
                                input_generation: proved_generation,
                                source: proved_source,
                                target: proved_target,
                                assumed_mask,
                                reverse_winding,
                            } = candidate.proof().handoff
                            else {
                                panic!("space facts must already be projected before freeze")
                            };
                            assert_eq!(proved_generation, input_generation);
                            assert_eq!(proved_source, resolved.source());
                            assert_eq!(proved_target, resolved.target());
                            assert_eq!(assumed_mask, u32::from(resolved.assumed_fields_mask()));
                            assert_eq!(reverse_winding, resolved.reverses_winding());
                            assumed_source_fields = Some(
                                source_fields.map(|field| resolved.assumed_source_field(field)),
                            );
                            candidate.request_finish(disposition).unwrap();
                        })
                        .unwrap();
                    let completion = finish.resume();
                    let baozi_operation::__retained_store_integration::activation::CanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
                        panic!("valid space edit must restore its actual candidate")
                    };
                    let CanonicalFreezePermitIssue::Ready(permit) = candidate
                        .try_issue_freeze_permit(
                            completion.generation.interruption,
                            final_progress(),
                        )
                        .unwrap()
                    else {
                        panic!("valid space edit must reach generation admission")
                    };
                    assert!(permit.destination_counters_are_zero());
                    let expected_root = permit.proof().scene.directory().into_store();
                    let expected_evidence_root = permit.proof().evidence_root;
                    let rejection_slot = if foreign_slot {
                        crate::canonical_scene::store::construction::finish_test_scene_operation(
                            &mut foreign_host,
                            &account,
                        )
                        .generation
                        .rejection
                    } else {
                        completion.generation.rejection
                    };
                    let generation_capture = GenerationCapture {
                        drops: &generation_drops,
                        account: &account,
                        retained_before: retained.live_retained_bytes(),
                    };
                    let result = consume_scene_edit_freeze_permit_with_generation_source(
                        permit,
                        rejection_slot,
                        || {
                            let _capture = generation_capture;
                            generation_calls.set(generation_calls.get() + 1);
                            SceneGenerationId::reserve_from(&generation_source)
                        },
                    );
                    let denied = foreign_slot || foreign_canonical;
                    if denied {
                        assert_eq!(
                            generation_calls.get(),
                            0,
                            "recovery/root mismatch must reject before source; exhausted={exhausted}"
                        );
                        assert!(matches!(
                            result,
                            Err(SceneEditPublicationError::Containment(_))
                        ));
                        let snapshot = account.snapshot();
                        assert!(snapshot.poisoned());
                        assert_eq!(
                            snapshot.first_poison_evidence().unwrap().reason(),
                            AccountPoisonReason::OwnershipGraphUnverified
                        );
                        assert_eq!(generation_source.load(Ordering::Relaxed), generation_before);
                    } else {
                        match (exhausted, result.unwrap()) {
                            (true, SceneEditPublication::MinimalFallback { published }) => {
                                let StoreTerminalCause::Failure(cause) =
                                    published.ledger_evidence.header().terminal_cause()
                                else {
                                    panic!("generation exhaustion must publish a failure")
                                };
                                assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
                                assert_eq!(cause.code(), SCENE_EDIT_GENERATION_REJECTION_CODE);
                                assert_eq!(
                                    published
                                        .ledger_evidence
                                        .dimension_by_semantic(STORE_DIMENSION)
                                        .unwrap()
                                        .committed_canonical_output(),
                                    0
                                );
                                drop(published);
                            }
                            (false, SceneEditPublication::Success(published)) => {
                                let (published, evidence, ledger) = published.into_parts();
                                assert_ne!(published.generation(), input_generation);
                                assert_eq!(
                                    published.as_node_scene().canonical.root_ref(),
                                    expected_root
                                );
                                let SceneEditEvidence::SpaceConversion(evidence) = evidence else {
                                    panic!("space edit must retain its conversion evidence")
                                };
                                assert_eq!(evidence._store.root_ref(), expected_evidence_root);
                                assert_eq!(evidence.input_generation(), input_generation);
                                assert_eq!(evidence.output_generation(), published.generation());
                                assert_eq!(evidence.source(), TargetSpace::STANDARD);
                                assert_eq!(evidence.target(), TargetSpace::STANDARD);
                                assert!(!evidence.reverses_triangle_winding());
                                assert_eq!(evidence.triangle_corner_order(), [0, 1, 2]);
                                for (field, assumed) in source_fields
                                    .into_iter()
                                    .zip(assumed_source_fields.unwrap())
                                {
                                    assert_eq!(evidence.assumed_source_field(field), assumed);
                                }
                                assert_eq!(
                                    ledger.header().terminal_cause(),
                                    StoreTerminalCause::Success
                                );
                                assert!(
                                    ledger
                                        .dimension_by_semantic(STORE_DIMENSION)
                                        .unwrap()
                                        .committed_canonical_output()
                                        > 0
                                );
                                drop((published, evidence, ledger));
                            }
                            _ => {
                                panic!("generation admission must select its fixed terminal owner")
                            }
                        }
                        assert_eq!(generation_calls.get(), 1);
                        assert_eq!(
                            generation_source.load(Ordering::Relaxed),
                            if exhausted {
                                u64::MAX
                            } else {
                                generation_before + 1
                            }
                        );
                        assert!(!account.snapshot().poisoned());
                    }
                    assert_eq!(generation_drops.get(), 1);
                }
                drop(foreign_host);
                drop(invocation_host);
                assert_eq!(scene.generation(), input_generation);
                if !foreign_slot && !foreign_canonical {
                    assert_eq!(
                        account.snapshot().live_retained_bytes(),
                        retained.live_retained_bytes()
                    );
                    assert_eq!(
                        account.snapshot().live_retained_allocations(),
                        retained.live_retained_allocations()
                    );
                }
                drop(scene);
                drop(source.report);
                if !foreign_slot && !foreign_canonical {
                    assert_eq!(account.snapshot().live_retained_bytes(), 0);
                    assert_eq!(account.snapshot().live_retained_allocations(), 0);
                }
            }
        }
    }
}
