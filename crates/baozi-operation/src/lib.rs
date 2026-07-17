#![forbid(unsafe_code)]

//! Runtime-neutral operation control and accounting mechanisms.

mod capacity;
mod control;
mod coordinator;
mod ledger;
mod owner;
mod retained;

pub use capacity::{CAPACITY_POLICY_REVISION, CapacityPlan, CapacityPlanError};
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
    MAX_RETAINED_INLINE_BYTES, PhysicalStorageAccount, PhysicalStorageAccountCreateError,
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryCreateError,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountFactorySnapshot,
    PhysicalStorageAccountLimits, PhysicalStorageAccountSnapshot, PhysicalStorageLimitError,
    RetainedAllocation, RetainedAllocationError, RetainedAllocationPlan,
    RetainedAllocationPlanError, RetainedBuildFailureCode,
};
