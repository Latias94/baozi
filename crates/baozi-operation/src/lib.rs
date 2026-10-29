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

pub use capacity::{
    CAPACITY_POLICY_REVISION, CapacityPlan, CapacityPlanError, GrowableCapacityPlan,
    ReplacementCapacityPlan,
};
pub use concrete_store::{
    ByteBlock, BytesBatchItem, BytesBatchPlan, BytesPlan, BytesWriter, CanonicalBytesRange,
    CanonicalBytesRef, CanonicalRecordRef, CanonicalRecordRowWriter, CanonicalRecordView,
    CanonicalRecordWriter, CanonicalSequenceRange, CanonicalSequenceRef, CanonicalStoreRange,
    CanonicalStoreRef, CanonicalUtf8Range, CanonicalUtf8Ref, OutcomeEvidenceBytesRef,
    OutcomeEvidenceRecordRef, OutcomeEvidenceRecordRowWriter, OutcomeEvidenceRecordView,
    OutcomeEvidenceRecordWriter, OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef,
    OutcomeEvidenceUtf8Ref, PublishedCanonicalStore, PublishedOutcomeEvidenceStore,
    PublishedOutcomeEvidenceStoreRef, PublishedRichFailureStore, PublishedStoreFailureParts,
    PublishedStoreFallbackOutput, PublishedStoreFallbackParts, PublishedStoreSuccessParts,
    RecordBlock, RecordPlan, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStoreFailureTerminalWorkPlan, RetainedStoreFailureTerminalWorkRequirement,
    ScalarSequence, SequenceBatchItem, SequenceBatchPlan, SequencePlan, SequenceView,
    SequenceWriter, ShareableStoreKind, StoreBatchRuntimeError, StoreBuildError,
    StoreBuildFailureClass, StoreControlledError, StoreControlledSuccessValidation,
    StoreDestination, StoreDimensionKey, StoreDimensionSemanticId, StoreDimensionSnapshot,
    StoreEvidenceValidationSession, StoreExecutionError, StoreFailureAssembly,
    StoreFailureContinuationExecution, StoreFailureValidation, StoreFallbackEnvelope,
    StoreFallbackOnly, StoreKeyDomain, StoreKeyDomainCreateError, StoreLedgerEvidence,
    StoreLedgerEvidenceHeader, StoreLedgerPlan, StoreLedgerPlanError, StorePhaseKey,
    StorePhaseSemanticId, StorePhaseSnapshot, StorePlanError, StorePreparationFailure,
    StorePreparationFailureClass, StorePrepareError, StoreRuntimeError, StoreSemanticCatalogId,
    StoreSuccessValidation, StoreTerminalCause, StoreTranscriptFailureValidation,
    StoreTranscriptIdentityExhausted, StoreTranscriptPlanError, StoreTranscriptStartCause,
    StoreTranscriptSuccessValidation, StoreValidationSession, StoreWorkError, StoredScalar,
    Utf8BatchItem, Utf8BatchPlan, Utf8Block, Utf8Plan, Utf8Writer, ValidatedStoreFailure,
    ValidatedStoreSuccess, ValidatedTranscriptStoreSuccess,
};

/// Internal retained-store integration surface for canonical activation.
///
/// This module is intentionally excluded from the supported crate-root API. It keeps the
/// consumer-facing activation capabilities separate from the trusted producer vocabulary used by
/// canonical construction modules.
#[doc(hidden)]
/// Raw output commitment scalars can no longer be submitted or replaced at the central-validation
/// transition; the host binds destination roles before sealing the keyless activation request:
///
/// ```compile_fail,E0432
/// use baozi_operation::__retained_store_integration::activation::CanonicalOutputIntent;
/// ```
///
/// Validation work cannot be completed independently of inspecting stored units:
///
/// ```compile_fail,E0432
/// use baozi_operation::__retained_store_integration::activation::CanonicalValidationWork;
/// ```
///
/// Scoped terminal receipts are invariant and cannot be widened into another transaction scope:
///
/// ```compile_fail
/// use baozi_operation::__retained_store_integration::activation::CanonicalScopedDisposition;
///
/// fn substitute<'short, 'long>(
///     receipt: CanonicalScopedDisposition<'short>,
/// ) -> CanonicalScopedDisposition<'long>
/// where
///     'long: 'short,
/// {
///     receipt
/// }
/// ```
///
/// The successful joint-commit binder moves owned output into the paired handoff callback.
/// Proof descriptors remain destructor-free; actual storage keeps its own central release owner:
///
/// ```no_run
/// use baozi_operation::__retained_store_integration::activation::CanonicalFreezePermit;
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
/// fn bind_owned_output(permit: Permit) {
///     permit.bind_joint_commit(|_, _, _, _| ());
/// }
/// ```
///
/// A proof cannot introduce an independently destructible owner:
///
/// ```compile_fail
/// use baozi_operation::__retained_store_integration::activation::CanonicalFreezePermit;
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
/// use baozi_operation::__retained_store_integration::activation::CanonicalFreezePermit;
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
/// use baozi_operation::__retained_store_integration::activation::CanonicalJointCommit;
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
/// use baozi_operation::__retained_store_integration::activation::CanonicalJointCommit;
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
/// use baozi_operation::__retained_store_integration::activation::CanonicalJointSeal;
///
/// fn cannot_copy(seal: CanonicalJointSeal<'_>) {
///     let moved = seal;
///     let _used_again = seal;
///     let _ = moved;
/// }
/// ```
///
/// ```compile_fail
/// use baozi_operation::__retained_store_integration::activation::CanonicalJointSeal;
///
/// fn cannot_clone(seal: CanonicalJointSeal<'_>) {
///     let _clone = seal.clone();
/// }
/// ```
pub mod __retained_store_integration {
    /// Internal owning-authority surface for retained-store hosts and construction drivers.
    ///
    /// These linear capabilities are intentionally absent from the supported crate-root API:
    ///
    /// ```compile_fail,E0432
    /// use baozi_operation::{
    ///     CanonicalInheritanceSource, PreparedStoreSuccess, RetainedStoreOperation,
    ///     RetainedStoreReservation,
    /// };
    /// ```
    pub mod authority {
        pub use crate::concrete_store::{
            CanonicalInheritanceSource, DerivedRetainedStoreOperation, FreshStoreLineage,
            PreparedStoreEvidenceOnlyOutcome, PreparedStoreEvidenceOnlySuccess,
            PreparedStoreFailure, PreparedStoreFailureOutcome, PreparedStoreFallback,
            PreparedStoreSuccess, PreparedStoreSuccessOutcome,
            PublishedStoreEvidenceOnlySuccessParts, RetainedStoreOperation,
            RetainedStorePublicationPlan, RetainedStoreReservation, RetainedStoreSharePlan,
            RetainedStoreStartError, RetainedStoreStartPlan, RetainedStoreWriterPlan,
        };
        pub use crate::retained::{RetainedStorage, SharedRetainedStorage};
    }

    /// Consumer-facing canonical activation capabilities.
    pub mod activation {
        pub use crate::concrete_store::activation::{
            ActivatedCanonicalStoreSession, CanonicalActivationOutcome, CanonicalActivationPermit,
            CanonicalActivationSeal, CanonicalActivationSealFailure,
            CanonicalActivationSessionPolicy, CanonicalBuildControl,
            CanonicalCandidateStoreSession, CanonicalCentralValidationOutcome,
            CanonicalFailureRequirements, CanonicalFailureTerminalWorkPlan,
            CanonicalFreezeInterruptionClass, CanonicalFreezePermit, CanonicalFreezePermitIssue,
            CanonicalJointCommit, CanonicalJointSeal, CanonicalLane, CanonicalMaterialization,
            CanonicalMaterializationRole, CanonicalRecordBuildRow, CanonicalReservation,
            CanonicalRetainedStorageBuildControl, CanonicalScopedDisposition,
            CanonicalSemanticCompletion, CanonicalSemanticWorkSession,
            CanonicalValidationCheckpoint, CanonicalValidationDecision, CanonicalValidationMember,
            CanonicalValidationPlanning, CanonicalValidationPreparationError,
            CanonicalValidationProgress, CanonicalValidationRole, CanonicalValidationSession,
            CentrallyRejectedCanonicalStoreSession, FailureEvidenceDecision, FailureEvidenceMember,
            FailureEvidenceOutcome, FailureEvidenceValidationSession, FrozenFailureState,
            InterruptedCanonicalStoreSession, NeverActivatedStoreSession, OutcomeEvidenceDecision,
            OutcomeEvidenceValidationMember, PreparedFailureEvidence,
            RejectedCanonicalStoreSession, RetainedStoreSessionOutcomeOwner,
            RootedFailureValidation, StoreActivationContainment, StoreCanonicalRejection,
            StoreNeverActivatedCause, VerifiedMinimalFallback,
        };
        pub use crate::concrete_store::{
            RetainedStoreDimensionState, RetainedStoreObservedWork, RetainedStoreWriterRole,
        };
    }

    /// Trusted producer surface for opaque canonical construction plans.
    ///
    /// Canonical construction implementations move one opaque description into the operation
    /// sealer. General operation consumers receive only already sealed domain plans and cannot
    /// name or rebind their success facts.
    pub mod producer {
        pub use crate::concrete_store::activation::{
            CanonicalActivationDescription, CanonicalActivationPlanSealer,
            CanonicalActivationRequest, CanonicalActivationRequestIdentity,
            CanonicalSealedActivationPlan, CanonicalSuccessRequirements, CanonicalTableShape,
            mint_canonical_activation_request,
        };
    }
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
