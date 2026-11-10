//! Consuming disposition requests retain the actual state in the armed outer host.

use super::CanonicalDispositionSlot;
use super::driver::{
    OperationDisposition, PendingDisposition, PendingNeverActivated, PendingOperation,
    PendingRejected, PendingRootedFailure,
};
use crate::concrete_store::activation::*;
use crate::concrete_store::deferred::{DeferredUnrootedStoreFailure, defer_operation};

impl<'inv, O, PP, U, L> RetainedStoreOperation<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Requests the evidence-only terminal after all body-owned values have been disposed.
    pub fn request_unchanged<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreActivationContainment> {
        self.request_operation(disposition, OperationDisposition::Unchanged)
    }

    /// Latches this requested cause now and defers its minimal fallback until body cleanup.
    pub fn request_minimal_fallback<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
        code: u32,
    ) -> Result<(), StoreActivationContainment> {
        self.request_operation(disposition, OperationDisposition::MinimalFallback { code })
    }

    fn request_operation<CK, CT, EK, ET>(
        mut self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
        request: OperationDisposition,
    ) -> Result<(), StoreActivationContainment> {
        if !disposition.is_vacant_for(self.raw.loan().invocation_identity()) {
            return Err(contain_operation(self));
        }
        if let OperationDisposition::MinimalFallback { code } = request {
            self.latch_activation_terminal_cause(PreparationFailureCode::domain(code));
        }
        let store = defer_operation(self).map_err(|raw| StoreActivationContainment {
            cause: raw.contain_activation_fault(),
        })?;
        disposition.install(PendingDisposition::Operation(PendingOperation {
            store,
            request,
        }));
        Ok(())
    }
}

impl<'inv, O, PP, U, E, L> NeverActivatedStoreSession<'inv, O, PP, U, E, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Retains the real dormant authority and typed cause without preparing failure evidence.
    pub fn request_finish<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreActivationContainment> {
        let Self {
            owner,
            failure_code,
            outcome_evidence_plan,
            failure_work,
            outcome_evidence_destination_dimension,
        } = self;
        let (cause, authority) = owner.into_parts();
        let (operation, fallback_only) = match authority {
            NeverActivatedAuthority::Operation(operation) => (operation, false),
            NeverActivatedAuthority::FallbackOnly(operation) => (operation, true),
        };
        if !disposition.is_vacant_for(cause.identity())
            || !disposition.is_vacant_for(operation.raw.loan().invocation_identity())
        {
            let _ = drop_scoped_value(cause);
            return Err(contain_operation(operation));
        }
        let store = match defer_operation(operation) {
            Ok(store) => store,
            Err(raw) => {
                let _ = drop_scoped_value(cause);
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
        };
        disposition.install(PendingDisposition::NeverActivated(PendingNeverActivated {
            store,
            fallback_only,
            failure_code,
            outcome_evidence_plan,
            failure_work,
            destination: outcome_evidence_destination_dimension,
        }));
        cause.retain_in_host();
        Ok(())
    }
}

impl<'inv, O, PP, U, E> RejectedCanonicalStoreSession<'inv, O, PP, U, E>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Retains the actual eager-cleaned rejection assembly and its original typed cause.
    pub fn request_finish<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreActivationContainment> {
        let UnrootedFailureEvidence {
            owner,
            domain,
            state,
            failure_code,
            failure_work,
            outcome_evidence_destination_dimension,
        } = self.owner;
        let (cause, raw) = owner.into_parts();
        if !disposition.is_vacant_for(cause.identity()) {
            let _ = drop_scoped_value(cause);
            return Err(StoreActivationContainment {
                cause: raw.contain_activation_fault(),
            });
        }
        if let Err(raw) = raw.defer_until_body_cleanup() {
            let _ = drop_scoped_value(cause);
            return Err(StoreActivationContainment {
                cause: raw.contain_activation_fault(),
            });
        }
        disposition.install(PendingDisposition::Rejected(PendingRejected {
            store: DeferredUnrootedStoreFailure {
                domain: domain.defer(),
                state,
            },
            failure_code,
            failure_work,
            destination: outcome_evidence_destination_dimension,
        }));
        cause.retain_in_host();
        Ok(())
    }
}

impl<'inv, O, PP, U, E, EK, ET> CentrallyRejectedCanonicalStoreSession<'inv, O, PP, U, E, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Defers the existing rooted failure without validating or freezing it inside the body.
    pub fn request_finish<CK, CT>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreActivationContainment> {
        let RootedFailureEvidence {
            owner,
            failure_work,
            outcome_evidence_destination_dimension,
            _lifecycle: _,
        } = self.root;
        let (cause, store) = owner.into_parts();
        if !disposition.is_vacant_for(cause.identity()) {
            let _ = drop_scoped_value(cause);
            return Err(StoreActivationContainment {
                cause: store.contain_activation_fault(),
            });
        }
        let store = match store.defer_until_body_cleanup() {
            Ok(store) => store,
            Err(store) => {
                let _ = drop_scoped_value(cause);
                return Err(StoreActivationContainment {
                    cause: store.contain_activation_fault(),
                });
            }
        };
        disposition.install(PendingDisposition::CentralRejected(PendingRootedFailure {
            store,
            failure_work,
            destination: outcome_evidence_destination_dimension,
        }));
        cause.retain_in_host();
        Ok(())
    }
}

impl<'inv, O, PP, U, EK, ET> InterruptedCanonicalStoreSession<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Retains the exact interrupted owner and its control/ledger cause for outer finish.
    pub fn request_finish<CK, CT>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreActivationContainment> {
        let RootedFailureEvidence {
            owner,
            failure_work,
            outcome_evidence_destination_dimension,
            _lifecycle: _,
        } = self.root;
        let (cause, store) = owner.into_parts();
        if !disposition.is_vacant_for(cause.identity()) {
            let _ = drop_scoped_value(cause);
            return Err(StoreActivationContainment {
                cause: store.contain_activation_fault(),
            });
        }
        let store = match store.defer_until_body_cleanup() {
            Ok(store) => store,
            Err(store) => {
                let _ = drop_scoped_value(cause);
                return Err(StoreActivationContainment {
                    cause: store.contain_activation_fault(),
                });
            }
        };
        disposition.install(PendingDisposition::Interrupted(PendingRootedFailure {
            store,
            failure_work,
            destination: outcome_evidence_destination_dimension,
        }));
        cause.retain_in_host();
        Ok(())
    }
}

impl<'inv, O, PP, U> PendingMinimalFallback<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Retains the still-live central fallback and its exact runtime cause until body cleanup.
    pub fn request_finish<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreActivationContainment> {
        let (cause, store) = self.owner.into_parts();
        if !disposition.is_vacant_for(cause.identity()) {
            let _ = drop_scoped_value(cause);
            return Err(StoreActivationContainment {
                cause: store.contain_activation_fault(),
            });
        }
        let store = match store.defer_until_body_cleanup() {
            Ok(store) => store,
            Err(store) => {
                let _ = drop_scoped_value(cause);
                return Err(StoreActivationContainment {
                    cause: store.contain_activation_fault(),
                });
            }
        };
        disposition.install(PendingDisposition::Fallback(store));
        cause.retain_in_host();
        Ok(())
    }
}

impl<'inv, O, PP, U, T, Proof, E, CK, CT, EK, ET>
    CanonicalCentralValidationOutcome<'inv, O, PP, U, T, Proof, E, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Records the actual central result without preparing any terminal inside the body.
    pub fn request_finish(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreActivationContainment> {
        match self {
            Self::Candidate(candidate) => candidate.request_finish(disposition),
            Self::Rejected(rejected) => rejected.request_finish(disposition),
            Self::Interrupted(interrupted) => interrupted.request_finish(disposition),
            Self::Fallback(fallback) => fallback.request_finish(disposition),
        }
    }
}
