//! Scoped session activation for the canonical retained-store lane.
//!
//! The public crate surface keeps this module behind `__canonical_activation` during U4. One
//! call owns preflight, physical open, materialization, rejection cleanup, and containment. No
//! request token or dormant authority can escape the call.

use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::{
    ByteBlock, BytesPlan, BytesWriter, CanonicalBytesRef, CanonicalRecordRef,
    CanonicalRecordWriter, CanonicalSequenceRef, CanonicalStoreRef, CanonicalUtf8Ref,
    FreshStoreLineage, OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef,
    OutcomeEvidenceRecordRowWriter, OutcomeEvidenceRecordView, OutcomeEvidenceRecordWriter,
    OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef, OutcomeEvidenceUtf8Ref,
    PreparedStoreFailureOutcome, PreparedStoreFallback, RawStoreFailureAssembly, RecordBlock,
    RecordPlan, RetainedStoreBlockTablePlan, RetainedStoreControlCheckpoint,
    RetainedStoreFailureTerminalWorkPlan, RetainedStoreFailureTerminalWorkRequirement,
    RetainedStoreOperation, RetainedStorePublicationAssembly, RetainedStorePublicationPlan,
    RetainedStoreReservation, RetainedStoreWorkControl, ScalarSequence, SequencePlan,
    SequenceWriter, StoreBuildError, StoreControlledError, StoreControlledSuccessValidation,
    StoreDimensionKey, StoreEvidenceValidationSession, StoreFailureAssembly,
    StoreFailureValidation, StoreFallbackOnly, StoreKeyDomain, StorePhaseKey, StorePlanError,
    StorePrepareError, StoreRuntimeError, StoreValidationSession, StoreWorkError, StoredBlock,
    StoredScalar, Utf8Block, Utf8Plan, Utf8Writer, ValidatedStoreFailure, ValidatedStoreSuccess,
    classify_prepared_fallback, map_publication_error, map_writer_error,
};
use crate::control::{ProgressPhase, ProgressUnit, discard_untrusted_panic_payload};
use crate::coordinator::FinalProgress;
use crate::owner::OperationOwner;
use crate::retained::{
    FinalizationContainmentError, PreparationFailureCode, RetainedFailureWorkConsumeError,
    RetainedFailureWorkControl, RetainedFailureWorkFinishError, RetainedFailureWorkPermit,
    RetainedFailureWorkStartError, RetainedPublicationOpenRecovery,
    RetainedPublicationPrepareError,
};

mod canonical;
mod failure_evidence;
mod fallback;
mod preflight;

const ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE: u32 = 0x4143_5047;
const FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE: u32 = 0x4657_4545;
const FAILURE_EVIDENCE_BUILD_CHUNK_UNITS: usize = 256;

/// Operation-derived protected work plan for the canonical activation failure path.
///
/// Construction and validation work are derived from sealed store plans. Each unit reserves the
/// protected charge pair used by the retained terminal rail, so callers never provide either the
/// protected dimension key or a self-attested work amount.
#[derive(Debug, PartialEq, Eq)]
pub struct CanonicalFailureTerminalWorkPlan {
    terminal_work: RetainedStoreFailureTerminalWorkPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureEvidenceWorkEnvelope {
    max_evidence_members: usize,
    construction_work_limit: u64,
    validation_work_limit: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureEvidenceWorkBudget {
    remaining_members: usize,
    remaining_construction_work: u64,
    remaining_validation_work: u64,
    planned_validation_work: u64,
}

impl FailureEvidenceWorkBudget {
    const fn new(envelope: FailureEvidenceWorkEnvelope) -> Self {
        Self {
            remaining_members: envelope.max_evidence_members,
            remaining_construction_work: envelope.construction_work_limit,
            remaining_validation_work: envelope.validation_work_limit,
            planned_validation_work: 0,
        }
    }

    fn reserve_construction_member(&mut self, amount: u64) -> Result<(), StoreRuntimeError> {
        if amount == 0
            || self.remaining_members == 0
            || amount > self.remaining_construction_work
            || amount > self.remaining_validation_work
        {
            return Err(failure_work_envelope_exhausted());
        }
        let Some(planned) = self.planned_validation_work.checked_add(amount) else {
            return Err(failure_work_envelope_exhausted());
        };
        self.remaining_members -= 1;
        self.remaining_construction_work -= amount;
        self.remaining_validation_work -= amount;
        self.planned_validation_work = planned;
        Ok(())
    }

    fn planned_validation_work(&self) -> Result<NonZeroU64, StoreRuntimeError> {
        NonZeroU64::new(self.planned_validation_work).ok_or_else(failure_work_envelope_exhausted)
    }
}

type RawFailureEvidenceWorkControl<'a, O, PP, U> =
    RetainedFailureWorkControl<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>;
type RawFailureEvidenceWorkPermit<'a, O, PP, U> =
    RetainedFailureWorkPermit<'a, O, StorePhaseKey, PP, U, StoreDimensionKey>;

/// Permit-bound protected writer for one admitted failure-evidence member.
///
/// The complete sealed member plan is recorded as attempted before the builder runs. Each mutating
/// method polls control state and records only the payload it actually advances as consumed. A
/// writer error, cancellation, panic, or abandoned permit therefore preserves the unfinished
/// attempted-only suffix and forces the terminal path away from rich evidence.
pub struct FailureEvidenceBuildControl<'a, O, PP, U, W>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    writer: W,
    permit: Option<RawFailureEvidenceWorkPermit<'a, O, PP, U>>,
    record_action_work: u64,
    sticky: Option<FailureBuildCallbackError>,
}

#[derive(Clone)]
enum FailureBuildCallbackError {
    Build(StoreBuildError),
    Runtime(StoreRuntimeError),
}

/// Operation-owned validator for one exact rich-failure evidence graph.
///
/// No raw slice, string, record view, protected dimension, or caller-selected work amount crosses
/// this boundary. Each concrete data unit advances under a linear permit and is recorded as
/// consumed only after the caller's inspection closure returns normally. A repeated DAG edge to an
/// already completed member succeeds without running that member's inspector again.
pub struct FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: StoreEvidenceValidationSession<'session, 'storage>,
    permit: Option<RawFailureEvidenceWorkPermit<'control, O, PP, U>>,
    sticky: Option<StoreRuntimeError>,
    completed_units: u64,
    rejected: bool,
}

/// Session-branded authority to validate one root-reachable failure-evidence member.
///
/// Values are issued only for the validation root or a typed record edge. They are deliberately
/// neither `Copy` nor constructible by callers, so a slot captured during construction cannot be
/// substituted for a member reached from the validation root.
#[must_use = "every root-reachable failure-evidence member must enter controlled validation"]
pub struct FailureEvidenceMember<'session, Kind, Tag> {
    raw: OutcomeEvidenceStoreRef<Kind, Tag>,
    validation: PhantomData<fn(&'session mut ()) -> &'session mut ()>,
}

type FailureEvidenceSequenceMember<'session, T, Tag> =
    FailureEvidenceMember<'session, ScalarSequence<T>, Tag>;
type FailureEvidenceBytesMember<'session, Tag> = FailureEvidenceMember<'session, ByteBlock, Tag>;
type FailureEvidenceUtf8Member<'session, Tag> = FailureEvidenceMember<'session, Utf8Block, Tag>;
type FailureEvidenceRecordMember<'session, Tag> = FailureEvidenceMember<'session, RecordBlock, Tag>;

/// One fixed-width record row whose fields can only be inspected through metered typed accessors.
pub struct FailureEvidenceRecordRow<'row, 'session, 'storage, 'control, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    session: &'row mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
    view: OutcomeEvidenceRecordView<'storage>,
    row: usize,
    fields: usize,
    visited: u64,
}

/// Linear owner for one session outcome and one still-dormant canonical authority.
#[must_use = "the session outcome owner must run its scoped activation transition"]
pub struct RetainedStoreSessionOutcomeOwner<O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: Option<RetainedStoreOperation<O, PP, U, L>>,
    outcome_evidence_plan: RetainedStoreBlockTablePlan,
    failure_work: FailureEvidenceWorkBudget,
}

fn finish_rejected<O, PP, U, T, E, L>(
    assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    failure_work: FailureEvidenceWorkBudget,
    cause: StoreCanonicalRejection<E>,
    failure_code: PreparationFailureCode,
) -> Result<CanonicalActivationOutcome<O, PP, U, T, E, L>, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let cause = ManuallyDrop::new(cause);
    let RetainedStorePublicationAssembly {
        raw,
        fallback_cause: _,
        success_rejection: _,
        domain,
        lineage,
    } = assembly;
    let lineage = ManuallyDrop::new(lineage);
    match raw.try_discard_canonical_for_rejection(failure_code) {
        Ok(raw) => {
            if drop_scoped_value(ManuallyDrop::into_inner(lineage)).is_err() {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
            Ok(CanonicalActivationOutcome::Rejected(
                RejectedCanonicalStoreSession {
                    owner: UnrootedFailureEvidence {
                        raw,
                        domain,
                        failure_code,
                        cause: ManuallyDrop::into_inner(cause),
                        failure_work,
                    },
                },
            ))
        }
        Err(containment) => {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            let _ = drop_scoped_value(ManuallyDrop::into_inner(lineage));
            Err(StoreActivationContainment { cause: containment })
        }
    }
}

struct ActivationFirstFault {
    cause: StoreRuntimeError,
    contract_seen: bool,
}

impl ActivationFirstFault {
    fn recoverable_code(&self) -> Option<PreparationFailureCode> {
        match classify_activation_fault(&self.cause) {
            ActivationFaultClassification::Recoverable(code) => Some(code),
            ActivationFaultClassification::Contract => None,
        }
    }
}

fn preserve_first_fault<T>(
    first_fault: &mut Option<ActivationFirstFault>,
    result: Result<T, StoreRuntimeError>,
) -> Result<T, StoreRuntimeError> {
    if let Err(cause) = &result {
        let classification = classify_activation_fault(cause);
        let contract = matches!(classification, ActivationFaultClassification::Contract);
        match first_fault {
            Some(first) => first.contract_seen |= contract,
            None => {
                *first_fault = Some(ActivationFirstFault {
                    cause: cause.clone(),
                    contract_seen: contract,
                });
            }
        }
    }
    result
}

fn preserved_fault(first_fault: &Option<ActivationFirstFault>) -> Option<StoreRuntimeError> {
    first_fault.as_ref().map(|fault| fault.cause.clone())
}

fn note_later_contract<T>(
    first_fault: &mut Option<ActivationFirstFault>,
    result: Result<T, StoreRuntimeError>,
) {
    if let Err(cause) = result {
        let _ = preserve_first_fault::<()>(first_fault, Err(cause));
    }
}

fn validate_dimension_contract(
    domain: &StoreKeyDomain,
    dimension: StoreDimensionKey,
) -> Result<(), StoreRuntimeError> {
    if domain.owns_dimension(dimension) {
        Ok(())
    } else {
        Err(StoreRuntimeError::ForeignDimensionDomain { dimension })
    }
}

fn drop_verified_rejection<E>(rejection: VerifiedRejection<E>) -> Result<(), ()> {
    let VerifiedRejection {
        cause,
        failure_code: _,
    } = rejection;
    drop_scoped_value(cause)
}

fn drop_materialization<T, E>(materialized: CanonicalMaterialization<T, E>) -> Result<(), ()> {
    match materialized.state {
        CanonicalMaterializationState::Activated(output) => drop_scoped_value(output),
        CanonicalMaterializationState::Rejected(rejection) => drop_verified_rejection(rejection),
    }
}

fn drop_scoped_value<T>(value: T) -> Result<(), ()> {
    match catch_unwind(AssertUnwindSafe(|| drop(value))) {
        Ok(()) => Ok(()),
        Err(payload) => {
            discard_untrusted_panic_payload(payload);
            Err(())
        }
    }
}

fn drop_optional_callback<F>(callback: ManuallyDrop<Option<F>>) -> Result<(), ()> {
    match ManuallyDrop::into_inner(callback) {
        Some(callback) => drop_scoped_value(callback),
        None => Ok(()),
    }
}

fn finish_scoped_callback<F, T>(
    callback: ManuallyDrop<Option<F>>,
    result: Result<T, StoreRuntimeError>,
) -> Result<T, StoreRuntimeError> {
    if drop_optional_callback(callback).is_ok() {
        result
    } else {
        let _ = drop_scoped_value(result);
        Err(StoreRuntimeError::BuilderPanicked)
    }
}

#[derive(Clone, Copy)]
enum ActivationFaultClassification {
    Contract,
    Recoverable(PreparationFailureCode),
}

fn classify_activation_fault(cause: &StoreRuntimeError) -> ActivationFaultClassification {
    match cause {
        StoreRuntimeError::Plan(_)
        | StoreRuntimeError::BatchCapacity
        | StoreRuntimeError::ForeignPhaseDomain { .. }
        | StoreRuntimeError::ForeignDimensionDomain { .. }
        | StoreRuntimeError::PhaseCompletionRegression { .. }
        | StoreRuntimeError::PhaseCompletionPublicationPending
        | StoreRuntimeError::PhaseCompletionPublicationCompleted
        | StoreRuntimeError::PhaseCompletionCatalogShapeMismatch
        | StoreRuntimeError::AccountPoisoned(_)
        | StoreRuntimeError::AccountPoisonedWithRetainedCharge(_)
        | StoreRuntimeError::BuilderPanicked
        | StoreRuntimeError::ConstructionInvariantUnverified
        | StoreRuntimeError::WrongOutcomeMode
        | StoreRuntimeError::InvalidSourceSlot
        | StoreRuntimeError::ForeignAccount
        | StoreRuntimeError::MembershipExhausted
        | StoreRuntimeError::InvalidPublicationPlan
        | StoreRuntimeError::StorageDropUnverified
        | StoreRuntimeError::OwnershipUnverified
        | StoreRuntimeError::ChargeReleaseUnverified
        | StoreRuntimeError::SuccessOutcomeRejected
        | StoreRuntimeError::UnverifiedControlledCause
        | StoreRuntimeError::FallbackOnly
        | StoreRuntimeError::Poisoned => ActivationFaultClassification::Contract,
        StoreRuntimeError::Control(cause) => {
            ActivationFaultClassification::Recoverable(PreparationFailureCode::from_control(cause))
        }
        StoreRuntimeError::Ledger(_) | StoreRuntimeError::AccountingOverflow => {
            ActivationFaultClassification::Recoverable(PreparationFailureCode::ledger())
        }
        StoreRuntimeError::Build(cause) => ActivationFaultClassification::Recoverable(
            PreparationFailureCode::product_graph(if cause.code() == 0 {
                ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE
            } else {
                cause.code()
            }),
        ),
        StoreRuntimeError::BatchAllocationFailed
        | StoreRuntimeError::PhysicalByteLimit { .. }
        | StoreRuntimeError::PhysicalAllocationLimit { .. }
        | StoreRuntimeError::AllocationFailed
        | StoreRuntimeError::SlotLimit { .. } => ActivationFaultClassification::Recoverable(
            PreparationFailureCode::product_graph(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE),
        ),
    }
}

fn activation_product_graph_cause() -> StoreRuntimeError {
    StoreRuntimeError::Build(
        StoreBuildError::domain(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE)
            .expect("the activation product-graph code is inside the domain range"),
    )
}

fn failure_work_envelope_exhausted() -> StoreRuntimeError {
    StoreRuntimeError::Build(
        StoreBuildError::domain(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE)
            .expect("the failure-work envelope code is inside the domain range"),
    )
}

fn prepare_product_graph_fallback<O, PP, U>(
    fallback: StoreFallbackOnly<O, PP, U>,
) -> Result<VerifiedMinimalFallback<StoreRuntimeError>, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match fallback.prepare_minimal_fallback() {
        Ok(prepared) => Ok(VerifiedMinimalFallback {
            prepared,
            cause: activation_product_graph_cause(),
        }),
        Err(StorePrepareError::FinalizationContainment(cause)) => {
            Err(StoreActivationContainment { cause })
        }
    }
}

fn contain_operation<O, PP, U, L>(
    operation: RetainedStoreOperation<O, PP, U, L>,
) -> StoreActivationContainment
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let RetainedStoreOperation {
        raw,
        domain: _,
        lineage,
    } = operation;
    let _ = drop_scoped_value(lineage);
    StoreActivationContainment {
        cause: raw.contain_activation_fault(),
    }
}

// Keeping the linear operation owner inline avoids an unmetered allocation on activation failure.
#[allow(clippy::large_enum_variant)]
enum ActivationOpenFailure<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Recoverable {
        operation: RetainedStoreOperation<O, PP, U, L>,
        cause: StoreRuntimeError,
        failure_code: PreparationFailureCode,
        fallback_only: bool,
    },
    Contained(FinalizationContainmentError),
}

#[allow(clippy::result_large_err, clippy::type_complexity)]
fn try_open_activation<O, PP, U, L>(
    operation: RetainedStoreOperation<O, PP, U, L>,
    plan: RetainedStorePublicationPlan,
) -> Result<RetainedStorePublicationAssembly<O, PP, U, L>, ActivationOpenFailure<O, PP, U, L>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if operation.validate_publication_plan_keys(&plan).is_err() {
        return Err(ActivationOpenFailure::Contained(
            contain_operation(operation).cause,
        ));
    }
    let RetainedStoreOperation {
        raw,
        domain,
        lineage,
    } = operation;
    match raw.try_publication_assembly(plan.into_raw()) {
        Ok(raw) => Ok(RetainedStorePublicationAssembly {
            raw,
            fallback_cause: None,
            success_rejection: None,
            domain,
            lineage,
        }),
        Err(error) => {
            let cause = map_writer_error(error.cause());
            let classification = classify_activation_fault(&cause);
            match error.into_activation_recovery() {
                Ok(recovery) => {
                    let (raw, fallback_only) = match recovery {
                        RetainedPublicationOpenRecovery::Retry(raw) => (raw, false),
                        RetainedPublicationOpenRecovery::FallbackOnly(raw) => (raw, true),
                    };
                    let mut operation = RetainedStoreOperation {
                        raw,
                        domain,
                        lineage,
                    };
                    match classification {
                        ActivationFaultClassification::Contract => Err(
                            ActivationOpenFailure::Contained(contain_operation(operation).cause),
                        ),
                        ActivationFaultClassification::Recoverable(failure_code) => {
                            operation.latch_activation_terminal_cause(failure_code);
                            Err(ActivationOpenFailure::Recoverable {
                                operation,
                                cause,
                                failure_code,
                                fallback_only,
                            })
                        }
                    }
                }
                Err(containment) => Err(ActivationOpenFailure::Contained(containment)),
            }
        }
    }
}

/// Borrowed preflight capability. It cannot open or mutate retained tables.
pub struct CanonicalPreflight<'scope, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: &'scope mut RetainedStoreOperation<O, PP, U, L>,
    first_fault: &'scope mut Option<ActivationFirstFault>,
}

/// Control-and-budget facade whose every failure is preserved by the activation transaction.
pub struct CanonicalControlCheckpoint<'borrow, 'activation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RetainedStoreControlCheckpoint<'borrow, O, PP, U>,
    first_fault: &'borrow mut Option<ActivationFirstFault>,
    activation: PhantomData<fn(&'activation ()) -> &'activation ()>,
}

/// Scoped live-storage reservation for canonical construction scratch.
///
/// The invariant lifetime brand prevents a reservation from becoming part of a preflight decision
/// or materialization output. The type remains private to the scoped callback surface, so the
/// activation integration tests exercise the real API rather than importing an unexported type.
#[must_use = "dropping a canonical reservation releases its live-byte accounting"]
pub struct CanonicalReservation<'scope, O>
where
    O: OperationOwner,
{
    raw: RetainedStoreReservation<O>,
    scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
}

/// Opaque result returned by the scoped preflight callback.
#[must_use = "return the sealed activation or verified rejection to the owning session"]
pub struct CanonicalPreflightDecision<E> {
    state: CanonicalPreflightState<E>,
}

enum CanonicalPreflightState<E> {
    Activate {
        canonical_plan: RetainedStoreBlockTablePlan,
    },
    Rejected(VerifiedRejection<E>),
}

struct VerifiedRejection<E> {
    cause: E,
    failure_code: PreparationFailureCode,
}

/// Canonical-only lane borrowed for exactly one materialization callback.
pub struct CanonicalLane<'lane, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly: &'lane mut RetainedStorePublicationAssembly<O, PP, U, L>,
    first_fault: &'lane mut Option<ActivationFirstFault>,
}

/// Opaque result returned by the scoped materialization callback.
#[must_use = "return the materialized candidate or typed rejection to the owning session"]
pub struct CanonicalMaterialization<T, E> {
    state: CanonicalMaterializationState<T, E>,
}

enum CanonicalMaterializationState<T, E> {
    Activated(T),
    Rejected(VerifiedRejection<E>),
}

/// Exact pre-activation rejection or recoverable open cause coupled to its continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreCanonicalRejection<E> {
    Requested(E),
    Runtime(StoreRuntimeError),
}

/// A materialized canonical candidate whose physical graph has not yet been published.
///
/// The activation seam deliberately exposes no writable assembly or raw ownership bridge.
#[must_use = "the candidate must be prepared or explicitly downgraded"]
pub struct ActivatedCanonicalStoreSession<O, PP, U, T, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    output: T,
    failure_work: FailureEvidenceWorkBudget,
}

/// Canonical output commitment owned by the consuming validation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalOutputIntent {
    dimension: StoreDimensionKey,
    amount: u64,
}

/// Typed semantic outcome returned by the central validator callback.
pub enum CanonicalValidationDecision<Proof, E> {
    Accept(Proof),
    Reject { cause: E, stable_code: u32 },
}

fn drop_validation_decision<Proof, E>(
    decision: Option<CanonicalValidationDecision<Proof, E>>,
) -> Result<(), ()> {
    match decision {
        Some(CanonicalValidationDecision::Accept(proof)) => drop_scoped_value(proof),
        Some(CanonicalValidationDecision::Reject {
            cause,
            stable_code: _,
        }) => drop_scoped_value(cause),
        None => Ok(()),
    }
}

/// One-shot evidence decision used by the central success path.
pub enum OutcomeEvidenceDecision<Kind, Tag> {
    Complete(OutcomeEvidenceStoreRef<Kind, Tag>),
    Abort,
}

/// Candidate after central semantic validation. It contains no writable or raw store seam.
///
/// This activation-only disposition deliberately has no prepare or handoff method. The later
/// cross-crate joint-commit slice must combine an operation-issued freeze permit with the exact
/// core-owned generation contribution before either side can prepare a success.
#[must_use = "the validated candidate must enter the later joint-commit transition"]
pub struct CanonicalCandidateStoreSession<
    O,
    PP,
    U,
    T,
    Proof,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    _validated:
        ValidatedStoreSuccess<O, PP, U, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>,
    proof: Proof,
    output: T,
    _failure_work: FailureEvidenceWorkBudget,
}

/// A centrally rejected canonical candidate with an eager-cleaned, rooted failure owner.
#[must_use = "the rejected candidate must validate rich evidence or prepare fallback"]
pub struct CentrallyRejectedCanonicalStoreSession<O, PP, U, E, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    root: RootedFailureEvidence<O, PP, U, StoreCanonicalRejection<E>, EvidenceKind, EvidenceTag>,
}

/// Rich-failure validation result for a centrally rejected canonical candidate.
pub type CanonicalRejectionFailureValidation<O, PP, U, E, EvidenceKind, EvidenceTag> =
    RootedFailureValidation<O, PP, U, StoreCanonicalRejection<E>, EvidenceKind, EvidenceTag>;

/// A centrally interrupted candidate with a trusted control or ledger cause.
#[must_use = "the interrupted candidate must validate evidence or prepare fallback"]
pub struct InterruptedCanonicalStoreSession<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    root: RootedFailureEvidence<O, PP, U, StoreWorkError, EvidenceKind, EvidenceTag>,
}

/// Opaque validation outcome for a materialized canonical session.
#[must_use = "every central-validation branch retains its exact owner"]
pub enum CanonicalCentralValidationOutcome<
    O,
    PP,
    U,
    T,
    Proof,
    E,
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Candidate(
        CanonicalCandidateStoreSession<
            O,
            PP,
            U,
            T,
            Proof,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
    ),
    Rejected(CentrallyRejectedCanonicalStoreSession<O, PP, U, E, EvidenceKind, EvidenceTag>),
    Fallback(VerifiedMinimalFallback<StoreRuntimeError>),
    Interrupted(InterruptedCanonicalStoreSession<O, PP, U, EvidenceKind, EvidenceTag>),
}

/// One-shot canonical outcome-evidence writer used only before central validation.
pub struct CanonicalOutcomeEvidenceLane<'lane, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly: &'lane mut RetainedStorePublicationAssembly<O, PP, U, L>,
    first_fault: &'lane mut Option<ActivationFirstFault>,
}

/// One-shot decision for rejected or never-activated evidence.
pub enum FailureEvidenceDecision<Kind, Tag> {
    Complete(OutcomeEvidenceStoreRef<Kind, Tag>),
    Abort,
}

/// Unrooted failure owner shared by materialization rejection and never-activation.
struct UnrootedFailureEvidence<O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawStoreFailureAssembly<O, PP, U>,
    domain: StoreKeyDomain,
    failure_code: PreparationFailureCode,
    cause: Cause,
    failure_work: FailureEvidenceWorkBudget,
}

/// Verified post-activation rejection after eager canonical cleanup.
#[must_use = "rejected canonical state must build rich evidence or prepare minimal fallback"]
pub struct RejectedCanonicalStoreSession<O, PP, U, E>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    owner: UnrootedFailureEvidence<O, PP, U, StoreCanonicalRejection<E>>,
}

/// Failure-evidence construction result for a rejected canonical candidate.
pub type CanonicalRejectionEvidenceOutcome<O, PP, U, E, EvidenceKind, EvidenceTag> =
    FailureEvidenceOutcome<O, PP, U, StoreCanonicalRejection<E>, EvidenceKind, EvidenceTag>;

/// One-shot failure-evidence lane. It is consumed together with the owner and cannot be reopened.
pub struct FailureEvidenceLane<'lane, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: &'lane mut RawStoreFailureAssembly<O, PP, U>,
    domain: &'lane StoreKeyDomain,
    first_fault: &'lane mut Option<ActivationFirstFault>,
    failure_work: &'lane mut FailureEvidenceWorkBudget,
}

/// Rooted failure evidence. No mutation API is exposed after the one-shot lane completes.
#[must_use = "rooted failure evidence must be validated or prepared"]
pub struct RootedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: Cause,
    failure_work: FailureEvidenceWorkBudget,
}

/// Result of validating an already-rooted failure graph.
#[must_use = "the validated or rejected failure owner must be consumed"]
pub enum RootedFailureValidation<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(ValidatedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>),
    Rejected(RejectedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>),
    Fallback(VerifiedMinimalFallback<StoreRuntimeError>),
}

#[must_use = "validated failure evidence must be prepared or downgraded"]
pub struct ValidatedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    validated: ValidatedStoreFailure<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: Cause,
}

#[must_use = "prepared failure evidence must be handed to the terminal publisher"]
pub struct PreparedFailureEvidence<Cause, EvidenceKind, EvidenceTag> {
    prepared: PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag>,
    cause: Cause,
}

#[must_use = "rejected failure evidence must prepare the minimal fallback"]
pub struct RejectedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    failure: StoreFailureAssembly<O, PP, U, EvidenceKind, EvidenceTag>,
    cause: Cause,
}

/// Minimal fallback whose typed source cause remains available without exposing physical state.
#[must_use = "the verified minimal fallback must be handed to the terminal publisher"]
pub struct VerifiedMinimalFallback<Cause> {
    prepared: PreparedStoreFallback,
    cause: Cause,
}

// Both variants are linear terminal owners. Boxing the larger owner would bypass the store ledger.
#[allow(clippy::large_enum_variant)]
#[must_use = "the rooted evidence or minimal fallback owner must be consumed"]
pub enum FailureEvidenceOutcome<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Rooted(RootedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>),
    MinimalFallback(VerifiedMinimalFallback<Cause>),
}

/// Exact post-activation rejection cause coupled to its evidence-only continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreNeverActivatedCause<E> {
    Requested(E),
    Runtime(StoreRuntimeError),
}

enum NeverActivatedAuthority<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Operation(RetainedStoreOperation<O, PP, U, L>),
    FallbackOnly(RetainedStoreOperation<O, PP, U, L>),
}

/// Session state proving that no canonical lane was exposed.
#[must_use = "never-activated state must publish rich evidence or minimal fallback"]
pub struct NeverActivatedStoreSession<O, PP, U, E, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    authority: NeverActivatedAuthority<O, PP, U, L>,
    cause: StoreNeverActivatedCause<E>,
    failure_code: PreparationFailureCode,
    outcome_evidence_plan: RetainedStoreBlockTablePlan,
    failure_work: FailureEvidenceWorkBudget,
}

fn prepare_operation_minimal<O, PP, U, L, Cause>(
    mut operation: RetainedStoreOperation<O, PP, U, L>,
    failure_code: PreparationFailureCode,
    cause: Cause,
) -> Result<VerifiedMinimalFallback<Cause>, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation.raw.poll_terminal_control();
    let RetainedStoreOperation {
        raw,
        domain,
        lineage,
    } = operation;
    if drop_scoped_value(lineage).is_err() {
        let _ = drop_scoped_value(cause);
        return Err(StoreActivationContainment {
            cause: raw.contain_activation_fault(),
        });
    }
    let prepared = raw
        .prepare_minimal_fallback::<super::StoredBlock, super::StoredBlock>(failure_code)
        .map(|raw| classify_prepared_fallback(raw, domain));
    match prepared {
        Ok(prepared) => Ok(VerifiedMinimalFallback { prepared, cause }),
        Err(error) => {
            let _ = drop_scoped_value(cause);
            Err(StoreActivationContainment {
                cause: map_raw_prepare_error(error),
            })
        }
    }
}

fn prepare_assembly_minimal<O, PP, U, L>(
    mut assembly: RetainedStorePublicationAssembly<O, PP, U, L>,
    cause: StoreRuntimeError,
    failure_code: PreparationFailureCode,
) -> Result<VerifiedMinimalFallback<StoreRuntimeError>, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    assembly.latch_activation_terminal_cause(failure_code);
    let RetainedStorePublicationAssembly {
        raw,
        fallback_cause,
        success_rejection,
        domain,
        lineage,
    } = assembly;
    let cause = ManuallyDrop::new(cause);
    let lineage = ManuallyDrop::new(lineage);
    let failure_code = fallback_cause.or(success_rejection).unwrap_or(failure_code);
    if drop_scoped_value(ManuallyDrop::into_inner(lineage)).is_err() {
        let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
        return Err(StoreActivationContainment {
            cause: raw.contain_activation_fault(),
        });
    }
    match raw
        .prepare_minimal_fallback_with(failure_code)
        .map(|prepared| classify_prepared_fallback(prepared, domain))
    {
        Ok(prepared) => Ok(VerifiedMinimalFallback {
            prepared,
            cause: ManuallyDrop::into_inner(cause),
        }),
        Err(error) => {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            Err(StoreActivationContainment {
                cause: map_raw_prepare_error(error),
            })
        }
    }
}

const fn map_raw_prepare_error(
    error: RetainedPublicationPrepareError,
) -> FinalizationContainmentError {
    match error {
        RetainedPublicationPrepareError::FinalizationContainment(cause) => cause,
    }
}

fn prepare_operation_minimal_direct<O, PP, U, L>(
    mut operation: RetainedStoreOperation<O, PP, U, L>,
    failure_code: PreparationFailureCode,
) -> Result<PreparedStoreFallback, StorePrepareError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation.raw.poll_terminal_control();
    let RetainedStoreOperation {
        raw,
        domain,
        lineage,
    } = operation;
    if drop_scoped_value(lineage).is_err() {
        return Err(StorePrepareError::FinalizationContainment(
            raw.contain_activation_fault(),
        ));
    }
    raw.prepare_minimal_fallback::<super::StoredBlock, super::StoredBlock>(failure_code)
        .map(|raw| classify_prepared_fallback(raw, domain))
        .map_err(StorePrepareError::from_raw)
}

/// Scoped activation disposition. No raw assembly is present in any outcome.
#[must_use = "every branch retains its exact linear session owner"]
pub enum CanonicalActivationOutcome<O, PP, U, T, E, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Activated(ActivatedCanonicalStoreSession<O, PP, U, T, L>),
    Rejected(RejectedCanonicalStoreSession<O, PP, U, E>),
    NeverActivated(NeverActivatedStoreSession<O, PP, U, E, L>),
    Fallback(VerifiedMinimalFallback<StoreRuntimeError>),
}

/// Fail-closed activation result retaining the quarantined account snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreActivationContainment {
    cause: FinalizationContainmentError,
}
