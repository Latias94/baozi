#![allow(
    clippy::result_large_err,
    reason = "operation failures retain bounded store evidence inline without a failure-path allocation"
)]

use baozi_operation::{
    ControlCause, OperationOwner, ProgressPhase, ProgressUnit, RetainedStoreControlCheckpoint,
    RetainedStoreReservation, StoreDimensionKey, StorePhaseKey, StoreRuntimeError,
};
use thiserror::Error;

use crate::{SceneReservationKind, SceneValidationControl, SceneValidationWork};

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum OperationAuthoringControlError {
    #[error("canonical node authoring was cancelled or reached its deadline")]
    Control(#[source] ControlCause),
    #[error("canonical node authoring accounting failed")]
    Store(#[source] StoreRuntimeError),
    #[error("canonical node authoring cannot reserve {kind:?} storage")]
    UnsupportedReservationKind { kind: SceneReservationKind },
    #[error("canonical node authoring cannot transition {from:?} storage to {to:?}")]
    UnsupportedReservationTransition {
        from: SceneReservationKind,
        to: SceneReservationKind,
    },
}

pub(in crate::canonical_scene::store) struct OperationAuthoringControl<'checkpoint, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    checkpoint: RetainedStoreControlCheckpoint<'checkpoint, O, PP, U>,
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
}

impl<'checkpoint, O, PP, U> OperationAuthoringControl<'checkpoint, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) const fn new(
        checkpoint: RetainedStoreControlCheckpoint<'checkpoint, O, PP, U>,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            checkpoint,
            phase,
            dimension,
        }
    }

    fn require_scratch(kind: SceneReservationKind) -> Result<(), OperationAuthoringControlError> {
        if kind == SceneReservationKind::ValidationScratch {
            Ok(())
        } else {
            Err(OperationAuthoringControlError::UnsupportedReservationKind { kind })
        }
    }

    fn store_error(error: StoreRuntimeError) -> OperationAuthoringControlError {
        OperationAuthoringControlError::Store(error)
    }
}

impl<O, PP, U> SceneValidationControl for OperationAuthoringControl<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    type Error = OperationAuthoringControlError;
    type Reservation = RetainedStoreReservation<O>;

    fn poll(&mut self) -> Result<(), Self::Error> {
        self.checkpoint.poll().map_err(|error| match error {
            StoreRuntimeError::Control(cause) => OperationAuthoringControlError::Control(cause),
            cause => OperationAuthoringControlError::Store(cause),
        })
    }

    fn charge_work(&mut self, _work: SceneValidationWork, amount: u64) -> Result<(), Self::Error> {
        self.checkpoint
            .charge_observed(self.dimension, amount)
            .map_err(Self::store_error)
    }

    fn reserve(
        &mut self,
        kind: SceneReservationKind,
        bytes: u64,
    ) -> Result<Self::Reservation, Self::Error> {
        Self::require_scratch(kind)?;
        self.checkpoint
            .reserve(self.phase, self.dimension, bytes)
            .map_err(Self::store_error)
    }

    fn validate_reservation(
        &mut self,
        reservation: &Self::Reservation,
        kind: SceneReservationKind,
    ) -> Result<(), Self::Error> {
        Self::require_scratch(kind)?;
        self.checkpoint
            .validate_live_owner(reservation)
            .map_err(Self::store_error)
    }

    fn resize_reservation(
        &mut self,
        reservation: &mut Self::Reservation,
        kind: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        Self::require_scratch(kind)?;
        self.checkpoint
            .resize(reservation, required_bytes)
            .map_err(Self::store_error)
    }

    fn transition_reservation(
        &mut self,
        _reservation: &mut Self::Reservation,
        from: SceneReservationKind,
        to: SceneReservationKind,
        _required_bytes: u64,
    ) -> Result<(), Self::Error> {
        Err(OperationAuthoringControlError::UnsupportedReservationTransition { from, to })
    }
}
