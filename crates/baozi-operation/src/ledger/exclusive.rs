//! One exclusive numeric view plus disjoint shared release storage. No full-Ledger alias is lent.
use super::*;

pub(crate) struct ExclusiveWork<'op, D> {
    pub(crate) catalog: &'op [D],
    pub(crate) work: work::WorkView<'op, D>,
}

pub(crate) struct ExclusiveStorage<'op, O, P, D> {
    pub(super) state: StorageOwner<'op, P, D>,
    pub(super) phases: &'op [PhaseEntry<P>],
    pub(super) catalog: &'op [D],
    pub(super) operation_live_byte_limit: u64,
    owner: PhantomData<fn() -> O>,
}

impl<'op, O, P, D> ExclusiveStorage<'op, O, P, D> {
    pub(super) fn new(
        state: StorageOwner<'op, P, D>,
        phases: &'op [PhaseEntry<P>],
        catalog: &'op [D],
        operation_live_byte_limit: u64,
    ) -> Self {
        Self {
            state,
            phases,
            catalog,
            operation_live_byte_limit,
            owner: PhantomData,
        }
    }

    pub(crate) fn reborrow(&mut self) -> ExclusiveStorage<'_, O, P, D> {
        ExclusiveStorage {
            state: self.state.reborrow(),
            phases: self.phases,
            catalog: self.catalog,
            operation_live_byte_limit: self.operation_live_byte_limit,
            owner: PhantomData,
        }
    }

    #[cfg(test)]
    pub(crate) fn planning_state(&self) -> LedgerPlanningState {
        let state = self.state.read();
        LedgerPlanningState {
            current_live_bytes: state.current_live_bytes - state.retained_output_bytes,
            active_reservations: state.active_reservations - state.retained_output_reservations,
        }
    }

    pub(crate) fn try_grow_progress<T>(
        &mut self,
        values: &mut Vec<T>,
        desired_capacity: usize,
        retained_bytes: &mut u64,
    ) -> Result<(), LedgerEvidenceError> {
        evidence::try_grow_selected_evidence_vec(
            self.state.reborrow(),
            values,
            desired_capacity,
            retained_bytes,
            LedgerEvidencePurpose::Progress,
        )
    }
}

pub(super) type WorkAndStorage<'op, O, P, D> =
    (ExclusiveWork<'op, D>, ExclusiveStorage<'op, O, P, D>);

impl<O, P, D> Ledger<O, P, D> {
    pub(crate) fn as_read(&self) -> LedgerReadView<'_, O, P, D> {
        LedgerReadView {
            identity: self.identity,
            work: self.work.as_read(&self.work_catalog),
            storage: storage::StorageRef::Dynamic(&self.state),
            phases: &self.phase_catalogue,
            operation_live_byte_limit: self.operation_live_byte_limit,
            owner: PhantomData,
        }
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> Ledger<O, P, D> {
    pub(crate) fn view(&mut self) -> LedgerView<'_, O, P, D> {
        LedgerView::new(
            self.identity,
            self.work.view(&self.work_catalog),
            ExclusiveStorage::new(
                StorageOwner::Dynamic(&mut self.state),
                &self.phase_catalogue,
                &self.work_catalog,
                self.operation_live_byte_limit,
            ),
        )
    }

    pub(crate) fn split_internal(
        &mut self,
    ) -> (InternalWork<'_, D>, ExclusiveStorage<'_, O, P, D>) {
        self.view().into_internal()
    }
}

#[cfg(test)]
impl<D> ExclusiveWork<'_, D> {
    pub(crate) fn reborrow(&mut self) -> ExclusiveWork<'_, D> {
        ExclusiveWork {
            catalog: self.catalog,
            work: WorkView {
                catalog: self.work.catalog,
                rows: self.work.rows,
                header: self.work.header,
            },
        }
    }
}

impl<D: Copy + Ord + fmt::Debug> ExclusiveWork<'_, D> {
    pub(crate) fn reserve_operation_attribution<
        Destination: PublicationDestination,
        O: OperationOwner,
        P: Copy + Ord,
    >(
        &self,
        storage: &mut ExclusiveStorage<'_, O, P, D>,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<OperationAttributionReservation<O, P, D, Destination>, LedgerCause<D>> {
        if self.work.header.integrity_fault || self.work.rows.len() != self.work.catalog.len() {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        let admission =
            Ledger::<O, P, D>::storage_admission_for_mode::<Destination>(self.work.header.mode)?;
        Ledger::<O, P, D>::reserve_selected_core(
            storage.state.reborrow(),
            storage.operation_live_byte_limit,
            phase,
            dimension,
            bytes,
            admission,
            ReservationPositions {
                dimension: storage
                    .catalog
                    .binary_search(&dimension)
                    .map_err(|_| LedgerCause::UnknownDimension { dimension }),
                phase: storage
                    .phases
                    .iter()
                    .find(|entry| entry.phase == phase)
                    .map(|entry| entry.state_position)
                    .ok_or(LedgerCause::UnknownPhase),
            },
        )
        .map(|core| OperationAttributionReservation {
            core,
            destination: PhantomData,
        })
    }

    pub(crate) fn bind<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        let layout = WorkLayout::bind(self.catalog, &dimensions, WorkGrant::Ordinary)?;
        layout.validate_active(&self.work)?;
        Ok(layout)
    }

    #[cfg(test)]
    pub(crate) fn bind_protected<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        let layout = WorkLayout::bind(self.catalog, &dimensions, WorkGrant::Protected)?;
        layout.validate_active(&self.work)?;
        Ok(layout)
    }

    pub(crate) fn validate_layout<const N: usize>(
        &self,
        layout: WorkLayout<N>,
    ) -> Result<(), LedgerCause<D>> {
        layout.validate_active(&self.work)
    }

    pub(crate) fn charge_attempted<const N: usize>(
        &mut self,
        layout: WorkLayout<N>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        layout.validate_active(&self.work)?;
        layout.charge_attempted_in_order(&mut self.work, amounts)
    }

    pub(crate) fn complete_observed<const N: usize>(
        &mut self,
        layout: WorkLayout<N>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        layout.validate_active(&self.work)?;
        layout.consume_observed_in_order(&mut self.work, amounts)
    }

    pub(crate) fn complete_direct<const N: usize>(
        &mut self,
        layout: WorkLayout<N>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        layout.validate_active(&self.work)?;
        layout.consume_direct_atomic(&mut self.work, amounts)
    }

    pub(crate) fn charge_observed<const N: usize>(
        &mut self,
        layout: WorkLayout<N>,
        amounts: [u64; N],
    ) -> Result<(), LedgerCause<D>> {
        layout.validate_active(&self.work)?;
        layout.charge_observed_ordered(&mut self.work, amounts)
    }

    pub(crate) fn dimension_evidence<O: OperationOwner, P: Copy + Ord>(
        &self,
        storage: &ExclusiveStorage<'_, O, P, D>,
        dimension: D,
    ) -> Result<DimensionSnapshot, LedgerCause<D>> {
        self.bind([dimension])?;
        let position = self
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        Ok(dimension_snapshot(DimensionState::join(
            self.work.rows[position].state,
            storage.state.read().view().dimensions[position].state,
        )))
    }

    pub(crate) fn check_quiescent(&self) -> Result<(), LedgerCause<D>> {
        work::check_quiescent(self.work.rows, self.work.header)
    }

    pub(crate) fn tighten_policy<O: OperationOwner, P: Copy + Ord>(
        &mut self,
        storage: &mut ExclusiveStorage<'_, O, P, D>,
        dimension: D,
        policy: DimensionPolicy,
    ) -> Result<(), LedgerCause<D>> {
        self.bind([dimension])?;
        let position = self
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        let mut storage = storage.state.write();
        Ledger::<O, P, D>::tighten_policy_in(
            &mut self.work,
            &mut storage.view_mut(),
            position,
            dimension,
            policy,
        )
    }

    pub(crate) fn counters(&self, dimension: D) -> Result<(u64, u64, u64), LedgerCause<D>> {
        self.bind([dimension])?;
        let position = self
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        let state = self.work.rows[position].state;
        Ok((state.attempted, state.consumed, state.pending_consumed))
    }
}

impl<'op, O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug>
    ExclusiveStorage<'op, O, P, D>
{
    pub(crate) fn storage_observation(
        &self,
        dimension: D,
    ) -> Result<LedgerStorageObservation<'op, P, D>, LedgerCause<D>> {
        let position = self
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        Ok(LedgerStorageObservation {
            position,
            local: PhantomData,
        })
    }

    pub(crate) fn observation_live_bytes(
        &self,
        observation: LedgerStorageObservation<'_, P, D>,
    ) -> u64 {
        self.state.read().view().dimensions[observation.position]
            .state
            .current_live_bytes
    }

    pub(crate) fn validate_live_owner(
        &self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        Ledger::<O, P, D>::check_reservation_in(
            self.state.shared().ok_or(LedgerCause::OwnerMismatch)?,
            reservation,
        )
    }

    pub(crate) fn transition(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        Ledger::<O, P, D>::transition_reservations_in(
            self.state.shared().ok_or(LedgerCause::OwnerMismatch)?,
            self.operation_live_byte_limit,
            std::slice::from_mut(reservation),
            phase,
            dimension,
            bytes,
            ReservationPositions {
                dimension: self
                    .catalog
                    .binary_search(&dimension)
                    .map_err(|_| LedgerCause::UnknownDimension { dimension }),
                phase: self
                    .phases
                    .iter()
                    .find(|entry| entry.phase == phase)
                    .map(|entry| entry.state_position)
                    .ok_or(LedgerCause::UnknownPhase),
            },
        )
    }

    pub(crate) fn reserve(
        &mut self,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<LiveReservation<O, P, D>, LedgerCause<D>> {
        let positions = ReservationPositions {
            dimension: self
                .catalog
                .binary_search(&dimension)
                .map_err(|_| LedgerCause::UnknownDimension { dimension }),
            phase: self
                .phases
                .iter()
                .find(|entry| entry.phase == phase)
                .map(|entry| entry.state_position)
                .ok_or(LedgerCause::UnknownPhase),
        };
        Ledger::<O, P, D>::reserve_selected_core(
            self.state.reborrow(),
            self.operation_live_byte_limit,
            phase,
            dimension,
            bytes,
            StorageAdmission::Ordinary,
            positions,
        )
        .map(|core| LiveReservation { core })
    }

    pub(crate) fn resize(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_live_owner(reservation)?;
        Ledger::<O, P, D>::resize_core_in(
            self.state.shared().ok_or(LedgerCause::OwnerMismatch)?,
            self.operation_live_byte_limit,
            &mut reservation.core,
            bytes,
            StorageAdmission::Ordinary,
        )
    }
}
