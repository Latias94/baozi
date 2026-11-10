//! Ordinary reborrows leave abandonment responsibility in the original concrete owner.

use super::*;

mod decision;
pub(crate) use decision::RetainedAssemblyDecision;

type AssemblyLoan<'loan, O, P, PP, U, D, F, CB, EB> =
    RetainedPublicationAssemblyLoan<'loan, O, P, PP, U, D, F, CB, EB>;

/// A short permission over original fields, without a second abandonment obligation.
pub(crate) struct RetainedPublicationAssemblyBorrow<'loan, O, P, PP, U, D, F, CB, EB>
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
    loan: AssemblyLoan<'loan, O, P, PP, U, D, F, CB, EB>,
}

impl<'loan, O, P, PP, U, D, F, CB, EB>
    RetainedPublicationAssemblyBorrow<'loan, O, P, PP, U, D, F, CB, EB>
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
    /// Wraps a loan of installed fields; their concrete root retains final cleanup responsibility.
    pub(crate) fn from_loan(loan: AssemblyLoan<'loan, O, P, PP, U, D, F, CB, EB>) -> Self {
        Self { loan }
    }

    pub(crate) fn loan(&self) -> &AssemblyLoan<'loan, O, P, PP, U, D, F, CB, EB> {
        &self.loan
    }

    pub(crate) fn loan_mut(&mut self) -> &mut AssemblyLoan<'loan, O, P, PP, U, D, F, CB, EB> {
        &mut self.loan
    }
}

impl<O, P, PP, U, D, F, CB, EB> Drop
    for RetainedPublicationAssemblyBorrow<'_, O, P, PP, U, D, F, CB, EB>
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
        // End only the reference bundle. The original slots and their concrete abandonment
        // guard stay armed; in particular no physical owner or terminal marker is changed.
        self.loan.fields.fields = None;
        self.loan.operation = None;
    }
}

impl<O, P, PP, U, D, F, CB, EB> RetainedPublicationAssemblyLoan<'_, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn borrow_ordinary(
        &mut self,
    ) -> RetainedPublicationAssemblyBorrow<'_, O, P, PP, U, D, F, CB, EB> {
        RetainedPublicationAssemblyBorrow::from_loan(RetainedPublicationAssemblyLoan {
            fields: self
                .fields
                .fields
                .as_deref_mut()
                .expect("an ordinary reborrow requires the original installed assembly loan")
                .into(),
            operation: self.operation.as_mut().map(RetainedOperationLoan::reborrow),
        })
    }

    pub(crate) fn terminal_observation(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.operation.as_ref().and_then(|operation| {
            operation
                .common
                .as_read()
                .dimension_evidence(dimension)
                .ok()
        })
    }
}

type SuccessStorageParts<'inv, O, P, PP, U, D, F, CB, EB> = (
    RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
    RetainedSuccessClosureProof<CB, EB>,
);

impl<'inv, O, P, PP, U, D, F, CB, EB> ValidatedRetainedSuccess<'inv, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn into_storage_parts(self) -> SuccessStorageParts<'inv, O, P, PP, U, D, F, CB, EB> {
        (self.assembly, self.proof)
    }

    pub(crate) fn from_storage_parts(
        assembly: RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
        proof: RetainedSuccessClosureProof<CB, EB>,
    ) -> Self {
        Self { assembly, proof }
    }

    pub(crate) fn terminal_observation(&self, dimension: D) -> Option<DimensionSnapshot> {
        self.assembly.loan().terminal_observation(dimension)
    }
}

impl<'inv, O, P, PP, U, D, F, CB, EB> RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn bind_discarded_failure(
        self,
        cause: PreparationFailureCode,
    ) -> RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CB, EB> {
        RetainedFailureAssembly {
            assembly: self,
            fallback_cause: cause,
        }
    }
}

impl<O, P, PP, U, D, F, CB, EB> RetainedFailureAssembly<'_, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn borrow_ordinary(
        &mut self,
    ) -> RetainedPublicationAssemblyBorrow<'_, O, P, PP, U, D, F, CB, EB> {
        self.assembly.loan.borrow_ordinary()
    }
}

type FailureAssemblyParts<'inv, O, P, PP, U, D, F, CB, EB> = (
    RetainedPublicationAssembly<'inv, O, P, PP, U, D, F, CB, EB>,
    PreparationFailureCode,
);

impl<'inv, O, P, PP, U, D, F, CB, EB> RetainedFailureAssembly<'inv, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn into_failure_parts(
        self,
    ) -> FailureAssemblyParts<'inv, O, P, PP, U, D, F, CB, EB> {
        (self.assembly, self.fallback_cause)
    }
}
