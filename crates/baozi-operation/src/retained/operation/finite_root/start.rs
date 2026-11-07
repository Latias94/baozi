use super::*;
use crate::ledger::{FiniteStartupLedgerPlanParts, StartupRetainedAttribution};

/// Owns an uninvoked builder through every earlier rejection, including parameter teardown.
struct UninvokedFiniteBuilder<'account, B> {
    builder: Option<B>,
    account: &'account PhysicalStorageAccount,
}

impl<'account, B> UninvokedFiniteBuilder<'account, B> {
    fn new(account: &'account PhysicalStorageAccount, builder: B) -> Self {
        Self {
            builder: Some(builder),
            account,
        }
    }

    fn take(&mut self) -> B {
        self.builder
            .take()
            .expect("the original finite builder transfers once")
    }
}

impl<B> Drop for UninvokedFiniteBuilder<'_, B> {
    fn drop(&mut self) {
        if let Some(builder) = self.builder.take()
            && let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(builder)))
        {
            let _ = self
                .account
                .record_poison(AccountPoisonReason::RetainedBuilderPanicked);
            discard_untrusted_panic_payload(payload);
        }
    }
}

type FiniteBindingResult<'inv, 'account, O, P, PP, U, D, F, CB, EB> = Result<
    FinitePendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CB, EB>,
    RetainedOperationStartError<D>,
>;

type FiniteAttachmentResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    RetainedOperationLoan<'inv, O, P, PP, U, D, F, CB, EB>,
    FiniteAttachmentFailure<O, P, D, F>,
>;

type FiniteAttachedFieldsResult<'inv, O, P, PP, U, D, F, CB, EB> = Result<
    FiniteInvocationRootFields<'inv, O, P, PP, U, D, F, CB, EB>,
    FiniteAttachmentFailure<O, P, D, F>,
>;

type FiniteFieldsResult<O, P, D, F, CB, EB> =
    Result<FiniteRetainedOperationFields<O, P, D, F, CB, EB>, FiniteAttachmentFailure<O, P, D, F>>;

impl<O, P, PP, U, D, F, CB, EB> FiniteInvocationRoot<O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn bind<'inv, 'account>(
        &'inv mut self,
        control: OperationControl<PP, U>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        entry: &'inv mut FiniteEntryAdmission<'account>,
        plan: FiniteRetainedOperationStartPlan<P, D>,
    ) -> FiniteBindingResult<'inv, 'account, O, P, PP, U, D, F, CB, EB> {
        self.bind_with_control(&mut Some(control), limits, phases, entry, plan)
    }

    pub(crate) fn bind_with_control<'inv, 'account>(
        &'inv mut self,
        control: &mut Option<OperationControl<PP, U>>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        entry: &'inv mut FiniteEntryAdmission<'account>,
        plan: FiniteRetainedOperationStartPlan<P, D>,
    ) -> FiniteBindingResult<'inv, 'account, O, P, PP, U, D, F, CB, EB> {
        self.bind_inner(
            control,
            limits,
            phases,
            entry,
            plan,
            CanonicalLineageAuthority::fresh(),
        )
    }

    pub(crate) fn bind_derived<'inv, 'account, SourceBlock>(
        &'inv mut self,
        control: OperationControl<PP, U>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        source: &PublishedCanonicalAggregate<SourceBlock>,
        entry: &'inv mut FiniteEntryAdmission<'account>,
        plan: FiniteRetainedOperationStartPlan<P, D>,
    ) -> FiniteBindingResult<'inv, 'account, O, P, PP, U, D, F, CB, EB> {
        self.bind_derived_with_control(&mut Some(control), limits, phases, source, entry, plan)
    }

    pub(crate) fn bind_derived_with_control<'inv, 'account, SourceBlock>(
        &'inv mut self,
        control: &mut Option<OperationControl<PP, U>>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        source: &PublishedCanonicalAggregate<SourceBlock>,
        entry: &'inv mut FiniteEntryAdmission<'account>,
        plan: FiniteRetainedOperationStartPlan<P, D>,
    ) -> FiniteBindingResult<'inv, 'account, O, P, PP, U, D, F, CB, EB> {
        let preflight = entry
            .account
            .try_begin_account_preflight()
            .map_err(|cause| {
                RetainedOperationStartError::Writer(RetainedWriterError::AccountPoisoned(cause))
            })?;
        if !source.belongs_to(entry.account) {
            return Err(RetainedOperationStartError::Writer(
                RetainedWriterError::ForeignAccount,
            ));
        }
        let lineage = CanonicalLineageAuthority::derived(source);
        drop(preflight);
        self.bind_inner(control, limits, phases, entry, plan, lineage)
    }

    pub(crate) fn try_start<'inv>(
        &'inv mut self,
        control: OperationControl<PP, U>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        entry: &'inv mut FiniteEntryAdmission<'_>,
        plan: FiniteRetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> FiniteAttachmentResult<'inv, O, P, PP, U, D, F, CB, EB> {
        let mut builder = UninvokedFiniteBuilder::new(entry.account, build_fallback);
        self.bind(control, limits, phases, entry, plan)
            .map_err(FiniteAttachmentFailure::clean)?
            .attach(builder.take())
    }

    pub(crate) fn try_start_derived<'inv, SourceBlock>(
        &'inv mut self,
        control: OperationControl<PP, U>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        source: (
            &PublishedCanonicalAggregate<SourceBlock>,
            &'inv mut FiniteEntryAdmission<'_>,
        ),
        plan: FiniteRetainedOperationStartPlan<P, D>,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> FiniteAttachmentResult<'inv, O, P, PP, U, D, F, CB, EB> {
        let (source, entry) = source;
        let mut builder = UninvokedFiniteBuilder::new(entry.account, build_fallback);
        self.bind_derived(control, limits, phases, source, entry, plan)
            .map_err(FiniteAttachmentFailure::clean)?
            .attach(builder.take())
    }

    fn bind_inner<'inv, 'account>(
        &'inv mut self,
        control: &mut Option<OperationControl<PP, U>>,
        limits: FiniteLedgerLimits<D>,
        phases: [P; 3],
        entry: &'inv mut FiniteEntryAdmission<'account>,
        plan: FiniteRetainedOperationStartPlan<P, D>,
        lineage: CanonicalLineageAuthority,
    ) -> FiniteBindingResult<'inv, 'account, O, P, PP, U, D, F, CB, EB> {
        if self.started || entry.pending.is_some() || entry.identity.is_some() {
            return Err(RetainedOperationStartError::InvocationAlreadyStarted);
        }
        self.started = true;
        let failure_terminal_work = super::super::super::operation_start::qualify_terminal_plan(
            plan.terminal_writer,
            plan.terminal_account_share_work,
            &[plan.fallback_commitment],
            plan.failure_terminal_work,
        )?;
        let dimension_count = limits.dimensions().len();
        let phase_count = phases.len();
        let account = entry.account;
        // This capability lives in the outer entry before any fallible construction or disposal.
        entry.pending = Some(account.try_admit_operation_start().map_err(|cause| {
            RetainedOperationStartError::Writer(match cause {
                startup::OperationStartAdmissionError::AccountPoisoned(poisoned) => {
                    RetainedWriterError::AccountPoisoned(poisoned)
                }
                startup::OperationStartAdmissionError::AccountingOverflow => {
                    RetainedWriterError::Physical(RetainedAllocationError::AccountingOverflow)
                }
            })
        })?);
        entry.identity = Some(self.invocation_identity());
        let _constructor_unwind = scopeguard::guard_on_unwind(account, |account| {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        });
        self.common = Some(
            FiniteOperationInvocation::try_new_with_control(control, limits, phases, lineage)
                .map_err(|cause| {
                    RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
                })?,
        );
        let common = self
            .common
            .as_mut()
            .expect("finite binding installs its actual common once");
        #[cfg(test)]
        let injected_catalog_failure =
            super::super::super::take_injected_terminal_evidence_catalog_failure();
        #[cfg(not(test))]
        let injected_catalog_failure = false;
        if injected_catalog_failure
            || !common
                .coordinator
                .retained_ledger()
                .as_read()
                .catalog_shape_is_strictly_ordered(dimension_count, phase_count)
        {
            self.record_payload_failure(0x4154_4341);
            return Err(RetainedOperationStartError::TerminalEvidenceCatalogInvariant);
        }
        if let Some((plan, _)) = failure_terminal_work {
            common
                .view()
                .coordinator
                .ledger_mut()
                .configure_protected_work(plan.dimension())
                .map_err(|cause| {
                    RetainedOperationStartError::Writer(RetainedWriterError::Ledger(cause))
                })?;
        }
        Ok(FinitePendingRetainedAttachment {
            root: self,
            entry,
            plan,
            failure_terminal_work,
            local: PhantomData,
        })
    }
}

struct FiniteAttachmentGuard<'loan, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    common: &'loan mut FiniteOperationInvocation<O, P, PP, U, D>,
    resolved: bool,
}

impl<O, P, PP, U, D> FiniteAttachmentGuard<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn reject(&mut self, cause: PreparationFailureCode) {
        reject_retained_attachment(&mut self.common.view(), cause);
        self.resolved = true;
    }
}

impl<O, P, PP, U, D> Drop for FiniteAttachmentGuard<'_, O, P, PP, U, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: ProgressPhase,
    U: ProgressUnit,
    D: Copy + Ord + fmt::Debug,
{
    fn drop(&mut self) {
        if !self.resolved {
            self.reject(PreparationFailureCode::product_graph(
                StoreExecutionState::ABANDONED,
            ));
        }
    }
}

impl<'inv, 'account, O, P, PP, U, D, F, CB, EB>
    FinitePendingRetainedAttachment<'inv, 'account, O, P, PP, U, D, F, CB, EB>
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
    pub(crate) fn invocation_identity(&self) -> *const () {
        self.root.invocation_identity()
    }

    pub(crate) fn exclusive_parts(
        &mut self,
    ) -> Result<ExclusiveOperationParts<'_, O, P, PP, U, D>, ExecutionError<D>> {
        let common = self.root.common.as_mut().ok_or(ExecutionError::Stopped)?;
        common.preflight_exclusive()?;
        if common
            .coordinator
            .retained_ledger()
            .as_read()
            .ordinary_work_is_sealed()
        {
            return Err(ExecutionError::Stopped);
        }
        common
            .view()
            .into_exclusive_parts(self.entry.account, self.plan.terminal_account_share_work)
    }

    pub(crate) fn defer_unattached(
        self,
        requested: Option<PreparationFailureCode>,
    ) -> Result<FiniteDeferredUnattachedBinding, Self> {
        if self.root.retained.is_some()
            || self.root.assembly.is_some()
            || self.root.terminal.is_some()
            || self.entry.pending.is_none()
        {
            return Err(self);
        }
        let Some(common) = self.root.common.as_mut() else {
            return Err(self);
        };
        if let Some(cause) = requested {
            record_terminal_cause_with_precedence(&common.lifecycle.sticky_terminal_cause, cause);
        } else if common.lifecycle.execution_state.get() != StoreExecutionState::Running {
            record_terminal_cause_with_precedence(
                &common.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::product_graph(StoreExecutionState::ABANDONED),
            );
        }
        if common
            .view()
            .coordinator
            .ledger_mut()
            .seal_ordinary_work()
            .is_err()
        {
            record_terminal_cause_with_precedence(
                &common.lifecycle.sticky_terminal_cause,
                PreparationFailureCode::ledger(),
            );
        }
        Ok(FiniteDeferredUnattachedBinding {
            identity: self.root.invocation_identity().addr(),
        })
    }

    pub(crate) fn attach(
        self,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> FiniteAttachmentResult<'inv, O, P, PP, U, D, F, CB, EB> {
        self.attach_fields(build_fallback)?
            .into_loan()
            .ok_or_else(|| {
                FiniteAttachmentFailure::clean(RetainedOperationStartError::Writer(
                    RetainedWriterError::OwnershipUnverified,
                ))
            })
    }

    pub(crate) fn attach_fields(
        self,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> FiniteAttachedFieldsResult<'inv, O, P, PP, U, D, F, CB, EB> {
        let mut builder = UninvokedFiniteBuilder::new(self.entry.account, build_fallback);
        let Self {
            root,
            entry,
            plan,
            failure_terminal_work,
            ..
        } = self;
        if root.retained.is_some() || root.assembly.is_some() || root.terminal.is_some() {
            return Err(FiniteAttachmentFailure::clean(
                RetainedOperationStartError::InvocationAlreadyStarted,
            ));
        }
        let Some(common) = root.common.as_mut() else {
            return Err(FiniteAttachmentFailure::clean(
                RetainedOperationStartError::Writer(RetainedWriterError::OwnershipUnverified),
            ));
        };
        let mut guard = FiniteAttachmentGuard {
            common,
            resolved: false,
        };
        let result = prepare_attachment(
            guard.common,
            entry,
            plan,
            failure_terminal_work,
            builder.take(),
        );
        let fields = match result {
            Ok(fields) => fields,
            Err(error) => {
                let code = match &error.cause {
                    RetainedOperationStartError::Control(cause) => {
                        PreparationFailureCode::from_control(cause)
                    }
                    RetainedOperationStartError::Writer(RetainedWriterError::Ledger(_)) => {
                        PreparationFailureCode::ledger()
                    }
                    _ => PreparationFailureCode::product_graph(0x4154_5443),
                };
                guard.reject(code);
                drop(guard);
                root.contained |= error.has_residue();
                return Err(error);
            }
        };
        guard.resolved = true;
        drop(guard);
        root.retained = Some(fields);
        Ok(root.fields())
    }
}

fn prepare_attachment<O, P, PP, U, D, F, CB, EB>(
    common: &mut FiniteOperationInvocation<O, P, PP, U, D>,
    entry: &mut FiniteEntryAdmission<'_>,
    plan: FiniteRetainedOperationStartPlan<P, D>,
    failure_work: QualifiedFailureTerminalWork<D>,
    build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
) -> FiniteFieldsResult<O, P, D, F, CB, EB>
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
    let account = entry.account;
    let mut builder = UninvokedFiniteBuilder::new(account, build_fallback);
    let _attachment_unwind = scopeguard::guard_on_unwind(account, |account| {
        let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
    });
    check_retained_attachment_entry(&common.view(), account)
        .map_err(FiniteAttachmentFailure::clean)?;
    let (canonical, evidence) = checked_terminal_assembly_limits(plan.terminal_writer.limits)
        .map_err(FiniteAttachmentFailure::clean)?;
    let writer = plan.terminal_writer;
    let batch_error = |cause| {
        FiniteAttachmentFailure::clean(RetainedOperationStartError::Writer(map_batch_error(cause)))
    };
    let fallback = AttributionBatchPlan::<O, P, D, OutcomeEvidenceDestination>::try_new(
        writer.phase,
        writer.capacity_dimension,
        1,
        1,
    )
    .map_err(batch_error)?;
    let canonical_assembly = AttributionBatchPlan::<O, P, D, CanonicalOutput>::try_new(
        writer.phase,
        writer.capacity_dimension,
        canonical.attribution_slots,
        canonical.commitment_slots,
    )
    .map_err(batch_error)?;
    let evidence_assembly = AttributionBatchPlan::<O, P, D, OutcomeEvidenceDestination>::try_new(
        writer.phase,
        writer.capacity_dimension,
        evidence.attribution_slots,
        evidence.commitment_slots,
    )
    .map_err(batch_error)?;
    let charge = account
        .checked_retained_charge::<F>(plan.fallback_plan)
        .map_err(|cause| {
            FiniteAttachmentFailure::clean(RetainedOperationStartError::Writer(
                RetainedWriterError::Physical(cause),
            ))
        })?;
    let parts = FiniteStartupLedgerPlanParts {
        canonical_assembly,
        evidence_assembly,
        fallback,
        fallback_attribution: StartupRetainedAttribution::new(
            writer.phase,
            writer.attribution_dimension,
            charge.bytes,
        ),
        account_shares: StartupAccountShareEnvelope::three_identical(
            plan.terminal_account_share_work.charges(),
        ),
        fallback_commitment: plan.fallback_commitment,
    };
    let cross_plane = FiniteStartupCrossPlanePlan::try_new(parts, charge).map_err(|_| {
        FiniteAttachmentFailure::clean(RetainedOperationStartError::Writer(
            RetainedWriterError::OwnershipUnverified,
        ))
    })?;
    preflight_retained_failure_work(common.coordinator.retained_ledger().as_read(), failure_work)
        .map_err(FiniteAttachmentFailure::clean)?;
    let admission = entry.pending.take().ok_or_else(|| {
        FiniteAttachmentFailure::clean(RetainedOperationStartError::Writer(
            RetainedWriterError::OwnershipUnverified,
        ))
    })?;
    let ready = {
        let mut common = common.view();
        admission
            .try_prepare_finite_startup_cross_plane(common.coordinator.ledger_mut(), cross_plane)
    }
    .map_err(FiniteAttachmentFailure::from_startup)?;
    let baseline = ready
        .into_retained_guard::<F>(plan.fallback_plan)
        .try_build(builder.take())
        .map_err(|failure| FiniteAttachmentFailure {
            cause: RetainedOperationStartError::Writer(failure.cause),
            residue: failure
                .residue
                .map_or(FiniteAttachmentResidue::None, |owner| {
                    FiniteAttachmentResidue::Retained { _owner: owner }
                }),
        })?;
    let baseline = baseline
        .try_attach_owned(common.coordinator.retained_ledger().as_read())
        .map_err(|(cause, baseline)| FiniteAttachmentFailure {
            cause: RetainedOperationStartError::Writer(cause),
            residue: FiniteAttachmentResidue::Retained {
                _owner: FiniteStartupRetainedResidue::from_baseline(baseline),
            },
        })?;
    Ok(FiniteRetainedOperationFields {
        baseline,
        metadata: RetainedOperationMetadata::from_qualified_plan(
            writer,
            plan.terminal_account_share_work,
            canonical,
            failure_work,
        ),
    })
}

impl<O, P, D, F> FiniteAttachmentFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn from_startup(failure: FiniteStartupCrossPlaneAdmissionFailure<O, P, D>) -> Self {
        let FiniteStartupCrossPlaneAdmissionFailure { cause, containment } = failure;
        let cause = match cause {
            FiniteStartupAdmissionCause::LedgerAdmission(cause) => map_batch_error(cause),
            FiniteStartupAdmissionCause::OwnerShare(cause) => cause,
            FiniteStartupAdmissionCause::PhysicalNoMutation(cause) => {
                map_startup_physical_no_mutation(cause)
            }
            FiniteStartupAdmissionCause::Uncertain => containment
                .as_ref()
                .and_then(|owner| owner.poisoned())
                .map_or(
                    RetainedWriterError::OwnershipUnverified,
                    RetainedWriterError::AccountPoisoned,
                ),
        };
        Self {
            cause: RetainedOperationStartError::Writer(cause),
            residue: containment.map_or(FiniteAttachmentResidue::None, |owner| {
                FiniteAttachmentResidue::CrossPlane { _owner: owner }
            }),
        }
    }
}
