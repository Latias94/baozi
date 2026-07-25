use baozi_operation::{
    BytesPlan, OutcomeEvidenceRecordRef, PreparedStoreFailureOutcome, PreparedStoreSuccess,
    PreparedStoreSuccessOutcome, PublishedCanonicalStore, PublishedOutcomeEvidenceStore,
    RecordBlock, RecordPlan, RetainedStoreBlockTablePlan, RetainedStorePublicationPlan,
    StoreFailureValidation, StorePlanError, StoreRuntimeError, StoreSuccessValidation, Utf8Plan,
};

use super::super::schema::{
    COMPLEX_ANIMATION_FIELD_COUNT, COMPLEX_ANIMATION_TIME_COUNT, COMPLEX_ANIMATION_TIMES,
    COMPLEX_CANONICAL_BLOCK_COUNT, COMPLEX_DIRECTORY_FIELD_COUNT, COMPLEX_IMAGE_FIELD_COUNT,
    COMPLEX_IMAGE_MIME, COMPLEX_IMAGE_PAYLOAD, COMPLEX_INFLUENCE_PAYLOAD_FIELD_COUNT,
    COMPLEX_INFLUENCE_ROW_COUNT, COMPLEX_INFLUENCE_WEIGHTS, COMPLEX_INSTANCE_OVERRIDE_FIELD_COUNT,
    COMPLEX_INSTANCE_OVERRIDE_WEIGHT, COMPLEX_INVERSE_BIND_MATRICES,
    COMPLEX_INVERSE_BIND_SCALAR_COUNT, COMPLEX_JOINT_FIELD_COUNT, COMPLEX_LOGICAL_MESH_FIELD_COUNT,
    COMPLEX_LOGICAL_MORPH_FIELD_COUNT, COMPLEX_LOGICAL_MORPH_WEIGHT,
    COMPLEX_MATERIAL_BINDING_FIELD_COUNT, COMPLEX_MATERIAL_FIELD_COUNT,
    COMPLEX_MESH_INSTANCE_FIELD_COUNT, COMPLEX_METADATA, COMPLEX_METADATA_DESCRIPTOR_FIELD_COUNT,
    COMPLEX_MORPH_WEIGHT_VALUE_COUNT, COMPLEX_MORPH_WEIGHT_VALUE_WIDTH,
    COMPLEX_MORPH_WEIGHT_VALUES, COMPLEX_NODE_COUNT, COMPLEX_NODE_FIELD_COUNT,
    COMPLEX_PRIMITIVE_FIELD_COUNT, COMPLEX_PROVENANCE, COMPLEX_PROVENANCE_ARENA_FIELD_COUNT,
    COMPLEX_SAMPLER_FIELD_COUNT, COMPLEX_SAMPLER_MAG_FILTER, COMPLEX_SAMPLER_MIN_FILTER,
    COMPLEX_SCHEMA_REVISION, COMPLEX_SKIN_FIELD_COUNT, COMPLEX_SOURCE_ATTACHMENT_FIELD_COUNT,
    COMPLEX_TEXTURE_FIELD_COUNT, CanonicalBlockRef, ComplexAnimationTableSchema,
    ComplexAnimationTimesSchema, ComplexDirectorySchema, ComplexImageMimeSchema,
    ComplexImagePayloadSchema, ComplexImageTableSchema, ComplexInfluencePayloadSchema,
    ComplexInstanceOverrideTableSchema, ComplexInverseBindMatricesSchema, ComplexJointTableSchema,
    ComplexLogicalMeshTableSchema, ComplexLogicalMorphTableSchema,
    ComplexMaterialBindingTableSchema, ComplexMaterialTableSchema, ComplexMeshInstanceTableSchema,
    ComplexMetadataDescriptorSchema, ComplexMetadataSchema, ComplexMorphWeightValuesSchema,
    ComplexNodeTableSchema, ComplexOrphanSchema, ComplexPrimitiveTableSchema,
    ComplexProvenanceArenaSchema, ComplexProvenanceSchema, ComplexSamplerTableSchema,
    ComplexSkinTableSchema, ComplexSourceAttachmentSchema, ComplexTextureTableSchema,
    EVIDENCE_FIELD_COUNT, EVIDENCE_OUTCOME_CANDIDATE, OutcomeEvidenceSchema, SCHEMA_REVISION,
};
use super::super::validation::{
    validate_complex_fixture_evidence_graph, validate_complex_fixture_graph,
};
use super::{
    ConstructionBrand, FALLBACK_CODE, PreparedTracerSceneFallback, PreparedTracerSceneRichFailure,
    SEMANTIC_REJECTION_CODE, SceneConstructionError, SceneConstructionLineage,
    SceneConstructionOperation, SceneConstructionOwner, SceneConstructionPhase,
    SceneConstructionUnit, SceneStoreAssembly, SceneStoreFailureAssembly, SceneStoreKeys,
    TracerSceneFallback, TracerSceneRichFailure, final_progress,
};
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

type ComplexValidatedStoreSuccess = baozi_operation::ValidatedStoreSuccess<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    ComplexDirectorySchema,
    RecordBlock,
    OutcomeEvidenceSchema,
>;
type ComplexPreparedStoreSuccess =
    PreparedStoreSuccess<RecordBlock, ComplexDirectorySchema, RecordBlock, OutcomeEvidenceSchema>;

mod semantic;

pub(in crate::canonical_scene::store) use semantic::COMPLEX_SEMANTIC_TRANSCRIPT_SCHEMA_ID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComplexFixtureFault {
    CrossTableIndex,
    ReferenceTarget,
    SequenceValue,
    Utf8Value,
    BytesValue,
    OrphanCanonicalBlock,
}

const COMPLEX_FIXED_CANONICAL_BLOCK_COUNT: usize = COMPLEX_CANONICAL_BLOCK_COUNT as usize - 2;
pub(in crate::canonical_scene::store) const COMPLEX_MAX_IMAGE_COUNT: usize = 40;

struct ComplexFixtureDraft {
    brand: ConstructionBrand,
    directory: CanonicalBlockRef<ComplexDirectorySchema>,
}

struct ComplexFixtureValidationProof;

struct PendingComplexDestinationCommitments;

struct ValidatedComplexFixture {
    store: ComplexValidatedStoreSuccess,
    directory: CanonicalBlockRef<ComplexDirectorySchema>,
    canonical_block_count: u32,
    proof: ComplexFixtureValidationProof,
    commitments: PendingComplexDestinationCommitments,
}

pub(crate) struct PreparedComplexFixtureSuccess {
    generation: ReservedSceneGeneration,
    store: ComplexPreparedStoreSuccess,
    directory: CanonicalBlockRef<ComplexDirectorySchema>,
    proof: ComplexFixtureValidationProof,
}

#[must_use = "the test fixture owns its frozen retained publication"]
pub(crate) enum PreparedComplexFixture {
    Success(PreparedComplexFixtureSuccess),
    RichFailure(PreparedTracerSceneRichFailure),
    MinimalFallback(PreparedTracerSceneFallback),
}

pub(crate) struct ComplexFixture {
    generation: SceneGenerationId,
    canonical: PublishedCanonicalStore<RecordBlock, ComplexDirectorySchema>,
    directory: CanonicalBlockRef<ComplexDirectorySchema>,
    _proof: ComplexFixtureValidationProof,
}

impl ComplexFixture {
    pub(crate) const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    pub(crate) fn account(&self) -> &baozi_operation::PhysicalStorageAccount {
        self.canonical.account()
    }

    pub(crate) fn canonical_block_count(&self) -> usize {
        self.canonical.len()
    }

    pub(crate) fn root_is_retained(&self) -> bool {
        self.canonical
            .records(self.directory.into_store())
            .is_some()
    }

    /// Reads the validated, published typed views; constants alone are never a digest input.
    pub(in crate::canonical_scene::store) fn semantic_transcript(&self) -> Option<String> {
        semantic::semantic_transcript(&self.canonical, self.directory)
    }

    pub(in crate::canonical_scene::store) fn inspect_fixed_image_views(
        &self,
        expected_image_count: usize,
    ) -> Option<()> {
        semantic::inspect_fixed_image_views(&self.canonical, self.directory, expected_image_count)
    }

    pub(in crate::canonical_scene::store) fn rejects_foreign_directory(
        &self,
        foreign: &Self,
    ) -> bool {
        self.canonical
            .records(foreign.directory.into_store())
            .is_none()
    }
}

pub(crate) struct ComplexFixtureSuccessReport {
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, OutcomeEvidenceSchema>,
    ledger_evidence: baozi_operation::StoreLedgerEvidence,
}

impl ComplexFixtureSuccessReport {
    pub(crate) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.len()
    }

    pub(crate) const fn ledger_evidence(&self) -> &baozi_operation::StoreLedgerEvidence {
        &self.ledger_evidence
    }
}

pub(crate) struct ComplexFixtureSuccessParts {
    pub(crate) fixture: ComplexFixture,
    pub(crate) report: ComplexFixtureSuccessReport,
}

#[must_use = "the terminal owns the exact complex fixture outcome"]
pub(crate) enum ComplexFixtureTerminal {
    Success(ComplexFixtureSuccessParts),
    RichFailure(TracerSceneRichFailure),
    MinimalFallback(TracerSceneFallback),
}

impl PreparedComplexFixture {
    pub(crate) fn handoff(self) -> ComplexFixtureTerminal {
        match self {
            Self::Success(prepared) => {
                let published = prepared.store.handoff();
                ComplexFixtureTerminal::Success(ComplexFixtureSuccessParts {
                    fixture: ComplexFixture {
                        generation: prepared.generation.into_id(),
                        canonical: published.canonical,
                        directory: prepared.directory,
                        _proof: prepared.proof,
                    },
                    report: ComplexFixtureSuccessReport {
                        outcome_evidence: published.outcome_evidence,
                        ledger_evidence: published.ledger_evidence,
                    },
                })
            }
            Self::RichFailure(prepared) => {
                let published = prepared.store.handoff();
                ComplexFixtureTerminal::RichFailure(TracerSceneRichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                })
            }
            Self::MinimalFallback(prepared) => {
                let published = prepared.store.handoff();
                ComplexFixtureTerminal::MinimalFallback(TracerSceneFallback {
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}

pub(super) fn prepare(
    operation: SceneConstructionOperation<'static>,
    image_count: usize,
    fault: Option<ComplexFixtureFault>,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedComplexFixture, SceneConstructionError> {
    let SceneConstructionOperation {
        raw,
        brand,
        keys,
        lineage,
    } = operation;
    let SceneConstructionLineage::Fresh = lineage else {
        let store = raw.prepare_minimal_fallback(SEMANTIC_REJECTION_CODE)?;
        return Ok(PreparedComplexFixture::MinimalFallback(
            PreparedTracerSceneFallback { store },
        ));
    };
    if !(1..=COMPLEX_MAX_IMAGE_COUNT).contains(&image_count) {
        let store = raw.prepare_minimal_fallback(SEMANTIC_REJECTION_CODE)?;
        return Ok(PreparedComplexFixture::MinimalFallback(
            PreparedTracerSceneFallback { store },
        ));
    }
    let canonical_blocks = complex_canonical_block_count(image_count, fault);
    let canonical_blocks_u32 = u32::try_from(canonical_blocks)
        .map_err(|_| SceneConstructionError::StorePlan(StorePlanError::SizeOverflow))?;
    let publication_plan = complex_publication_plan(keys, canonical_blocks)?;
    let mut assembly = match raw.try_publication_assembly(publication_plan) {
        Ok(assembly) => assembly,
        Err(error) => {
            let operation = error.try_into_operation()?;
            let store = operation.prepare_minimal_fallback(FALLBACK_CODE)?;
            return Ok(PreparedComplexFixture::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ));
        }
    };

    macro_rules! retain_or_fallback {
        ($operation:expr) => {
            match $operation {
                Ok(value) => value,
                Err(_) => return prepare_minimal_fallback_from_assembly(assembly),
            }
        };
    }

    let evidence = retain_or_fallback!(retain_evidence(&mut assembly, canonical_blocks_u32));
    let mut image_mimes = [None; COMPLEX_MAX_IMAGE_COUNT];
    let mut image_payloads = [None; COMPLEX_MAX_IMAGE_COUNT];
    for index in 0..image_count {
        let leaf_fault = (index == image_count - 1).then_some(fault).flatten();
        image_mimes[index] = Some(retain_or_fallback!(retain_image_mime(
            &mut assembly,
            leaf_fault
        )));
        image_payloads[index] = Some(retain_or_fallback!(retain_image_payload(
            &mut assembly,
            leaf_fault
        )));
    }
    let images = retain_or_fallback!(retain_images(
        &mut assembly,
        &image_mimes,
        &image_payloads,
        image_count,
    ));
    let samplers = retain_or_fallback!(retain_samplers(&mut assembly));
    let textures = retain_or_fallback!(retain_textures(
        &mut assembly,
        images,
        samplers,
        image_count,
        fault,
    ));
    let material_bindings = retain_or_fallback!(retain_material_bindings(&mut assembly, textures));
    let materials = retain_or_fallback!(retain_materials(&mut assembly, material_bindings));
    let joints = retain_or_fallback!(retain_joints(&mut assembly));
    let inverse_binds = retain_or_fallback!(retain_inverse_binds(&mut assembly, fault));
    let influences = retain_or_fallback!(retain_influences(&mut assembly, joints));
    let skins = retain_or_fallback!(retain_skins(
        &mut assembly,
        joints,
        inverse_binds,
        influences,
    ));
    let primitives = retain_or_fallback!(retain_primitives(
        &mut assembly,
        materials,
        skins,
        influences
    ));
    let logical_meshes = retain_or_fallback!(retain_logical_meshes(&mut assembly, primitives));
    let logical_morphs = retain_or_fallback!(retain_logical_morphs(&mut assembly));
    let instance_overrides =
        retain_or_fallback!(retain_instance_overrides(&mut assembly, logical_morphs));
    let animation_times = retain_or_fallback!(retain_animation_times(&mut assembly));
    let morph_weight_values = retain_or_fallback!(retain_morph_weight_values(&mut assembly));
    let animations = retain_or_fallback!(retain_animations(
        &mut assembly,
        instance_overrides,
        animation_times,
        morph_weight_values,
    ));
    let mesh_instances = retain_or_fallback!(retain_mesh_instances(
        &mut assembly,
        logical_meshes,
        skins,
        instance_overrides,
    ));
    let nodes = retain_or_fallback!(retain_nodes(&mut assembly, mesh_instances));
    let metadata = retain_or_fallback!(retain_metadata(&mut assembly));
    let metadata_descriptors =
        retain_or_fallback!(retain_metadata_descriptors(&mut assembly, metadata));
    let provenance = retain_or_fallback!(retain_provenance(&mut assembly));
    let provenance_arenas =
        retain_or_fallback!(retain_provenance_arenas(&mut assembly, provenance));
    let source_attachments = retain_or_fallback!(retain_source_attachments(
        &mut assembly,
        metadata_descriptors,
        provenance_arenas,
    ));
    if matches!(fault, Some(ComplexFixtureFault::OrphanCanonicalBlock))
        && assembly
            .try_retain_canonical_sequence(
                baozi_operation::SequencePlan::<u8>::new(1).map_err(StoreRuntimeError::Plan)?,
                ComplexOrphanSchema,
                |writer| writer.push(1),
            )
            .is_err()
    {
        return prepare_minimal_fallback_from_assembly(assembly);
    }
    let directory = retain_or_fallback!(retain_directory(
        &mut assembly,
        image_count,
        images,
        samplers,
        textures,
        material_bindings,
        materials,
        primitives,
        logical_meshes,
        nodes,
        mesh_instances,
        skins,
        joints,
        logical_morphs,
        instance_overrides,
        animations,
        source_attachments,
    ));
    let commitments = match register_pending_destination_commitments(
        &mut assembly,
        keys,
        canonical_blocks as u64,
    ) {
        Ok(commitments) => commitments,
        Err(_) => return prepare_minimal_fallback_from_assembly(assembly),
    };
    let draft = ComplexFixtureDraft { brand, directory };
    let brand_matches = draft.brand == brand;
    match assembly.validate_success(
        draft.directory.into_store(),
        evidence,
        |session, directory, evidence| {
            brand_matches
                && validate_complex_fixture_graph(
                    session,
                    directory,
                    evidence,
                    image_count,
                    canonical_blocks_u32,
                )
        },
    ) {
        StoreSuccessValidation::Valid(store) => prepare_validated_success(
            ValidatedComplexFixture {
                store,
                directory,
                canonical_block_count: canonical_blocks_u32,
                proof: ComplexFixtureValidationProof,
                commitments,
            },
            reserve_generation,
        ),
        StoreSuccessValidation::Rejected(failure) => {
            prepare_rich_failure(failure, SEMANTIC_REJECTION_CODE, canonical_blocks_u32)
        }
        StoreSuccessValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedComplexFixture::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn complex_canonical_block_count(image_count: usize, fault: Option<ComplexFixtureFault>) -> usize {
    COMPLEX_FIXED_CANONICAL_BLOCK_COUNT
        + image_count * 2
        + usize::from(matches!(
            fault,
            Some(ComplexFixtureFault::OrphanCanonicalBlock)
        ))
}

pub(super) fn scaled_complex_attribution_slots(image_count: usize) -> Option<usize> {
    (1..=COMPLEX_MAX_IMAGE_COUNT)
        .contains(&image_count)
        .then(|| {
            complex_canonical_block_count(
                image_count,
                Some(ComplexFixtureFault::OrphanCanonicalBlock),
            ) + 1
        })
}

fn complex_publication_plan(
    keys: SceneStoreKeys,
    canonical_blocks: usize,
) -> Result<RetainedStorePublicationPlan, StorePlanError> {
    let canonical_writer = super::scene_writer_plan(keys, canonical_blocks + 1, 2)?;
    let evidence_writer = super::scene_writer_plan(keys, 2, 2)?;
    Ok(RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(canonical_writer, canonical_blocks, canonical_blocks)?,
        RetainedStoreBlockTablePlan::new(evidence_writer, 1, 1)?,
    ))
}

fn retain_evidence(
    assembly: &mut SceneStoreAssembly,
    canonical_block_count: u32,
) -> Result<OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>, StoreRuntimeError> {
    assembly.try_retain_outcome_evidence_records(
        RecordPlan::new(1, EVIDENCE_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
        OutcomeEvidenceSchema,
        |writer| {
            writer.write_row(|row| {
                row.write_scalar(SCHEMA_REVISION)?;
                row.write_scalar(EVIDENCE_OUTCOME_CANDIDATE)?;
                row.write_scalar(canonical_block_count)?;
                row.write_scalar(COMPLEX_NODE_COUNT)
            })
        },
    )
}

fn retain_image_mime(
    assembly: &mut SceneStoreAssembly,
    fault: Option<ComplexFixtureFault>,
) -> Result<CanonicalBlockRef<ComplexImageMimeSchema>, StoreRuntimeError> {
    const MUTATED_IMAGE_MIME: &str = "image/gif";
    let mime = if matches!(fault, Some(ComplexFixtureFault::Utf8Value)) {
        MUTATED_IMAGE_MIME
    } else {
        COMPLEX_IMAGE_MIME
    };
    assembly
        .try_retain_canonical_utf8(
            Utf8Plan::new(mime.len()).map_err(StoreRuntimeError::Plan)?,
            ComplexImageMimeSchema,
            |writer| writer.write_str(mime),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_image_payload(
    assembly: &mut SceneStoreAssembly,
    fault: Option<ComplexFixtureFault>,
) -> Result<CanonicalBlockRef<ComplexImagePayloadSchema>, StoreRuntimeError> {
    const MUTATED_IMAGE_PAYLOAD: &[u8] = b"BAD\x0d\x0a";
    let payload = if matches!(fault, Some(ComplexFixtureFault::BytesValue)) {
        MUTATED_IMAGE_PAYLOAD
    } else {
        COMPLEX_IMAGE_PAYLOAD
    };
    assembly
        .try_retain_canonical_bytes(
            BytesPlan::new(payload.len()).map_err(StoreRuntimeError::Plan)?,
            ComplexImagePayloadSchema,
            |writer| writer.write_bytes(payload),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_images(
    assembly: &mut SceneStoreAssembly,
    mimes: &[Option<CanonicalBlockRef<ComplexImageMimeSchema>>; COMPLEX_MAX_IMAGE_COUNT],
    payloads: &[Option<CanonicalBlockRef<ComplexImagePayloadSchema>>; COMPLEX_MAX_IMAGE_COUNT],
    image_count: usize,
) -> Result<CanonicalBlockRef<ComplexImageTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(image_count, COMPLEX_IMAGE_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexImageTableSchema,
            |writer| {
                for index in 0..image_count {
                    let image =
                        u32::try_from(index).expect("the admitted complex image count fits u32");
                    let mime = mimes[index].expect("every admitted image has a MIME block");
                    let payload =
                        payloads[index].expect("every admitted image has a payload block");
                    writer.write_row(|row| {
                        row.write_scalar(image)?;
                        row.write_utf8_ref(mime.into_store())?;
                        row.write_bytes_ref(payload.into_store())
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_samplers(
    assembly: &mut SceneStoreAssembly,
) -> Result<CanonicalBlockRef<ComplexSamplerTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_SAMPLER_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexSamplerTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(COMPLEX_SAMPLER_MAG_FILTER)?;
                    row.write_scalar(COMPLEX_SAMPLER_MIN_FILTER)
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_textures(
    assembly: &mut SceneStoreAssembly,
    images: CanonicalBlockRef<ComplexImageTableSchema>,
    samplers: CanonicalBlockRef<ComplexSamplerTableSchema>,
    image_count: usize,
    fault: Option<ComplexFixtureFault>,
) -> Result<CanonicalBlockRef<ComplexTextureTableSchema>, StoreRuntimeError> {
    let image_index = if matches!(fault, Some(ComplexFixtureFault::CrossTableIndex)) {
        u32::try_from(image_count)
            .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?
    } else {
        0
    };
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_TEXTURE_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexTextureTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(image_index)?;
                    if matches!(fault, Some(ComplexFixtureFault::ReferenceTarget)) {
                        row.write_record_ref(samplers.into_store())?;
                    } else {
                        row.write_record_ref(images.into_store())?;
                    }
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(samplers.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_material_bindings(
    assembly: &mut SceneStoreAssembly,
    textures: CanonicalBlockRef<ComplexTextureTableSchema>,
) -> Result<CanonicalBlockRef<ComplexMaterialBindingTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_MATERIAL_BINDING_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexMaterialBindingTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(textures.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_materials(
    assembly: &mut SceneStoreAssembly,
    material_bindings: CanonicalBlockRef<ComplexMaterialBindingTableSchema>,
) -> Result<CanonicalBlockRef<ComplexMaterialTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_MATERIAL_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexMaterialTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(material_bindings.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_joints(
    assembly: &mut SceneStoreAssembly,
) -> Result<CanonicalBlockRef<ComplexJointTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_JOINT_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexJointTableSchema,
            |writer| writer.write_row(|row| row.write_scalar(0_u32)),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_inverse_binds(
    assembly: &mut SceneStoreAssembly,
    fault: Option<ComplexFixtureFault>,
) -> Result<CanonicalBlockRef<ComplexInverseBindMatricesSchema>, StoreRuntimeError> {
    let mut values = COMPLEX_INVERSE_BIND_MATRICES;
    if matches!(fault, Some(ComplexFixtureFault::SequenceValue)) {
        values[COMPLEX_INVERSE_BIND_SCALAR_COUNT - 1] = 2.0;
    }
    assembly
        .try_retain_canonical_sequence(
            baozi_operation::SequencePlan::<f32>::new(COMPLEX_INVERSE_BIND_SCALAR_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexInverseBindMatricesSchema,
            |writer| writer.extend_from_slice(&values),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_influences(
    assembly: &mut SceneStoreAssembly,
    joints: CanonicalBlockRef<ComplexJointTableSchema>,
) -> Result<CanonicalBlockRef<ComplexInfluencePayloadSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(
                COMPLEX_INFLUENCE_ROW_COUNT,
                COMPLEX_INFLUENCE_PAYLOAD_FIELD_COUNT,
            )
            .map_err(StoreRuntimeError::Plan)?,
            ComplexInfluencePayloadSchema,
            |writer| {
                for (vertex, weight) in COMPLEX_INFLUENCE_WEIGHTS.iter().copied().enumerate() {
                    writer.write_row(|row| {
                        row.write_scalar(vertex as u32)?;
                        row.write_scalar(0_u32)?;
                        row.write_record_ref(joints.into_store())?;
                        row.write_scalar(weight)
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_skins(
    assembly: &mut SceneStoreAssembly,
    joints: CanonicalBlockRef<ComplexJointTableSchema>,
    inverse_binds: CanonicalBlockRef<ComplexInverseBindMatricesSchema>,
    influences: CanonicalBlockRef<ComplexInfluencePayloadSchema>,
) -> Result<CanonicalBlockRef<ComplexSkinTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_SKIN_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexSkinTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(joints.into_store())?;
                    row.write_scalar(COMPLEX_INVERSE_BIND_SCALAR_COUNT as u32)?;
                    row.write_sequence_ref(inverse_binds.into_store())?;
                    row.write_scalar(COMPLEX_INFLUENCE_ROW_COUNT as u32)?;
                    row.write_record_ref(influences.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_primitives(
    assembly: &mut SceneStoreAssembly,
    materials: CanonicalBlockRef<ComplexMaterialTableSchema>,
    skins: CanonicalBlockRef<ComplexSkinTableSchema>,
    influences: CanonicalBlockRef<ComplexInfluencePayloadSchema>,
) -> Result<CanonicalBlockRef<ComplexPrimitiveTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_PRIMITIVE_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexPrimitiveTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(materials.into_store())?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(skins.into_store())?;
                    row.write_scalar(COMPLEX_INFLUENCE_ROW_COUNT as u32)?;
                    row.write_record_ref(influences.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_logical_meshes(
    assembly: &mut SceneStoreAssembly,
    primitives: CanonicalBlockRef<ComplexPrimitiveTableSchema>,
) -> Result<CanonicalBlockRef<ComplexLogicalMeshTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_LOGICAL_MESH_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexLogicalMeshTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(primitives.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_logical_morphs(
    assembly: &mut SceneStoreAssembly,
) -> Result<CanonicalBlockRef<ComplexLogicalMorphTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_LOGICAL_MORPH_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexLogicalMorphTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(1_u32)?;
                    row.write_scalar(COMPLEX_LOGICAL_MORPH_WEIGHT)
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_instance_overrides(
    assembly: &mut SceneStoreAssembly,
    logical_morphs: CanonicalBlockRef<ComplexLogicalMorphTableSchema>,
) -> Result<CanonicalBlockRef<ComplexInstanceOverrideTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_INSTANCE_OVERRIDE_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexInstanceOverrideTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(logical_morphs.into_store())?;
                    row.write_scalar(COMPLEX_INSTANCE_OVERRIDE_WEIGHT)
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_animation_times(
    assembly: &mut SceneStoreAssembly,
) -> Result<CanonicalBlockRef<ComplexAnimationTimesSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence(
            baozi_operation::SequencePlan::<f64>::new(COMPLEX_ANIMATION_TIME_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexAnimationTimesSchema,
            |writer| writer.extend_from_slice(&COMPLEX_ANIMATION_TIMES),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_morph_weight_values(
    assembly: &mut SceneStoreAssembly,
) -> Result<CanonicalBlockRef<ComplexMorphWeightValuesSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence(
            baozi_operation::SequencePlan::<f64>::new(COMPLEX_MORPH_WEIGHT_VALUE_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexMorphWeightValuesSchema,
            |writer| writer.extend_from_slice(&COMPLEX_MORPH_WEIGHT_VALUES),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_animations(
    assembly: &mut SceneStoreAssembly,
    instance_overrides: CanonicalBlockRef<ComplexInstanceOverrideTableSchema>,
    times: CanonicalBlockRef<ComplexAnimationTimesSchema>,
    morph_weight_values: CanonicalBlockRef<ComplexMorphWeightValuesSchema>,
) -> Result<CanonicalBlockRef<ComplexAnimationTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_ANIMATION_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexAnimationTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(instance_overrides.into_store())?;
                    row.write_scalar(COMPLEX_ANIMATION_TIME_COUNT as u32)?;
                    row.write_sequence_ref(times.into_store())?;
                    row.write_scalar(COMPLEX_MORPH_WEIGHT_VALUE_WIDTH as u32)?;
                    row.write_sequence_ref(morph_weight_values.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_mesh_instances(
    assembly: &mut SceneStoreAssembly,
    logical_meshes: CanonicalBlockRef<ComplexLogicalMeshTableSchema>,
    skins: CanonicalBlockRef<ComplexSkinTableSchema>,
    instance_overrides: CanonicalBlockRef<ComplexInstanceOverrideTableSchema>,
) -> Result<CanonicalBlockRef<ComplexMeshInstanceTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_MESH_INSTANCE_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexMeshInstanceTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(logical_meshes.into_store())?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(skins.into_store())?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(instance_overrides.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_nodes(
    assembly: &mut SceneStoreAssembly,
    mesh_instances: CanonicalBlockRef<ComplexMeshInstanceTableSchema>,
) -> Result<CanonicalBlockRef<ComplexNodeTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_NODE_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexNodeTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(mesh_instances.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_metadata(
    assembly: &mut SceneStoreAssembly,
) -> Result<CanonicalBlockRef<ComplexMetadataSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_utf8(
            Utf8Plan::new(COMPLEX_METADATA.len()).map_err(StoreRuntimeError::Plan)?,
            ComplexMetadataSchema,
            |writer| writer.write_str(COMPLEX_METADATA),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_metadata_descriptors(
    assembly: &mut SceneStoreAssembly,
    metadata: CanonicalBlockRef<ComplexMetadataSchema>,
) -> Result<CanonicalBlockRef<ComplexMetadataDescriptorSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_METADATA_DESCRIPTOR_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexMetadataDescriptorSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_utf8_ref(metadata.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_provenance(
    assembly: &mut SceneStoreAssembly,
) -> Result<CanonicalBlockRef<ComplexProvenanceSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_bytes(
            BytesPlan::new(COMPLEX_PROVENANCE.len()).map_err(StoreRuntimeError::Plan)?,
            ComplexProvenanceSchema,
            |writer| writer.write_bytes(COMPLEX_PROVENANCE),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_provenance_arenas(
    assembly: &mut SceneStoreAssembly,
    provenance: CanonicalBlockRef<ComplexProvenanceSchema>,
) -> Result<CanonicalBlockRef<ComplexProvenanceArenaSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_PROVENANCE_ARENA_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexProvenanceArenaSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_bytes_ref(provenance.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_source_attachments(
    assembly: &mut SceneStoreAssembly,
    metadata_descriptors: CanonicalBlockRef<ComplexMetadataDescriptorSchema>,
    provenance_arenas: CanonicalBlockRef<ComplexProvenanceArenaSchema>,
) -> Result<CanonicalBlockRef<ComplexSourceAttachmentSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_SOURCE_ATTACHMENT_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ComplexSourceAttachmentSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(metadata_descriptors.into_store())?;
                    row.write_scalar(0_u32)?;
                    row.write_record_ref(provenance_arenas.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the fixture root names every retained semantic table"
)]
fn retain_directory(
    assembly: &mut SceneStoreAssembly,
    image_count: usize,
    images: CanonicalBlockRef<ComplexImageTableSchema>,
    samplers: CanonicalBlockRef<ComplexSamplerTableSchema>,
    textures: CanonicalBlockRef<ComplexTextureTableSchema>,
    material_bindings: CanonicalBlockRef<ComplexMaterialBindingTableSchema>,
    materials: CanonicalBlockRef<ComplexMaterialTableSchema>,
    primitives: CanonicalBlockRef<ComplexPrimitiveTableSchema>,
    logical_meshes: CanonicalBlockRef<ComplexLogicalMeshTableSchema>,
    nodes: CanonicalBlockRef<ComplexNodeTableSchema>,
    mesh_instances: CanonicalBlockRef<ComplexMeshInstanceTableSchema>,
    skins: CanonicalBlockRef<ComplexSkinTableSchema>,
    joints: CanonicalBlockRef<ComplexJointTableSchema>,
    logical_morphs: CanonicalBlockRef<ComplexLogicalMorphTableSchema>,
    instance_overrides: CanonicalBlockRef<ComplexInstanceOverrideTableSchema>,
    animations: CanonicalBlockRef<ComplexAnimationTableSchema>,
    source_attachments: CanonicalBlockRef<ComplexSourceAttachmentSchema>,
) -> Result<CanonicalBlockRef<ComplexDirectorySchema>, StoreRuntimeError> {
    let image_count = u32::try_from(image_count)
        .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, COMPLEX_DIRECTORY_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            ComplexDirectorySchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(COMPLEX_SCHEMA_REVISION)?;
                    row.write_scalar(image_count)?;
                    row.write_record_ref(images.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(samplers.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(textures.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(material_bindings.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(materials.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(primitives.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(logical_meshes.into_store())?;
                    row.write_scalar(COMPLEX_NODE_COUNT)?;
                    row.write_record_ref(nodes.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(mesh_instances.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(skins.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(joints.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(logical_morphs.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(instance_overrides.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(animations.into_store())?;
                    row.write_scalar(1_u32)?;
                    row.write_record_ref(source_attachments.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn register_pending_destination_commitments(
    assembly: &mut SceneStoreAssembly,
    keys: SceneStoreKeys,
    canonical_blocks: u64,
) -> Result<PendingComplexDestinationCommitments, StoreRuntimeError> {
    assembly.try_commit_canonical(keys.dimension, canonical_blocks)?;
    assembly.try_commit_outcome_evidence(keys.dimension, 1)?;
    Ok(PendingComplexDestinationCommitments)
}

fn prepare_validated_success(
    candidate: ValidatedComplexFixture,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedComplexFixture, SceneConstructionError> {
    let ValidatedComplexFixture {
        store: validated,
        directory,
        canonical_block_count,
        proof,
        commitments,
    } = candidate;
    let reservation = match reserve_generation() {
        Ok(reservation) => reservation,
        Err(SceneGenerationError::Exhausted) => {
            return prepare_rich_failure(
                validated.downgrade_to_rich_failure(super::GENERATION_EXHAUSTED_CODE),
                super::GENERATION_EXHAUSTED_CODE,
                canonical_block_count,
            );
        }
    };
    let PendingComplexDestinationCommitments = commitments;
    match validated.try_prepare(final_progress())? {
        PreparedStoreSuccessOutcome::Success(store) => Ok(PreparedComplexFixture::Success(
            PreparedComplexFixtureSuccess {
                generation: reservation,
                store,
                directory,
                proof,
            },
        )),
        PreparedStoreSuccessOutcome::RichFailure(store) => {
            super::burn_generation_reservation(reservation);
            Ok(PreparedComplexFixture::RichFailure(
                PreparedTracerSceneRichFailure { store },
            ))
        }
        PreparedStoreSuccessOutcome::MinimalFallback(store) => {
            super::burn_generation_reservation(reservation);
            Ok(PreparedComplexFixture::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn prepare_rich_failure(
    failure: SceneStoreFailureAssembly,
    cause: u32,
    canonical_block_count: u32,
) -> Result<PreparedComplexFixture, SceneConstructionError> {
    let evidence = failure.evidence_root();
    match failure.validate_rich_failure(evidence, |session, evidence| {
        validate_complex_fixture_evidence_graph(session, evidence, canonical_block_count)
    }) {
        StoreFailureValidation::Valid(validated) => {
            map_prepared_failure(validated.try_prepare(cause, final_progress())?)
        }
        StoreFailureValidation::Rejected(rejected) => {
            let store = rejected.prepare_minimal_fallback()?;
            Ok(PreparedComplexFixture::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
        StoreFailureValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedComplexFixture::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn map_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, OutcomeEvidenceSchema>,
) -> Result<PreparedComplexFixture, SceneConstructionError> {
    Ok(match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedComplexFixture::RichFailure(PreparedTracerSceneRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedComplexFixture::MinimalFallback(PreparedTracerSceneFallback { store })
        }
    })
}

fn prepare_minimal_fallback_from_assembly(
    assembly: SceneStoreAssembly,
) -> Result<PreparedComplexFixture, SceneConstructionError> {
    let store = assembly.prepare_minimal_fallback()?;
    Ok(PreparedComplexFixture::MinimalFallback(
        PreparedTracerSceneFallback { store },
    ))
}
