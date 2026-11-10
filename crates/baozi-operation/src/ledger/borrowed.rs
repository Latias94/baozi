//! Short loans of the actual finite or dynamic ledger fields.

use super::storage::StorageRef;
use super::work::WorkReadView;

mod protected;
mod qualification;
mod terminal;
use super::*;

pub(crate) struct LedgerReadView<'loan, O, P, D> {
    pub(super) identity: LedgerIdentity,
    pub(super) work: WorkReadView<'loan, D>,
    pub(super) storage: StorageRef<'loan, P, D>,
    pub(super) phases: &'loan [PhaseEntry<P>],
    pub(super) operation_live_byte_limit: u64,
    pub(super) owner: PhantomData<fn() -> O>,
}

pub(crate) struct LedgerView<'loan, O, P, D> {
    identity: LedgerIdentity,
    pub(super) work: WorkView<'loan, D>,
    pub(super) storage: ExclusiveStorage<'loan, O, P, D>,
}

impl<'loan, O, P, D> LedgerView<'loan, O, P, D> {
    pub(super) fn new(
        identity: LedgerIdentity,
        work: WorkView<'loan, D>,
        storage: ExclusiveStorage<'loan, O, P, D>,
    ) -> Self {
        Self {
            identity,
            work,
            storage,
        }
    }

    pub(crate) fn storage_view(&mut self) -> ExclusiveStorage<'_, O, P, D> {
        self.storage.reborrow()
    }

    pub(crate) fn as_read(&self) -> LedgerReadView<'_, O, P, D> {
        LedgerReadView {
            identity: self.identity,
            work: self.work.as_read(),
            storage: self.storage.state.as_read(),
            phases: self.storage.phases,
            operation_live_byte_limit: self.storage.operation_live_byte_limit,
            owner: PhantomData,
        }
    }

    pub(crate) fn reborrow(&mut self) -> LedgerView<'_, O, P, D> {
        LedgerView::new(self.identity, self.work.reborrow(), self.storage.reborrow())
    }

    /// Binds keys, mode and roles before lending the original work; binding admits no work.
    pub(crate) fn bind_internal_work<const N: usize>(
        self,
        dimensions: [D; N],
    ) -> Result<(ExclusiveWork<'loan, D>, WorkLayout<N>), LedgerCause<D>>
    where
        D: Copy + Ord,
    {
        let grant = match self.work.header.mode {
            WorkMode::Protected => WorkGrant::Protected,
            _ => WorkGrant::Ordinary,
        };
        let layout = self.work.as_read().bind_layout(dimensions, grant)?;
        layout.validate_active(&self.work)?;
        Ok((
            ExclusiveWork {
                catalog: self.work.catalog,
                work: self.work,
            },
            layout,
        ))
    }

    pub(crate) fn into_internal(
        self,
    ) -> (InternalWork<'loan, D>, ExclusiveStorage<'loan, O, P, D>) {
        (InternalWork::new(self.work), self.storage)
    }

    pub(crate) fn into_exclusive(
        self,
    ) -> Result<super::exclusive::WorkAndStorage<'loan, O, P, D>, LedgerCause<D>> {
        self.check_work_quiescent()?;
        self.work.as_read().ensure_active(WorkGrant::Ordinary)?;
        Ok((
            ExclusiveWork {
                catalog: self.work.catalog,
                work: self.work,
            },
            self.storage,
        ))
    }

    pub(crate) fn ensure_work_open(&self) -> Result<(), LedgerCause<D>> {
        self.work.as_read().ensure_ordinary_open()
    }

    pub(crate) fn check_work_quiescent(&self) -> Result<(), LedgerCause<D>> {
        self.work.as_read().check_quiescent()
    }

    pub(crate) fn seal_ordinary_work(&mut self) -> Result<(), LedgerCause<D>> {
        self.check_work_quiescent()?;
        let mut storage = self.storage.state.write();
        if self.work.header.mode == WorkMode::Ordinary {
            if storage.publication == PublicationState::Open {
                storage.publication = PublicationState::Closing;
            }
            self.work.header.mode = WorkMode::OrdinarySealed;
        }
        Ok(())
    }

    pub(crate) fn close_work(&mut self) -> Result<(), LedgerCause<D>> {
        self.check_work_quiescent()?;
        match self.work.header.mode {
            WorkMode::Ordinary => return Err(LedgerCause::InactiveLinearWorkPermit),
            WorkMode::OrdinarySealed | WorkMode::Protected | WorkMode::Closed => {
                self.work.header.mode = WorkMode::Closed;
            }
        }
        Ok(())
    }
}

impl<O, P: Copy + Ord, D: Copy + Ord> LedgerReadView<'_, O, P, D> {
    pub(crate) fn ordinary_work_is_sealed(&self) -> bool {
        self.work.header.mode != WorkMode::Ordinary
    }

    /// Checks reservation headroom under the live policy without reserving it.
    /// A later reservation still needs its ordinary failure containment if another owner wins a race.
    pub(crate) fn preflight_reservation_batch(
        &self,
        phase: P,
        dimension: D,
        bytes: u64,
        reservation_count: NonZeroU64,
    ) -> Result<(), LedgerCause<D>>
    where
        O: OperationOwner,
        D: fmt::Debug,
    {
        let dimension_position = self
            .work
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension });
        let phase_position = self
            .phases
            .iter()
            .find(|entry| entry.phase == phase)
            .map(|entry| entry.state_position)
            .ok_or(LedgerCause::UnknownPhase);
        let storage = self.storage.read();
        Ledger::<O, P, D>::project_reservation_in(
            self.operation_live_byte_limit,
            &storage.view(),
            dimension,
            bytes,
            reservation_count,
            StorageAdmission::Ordinary,
            ReservationPositions {
                dimension: dimension_position,
                phase: phase_position,
            },
        )
        .map(drop)
    }

    pub(crate) fn planning_state(&self) -> LedgerPlanningState {
        let storage = self.storage.read();
        let state = storage.view();
        LedgerPlanningState {
            current_live_bytes: state.current_live_bytes - state.retained_output_bytes,
            active_reservations: state.active_reservations - state.retained_output_reservations,
        }
    }

    pub(crate) fn identity(&self) -> LedgerIdentity {
        self.identity
    }

    pub(crate) fn check_work_quiescent(&self) -> Result<(), LedgerCause<D>> {
        self.work.check_quiescent()
    }

    pub(crate) fn contains_dimension(&self, dimension: D) -> bool {
        self.work.catalog.binary_search(&dimension).is_ok()
    }

    pub(crate) fn contains_phase(&self, phase: P) -> bool {
        self.phases.iter().any(|entry| entry.phase == phase)
    }

    pub(crate) fn dimension_count(&self) -> usize {
        self.work.catalog.len()
    }

    pub(crate) fn dimension_at(&self, index: usize) -> Option<D> {
        self.work.catalog.get(index).copied()
    }

    pub(crate) fn phase_at(&self, index: usize) -> Option<P> {
        self.phases.get(index).map(|entry| entry.phase)
    }

    pub(crate) fn phase_count(&self) -> usize {
        self.phases.len()
    }

    pub(crate) fn catalog_shape_is_strictly_ordered(
        &self,
        expected_dimensions: usize,
        expected_phases: usize,
    ) -> bool {
        self.work.catalog.len() == expected_dimensions
            && self.phases.len() == expected_phases
            && self.work.catalog.windows(2).all(|pair| pair[0] < pair[1])
            && self
                .phases
                .windows(2)
                .all(|pair| pair[0].phase < pair[1].phase)
    }

    pub(crate) fn dimension_evidence(
        &self,
        dimension: D,
    ) -> Result<DimensionSnapshot, LedgerCause<D>> {
        let position = self
            .work
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        self.work.check_integrity()?;
        let storage = self.storage.read();
        Ok(dimension_snapshot(DimensionState::join(
            self.work.rows[position].state,
            storage.view().dimensions[position].state,
        )))
    }

    pub(crate) fn phase_evidence(&self, phase: P) -> Option<PhaseSnapshot> {
        let position = self
            .phases
            .iter()
            .find(|entry| entry.phase == phase)?
            .state_position;
        self.storage
            .read()
            .view()
            .phase_states
            .get(position)
            .copied()
            .map(phase_snapshot)
    }

    pub(crate) fn terminal_residue_cause(&self) -> Option<LedgerCause<D>> {
        let state = self.storage.read();
        let (dimension_staging_len, phase_staging_len) = state.snapshot_staging_lengths();
        self.work
            .check_integrity()
            .err()
            .or_else(|| {
                snapshot::SnapshotRows {
                    storage: state.view(),
                    phases: state.phases(),
                    work_rows: self.work.rows,
                    work_header: self.work.header,
                }
                .validate_final_shape(dimension_staging_len, phase_staging_len)
                .err()
            })
            .or_else(|| {
                (state.current_live_bytes != 0).then_some(LedgerCause::LiveReservationsRemain {
                    bytes: state.current_live_bytes,
                })
            })
            .or_else(|| {
                matches!(
                    state.publication,
                    PublicationState::Pending | PublicationState::TerminalQualified
                )
                .then_some(LedgerCause::PendingPublicationsRemain)
            })
            .or_else(|| {
                (state.active_reservations != 0).then_some(LedgerCause::ActiveReservationsRemain {
                    count: state.active_reservations,
                })
            })
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> LedgerView<'_, O, P, D> {
    pub(crate) fn try_complete_phase(&mut self, phase: P) -> Result<(), PhaseCompletionError<P>> {
        let position = self
            .storage
            .phases
            .iter()
            .find(|entry| entry.phase == phase)
            .map(|entry| entry.state_position)
            .ok_or(LedgerCause::UnknownPhase);
        let mut state = self.storage.state.write();
        match state.publication {
            PublicationState::Open => {}
            PublicationState::Closing | PublicationState::Pending => {
                return Err(PhaseCompletionError::PublicationPending);
            }
            PublicationState::TerminalQualified | PublicationState::Published => {
                return Err(PhaseCompletionError::PublicationCompleted);
            }
        }
        if !state.phase_positions_are_valid() {
            return Err(PhaseCompletionError::CatalogShapeMismatch);
        }
        Ledger::<O, P, D>::complete_phase_in(&mut state.view_mut(), phase, position)
    }

    pub(crate) fn try_attribution_batch_from_plan<Destination: PublicationDestination>(
        &mut self,
        plan: AttributionBatchPlan<O, P, D, Destination>,
    ) -> Result<AttributionReservationBatch<O, P, D, Destination>, AttributionBatchCreateError<D>>
    {
        #[cfg(test)]
        ATTRIBUTION_BATCH_ALLOCATION_ENTRIES.with(|entries| {
            entries.set(entries.get().saturating_add(1));
        });
        let AttributionBatchPlan {
            phase,
            capacity_dimension,
            attribution_capacity,
            commitment_capacity,
            planned_bytes,
            attribution_limit,
            commitment_limit,
            destination: _,
        } = plan;
        self.work
            .as_read()
            .check_integrity()
            .map_err(AttributionBatchCreateError::Ledger)?;
        let admission =
            Ledger::<O, P, D>::storage_admission_for_mode::<Destination>(self.work.header.mode)
                .map_err(AttributionBatchCreateError::Ledger)?;
        let mut capacity_cover = if planned_bytes == 0 {
            None
        } else {
            let positions = ReservationPositions {
                dimension: self
                    .work
                    .catalog
                    .binary_search(&capacity_dimension)
                    .map_err(|_| LedgerCause::UnknownDimension {
                        dimension: capacity_dimension,
                    }),
                phase: self
                    .storage
                    .phases
                    .iter()
                    .find(|entry| entry.phase == phase)
                    .map(|entry| entry.state_position)
                    .ok_or(LedgerCause::UnknownPhase),
            };
            Some(StorageCoveringReservation {
                core: Ledger::<O, P, D>::reserve_selected_core(
                    self.storage.state.reborrow(),
                    self.storage.operation_live_byte_limit,
                    phase,
                    capacity_dimension,
                    planned_bytes,
                    admission,
                    positions,
                )
                .map_err(AttributionBatchCreateError::Ledger)?,
            })
        };
        let mut attributions = Vec::new();
        attributions
            .try_reserve_exact(attribution_capacity.allocator_additional())
            .map_err(|_| AttributionBatchCreateError::AllocationFailed)?;
        let mut commitments = Vec::new();
        commitments
            .try_reserve_exact(commitment_capacity.allocator_additional())
            .map_err(|_| AttributionBatchCreateError::AllocationFailed)?;
        let attribution_bytes = attribution_capacity
            .finish(attributions.len(), attributions.capacity())
            .map_err(|_| AttributionBatchCreateError::CapacityOverflow)?;
        let commitment_bytes = commitment_capacity
            .finish(commitments.len(), commitments.capacity())
            .map_err(|_| AttributionBatchCreateError::CapacityOverflow)?;
        let actual_bytes = attribution_bytes
            .checked_add(commitment_bytes)
            .ok_or(AttributionBatchCreateError::CapacityOverflow)?;
        if actual_bytes > planned_bytes {
            return Err(AttributionBatchCreateError::CapacityOverflow);
        }
        match (&mut capacity_cover, actual_bytes) {
            (Some(_), 0) => {
                drop(capacity_cover.take());
            }
            (Some(cover), bytes) => Ledger::<O, P, D>::resize_core_in(
                self.storage
                    .state
                    .shared()
                    .ok_or(AttributionBatchCreateError::Ledger(
                        LedgerCause::OwnerMismatch,
                    ))?,
                self.storage.operation_live_byte_limit,
                &mut cover.core,
                bytes,
                admission,
            )
            .map_err(AttributionBatchCreateError::Ledger)?,
            (None, 0) => {}
            (None, _) => return Err(AttributionBatchCreateError::CapacityOverflow),
        }
        let owner = if capacity_cover.is_none() {
            reservation::AttributionBatchOwner::Empty(self.identity)
        } else {
            Arc::clone(
                self.storage
                    .state
                    .shared()
                    .expect("a capacity cover retains the installed shared owner"),
            )
            .into()
        };
        Ok(AttributionReservationBatch {
            attributions,
            commitments,
            capacity_cover,
            attribution_limit,
            commitment_limit,
            owner,
        })
    }

    pub(crate) fn tighten_dimension_policy(
        &mut self,
        dimension: D,
        upper_bound: DimensionPolicy,
    ) -> Result<(), LedgerCause<D>> {
        let position = self
            .work
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        let mut storage = self.storage.state.write();
        Ledger::<O, P, D>::tighten_policy_in(
            &mut self.work,
            &mut storage.view_mut(),
            position,
            dimension,
            upper_bound,
        )
    }

    pub(crate) fn charge_attempted(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        let layout = self
            .work
            .as_read()
            .bind_layout([dimension], WorkGrant::Ordinary)?;
        layout.charge_attempted_in_order(&mut self.work, [amount])
    }

    pub(crate) fn charge_producer_attempted(
        &mut self,
        dimension: D,
        amount: u64,
        protected_dimension: Option<D>,
    ) -> Result<(), LedgerCause<D>> {
        if protected_dimension == Some(dimension) {
            return Err(LedgerCause::UnknownDimension { dimension });
        }
        let layout = self
            .work
            .as_read()
            .bind_layout([dimension], WorkGrant::Ordinary)?;
        layout.charge_attempted_in_order(&mut self.work, [amount])
    }

    pub(crate) fn charge_consumed(
        &mut self,
        dimension: D,
        amount: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        let layout = self
            .work
            .as_read()
            .bind_layout([dimension], WorkGrant::Ordinary)?;
        layout.consume_observed_in_order(&mut self.work, [amount])
    }

    pub(crate) fn charge_observed_batch(
        &mut self,
        charges: &[(D, u64)],
    ) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        for &(dimension, amount) in charges {
            let layout = self
                .work
                .as_read()
                .bind_layout([dimension], WorkGrant::Ordinary)?;
            layout.charge_observed_ordered(&mut self.work, [amount])?;
        }
        Ok(())
    }

    pub(crate) fn charge_consumed_batch<const N: usize>(
        &mut self,
        charges: &[(D, u64); N],
    ) -> Result<(), LedgerCause<D>> {
        self.ensure_work_open()?;
        let mut previous = None;
        for &(dimension, _) in charges {
            let position = self
                .work
                .catalog
                .binary_search(&dimension)
                .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
            if let Some((previous_position, previous_dimension)) = previous
                && position < previous_position
            {
                return Err(LedgerCause::ConsumedBatchOutOfOrder {
                    previous: previous_dimension,
                    dimension,
                });
            }
            previous = Some((position, dimension));
        }
        let layout = self
            .work
            .as_read()
            .bind_layout(charges.map(|(dimension, _)| dimension), WorkGrant::Ordinary)?;
        layout.consume_direct_atomic(&mut self.work, charges.map(|(_, amount)| amount))
    }

    pub(crate) fn charge_producer_consumed_batch<const N: usize>(
        &mut self,
        charges: &[(D, u64); N],
        protected_dimension: Option<D>,
    ) -> Result<(), LedgerCause<D>> {
        for &(dimension, _) in charges {
            if protected_dimension == Some(dimension)
                || self.work.catalog.binary_search(&dimension).is_err()
            {
                return Err(LedgerCause::UnknownDimension { dimension });
            }
        }
        let bound = self
            .work
            .as_read()
            .bind_layout(charges.map(|(dimension, _)| dimension), WorkGrant::Ordinary)?;
        self.ensure_work_open()?;
        for pair in charges.windows(2) {
            if pair[1].0 < pair[0].0 {
                return Err(LedgerCause::ConsumedBatchOutOfOrder {
                    previous: pair[0].0,
                    dimension: pair[1].0,
                });
            }
        }
        bound.consume_direct_atomic(&mut self.work, charges.map(|(_, amount)| amount))
    }

    pub(crate) fn reserve(
        &mut self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        self.storage.reserve(phase, dimension, bytes)
    }

    pub(crate) fn resize_reservation(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.storage.resize(reservation, bytes)
    }

    pub(crate) fn transition_reservation(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.storage
            .transition(reservation, phase, dimension, bytes)
    }

    pub(crate) fn validate_live_reservation(
        &self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.storage.validate_live_owner(reservation)
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> LedgerReadView<'_, O, P, D> {
    pub(crate) fn try_push_commitment<Destination: PublicationDestination>(
        &self,
        batch: &mut AttributionReservationBatch<O, P, D, Destination>,
        mut commitment: OutputCommitment<D, Destination>,
    ) -> Result<(), CommitmentPushError<D, Destination>> {
        if !batch.validate_owner_view(self) {
            return Err(CommitmentPushError::Ledger {
                commitment,
                cause: LedgerCause::OwnerMismatch,
            });
        }
        if !batch.has_commitment_slot() {
            return Err(CommitmentPushError::BatchFull(commitment));
        }
        let admission = self.work.check_integrity().and_then(|()| {
            Ledger::<O, P, D>::storage_admission_for_mode::<Destination>(self.work.header.mode)
        });
        let admission = match admission {
            Ok(admission) => admission,
            Err(cause) => return Err(CommitmentPushError::Ledger { commitment, cause }),
        };
        let bound_position = self
            .work
            .catalog
            .binary_search(&commitment.dimension)
            .map_err(|_| LedgerCause::UnknownDimension {
                dimension: commitment.dimension,
            });
        let state = self.storage.read();
        if let Err(cause) = Ledger::<O, P, D>::ensure_storage_admission_header(&state, admission) {
            return Err(CommitmentPushError::Ledger { commitment, cause });
        }
        let dimension_position = match bound_position {
            Ok(position) => position,
            Err(cause) => return Err(CommitmentPushError::Ledger { commitment, cause }),
        };
        commitment.dimension_position = dimension_position;
        batch.push_positioned_commitment(commitment);
        Ok(())
    }
}
