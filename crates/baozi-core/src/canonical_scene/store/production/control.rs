#![allow(
    clippy::result_large_err,
    reason = "operation failures retain bounded store evidence inline without a failure-path allocation"
)]

use std::error::Error;

use baozi_operation::__canonical_activation::{CanonicalReservation, CanonicalSemanticWorkSession};
use baozi_operation::{
    ControlCause, FreshStoreLineage, OperationOwner, ProgressPhase, ProgressUnit,
    RetainedStoreOperation, RetainedStoreReservation, StoreDimensionKey, StorePhaseKey,
    StoreRuntimeError,
};
use thiserror::Error;

use crate::SceneValidationWork;

/// Core-owned control surface for one canonical authoring preflight.
///
/// Each work unit wraps the semantic action it accounts for. The scoped activation implementation
/// therefore admits attempted work immediately before that action and advances consumed work only
/// after it returns normally. The trait remains private to canonical Scene construction and
/// intentionally exposes no raw ledger key, progress channel, or terminal owner.
pub(in crate::canonical_scene::store) trait CanonicalAuthoringControl {
    type Error: Error + Send + Sync + 'static;
    type Reservation;

    fn poll(&mut self) -> Result<(), Self::Error>;

    fn run_unit<R>(
        &mut self,
        work: SceneValidationWork,
        action: impl FnOnce() -> R,
    ) -> Result<R, Self::Error>;

    fn reserve_scratch(&mut self, bytes: u64) -> Result<Self::Reservation, Self::Error>;

    fn validate_scratch(&mut self, reservation: &Self::Reservation) -> Result<(), Self::Error>;

    fn resize_scratch(
        &mut self,
        reservation: &mut Self::Reservation,
        required_bytes: u64,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum OperationAuthoringControlError {
    #[error("canonical node authoring was cancelled or reached its deadline")]
    Control(#[source] ControlCause),
    #[error("canonical node authoring accounting failed")]
    Store(#[source] StoreRuntimeError),
}

pub(in crate::canonical_scene::store) struct OperationAuthoringControl<
    'operation,
    O,
    PP,
    U,
    L = FreshStoreLineage,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: &'operation mut RetainedStoreOperation<O, PP, U, L>,
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
}

/// Action-scoped adapter used by the import-bound activation transaction.
///
/// Explicit validator polls delegate to the operation-owned checkpoint cadence. Validators retain
/// their scheduling shape without introducing a second work counter or permitting scratch work to
/// cross a pending cancellation checkpoint.
#[allow(
    dead_code,
    reason = "consumed by the complete-batch scoped activation cutover in U4e"
)]
pub(in crate::canonical_scene::store) struct ScopedAuthoringControl<
    'session,
    'borrow,
    'scope,
    O,
    PP,
    U,
    L = FreshStoreLineage,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    session: &'session mut CanonicalSemanticWorkSession<'borrow, 'scope, O, PP, U, L>,
}

impl<'session, 'borrow, 'scope, O, PP, U, L>
    ScopedAuthoringControl<'session, 'borrow, 'scope, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) const fn new(
        session: &'session mut CanonicalSemanticWorkSession<'borrow, 'scope, O, PP, U, L>,
    ) -> Self {
        Self { session }
    }
}

impl<'operation, O, PP, U, L> OperationAuthoringControl<'operation, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) const fn new(
        operation: &'operation mut RetainedStoreOperation<O, PP, U, L>,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            operation,
            phase,
            dimension,
        }
    }

    fn runtime_error(error: StoreRuntimeError) -> OperationAuthoringControlError {
        match error {
            StoreRuntimeError::Control(cause) => OperationAuthoringControlError::Control(cause),
            cause => OperationAuthoringControlError::Store(cause),
        }
    }

    fn store_error(error: StoreRuntimeError) -> OperationAuthoringControlError {
        OperationAuthoringControlError::Store(error)
    }
}

impl<O, PP, U, L> CanonicalAuthoringControl for OperationAuthoringControl<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    type Error = OperationAuthoringControlError;
    type Reservation = RetainedStoreReservation<O>;

    fn poll(&mut self) -> Result<(), Self::Error> {
        self.operation
            .producer_host()
            .poll()
            .map_err(Self::runtime_error)
    }

    fn run_unit<R>(
        &mut self,
        _work: SceneValidationWork,
        action: impl FnOnce() -> R,
    ) -> Result<R, Self::Error> {
        // This adapter exists only for the legacy construction rail. Keep its evidence truthful
        // while U4e replaces the rail atomically: admission records attempted work, and completion
        // becomes visible only after the corresponding semantic action returns normally.
        let mut host = self.operation.producer_host();
        let mut observed = host
            .try_begin_observed_work([(self.dimension, 1)])
            .map_err(Self::runtime_error)?;
        let result = action();
        if let Err(cause) = observed.complete([1]) {
            observed.finish_partial();
            return Err(Self::runtime_error(cause));
        }
        observed.finish().map_err(Self::runtime_error)?;
        Ok(result)
    }

    fn reserve_scratch(&mut self, bytes: u64) -> Result<Self::Reservation, Self::Error> {
        self.operation
            .control_checkpoint()
            .reserve(self.phase, self.dimension, bytes)
            .map_err(Self::store_error)
    }

    fn validate_scratch(&mut self, reservation: &Self::Reservation) -> Result<(), Self::Error> {
        self.operation
            .control_checkpoint()
            .validate_live_owner(reservation)
            .map_err(Self::store_error)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut Self::Reservation,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        self.operation
            .control_checkpoint()
            .resize(reservation, required_bytes)
            .map_err(Self::store_error)
    }
}

impl<'session, 'borrow, 'scope, O, PP, U, L> CanonicalAuthoringControl
    for ScopedAuthoringControl<'session, 'borrow, 'scope, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    type Error = StoreRuntimeError;
    type Reservation = CanonicalReservation<'scope, O>;

    fn poll(&mut self) -> Result<(), Self::Error> {
        self.session.poll_if_due()
    }

    fn run_unit<R>(
        &mut self,
        _work: SceneValidationWork,
        action: impl FnOnce() -> R,
    ) -> Result<R, Self::Error> {
        self.session.run_unit(action)
    }

    fn reserve_scratch(&mut self, bytes: u64) -> Result<Self::Reservation, Self::Error> {
        self.session.reserve(bytes)
    }

    fn validate_scratch(&mut self, reservation: &Self::Reservation) -> Result<(), Self::Error> {
        self.session.validate_live_owner(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut Self::Reservation,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        self.session.resize(reservation, required_bytes)
    }
}
