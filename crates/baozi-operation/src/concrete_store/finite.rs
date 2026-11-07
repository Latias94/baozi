//! Concrete finite Store ownership beside the original vocabulary domain.

use super::*;
use crate::retained::operation::finite_root::{
    FiniteAttachmentFailure, FiniteDeferredUnattachedBinding, FiniteEntryAdmission,
    FiniteInvocationRemainder, FiniteInvocationRoot, FiniteInvocationRootFields,
    FinitePendingRetainedAttachment, FiniteRootTerminalRejection, FiniteRootUnattachedFailure,
};

#[cfg(test)]
use super::activation::FiniteCanonicalInvocationHost;

mod operation;
pub use operation::FiniteRetainedStoreOperation;
mod deferred;
mod publication;
pub(in crate::concrete_store) use publication::FinitePublicationTransition;
pub(in crate::concrete_store) use publication::FiniteUnrootedStoreFailure;
pub use publication::{
    FiniteRetainedStorePublicationAssembly, FiniteStoreControlledSuccessValidation,
    FiniteStoreFailureAssembly, FiniteStoreFailureValidation, FiniteStoreFallbackOnly,
    FiniteStoreSuccessValidation, ValidatedFiniteStoreFailure, ValidatedFiniteStoreSuccess,
};

type RawFiniteStoreRoot<O, PP, U> = FiniteInvocationRoot<
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;
type RawFiniteStoreRemainder<O, PP, U> = FiniteInvocationRemainder<
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;
type RawFiniteStoreFields<'inv, O, PP, U> = FiniteInvocationRootFields<
    'inv,
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;
type RawFiniteStoreAttachment<'inv, 'account, O, PP, U> = FinitePendingRetainedAttachment<
    'inv,
    'account,
    O,
    StorePhaseKey,
    PP,
    U,
    StoreDimensionKey,
    StoreFallbackEnvelope,
    StoredBlock,
    StoredBlock,
>;
type StoreAttachmentFailure<O> =
    FiniteAttachmentFailure<O, StorePhaseKey, StoreDimensionKey, StoreFallbackEnvelope>;
type StoreUnattachedFailure<O, PP, U> =
    FiniteRootUnattachedFailure<O, StorePhaseKey, PP, U, StoreDimensionKey>;

/// The real finite numerical owner and its original runtime vocabulary.
pub struct FiniteStoreInvocationRoot<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> {
    raw: RawFiniteStoreRoot<O, PP, U>,
    domain: Option<StoreKeyDomain>,
    startup: Option<StoreAttachmentFailure<O>>,
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> FiniteStoreInvocationRoot<O, PP, U> {
    pub const fn new() -> Self {
        Self {
            raw: RawFiniteStoreRoot::new(),
            domain: None,
            startup: None,
        }
    }

    pub(super) fn invocation_identity(&self) -> *const () {
        self.raw.invocation_identity()
    }

    pub(super) fn record_payload_failure(&mut self, code: u32) {
        self.raw.record_payload_failure(code);
    }

    pub(in crate::concrete_store) fn contain_payload_failure(
        &mut self,
    ) -> Option<FinalizationContainmentError> {
        self.raw.fields().contain_deferred_fault()
    }

    pub(super) fn startup_error(&self) -> Option<RetainedStoreStartError> {
        self.startup
            .as_ref()
            .map(|failure| map_store_start_error_ref(&failure.cause))
    }

    pub(super) fn fields(&mut self) -> FiniteStoreFinalFields<'_, O, PP, U> {
        FiniteStoreFinalFields {
            raw: self.raw.fields(),
            domain: &mut self.domain,
            startup: &mut self.startup,
        }
    }

    pub(super) fn finish_unattached(
        &mut self,
        binding: FiniteDeferredUnattachedBinding,
        entry: &mut FiniteEntryAdmission<'_>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<FiniteStoreFacts, StoreUnattachedFailure<O, PP, U>> {
        let terminal = self.raw.finish_unattached(binding, entry, final_progress)?;
        let cause = terminal.cause.map_or(StoreTerminalCause::Success, |cause| {
            StoreTerminalCause::Failure(StorePreparationFailure::from_raw(cause))
        });
        Ok(FiniteStoreFacts::from_snapshot(
            terminal.snapshot,
            cause,
            self.domain
                .take()
                .expect("finite completion consumes the original domain once"),
        ))
    }

    pub(super) fn into_error(self, cause: FiniteStoreExitCause) -> FiniteStoreExitError<O, PP, U> {
        FiniteStoreExitError {
            cause,
            state: FiniteStoreExitState::Rooted {
                root: Some(self),
                failure: None,
            },
            rejected_domain: None,
            original_freeze_control: None,
        }
    }

    fn into_extracted_remainder(mut self) -> FiniteStoreRemainder<O, PP, U> {
        let domain = scopeguard::guard(self.domain.take(), std::mem::forget);
        let raw = self.raw.fields().into_owned().into_extracted_remainder();
        FiniteStoreRemainder {
            raw,
            domain: scopeguard::ScopeGuard::into_inner(domain),
            startup: self.startup.take(),
        }
    }

    pub(in crate::concrete_store) fn into_unattached_error(
        self,
        failure: StoreUnattachedFailure<O, PP, U>,
    ) -> FiniteStoreExitError<O, PP, U> {
        let state = match failure {
            StoreUnattachedFailure::Rejected(binding) => FiniteStoreExitState::Rooted {
                root: Some(self),
                failure: Some(StoreRootedFailure::Unattached { _binding: binding }),
            },
            failure @ StoreUnattachedFailure::Owned(_) => FiniteStoreExitState::Unattached {
                remainder: self.into_extracted_remainder(),
                failure,
            },
        };
        FiniteStoreExitError {
            cause: FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Unattached),
            state,
            rejected_domain: None,
            original_freeze_control: None,
        }
    }

    fn into_terminal_error(
        self,
        cause: FiniteStoreExitCause,
        failure: operation::StoreTerminalFailure<O, PP, U>,
    ) -> FiniteStoreExitError<O, PP, U> {
        let state = match failure {
            operation::StoreTerminalFailure::Rejected(rejection) => FiniteStoreExitState::Rooted {
                root: Some(self),
                failure: Some(StoreRootedFailure::Terminal {
                    _rejection: rejection,
                }),
            },
            failure @ (operation::StoreTerminalFailure::Detach { .. }
            | operation::StoreTerminalFailure::Commit { .. }) => FiniteStoreExitState::Terminal {
                remainder: self.into_extracted_remainder(),
                failure,
            },
        };
        FiniteStoreExitError {
            cause,
            state,
            rejected_domain: None,
            original_freeze_control: None,
        }
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> Default
    for FiniteStoreInvocationRoot<O, PP, U>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> Drop
    for FiniteStoreInvocationRoot<O, PP, U>
{
    fn drop(&mut self) {
        let domain = scopeguard::guard(self.domain.take(), std::mem::forget);
        let startup_uncertain = self
            .startup
            .as_ref()
            .is_some_and(StoreAttachmentFailure::has_residue);
        // Independent startup containment protects its actual graph before common retirement.
        drop(self.startup.take());
        self.raw.cleanup_owned_state();
        if !startup_uncertain && !self.raw.must_retain_domain() {
            let _domain = scopeguard::ScopeGuard::into_inner(domain);
        }
    }
}

struct FiniteStoreRemainder<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> {
    raw: RawFiniteStoreRemainder<O, PP, U>,
    domain: Option<StoreKeyDomain>,
    startup: Option<StoreAttachmentFailure<O>>,
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> Drop
    for FiniteStoreRemainder<O, PP, U>
{
    fn drop(&mut self) {
        let domain = scopeguard::guard(self.domain.take(), std::mem::forget);
        let startup_uncertain = self
            .startup
            .as_ref()
            .is_some_and(StoreAttachmentFailure::has_residue);
        drop(self.startup.take());
        self.raw.cleanup_owned_state();
        if !startup_uncertain && !self.raw.must_retain_domain() {
            let _domain = scopeguard::ScopeGuard::into_inner(domain);
        }
    }
}

/// The original owning Options remain recoverable through every shorter ordinary loan.
pub(in crate::concrete_store) struct FiniteStoreFinalFields<
    'inv,
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
> {
    raw: RawFiniteStoreFields<'inv, O, PP, U>,
    domain: &'inv mut Option<StoreKeyDomain>,
    startup: &'inv mut Option<StoreAttachmentFailure<O>>,
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> FiniteStoreFinalFields<'_, O, PP, U> {
    fn into_owned(self) -> FiniteStoreInvocationRoot<O, PP, U> {
        FiniteStoreInvocationRoot {
            raw: self.raw.into_owned(),
            domain: self.domain.take(),
            startup: self.startup.take(),
        }
    }

    pub(in crate::concrete_store) fn into_error(
        self,
        cause: FiniteStoreExitCause,
    ) -> FiniteStoreExitError<O, PP, U> {
        self.into_owned().into_error(cause)
    }

    pub(in crate::concrete_store) fn into_request_error(
        mut self,
        error: Option<StorePrepareError>,
    ) -> FiniteStoreExitError<O, PP, U> {
        let cause = error.or_else(|| {
            self.raw
                .contain_deferred_fault()
                .map(StorePrepareError::FinalizationContainment)
        });
        self.into_error(FiniteStoreExitCause::Finalize(cause.map_or(
            FiniteStoreFinalizeCause::ForeignTerminalAuthority,
            FiniteStoreFinalizeCause::Publication,
        )))
    }
}

/// The semantic stage that rejected the one finite invocation.
#[derive(Debug)]
pub enum FiniteStoreExitCause {
    Start(RetainedStoreStartError),
    Body(activation::CanonicalInvocationBodyError),
    Finalize(FiniteStoreFinalizeCause),
}

/// Final rejection of the actual finite numerical or retained-publication owner.
#[derive(Debug)]
pub enum FiniteStoreFinalizeCause {
    ForeignTerminalAuthority,
    Unattached,
    PublicationOpen(StoreRuntimeError),
    Publication(StorePrepareError),
}

impl fmt::Display for FiniteStoreFinalizeCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForeignTerminalAuthority => {
                formatter.write_str("the finite terminal permission belongs to another invocation")
            }
            Self::Unattached => formatter.write_str(
                "the actual finite invocation could not be captured as clean terminal facts",
            ),
            Self::PublicationOpen(cause) => cause.fmt(formatter),
            Self::Publication(cause) => cause.fmt(formatter),
        }
    }
}

impl fmt::Display for FiniteStoreExitCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(cause) => cause.fmt(formatter),
            Self::Body(cause) => cause.fmt(formatter),
            Self::Finalize(cause) => cause.fmt(formatter),
        }
    }
}

enum StoreRootedFailure {
    Unattached {
        _binding: FiniteDeferredUnattachedBinding,
    },
    Terminal {
        _rejection: FiniteRootTerminalRejection,
    },
}

#[allow(
    clippy::large_enum_variant,
    reason = "original numerical failure owners remain inline without failure-path allocation"
)]
enum FiniteStoreExitState<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> {
    Rooted {
        root: Option<FiniteStoreInvocationRoot<O, PP, U>>,
        failure: Option<StoreRootedFailure>,
    },
    Unattached {
        remainder: FiniteStoreRemainder<O, PP, U>,
        failure: StoreUnattachedFailure<O, PP, U>,
    },
    Terminal {
        remainder: FiniteStoreRemainder<O, PP, U>,
        failure: operation::StoreTerminalFailure<O, PP, U>,
    },
}

/// An owned failure: no callback, host/account loan, retry or deferred drain is required.
pub struct FiniteStoreExitError<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> {
    cause: FiniteStoreExitCause,
    state: FiniteStoreExitState<O, PP, U>,
    rejected_domain: Option<StoreKeyDomain>,
    original_freeze_control: Option<ControlCause>,
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> FiniteStoreExitError<O, PP, U> {
    pub(in crate::concrete_store) fn spent(cause: FiniteStoreExitCause) -> Self {
        Self {
            cause,
            state: FiniteStoreExitState::Rooted {
                root: None,
                failure: None,
            },
            rejected_domain: None,
            original_freeze_control: None,
        }
    }
    pub const fn cause(&self) -> &FiniteStoreExitCause {
        &self.cause
    }

    /// Borrows the detailed control cause observed by the original finite freeze, if any.
    pub const fn original_freeze_control(&self) -> Option<&ControlCause> {
        self.original_freeze_control.as_ref()
    }

    pub(in crate::concrete_store) fn attach_original_freeze_control(
        &mut self,
        control: Option<ControlCause>,
    ) {
        if self.original_freeze_control.is_none() {
            self.original_freeze_control = control;
        }
    }

    /// Borrows the original domain even when startup never produced qualified facts.
    pub fn domain(&self) -> Option<&StoreKeyDomain> {
        let domain = match &self.state {
            FiniteStoreExitState::Rooted { root, .. } => {
                root.as_ref().and_then(|root| root.domain.as_ref())
            }
            FiniteStoreExitState::Unattached { remainder, .. }
            | FiniteStoreExitState::Terminal { remainder, .. } => remainder.domain.as_ref(),
        };
        domain.or(self.rejected_domain.as_ref())
    }

    /// Returns whether this error owns an original root or low-level failure residue.
    pub fn has_residue(&self) -> bool {
        match &self.state {
            FiniteStoreExitState::Rooted { root, failure } => root.is_some() || failure.is_some(),
            FiniteStoreExitState::Unattached { .. } | FiniteStoreExitState::Terminal { .. } => true,
        }
    }

    #[cfg(test)]
    pub(in crate::concrete_store) fn test_uncommitted_cover_observation(
        &mut self,
    ) -> Option<(u64, u64, u64, usize)> {
        match &mut self.state {
            FiniteStoreExitState::Terminal {
                failure: operation::StoreTerminalFailure::Commit { proof, .. },
                ..
            } => proof.test_cover_observation(),
            _ => None,
        }
    }

    pub(in crate::concrete_store) fn record_payload_failure(&mut self, code: u32) {
        if let FiniteStoreExitState::Rooted {
            root: Some(root), ..
        } = &mut self.state
        {
            root.record_payload_failure(code);
        }
    }

    pub(in crate::concrete_store) fn contain_payload_failure(&mut self) {
        match &mut self.state {
            FiniteStoreExitState::Rooted {
                root: Some(root), ..
            } => {
                root.contain_payload_failure();
            }
            FiniteStoreExitState::Unattached { remainder, .. }
            | FiniteStoreExitState::Terminal { remainder, .. } => {
                remainder.raw.contain_payload_failure();
            }
            FiniteStoreExitState::Rooted { root: None, .. } => {}
        }
    }

    pub(in crate::concrete_store) fn with_rejected_domain(
        mut self,
        domain: Option<StoreKeyDomain>,
    ) -> Self {
        self.rejected_domain = domain;
        self
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> fmt::Debug
    for FiniteStoreExitError<O, PP, U>
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FiniteStoreExitError")
            .field("cause", &self.cause)
            .field("domain", &self.domain())
            .field("has_residue", &self.has_residue())
            .finish_non_exhaustive()
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> fmt::Display
    for FiniteStoreExitError<O, PP, U>
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> std::error::Error
    for FiniteStoreExitError<O, PP, U>
{
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> Drop
    for FiniteStoreExitError<O, PP, U>
{
    fn drop(&mut self) {
        let state = std::mem::replace(
            &mut self.state,
            FiniteStoreExitState::Rooted {
                root: None,
                failure: None,
            },
        );
        match state {
            FiniteStoreExitState::Rooted { root, failure } => {
                let failure = scopeguard::guard(failure, std::mem::forget);
                drop(root);
                let _failure = scopeguard::ScopeGuard::into_inner(failure);
            }
            FiniteStoreExitState::Unattached { remainder, failure } => {
                let failure = scopeguard::guard(failure, std::mem::forget);
                drop(remainder);
                drop(scopeguard::ScopeGuard::into_inner(failure));
            }
            FiniteStoreExitState::Terminal { remainder, failure } => {
                let failure = scopeguard::guard(failure, std::mem::forget);
                drop(remainder);
                drop(scopeguard::ScopeGuard::into_inner(failure));
            }
        }
    }
}

pub(super) struct FiniteBoundStoreOperation<'inv, 'account, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: RawFiniteStoreAttachment<'inv, 'account, O, PP, U>,
    domain: &'inv mut Option<StoreKeyDomain>,
    startup: &'inv mut Option<StoreAttachmentFailure<O>>,
    fallback: StoreFallbackEnvelope,
    lineage: L,
}

pub(super) fn bind_finite_store<'inv, 'account, O, PP, U, L>(
    root: &'inv mut FiniteStoreInvocationRoot<O, PP, U>,
    entry: &'inv mut FiniteEntryAdmission<'account>,
    control: &mut Option<OperationControl<PP, U>>,
    ledger: FiniteStoreLedgerPlan,
    source: Option<&PublishedCanonicalOutput<StoredBlock>>,
    plan: FiniteRetainedStoreStartPlan,
    lineage: L,
) -> Result<FiniteBoundStoreOperation<'inv, 'account, O, PP, U, L>, RetainedStoreStartError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let validation = plan.validate(&ledger);
    let (domain, limits, phases) = ledger.into_parts();
    if root.domain.is_none() {
        root.domain = Some(domain);
    }
    validation?;
    let (plan, fallback) = plan.into_raw();
    let raw = match source {
        Some(source) => root.raw.bind_derived_with_control(
            control,
            limits,
            phases,
            source.aggregate(),
            entry,
            plan,
        ),
        None => root
            .raw
            .bind_with_control(control, limits, phases, entry, plan),
    }
    .map_err(map_store_start_error)?;
    Ok(FiniteBoundStoreOperation {
        raw,
        domain: &mut root.domain,
        startup: &mut root.startup,
        fallback,
        lineage,
    })
}

impl<'inv, O, PP, U, L> FiniteBoundStoreOperation<'inv, '_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) fn invocation_identity(&self) -> *const () {
        self.raw.invocation_identity()
    }
    pub(super) fn with_exclusive_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        let parts = self
            .raw
            .exclusive_parts()
            .map_err(execution::map_execution_error)
            .map_err(StoreInvocationError::from)?;
        execution::execute_store_parts(
            parts,
            self.domain.as_ref().expect("bound original domain"),
            body,
        )
    }

    pub(super) fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        let parts = self
            .raw
            .exclusive_parts()
            .map_err(execution::map_execution_error)?;
        execution::execute_store_action_parts(
            parts,
            self.domain.as_ref().expect("bound original domain"),
            dimension,
            body,
        )
    }

    pub(super) fn defer_unattached(
        self,
        requested: Option<PreparationFailureCode>,
    ) -> Result<FiniteDeferredUnattachedBinding, StoreRuntimeError> {
        self.raw
            .defer_unattached(requested)
            .map_err(|_| StoreRuntimeError::OwnershipUnverified)
    }

    pub(super) fn attach(
        self,
    ) -> Result<FiniteRetainedStoreOperation<'inv, O, PP, U, L>, RetainedStoreStartError> {
        let Self {
            raw,
            domain,
            startup,
            fallback,
            lineage,
        } = self;
        match raw.attach_fields(|| Ok(fallback)) {
            Ok(raw) => Ok(FiniteRetainedStoreOperation {
                fields: FiniteStoreFinalFields {
                    raw,
                    domain,
                    startup,
                },
                lineage,
            }),
            Err(failure) => {
                let cause = map_store_start_error_ref(&failure.cause);
                *startup = Some(failure);
                Err(cause)
            }
        }
    }
}

#[cfg(test)]
pub(in crate::concrete_store) mod tests;
