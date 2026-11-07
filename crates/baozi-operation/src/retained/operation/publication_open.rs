//! Open installed publication fields once, then bind the original concrete permission.

use super::*;
use crate::ledger::LedgerIdentity;
use crate::retained::{
    RetainedPublicationAssemblyBorrow, RetainedPublicationAssemblyLoan, RetainedSetId,
};

/// References and physical/account ownership remain in the original root.
pub(crate) struct RetainedAssemblyOpened {
    identity: LedgerIdentity,
    set_id: RetainedSetId,
}

impl RetainedAssemblyOpened {
    pub(in crate::retained) fn matches(
        &self,
        identity: LedgerIdentity,
        set_id: RetainedSetId,
    ) -> bool {
        self.identity == identity && self.set_id == set_id
    }
}

/// The original open failure without the shorter operation reference bundle.
pub(crate) struct RetainedPublicationBorrowFailure<D> {
    identity: LedgerIdentity,
    cause: RetainedWriterError<D>,
    disposition: RetainedPublicationOpenDisposition,
}

impl<D> RetainedPublicationBorrowFailure<D> {
    pub(in crate::retained) fn belongs_to(&self, identity: LedgerIdentity) -> bool {
        self.identity == identity
    }

    pub(crate) fn cause(&self) -> &RetainedWriterError<D> {
        &self.cause
    }

    pub(crate) fn is_fallback_only(&self) -> bool {
        self.disposition == RetainedPublicationOpenDisposition::FallbackOnly
    }

    pub(crate) fn is_fail_closed(&self) -> bool {
        self.disposition == RetainedPublicationOpenDisposition::FailClosed
    }
}

type BorrowedOpenResult<'op, O, P, PP, U, D, F, CB, EB> = Result<
    RetainedPublicationAssemblyBorrow<'op, O, P, PP, U, D, F, CB, EB>,
    RetainedPublicationBorrowFailure<D>,
>;
type OpenBindingResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    super::super::RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
    (
        RetainedOperation<'inv, O, P, PP, U, D, F, CB, EB>,
        RetainedAssemblyOpened,
    ),
>;
type OpenFailureBindingResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    RetainedPublicationOpenError<RetainedOperation<'inv, O, P, PP, U, D, F, CB, EB>, D>,
    (
        RetainedOperation<'inv, O, P, PP, U, D, F, CB, EB>,
        RetainedPublicationBorrowFailure<D>,
    ),
>;

type BorrowedOpenFailureParts<'op, O, P, PP, U, D, F, CB, EB> = (
    RetainedOperationLoan<'op, O, P, PP, U, D, F, CB, EB>,
    RetainedPublicationBorrowFailure<D>,
);

impl<'op, O, P, PP, U, D, F, CB, EB>
    RetainedPublicationOpenError<RetainedOperationLoan<'op, O, P, PP, U, D, F, CB, EB>, D>
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
    pub(crate) fn into_borrowed_failure(
        self,
    ) -> BorrowedOpenFailureParts<'op, O, P, PP, U, D, F, CB, EB> {
        let Self {
            operation,
            cause,
            disposition,
        } = self;
        let identity = operation.common.as_read().identity();
        (
            operation,
            RetainedPublicationBorrowFailure {
                identity,
                cause,
                disposition,
            },
        )
    }
}

impl<'op, O, P, PP, U, D, F, CB, EB> RetainedOperationLoan<'op, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn open_publication_borrow(
        self,
        plan: RetainedPublicationPlan<P, D>,
    ) -> BorrowedOpenResult<'op, O, P, PP, U, D, F, CB, EB> {
        self.try_publication_assembly(plan)
            .map(RetainedPublicationAssemblyBorrow::from_loan)
            .map_err(|error| {
                let (operation, failure) = error.into_borrowed_failure();
                drop(operation);
                failure
            })
    }
}

impl<O, P, PP, U, D, F, CB, EB> RetainedPublicationAssemblyBorrow<'_, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn opened_stamp(&self) -> Option<RetainedAssemblyOpened> {
        Some(RetainedAssemblyOpened {
            identity: self.loan().operation.as_ref()?.common.as_read().identity(),
            set_id: self.loan().fields.set_id,
        })
    }
}

impl<'inv, O, P, PP, U, D, F, CB, EB> RetainedOperation<'inv, O, P, PP, U, D, F, CB, EB>
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
    fn has_original_dynamic_open_fields(&self, identity: LedgerIdentity) -> bool {
        matches!(&self.loan.common.common, InvocationSlot::Dynamic(_))
            && self.loan.common.common.is_installed()
            && matches!(
                &self.loan.retained.baseline,
                FallbackPublicationBaselineLoan::Dynamic(_)
            )
            && self.loan.retained.baseline.first_account().is_some()
            && matches!(&self.loan.terminal, TerminalPhysicalSlot::Dynamic(slot) if slot.is_none())
            && !*self.loan.common.contained
            && !self.loan.common.terminal_is_outstanding()
            && self.loan.common.as_read().identity() == identity
    }

    /// Reuses the original long-lived fields after every short ordinary loan has ended.
    pub(crate) fn bind_opened_assembly(
        mut self,
        opened: RetainedAssemblyOpened,
    ) -> OpenBindingResult<'inv, O, P, PP, U, D, F, CB, EB> {
        if !self.has_original_dynamic_open_fields(opened.identity)
            || !self
                .loan
                .assembly
                .as_ref()
                .and_then(|slot| slot.as_ref())
                .is_some_and(|fields| fields.set_id == opened.set_id)
        {
            return Err((self, opened));
        }
        let fields =
            self.loan.assembly.take().and_then(Option::as_mut).expect(
                "the same installed original assembly was checked before reference transfer",
            );
        Ok(super::super::RetainedPublicationAssembly::from_loan(
            RetainedPublicationAssemblyLoan {
                fields: fields.into(),
                operation: Some(self.loan),
            },
        ))
    }

    pub(crate) fn bind_publication_open_failure(
        self,
        failure: RetainedPublicationBorrowFailure<D>,
    ) -> OpenFailureBindingResult<'inv, O, P, PP, U, D, F, CB, EB> {
        if !self.has_original_dynamic_open_fields(failure.identity)
            || !self
                .loan
                .assembly
                .as_ref()
                .is_some_and(|slot| slot.is_none())
        {
            return Err((self, failure));
        }
        Ok(RetainedPublicationOpenError {
            operation: self,
            cause: failure.cause,
            disposition: failure.disposition,
        })
    }
}
