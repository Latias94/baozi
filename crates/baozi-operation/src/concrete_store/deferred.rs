//! Reference-free concrete facts retained beside the raw invocation's deferred stage.

use super::*;

/// Lineage of a terminal-only operation after its ordinary body and source loans have ended.
/// The exact fresh/derived source identity remains in the same raw invocation.
pub struct TerminalStoreLineage {
    _private: (),
}

impl TerminalStoreLineage {
    pub(super) const fn new() -> Self {
        Self { _private: () }
    }
}

pub(super) struct DeferredStoreOperation {
    pub(super) domain: StoreDomainStamp,
}

pub(super) struct DeferredUnattachedStore<'account> {
    binding: crate::retained::DeferredUnattachedBinding<'account>,
    domain: StoreDomainStamp,
}

impl<'inv, 'account, O, PP, U, L> BoundStoreOperation<'inv, 'account, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn invocation_identity(&self) -> *const () {
        self.raw.invocation_identity()
    }

    pub(super) fn defer_unattached(
        self,
        requested: Option<crate::retained::PreparationFailureCode>,
    ) -> Result<DeferredUnattachedStore<'account>, StoreRuntimeError> {
        let Self {
            raw,
            domain,
            fallback,
            lineage,
        } = self;
        drop((fallback, lineage));
        let binding = raw
            .defer_unattached(requested)
            .map_err(|_| StoreRuntimeError::OwnershipUnverified)?;
        Ok(DeferredUnattachedStore {
            binding,
            domain: domain.defer(),
        })
    }
}

pub(super) fn defer_operation<'inv, O, PP, U, L>(
    operation: RetainedStoreOperation<'inv, O, PP, U, L>,
) -> Result<DeferredStoreOperation, RawStoreOperation<'inv, O, PP, U>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let RetainedStoreOperation {
        raw,
        domain,
        lineage,
    } = operation;
    if activation::drop_scoped_value(lineage).is_err() {
        return Err(raw);
    }
    raw.defer_until_body_cleanup()
        .map(|()| DeferredStoreOperation {
            domain: domain.defer(),
        })
}

pub(super) struct DeferredUnrootedStoreFailure {
    pub(super) domain: StoreDomainStamp,
    pub(super) state: StorePublicationState,
}

pub(super) struct DeferredStoreFailure<EvidenceKind, EvidenceTag> {
    pub(super) state: StorePublicationState,
    pub(super) forced_cause: Option<PreparationFailureCode>,
    pub(super) evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    pub(super) domain: StoreDomainStamp,
}

pub(super) enum DeferredStoreFallbackState {
    Assembly { cause: PreparationFailureCode },
    FailureAssembly,
    Validated,
}

pub(super) struct DeferredStoreFallback {
    pub(super) state: DeferredStoreFallbackState,
    pub(super) domain: StoreDomainStamp,
}

impl<'inv, O, PP, U, EK, ET> StoreFailureAssembly<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn defer_until_body_cleanup(self) -> Result<DeferredStoreFailure<EK, ET>, Self> {
        let Self {
            raw,
            state,
            forced_cause,
            evidence_root,
            domain,
        } = self;
        match raw.defer_until_body_cleanup() {
            Ok(()) => Ok(DeferredStoreFailure {
                state,
                forced_cause,
                evidence_root,
                domain: domain.defer(),
            }),
            Err(raw) => Err(Self {
                raw,
                state,
                forced_cause,
                evidence_root,
                domain,
            }),
        }
    }
}

impl<'inv, O, PP, U> StoreFallbackOnly<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn defer_until_body_cleanup(self) -> Result<DeferredStoreFallback, Self> {
        let Self { state, domain } = self;
        let state = match state {
            StoreFallbackState::Assembly { raw, cause } => match raw.defer_until_body_cleanup() {
                Ok(()) => DeferredStoreFallbackState::Assembly { cause },
                Err(raw) => {
                    return Err(Self {
                        state: StoreFallbackState::Assembly { raw, cause },
                        domain,
                    });
                }
            },
            StoreFallbackState::FailureAssembly(raw) => match raw.defer_until_body_cleanup() {
                Ok(()) => DeferredStoreFallbackState::FailureAssembly,
                Err(raw) => {
                    return Err(Self {
                        state: StoreFallbackState::FailureAssembly(raw),
                        domain,
                    });
                }
            },
            StoreFallbackState::Validated(raw) => match raw.defer_until_body_cleanup() {
                Ok(()) => DeferredStoreFallbackState::Validated,
                Err(raw) => {
                    return Err(Self {
                        state: StoreFallbackState::Validated(raw),
                        domain,
                    });
                }
            },
        };
        Ok(DeferredStoreFallback {
            state,
            domain: domain.defer(),
        })
    }
}

impl<O, PP, U> StoreInvocationRoot<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn finish_deferred_evidence_only_operation(
        &mut self,
        pending: DeferredStoreOperation,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedStoreEvidenceOnlyOutcome, StorePrepareError> {
        let domain = StoreDomainLoan::resume(&mut self.domain, pending.domain)
            .expect("terminal completion keeps the original domain slot");
        let stamp = self
            .raw
            .prepare_evidence_only_terminal(final_progress)
            .map_err(StorePrepareError::from_raw)?
            .expect("terminal completion retains its deferred operation");
        let prepared = match self.raw.try_finish_evidence_only_terminal(stamp) {
            Ok(result) => result.map_err(StorePrepareError::FinalizationContainment)?,
            Err(_) => {
                let cause = self
                    .raw
                    .contain_payload_failure()
                    .expect("the original terminal graph provides its containment anchor");
                return Err(StorePrepareError::FinalizationContainment(cause));
            }
        };
        Ok(classify_prepared_evidence_only(
            prepared,
            domain.into_owned(),
        ))
    }

    pub(super) fn finish_deferred_minimal_operation(
        &mut self,
        pending: DeferredStoreOperation,
        code: u32,
    ) -> Result<PreparedStoreFallback, StorePrepareError> {
        let domain = StoreDomainLoan::resume(&mut self.domain, pending.domain)
            .expect("terminal completion keeps the original domain slot");
        let stamp = self
            .raw
            .prepare_minimal_terminal(PreparationFailureCode::domain(code))
            .map_err(StorePrepareError::from_raw)?
            .expect("terminal completion retains its deferred operation");
        let prepared = match self.raw.try_finish_minimal_terminal(stamp) {
            Ok(result) => result.map_err(|cause| {
                StorePrepareError::from_raw(
                    crate::retained::RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ),
                )
            })?,
            Err(_) => {
                let cause = self
                    .raw
                    .contain_payload_failure()
                    .expect("the original terminal graph provides its containment anchor");
                return Err(StorePrepareError::from_raw(
                    crate::retained::RetainedPublicationPrepareError::FinalizationContainment(
                        cause,
                    ),
                ));
            }
        };
        Ok(classify_prepared_fallback(prepared, domain.into_owned()))
    }

    pub(super) fn prepare_deferred_unattached(
        &mut self,
        pending: DeferredUnattachedStore<'_>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<StoreLedgerEvidence, StoreRuntimeError> {
        let DeferredUnattachedStore { binding, domain } = pending;
        let domain = StoreDomainLoan::resume(&mut self.domain, domain)
            .ok_or(StoreRuntimeError::OwnershipUnverified)?;
        let prepared = self
            .raw
            .prepare_unattached_terminal(binding, final_progress)
            .map_err(|_| StoreRuntimeError::OwnershipUnverified)?;
        let cause = prepared.cause.map_or(StoreTerminalCause::Success, |cause| {
            StoreTerminalCause::Failure(StorePreparationFailure::from_raw(cause))
        });
        Ok(StoreLedgerEvidence::from_unattached_snapshot(
            prepared.snapshot,
            cause,
            domain.into_owned(),
            prepared.contained,
        ))
    }

    pub(super) fn resume_deferred_operation(
        &mut self,
        pending: DeferredStoreOperation,
    ) -> Option<RetainedStoreOperation<'_, O, PP, U, TerminalStoreLineage>> {
        Some(RetainedStoreOperation {
            raw: self.raw.resume_deferred_operation()?,
            domain: StoreDomainLoan::resume(&mut self.domain, pending.domain)?,
            lineage: TerminalStoreLineage::new(),
        })
    }

    pub(super) fn resume_deferred_unrooted_failure(
        &mut self,
        pending: DeferredUnrootedStoreFailure,
    ) -> Option<(
        RawStoreFailureAssembly<'_, O, PP, U>,
        StoreDomainLoan<'_>,
        StorePublicationState,
    )> {
        Some((
            self.raw.resume_deferred_failure()?,
            StoreDomainLoan::resume(&mut self.domain, pending.domain)?,
            pending.state,
        ))
    }

    pub(super) fn resume_deferred_failure<EK, ET>(
        &mut self,
        pending: DeferredStoreFailure<EK, ET>,
    ) -> Option<StoreFailureAssembly<'_, O, PP, U, EK, ET>> {
        Some(StoreFailureAssembly {
            raw: self.raw.resume_deferred_failure()?,
            state: pending.state,
            forced_cause: pending.forced_cause,
            evidence_root: pending.evidence_root,
            domain: StoreDomainLoan::resume(&mut self.domain, pending.domain)?,
        })
    }

    pub(super) fn resume_deferred_fallback(
        &mut self,
        pending: DeferredStoreFallback,
    ) -> Option<StoreFallbackOnly<'_, O, PP, U>> {
        let state = match pending.state {
            DeferredStoreFallbackState::Assembly { cause } => StoreFallbackState::Assembly {
                raw: self.raw.resume_deferred_assembly()?,
                cause,
            },
            DeferredStoreFallbackState::FailureAssembly => {
                StoreFallbackState::FailureAssembly(self.raw.resume_deferred_failure()?)
            }
            DeferredStoreFallbackState::Validated => {
                StoreFallbackState::Validated(self.raw.resume_deferred_fallback()?)
            }
        };
        Some(StoreFallbackOnly {
            state,
            domain: StoreDomainLoan::resume(&mut self.domain, pending.domain)?,
        })
    }
}
