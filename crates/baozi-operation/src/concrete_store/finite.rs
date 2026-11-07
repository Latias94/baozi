//! Concrete finite Store ownership beside the original vocabulary domain.

use super::*;
use crate::retained::operation::finite_root::{
    FiniteAttachmentFailure, FiniteDeferredUnattachedBinding, FiniteEntryAdmission,
    FiniteInvocationRoot, FiniteInvocationRootFields, FinitePendingRetainedAttachment,
    FiniteRootUnattachedFailure,
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
            residue: Some(self),
            unattached: None,
            terminal: None,
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
        let startup_uncertain = self
            .startup
            .as_ref()
            .is_some_and(|failure| failure.startup.is_some() || failure.retained.is_some());
        // Independent startup containment protects its actual graph before common retirement.
        drop(self.startup.take());
        self.raw.cleanup_owned_state();
        if startup_uncertain || self.raw.must_retain_domain() {
            let _domain = std::mem::ManuallyDrop::new(self.domain.take());
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
    pub(in crate::concrete_store) fn into_error(
        self,
        cause: FiniteStoreExitCause,
    ) -> FiniteStoreExitError<O, PP, U> {
        FiniteStoreInvocationRoot {
            raw: self.raw.into_owned(),
            domain: self.domain.take(),
            startup: self.startup.take(),
        }
        .into_error(cause)
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

/// An owned failure: no callback, host/account loan, retry or deferred drain is required.
pub struct FiniteStoreExitError<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> {
    cause: FiniteStoreExitCause,
    residue: Option<FiniteStoreInvocationRoot<O, PP, U>>,
    unattached: Option<StoreUnattachedFailure<O, PP, U>>,
    terminal: Option<operation::StoreTerminalFailure<O, PP, U>>,
    rejected_domain: Option<StoreKeyDomain>,
    original_freeze_control: Option<ControlCause>,
}

impl<O: OperationOwner, PP: ProgressPhase, U: ProgressUnit> FiniteStoreExitError<O, PP, U> {
    pub(in crate::concrete_store) fn spent(cause: FiniteStoreExitCause) -> Self {
        Self {
            cause,
            residue: None,
            unattached: None,
            terminal: None,
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
        self.residue
            .as_ref()
            .and_then(|root| root.domain.as_ref())
            .or(self.rejected_domain.as_ref())
    }

    /// Returns whether this error owns an original root or low-level failure residue.
    pub fn has_residue(&self) -> bool {
        self.residue.is_some() || self.unattached.is_some() || self.terminal.is_some()
    }

    #[cfg(test)]
    pub(in crate::concrete_store) fn test_uncommitted_cover_observation(
        &mut self,
    ) -> Option<(u64, u64, u64, usize)> {
        match self.terminal.as_mut()? {
            crate::retained::operation::finite_root::FiniteRootTerminalFailure::Commit {
                proof,
                ..
            } => proof.test_cover_observation(),
            _ => None,
        }
    }

    pub(in crate::concrete_store) fn record_payload_failure(&mut self, code: u32) {
        if let Some(root) = self.residue.as_mut() {
            root.record_payload_failure(code);
        }
    }

    pub(in crate::concrete_store) fn contain_payload_failure(&mut self) {
        if let Some(root) = self.residue.as_mut() {
            root.contain_payload_failure();
        }
    }

    pub(super) fn with_unattached(mut self, failure: StoreUnattachedFailure<O, PP, U>) -> Self {
        self.unattached = Some(failure);
        self
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
        // The root contains physical owners before independent numerical failure owners retire.
        drop(self.residue.take());
        drop(self.unattached.take());
        drop(self.terminal.take());
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
