//! Short shared preparation ends before concrete finite root extraction.

use super::*;
use crate::ledger::finite::{FiniteDetachedTypedFreezeProof, FiniteTypedFreezeDetachError};
use crate::retained::operation::invocation::FiniteTerminalRootFields;
use crate::retained::{
    DeferredAttachedTerminal, PreparedFiniteRetainedPublication, RetainedAssemblyOpened,
    RetainedPublicationAssemblyLoan, RetainedPublicationBorrowFailure, RetainedTerminalRequest,
};

/// Original numerical owners returned by a normal finite terminal rejection.
#[expect(
    clippy::large_enum_variant,
    reason = "typed detach and commit failures retain their original owners inline without allocating during failure"
)]
pub(crate) enum FiniteRootTerminalFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    RejectedPreparation(DeferredAttachedTerminal),
    RejectedStamp(AttachedTerminalStamp),
    Preparation(RetainedPublicationPrepareError),
    Detach {
        cause: FinalizationContainmentError,
        residue: FiniteTypedFreezeDetachError<O, P, PP, U, D>,
    },
    Commit {
        cause: FinalizationContainmentError,
        proof: FiniteDetachedTypedFreezeProof<O, P, D>,
    },
}

impl<O, P, PP, U, D> FiniteRootTerminalFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn cause(&self) -> Option<&FinalizationContainmentError> {
        match self {
            Self::RejectedPreparation(_) | Self::RejectedStamp(_) => None,
            Self::Preparation(RetainedPublicationPrepareError::FinalizationContainment(cause))
            | Self::Detach { cause, .. }
            | Self::Commit { cause, .. } => Some(cause),
        }
    }
}

type PreparedResult<O, P, PP, U, D, F, CB, EB> = Result<
    PreparedFiniteRetainedPublication<CB, EB, F, P, D>,
    FiniteRootTerminalFailure<O, P, PP, U, D>,
>;
type OptionalAssemblyLoan<'inv, O, P, PP, U, D, F, CB, EB> =
    Option<RetainedPublicationAssemblyLoan<'inv, O, P, PP, U, D, F, CB, EB>>;
type PreparedWithControlResult<O, P, PP, U, D, F, CB, EB> = Result<
    (
        PreparedFiniteRetainedPublication<CB, EB, F, P, D>,
        Option<ControlCause>,
    ),
    FiniteRootTerminalFailure<O, P, PP, U, D>,
>;

impl<O, P, PP, U, D, F, CB, EB> FiniteInvocationRootFields<'_, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
    CB: Send + Sync + 'static,
    EB: Send + Sync + 'static,
{
    pub(crate) fn assembly_loan(&mut self) -> OptionalAssemblyLoan<'_, O, P, PP, U, D, F, CB, EB> {
        self.common.as_ref()?;
        Some(RetainedPublicationAssemblyLoan {
            fields: self.assembly.as_mut()?.into(),
            operation: Some(RetainedOperationLoan {
                retained: self.retained.as_mut()?.into(),
                terminal: TerminalPhysicalSlot::Finite(self.terminal),
                common: InvocationLoan {
                    common: InvocationSlot::Finite(self.common),
                    contained: self.contained,
                    not_send_or_sync: PhantomData,
                },
                assembly: None,
            }),
        })
    }

    fn original_publication_identity(&mut self) -> Option<crate::ledger::LedgerIdentity> {
        if *self.contained
            || self.terminal.is_some()
            || self
                .retained
                .as_ref()
                .is_none_or(|retained| retained.baseline.first_account().is_none())
        {
            return None;
        }
        let operation = self.loan()?;
        if operation.common.terminal_is_outstanding() {
            return None;
        }
        Some(operation.common.as_read().identity())
    }

    /// Consumes only the stamp, without transferring or reopening an original root field.
    pub(crate) fn bind_opened_stamp(
        &mut self,
        opened: RetainedAssemblyOpened,
    ) -> Result<(), RetainedAssemblyOpened> {
        let Some(identity) = self.original_publication_identity() else {
            return Err(opened);
        };
        if self
            .assembly
            .as_ref()
            .is_some_and(|fields| opened.matches(identity, fields.set_id))
        {
            Ok(())
        } else {
            Err(opened)
        }
    }

    pub(crate) fn bind_publication_failure(
        &mut self,
        failure: RetainedPublicationBorrowFailure<D>,
    ) -> Result<RetainedPublicationBorrowFailure<D>, RetainedPublicationBorrowFailure<D>> {
        let Some(identity) = self.original_publication_identity() else {
            return Err(failure);
        };
        if self.assembly.is_none() && failure.belongs_to(identity) {
            Ok(failure)
        } else {
            Err(failure)
        }
    }

    fn ownership_failure(&mut self) -> FiniteRootTerminalFailure<O, P, PP, U, D> {
        let reason = AccountPoisonReason::OwnershipGraphUnverified;
        let account = self
            .terminal
            .as_ref()
            .and_then(|physical| physical.graph.account.as_ref())
            .or_else(|| {
                self.assembly
                    .as_ref()
                    .and_then(|fields| fields.account.as_ref())
            })
            .or_else(|| {
                self.retained
                    .as_ref()
                    .and_then(|fields| fields.baseline.first_account())
            })
            .expect("an attached finite terminal retains its original account anchor");
        let poisoned = account.record_poison(reason);
        *self.contained = true;
        FiniteRootTerminalFailure::Preparation(
            RetainedPublicationPrepareError::FinalizationContainment(
                FinalizationContainmentError::new(poisoned, reason),
            ),
        )
    }

    pub(crate) fn finish_deferred(
        &mut self,
        prepared: DeferredAttachedTerminal,
    ) -> PreparedResult<O, P, PP, U, D, F, CB, EB> {
        let Some(retained) = self.retained.as_mut() else {
            return Err(FiniteRootTerminalFailure::RejectedPreparation(prepared));
        };
        let mut fields = FiniteTerminalRootFields::new(
            self.common,
            &mut retained.baseline,
            self.terminal,
            self.assembly.as_mut(),
            self.contained,
        );
        let stamp = match fields.install_terminal_preparation(prepared) {
            Ok(stamp) => stamp,
            Err(prepared) => return Err(FiniteRootTerminalFailure::RejectedPreparation(prepared)),
        };
        let commit = match fields.try_detach_terminal(stamp) {
            Ok(Ok(commit)) => commit,
            Ok(Err(error)) => {
                return Err(FiniteRootTerminalFailure::Detach {
                    cause: error.cause,
                    residue: error.residue,
                });
            }
            Err(stamp) => return Err(FiniteRootTerminalFailure::RejectedStamp(stamp)),
        };
        commit
            .try_into_prepared()
            .map_err(|error| FiniteRootTerminalFailure::Commit {
                cause: error.cause,
                proof: error.proof,
            })
    }

    pub(crate) fn finish_evidence_only(
        &mut self,
        final_progress: FinalProgress<PP, U>,
    ) -> PreparedResult<O, P, PP, U, D, F, CB, EB> {
        let Some(operation) = self.loan() else {
            return Err(self.ownership_failure());
        };
        let prepared = operation
            .prepare_evidence_only_deferred(final_progress)
            .map_err(FiniteRootTerminalFailure::Preparation)?;
        self.finish_deferred(prepared)
    }

    pub(crate) fn finish_minimal(
        &mut self,
        cause: PreparationFailureCode,
    ) -> PreparedResult<O, P, PP, U, D, F, CB, EB> {
        let Some(operation) = self.loan() else {
            return Err(self.ownership_failure());
        };
        let prepared = operation
            .prepare_minimal_fallback_deferred(cause)
            .map_err(FiniteRootTerminalFailure::Preparation)?;
        self.finish_deferred(prepared)
    }

    pub(crate) fn finish_assembly(
        &mut self,
        request: RetainedTerminalRequest<CB, EB, PP, U>,
    ) -> PreparedWithControlResult<O, P, PP, U, D, F, CB, EB> {
        let strict_success = matches!(&request, RetainedTerminalRequest::FrozenSuccess { .. });
        let Some(assembly) = self.assembly_loan() else {
            return Err(self.ownership_failure());
        };
        let classification = assembly
            .try_classify_terminal_preparation(request)
            .map_err(FiniteRootTerminalFailure::Preparation)?;
        let classification = if strict_success {
            classification
                .require_success()
                .map_err(FiniteRootTerminalFailure::Preparation)?
        } else {
            classification
        };
        let (prepared, control) = classification
            .finish_deferred()
            .map_err(FiniteRootTerminalFailure::Preparation)?;
        Ok((self.finish_deferred(prepared)?, control))
    }
}
