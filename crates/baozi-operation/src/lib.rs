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

pub use capacity::{CAPACITY_POLICY_REVISION, CapacityPlan, CapacityPlanError};
pub use concrete_store::{
    ByteBlock, BytesPlan, BytesWriter, CanonicalBytesRef, CanonicalRecordRef,
    CanonicalRecordRowWriter, CanonicalRecordView, CanonicalRecordWriter, CanonicalSequenceRef,
    CanonicalStoreRef, CanonicalUtf8Ref, OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef,
    OutcomeEvidenceRecordRowWriter, OutcomeEvidenceRecordView, OutcomeEvidenceRecordWriter,
    OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef, OutcomeEvidenceUtf8Ref,
    PreparedStoreFailure, PreparedStoreFailureOutcome, PreparedStoreFallback, PreparedStoreSuccess,
    PreparedStoreSuccessOutcome, PublishedCanonicalStore, PublishedOutcomeEvidenceStore,
    PublishedOutcomeEvidenceStoreRef, PublishedRichFailureStore, PublishedStoreFailureParts,
    PublishedStoreFallbackOutput, PublishedStoreFallbackParts, PublishedStoreSuccessParts,
    RecordBlock, RecordPlan, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStoreOpenError, RetainedStoreOperation, RetainedStorePublicationAssembly,
    RetainedStorePublicationPlan, RetainedStoreSharePlan, RetainedStoreStartError,
    RetainedStoreStartPlan, RetainedStoreWriterPlan, ScalarSequence, SequencePlan, SequenceView,
    SequenceWriter, ShareableStoreKind, StoreBuildError, StoreBuildFailureClass, StoreDestination,
    StoreDimensionKey, StoreDimensionSnapshot, StoreEvidenceValidationSession,
    StoreFailureAssembly, StoreFailureValidation, StoreFallbackEnvelope, StoreFallbackOnly,
    StoreKeyDomain, StoreKeyDomainCreateError, StoreLedgerEvidence, StoreLedgerPlan,
    StoreLedgerPlanError, StorePhaseKey, StorePlanError, StorePreparationFailure,
    StorePreparationFailureClass, StorePrepareError, StoreRuntimeError, StoreSuccessValidation,
    StoreValidationSession, StoredScalar, Utf8Block, Utf8Plan, Utf8Writer, ValidatedStoreFailure,
    ValidatedStoreSuccess,
};
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
