use crate::error::{
    PostProcessError, PostProcessFailure, PostProcessPlanError, PostProcessRunError,
    PostProcessStepIssue, PostProcessSuccess, Result, StepResult,
};
use crate::operation::{
    PostProcessControl, PostProcessDimension, PostProcessLedgerPhase, PostProcessLimits,
    PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit,
};
use baozi_core::{
    Aabb, PrimitiveTopology, SceneEdit, SceneReservationKind, SceneValidationControl,
    SceneValidationFailure, SceneValidationLimits, SceneValidationSuccess, SceneValidationWork,
    ValidatedScene, Vec3,
};
use baozi_operation::{
    CapacityPlan, CapacityPlanError, Commitment, ControlCause, FinalProgress, LedgerCause,
    LedgerLimits, LedgerLimitsError, LiveReservation, OperationCoordinator, PendingPublication,
    PublicationCause, PublicationReservationBatch, ReplacementCapacityPlan,
};
use std::mem::size_of;

type Coordinator = OperationCoordinator<
    PostProcessOwner,
    PostProcessLedgerPhase,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    PostProcessDimension,
>;
type BufferReservation =
    LiveReservation<PostProcessOwner, PostProcessLedgerPhase, PostProcessDimension>;
type PublicationBatch =
    PublicationReservationBatch<PostProcessOwner, PostProcessLedgerPhase, PostProcessDimension>;
type PublicationBatchError = baozi_operation::PublicationReservationBatchError<
    PostProcessOwner,
    PostProcessLedgerPhase,
    PostProcessDimension,
>;
type PostProcessPendingPublication =
    PendingPublication<PostProcessOwner, PostProcessLedgerPhase, PostProcessDimension>;

fn try_postprocess_coordinator(
    control: PostProcessControl,
    ledger_limits: LedgerLimits<PostProcessDimension>,
) -> std::result::Result<Coordinator, LedgerCause<PostProcessDimension>> {
    Coordinator::try_new(control, ledger_limits, &PostProcessLedgerPhase::ALL)
}

/// A currently implemented post-process transformation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostProcessStep {
    Triangulate,
    GenerateNormals,
    GenerateBoundingBoxes,
}

impl PostProcessStep {
    pub const COUNT: usize = 3;
    pub const MAX_REQUESTED_ENTRIES: usize = 64;
    pub const CANONICAL_ORDER: &'static [Self] = &[
        Self::Triangulate,
        Self::GenerateNormals,
        Self::GenerateBoundingBoxes,
    ];

    pub const fn is_destructive(self) -> bool {
        matches!(self, Self::Triangulate | Self::GenerateNormals)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Triangulate => "Triangulate",
            Self::GenerateNormals => "GenerateNormals",
            Self::GenerateBoundingBoxes => "GenerateBoundingBoxes",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Triangulate => 1 << 0,
            Self::GenerateNormals => 1 << 1,
            Self::GenerateBoundingBoxes => 1 << 2,
        }
    }
}

/// An ordered post-process invocation with hard resource limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessPipeline {
    steps: [PostProcessStep; PostProcessStep::COUNT],
    step_count: u8,
    limits: PostProcessLimits,
}

impl PostProcessPipeline {
    pub fn new(
        steps: impl IntoIterator<Item = PostProcessStep>,
    ) -> std::result::Result<Self, PostProcessPlanError> {
        let mut mask = 0_u8;
        for (index, step) in steps.into_iter().enumerate() {
            if index == PostProcessStep::MAX_REQUESTED_ENTRIES {
                return Err(PostProcessPlanError::TooManyRequestedEntries {
                    maximum: PostProcessStep::MAX_REQUESTED_ENTRIES,
                });
            }
            mask |= step.bit();
            if mask.count_ones() as usize == PostProcessStep::COUNT {
                break;
            }
        }

        let mut ordered = [PostProcessStep::Triangulate; PostProcessStep::COUNT];
        let mut step_count = 0_usize;
        for step in PostProcessStep::CANONICAL_ORDER.iter().copied() {
            if mask & step.bit() != 0 {
                ordered[step_count] = step;
                step_count += 1;
            }
        }

        Ok(Self {
            steps: ordered,
            step_count: step_count as u8,
            limits: PostProcessLimits::default(),
        })
    }

    /// Replaces the default hard operation limits.
    pub fn with_limits(mut self, limits: PostProcessLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn steps(&self) -> &[PostProcessStep] {
        &self.steps[..usize::from(self.step_count)]
    }

    pub const fn limits(&self) -> PostProcessLimits {
        self.limits
    }

    /// Runs with no cancellation signal, observer, or deadline.
    #[allow(
        clippy::result_large_err,
        reason = "terminal post-process failures retain complete bounded evidence without heap indirection"
    )]
    pub fn run(&self, input: &ValidatedScene) -> Result<PostProcessSuccess> {
        self.run_with_control(input, PostProcessControl::default())
    }

    /// Runs transactionally with invocation-local operation control.
    ///
    /// The input is never mutated. Every owned output is published only after
    /// a controlled clone, full central-validation commit, and final checkpoint.
    #[allow(
        clippy::result_large_err,
        reason = "terminal post-process failures retain complete bounded evidence without heap indirection"
    )]
    pub fn run_with_control(
        &self,
        input: &ValidatedScene,
        control: PostProcessControl,
    ) -> Result<PostProcessSuccess> {
        self.run_with_control_and_ledger_limits(input, control, self.limits.try_ledger_limits())
    }

    #[allow(
        clippy::result_large_err,
        reason = "terminal post-process failures retain complete bounded evidence without heap indirection"
    )]
    fn run_with_control_and_ledger_limits(
        &self,
        input: &ValidatedScene,
        control: PostProcessControl,
        ledger_limits: std::result::Result<
            LedgerLimits<PostProcessDimension>,
            LedgerLimitsError<PostProcessDimension>,
        >,
    ) -> Result<PostProcessSuccess> {
        let ledger_limits = ledger_limits.map_err(PostProcessRunError::initialization)?;
        let mut coordinator = try_postprocess_coordinator(control, ledger_limits)
            .map_err(PostProcessRunError::coordinator_initialization)?;
        #[cfg(feature = "tracing")]
        let _span = tracing::info_span!("baozi.postprocess").entered();

        let outcome = self.execute(input, &mut coordinator);

        match outcome {
            Ok(output) => {
                finish_success(coordinator, output).map_err(PostProcessRunError::operation)
            }
            Err(error) => Err(PostProcessRunError::operation(finish_failure(
                coordinator,
                error,
            ))),
        }
    }

    fn execute(
        &self,
        input: &ValidatedScene,
        coordinator: &mut Coordinator,
    ) -> StepResult<ValidatedScene> {
        coordinator.poll().map_err(control_error)?;

        let mut state = ExecutionState::new(coordinator);
        state.begin_transform()?;
        let validation_limits = ExecutionState::validation_limits();
        let mut edit = input
            .begin_edit_with(validation_limits, &mut state)
            .map_err(validation_failure_error)?;
        for step in self.steps() {
            match step {
                PostProcessStep::Triangulate => {
                    triangulate_scene(&mut edit, &mut state)?;
                }
                PostProcessStep::GenerateNormals => {
                    generate_normals(&mut edit, &mut state)?;
                }
                PostProcessStep::GenerateBoundingBoxes => {
                    generate_bounding_boxes(&mut edit, &mut state)?;
                }
            }
        }

        state.begin_validation()?;
        let validated = edit
            .commit_with(validation_limits, &mut state)
            .map_err(validation_failure_error)?;
        state.publish(validated)
    }
}

#[allow(
    clippy::result_large_err,
    reason = "terminal post-process failures retain complete bounded evidence without heap indirection"
)]
fn finish_success(
    coordinator: Coordinator,
    output: ValidatedScene,
) -> std::result::Result<PostProcessSuccess, PostProcessFailure> {
    match coordinator.finish() {
        Ok(ledger) => Ok(PostProcessSuccess::new(output, ledger)),
        Err(finalization_error) => Err(PostProcessFailure::with_finalization_error(
            PostProcessError::LedgerFinalization,
            finalization_error,
        )),
    }
}

fn finish_failure(coordinator: Coordinator, error: PostProcessError) -> PostProcessFailure {
    match coordinator.finish() {
        Ok(ledger) => PostProcessFailure::new(error, ledger),
        Err(finalization_error) => {
            PostProcessFailure::with_finalization_error(error, finalization_error)
        }
    }
}

struct SceneReservation {
    kind: SceneReservationKind,
    primary: Option<BufferReservation>,
    retained: ReservationList,
    publication_batch: Option<PublicationBatch>,
    rejected_publication_batch: Option<PublicationBatchError>,
    pending_publication: Option<PostProcessPendingPublication>,
}

struct ReservationList {
    values: Vec<BufferReservation>,
    capacity: Option<BufferReservation>,
}

impl ReservationList {
    const fn new() -> Self {
        Self {
            values: Vec::new(),
            capacity: None,
        }
    }

    fn is_vacant(&self) -> bool {
        self.values.is_empty() && self.capacity.is_none()
    }

    fn has_slots(&self, additional: usize) -> bool {
        self.values
            .len()
            .checked_add(additional)
            .is_some_and(|required| required <= self.values.capacity())
    }

    fn ensure_slots(
        &mut self,
        coordinator: &mut Coordinator,
        phase: PostProcessLedgerPhase,
        additional: usize,
        context: &'static str,
        resource: &'static str,
    ) -> StepResult<()> {
        let required = self
            .values
            .len()
            .checked_add(additional)
            .ok_or(PostProcessError::CapacityExceeded { context, resource })?;
        if required <= self.values.capacity() {
            return Ok(());
        }

        let desired = required.max(4);
        let replacement_plan = ReplacementCapacityPlan::for_type::<BufferReservation>(
            self.values.len(),
            self.values.capacity(),
            desired,
        )
        .map_err(|_| PostProcessError::CapacityExceeded { context, resource })?;
        let admitted = replacement_plan
            .candidate_envelope_bytes()
            .map_err(|_| PostProcessError::CapacityExceeded { context, resource })?;
        let mut candidate = Self::new();
        resize_or_reserve(
            coordinator,
            &mut candidate.capacity,
            phase,
            PostProcessDimension::StagingBytes,
            admitted,
        )?;
        replacement_plan
            .verify_source(self.values.len(), self.values.capacity())
            .map_err(|_| PostProcessError::ValidationControlInvariant {
                message: "reservation-list source plan became stale before allocator entry",
            })?;
        let candidate_plan = replacement_plan.candidate();
        candidate_plan
            .verify_preflight(candidate.values.len(), candidate.values.capacity())
            .map_err(|_| PostProcessError::ValidationControlInvariant {
                message: "reservation-list candidate plan became stale before allocator entry",
            })?;
        candidate
            .values
            .try_reserve_exact(candidate_plan.allocator_additional())
            .map_err(|_| PostProcessError::CapacityExceeded { context, resource })?;
        candidate_plan
            .finish(candidate.values.len(), candidate.values.capacity())
            .map_err(|_| PostProcessError::ValidationControlInvariant {
                message: "reservation-list candidate violated the qualified capacity envelope",
            })?;

        candidate.values.append(&mut self.values);
        std::mem::swap(self, &mut candidate);
        drop(candidate);
        Ok(())
    }

    fn transfer_all(
        &mut self,
        coordinator: &mut Coordinator,
        phase: PostProcessLedgerPhase,
    ) -> StepResult<()> {
        for reservation in &mut self.values {
            coordinator.poll().map_err(control_error)?;
            coordinator
                .transfer_reservation(reservation, phase)
                .map_err(accounting_error)?;
        }
        if let Some(capacity) = &mut self.capacity {
            coordinator.poll().map_err(control_error)?;
            coordinator
                .transfer_reservation(capacity, phase)
                .map_err(accounting_error)?;
        }
        Ok(())
    }

    fn payload_bytes(&self) -> StepResult<u64> {
        self.values.iter().try_fold(0_u64, |total, token| {
            total.checked_add(token.bytes()).ok_or_else(|| {
                accounting_error(LedgerCause::ArithmeticOverflow {
                    class: baozi_operation::CounterClass::Live,
                    dimension: PostProcessDimension::CanonicalBytes,
                })
            })
        })
    }

    fn into_publication(
        mut self,
        primary: Option<BufferReservation>,
    ) -> std::result::Result<PublicationBatch, PublicationBatchError> {
        let additional = usize::from(primary.is_some());
        assert!(
            self.has_slots(additional),
            "publication reservation slots must be admitted before conversion"
        );
        if let Some(primary) = primary {
            self.values.push(primary);
        }
        PublicationReservationBatch::try_new(self.values, self.capacity)
    }
}

impl Default for ReservationList {
    fn default() -> Self {
        Self::new()
    }
}

struct ExecutionState<'a> {
    coordinator: &'a mut Coordinator,
    completed_work: u64,
    retained_output: ReservationList,
}

impl<'a> ExecutionState<'a> {
    fn new(coordinator: &'a mut Coordinator) -> Self {
        Self {
            coordinator,
            completed_work: 0,
            retained_output: ReservationList::new(),
        }
    }

    fn validation_limits() -> SceneValidationLimits {
        SceneValidationLimits::default()
            .with_max_work(u64::MAX)
            .with_max_canonical_bytes(u64::MAX)
            .with_max_scratch_bytes(u64::MAX)
    }

    fn begin_transform(&mut self) -> StepResult<()> {
        self.coordinator
            .report_progress(
                PostProcessProgressPhase::Transform,
                PostProcessProgressUnit::WorkItems,
                0,
                None,
            )
            .map_err(control_error)
    }

    fn start_work(&mut self, amount: u64) -> StepResult<()> {
        let mut checkpoint = self.coordinator.checkpoint();
        checkpoint.poll().map_err(control_error)?;
        checkpoint
            .charge_attempted(PostProcessDimension::WorkItems, amount)
            .map_err(accounting_error)
    }

    fn finish_work(&mut self, amount: u64) -> StepResult<()> {
        self.completed_work = self.completed_work.checked_add(amount).ok_or_else(|| {
            accounting_error(LedgerCause::ArithmeticOverflow {
                class: baozi_operation::CounterClass::Consumed,
                dimension: PostProcessDimension::WorkItems,
            })
        })?;
        let mut checkpoint = self.coordinator.checkpoint();
        checkpoint
            .charge_consumed(PostProcessDimension::WorkItems, amount)
            .map_err(accounting_error)?;
        checkpoint
            .report_progress(
                PostProcessProgressPhase::Transform,
                PostProcessProgressUnit::WorkItems,
                self.completed_work,
                None,
            )
            .map_err(control_error)
    }

    fn perform<T>(&mut self, action: impl FnOnce() -> StepResult<T>) -> StepResult<T> {
        self.start_work(1)?;
        let value = action()?;
        self.finish_work(1)?;
        Ok(value)
    }

    fn reserve_staging(&mut self, bytes: u64) -> StepResult<Option<BufferReservation>> {
        if bytes == 0 {
            return Ok(None);
        }
        let mut checkpoint = self.coordinator.checkpoint();
        checkpoint.poll().map_err(control_error)?;
        checkpoint
            .reserve(
                PostProcessLedgerPhase::TransactionStaging,
                PostProcessDimension::StagingBytes,
                bytes,
            )
            .map(Some)
            .map_err(accounting_error)
    }

    fn begin_validation(&mut self) -> StepResult<()> {
        self.retained_output.ensure_slots(
            self.coordinator,
            PostProcessLedgerPhase::TransactionStaging,
            2,
            "publication",
            "publication reservation evidence",
        )?;
        self.retained_output
            .transfer_all(self.coordinator, PostProcessLedgerPhase::ValidationCommit)?;
        self.coordinator
            .report_progress(
                PostProcessProgressPhase::Validate,
                PostProcessProgressUnit::WorkItems,
                0,
                Some(1),
            )
            .map_err(control_error)
    }

    fn ensure_output_slot(&mut self, step: PostProcessStep) -> StepResult<()> {
        self.retained_output.ensure_slots(
            self.coordinator,
            PostProcessLedgerPhase::TransactionStaging,
            1,
            step.name(),
            "staging reservation evidence",
        )
    }

    fn retain_output(&mut self, reservation: Option<BufferReservation>) {
        if let Some(reservation) = reservation {
            self.retained_output.values.push(reservation);
        }
    }

    fn publish(
        &mut self,
        validated: SceneValidationSuccess<SceneReservation>,
    ) -> StepResult<ValidatedScene> {
        let canonical_bytes = validated.stats().footprint().accounted_payload_bytes();
        validated.try_publish(|reservation| {
            if !self.retained_output.is_vacant() {
                return Err(PostProcessError::ValidationControlInvariant {
                    message: "canonical transition did not absorb retained output reservations",
                });
            }
            if reservation.kind != SceneReservationKind::CanonicalOutput {
                return Err(PostProcessError::ValidationControlInvariant {
                    message: "publication received a non-canonical scene reservation",
                });
            }
            if reservation.publication_batch.is_some()
                || reservation.rejected_publication_batch.is_some()
                || reservation.pending_publication.is_some()
            {
                return Err(PostProcessError::ValidationControlInvariant {
                    message: "publication reservation was already prepared",
                });
            }
            let additional = usize::from(reservation.primary.is_some());
            if !reservation.retained.has_slots(additional) {
                return Err(PostProcessError::ValidationControlInvariant {
                    message: "publication reservation slots were not pre-admitted",
                });
            }

            let retained = std::mem::take(&mut reservation.retained);
            let primary = reservation.primary.take();
            reservation.publication_batch = match retained.into_publication(primary) {
                Ok(publication_batch) => Some(publication_batch),
                Err(error) => {
                    reservation.rejected_publication_batch = Some(error);
                    return Err(PostProcessError::ValidationControlInvariant {
                        message: "publication reservation capacity was not fully covered",
                    });
                }
            };
            let publication_batch = reservation.publication_batch.as_mut().ok_or(
                PostProcessError::ValidationControlInvariant {
                    message: "publication batch state was lost",
                },
            )?;
            let publication = self
                .coordinator
                .prepare_publication(
                    publication_batch,
                    &[Commitment::new(
                        PostProcessDimension::CanonicalBytes,
                        canonical_bytes,
                    )],
                )
                .map_err(accounting_error)?;
            reservation.pending_publication = Some(publication);
            reservation.publication_batch = None;
            let pending_publication = reservation.pending_publication.as_mut().ok_or(
                PostProcessError::ValidationControlInvariant {
                    message: "pending publication state was lost",
                },
            )?;
            self.coordinator
                .publish(
                    pending_publication,
                    FinalProgress::new(
                        PostProcessProgressPhase::Validate,
                        PostProcessProgressUnit::WorkItems,
                        1,
                    ),
                )
                .map_err(publication_error)
        })
    }

    fn transition_scene_reservation(
        &mut self,
        reservation: &mut SceneReservation,
        from: SceneReservationKind,
        to: SceneReservationKind,
        required_bytes: u64,
    ) -> StepResult<()> {
        if reservation.kind != from {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "core reservation transition source did not match its live token",
            });
        }
        if from == to {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "core used a classification transition for a same-class resize",
            });
        }

        let absorbs_output = from == SceneReservationKind::EditCandidate
            && to == SceneReservationKind::CanonicalOutput;
        if absorbs_output && !reservation.retained.is_vacant() {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "edit reservation already retained output evidence",
            });
        }

        let retained = if absorbs_output {
            &self.retained_output
        } else {
            &reservation.retained
        };
        let primary_bytes = reservation
            .primary
            .as_ref()
            .map_or(0, BufferReservation::bytes);
        let available = primary_bytes
            .checked_add(retained.payload_bytes()?)
            .ok_or_else(|| {
                accounting_error(LedgerCause::ArithmeticOverflow {
                    class: baozi_operation::CounterClass::Live,
                    dimension: PostProcessDimension::CanonicalBytes,
                })
            })?;
        if available < required_bytes {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "scene reservation transition did not cover the required footprint",
            });
        }

        let additional_slots = usize::from(reservation.primary.is_some());
        if !retained.has_slots(additional_slots) {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "scene reservation transition slots were not pre-admitted",
            });
        }

        let mut combined = if absorbs_output {
            std::mem::take(&mut self.retained_output)
        } else {
            std::mem::take(&mut reservation.retained)
        };
        let had_primary = reservation.primary.is_some();
        if let Some(primary) = reservation.primary.take() {
            combined.values.insert(0, primary);
        }

        let (target_phase, target_dimension) = reservation_class(to);
        if let Err(cause) = self.coordinator.transition_reservations(
            &mut combined.values,
            target_phase,
            target_dimension,
            required_bytes,
        ) {
            if had_primary {
                reservation.primary = Some(combined.values.remove(0));
            }
            if absorbs_output {
                self.retained_output = combined;
            } else {
                reservation.retained = combined;
            }
            return Err(accounting_error(cause));
        }

        debug_assert!(
            !combined.values.is_empty(),
            "a successful scene transition must retain an owner capability"
        );
        if required_bytes == 0 {
            combined.values.truncate(1);
        } else {
            combined.values.retain(|token| token.bytes() != 0);
        }
        reservation.retained = combined;
        reservation.kind = to;
        Ok(())
    }
}

impl SceneValidationControl for ExecutionState<'_> {
    type Error = PostProcessError;
    type Reservation = SceneReservation;

    fn poll(&mut self) -> std::result::Result<(), Self::Error> {
        self.coordinator.poll().map_err(control_error)
    }

    fn charge_work(
        &mut self,
        _work: SceneValidationWork,
        amount: u64,
    ) -> std::result::Result<(), Self::Error> {
        let mut checkpoint = self.coordinator.checkpoint();
        checkpoint
            .charge_attempted(PostProcessDimension::WorkItems, amount)
            .map_err(accounting_error)?;
        checkpoint
            .charge_consumed(PostProcessDimension::WorkItems, amount)
            .map_err(accounting_error)
    }

    fn reserve(
        &mut self,
        kind: SceneReservationKind,
        bytes: u64,
    ) -> std::result::Result<Self::Reservation, Self::Error> {
        let (phase, dimension) = reservation_class(kind);
        let primary = Some(
            self.coordinator
                .checkpoint()
                .reserve(phase, dimension, bytes)
                .map_err(accounting_error)?,
        );
        Ok(SceneReservation {
            kind,
            primary,
            retained: ReservationList::new(),
            publication_batch: None,
            rejected_publication_batch: None,
            pending_publication: None,
        })
    }

    fn validate_reservation(
        &mut self,
        reservation: &Self::Reservation,
        kind: SceneReservationKind,
    ) -> std::result::Result<(), Self::Error> {
        if reservation.kind != kind {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "core reservation validation kind did not match its live token",
            });
        }
        if reservation.publication_batch.is_some()
            || reservation.rejected_publication_batch.is_some()
            || reservation.pending_publication.is_some()
        {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "published reservation state cannot re-enter core validation",
            });
        }

        let mut has_capability = false;
        for token in reservation
            .primary
            .iter()
            .chain(reservation.retained.values.iter())
            .chain(reservation.retained.capacity.iter())
        {
            self.coordinator
                .validate_live_reservation(token)
                .map_err(accounting_error)?;
            has_capability = true;
        }
        if !has_capability {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "stateful scene reservation lost its owner capability",
            });
        }
        Ok(())
    }

    fn resize_reservation(
        &mut self,
        reservation: &mut Self::Reservation,
        kind: SceneReservationKind,
        required_bytes: u64,
    ) -> std::result::Result<(), Self::Error> {
        if reservation.kind != kind {
            return Err(PostProcessError::ValidationControlInvariant {
                message: "core reservation resize kind did not match its live token",
            });
        }

        match &mut reservation.primary {
            None => Err(PostProcessError::ValidationControlInvariant {
                message: "scene reservation resize lost its owner capability",
            }),
            Some(token) => {
                self.coordinator
                    .resize_reservation(token, required_bytes)
                    .map_err(accounting_error)?;
                Ok(())
            }
        }
    }

    fn transition_reservation(
        &mut self,
        reservation: &mut Self::Reservation,
        from: SceneReservationKind,
        to: SceneReservationKind,
        required_bytes: u64,
    ) -> std::result::Result<(), Self::Error> {
        self.transition_scene_reservation(reservation, from, to, required_bytes)
    }
}

fn resize_or_reserve(
    coordinator: &mut Coordinator,
    reservation: &mut Option<BufferReservation>,
    phase: PostProcessLedgerPhase,
    dimension: PostProcessDimension,
    required_bytes: u64,
) -> StepResult<()> {
    match reservation {
        Some(token) => {
            coordinator
                .resize_reservation(token, required_bytes)
                .map_err(accounting_error)?;
            if required_bytes == 0 {
                *reservation = None;
            }
            Ok(())
        }
        None if required_bytes == 0 => Ok(()),
        None => {
            *reservation = Some(
                coordinator
                    .checkpoint()
                    .reserve(phase, dimension, required_bytes)
                    .map_err(accounting_error)?,
            );
            Ok(())
        }
    }
}

fn reservation_class(kind: SceneReservationKind) -> (PostProcessLedgerPhase, PostProcessDimension) {
    match kind {
        SceneReservationKind::EditCandidate => (
            PostProcessLedgerPhase::TransactionStaging,
            PostProcessDimension::StagingBytes,
        ),
        SceneReservationKind::ValidationScratch => (
            PostProcessLedgerPhase::ValidationCommit,
            PostProcessDimension::StagingBytes,
        ),
        SceneReservationKind::CanonicalOutput => (
            PostProcessLedgerPhase::ValidationCommit,
            PostProcessDimension::CanonicalBytes,
        ),
    }
}

fn triangulate_scene(
    edit: &mut SceneEdit<SceneReservation>,
    state: &mut ExecutionState<'_>,
) -> StepResult<bool> {
    let mut changed = false;
    for mesh_index in 0..edit.meshes().len() {
        state.perform(|| Ok(()))?;
        let mesh = edit.meshes_mut().get_mut(mesh_index).ok_or_else(|| {
            PostProcessError::step(
                PostProcessStep::Triangulate,
                PostProcessStepIssue::MeshMissing { mesh: mesh_index },
            )
        })?;
        if mesh.topology != PrimitiveTopology::Polygons {
            continue;
        }

        let uses_implicit_indices = mesh.indices.is_empty();
        let source_len = if uses_implicit_indices {
            mesh.element_count()
        } else {
            mesh.indices.len()
        };
        if uses_implicit_indices && source_len > u32::MAX as usize {
            return Err(PostProcessError::step(
                PostProcessStep::Triangulate,
                PostProcessStepIssue::ImplicitIndexOutOfRange {
                    mesh: mesh_index,
                    index: source_len.saturating_sub(1),
                },
            ));
        }

        let (_source_reservation, mut source_elements) = allocate_vec::<u32>(
            state,
            source_len,
            PostProcessStep::Triangulate,
            "polygon indices",
        )?;
        if uses_implicit_indices {
            for index in 0..source_len {
                let index = state.perform(|| {
                    u32::try_from(index).map_err(|_| {
                        PostProcessError::step(
                            PostProcessStep::Triangulate,
                            PostProcessStepIssue::ImplicitIndexOutOfRange {
                                mesh: mesh_index,
                                index,
                            },
                        )
                    })
                })?;
                source_elements.push(index);
            }
        } else {
            for index in mesh.indices.iter().copied() {
                state.perform(|| Ok(()))?;
                source_elements.push(index);
            }
        }

        let mut triangle_index_count = 0usize;
        for (face_index, count) in mesh.face_vertex_counts.iter().copied().enumerate() {
            state.start_work(1)?;
            let count = usize::try_from(count).map_err(|_| {
                PostProcessError::step(
                    PostProcessStep::Triangulate,
                    PostProcessStepIssue::PolygonCornerCountOutOfRange {
                        mesh: mesh_index,
                        face: face_index,
                    },
                )
            })?;
            let triangles = count.checked_sub(2).and_then(|count| count.checked_mul(3));
            triangle_index_count = triangle_index_count
                .checked_add(triangles.ok_or_else(|| {
                    PostProcessError::step(
                        PostProcessStep::Triangulate,
                        PostProcessStepIssue::TriangleIndexCountOverflow { mesh: mesh_index },
                    )
                })?)
                .ok_or_else(|| {
                    PostProcessError::step(
                        PostProcessStep::Triangulate,
                        PostProcessStepIssue::TriangleIndexCountOverflow { mesh: mesh_index },
                    )
                })?;
            state.finish_work(1)?;
        }

        let (reservation, mut triangulated) = allocate_vec::<u32>(
            state,
            triangle_index_count,
            PostProcessStep::Triangulate,
            "triangulated indices",
        )?;
        let mut cursor = 0usize;
        for (face_index, count) in mesh.face_vertex_counts.iter().copied().enumerate() {
            let count = usize::try_from(count).map_err(|_| {
                PostProcessError::step(
                    PostProcessStep::Triangulate,
                    PostProcessStepIssue::PolygonCornerCountOutOfRange {
                        mesh: mesh_index,
                        face: face_index,
                    },
                )
            })?;
            let end = cursor.checked_add(count).ok_or_else(|| {
                PostProcessError::step(
                    PostProcessStep::Triangulate,
                    PostProcessStepIssue::PolygonFaceRangeOverflow {
                        mesh: mesh_index,
                        face: face_index,
                    },
                )
            })?;
            let face = source_elements.get(cursor..end).ok_or_else(|| {
                PostProcessError::step(
                    PostProcessStep::Triangulate,
                    PostProcessStepIssue::PolygonFaceRangeInvalid {
                        mesh: mesh_index,
                        face: face_index,
                        end,
                        available: source_elements.len(),
                    },
                )
            })?;
            let anchor = face.first().copied().ok_or_else(|| {
                PostProcessError::step(
                    PostProcessStep::Triangulate,
                    PostProcessStepIssue::PolygonFaceEmpty {
                        mesh: mesh_index,
                        face: face_index,
                    },
                )
            })?;
            for index in 1..count.saturating_sub(1) {
                state.start_work(1)?;
                let second = face.get(index).copied().ok_or_else(|| {
                    PostProcessError::step(
                        PostProcessStep::Triangulate,
                        PostProcessStepIssue::PolygonCornerMissing {
                            mesh: mesh_index,
                            face: face_index,
                            corner: index,
                        },
                    )
                })?;
                let third = face.get(index + 1).copied().ok_or_else(|| {
                    PostProcessError::step(
                        PostProcessStep::Triangulate,
                        PostProcessStepIssue::PolygonCornerMissing {
                            mesh: mesh_index,
                            face: face_index,
                            corner: index + 1,
                        },
                    )
                })?;
                triangulated.extend([anchor, second, third]);
                state.finish_work(1)?;
            }
            cursor = end;
        }
        if cursor != source_elements.len() {
            return Err(PostProcessError::step(
                PostProcessStep::Triangulate,
                PostProcessStepIssue::PolygonFaceCountsMismatch {
                    mesh: mesh_index,
                    consumed: cursor,
                    available: source_elements.len(),
                },
            ));
        }

        if reservation.is_some() {
            state.ensure_output_slot(PostProcessStep::Triangulate)?;
        }
        state.retain_output(reservation);
        mesh.topology = PrimitiveTopology::Triangles;
        mesh.indices = triangulated;
        mesh.face_vertex_counts.clear();
        changed = true;
    }
    Ok(changed)
}

fn generate_normals(
    edit: &mut SceneEdit<SceneReservation>,
    state: &mut ExecutionState<'_>,
) -> StepResult<bool> {
    let mut changed = false;
    for mesh_index in 0..edit.meshes().len() {
        state.perform(|| Ok(()))?;
        let mesh = edit.meshes_mut().get_mut(mesh_index).ok_or_else(|| {
            PostProcessError::step(
                PostProcessStep::GenerateNormals,
                PostProcessStepIssue::MeshMissing { mesh: mesh_index },
            )
        })?;
        if !mesh.normals.is_empty() {
            continue;
        }
        if mesh.topology != PrimitiveTopology::Triangles {
            return Err(PostProcessError::step(
                PostProcessStep::GenerateNormals,
                PostProcessStepIssue::RequiresTriangles {
                    mesh: mesh_index,
                    topology: mesh.topology,
                },
            ));
        }

        let (_scratch_reservation, mut normals) = allocate_vec::<Option<Vec3>>(
            state,
            mesh.positions.len(),
            PostProcessStep::GenerateNormals,
            "normal scratch slots",
        )?;
        for _ in 0..mesh.positions.len() {
            state.perform(|| Ok(()))?;
            normals.push(None);
        }

        if mesh.indices.is_empty() {
            for (triangle_index, triangle) in mesh.positions.chunks_exact(3).enumerate() {
                state.start_work(1)?;
                let &[a, b, c] = triangle else {
                    return Err(PostProcessError::step(
                        PostProcessStep::GenerateNormals,
                        PostProcessStepIssue::TriangleArityMismatch {
                            mesh: mesh_index,
                            triangle: triangle_index,
                            indexed: false,
                        },
                    ));
                };
                let normal = triangle_normal(mesh_index, triangle_index, a, b, c)?;
                for vertex_offset in 0..3 {
                    let vertex_index = triangle_index
                        .checked_mul(3)
                        .and_then(|index| index.checked_add(vertex_offset))
                        .ok_or_else(|| {
                            PostProcessError::step(
                                PostProcessStep::GenerateNormals,
                                PostProcessStepIssue::VertexIndexOverflow {
                                    mesh: mesh_index,
                                    triangle: triangle_index,
                                },
                            )
                        })?;
                    let normal_count = normals.len();
                    let slot = normals.get_mut(vertex_index).ok_or_else(|| {
                        PostProcessError::step(
                            PostProcessStep::GenerateNormals,
                            PostProcessStepIssue::VertexIndexOutOfRange {
                                mesh: mesh_index,
                                triangle: triangle_index,
                                vertex: vertex_index,
                                upper_bound: normal_count,
                            },
                        )
                    })?;
                    assign_normal(slot, normal, mesh_index, vertex_index)?;
                }
                state.finish_work(1)?;
            }
        } else {
            for (triangle_index, triangle) in mesh.indices.chunks_exact(3).enumerate() {
                state.start_work(1)?;
                let &[a, b, c] = triangle else {
                    return Err(PostProcessError::step(
                        PostProcessStep::GenerateNormals,
                        PostProcessStepIssue::TriangleArityMismatch {
                            mesh: mesh_index,
                            triangle: triangle_index,
                            indexed: true,
                        },
                    ));
                };
                let indices = [a as usize, b as usize, c as usize];
                let position = |index| {
                    mesh.positions.get(index).copied().ok_or_else(|| {
                        PostProcessError::step(
                            PostProcessStep::GenerateNormals,
                            PostProcessStepIssue::VertexIndexOutOfRange {
                                mesh: mesh_index,
                                triangle: triangle_index,
                                vertex: index,
                                upper_bound: mesh.positions.len(),
                            },
                        )
                    })
                };
                let normal = triangle_normal(
                    mesh_index,
                    triangle_index,
                    position(a as usize)?,
                    position(b as usize)?,
                    position(c as usize)?,
                )?;
                for vertex_index in indices {
                    let normal_count = normals.len();
                    let slot = normals.get_mut(vertex_index).ok_or_else(|| {
                        PostProcessError::step(
                            PostProcessStep::GenerateNormals,
                            PostProcessStepIssue::VertexIndexOutOfRange {
                                mesh: mesh_index,
                                triangle: triangle_index,
                                vertex: vertex_index,
                                upper_bound: normal_count,
                            },
                        )
                    })?;
                    assign_normal(slot, normal, mesh_index, vertex_index)?;
                }
                state.finish_work(1)?;
            }
        }

        let (reservation, mut generated) = allocate_vec::<Vec3>(
            state,
            normals.len(),
            PostProcessStep::GenerateNormals,
            "generated normals",
        )?;
        for (vertex_index, normal) in normals.into_iter().enumerate() {
            state.start_work(1)?;
            generated.push(normal.ok_or_else(|| {
                PostProcessError::step(
                    PostProcessStep::GenerateNormals,
                    PostProcessStepIssue::UnreferencedVertex {
                        mesh: mesh_index,
                        vertex: vertex_index,
                    },
                )
            })?);
            state.finish_work(1)?;
        }
        if reservation.is_some() {
            state.ensure_output_slot(PostProcessStep::GenerateNormals)?;
        }
        state.retain_output(reservation);
        mesh.normals = generated;
        changed = true;
    }
    Ok(changed)
}

fn generate_bounding_boxes(
    edit: &mut SceneEdit<SceneReservation>,
    state: &mut ExecutionState<'_>,
) -> StepResult<bool> {
    let mut changed = false;
    for mesh_index in 0..edit.meshes().len() {
        state.perform(|| Ok(()))?;
        let mesh = edit.meshes_mut().get_mut(mesh_index).ok_or_else(|| {
            PostProcessError::step(
                PostProcessStep::GenerateBoundingBoxes,
                PostProcessStepIssue::MeshMissing { mesh: mesh_index },
            )
        })?;
        if mesh.bounds.is_none() {
            mesh.bounds = compute_bounds(&mesh.positions, state)?;
            changed |= mesh.bounds.is_some();
        }
    }
    Ok(changed)
}

fn compute_bounds(positions: &[Vec3], state: &mut ExecutionState<'_>) -> StepResult<Option<Aabb>> {
    let Some(first) = positions.first().copied() else {
        return Ok(None);
    };
    state.perform(|| Ok(()))?;
    let mut min = first;
    let mut max = first;
    for position in positions.iter().copied().skip(1) {
        state.start_work(1)?;
        min.x = min.x.min(position.x);
        min.y = min.y.min(position.y);
        min.z = min.z.min(position.z);
        max.x = max.x.max(position.x);
        max.y = max.y.max(position.y);
        max.z = max.z.max(position.z);
        state.finish_work(1)?;
    }
    Ok(Some(Aabb { min, max }))
}

fn triangle_normal(
    mesh_index: usize,
    triangle_index: usize,
    a: Vec3,
    b: Vec3,
    c: Vec3,
) -> StepResult<Vec3> {
    let ab = sub_vec3(b, a);
    let ac = sub_vec3(c, a);
    let normal = cross_vec3(ab, ac);
    let length_squared = dot_vec3(normal, normal);
    if length_squared <= 1.0e-20 || !length_squared.is_finite() {
        return Err(PostProcessError::step(
            PostProcessStep::GenerateNormals,
            PostProcessStepIssue::DegenerateTriangle {
                mesh: mesh_index,
                triangle: triangle_index,
            },
        ));
    }
    Ok(scale_vec3(normal, length_squared.sqrt().recip()))
}

fn assign_normal(
    slot: &mut Option<Vec3>,
    normal: Vec3,
    mesh_index: usize,
    vertex_index: usize,
) -> StepResult<()> {
    if let Some(existing) = *slot {
        if normals_compatible(existing, normal) {
            return Ok(());
        }
        return Err(PostProcessError::step(
            PostProcessStep::GenerateNormals,
            PostProcessStepIssue::ConflictingGeneratedNormal {
                mesh: mesh_index,
                vertex: vertex_index,
            },
        ));
    }
    *slot = Some(normal);
    Ok(())
}

fn normals_compatible(a: Vec3, b: Vec3) -> bool {
    let delta = sub_vec3(a, b);
    dot_vec3(delta, delta) <= 1.0e-10
}

fn allocate_vec<T>(
    state: &mut ExecutionState<'_>,
    element_count: usize,
    step: PostProcessStep,
    resource: &'static str,
) -> StepResult<(Option<BufferReservation>, Vec<T>)> {
    let plan = CapacityPlan::for_type::<T>(0, 0, element_count)
        .map_err(|error| capacity_plan_error::<T>(step, element_count, error))?;
    let admitted = plan
        .envelope_delta_bytes()
        .map_err(|error| capacity_plan_error::<T>(step, element_count, error))?;
    let mut reservation = state.reserve_staging(admitted)?;
    let mut values = Vec::new();
    plan.verify_preflight(values.len(), values.capacity())
        .map_err(|_| PostProcessError::ValidationControlInvariant {
            message: "staging capacity plan became stale before allocator entry",
        })?;
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| PostProcessError::CapacityExceeded {
            context: step.name(),
            resource,
        })?;
    let actual = plan.finish(values.len(), values.capacity()).map_err(|_| {
        PostProcessError::ValidationControlInvariant {
            message: "staging allocator violated the qualified capacity envelope",
        }
    })?;
    if let Some(token) = &mut reservation {
        state
            .coordinator
            .resize_reservation(token, actual)
            .map_err(accounting_error)?;
        if actual == 0 {
            reservation = None;
        }
    }
    Ok((reservation, values))
}

fn capacity_plan_error<T>(
    step: PostProcessStep,
    element_count: usize,
    error: CapacityPlanError,
) -> PostProcessError {
    match error {
        CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
            PostProcessError::step(
                step,
                PostProcessStepIssue::CapacityByteCountOverflow {
                    element_count,
                    element_size: size_of::<T>(),
                },
            )
        }
        _ => PostProcessError::ValidationControlInvariant {
            message: "staging capacity plan violated its source or platform invariant",
        },
    }
}

fn control_error(source: ControlCause) -> PostProcessError {
    PostProcessError::Control { source }
}

fn accounting_error(source: LedgerCause<PostProcessDimension>) -> PostProcessError {
    PostProcessError::Accounting { source }
}

fn publication_error(source: PublicationCause<PostProcessDimension>) -> PostProcessError {
    match source {
        PublicationCause::Control(source) => control_error(source),
        PublicationCause::Ledger(source) => accounting_error(source),
        source => PostProcessError::Publication { source },
    }
}

fn validation_failure_error(source: SceneValidationFailure<PostProcessError>) -> PostProcessError {
    match source {
        SceneValidationFailure::Scene(source) => PostProcessError::Validation { source },
        SceneValidationFailure::Control(source) => source,
    }
}

fn sub_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn scale_vec3(value: Vec3, scale: f32) -> Vec3 {
    Vec3::new(value.x * scale, value.y * scale, value.z * scale)
}

fn cross_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn dot_vec3(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PostProcessInitializationError;
    use baozi_core::{SceneBuilder, validate_scene};
    use baozi_operation::{LedgerEvidenceError, LedgerEvidencePurpose, LedgerLimitsError};
    use std::error::Error;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn test_coordinator(
        limits: PostProcessLimits,
    ) -> std::result::Result<Coordinator, PostProcessInitializationError> {
        let ledger_limits = limits
            .try_ledger_limits()
            .map_err(PostProcessInitializationError::ledger_limits)?;
        try_postprocess_coordinator(PostProcessControl::default(), ledger_limits)
            .map_err(PostProcessInitializationError::coordinator)
    }

    #[test]
    #[allow(
        clippy::result_large_err,
        reason = "the unwind boundary exercises the complete by-value run error"
    )]
    fn ledger_initialization_failure_is_typed_and_does_not_start_the_operation() {
        let scene = validate_scene(
            SceneBuilder::try_new()
                .expect("test scene builder allocation")
                .finish_draft(),
        )
        .expect("empty test scene should be valid");
        let generation = scene.generation();
        let node_count = scene.nodes().len();
        let observer_calls = Arc::new(AtomicUsize::new(0));
        let observed_calls = Arc::clone(&observer_calls);
        let control = PostProcessControl::default().with_progress_observer(move |_| {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            baozi_operation::ProgressDecision::Continue
        });
        let injected = LedgerLimitsError::Evidence {
            cause: LedgerEvidenceError::AllocationFailed {
                purpose: LedgerEvidencePurpose::Dimensions,
                requested_bytes: 128,
            },
        };
        let pipeline = PostProcessPipeline::new([]).expect("empty plan should compile");

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            pipeline.run_with_control_and_ledger_limits(&scene, control, Err(injected.clone()))
        }));
        let error = outcome
            .expect("ledger initialization failure must not unwind")
            .expect_err("injected ledger initialization must fail");

        assert!(matches!(
            &error,
            PostProcessRunError::Initialization {
                source: PostProcessInitializationError::LedgerLimits { source }
            } if source == &injected
        ));
        let initialization_source = Error::source(&error)
            .and_then(|source| source.downcast_ref::<PostProcessInitializationError>())
            .expect("run error must retain the typed initialization source");
        let limits_source = Error::source(initialization_source)
            .and_then(|source| source.downcast_ref::<LedgerLimitsError<PostProcessDimension>>())
            .expect("run error must retain the typed limits source");
        assert_eq!(limits_source, &injected);
        assert!(
            Error::source(limits_source)
                .and_then(|source| source.downcast_ref::<LedgerEvidenceError>())
                .is_some_and(|source| {
                    matches!(
                        source,
                        LedgerEvidenceError::AllocationFailed {
                            purpose: LedgerEvidencePurpose::Dimensions,
                            requested_bytes: 128,
                        }
                    )
                })
        );
        assert_eq!(observer_calls.load(Ordering::SeqCst), 0);
        assert_eq!(scene.generation(), generation);
        assert_eq!(scene.nodes().len(), node_count);
    }

    #[test]
    fn finalization_invariant_preserves_primary_error_and_exact_snapshot()
    -> std::result::Result<(), PostProcessInitializationError> {
        let limits = PostProcessLimits::new(8, 64);
        let mut coordinator = test_coordinator(limits)?;
        let reservation = coordinator
            .checkpoint()
            .reserve(
                PostProcessLedgerPhase::TransactionStaging,
                PostProcessDimension::StagingBytes,
                32,
            )
            .expect("test reservation should fit");
        let primary = PostProcessError::step(
            PostProcessStep::GenerateNormals,
            PostProcessStepIssue::DegenerateTriangle {
                mesh: 7,
                triangle: 11,
            },
        );

        let failure = finish_failure(coordinator, primary);

        assert_eq!(
            failure.error(),
            &PostProcessError::step(
                PostProcessStep::GenerateNormals,
                PostProcessStepIssue::DegenerateTriangle {
                    mesh: 7,
                    triangle: 11,
                },
            )
        );
        let finalization = failure
            .finalization_error()
            .expect("live storage must retain a finalization invariant");
        assert!(matches!(
            finalization.cause(),
            LedgerCause::LiveReservationsRemain { bytes: 32 }
        ));
        assert_eq!(finalization.snapshot().current_live_bytes(), 32);
        assert_eq!(failure.ledger(), finalization.snapshot());

        let (actual_primary, snapshot, finalization) = failure.into_parts();
        assert_eq!(
            actual_primary,
            PostProcessError::step(
                PostProcessStep::GenerateNormals,
                PostProcessStepIssue::DegenerateTriangle {
                    mesh: 7,
                    triangle: 11,
                },
            )
        );
        let finalization = finalization.expect("finalization failure must remain owned evidence");
        assert!(matches!(
            finalization,
            LedgerCause::LiveReservationsRemain { bytes: 32 }
        ));
        assert_eq!(snapshot.current_live_bytes(), 32);
        drop(reservation);
        Ok(())
    }

    #[test]
    fn rejected_scene_reservation_resize_preserves_the_live_token()
    -> std::result::Result<(), PostProcessInitializationError> {
        let limits = PostProcessLimits::new(8, 8);
        let mut coordinator = test_coordinator(limits)?;
        let mut state = ExecutionState::new(&mut coordinator);
        let mut reservation = state
            .reserve(SceneReservationKind::EditCandidate, 8)
            .expect("initial reservation should fit");

        let error = state
            .resize_reservation(&mut reservation, SceneReservationKind::EditCandidate, 9)
            .expect_err("growth above the frozen limit must fail");

        assert!(matches!(
            error,
            PostProcessError::Accounting {
                source: LedgerCause::LimitExceeded {
                    dimension: PostProcessDimension::StagingBytes,
                    value: 9,
                    limit: 8,
                    ..
                }
            }
        ));
        assert_eq!(
            reservation.primary.as_ref().map(BufferReservation::bytes),
            Some(8)
        );
        assert!(reservation.retained.is_vacant());

        drop(reservation);
        drop(state);
        let ledger = coordinator
            .finish()
            .expect("dropping the preserved token should finalize cleanly");
        assert_eq!(ledger.current_live_bytes(), 0);
        assert_eq!(ledger.peak_live_bytes(), 8);
        Ok(())
    }

    #[test]
    fn zero_byte_scene_reservation_retains_owner_capability()
    -> std::result::Result<(), PostProcessInitializationError> {
        let limits = PostProcessLimits::new(8, 1024 * 1024);
        let mut owner_coordinator = test_coordinator(limits)?;
        let mut owner_state = ExecutionState::new(&mut owner_coordinator);
        let mut reservation = owner_state
            .reserve(SceneReservationKind::EditCandidate, 0)
            .expect("zero-byte candidate capability should be admitted");
        assert_eq!(
            reservation.primary.as_ref().map(BufferReservation::bytes),
            Some(0)
        );
        let wrong_kind = owner_state
            .validate_reservation(&reservation, SceneReservationKind::CanonicalOutput)
            .expect_err("the owner must reject a mismatched reservation kind");
        assert!(matches!(
            wrong_kind,
            PostProcessError::ValidationControlInvariant {
                message: "core reservation validation kind did not match its live token",
            }
        ));
        assert_eq!(reservation.kind, SceneReservationKind::EditCandidate);
        assert_eq!(
            reservation.primary.as_ref().map(BufferReservation::bytes),
            Some(0)
        );
        owner_state
            .validate_reservation(&reservation, SceneReservationKind::EditCandidate)
            .expect("wrong-kind rejection must preserve the owner capability");
        drop(owner_state);

        let mut foreign_coordinator = test_coordinator(limits)?;
        let mut foreign_state = ExecutionState::new(&mut foreign_coordinator);
        let validation_error = foreign_state
            .validate_reservation(&reservation, SceneReservationKind::EditCandidate)
            .expect_err("a foreign coordinator must reject the owner capability before work");
        assert!(matches!(
            validation_error,
            PostProcessError::Accounting {
                source: LedgerCause::OwnerMismatch,
            }
        ));
        let error = foreign_state
            .resize_reservation(&mut reservation, SceneReservationKind::EditCandidate, 1)
            .expect_err("a foreign coordinator must not adopt the zero-byte capability");
        assert!(matches!(
            error,
            PostProcessError::Accounting {
                source: LedgerCause::OwnerMismatch,
            }
        ));
        assert_eq!(
            reservation.primary.as_ref().map(BufferReservation::bytes),
            Some(0)
        );

        drop(foreign_state);
        let mut owner_state = ExecutionState::new(&mut owner_coordinator);
        owner_state
            .retained_output
            .ensure_slots(
                owner_state.coordinator,
                PostProcessLedgerPhase::ValidationCommit,
                3,
                "test",
                "zero-byte owner capability evidence",
            )
            .expect("zero-byte transition slots should be admitted");
        for _ in 0..2 {
            let retained = owner_state
                .coordinator
                .checkpoint()
                .reserve(
                    PostProcessLedgerPhase::ValidationCommit,
                    PostProcessDimension::CanonicalBytes,
                    0,
                )
                .expect("additional zero-byte capability should be admitted");
            owner_state.retain_output(Some(retained));
        }
        owner_state
            .transition_reservation(
                &mut reservation,
                SceneReservationKind::EditCandidate,
                SceneReservationKind::CanonicalOutput,
                0,
            )
            .expect("zero-byte transition should preserve its owner capability");
        assert_eq!(reservation.kind, SceneReservationKind::CanonicalOutput);
        assert!(reservation.primary.is_none());
        assert_eq!(reservation.retained.values.len(), 1);
        assert_eq!(reservation.retained.values[0].bytes(), 0);
        owner_state
            .validate_reservation(&reservation, SceneReservationKind::CanonicalOutput)
            .expect("the original coordinator should retain owner validation");
        drop(owner_state);

        let mut foreign_state = ExecutionState::new(&mut foreign_coordinator);
        let error = foreign_state
            .validate_reservation(&reservation, SceneReservationKind::CanonicalOutput)
            .expect_err("transitioned zero-byte capability must still reject a foreign owner");
        assert!(matches!(
            error,
            PostProcessError::Accounting {
                source: LedgerCause::OwnerMismatch,
            }
        ));
        drop(foreign_state);
        drop(reservation);
        assert_eq!(
            owner_coordinator
                .finish()
                .expect("owner capability should roll back cleanly")
                .current_live_bytes(),
            0
        );
        assert_eq!(
            foreign_coordinator
                .finish()
                .expect("foreign ledger should remain untouched")
                .current_live_bytes(),
            0
        );
        Ok(())
    }

    #[test]
    fn zero_byte_scene_reservation_completes_publication()
    -> std::result::Result<(), PostProcessInitializationError> {
        let limits = PostProcessLimits::new(8, 1024 * 1024);
        let mut coordinator = test_coordinator(limits)?;
        let mut state = ExecutionState::new(&mut coordinator);
        state
            .begin_transform()
            .expect("test transform phase should start");
        let mut reservation = state
            .reserve(SceneReservationKind::EditCandidate, 0)
            .expect("zero-byte candidate capability should be admitted");
        state
            .retained_output
            .ensure_slots(
                state.coordinator,
                PostProcessLedgerPhase::TransactionStaging,
                3,
                "test",
                "zero-byte publication capabilities",
            )
            .expect("zero-byte publication slots should be admitted");
        for _ in 0..2 {
            let retained = state
                .coordinator
                .checkpoint()
                .reserve(
                    PostProcessLedgerPhase::TransactionStaging,
                    PostProcessDimension::StagingBytes,
                    0,
                )
                .expect("additional zero-byte capability should be admitted");
            state.retain_output(Some(retained));
        }
        state
            .begin_validation()
            .expect("test validation phase should start");
        state
            .transition_reservation(
                &mut reservation,
                SceneReservationKind::EditCandidate,
                SceneReservationKind::CanonicalOutput,
                0,
            )
            .expect("zero-byte transition should preserve one owner capability");
        assert_eq!(reservation.retained.values.len(), 1);

        let retained = std::mem::take(&mut reservation.retained);
        let primary = reservation.primary.take();
        let mut batch = retained
            .into_publication(primary)
            .expect("compacted zero-byte batch should retain capacity coverage");
        let mut publication = state
            .coordinator
            .prepare_publication(
                &mut batch,
                &[Commitment::new(PostProcessDimension::CanonicalBytes, 0)],
            )
            .expect("zero-byte publication should prepare");
        state
            .coordinator
            .publish(
                &mut publication,
                FinalProgress::new(
                    PostProcessProgressPhase::Validate,
                    PostProcessProgressUnit::WorkItems,
                    1,
                ),
            )
            .expect("zero-byte publication should commit");
        drop(publication);
        drop(reservation);
        drop(state);

        let ledger = coordinator
            .finish()
            .expect("published zero-byte ledger should finalize cleanly");
        assert_eq!(ledger.active_reservations(), 0);
        assert_eq!(ledger.current_live_bytes(), 0);
        assert_eq!(
            ledger
                .dimension(PostProcessDimension::CanonicalBytes)
                .expect("canonical dimension should be frozen")
                .committed(),
            0
        );
        Ok(())
    }

    #[test]
    fn rejected_reservation_list_growth_keeps_previous_capacity_coverage()
    -> std::result::Result<(), PostProcessInitializationError> {
        let initial_bytes = CapacityPlan::for_type::<BufferReservation>(0, 0, 4)
            .and_then(CapacityPlan::envelope_delta_bytes)
            .expect("test evidence capacity should be representable");
        let limits = PostProcessLimits::new(8, initial_bytes);
        let mut coordinator = test_coordinator(limits)?;
        let mut reservations = ReservationList::new();
        reservations
            .ensure_slots(
                &mut coordinator,
                PostProcessLedgerPhase::TransactionStaging,
                1,
                "test",
                "reservation evidence",
            )
            .expect("initial evidence allocation should fit");
        let original_capacity = reservations.values.capacity();
        let original_bytes = reservations
            .capacity
            .as_ref()
            .map(BufferReservation::bytes)
            .expect("allocated evidence must retain a capacity token");

        let error = reservations
            .ensure_slots(
                &mut coordinator,
                PostProcessLedgerPhase::TransactionStaging,
                original_capacity + 1,
                "test",
                "reservation evidence",
            )
            .expect_err("growth beyond the frozen live limit must fail");

        assert!(matches!(
            error,
            PostProcessError::Accounting {
                source: LedgerCause::LimitExceeded { .. }
            }
        ));
        assert_eq!(reservations.values.capacity(), original_capacity);
        assert_eq!(
            reservations.capacity.as_ref().map(BufferReservation::bytes),
            Some(original_bytes)
        );

        drop(reservations);
        let ledger = coordinator
            .finish()
            .expect("dropping capacity evidence should finalize cleanly");
        assert_eq!(ledger.current_live_bytes(), 0);
        Ok(())
    }

    #[test]
    fn rejected_scene_transition_remains_in_the_callers_reservation()
    -> std::result::Result<(), PostProcessInitializationError> {
        let limits = PostProcessLimits::new(8, 1024 * 1024);
        let mut coordinator = test_coordinator(limits)?;
        let mut state = ExecutionState::new(&mut coordinator);
        let mut reservation = state
            .reserve(SceneReservationKind::EditCandidate, 8)
            .expect("initial reservation should fit");

        let error = match state.transition_reservation(
            &mut reservation,
            SceneReservationKind::EditCandidate,
            SceneReservationKind::CanonicalOutput,
            9,
        ) {
            Ok(_) => panic!("an under-covered transition must fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            PostProcessError::ValidationControlInvariant { .. }
        ));
        assert_eq!(
            reservation.primary.as_ref().map(BufferReservation::bytes),
            Some(8)
        );
        assert!(reservation.retained.is_vacant());
        assert!(state.retained_output.is_vacant());

        drop(reservation);
        drop(state);
        let ledger = coordinator
            .finish()
            .expect("dropping rollback evidence should finalize cleanly");
        assert_eq!(ledger.current_live_bytes(), 0);
        assert_eq!(ledger.peak_live_bytes(), 8);
        Ok(())
    }

    #[test]
    fn failed_composite_scene_transition_restores_every_owner_and_can_retry()
    -> std::result::Result<(), PostProcessInitializationError> {
        let limits = PostProcessLimits::new(8, 1024 * 1024);
        let mut coordinator = test_coordinator(limits)?;
        let mut foreign_coordinator = test_coordinator(limits)?;
        let foreign = foreign_coordinator
            .checkpoint()
            .reserve(
                PostProcessLedgerPhase::ValidationCommit,
                PostProcessDimension::StagingBytes,
                1,
            )
            .expect("foreign reservation should fit");
        let mut state = ExecutionState::new(&mut coordinator);
        let mut reservation = state
            .reserve(SceneReservationKind::EditCandidate, 8)
            .expect("initial reservation should fit");
        state
            .retained_output
            .ensure_slots(
                state.coordinator,
                PostProcessLedgerPhase::ValidationCommit,
                3,
                "test",
                "composite transition evidence",
            )
            .expect("rollback slots should be pre-admitted");
        state.retained_output.values.push(foreign);
        let before = state
            .coordinator
            .try_ledger_snapshot()
            .expect("test evidence snapshot must fit");

        let error = state
            .transition_reservation(
                &mut reservation,
                SceneReservationKind::EditCandidate,
                SceneReservationKind::CanonicalOutput,
                8,
            )
            .expect_err("foreign batch member must reject the whole transition");

        assert!(matches!(
            error,
            PostProcessError::Accounting {
                source: LedgerCause::OwnerMismatch,
            }
        ));
        assert_eq!(
            state
                .coordinator
                .try_ledger_snapshot()
                .expect("test evidence snapshot must fit"),
            before
        );
        assert_eq!(
            reservation.primary.as_ref().map(BufferReservation::bytes),
            Some(8)
        );
        assert!(reservation.retained.is_vacant());
        assert_eq!(state.retained_output.values.len(), 1);
        assert!(state.retained_output.capacity.is_some());

        let foreign = state
            .retained_output
            .values
            .pop()
            .expect("foreign reservation remains in the original owner");
        drop(foreign);
        state
            .transition_reservation(
                &mut reservation,
                SceneReservationKind::EditCandidate,
                SceneReservationKind::CanonicalOutput,
                8,
            )
            .expect("restored composite transition can be retried");
        assert_eq!(reservation.kind, SceneReservationKind::CanonicalOutput);
        assert!(reservation.primary.is_none());
        assert_eq!(reservation.retained.values.len(), 1);
        assert!(state.retained_output.is_vacant());

        drop(reservation);
        drop(state);
        let ledger = coordinator
            .finish()
            .expect("dropping the transitioned reservation should finalize cleanly");
        assert_eq!(ledger.current_live_bytes(), 0);
        let foreign_ledger = foreign_coordinator
            .finish()
            .expect("dropping the foreign reservation should finalize cleanly");
        assert_eq!(foreign_ledger.current_live_bytes(), 0);
        Ok(())
    }
}
