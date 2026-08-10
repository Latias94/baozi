use std::num::NonZeroU64;

use baozi_operation::__canonical_activation::{
    CanonicalActivationOutcome, CanonicalActivationPolicy, CanonicalActivationRequirements,
    CanonicalBuildControl, CanonicalCandidateStoreSession, CanonicalCentralValidationOutcome,
    CanonicalTableShape, CanonicalValidationDecision, FailureEvidenceMember,
    FailureEvidenceOutcome, FailureEvidenceValidationSession, RootedFailureValidation,
};
use baozi_operation::{
    CanonicalRecordWriter, OperationControl, PhysicalStorageAccount, RecordBlock, RecordPlan,
    SequencePlan, SequenceWriter, StoreControlledError,
};

#[cfg(test)]
use baozi_operation::{RetainedStoreFailureTerminalWorkPlan, StorePlanError};

use super::super::validation::{
    authored_matrix_scalar_is_valid, scene_graph_validation_work, validate_scene_graph_metered,
};
use super::{
    AuthoredMatrixSchema, CANONICAL_BLOCK_COUNT, CanonicalBlockRef, ConstructionBrand,
    DIRECTORY_FIELD_COUNT, DirectorySchema, EVIDENCE_FIELD_COUNT, EVIDENCE_OUTCOME_CANDIDATE,
    FALLBACK_CODE, MATRIX_SCALAR_COUNT, MATRIX_SCALARS_PER_NODE, NODE_COUNT, NODE_FIELD_COUNT,
    NodeTableSchema, OutcomeEvidenceSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
    PreparedSceneConstruction, PreparedTracerSceneFallback, REFERENCE_COUNT, REFERENCE_FIELD_COUNT,
    ReferenceTableSchema, SCHEMA_REVISION, SEMANTIC_REJECTION_CODE, SceneBlockRefs,
    SceneConstructionError, SceneConstructionLimits, SceneConstructionOperation,
    SceneConstructionOwner, SceneConstructionPhase, SceneConstructionStartError,
    SceneConstructionUnit, SceneDraft, SceneFailureEvidenceWork, SceneStoreKeys,
    SceneStoreOperation, TERMINAL_ATTRIBUTION_SLOTS, TracerSceneValidationProof,
    TwoNodeAuthoredMatrices, final_progress, map_prepared_failure, scene_writer_plan,
    try_plan_scene_operation, write_outcome_evidence_row,
};

type ActivatedSceneCandidate = CanonicalCandidateStoreSession<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    SceneActivationOutput,
    TracerSceneValidationProof,
    RecordBlock,
    DirectorySchema,
    RecordBlock,
    OutcomeEvidenceSchema,
>;

/// Core-owned candidate produced by the operation activation seam.
///
/// It deliberately has no generation or publication method yet. The next U4 slice must consume it
/// through the freeze-permit/core-token joint commit instead of recovering the writable assembly.
#[must_use = "the activated Scene candidate must enter joint commit or be deliberately dropped"]
pub(in crate::canonical_scene::store) struct PendingSceneActivation {
    candidate: ActivatedSceneCandidate,
    canonical_blocks: usize,
}

impl PendingSceneActivation {
    pub(in crate::canonical_scene::store) fn canonical_blocks(&self) -> usize {
        self.canonical_blocks
    }

    pub(in crate::canonical_scene::store) fn directory(
        &self,
    ) -> CanonicalBlockRef<DirectorySchema> {
        self.candidate.output().draft.directory
    }

    pub(in crate::canonical_scene::store) fn refs(&self) -> SceneBlockRefs {
        self.candidate.output().refs
    }
}

#[must_use = "the activated candidate or prepared terminal outcome must be consumed"]
#[allow(
    clippy::large_enum_variant,
    reason = "activation owners stay inline so the post-admission transition cannot allocate"
)]
pub(in crate::canonical_scene::store) enum SceneActivationTransition {
    Candidate(PendingSceneActivation),
    Terminal(PreparedSceneConstruction),
}

struct SceneActivationRequest {
    requirements: CanonicalActivationRequirements,
    materialization: SceneActivationMaterialization,
    evidence_plan: RecordPlan,
}

struct SceneActivationMaterialization {
    matrices_plan: SequencePlan<f32>,
    references_plan: RecordPlan,
    nodes_plan: RecordPlan,
    directory_plan: RecordPlan,
    authored: [f32; MATRIX_SCALAR_COUNT],
    brand: ConstructionBrand,
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct SceneActivationMaterializationWork {
    matrices: u64,
    references: u64,
    nodes: u64,
    directory: u64,
}

struct SceneActivationOutput {
    draft: SceneDraft,
    refs: SceneBlockRefs,
}

type SceneMatrixBuildControl<'writer, 'permit, 'operation> = CanonicalBuildControl<
    'writer,
    'permit,
    'operation,
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    SequenceWriter<f32>,
>;

type SceneRecordBuildControl<'writer, 'permit, 'operation> = CanonicalBuildControl<
    'writer,
    'permit,
    'operation,
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    CanonicalRecordWriter,
>;

impl SceneActivationRequest {
    fn try_new(
        brand: ConstructionBrand,
        authored: TwoNodeAuthoredMatrices,
    ) -> Result<Self, SceneConstructionError> {
        let requirements = scene_activation_requirements()?;
        let matrices_plan = SequencePlan::<f32>::new(MATRIX_SCALAR_COUNT)?;
        let references_plan = RecordPlan::new(REFERENCE_COUNT, REFERENCE_FIELD_COUNT)?;
        let nodes_plan = RecordPlan::new(NODE_COUNT, NODE_FIELD_COUNT)?;
        let directory_plan = RecordPlan::new(1, DIRECTORY_FIELD_COUNT)?;
        let evidence_plan = RecordPlan::new(1, EVIDENCE_FIELD_COUNT)?;
        Ok(Self {
            requirements,
            materialization: SceneActivationMaterialization {
                matrices_plan,
                references_plan,
                nodes_plan,
                directory_plan,
                authored: authored.scalars(),
                brand,
            },
            evidence_plan,
        })
    }
}

fn scene_activation_requirements() -> Result<CanonicalActivationRequirements, SceneConstructionError>
{
    let canonical_blocks = usize::try_from(CANONICAL_BLOCK_COUNT)
        .expect("the canonical tracer block count fits usize");
    Ok(CanonicalActivationRequirements::new(
        CanonicalTableShape::new(canonical_blocks, canonical_blocks)?,
        CanonicalTableShape::new(1, 1)?,
        canonical_blocks,
        scene_activation_preflight_work(),
        NonZeroU64::new(scene_graph_validation_work())
            .expect("the Scene validation work is non-zero"),
    )?)
}

fn scene_activation_preflight_work() -> u64 {
    u64::try_from(MATRIX_SCALAR_COUNT).expect("the fixed authored matrix width fits u64")
}

fn authored_activation_matrix_scalar_is_valid(
    values: &[f32; MATRIX_SCALAR_COUNT],
    index: u64,
) -> bool {
    let Ok(index) = usize::try_from(index) else {
        return false;
    };
    let matrix_start = index / MATRIX_SCALARS_PER_NODE * MATRIX_SCALARS_PER_NODE;
    let local_index = index % MATRIX_SCALARS_PER_NODE;
    values
        .get(matrix_start..matrix_start + MATRIX_SCALARS_PER_NODE)
        .is_some_and(|matrix| authored_matrix_scalar_is_valid(matrix, local_index))
}

fn scene_activation_policy(
    keys: SceneStoreKeys,
) -> Result<CanonicalActivationPolicy, SceneConstructionError> {
    let canonical_blocks = usize::try_from(CANONICAL_BLOCK_COUNT)
        .expect("the canonical tracer block count fits usize");
    let canonical_writer = scene_writer_plan(
        keys,
        canonical_blocks
            .checked_add(1)
            .expect("the fixed canonical attribution capacity fits usize"),
        4,
    )?;
    let evidence_writer = scene_writer_plan(keys, 2, 4)?;
    Ok(CanonicalActivationPolicy::new(
        canonical_writer,
        evidence_writer,
        keys.dimension,
        keys.dimension,
        keys.dimension,
        keys.dimension,
    ))
}

#[cfg(test)]
impl SceneActivationMaterializationWork {
    fn try_new(
        matrices: SequencePlan<f32>,
        references: RecordPlan,
        nodes: RecordPlan,
        directory: RecordPlan,
    ) -> Result<Self, StorePlanError> {
        Ok(Self {
            matrices: u64::try_from(matrices.element_count())
                .map_err(|_| StorePlanError::SizeOverflow)?,
            references: record_plan_work(references)?,
            nodes: record_plan_work(nodes)?,
            directory: record_plan_work(directory)?,
        })
    }

    fn total(self) -> Result<u64, StorePlanError> {
        self.matrices
            .checked_add(self.references)
            .and_then(|work| work.checked_add(self.nodes))
            .and_then(|work| work.checked_add(self.directory))
            .ok_or(StorePlanError::SizeOverflow)
    }
}

#[cfg(test)]
fn record_plan_work(plan: RecordPlan) -> Result<u64, StorePlanError> {
    let cells = plan
        .rows()
        .checked_mul(plan.fields())
        .ok_or(StorePlanError::SizeOverflow)?;
    u64::try_from(cells).map_err(|_| StorePlanError::SizeOverflow)
}

fn write_activation_matrices(
    control: &mut SceneMatrixBuildControl<'_, '_, '_>,
    values: &[f32; MATRIX_SCALAR_COUNT],
) -> Result<(), StoreControlledError> {
    control.poll()?;
    for (index, value) in values.iter().copied().enumerate() {
        control.push(value)?;
        if (index + 1) % MATRIX_SCALARS_PER_NODE == 0 {
            control.poll()?;
        }
    }
    Ok(())
}

fn write_activation_references(
    control: &mut SceneRecordBuildControl<'_, '_, '_>,
) -> Result<(), StoreControlledError> {
    control.poll()?;
    control.write_row(|row| {
        row.write_scalar(0_u32)?;
        row.write_scalar(0_u32)?;
        row.write_scalar(1_u32)
    })?;
    control.poll()?;
    Ok(())
}

fn write_activation_nodes(
    control: &mut SceneRecordBuildControl<'_, '_, '_>,
    matrices: CanonicalBlockRef<AuthoredMatrixSchema>,
) -> Result<(), StoreControlledError> {
    control.poll()?;
    control.write_row(|row| {
        row.write_scalar(0_u32)?;
        row.write_scalar(PRESENCE_ABSENT)?;
        row.write_scalar(0_u32)?;
        row.write_scalar(0_u32)?;
        row.write_scalar(0_u32)?;
        row.write_scalar(MATRIX_SCALARS_PER_NODE as u32)?;
        row.write_sequence_ref(matrices.into_store())
    })?;
    control.poll()?;
    control.write_row(|row| {
        row.write_scalar(1_u32)?;
        row.write_scalar(PRESENCE_PRESENT)?;
        row.write_scalar(0_u32)?;
        row.write_scalar(1_u32)?;
        row.write_scalar(MATRIX_SCALARS_PER_NODE as u32)?;
        row.write_scalar(MATRIX_SCALARS_PER_NODE as u32)?;
        row.write_sequence_ref(matrices.into_store())
    })?;
    control.poll()?;
    Ok(())
}

fn write_activation_directory(
    control: &mut SceneRecordBuildControl<'_, '_, '_>,
    nodes: CanonicalBlockRef<NodeTableSchema>,
    references: CanonicalBlockRef<ReferenceTableSchema>,
    matrices: CanonicalBlockRef<AuthoredMatrixSchema>,
) -> Result<(), StoreControlledError> {
    control.poll()?;
    control.write_row(|row| {
        row.write_scalar(SCHEMA_REVISION)?;
        row.write_scalar(0_u32)?;
        row.write_scalar(NODE_COUNT as u32)?;
        row.write_record_ref(nodes.into_store())?;
        row.write_scalar(REFERENCE_COUNT as u32)?;
        row.write_record_ref(references.into_store())?;
        row.write_scalar(PRESENCE_PRESENT)?;
        row.write_scalar(1_u32)?;
        row.write_scalar(PRESENCE_PRESENT)?;
        row.write_scalar(0_u32)?;
        row.write_scalar(PRESENCE_ABSENT)?;
        row.write_scalar(0_u32)?;
        row.write_scalar(MATRIX_SCALAR_COUNT as u32)?;
        row.write_sequence_ref(matrices.into_store())
    })?;
    control.poll()?;
    Ok(())
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn exact_scene_activation_protected_work_limit() -> u64 {
    let evidence = RecordPlan::new(1, EVIDENCE_FIELD_COUNT)
        .expect("the fixed activation evidence plan is valid");
    let evidence_work = record_plan_work(evidence).expect("the fixed evidence work fits u64");
    RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(
        1,
        evidence_work,
        evidence_work,
    )
    .expect("the fixed failure-evidence envelope is valid")
    .attempted_work_limit()
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn exact_scene_activation_work_limit() -> u64 {
    let matrices = SequencePlan::<f32>::new(MATRIX_SCALAR_COUNT)
        .expect("the fixed activation matrix plan is valid");
    let references = RecordPlan::new(REFERENCE_COUNT, REFERENCE_FIELD_COUNT)
        .expect("the fixed activation reference plan is valid");
    let nodes = RecordPlan::new(NODE_COUNT, NODE_FIELD_COUNT)
        .expect("the fixed activation node plan is valid");
    let directory = RecordPlan::new(1, DIRECTORY_FIELD_COUNT)
        .expect("the fixed activation directory plan is valid");
    let evidence = RecordPlan::new(1, EVIDENCE_FIELD_COUNT)
        .expect("the fixed activation evidence plan is valid");
    let materialization =
        SceneActivationMaterializationWork::try_new(matrices, references, nodes, directory)
            .expect("the fixed activation work plan fits u64");
    let evidence_work = record_plan_work(evidence).expect("the fixed evidence work fits u64");
    let protected_failure_work = exact_scene_activation_protected_work_limit();
    let requirements =
        scene_activation_requirements().expect("the fixed activation requirements are valid");
    let structural_work = requirements.structural_work_requirement();

    materialization
        .total()
        .and_then(|work| {
            work.checked_add(scene_activation_preflight_work())
                .ok_or(StorePlanError::SizeOverflow)
        })
        .and_then(|work| {
            work.checked_add(evidence_work)
                .ok_or(StorePlanError::SizeOverflow)
        })
        .and_then(|work| {
            work.checked_add(scene_graph_validation_work())
                .ok_or(StorePlanError::SizeOverflow)
        })
        .and_then(|work| {
            work.checked_add(protected_failure_work)
                .ok_or(StorePlanError::SizeOverflow)
        })
        .and_then(|work| {
            work.checked_add(structural_work)
                .ok_or(StorePlanError::SizeOverflow)
        })
        .expect("the fixed activation total work fits u64")
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn exact_scene_activation_preflight_ordinary_work() -> u64 {
    scene_activation_requirements()
        .expect("the fixed activation requirements are valid")
        .pre_activation_work_requirement()
        .expect("the fixed activation preflight work fits u64")
}

impl SceneConstructionOperation {
    pub(in crate::canonical_scene::store) fn with_account_for_activation(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        let start = try_plan_scene_operation(
            limits,
            OperationControl::default(),
            TERMINAL_ATTRIBUTION_SLOTS,
            SceneFailureEvidenceWork::exact_single_record(EVIDENCE_FIELD_COUNT),
        )?;
        let raw = SceneStoreOperation::try_start(
            start.control,
            start.ledger_plan,
            account,
            start.start_plan,
        )?;
        Ok(Self {
            raw,
            brand: start.brand,
            keys: start.keys,
        })
    }

    /// Executes the first core-owned consumer of the sealed activation rail.
    ///
    /// This private U4d path ends at an operation-owned validated candidate. It shares the existing
    /// canonical builders and semantic validator, preserves rich/minimal terminal outcomes, and
    /// intentionally cannot publish until the later joint-commit slice exists.
    pub(in crate::canonical_scene::store) fn activate_fresh_two_node(
        self,
        authored: TwoNodeAuthoredMatrices,
    ) -> Result<SceneActivationTransition, SceneConstructionError> {
        let Self { raw, brand, keys } = self;
        let expected_brand = brand;
        let SceneActivationRequest {
            requirements,
            materialization,
            evidence_plan,
        } = SceneActivationRequest::try_new(brand, authored)?;
        let policy = scene_activation_policy(keys)?;
        let (owner, sealer) = raw.into_session_outcome_owner(policy);
        let request = sealer.seal(requirements);
        let authored_preflight = materialization.authored;

        let outcome = owner.plan_and_activate(
            request,
            move |mut preflight| {
                let valid = preflight
                    .inspect_semantic_work(|index| {
                        authored_activation_matrix_scalar_is_valid(&authored_preflight, index)
                    })
                    .unwrap_or(false);
                if valid {
                    preflight.seal::<()>()
                } else {
                    preflight.reject((), SEMANTIC_REJECTION_CODE)
                }
            },
            move |mut lane| {
                let matrices = match lane.try_retain_sequence(
                    materialization.matrices_plan,
                    AuthoredMatrixSchema,
                    |control| write_activation_matrices(control, &materialization.authored),
                ) {
                    Ok(matrices) => CanonicalBlockRef::from_store(matrices),
                    Err(_) => {
                        return lane
                            .reject::<SceneActivationOutput, (), RecordBlock, DirectorySchema>(
                                (),
                                FALLBACK_CODE,
                            );
                    }
                };
                let references = match lane.try_retain_records(
                    materialization.references_plan,
                    ReferenceTableSchema,
                    write_activation_references,
                ) {
                    Ok(references) => CanonicalBlockRef::from_store(references),
                    Err(_) => {
                        return lane
                            .reject::<SceneActivationOutput, (), RecordBlock, DirectorySchema>(
                                (),
                                FALLBACK_CODE,
                            );
                    }
                };
                let nodes = match lane.try_retain_records(
                    materialization.nodes_plan,
                    NodeTableSchema,
                    |control| write_activation_nodes(control, matrices),
                ) {
                    Ok(nodes) => CanonicalBlockRef::from_store(nodes),
                    Err(_) => {
                        return lane
                            .reject::<SceneActivationOutput, (), RecordBlock, DirectorySchema>(
                                (),
                                FALLBACK_CODE,
                            );
                    }
                };
                let directory = match lane.try_retain_records(
                    materialization.directory_plan,
                    DirectorySchema,
                    |control| write_activation_directory(control, nodes, references, matrices),
                ) {
                    Ok(directory) => CanonicalBlockRef::from_store(directory),
                    Err(_) => {
                        return lane
                            .reject::<SceneActivationOutput, (), RecordBlock, DirectorySchema>(
                                (),
                                FALLBACK_CODE,
                            );
                    }
                };
                let refs = SceneBlockRefs {
                    directory,
                    nodes,
                    references,
                    matrices,
                };
                lane.complete::<_, (), _, _>(
                    directory.into_store(),
                    SceneActivationOutput {
                        draft: SceneDraft {
                            brand: materialization.brand,
                            directory,
                        },
                        refs,
                    },
                )
            },
        )?;

        let activated = match outcome {
            CanonicalActivationOutcome::Activated(activated) => activated,
            CanonicalActivationOutcome::Rejected(rejected) => {
                let failure = rejected.build_failure_evidence(|lane| {
                    let evidence = match lane.try_retain_records(
                        evidence_plan,
                        OutcomeEvidenceSchema,
                        |writer| writer.write_row(write_outcome_evidence_row),
                    ) {
                        Ok(evidence) => evidence,
                        Err(_) => return lane.abort(),
                    };
                    lane.complete(evidence)
                })?;
                return prepare_failure_outcome(failure).map(SceneActivationTransition::Terminal);
            }
            CanonicalActivationOutcome::NeverActivated(never_activated) => {
                let failure = never_activated.build_failure_evidence(|lane| {
                    let evidence = match lane.try_retain_records(
                        evidence_plan,
                        OutcomeEvidenceSchema,
                        |writer| writer.write_row(write_outcome_evidence_row),
                    ) {
                        Ok(evidence) => evidence,
                        Err(_) => return lane.abort(),
                    };
                    lane.complete(evidence)
                })?;
                return prepare_failure_outcome(failure).map(SceneActivationTransition::Terminal);
            }
            CanonicalActivationOutcome::Fallback(fallback) => {
                let (store, _cause) = fallback.into_parts();
                return Ok(SceneActivationTransition::Terminal(
                    PreparedSceneConstruction::MinimalFallback(PreparedTracerSceneFallback {
                        store,
                    }),
                ));
            }
        };
        let canonical_blocks = activated.canonical_len();
        let outcome = activated.validate_centrally(
            |lane, _output| {
                let evidence =
                    match lane.try_retain_records(evidence_plan, OutcomeEvidenceSchema, |writer| {
                        writer.write_row(write_outcome_evidence_row)
                    }) {
                        Ok(evidence) => evidence,
                        Err(_) => return lane.abort(),
                    };
                lane.complete(evidence)
            },
            |session, directory, evidence, output| {
                let identity_valid = directory.identifies(output.draft.directory.into_store())
                    && output.draft.brand == expected_brand
                    && output.refs.directory == output.draft.directory;
                let valid = if identity_valid {
                    validate_scene_graph_metered(session, directory, evidence)?
                } else {
                    false
                };
                Ok(if valid {
                    CanonicalValidationDecision::accept(TracerSceneValidationProof)
                } else {
                    CanonicalValidationDecision::reject((), SEMANTIC_REJECTION_CODE)
                })
            },
        )?;
        match outcome {
            CanonicalCentralValidationOutcome::Candidate(candidate) => Ok(
                SceneActivationTransition::Candidate(PendingSceneActivation {
                    candidate,
                    canonical_blocks,
                }),
            ),
            CanonicalCentralValidationOutcome::Rejected(rejected) => prepare_failure_validation(
                rejected.validate_rich_failure(validate_failure_evidence)?,
            )
            .map(SceneActivationTransition::Terminal),
            CanonicalCentralValidationOutcome::Interrupted(interrupted) => {
                prepare_failure_validation(
                    interrupted.validate_rich_failure(validate_failure_evidence)?,
                )
                .map(SceneActivationTransition::Terminal)
            }
            CanonicalCentralValidationOutcome::Fallback(fallback) => {
                let (store, _cause) = fallback.into_parts();
                Ok(SceneActivationTransition::Terminal(
                    PreparedSceneConstruction::MinimalFallback(PreparedTracerSceneFallback {
                        store,
                    }),
                ))
            }
        }
    }
}

fn validate_failure_evidence<'session, 'storage, 'control>(
    session: &mut FailureEvidenceValidationSession<
        'session,
        'storage,
        'control,
        SceneConstructionOwner,
        SceneConstructionPhase,
        SceneConstructionUnit,
    >,
    root: FailureEvidenceMember<'session, RecordBlock, OutcomeEvidenceSchema>,
) -> bool {
    session
        .visit_records(root, |row_index, row| {
            row_index == 0
                && row
                    .scalar::<u32, _>(0, |value| value == SCHEMA_REVISION)
                    .unwrap_or(false)
                && row
                    .scalar::<u32, _>(1, |value| value == EVIDENCE_OUTCOME_CANDIDATE)
                    .unwrap_or(false)
                && row
                    .scalar::<u32, _>(2, |value| value == CANONICAL_BLOCK_COUNT)
                    .unwrap_or(false)
                && row
                    .scalar::<u32, _>(3, |value| value == NODE_COUNT as u32)
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn prepare_failure_outcome<Cause>(
    outcome: FailureEvidenceOutcome<
        SceneConstructionOwner,
        SceneConstructionPhase,
        SceneConstructionUnit,
        Cause,
        RecordBlock,
        OutcomeEvidenceSchema,
    >,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => {
            prepare_failure_validation(rooted.validate_rich_failure(validate_failure_evidence)?)
        }
        FailureEvidenceOutcome::MinimalFallback(fallback) => {
            let (store, _cause) = fallback.into_parts();
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn prepare_failure_validation<Cause>(
    validation: RootedFailureValidation<
        SceneConstructionOwner,
        SceneConstructionPhase,
        SceneConstructionUnit,
        Cause,
        RecordBlock,
        OutcomeEvidenceSchema,
    >,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    match validation {
        RootedFailureValidation::Valid(validated) => {
            let (prepared, _cause) = validated.try_prepare(final_progress())?.into_parts();
            map_prepared_failure(prepared)
        }
        RootedFailureValidation::Rejected(rejected) => {
            let (store, _cause) = rejected.prepare_minimal_fallback()?.into_parts();
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
        RootedFailureValidation::Fallback(fallback) => {
            let (store, _cause) = fallback.into_parts();
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}
