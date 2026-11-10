//! Finite deferral retains original root fields and shares the concrete domain stamps.

use super::*;
use crate::concrete_store::deferred::{
    DeferredStoreFailure, DeferredStoreFallback, DeferredStoreFallbackState,
    DeferredStoreOperation, DeferredUnrootedStoreFailure,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

type RestoredOperationResult<'host, O, PP, U> = Result<
    FiniteRetainedStoreOperation<'host, O, PP, U, TerminalStoreLineage>,
    FiniteStoreExitError<O, PP, U>,
>;
type RestoredSuccessResult<'host, O, PP, U, CK, CT, EK, ET> = Result<
    ValidatedFiniteStoreSuccess<'host, O, PP, U, CK, CT, EK, ET>,
    FiniteStoreExitError<O, PP, U>,
>;
type RestoredFailureResult<'host, O, PP, U, EK, ET> =
    Result<FiniteStoreFailureAssembly<'host, O, PP, U, EK, ET>, FiniteStoreExitError<O, PP, U>>;

impl<O, PP, U, L> FiniteRetainedStoreOperation<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn defer_until_body_cleanup(
        mut self,
    ) -> Result<DeferredStoreOperation, Self> {
        if self.fields.domain.is_none()
            || self.fields.startup.is_some()
            || !self.fields.raw.defer_operation_until_body_cleanup()
        {
            return Err(self);
        }
        let Self {
            mut fields,
            lineage,
        } = self;
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(lineage))) {
            fields.contain_publication_fault();
            std::panic::resume_unwind(payload);
        }
        Ok(DeferredStoreOperation {
            domain: StoreDomainLoan {
                slot: fields.domain,
            }
            .defer(),
        })
    }
}

impl<O, PP, U> FiniteUnrootedStoreFailure<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn defer_until_body_cleanup(
        mut self,
    ) -> Result<DeferredUnrootedStoreFailure, Self> {
        if self.fields.domain.is_none()
            || self.fields.startup.is_some()
            || !self
                .fields
                .raw
                .defer_failure_until_body_cleanup(self.failure_cause)
        {
            return Err(self);
        }
        Ok(DeferredUnrootedStoreFailure {
            domain: StoreDomainLoan {
                slot: self.fields.domain,
            }
            .defer(),
            state: self.state,
        })
    }
}

impl<O, PP, U, CK, CT, EK, ET> ValidatedFiniteStoreSuccess<'_, O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn defer_until_body_cleanup(
        self,
    ) -> Result<DeferredStoreSuccess<CK, CT, EK, ET>, Self> {
        if self.fields.domain.is_none() || self.fields.startup.is_some() {
            return Err(self);
        }
        let Self {
            mut fields,
            proof,
            canonical_root,
            evidence_root,
        } = self;
        match fields.raw.defer_success_until_body_cleanup(proof) {
            Ok(()) => Ok(DeferredStoreSuccess {
                canonical_root,
                evidence_root,
                domain: StoreDomainLoan {
                    slot: fields.domain,
                }
                .defer(),
            }),
            Err(proof) => Err(Self::from_bound_fields(
                fields,
                proof,
                canonical_root,
                evidence_root,
            )),
        }
    }
}

impl<O, PP, U, EK, ET> FiniteStoreFailureAssembly<'_, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn defer_until_body_cleanup(
        mut self,
    ) -> Result<DeferredStoreFailure<EK, ET>, Self> {
        if self.error.is_some()
            || self.fields.domain.is_none()
            || self.fields.startup.is_some()
            || !self
                .fields
                .raw
                .defer_failure_until_body_cleanup(self.failure_cause)
        {
            return Err(self);
        }
        Ok(DeferredStoreFailure {
            state: self.state,
            forced_cause: self.forced_cause,
            evidence_root: self.evidence_root,
            domain: StoreDomainLoan {
                slot: self.fields.domain,
            }
            .defer(),
        })
    }
}

impl<O, PP, U> FiniteStoreFallbackOnly<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn defer_until_body_cleanup(
        mut self,
    ) -> Result<DeferredStoreFallback, Self> {
        if self.error.is_some() || self.fields.domain.is_none() || self.fields.startup.is_some() {
            return Err(self);
        }
        let state = match &self.provenance {
            ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch => {
                if !self.fields.raw.defer_assembly_until_body_cleanup() {
                    return Err(self);
                }
                DeferredStoreFallbackState::Assembly { cause: self.cause }
            }
            ordinary::StoreSuccessFallbackProvenance::FailureUnverified => {
                if !self.fields.raw.defer_failure_until_body_cleanup(self.cause) {
                    return Err(self);
                }
                DeferredStoreFallbackState::FailureAssembly
            }
            ordinary::StoreSuccessFallbackProvenance::RawValidated {
                activation_containment_required,
            } => {
                if !self
                    .fields
                    .raw
                    .defer_fallback_until_body_cleanup(self.cause, *activation_containment_required)
                {
                    return Err(self);
                }
                DeferredStoreFallbackState::Validated
            }
        };
        Ok(DeferredStoreFallback {
            state,
            domain: StoreDomainLoan {
                slot: self.fields.domain,
            }
            .defer(),
        })
    }
}

impl<O, PP, U> FiniteStoreInvocationRoot<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn resume_deferred_operation<'host>(
        &'host mut self,
        pending: DeferredStoreOperation,
    ) -> RestoredOperationResult<'host, O, PP, U> {
        let mut fields = self.fields();
        if StoreDomainLoan::resume(&mut *fields.domain, pending.domain).is_none()
            || fields.startup.is_some()
            || !fields.raw.resume_deferred_operation()
        {
            return Err(fields.into_request_error(None));
        }
        Ok(FiniteRetainedStoreOperation::from_activation_parts(
            fields,
            TerminalStoreLineage::new(),
        ))
    }

    pub(in crate::concrete_store) fn resume_deferred_unrooted_failure<'host>(
        &'host mut self,
        pending: DeferredUnrootedStoreFailure,
    ) -> Result<FiniteUnrootedStoreFailure<'host, O, PP, U>, FiniteStoreExitError<O, PP, U>> {
        let mut fields = self.fields();
        if StoreDomainLoan::resume(&mut *fields.domain, pending.domain).is_none()
            || fields.startup.is_some()
        {
            return Err(fields.into_request_error(None));
        }
        let Some(failure_cause) = fields.raw.resume_deferred_failure() else {
            return Err(fields.into_request_error(None));
        };
        Ok(FiniteUnrootedStoreFailure {
            fields,
            state: pending.state,
            failure_cause,
        })
    }

    pub(in crate::concrete_store) fn resume_deferred_success<'host, CK, CT, EK, ET>(
        &'host mut self,
        pending: DeferredStoreSuccess<CK, CT, EK, ET>,
    ) -> RestoredSuccessResult<'host, O, PP, U, CK, CT, EK, ET> {
        let mut fields = self.fields();
        if StoreDomainLoan::resume(&mut *fields.domain, pending.domain).is_none()
            || fields.startup.is_some()
        {
            return Err(fields.into_request_error(None));
        }
        let Some(proof) = fields.raw.resume_deferred_success() else {
            return Err(fields.into_request_error(None));
        };
        Ok(ValidatedFiniteStoreSuccess::from_bound_fields(
            fields,
            proof,
            pending.canonical_root,
            pending.evidence_root,
        ))
    }

    pub(in crate::concrete_store) fn resume_deferred_failure<'host, EK, ET>(
        &'host mut self,
        pending: DeferredStoreFailure<EK, ET>,
    ) -> RestoredFailureResult<'host, O, PP, U, EK, ET> {
        let mut fields = self.fields();
        if StoreDomainLoan::resume(&mut *fields.domain, pending.domain).is_none()
            || fields.startup.is_some()
        {
            return Err(fields.into_request_error(None));
        }
        let Some(failure_cause) = fields.raw.resume_deferred_failure() else {
            return Err(fields.into_request_error(None));
        };
        Ok(FiniteStoreFailureAssembly::from_bound_fields(
            fields,
            pending.state,
            failure_cause,
            pending.forced_cause,
            pending.evidence_root,
            None,
        ))
    }

    pub(in crate::concrete_store) fn resume_deferred_fallback<'host>(
        &'host mut self,
        pending: DeferredStoreFallback,
    ) -> Result<FiniteStoreFallbackOnly<'host, O, PP, U>, FiniteStoreExitError<O, PP, U>> {
        let mut fields = self.fields();
        if StoreDomainLoan::resume(&mut *fields.domain, pending.domain).is_none()
            || fields.startup.is_some()
        {
            return Err(fields.into_request_error(None));
        }
        let (cause, provenance) = match pending.state {
            DeferredStoreFallbackState::Assembly { cause } => {
                if !fields.raw.resume_deferred_assembly() {
                    return Err(fields.into_request_error(None));
                }
                (
                    cause,
                    ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                )
            }
            DeferredStoreFallbackState::FailureAssembly => {
                let Some(cause) = fields.raw.resume_deferred_failure() else {
                    return Err(fields.into_request_error(None));
                };
                (
                    cause,
                    ordinary::StoreSuccessFallbackProvenance::FailureUnverified,
                )
            }
            DeferredStoreFallbackState::Validated => {
                let Some((cause, activation_containment_required)) =
                    fields.raw.resume_deferred_fallback()
                else {
                    return Err(fields.into_request_error(None));
                };
                (
                    cause,
                    ordinary::StoreSuccessFallbackProvenance::RawValidated {
                        activation_containment_required,
                    },
                )
            }
        };
        Ok(FiniteStoreFallbackOnly::from_bound_fields(
            fields, cause, provenance, None,
        ))
    }
}
