/// Final capability for one exact canonical state restored after retained-body cleanup.
///
/// Terminal transitions are available only for their concrete state families. The underlying
/// state cannot be extracted or replaced through the public API.
///
/// A restored candidate may issue its final permit:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalCandidateStoreSession, CanonicalCauseSlot, CanonicalFinal,
///     CanonicalFreezeInterruption,
/// };
/// use baozi_operation::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit};
/// fn finish<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     candidate: CanonicalFinal<
///         CanonicalCandidateStoreSession<'inv, O, P, U, (), (), (), (), (), ()>>,
///     slot: CanonicalCauseSlot<'inv, CanonicalFreezeInterruption>,
///     progress: FinalProgress<P, U>,
/// ) {
///     let _ = candidate.output();
///     let _ = candidate.try_issue_freeze_permit(slot, progress);
/// }
/// ```
///
/// Possessing a candidate and a separately supplied slot inside the body does not grant finish:
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalCandidateStoreSession, CanonicalCauseSlot, CanonicalFinal,
///     CanonicalFreezeInterruption,
/// };
/// use baozi_operation::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit};
/// fn finish<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     candidate: CanonicalCandidateStoreSession<'inv, O, P, U, (), (), (), (), (), ()>,
///     slot: CanonicalCauseSlot<'inv, CanonicalFreezeInterruption>,
///     progress: FinalProgress<P, U>,
/// ) {
///     let _ = candidate.output();
///     let _ = candidate.try_issue_freeze_permit(slot, progress);
/// }
/// ```
///
/// A caller cannot turn a body state into a final capability:
///
/// ```compile_fail,E0451
/// use baozi_operation::__retained_store_integration::activation::CanonicalFinal;
/// fn forge<S>(state: S) -> CanonicalFinal<S> {
///     CanonicalFinal { state }
/// }
/// ```
///
/// Nor can it extract a final state for replacement with another body state:
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::CanonicalFinal;
/// fn extract<S>(state: CanonicalFinal<S>) -> S {
///     state.into_inner()
/// }
/// ```
///
/// Failure builders also require the restored final capability, because building may
/// internally select and prepare fallback:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalFinal, NeverActivatedStoreSession, RejectedCanonicalStoreSession,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn build<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     never: CanonicalFinal<NeverActivatedStoreSession<'inv, O, P, U, ()>>,
///     rejected: CanonicalFinal<RejectedCanonicalStoreSession<'inv, O, P, U, ()>>,
/// ) {
///     let _ = never.build_failure_evidence::<(), ()>(|lane| lane.abort());
///     let _ = rejected.build_failure_evidence::<(), ()>(|lane| lane.abort());
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::{
///     NeverActivatedStoreSession,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn build<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     never: NeverActivatedStoreSession<'inv, O, P, U, ()>,
/// ) {
///     let _ = never.build_failure_evidence::<(), ()>(|lane| lane.abort());
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::{
///     RejectedCanonicalStoreSession,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn build<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     rejected: RejectedCanonicalStoreSession<'inv, O, P, U, ()>,
/// ) {
///     let _ = rejected.build_failure_evidence::<(), ()>(|lane| lane.abort());
/// }
/// ```
///
/// Restored failures may consume their existing fallback owner:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalFinal, CentrallyRejectedCanonicalStoreSession, InterruptedCanonicalStoreSession,
///     NeverActivatedStoreSession, PendingMinimalFallback, RejectedCanonicalStoreSession,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn prepare<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     pending: CanonicalFinal<PendingMinimalFallback<'inv, O, P, U>>,
///     never: CanonicalFinal<NeverActivatedStoreSession<'inv, O, P, U, ()>>,
///     rejected: CanonicalFinal<RejectedCanonicalStoreSession<'inv, O, P, U, ()>>,
///     central: CanonicalFinal<CentrallyRejectedCanonicalStoreSession<'inv, O, P, U, (), (), ()>>,
///     interrupted: CanonicalFinal<InterruptedCanonicalStoreSession<'inv, O, P, U, (), ()>>,
/// ) {
///     let _ = pending.prepare_minimal_fallback();
///     let _ = never.prepare_minimal_fallback();
///     let _ = rejected.prepare_minimal_fallback();
///     let _ = central.prepare_minimal_fallback();
///     let _ = interrupted.prepare_minimal_fallback();
/// }
/// ```
///
/// Each body failure state independently excludes direct fallback preparation:
///
/// ```compile_fail,E0624
/// use baozi_operation::__retained_store_integration::activation::PendingMinimalFallback;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     pending: PendingMinimalFallback<'_, O, P, U>,
/// ) {
///     let _ = pending.prepare_minimal_fallback();
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::NeverActivatedStoreSession;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     never: NeverActivatedStoreSession<'_, O, P, U, ()>,
/// ) {
///     let _ = never.prepare_minimal_fallback();
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::RejectedCanonicalStoreSession;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     rejected: RejectedCanonicalStoreSession<'_, O, P, U, ()>,
/// ) {
///     let _ = rejected.prepare_minimal_fallback();
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::CentrallyRejectedCanonicalStoreSession;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     central: CentrallyRejectedCanonicalStoreSession<'_, O, P, U, (), (), ()>,
/// ) {
///     let _ = central.prepare_minimal_fallback();
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::InterruptedCanonicalStoreSession;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     interrupted: InterruptedCanonicalStoreSession<'_, O, P, U, (), ()>,
/// ) {
///     let _ = interrupted.prepare_minimal_fallback();
/// }
/// ```
///
/// Rich-failure validation likewise starts from restored states:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalCauseSlot, CanonicalFinal, CentrallyRejectedCanonicalStoreSession,
///     InterruptedCanonicalStoreSession,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};
/// fn validate<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     central: CanonicalFinal<CentrallyRejectedCanonicalStoreSession<'inv, O, P, U, (), (), ()>>,
///     interrupted: CanonicalFinal<InterruptedCanonicalStoreSession<'inv, O, P, U, (), ()>>,
///     central_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
///     interruption_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
/// ) {
///     let _ = central.validate_rich_failure(central_slot, |_, _| true);
///     let _ = interrupted.validate_rich_failure(interruption_slot, |_, _| true);
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalCauseSlot, CentrallyRejectedCanonicalStoreSession,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};
/// fn validate<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     central: CentrallyRejectedCanonicalStoreSession<'inv, O, P, U, (), (), ()>,
///     slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
/// ) {
///     let _ = central.validate_rich_failure(slot, |_, _| true);
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalCauseSlot, InterruptedCanonicalStoreSession,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};
/// fn validate<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     interrupted: InterruptedCanonicalStoreSession<'inv, O, P, U, (), ()>,
///     slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
/// ) {
///     let _ = interrupted.validate_rich_failure(slot, |_, _| true);
/// }
/// ```
///
/// The wrapper also prevents direct extraction or replacement with a body candidate:
///
/// ```compile_fail,E0616
/// use baozi_operation::__retained_store_integration::activation::CanonicalFinal;
/// fn extract<S>(state: CanonicalFinal<S>) -> S {
///     state.state
/// }
/// ```
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalCandidateStoreSession, CanonicalFinal,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn replace<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     destination: &mut CanonicalFinal<
///         CanonicalCandidateStoreSession<'inv, O, P, U, (), (), (), (), (), ()>>,
///     replacement: CanonicalFinal<
///         CanonicalCandidateStoreSession<'inv, O, P, U, (), (), (), (), (), ()>>,
/// ) {
///     let _previous = std::mem::replace(destination, replacement);
/// }
/// ```
///
/// ```compile_fail,E0308
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalCandidateStoreSession, CanonicalFinal,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn replace<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     destination: &mut CanonicalFinal<
///         CanonicalCandidateStoreSession<'inv, O, P, U, (), (), (), (), (), ()>>,
///     replacement: CanonicalCandidateStoreSession<'inv, O, P, U, (), (), (), (), (), ()>,
/// ) {
///     let _previous = std::mem::replace(destination, replacement);
/// }
/// ```
///
/// A bounded authoring adapter returns its candidate to the current body for disposition:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalBodyParts, CanonicalCandidateStoreSession, CanonicalInvocationHost,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn run<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
///     mut author: impl for<'call> FnMut(CanonicalBodyParts<'call, O, P, U, (), (), (), ()>)
///         -> CanonicalCandidateStoreSession<'call, O, P, U, (), (), (), (), (), ()>,
/// ) {
///     let _ = host.run_body(|parts, disposition| {
///         author(parts).request_finish(disposition).unwrap();
///     });
/// }
/// ```
///
/// The candidate itself cannot escape that higher-ranked body:
///
/// ```compile_fail,E0521
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalBodyParts, CanonicalCandidateStoreSession, CanonicalInvocationHost,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
///     mut author: impl for<'call> FnMut(CanonicalBodyParts<'call, O, P, U, (), (), (), ()>)
///         -> CanonicalCandidateStoreSession<'call, O, P, U, (), (), (), (), (), ()>,
/// ) {
///     let mut escaped = None;
///     let _ = host.run_body(|parts, _| escaped = Some(author(parts)));
///     drop(escaped);
/// }
/// ```
#[must_use = "the restored canonical state must complete its terminal transition or be dropped"]
pub struct CanonicalFinal<S> {
    pub(super) state: S,
}

impl<S> CanonicalFinal<S> {
    pub(super) const fn new(state: S) -> Self {
        Self { state }
    }
}
