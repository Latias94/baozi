//! Concrete finite extraction keeps fixed facts separate from its actual live cover.

use super::*;
use crate::ledger::attached::{
    DetachedTypedBatchSlots, SharedStorageRecovery, SharedStorageRecoveryError,
};
use crate::retained::operation::FiniteOperationInvocation;

/// Only the finite terminal owner carries its original inline or promoted storage.
enum FiniteDetachedStorage<P, D> {
    Inline(FiniteStorageState<P, D>),
    Promoted(LedgerState<P, D>),
}

impl<P, D> FiniteDetachedStorage<P, D> {
    fn view_mut(&mut self) -> StorageViewMut<'_, P, D> {
        match self {
            Self::Inline(state) => state.view_mut(),
            Self::Promoted(state) => state.view_mut(),
        }
    }

    fn retire_evidence(&mut self) {
        match self {
            Self::Inline(state) => {
                state.header.current_administrative_bytes = 0;
                state.header.publication = PublicationState::Published;
            }
            Self::Promoted(state) => state.retire_terminal_evidence(),
        }
    }
}

struct FiniteDetachedLiveCover<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    storage: FiniteDetachedStorage<P, D>,
    batches: DetachedTypedBatches<O, P, D>,
}

/// Fixed projection and actual uncommitted accounting, consumed only under account eligibility.
pub(crate) struct FiniteDetachedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    projection: Option<FiniteLedgerSnapshot<P, D>>,
    cover: Option<FiniteDetachedLiveCover<O, P, D>>,
}

/// A failed extraction owns every original numerical field, including recovered promoted state.
pub(crate) struct FiniteTypedFreezeDetachError<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    kind: TypedFreezeDetachErrorKind,
    projection: Option<FiniteLedgerSnapshot<P, D>>,
    invocation: Option<FiniteOperationInvocation<O, P, PP, U, D>>,
    recovered: Option<SharedStorageRecovery<P, D>>,
    batches: Option<DetachedTypedBatches<O, P, D>>,
}

struct FiniteDetachGuard<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    invocation: Option<FiniteOperationInvocation<O, P, PP, U, D>>,
    projection: Option<FiniteLedgerSnapshot<P, D>>,
    inline: Option<FiniteStorageState<P, D>>,
    recovered: Option<SharedStorageRecovery<P, D>>,
    batches: DetachedTypedBatchSlots<O, P, D>,
}

impl<O, P, PP, U, D> FiniteDetachGuard<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn new(invocation: FiniteOperationInvocation<O, P, PP, U, D>) -> Self {
        Self {
            invocation: Some(invocation),
            projection: None,
            inline: None,
            recovered: None,
            batches: DetachedTypedBatchSlots::new(),
        }
    }

    fn into_error(
        mut self,
        kind: TypedFreezeDetachErrorKind,
    ) -> FiniteTypedFreezeDetachError<O, P, PP, U, D> {
        assert!(self.projection.is_some() && self.invocation.is_some());
        let batches = self.batches.take_batches();
        FiniteTypedFreezeDetachError {
            kind,
            projection: self.projection.take(),
            invocation: self.invocation.take(),
            recovered: self.recovered.take(),
            batches: Some(batches),
        }
    }

    fn into_proof(mut self) -> FiniteDetachedTypedFreezeProof<O, P, D> {
        assert!(
            self.projection.is_some()
                && (self.inline.is_some()
                    || self
                        .recovered
                        .as_ref()
                        .is_some_and(|state| state.state().is_some()))
        );
        // All required state is still guarded when the batch transfer checks its required slots.
        let batches = self.batches.take_batches();
        let storage = match self.inline.take() {
            Some(state) => FiniteDetachedStorage::Inline(state),
            None => FiniteDetachedStorage::Promoted(
                self.recovered
                    .as_mut()
                    .expect("the original promoted state was checked before owner transfer")
                    .take_state()
                    .expect("the original promoted state was checked before owner transfer"),
            ),
        };
        FiniteDetachedTypedFreezeProof {
            projection: self.projection.take(),
            cover: Some(FiniteDetachedLiveCover { storage, batches }),
        }
    }
}

impl<O, P, PP, U, D> Drop for FiniteDetachGuard<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        self.projection = None;
        std::mem::forget(self.invocation.take());
        self.inline = None;
        // The recovery and batch fields retain their own original owners on unwind.
    }
}

type FiniteDetachResult<O, P, PP, U, D> =
    Result<FiniteDetachedTypedFreezeProof<O, P, D>, FiniteTypedFreezeDetachError<O, P, PP, U, D>>;

impl<O, P, PP, U, D> FiniteOperationInvocation<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    #[cfg(test)]
    pub(crate) fn test_panic_after_next_canonical_detach(&mut self) {
        crate::ledger::attached::arm_partial_detach_panic_for_test();
    }

    /// The concrete root has already ended qualification loans and consumed its Ready decision.
    #[expect(
        clippy::result_large_err,
        reason = "finite rejection returns its actual fixed owner and all detached backing"
    )]
    pub(crate) fn try_detach_typed(
        self,
        last_completed_phase: Option<P>,
        batches: TypedTerminalBatches<'_, O, P, D>,
    ) -> FiniteDetachResult<O, P, PP, U, D> {
        let mut guard = FiniteDetachGuard::new(self);
        match guard.prepare(last_completed_phase, batches) {
            Ok(()) => Ok(guard.into_proof()),
            Err(kind) => Err(guard.into_error(kind)),
        }
    }
}

impl<O, P, PP, U, D> FiniteDetachGuard<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn prepare(
        &mut self,
        last_completed_phase: Option<P>,
        batches: TypedTerminalBatches<'_, O, P, D>,
    ) -> Result<(), TypedFreezeDetachErrorKind> {
        let common = self
            .invocation
            .as_mut()
            .expect("the original finite invocation stays guarded");
        let known = common.known_administrative_bytes();
        let raw = common.terminal_ledger_mut();
        let identity = raw.identity;
        self.projection = Some(raw.take_typed_terminal_projection(
            last_completed_phase,
            batches.canonical,
            batches.evidence,
            known,
        ));
        let unexpected_discard =
            batches.discard_canonical.is_some() || batches.discard_evidence.is_some();
        self.batches
            .detach_in_place(batches, raw.storage.shared(), identity);
        if unexpected_discard {
            return Err(TypedFreezeDetachErrorKind::UnexpectedDiscardOwner);
        }
        if raw
            .storage
            .shared()
            .is_some_and(|shared| shared.is_poisoned())
        {
            return Err(TypedFreezeDetachErrorKind::PoisonedLedger);
        }
        match std::mem::replace(&mut raw.storage, FiniteStorage::Inline(None)) {
            FiniteStorage::Inline(state) => {
                self.inline = state;
                assert!(
                    self.inline.is_some(),
                    "the actual inline state stays installed"
                );
            }
            FiniteStorage::Shared(shared) => {
                self.recovered = Some(SharedStorageRecovery::new(shared));
                let recovery = self
                    .recovered
                    .as_mut()
                    .expect("the original shared shell stays guarded");
                match recovery.recover() {
                    Ok(()) => {}
                    Err(SharedStorageRecoveryError::SharedOwner) => {
                        raw.storage = FiniteStorage::Shared(
                            recovery
                                .take_shared()
                                .expect("refused recovery retains the original shared shell"),
                        );
                        return Err(TypedFreezeDetachErrorKind::SharedLedgerOwner);
                    }
                    Err(SharedStorageRecoveryError::Poisoned) => {
                        // Its actual recovered promoted state cannot be installed as fixed inline
                        // rows. The owned error keeps that same state beside the original owner.
                        return Err(TypedFreezeDetachErrorKind::PoisonedLedger);
                    }
                }
            }
        }
        // Fixed source catalogs and progress own no heap backing. Control owners were closed
        // before Ready; only the original live storage and detached batches remain.
        drop(self.invocation.take());
        Ok(())
    }
}

impl<O, P, D> FiniteDetachedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    #[cfg(test)]
    pub(crate) fn terminal_projection(&self) -> &FiniteLedgerSnapshot<P, D> {
        self.projection
            .as_ref()
            .expect("the finite proof owns its original fixed projection")
    }

    fn commit_inner(&mut self) -> (FiniteLedgerSnapshot<P, D>, (u64, u64, u64, bool)) {
        let cover = self
            .cover
            .as_mut()
            .expect("finite commit keeps its live cover installed");
        cover
            .batches
            .commit_account_bound(&mut cover.storage.view_mut());
        let projection = self
            .projection
            .take()
            .expect("finite commit consumes its fixed projection once");
        cover.storage.retire_evidence();
        let state = cover.storage.view_mut();
        let terminal = (
            state.current_live_bytes,
            state.active_reservations,
            state.current_administrative_bytes,
            state.publication == PublicationState::Published,
        );
        drop(self.cover.take());
        (projection, terminal)
    }

    pub(crate) fn commit_account_bound(
        mut self,
        _permit: crate::retained::DetachedLedgerCommitPermit<'_>,
    ) -> FiniteLedgerSnapshot<P, D> {
        self.commit_inner().0
    }

    pub(crate) fn retain_fail_closed(self) {
        drop(self);
    }

    #[cfg(test)]
    pub(crate) fn commit_account_bound_for_test(
        &mut self,
    ) -> (FiniteLedgerSnapshot<P, D>, DetachedLedgerTerminalScalars) {
        let (
            snapshot,
            (current_live_bytes, active_reservations, current_administrative_bytes, published),
        ) = self.commit_inner();
        (
            snapshot,
            DetachedLedgerTerminalScalars {
                current_live_bytes,
                active_reservations,
                current_administrative_bytes,
                published,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn test_arm_commit_panic() {
        crate::ledger::attached::arm_commit_panic_for_test();
    }

    #[cfg(test)]
    pub(crate) fn test_shared_shell_bytes() -> u64 {
        size_of::<Mutex<Option<LedgerState<P, D>>>>() as u64
    }

    #[cfg(test)]
    pub(crate) fn test_cover_observation(&mut self) -> Option<(u64, u64, u64, usize)> {
        let cover = self.cover.as_mut()?;
        let state = cover.storage.view_mut();
        Some((
            state.current_live_bytes,
            state.active_reservations,
            state.current_administrative_bytes,
            state.dimensions.as_ptr() as usize,
        ))
    }
}

impl<O, P, D> Drop for FiniteDetachedTypedFreezeProof<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        self.projection = None;
        std::mem::forget(self.cover.take());
    }
}

impl<O, P, PP, U, D> FiniteTypedFreezeDetachError<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    pub(crate) fn kind(&self) -> TypedFreezeDetachErrorKind {
        self.kind
    }

    #[cfg(test)]
    pub(crate) fn test_invocation(&self) -> &FiniteOperationInvocation<O, P, PP, U, D> {
        self.invocation
            .as_ref()
            .expect("finite detach rejection retains its original invocation")
    }
}

impl<O, P, PP, U, D> Drop for FiniteTypedFreezeDetachError<O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        self.projection = None;
        std::mem::forget(self.invocation.take());
        std::mem::forget(self.batches.take());
        // A recovered promoted state is retained by its original recovery guard.
        drop(self.recovered.take());
    }
}
