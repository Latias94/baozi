#![allow(
    clippy::result_large_err,
    reason = "operation failures retain bounded store evidence inline without a failure-path allocation"
)]

use std::error::Error;

use baozi_operation::__retained_store_integration::activation::{
    CanonicalReservation, CanonicalSemanticWorkSession,
};
use baozi_operation::__retained_store_integration::authority::FreshStoreLineage;
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};

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
        action: impl FnMut() -> R,
    ) -> Result<R, Self::Error>;

    fn reserve_scratch(&mut self, bytes: u64) -> Result<Self::Reservation, Self::Error>;

    fn validate_scratch(&mut self, reservation: &Self::Reservation) -> Result<(), Self::Error>;

    fn resize_scratch(
        &mut self,
        reservation: &mut Self::Reservation,
        required_bytes: u64,
    ) -> Result<(), Self::Error>;
}

/// Action-scoped adapter used by the import-bound activation transaction.
///
/// Explicit validator polls delegate to the operation-owned checkpoint cadence. Validators retain
/// their scheduling shape without introducing a second work counter or permitting scratch work to
/// cross a pending cancellation checkpoint.
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
        action: impl FnMut() -> R,
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
