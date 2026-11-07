//! Concrete finite invocation ownership; ordinary permissions only borrow its actual slots.

use super::*;
use crate::ledger::finite::FiniteLedgerLimits;
use crate::retained::operation_start::{
    FiniteRetainedOperationStartPlan, FiniteStartupRetainedResidue, QualifiedFailureTerminalWork,
};
use crate::retained::ownership::FiniteFallbackPublicationBaseline;
use crate::retained::startup::{
    FiniteStartupAdmissionCause, FiniteStartupCrossPlaneAdmissionFailure,
    FiniteStartupCrossPlaneContainment, FiniteStartupCrossPlanePlan,
};

mod start;
mod terminal;
pub(crate) use terminal::FiniteRootTerminalFailure;
#[cfg(test)]
mod tests;

pub(crate) struct FiniteRetainedOperationFields<O, P, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    baseline: FiniteFallbackPublicationBaseline<O, P, D, F>,
    metadata: RetainedOperationMetadata<P, D, CB, EB>,
}

impl<'inv, O, P, D, F, CB, EB> From<&'inv mut FiniteRetainedOperationFields<O, P, D, F, CB, EB>>
    for RetainedOperationFieldsLoan<'inv, O, P, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn from(fields: &'inv mut FiniteRetainedOperationFields<O, P, D, F, CB, EB>) -> Self {
        Self {
            baseline: FallbackPublicationBaselineLoan::Finite(&mut fields.baseline),
            metadata: &mut fields.metadata,
        }
    }
}

/// The sole concrete numerical and physical owner of a finite invocation.
pub(crate) struct FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    terminal: Option<super::super::FiniteTerminalPhysicalState<F, CB, EB>>,
    assembly: super::super::PublicationAssemblySlot<O, P, D, CB, EB>,
    retained: Option<FiniteRetainedOperationFields<O, P, D, F, CB, EB>>,
    common: Option<FiniteOperationInvocation<O, P, PP, U, D>>,
    started: bool,
    contained: bool,
}

/// Entry-owned admission. A pending permission borrows this guard; it never owns it.
pub(crate) struct FiniteEntryAdmission<'account> {
    account: &'account PhysicalStorageAccount,
    pending: Option<startup::PendingOperationStartAdmission<'account>>,
    identity: Option<*const ()>,
}

impl<'account> FiniteEntryAdmission<'account> {
    pub(crate) const fn new(account: &'account PhysicalStorageAccount) -> Self {
        Self {
            account,
            pending: None,
            identity: None,
        }
    }

    pub(crate) const fn account(&self) -> &'account PhysicalStorageAccount {
        self.account
    }
}

/// This token owns no admission, control owner, numerical state or account share.
pub(crate) struct FiniteDeferredUnattachedBinding {
    identity: *const (),
}

#[expect(
    clippy::large_enum_variant,
    reason = "normal rejection transfers its actual finite owner inline without a failure-path allocation"
)]
pub(crate) enum FiniteRootUnattachedFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    Rejected(FiniteDeferredUnattachedBinding),
    Owned(invocation::FiniteUnattachedFailure<O, P, PP, U, D>),
}

type OptionalOperationLoan<'inv, O, P, PP, U, D, F, CB, EB> =
    Option<RetainedOperationLoan<'inv, O, P, PP, U, D, F, CB, EB>>;
type UnattachedResult<O, P, PP, U, D> =
    Result<invocation::FiniteUnattachedTerminal<P, D>, FiniteRootUnattachedFailure<O, P, PP, U, D>>;

/// All fields are original root references; no field points into another owned field.
pub(crate) struct FiniteInvocationRootFields<'inv, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    terminal: &'inv mut Option<super::super::FiniteTerminalPhysicalState<F, CB, EB>>,
    assembly: &'inv mut super::super::PublicationAssemblySlot<O, P, D, CB, EB>,
    retained: &'inv mut Option<FiniteRetainedOperationFields<O, P, D, F, CB, EB>>,
    common: &'inv mut Option<FiniteOperationInvocation<O, P, PP, U, D>>,
    started: &'inv mut bool,
    contained: &'inv mut bool,
}

impl<O, P, PP, U, D, F, CB, EB> FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    pub(crate) const fn new() -> Self {
        Self {
            terminal: None,
            assembly: None,
            retained: None,
            common: None,
            started: false,
            contained: false,
        }
    }

    pub(crate) fn fields(&mut self) -> FiniteInvocationRootFields<'_, O, P, PP, U, D, F, CB, EB> {
        FiniteInvocationRootFields {
            terminal: &mut self.terminal,
            assembly: &mut self.assembly,
            retained: &mut self.retained,
            common: &mut self.common,
            started: &mut self.started,
            contained: &mut self.contained,
        }
    }

    pub(crate) fn loan(&mut self) -> OptionalOperationLoan<'_, O, P, PP, U, D, F, CB, EB> {
        self.common.as_ref()?;
        Some(RetainedOperationLoan {
            retained: self.retained.as_mut()?.into(),
            assembly: Some(&mut self.assembly),
            terminal: TerminalPhysicalSlot::Finite(&mut self.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Finite(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            },
        })
    }

    pub(crate) fn has_started(&self) -> bool {
        self.started
    }

    pub(crate) fn must_retain_domain(&self) -> bool {
        self.contained
    }

    pub(crate) fn invocation_identity(&self) -> *const () {
        std::ptr::from_ref(&self.common).cast()
    }

    pub(crate) fn terminal_catalog(&self) -> Option<crate::ledger::LedgerReadView<'_, O, P, D>> {
        self.common
            .as_ref()
            .map(|common| common.coordinator.retained_ledger().as_read())
    }

    pub(crate) fn record_payload_failure(&mut self, code: u32) {
        if let Some(common) = self.common.as_mut() {
            reject_retained_attachment(
                &mut common.view(),
                PreparationFailureCode::product_graph(code),
            );
        }
    }

    pub(crate) fn cleanup_owned_state(&mut self) {
        if let Some(physical) = self.terminal.take() {
            self.contained |= !physical.graph.is_empty();
            drop(physical);
        }
        let retain = cleanup_retained_root_graph(
            self.retained
                .as_mut()
                .map(RetainedOperationFieldsLoan::from),
            &mut self.assembly,
            TerminalPhysicalSlot::Finite(&mut self.terminal),
            InvocationLoan {
                common: InvocationSlot::Finite(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            },
        );
        if retain {
            std::mem::forget(self.assembly.take());
            std::mem::forget(self.retained.take());
            std::mem::forget(self.common.take());
        } else {
            drop(self.retained.take());
            let unresolved = InvocationLoan {
                common: InvocationSlot::Finite(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            }
            .close_abandoned_common();
            if unresolved {
                std::mem::forget(self.common.take());
            }
        }
    }

    /// Runs after body captures are disposed and consumes this entry's original admission.
    pub(crate) fn finish_unattached(
        &mut self,
        binding: FiniteDeferredUnattachedBinding,
        entry: &mut FiniteEntryAdmission<'_>,
        final_progress: FinalProgress<PP, U>,
    ) -> UnattachedResult<O, P, PP, U, D> {
        if binding.identity != self.invocation_identity()
            || entry.identity != Some(binding.identity)
            || self.retained.is_some()
            || self.assembly.is_some()
            || self.terminal.is_some()
            || self.common.is_none()
            || entry.pending.is_none()
        {
            return Err(FiniteRootUnattachedFailure::Rejected(binding));
        }
        let admission = entry
            .pending
            .take()
            .expect("the original entry admission was checked");
        // Preserve outer domain duty if fixed capture or control disposal unwinds.
        let already_contained = self.contained;
        self.contained = true;
        let common = self
            .common
            .take()
            .expect("unattached completion takes the actual finite owner");
        match common.prepare_and_finish_unattached(admission, final_progress) {
            Ok(prepared) => {
                self.contained = already_contained;
                Ok(prepared)
            }
            Err(error) => Err(FiniteRootUnattachedFailure::Owned(error)),
        }
    }
}

impl<O, P, PP, U, D, F, CB, EB> Drop for FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    fn drop(&mut self) {
        self.cleanup_owned_state();
    }
}

impl<'inv, O, P, PP, U, D, F, CB, EB> FiniteInvocationRootFields<'inv, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    pub(crate) fn loan(&mut self) -> OptionalOperationLoan<'_, O, P, PP, U, D, F, CB, EB> {
        self.common.as_ref()?;
        Some(RetainedOperationLoan {
            retained: self.retained.as_mut()?.into(),
            assembly: Some(self.assembly),
            terminal: TerminalPhysicalSlot::Finite(self.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Finite(self.common),
                contained: self.contained,
                not_send_or_sync: PhantomData,
            },
        })
    }

    pub(crate) fn into_loan(self) -> OptionalOperationLoan<'inv, O, P, PP, U, D, F, CB, EB> {
        self.common.as_ref()?;
        Some(RetainedOperationLoan {
            retained: self.retained.as_mut()?.into(),
            assembly: Some(self.assembly),
            terminal: TerminalPhysicalSlot::Finite(self.terminal),
            common: InvocationLoan {
                common: InvocationSlot::Finite(self.common),
                contained: self.contained,
                not_send_or_sync: PhantomData,
            },
        })
    }

    /// Transfers original owners to a normal error; the source shell stays irreversibly spent.
    pub(crate) fn into_owned(self) -> FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB> {
        let contained = *self.contained;
        *self.started = true;
        *self.contained = false;
        FiniteInvocationRoot {
            terminal: self.terminal.take(),
            assembly: self.assembly.take(),
            retained: self.retained.take(),
            common: self.common.take(),
            started: true,
            contained,
        }
    }
}

pub(crate) struct FinitePendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    root: &'inv mut FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>,
    entry: &'inv mut FiniteEntryAdmission<'account>,
    plan: FiniteRetainedOperationStartPlan<P, D>,
    failure_terminal_work: QualifiedFailureTerminalWork<D>,
    local: PhantomData<Rc<()>>,
}

/// Both uncertainty paths keep their actual independent physical owners beside the cause.
pub(crate) struct FiniteAttachmentFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) cause: RetainedOperationStartError<D>,
    pub(crate) startup: Option<FiniteStartupCrossPlaneContainment<O, P, D>>,
    pub(crate) retained: Option<FiniteStartupRetainedResidue<O, P, D, F>>,
}

impl<O, P, D, F> fmt::Debug for FiniteAttachmentFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FiniteAttachmentFailure")
            .field("cause", &self.cause)
            .field("retains_cross_plane_owners", &self.startup.is_some())
            .field("retains_construction_owners", &self.retained.is_some())
            .finish()
    }
}

impl<O, P, D, F> FiniteAttachmentFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn clean(cause: RetainedOperationStartError<D>) -> Self {
        Self {
            cause,
            startup: None,
            retained: None,
        }
    }
}
