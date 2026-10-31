mod config;
mod pair;
mod share;

#[cfg(test)]
pub(in crate::retained) use config::map_qualified_startup_batch_error;
pub(crate) use config::{
    RetainedBatchLimits, RetainedFailureTerminalWorkPlan, RetainedHandleSharePlan,
    RetainedOperationStartError, RetainedOperationStartPlan, RetainedWriterError,
    RetainedWriterPlan,
};
pub(in crate::retained) use config::{
    map_account_action_admission_error, map_batch_error, map_faulted_pending_rollback,
    map_pending_rollback_or,
};
pub(in crate::retained) use pair::{StartupRetainedPartGuard, StartupTerminalEvidenceBuildError};
#[cfg(test)]
pub(crate) use pair::{
    mismatch_next_startup_evidence_footprint, mismatch_next_startup_fallback_footprint,
    reset_startup_footprint_mismatches, reset_startup_payload_allocation_attempts,
    test_startup_payload_allocation_attempts,
};
pub(in crate::retained) use share::RetainedShareWorkPermit;
pub(in crate::retained) use share::{
    try_admit_retained_share_work, try_admit_retained_share_work_before_physical,
    try_admit_start_share_work, try_open_publication_account,
};
