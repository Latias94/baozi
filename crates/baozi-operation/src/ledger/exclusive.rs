//! One exclusive numeric view plus disjoint shared release storage. No full-Ledger alias is lent.
use super::*;

pub(crate) struct ExclusiveWork<'op, D> {
    pub(crate) catalog: &'op [D],
    pub(crate) work: work::WorkView<'op, D>,
}

pub(crate) struct ExclusiveStorage<'op, O, P, D> {
    state: &'op Arc<Mutex<LedgerState<P, D>>>,
    phases: &'op [PhaseEntry<P>],
    catalog: &'op [D],
    operation_live_byte_limit: u64,
    owner: PhantomData<fn() -> O>,
}

impl<'op, O, P, D> ExclusiveStorage<'op, O, P, D> {
    pub(crate) fn reborrow(&self) -> ExclusiveStorage<'_, O, P, D> {
        ExclusiveStorage {
            state: self.state,
            phases: self.phases,
            catalog: self.catalog,
            operation_live_byte_limit: self.operation_live_byte_limit,
            owner: PhantomData,
        }
    }

    pub(crate) fn planning_state(&self) -> LedgerPlanningState {
        let state = lock(self.state);
        LedgerPlanningState {
            current_live_bytes: state.current_live_bytes - state.retained_output_bytes,
            active_reservations: state.active_reservations - state.retained_output_reservations,
        }
    }

    pub(crate) fn progress_evidence(&self) -> ProgressEvidenceStorage<'op, O, P, D> {
        ProgressEvidenceStorage::new(self.state)
    }
}

type WorkAndStorage<'op, O, P, D> = (ExclusiveWork<'op, D>, ExclusiveStorage<'op, O, P, D>);

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> Ledger<O, P, D> {
    pub(crate) fn split_internal(
        &mut self,
    ) -> (InternalWork<'_, D>, ExclusiveStorage<'_, O, P, D>) {
        let work = self.work.split_internal();
        let storage = ExclusiveStorage {
            state: &self.state,
            phases: &self.phase_catalogue,
            catalog: work.catalog,
            operation_live_byte_limit: self.operation_live_byte_limit,
            owner: PhantomData,
        };
        (work, storage)
    }

    /// Transitional storage-only adapter: numeric work is never borrowed by this view.
    pub(crate) fn storage_view(&self) -> ExclusiveStorage<'_, O, P, D> {
        ExclusiveStorage {
            state: &self.state,
            phases: &self.phase_catalogue,
            catalog: self.work.catalogue(),
            operation_live_byte_limit: self.operation_live_byte_limit,
            owner: PhantomData,
        }
    }

    pub(crate) fn exclusive(&mut self) -> Result<WorkAndStorage<'_, O, P, D>, LedgerCause<D>> {
        let (catalog, work) = self.work.exclusive()?;
        Ok((
            ExclusiveWork { catalog, work },
            ExclusiveStorage {
                state: &self.state,
                phases: &self.phase_catalogue,
                catalog,
                operation_live_byte_limit: self.operation_live_byte_limit,
                owner: PhantomData,
            },
        ))
    }

    /// Binds at the actual Internal action boundary, then lends the sole numeric state. Binding
    /// retains the existing key/mode/role precedence and never admits attempted or consumed work.
    pub(crate) fn bind_internal_work<const N: usize>(
        &mut self,
        dimensions: [D; N],
    ) -> Result<(ExclusiveWork<'_, D>, WorkLayout<N>), LedgerCause<D>> {
        let grant = match self.work.borrow_state()?.header.mode {
            WorkMode::Protected => WorkGrant::Protected,
            _ => WorkGrant::Ordinary,
        };
        let layout = self.work.bind_layout(dimensions, grant)?;
        let (catalog, work) = self.work.reborrow()?;
        layout.validate_active(&work)?;
        Ok((ExclusiveWork { catalog, work }, layout))
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
        storage: &ExclusiveStorage<'_, O, P, D>,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<OperationAttributionReservation<O, P, D, Destination>, LedgerCause<D>> {
        if self.work.header.integrity_fault || self.work.rows.len() != self.work.catalog.len() {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        let admission =
            Ledger::<O, P, D>::storage_admission_for_mode::<Destination>(self.work.header.mode)?;
        Ledger::<O, P, D>::reserve_core_in(
            storage.state,
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
            lock(storage.state).dimensions[position].state,
        )))
    }

    pub(crate) fn check_quiescent(&self) -> Result<(), LedgerCause<D>> {
        work::check_quiescent(self.work.rows, self.work.header)
    }

    pub(crate) fn tighten_policy<O: OperationOwner, P: Copy + Ord>(
        &mut self,
        storage: &ExclusiveStorage<'_, O, P, D>,
        dimension: D,
        policy: DimensionPolicy,
    ) -> Result<(), LedgerCause<D>> {
        self.bind([dimension])?;
        let position = self
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        let mut storage = lock(storage.state);
        Ledger::<O, P, D>::tighten_policy_in(
            &mut self.work,
            &mut storage,
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
            state: self.state,
            position,
            local: PhantomData,
        })
    }

    pub(crate) fn validate_live_owner(
        &self,
        reservation: &LiveReservation<O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        Ledger::<O, P, D>::check_reservation_in(self.state, reservation)
    }

    pub(crate) fn transition(
        &self,
        reservation: &mut LiveReservation<O, P, D>,
        phase: P,
        dimension: D,
        bytes: u64,
    ) -> Result<(), LedgerCause<D>> {
        Ledger::<O, P, D>::transition_reservations_in(
            self.state,
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
        &self,
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
        Ledger::<O, P, D>::reserve_core_in(
            self.state,
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
            self.state,
            self.operation_live_byte_limit,
            &mut reservation.core,
            bytes,
            StorageAdmission::Ordinary,
        )
    }
}
