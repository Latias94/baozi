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

type WorkAndStorage<'op, O, P, D> = (ExclusiveWork<'op, D>, ExclusiveStorage<'op, O, P, D>);

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> Ledger<O, P, D> {
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
}

impl<D: Copy + Ord + fmt::Debug> ExclusiveWork<'_, D> {
    pub(crate) fn bind<const N: usize>(
        &self,
        dimensions: [D; N],
    ) -> Result<WorkLayout<N>, LedgerCause<D>> {
        let layout = WorkLayout::bind(self.catalog, &dimensions, WorkGrant::Ordinary)?;
        if self.work.header.integrity_fault {
            return Err(LedgerCause::FinalSnapshotShapeMismatch);
        }
        layout.validate_binding(self.work.header, &dimensions)?;
        Ok(layout)
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

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> ExclusiveStorage<'_, O, P, D> {
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
        if !reservation.core.is_owned_by(self.state) {
            return Err(LedgerCause::OwnerMismatch);
        }
        if !reservation.core.active {
            return Err(LedgerCause::InactiveReservation);
        }
        Ledger::<O, P, D>::resize_core_in(
            self.state,
            self.operation_live_byte_limit,
            &mut reservation.core,
            bytes,
            StorageAdmission::Ordinary,
        )
    }
}
