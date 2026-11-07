//! Final finite freeze and joint transitions retain the original concrete terminal authority.

use super::super::super::freeze_driver;
use super::super::super::payload_owner::PayloadOrder;
use super::*;
use crate::concrete_store::PreparedFiniteStoreSuccess;
use crate::concrete_store::finite::FiniteStoreFinalFields;
use crate::retained::{RetainedSuccessClosureProof, RetainedSuccessFreezeDecision};

type FrozenPayloadOwner<'host, O, PP, U, T, Proof, CK, CT, EK, ET> = PayloadOwner<
    PayloadLoan<'host, T>,
    PayloadOwner<
        PayloadLoan<'host, Proof>,
        ValidatedFiniteStoreSuccess<'host, O, PP, U, CK, CT, EK, ET>,
    >,
>;

type FreezeFailureResult<'host, O, PP, U, Cause, EK, ET, State> = Result<
    FiniteRootedFailureEvidence<'host, O, PP, U, Cause, EK, ET, State>,
    FiniteStoreExitError<O, PP, U>,
>;

/// The exact finite decision before generation admission.
#[must_use = "every finite freeze decision retains the original terminal authority"]
pub enum FiniteCanonicalFreezePermitIssue<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Ready(FiniteCanonicalFreezePermit<'host, O, PP, U, T, Proof, CK, CT, EK, ET>),
    Interrupted(
        FiniteRootedFailureEvidence<
            'host,
            O,
            PP,
            U,
            CanonicalFreezeInterruption,
            EK,
            ET,
            FrozenFailureState,
        >,
    ),
    DestinationRejected(
        FiniteRootedFailureEvidence<
            'host,
            O,
            PP,
            U,
            CanonicalFreezeInterruption,
            EK,
            ET,
            OrdinaryFailureState,
        >,
    ),
}

/// Original finite output, proof, and storage after final operation control.
#[must_use = "the finite freeze permit must be rejected or bound to one joint commit"]
pub struct FiniteCanonicalFreezePermit<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    owner: FrozenPayloadOwner<'host, O, PP, U, T, Proof, CK, CT, EK, ET>,
    destinations: ZeroCommitDestinationFacts,
    failure_work: FailureEvidenceWorkBudget,
}

/// Invariant finite contribution paired with the existing canonical joint seal.
#[must_use = "the finite joint contribution must consume its matching seal"]
pub struct FiniteCanonicalJointCommit<'host, 'joint, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    owner: FrozenPayloadOwner<'host, O, PP, U, T, Proof, CK, CT, EK, ET>,
    destinations: ZeroCommitDestinationFacts,
    failure_work: FailureEvidenceWorkBudget,
    joint: PhantomData<fn(&'joint mut ()) -> &'joint mut ()>,
}

/// Original payloads released only after the exact finite frozen success commit.
#[must_use = "finite joint success owns the output, proof, and committed store"]
pub struct FiniteCanonicalJointSuccess<T, Proof, CK, CT, EK, ET> {
    pub output: T,
    pub proof: Proof,
    pub store: PreparedFiniteStoreSuccess<CK, CT, EK, ET>,
}

struct FiniteFreezeTransition<'host, O, PP, U, T, Proof, CK, CT, EK, ET, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fields: Option<FiniteStoreFinalFields<'host, O, PP, U>>,
    closure: Option<RetainedSuccessClosureProof<StoredBlock, StoredBlock>>,
    output: Option<PayloadLoan<'host, T>>,
    proof: Option<PayloadLoan<'host, Proof>>,
    cause: Option<Cause>,
    source: Option<PayloadLoan<'host, Cause>>,
    canonical_root: CanonicalStoreRef<CK, CT>,
    evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    destinations: Option<ZeroCommitDestinationFacts>,
    failure_work: FailureEvidenceWorkBudget,
    order: PayloadOrder,
    error: Option<StorePrepareError>,
    exit_error: Option<FiniteStoreExitError<O, PP, U>>,
    original_freeze_control: Option<ControlCause>,
}

impl<'host, O, PP, U, T, Proof, CK, CT, EK, ET, Cause>
    FiniteFreezeTransition<'host, O, PP, U, T, Proof, CK, CT, EK, ET, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(
        output: PayloadLoan<'host, T>,
        proof: PayloadLoan<'host, Proof>,
        storage: ValidatedFiniteStoreSuccess<'host, O, PP, U, CK, CT, EK, ET>,
        destinations: ZeroCommitDestinationFacts,
        failure_work: FailureEvidenceWorkBudget,
        order: PayloadOrder,
        cause: Option<Cause>,
    ) -> Self {
        let (fields, closure, canonical_root, evidence_root) = storage.into_storage_parts();
        Self {
            fields: Some(fields),
            closure: Some(closure),
            output: Some(output),
            proof: Some(proof),
            cause,
            source: None,
            canonical_root,
            evidence_root,
            destinations: Some(destinations),
            failure_work,
            order,
            error: None,
            exit_error: None,
            original_freeze_control: None,
        }
    }

    fn from_permit(
        permit: FiniteCanonicalFreezePermit<'host, O, PP, U, T, Proof, CK, CT, EK, ET>,
        cause: Option<Cause>,
    ) -> Self {
        let FiniteCanonicalFreezePermit {
            owner,
            destinations,
            failure_work,
        } = permit;
        let (output, proof_owner) = owner.into_parts();
        let (proof, storage) = proof_owner.into_parts();
        Self::new(
            output,
            proof,
            storage,
            destinations,
            failure_work,
            PayloadOrder::OutputBeforeProof,
            cause,
        )
    }

    fn output_identity(&self) -> *const () {
        self.output
            .as_ref()
            .expect("original finite output permission")
            .identity()
    }

    fn retain_error(&mut self) {
        if self.exit_error.is_none() {
            self.exit_error = Some(
                self.fields
                    .take()
                    .expect("the finite transition retains fields until terminal transfer")
                    .into_request_error(self.error.take()),
            );
        }
        self.exit_error
            .as_mut()
            .expect("the original finite error remains installed during cleanup")
            .attach_original_freeze_control(self.original_freeze_control.take());
    }

    fn record_payload_failure(&mut self) {
        if self.fields.is_some() || self.exit_error.is_some() {
            self.retain_error();
            let error = self.exit_error.as_mut().expect("original finite error");
            error.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
            error.contain_payload_failure();
        }
    }

    fn dispose_payloads(&mut self, order: PayloadOrder) -> Result<(), ()> {
        self.order = order;
        let result = freeze_driver::dispose_pair(&mut self.output, &mut self.proof, order);
        if result.is_err() {
            self.record_payload_failure();
        }
        result
    }

    fn cleanup_values(&mut self) {
        let _ = self.dispose_payloads(self.order);
        let source_failed = drop_scoped_value(self.source.take()).is_err();
        let cause_failed = drop_scoped_value(self.cause.take()).is_err();
        let closure_failed = drop_scoped_value(self.closure.take()).is_err();
        if source_failed || cause_failed || closure_failed {
            self.record_payload_failure();
        }
    }

    fn into_error(mut self) -> FiniteStoreExitError<O, PP, U> {
        self.retain_error();
        self.cleanup_values();
        self.exit_error.take().expect("original owned finite error")
    }

    fn take_storage(&mut self) -> ValidatedFiniteStoreSuccess<'host, O, PP, U, CK, CT, EK, ET> {
        ValidatedFiniteStoreSuccess::from_bound_fields(
            self.fields.take().expect("original finite terminal fields"),
            self.closure.take().expect("original finite closure proof"),
            self.canonical_root,
            self.evidence_root,
        )
    }

    fn take_owner(&mut self) -> FrozenPayloadOwner<'host, O, PP, U, T, Proof, CK, CT, EK, ET> {
        let storage = self.take_storage();
        PayloadOwner::new(
            self.output
                .take()
                .expect("original finite output permission"),
            PayloadOwner::new(
                self.proof.take().expect("original finite proof permission"),
                storage,
            ),
        )
    }

    fn into_permit(
        mut self,
    ) -> FiniteCanonicalFreezePermit<'host, O, PP, U, T, Proof, CK, CT, EK, ET> {
        FiniteCanonicalFreezePermit {
            owner: self.take_owner(),
            destinations: self
                .destinations
                .take()
                .expect("original finite destinations"),
            failure_work: self.failure_work,
        }
    }

    fn into_failure<State>(
        mut self,
        slot: CanonicalCauseSlot<'host, Cause>,
        failure_cause: PreparationFailureCode,
        forced_cause: Option<PreparationFailureCode>,
    ) -> FreezeFailureResult<'host, O, PP, U, Cause, EK, ET, State>
    where
        State: FailureLifecycleState,
    {
        if drop_scoped_value(self.closure.take()).is_err() {
            self.record_payload_failure();
            return Err(self.into_error());
        }
        match slot.place(self.cause.take().expect("original finite failure cause")) {
            Ok(source) => self.source = Some(source),
            Err(cause) => {
                self.cause = Some(cause);
                return Err(self.into_error());
            }
        }
        let storage = FiniteStoreFailureAssembly::from_bound_fields(
            self.fields.take().expect("original finite failure fields"),
            StorePublicationState::default(),
            failure_cause,
            forced_cause,
            self.evidence_root,
            None,
        )
        .with_original_freeze_control(self.original_freeze_control.take());
        Ok(CanonicalFinal::new(RootedFailureEvidence {
            owner: PayloadOwner::new(
                self.source
                    .take()
                    .expect("original finite cause permission"),
                storage,
            ),
            failure_work: self.failure_work,
            outcome_evidence_destination_dimension: self
                .destinations
                .as_ref()
                .expect("original finite destinations")
                ._outcome_evidence_dimension,
            _lifecycle: PhantomData::<State>,
        }))
    }
}

impl<O, PP, U, T, Proof, CK, CT, EK, ET, Cause> Drop
    for FiniteFreezeTransition<'_, O, PP, U, T, Proof, CK, CT, EK, ET, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.fields.is_some() || self.exit_error.is_some() {
            self.retain_error();
        }
        self.cleanup_values();
        let _ = drop_scoped_value(self.exit_error.take());
    }
}

impl<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
    CanonicalFinal<FiniteCentralCandidate<'host, O, PP, U, T, Proof, CK, CT, EK, ET>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn canonical_member_count(&self) -> u64 {
        self.state.canonical_member_count()
    }

    pub const fn outcome_evidence_member_count(&self) -> u64 {
        self.state.outcome_evidence_member_count()
    }

    /// Freezes exact destinations and final control before generation admission.
    #[allow(clippy::type_complexity)]
    pub fn try_issue_freeze_permit(
        self,
        interruption_slot: CanonicalCauseSlot<'host, CanonicalFreezeInterruption>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        FiniteCanonicalFreezePermitIssue<'host, O, PP, U, T, Proof, CK, CT, EK, ET>,
        FiniteStoreExitError<O, PP, U>,
    > {
        let FiniteCentralCandidate {
            owner,
            destinations,
            failure_work,
        } = self.state;
        let (proof, output_owner) = owner.into_parts();
        let (output, storage) = output_owner.into_parts();
        let mut transition = FiniteFreezeTransition::new(
            output,
            proof,
            storage,
            destinations,
            failure_work,
            PayloadOrder::ProofBeforeOutput,
            None::<CanonicalFreezeInterruption>,
        );
        let identity = transition.output_identity();
        if !interruption_slot.is_vacant_for(identity)
            || transition
                .proof
                .as_ref()
                .expect("original finite proof permission")
                .identity()
                != identity
        {
            return Err(transition.into_error());
        }
        let issued = {
            let mut assembly = transition
                .fields
                .as_mut()
                .expect("original finite fields remain installed during freeze")
                .assembly_borrow();
            freeze_driver::issue_freeze(
                &mut assembly,
                transition
                    .closure
                    .as_ref()
                    .expect("original finite closure"),
                transition
                    .destinations
                    .as_ref()
                    .expect("original destinations"),
                final_progress,
            )
        };
        match issued {
            Err(cause) => {
                transition.error = Some(StorePrepareError::FinalizationContainment(cause));
                Err(transition.into_error())
            }
            Ok(RetainedSuccessFreezeDecision::Ready) => {
                transition
                    .output
                    .as_ref()
                    .expect("original finite output permission")
                    .freeze_order();
                Ok(FiniteCanonicalFreezePermitIssue::Ready(
                    transition.into_permit(),
                ))
            }
            Ok(RetainedSuccessFreezeDecision::RichFailure {
                cause,
                control_cause,
            }) => {
                transition.original_freeze_control = control_cause.clone();
                transition.cause = Some(CanonicalFreezeInterruption::from_failure_code(
                    cause,
                    control_cause,
                ));
                if transition
                    .dispose_payloads(PayloadOrder::ProofBeforeOutput)
                    .is_err()
                {
                    return Err(transition.into_error());
                }
                transition
                    .into_failure::<FrozenFailureState>(interruption_slot, cause, Some(cause))
                    .map(FiniteCanonicalFreezePermitIssue::Interrupted)
            }
            Ok(RetainedSuccessFreezeDecision::DestinationRejected { cause }) => {
                transition.cause =
                    Some(CanonicalFreezeInterruption::from_failure_code(cause, None));
                if transition
                    .dispose_payloads(PayloadOrder::ProofBeforeOutput)
                    .is_err()
                {
                    return Err(transition.into_error());
                }
                transition
                    .into_failure::<OrdinaryFailureState>(interruption_slot, cause, Some(cause))
                    .map(FiniteCanonicalFreezePermitIssue::DestinationRejected)
            }
        }
    }
}

impl<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
    FiniteCanonicalFreezePermit<'host, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        self.owner.value().value()
    }

    pub const fn proof(&self) -> &Proof {
        self.owner.storage().value().value()
    }

    /// Retains the original finite error and storage after a generation contract fault.
    pub fn contain_generation_fault(self) -> FiniteStoreExitError<O, PP, U> {
        FiniteFreezeTransition::from_permit(self, None::<()>).into_error()
    }

    /// Admits the original recovery slot before any generation or result callback runs.
    #[allow(clippy::type_complexity)]
    pub fn admit_generation_rejection<E, Generation, Reject, Prepare>(
        self,
        rejection_slot: &CanonicalCauseSlot<'host, StoreCanonicalRejection<E>>,
        inputs: (Generation, Reject, Prepare),
    ) -> Result<(Self, (Generation, Reject, Prepare)), FiniteStoreExitError<O, PP, U>> {
        let mut transition = FiniteFreezeTransition::from_permit(self, None::<()>);
        if !rejection_slot.is_vacant_for(transition.output_identity()) {
            transition.retain_error();
            freeze_driver::dispose_generation_inputs(inputs);
            return Err(transition.into_error());
        }
        Ok((transition.into_permit(), inputs))
    }

    /// Checks both original proven roots before the generation source executes.
    pub fn admit_generation_pairing<Generation>(
        self,
        canonical_root: CanonicalStoreRef<CK, CT>,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
        generation: Generation,
    ) -> Result<(Self, Generation), FiniteStoreExitError<O, PP, U>> {
        let paired = self
            .owner
            .storage()
            .storage()
            .roots_match(canonical_root, evidence_root);
        let mut transition = FiniteFreezeTransition::from_permit(self, None::<()>);
        if !paired {
            transition.retain_error();
            freeze_driver::dispose_generation_inputs((generation, (), ()));
            return Err(transition.into_error());
        }
        Ok((transition.into_permit(), generation))
    }

    /// Observes the actual finite dimensions without staging or copying counters.
    pub fn destination_counters_are_zero(&mut self) -> bool {
        let assembly = self.owner.storage_mut().storage_mut().ordinary_assembly();
        let canonical = assembly
            .loan()
            .terminal_observation(self.destinations._canonical_dimension)
            .expect("the sealed finite canonical destination remains registered");
        let evidence = assembly
            .loan()
            .terminal_observation(self.destinations._outcome_evidence_dimension)
            .expect("the sealed finite evidence destination remains registered");
        freeze_driver::destinations_are_zero(canonical, evidence)
    }

    /// Discards canonical ownership once while retaining the already staged evidence row.
    #[allow(clippy::type_complexity)]
    pub fn reject_generation<E>(
        self,
        rejection_slot: CanonicalCauseSlot<'host, StoreCanonicalRejection<E>>,
        cause: E,
        stable_code: u32,
    ) -> Result<
        FiniteRootedFailureEvidence<
            'host,
            O,
            PP,
            U,
            StoreCanonicalRejection<E>,
            EK,
            ET,
            FrozenFailureState,
        >,
        FiniteStoreExitError<O, PP, U>,
    > {
        let mut transition = FiniteFreezeTransition::from_permit(
            self,
            Some(StoreCanonicalRejection::Requested(cause)),
        );
        transition.order = PayloadOrder::ProofBeforeOutput;
        if !rejection_slot.is_vacant_for(transition.output_identity()) {
            return Err(transition.into_error());
        }
        if transition
            .dispose_payloads(PayloadOrder::ProofBeforeOutput)
            .is_err()
        {
            return Err(transition.into_error());
        }
        let cause = PreparationFailureCode::domain(stable_code);
        let discarded = {
            let mut assembly = transition
                .fields
                .as_mut()
                .expect("original finite fields remain installed during rejection")
                .assembly_borrow();
            assembly
                .loan_mut()
                .try_discard_canonical_for_rejection(cause)
        };
        if let Err(containment) = discarded {
            transition.error = Some(StorePrepareError::FinalizationContainment(containment));
            return Err(transition.into_error());
        }
        transition.into_failure::<FrozenFailureState>(rejection_slot, cause, Some(cause))
    }

    /// Opens the same invariant binder used by dynamic canonical joint commits.
    #[allow(clippy::type_complexity)]
    pub fn bind_joint_commit<R>(
        self,
        bind: impl for<'joint> FnOnce(
            FiniteCanonicalJointCommit<'host, 'joint, O, PP, U, T, Proof, CK, CT, EK, ET>,
            CanonicalJointSeal<'joint>,
        ) -> R,
    ) -> R {
        let mut transition = FiniteFreezeTransition::from_permit(self, None::<()>);
        let joint = FiniteCanonicalJointCommit {
            owner: transition.take_owner(),
            destinations: transition
                .destinations
                .take()
                .expect("original finite destinations"),
            failure_work: transition.failure_work,
            joint: PhantomData,
        };
        bind(joint, CanonicalJointSeal { joint: PhantomData })
    }
}

impl<'host, 'joint, O, PP, U, T, Proof, CK, CT, EK, ET>
    FiniteCanonicalJointCommit<'host, 'joint, O, PP, U, T, Proof, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Commits exact FrozenSuccess without observing control again.
    #[allow(clippy::type_complexity)]
    pub fn prepare(
        self,
        seal: CanonicalJointSeal<'joint>,
    ) -> Result<FiniteCanonicalJointSuccess<T, Proof, CK, CT, EK, ET>, FiniteStoreExitError<O, PP, U>>
    {
        let Self {
            owner,
            destinations,
            failure_work,
            joint: _,
        } = self;
        let CanonicalJointSeal { joint: _ } = seal;
        let (output, proof_owner) = owner.into_parts();
        let (proof, storage) = proof_owner.into_parts();
        let mut transition = FiniteFreezeTransition::new(
            output,
            proof,
            storage,
            destinations,
            failure_work,
            PayloadOrder::OutputBeforeProof,
            None::<()>,
        );
        let store = match transition.take_storage().try_prepare_frozen() {
            Ok(store) => store,
            Err(error) => {
                transition.exit_error = Some(error);
                return Err(transition.into_error());
            }
        };
        Ok(FiniteCanonicalJointSuccess {
            output: transition
                .output
                .take()
                .expect("original finite output permission")
                .into_owned(),
            proof: transition
                .proof
                .take()
                .expect("original finite proof permission")
                .into_owned(),
            store,
        })
    }
}
