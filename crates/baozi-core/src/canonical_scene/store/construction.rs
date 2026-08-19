#![allow(
    clippy::result_large_err,
    reason = "retained construction errors preserve bounded fail-closed evidence inline without an unmetered failure-path allocation"
)]

use std::num::{NonZeroU32, NonZeroU64};

use baozi_operation::__retained_store_integration::activation::{
    CanonicalFailureTerminalWorkPlan, StoreActivationContainment,
};
use baozi_operation::{
    DimensionPolicy, FinalProgress, FreshStoreLineage, LedgerLimits, LedgerLimitsError,
    OperationControl, OperationOwner, ProgressPhase, ProgressUnit, RetainedStoreBatchLimits,
    RetainedStoreFailureTerminalWorkPlan, RetainedStoreFailureTerminalWorkRequirement,
    RetainedStoreOperation, RetainedStorePublicationAssembly, RetainedStoreSharePlan,
    RetainedStoreStartError, RetainedStoreStartPlan, RetainedStoreWriterPlan, StoreDimensionKey,
    StoreDimensionSemanticId, StoreKeyDomain, StoreKeyDomainCreateError, StoreLedgerPlan,
    StoreLedgerPlanError, StorePhaseKey, StorePhaseSemanticId, StorePlanError, StorePrepareError,
    StoreRuntimeError, StoreSemanticCatalogId,
};
use thiserror::Error;

use crate::identity::ReservedSceneGeneration;

use super::production::SceneInputEnvelopeError;
use super::schema::NODE_SCENE_EVIDENCE_FIELD_COUNT;

pub(super) mod node_scene;

const STORE_CATALOG: StoreSemanticCatalogId =
    StoreSemanticCatalogId::new(*b"baozi-core-scene", NonZeroU32::MIN);
const STORE_PHASE: StorePhaseSemanticId = StorePhaseSemanticId::new(STORE_CATALOG, NonZeroU64::MIN);
pub(super) const STORE_DIMENSION: StoreDimensionSemanticId =
    StoreDimensionSemanticId::new(STORE_CATALOG, NonZeroU64::MIN);
pub(super) const FAILURE_TERMINAL_WORK_DIMENSION: StoreDimensionSemanticId =
    StoreDimensionSemanticId::new(STORE_CATALOG, NonZeroU64::new(2).unwrap());
const FALLBACK_CODE: u32 = 0x5343_0001;
const GENERATION_EXHAUSTED_CODE: u32 = 0x5343_0003;
const TERMINAL_COMMITMENT_SLOTS: usize = 8;
pub(in crate::canonical_scene::store) const FAILURE_TERMINAL_EVIDENCE_MEMBERS: usize = 1;

pub(in crate::canonical_scene::store) fn failure_terminal_work_requirement()
-> Result<RetainedStoreFailureTerminalWorkRequirement, StorePlanError> {
    RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(
        FAILURE_TERMINAL_EVIDENCE_MEMBERS,
        NODE_SCENE_EVIDENCE_FIELD_COUNT as u64,
        NODE_SCENE_EVIDENCE_FIELD_COUNT as u64,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::canonical_scene::store) enum SceneConstructionPhase {
    Validation,
    Publication,
}

impl ProgressPhase for SceneConstructionPhase {
    fn order(self) -> u32 {
        self as u32
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Validation => "canonical-scene-validation",
            Self::Publication => "canonical-scene-publication",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::canonical_scene::store) enum SceneConstructionUnit {
    WorkUnits,
    Scene,
}

impl ProgressUnit for SceneConstructionUnit {
    fn stable_id(self) -> &'static str {
        match self {
            Self::WorkUnits => "work_units",
            Self::Scene => "scene",
        }
    }
}

pub(in crate::canonical_scene::store) struct SceneConstructionOwner;

impl OperationOwner for SceneConstructionOwner {}

type SceneStoreOperation<L = FreshStoreLineage> = RetainedStoreOperation<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    L,
>;
type SceneStoreControlCheckpoint<'a> = baozi_operation::RetainedStoreControlCheckpoint<
    'a,
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
>;
type SceneStoreAssembly<L = FreshStoreLineage> = RetainedStorePublicationAssembly<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    L,
>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SceneConstructionLimits {
    pub(crate) operation_live_bytes: u64,
    pub(crate) dimension_live_bytes: u64,
    pub(crate) attempted_work: u64,
    pub(crate) consumed_work: u64,
    pub(crate) committed_output: u64,
    pub(crate) published_evidence: u64,
}

#[derive(Debug, Error)]
pub(crate) enum SceneConstructionStartError {
    #[error("canonical Scene input envelope could not be admitted before operation start")]
    InputEnvelope(#[from] SceneInputEnvelopeError),
    #[error("canonical Scene construction-brand space is exhausted")]
    BrandExhausted,
    #[error("the canonical Scene store key domain could not be created")]
    KeyDomain(#[from] StoreKeyDomainCreateError),
    #[error("the canonical Scene store semantic key could not be admitted")]
    KeyAdmissionInvariant,
    #[error("the canonical Scene work budget cannot cover its bounded terminal reserve")]
    TerminalWorkBudget,
    #[error("canonical Scene ledger limits are invalid")]
    LedgerLimits(#[from] LedgerLimitsError<StoreDimensionKey>),
    #[error("the canonical Scene store ledger plan is invalid")]
    LedgerPlan(#[from] StoreLedgerPlanError),
    #[error("the canonical Scene retained-store plan is invalid")]
    StorePlan(#[from] StorePlanError),
    #[error("the canonical Scene retained-store operation could not start")]
    StoreStart(#[from] RetainedStoreStartError),
}

#[derive(Debug, Error)]
pub(crate) enum SceneConstructionError {
    #[error("the canonical Scene retained-store publication plan is invalid")]
    StorePlan(#[from] StorePlanError),
    #[error("canonical Scene retained-store construction failed")]
    Store(#[from] StoreRuntimeError),
    #[error("canonical Scene terminal preparation failed closed")]
    Prepare(#[from] StorePrepareError),
    #[error("canonical Scene activation failed closed")]
    Activation(#[from] StoreActivationContainment),
    #[error("the canonical Scene validation proof does not match its output descriptor")]
    JointCommitInvariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConstructionBrand(node_scene::ConstructionRequestIdentity);

#[derive(Clone, Copy)]
struct SceneStoreKeys {
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
    failure_terminal_work_dimension: StoreDimensionKey,
}

struct SceneOperationStart {
    keys: SceneStoreKeys,
    control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ledger_plan: StoreLedgerPlan,
    start_plan: RetainedStoreStartPlan,
}

#[derive(Clone, Copy)]
struct SceneFailureEvidenceWork {
    max_evidence_members: usize,
    construction_work_limit: u64,
    validation_work_limit: u64,
}

impl SceneFailureEvidenceWork {
    const fn exact_single_record(fields: usize) -> Self {
        Self {
            max_evidence_members: 1,
            construction_work_limit: fields as u64,
            validation_work_limit: fields as u64,
        }
    }
}

fn try_plan_scene_operation(
    limits: SceneConstructionLimits,
    control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    terminal_attribution_slots: usize,
    failure_evidence_work: SceneFailureEvidenceWork,
) -> Result<SceneOperationStart, SceneConstructionStartError> {
    let domain = StoreKeyDomain::try_new(STORE_CATALOG)?;
    let keys = SceneStoreKeys {
        phase: domain
            .try_admit_phase(STORE_PHASE)
            .ok_or(SceneConstructionStartError::KeyAdmissionInvariant)?,
        dimension: domain
            .try_admit_dimension(STORE_DIMENSION)
            .ok_or(SceneConstructionStartError::KeyAdmissionInvariant)?,
        failure_terminal_work_dimension: domain
            .try_admit_dimension(FAILURE_TERMINAL_WORK_DIMENSION)
            .ok_or(SceneConstructionStartError::KeyAdmissionInvariant)?,
    };
    let failure_terminal_work = CanonicalFailureTerminalWorkPlan::new(
        keys.failure_terminal_work_dimension,
        failure_evidence_work.max_evidence_members,
        failure_evidence_work.construction_work_limit,
        failure_evidence_work.validation_work_limit,
        limits.attempted_work,
        limits.consumed_work,
    )
    .map_err(|cause| match cause {
        StorePlanError::InvalidFailureTerminalWork => {
            SceneConstructionStartError::TerminalWorkBudget
        }
        cause => SceneConstructionStartError::StorePlan(cause),
    })?;
    let dimension_policy = DimensionPolicy::reservable_bytes(
        failure_terminal_work.ordinary_attempted_work_limit(),
        failure_terminal_work.ordinary_consumed_work_limit(),
        limits.committed_output,
        limits.dimension_live_bytes,
    )
    .with_published_outcome_evidence_limit(limits.published_evidence);
    let ledger_limits = LedgerLimits::new(
        limits.operation_live_bytes,
        [
            (keys.dimension, dimension_policy),
            (
                keys.failure_terminal_work_dimension,
                failure_terminal_work.requirement().dimension_policy(),
            ),
        ],
    )?;
    let ledger_plan = StoreLedgerPlan::try_new(domain, ledger_limits, &[keys.phase])?;
    let terminal_limits =
        RetainedStoreBatchLimits::new(terminal_attribution_slots, TERMINAL_COMMITMENT_SLOTS)?;
    let terminal_writer =
        RetainedStoreWriterPlan::new(keys.phase, keys.dimension, keys.dimension, terminal_limits);
    let start_plan = RetainedStoreStartPlan::new(
        terminal_writer,
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        baozi_operation::StoreFallbackEnvelope::new(FALLBACK_CODE),
        keys.dimension,
        1,
        keys.dimension,
        1,
        failure_terminal_work.into_store_plan(),
    );
    Ok(SceneOperationStart {
        keys,
        control,
        ledger_plan,
        start_plan,
    })
}

fn burn_generation_reservation(_reservation: ReservedSceneGeneration) {}

const fn final_progress() -> FinalProgress<SceneConstructionPhase, SceneConstructionUnit> {
    FinalProgress::new(
        SceneConstructionPhase::Publication,
        SceneConstructionUnit::Scene,
        1,
    )
}
