#![forbid(unsafe_code)]

//! Runtime-neutral operation control and accounting mechanisms.

mod capacity;
mod concrete_store;
mod control;
mod coordinator;
mod ledger;
mod owner;
#[allow(
    dead_code,
    reason = "the private retained-publication foundation precedes the atomic U4 consumer switch"
)]
mod retained;
#[cfg(feature = "_u4c-collector")]
mod u4c_collector;

/// Temporary repository measurement surface for the U4 canonical-store cutover.
///
/// This module is not a supported runtime API and is removed at U4f. Calling it cannot certify a
/// scenario: the repository runner must select the fixed core probe and its frozen source closure
/// before it may accept any report as evidence.
#[cfg(feature = "_u4c-collector")]
#[doc(hidden)]
pub mod __u4c {
    pub use crate::u4c_collector::{MeasurementError, MeasurementReport, MeasurementSession};
}

pub use capacity::{
    CAPACITY_POLICY_REVISION, CapacityPlan, CapacityPlanError, GrowableCapacityPlan,
    ReplacementCapacityPlan,
};
pub use concrete_store::{
    ByteBlock, BytesBatchItem, BytesBatchPlan, BytesPlan, BytesWriter, CanonicalBytesRange,
    CanonicalBytesRef, CanonicalInheritanceSource, CanonicalRecordRef, CanonicalRecordRowWriter,
    CanonicalRecordView, CanonicalRecordWriter, CanonicalSequenceRange, CanonicalSequenceRef,
    CanonicalStoreRange, CanonicalStoreRef, CanonicalUtf8Range, CanonicalUtf8Ref,
    DerivedRetainedStoreOperation, DerivedRetainedStorePublicationAssembly, FreshStoreLineage,
    OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef, OutcomeEvidenceRecordRowWriter,
    OutcomeEvidenceRecordView, OutcomeEvidenceRecordWriter, OutcomeEvidenceSequenceRef,
    OutcomeEvidenceStoreRef, OutcomeEvidenceUtf8Ref, PlannedCanonicalBytes,
    PlannedCanonicalBytesBatch, PlannedCanonicalCommit, PlannedCanonicalRecords,
    PlannedCanonicalSequence, PlannedCanonicalSequenceBatch, PlannedCanonicalShare,
    PlannedCanonicalUtf8, PlannedCanonicalUtf8Batch, PlannedOutcomeEvidenceBytes,
    PlannedOutcomeEvidenceCommit, PlannedOutcomeEvidenceRecords, PlannedOutcomeEvidenceSequence,
    PlannedOutcomeEvidenceShare, PlannedOutcomeEvidenceUtf8, PreparedStoreFailure,
    PreparedStoreFailureOutcome, PreparedStoreFallback, PreparedStoreSuccess,
    PreparedStoreSuccessOutcome, PublishedCanonicalStore, PublishedOutcomeEvidenceStore,
    PublishedOutcomeEvidenceStoreRef, PublishedRichFailureStore, PublishedStoreFailureParts,
    PublishedStoreFallbackOutput, PublishedStoreFallbackParts, PublishedStoreSuccessParts,
    RecordBlock, RecordPlan, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStoreControlCheckpoint, RetainedStoreExecution, RetainedStoreFailureTerminalWorkPlan,
    RetainedStoreFailureTerminalWorkRequirement, RetainedStoreOpenError, RetainedStoreOpenRecovery,
    RetainedStoreOperation, RetainedStorePublicationAssembly, RetainedStorePublicationPlan,
    RetainedStoreReservation, RetainedStoreSharePlan, RetainedStoreStartError,
    RetainedStoreStartPlan, RetainedStoreTranscript, RetainedStoreWorkControl,
    RetainedStoreWriterPlan, ScalarSequence, SequenceBatchItem, SequenceBatchPlan, SequencePlan,
    SequenceView, SequenceWriter, ShareableStoreKind, StoreBatchRuntimeError, StoreBuildError,
    StoreBuildFailureClass, StoreControlledError, StoreControlledSuccessValidation,
    StoreDestination, StoreDimensionKey, StoreDimensionSemanticId, StoreDimensionSnapshot,
    StoreEvidenceValidationSession, StoreExecutionError, StoreFailureAssembly,
    StoreFailureContinuationExecution, StoreFailureValidation, StoreFallbackEnvelope,
    StoreFallbackOnly, StoreKeyDomain, StoreKeyDomainCreateError, StoreLedgerEvidence,
    StoreLedgerEvidenceHeader, StoreLedgerPlan, StoreLedgerPlanError, StorePhaseKey,
    StorePhaseSemanticId, StorePhaseSnapshot, StorePlanError, StorePreparationFailure,
    StorePreparationFailureClass, StorePrepareError, StoreRuntimeError, StoreSemanticCatalogId,
    StoreSuccessValidation, StoreTerminalCause, StoreTranscriptFailureValidation,
    StoreTranscriptIdentityExhausted, StoreTranscriptOpenError, StoreTranscriptOpenRecovery,
    StoreTranscriptOpenResult, StoreTranscriptPlanError, StoreTranscriptStartCause,
    StoreTranscriptStartError, StoreTranscriptStartResult, StoreTranscriptStateError,
    StoreTranscriptSuccessValidation, StoreValidationSession, StoreWorkError, StoredScalar,
    Utf8BatchItem, Utf8BatchPlan, Utf8Block, Utf8Plan, Utf8Writer, ValidatedStoreFailure,
    ValidatedStoreSuccess, ValidatedTranscriptStoreSuccess,
};

/// Temporary internal seam for the U4 single-owner canonical activation cutover.
///
/// This module is intentionally excluded from the supported crate-root API. It is removed or
/// privatized once core, import, formats, facade, snapshot, and fuzz use the same production path.
#[cfg(feature = "_u4-activation")]
#[doc(hidden)]
/// Temporary canonical activation vocabulary.
///
/// Raw output commitment scalars can no longer be submitted or replaced at the central-validation
/// transition; the host binds destination roles before sealing the keyless activation request:
///
/// ```compile_fail,E0432
/// use baozi_operation::__canonical_activation::CanonicalOutputIntent;
/// ```
///
/// Validation work cannot be completed independently of inspecting stored units:
///
/// ```compile_fail,E0432
/// use baozi_operation::__canonical_activation::CanonicalValidationWork;
/// ```
///
/// The successful joint-commit binder accepts only destructor-free descriptors:
///
/// ```compile_fail
/// use baozi_operation::__canonical_activation::CanonicalFreezePermit;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
///
/// struct Owner;
/// impl OperationOwner for Owner {}
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Phase { Final }
/// impl ProgressPhase for Phase {
///     fn order(self) -> u32 { 0 }
///     fn stable_id(self) -> &'static str { "final" }
/// }
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Unit { Item }
/// impl ProgressUnit for Unit {
///     fn stable_id(self) -> &'static str { "item" }
/// }
///
/// type Permit = CanonicalFreezePermit<Owner, Phase, Unit, String, (), (), (), (), ()>;
/// fn bind_non_copy_output(permit: Permit) {
///     permit.bind_joint_commit(|_, _, _, _| ());
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::__canonical_activation::CanonicalFreezePermit;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
///
/// struct Owner;
/// impl OperationOwner for Owner {}
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Phase { Final }
/// impl ProgressPhase for Phase {
///     fn order(self) -> u32 { 0 }
///     fn stable_id(self) -> &'static str { "final" }
/// }
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Unit { Item }
/// impl ProgressUnit for Unit {
///     fn stable_id(self) -> &'static str { "item" }
/// }
///
/// type Permit = CanonicalFreezePermit<Owner, Phase, Unit, (), String, (), (), (), ()>;
/// fn bind_non_copy_proof(permit: Permit) {
///     permit.bind_joint_commit(|_, _, _, _| ());
/// }
/// ```
///
/// Independent binder scopes cannot exchange their invariant seals:
///
/// ```compile_fail
/// use baozi_operation::__canonical_activation::CanonicalFreezePermit;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
///
/// struct Owner;
/// impl OperationOwner for Owner {}
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Phase { Final }
/// impl ProgressPhase for Phase {
///     fn order(self) -> u32 { 0 }
///     fn stable_id(self) -> &'static str { "final" }
/// }
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Unit { Item }
/// impl ProgressUnit for Unit {
///     fn stable_id(self) -> &'static str { "item" }
/// }
///
/// type Permit = CanonicalFreezePermit<Owner, Phase, Unit, (), (), (), (), (), ()>;
/// fn cross_scopes(first: Permit, second: Permit) {
///     first.bind_joint_commit(|_, _, first_joint, _| {
///         second.bind_joint_commit(|_, _, _, second_seal| {
///             let _ = first_joint.prepare(second_seal);
///         });
///     });
/// }
/// ```
///
/// Scoped joint capabilities are linear: a joint contribution cannot be copied or cloned.
///
/// ```compile_fail
/// use baozi_operation::__canonical_activation::CanonicalJointCommit;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
///
/// struct Owner;
/// impl OperationOwner for Owner {}
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Phase { Final }
/// impl ProgressPhase for Phase {
///     fn order(self) -> u32 { 0 }
///     fn stable_id(self) -> &'static str { "final" }
/// }
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Unit { Item }
/// impl ProgressUnit for Unit {
///     fn stable_id(self) -> &'static str { "item" }
/// }
///
/// type Joint<'a> = CanonicalJointCommit<'a, Owner, Phase, Unit, (), (), (), ()>;
/// fn cannot_copy(joint: Joint<'_>) {
///     let moved = joint;
///     let _used_again = joint;
///     let _ = moved;
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::__canonical_activation::CanonicalJointCommit;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
///
/// struct Owner;
/// impl OperationOwner for Owner {}
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Phase { Final }
/// impl ProgressPhase for Phase {
///     fn order(self) -> u32 { 0 }
///     fn stable_id(self) -> &'static str { "final" }
/// }
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// enum Unit { Item }
/// impl ProgressUnit for Unit {
///     fn stable_id(self) -> &'static str { "item" }
/// }
///
/// type Joint<'a> = CanonicalJointCommit<'a, Owner, Phase, Unit, (), (), (), ()>;
/// fn cannot_clone(joint: Joint<'_>) {
///     let _clone = joint.clone();
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::__canonical_activation::CanonicalJointSeal;
///
/// fn cannot_copy(seal: CanonicalJointSeal<'_>) {
///     let moved = seal;
///     let _used_again = seal;
///     let _ = moved;
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::__canonical_activation::CanonicalJointSeal;
///
/// fn cannot_clone(seal: CanonicalJointSeal<'_>) {
///     let _clone = seal.clone();
/// }
/// ```
pub mod __canonical_activation {
    pub use crate::concrete_store::activation::{
        CanonicalActivationOutcome, CanonicalActivationPolicy, CanonicalActivationRequestSealer,
        CanonicalActivationRequirements, CanonicalBuildControl, CanonicalCandidateStoreSession,
        CanonicalCentralValidationOutcome, CanonicalFailureTerminalWorkPlan,
        CanonicalFreezeInterruption, CanonicalFreezeInterruptionClass, CanonicalFreezePermit,
        CanonicalFreezePermitIssue, CanonicalJointCommit, CanonicalJointSeal,
        CanonicalMaterialization, CanonicalPreflightDecision, CanonicalRecordBuildRow,
        CanonicalTableShape, CanonicalValidationDecision, CanonicalValidationMember,
        CanonicalValidationRecordRow, CanonicalValidationSession, FailureEvidenceDecision,
        FailureEvidenceMember, FailureEvidenceOutcome, FailureEvidenceRecordRow,
        FailureEvidenceValidationSession, NeverActivatedStoreSession, OutcomeEvidenceBuildControl,
        OutcomeEvidenceDecision, OutcomeEvidenceValidationMember,
        OutcomeEvidenceValidationRecordRow, RetainedStoreSessionOutcomeOwner,
        RootedFailureValidation, SealedCanonicalActivationRequest, StoreActivationContainment,
        StoreCanonicalRejection, StoreNeverActivatedCause,
    };
}
pub use control::{
    CancellationOrigin, CancellationSignal, CancellationToken, ControlCause, OperationControl,
    OperationPoll, ProgressDecision, ProgressDeliveryLimits, ProgressEvent, ProgressObserver,
    ProgressPhase, ProgressStorageError, ProgressUnit, ProgressViolation,
};
pub use coordinator::{FinalProgress, OperationCheckpoint, OperationCoordinator, PublicationCause};
pub use ledger::{
    Commitment, CounterClass, DEFAULT_LEDGER_EVIDENCE_BYTE_LIMIT, DimensionPolicy,
    DimensionSnapshot, LedgerCause, LedgerEvidenceError, LedgerEvidencePurpose, LedgerFinishError,
    LedgerLimits, LedgerLimitsError, LedgerSnapshot, LiveReservation, PendingPublication,
    PhaseSnapshot, PublicationReservationBatch, PublicationReservationBatchError,
    PublicationReservationBatchErrorKind,
};
pub use owner::OperationOwner;
pub use retained::{
    AccountPoisonEvidence, AccountPoisonReason, AccountPoisoned, FinalizationContainmentError,
    PhysicalStorageAccount, PhysicalStorageAccountCreateError, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryCreateError, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountFactorySnapshot, PhysicalStorageAccountLimits,
    PhysicalStorageAccountSnapshot, PhysicalStorageLimitError,
};
