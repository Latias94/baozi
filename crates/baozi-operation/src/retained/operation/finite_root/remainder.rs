//! Original physical ownership after the finite common has transferred elsewhere.

use super::*;

type OptionalFiniteCommon<'loan, O, P, PP, U, D> =
    Option<&'loan mut Option<FiniteOperationInvocation<O, P, PP, U, D>>>;

pub(crate) struct FiniteInvocationRemainder<O, P, PP, U, D, F, CB, EB>
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
    terminal: Option<crate::retained::FiniteTerminalPhysicalState<F, CB, EB>>,
    assembly: crate::retained::PublicationAssemblySlot<O, P, D, CB, EB>,
    retained: Option<FiniteRetainedOperationFields<O, P, D, F, CB, EB>>,
    _started: bool,
    contained: bool,
    progress: PhantomData<fn() -> (PP, U)>,
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
    /// Transfers the original physical fields only after consuming numerical extraction.
    pub(crate) fn into_extracted_remainder(
        mut self,
    ) -> FiniteInvocationRemainder<O, P, PP, U, D, F, CB, EB> {
        assert!(
            self.common.is_none(),
            "the actual common must already be extracted"
        );
        let remainder = FiniteInvocationRemainder {
            terminal: self.terminal.take(),
            assembly: self.assembly.take(),
            retained: self.retained.take(),
            _started: self.started,
            contained: self.contained,
            progress: PhantomData,
        };
        self.started = true;
        self.contained = false;
        remainder
    }
}

impl<O, P, PP, U, D, F, CB, EB> FiniteInvocationRemainder<O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn must_retain_domain(&self) -> bool {
        self.contained
    }

    pub(crate) fn cleanup_owned_state(&mut self) {
        cleanup_finite_physical_state::<O, P, PP, U, D, F, CB, EB>(
            &mut self.terminal,
            &mut self.assembly,
            &mut self.retained,
            None,
            &mut self.contained,
        );
    }

    pub(crate) fn contain_payload_failure(&mut self) -> Option<FinalizationContainmentError> {
        // With common absent, the original root could contain only this remaining anchor.
        let account = self.terminal.as_ref()?.graph.account.as_ref()?;
        let reason = account
            .snapshot()
            .first_poison_evidence()
            .map_or(AccountPoisonReason::OwnershipGraphUnverified, |evidence| {
                evidence.reason()
            });
        let poisoned = account.record_poison(reason);
        self.contained = true;
        Some(FinalizationContainmentError::new(poisoned, reason))
    }

    #[cfg(test)]
    pub(crate) fn test_has_started(&self) -> bool {
        self._started
    }
}

impl<O, P, PP, U, D, F, CB, EB> Drop for FiniteInvocationRemainder<O, P, PP, U, D, F, CB, EB>
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

pub(super) fn cleanup_finite_physical_state<O, P, PP, U, D, F, CB, EB>(
    terminal: &mut Option<crate::retained::FiniteTerminalPhysicalState<F, CB, EB>>,
    assembly: &mut crate::retained::PublicationAssemblySlot<O, P, D, CB, EB>,
    retained: &mut Option<FiniteRetainedOperationFields<O, P, D, F, CB, EB>>,
    common: OptionalFiniteCommon<'_, O, P, PP, U, D>,
    contained: &mut bool,
) -> bool
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
    if let Some(physical) = terminal.take() {
        *contained |= !physical.graph.is_empty();
        drop(physical);
    }
    let retain = cleanup_retained_root_graph(
        retained.as_mut().map(RetainedOperationFieldsLoan::from),
        assembly,
        TerminalPhysicalSlot::Finite(terminal),
        common.map(InvocationSlot::Finite),
        contained,
    );
    if retain {
        std::mem::forget(assembly.take());
        std::mem::forget(retained.take());
    } else {
        drop(retained.take());
    }
    retain
}
