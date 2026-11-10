//! The finite terminal boundary consumes original fields only after every shorter loan ends.

use super::attached::select_terminal_batches;
use super::*;
use crate::ledger::TypedTerminalLayout;
use crate::ledger::finite::FiniteTypedFreezeDetachError;
use crate::retained::ownership::FiniteFallbackPublicationBaseline;
use crate::retained::physical::FiniteAccountBoundJointCommit;
use crate::retained::terminal_physical::TerminalPhysicalSlots;
use crate::retained::{DeferredAttachedTerminal, FiniteTerminalPhysicalState};

pub(in crate::retained) struct FiniteTerminalRootFields<'inv, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(super) common: &'inv mut Option<FiniteOperationInvocation<O, P, PP, U, D>>,
    pub(super) baseline: &'inv mut FiniteFallbackPublicationBaseline<O, P, D, F>,
    pub(super) terminal: &'inv mut Option<FiniteTerminalPhysicalState<F, CB, EB>>,
    pub(super) assembly:
        Option<&'inv mut super::super::super::PublicationAssemblyFields<O, P, D, CB, EB>>,
    pub(super) contained: &'inv mut bool,
}

type FiniteTerminalCommit<O, P, D, F, CB, EB> =
    FiniteAccountBoundJointCommit<FiniteTerminalPhysicalState<F, CB, EB>, O, P, D>;

type FiniteTerminalDetachResult<O, P, PP, U, D, F, CB, EB> = Result<
    Result<FiniteTerminalCommit<O, P, D, F, CB, EB>, FiniteTerminalDetachFailure<O, P, PP, U, D>>,
    AttachedTerminalStamp,
>;

/// The caller receives the original failed numerical extraction, not only its classification.
pub(in crate::retained) struct FiniteTerminalDetachFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(in crate::retained) cause: FinalizationContainmentError,
    pub(in crate::retained) residue: FiniteTypedFreezeDetachError<O, P, PP, U, D>,
}

impl<O, P, PP, U, D> fmt::Debug for FiniteTerminalDetachFailure<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FiniteTerminalDetachFailure")
            .field("cause", &self.cause)
            .field("kind", &self.residue.kind())
            .finish_non_exhaustive()
    }
}

impl<O, P, PP, U, D, F, CB, EB> FiniteTerminalRootFields<'_, O, P, PP, U, D, F, CB, EB>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(in crate::retained) fn new<'loan>(
        common: &'loan mut Option<FiniteOperationInvocation<O, P, PP, U, D>>,
        baseline: &'loan mut FiniteFallbackPublicationBaseline<O, P, D, F>,
        terminal: &'loan mut Option<FiniteTerminalPhysicalState<F, CB, EB>>,
        assembly: Option<
            &'loan mut super::super::super::PublicationAssemblyFields<O, P, D, CB, EB>,
        >,
        contained: &'loan mut bool,
    ) -> FiniteTerminalRootFields<'loan, O, P, PP, U, D, F, CB, EB> {
        FiniteTerminalRootFields {
            common,
            baseline,
            terminal,
            assembly,
            contained,
        }
    }

    pub(in crate::retained) fn install_terminal_preparation(
        &mut self,
        prepared: DeferredAttachedTerminal,
    ) -> Result<AttachedTerminalStamp, DeferredAttachedTerminal> {
        let Some(common) = self.common.as_ref() else {
            return Err(prepared);
        };
        let identity = common.coordinator.retained_ledger().as_read().identity();
        if !common
            .lifecycle
            .has_attached_stamp(&prepared.stamp, identity)
            || !matches!(
                &prepared.qualification,
                crate::retained::terminal_loan::TerminalEvidenceQualification::Finite
            )
            || !self.terminal.as_ref().is_some_and(|physical| {
                physical.disposition.is_none() && physical.graph.account.is_some()
            })
        {
            return Err(prepared);
        }
        self.terminal
            .as_mut()
            .expect("the original finite graph was checked")
            .disposition = Some(prepared.disposition);
        Ok(prepared.stamp)
    }

    fn contain_extraction_fault(&mut self) -> Option<FinalizationContainmentError> {
        *self.contained = true;
        let reason = AccountPoisonReason::OwnershipGraphUnverified;
        let cause = self
            .terminal
            .take()
            .map(|physical| physical.contain_new_fault(reason));
        if let Some(assembly) = self.assembly.as_deref_mut() {
            TerminalPhysicalSlots::<P, D, F, CB, EB> {
                fallback: None,
                evidence: None,
                canonical_table: Some(&mut assembly.canonical_table),
                evidence_table: Some(&mut assembly.evidence_table),
                account: Some(&mut assembly.account),
            }
            .quarantine_unresolved();
        }
        if let Some(account) = self.baseline.first_account() {
            let _ = account.record_poison(reason);
        }
        self.baseline.record_forgotten_physical_edges();
        self.baseline.fail_closed_after_recorded_poison();
        if let Some(assembly) = self.assembly.as_deref_mut() {
            assembly.retain_batches_fail_closed();
        }
        std::mem::forget(self.common.take());
        cause
    }

    pub(in crate::retained) fn try_detach_terminal(
        &mut self,
        stamp: AttachedTerminalStamp,
    ) -> FiniteTerminalDetachResult<O, P, PP, U, D, F, CB, EB> {
        let Some(common) = self.common.as_ref() else {
            return Err(stamp);
        };
        let identity = common.coordinator.retained_ledger().as_read().identity();
        if !common.lifecycle.has_attached_stamp(&stamp, identity)
            || self
                .terminal
                .as_ref()
                .is_none_or(|physical| physical.disposition.is_none())
        {
            return Err(stamp);
        }
        let decision = common
            .lifecycle
            .attached_decision()
            .expect("the original Ready decision was checked");
        if decision.roles.deferred_evidence_first
            || (decision.layout == TypedTerminalLayout::Fallback
                && decision.roles.deferred_canonical)
        {
            return Err(stamp);
        }
        let mut root = scopeguard::guard_on_unwind(self, |root| {
            let _ = root.contain_extraction_fault();
        });
        *root.contained = true;
        let decision = root
            .common
            .as_mut()
            .expect("the original finite invocation is installed")
            .lifecycle
            .take_attached_decision(stamp, identity)
            .expect("the original stamp was checked without an intervening callback");
        let common = scopeguard::guard(
            root.common
                .take()
                .expect("concrete extraction takes the original finite owner once"),
            std::mem::forget,
        );
        let original: &mut Self = &mut root;
        let baseline = original
            .baseline
            .terminal_batches()
            .expect("qualification retains the original finite batches");
        let batches =
            select_terminal_batches(&decision, baseline, original.assembly.as_deref_mut());
        let proof = match scopeguard::ScopeGuard::into_inner(common)
            .try_detach_typed(decision.last_completed_phase, batches)
        {
            Ok(proof) => proof,
            Err(error) => {
                let cause = root.contain_extraction_fault().expect(
                    "the original finite physical graph remains installed until detach succeeds",
                );
                return Ok(Err(FiniteTerminalDetachFailure {
                    cause,
                    residue: error,
                }));
            }
        };
        let physical = root
            .terminal
            .take()
            .expect("only successful detach transfers the physical graph");
        Ok(Ok(FiniteAccountBoundJointCommit::new(physical, proof)))
    }
}
