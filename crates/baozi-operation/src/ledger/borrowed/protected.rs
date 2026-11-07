//! The same protected-work lifecycle operates on either concrete ledger owner.

use super::*;

impl<O, P: Copy + Ord, D: Copy + Ord> LedgerReadView<'_, O, P, D> {
    fn protected_work_position(&self, dimension: D) -> Result<usize, LedgerCause<D>> {
        let position = self
            .work
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        self.work.check_quiescent()?;
        Ok(position)
    }

    fn qualified_configuration_position(&self, dimension: D) -> Result<usize, LedgerCause<D>> {
        let position = self.protected_work_position(dimension)?;
        if self.work.header.mode != WorkMode::Ordinary
            || self.work.header.protected_dimension.is_some()
        {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        Ok(position)
    }

    #[cfg(test)]
    pub(crate) fn preflight_configure_protected_work(
        &self,
        dimension: D,
    ) -> Result<(), LedgerCause<D>> {
        self.qualified_configuration_position(dimension)?;
        Ok(())
    }

    pub(crate) fn preflight_protected_work_attachment<const N: usize>(
        &self,
        dimension: D,
        charges: &[(D, u64); N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        let position = self.protected_work_position(dimension)?;
        if self.work.header.mode != WorkMode::Ordinary
            || self.work.header.protected_dimension != Some(position)
        {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        let layout = WorkLayout::bind(
            self.work.catalog,
            &charges.map(|(dimension, _)| dimension),
            WorkGrant::Protected,
        )?;
        layout.preflight_repeated_observed(
            self.work.catalog,
            self.work.rows,
            self.work.header,
            charges.map(|(_, amount)| amount),
            repetitions,
        )
    }

    pub(crate) fn preflight_repeated_observed_batch<const N: usize>(
        &self,
        charges: &[(D, u64); N],
        repetitions: NonZeroU64,
    ) -> Result<(), LedgerCause<D>> {
        self.work.ensure_ordinary_open()?;
        self.work
            .bind_layout(charges.map(|(dimension, _)| dimension), WorkGrant::Ordinary)?
            .preflight_repeated_observed(
                self.work.catalog,
                self.work.rows,
                self.work.header,
                charges.map(|(_, amount)| amount),
                repetitions,
            )
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> LedgerView<'_, O, P, D> {
    pub(crate) fn configure_protected_work(&mut self, dimension: D) -> Result<(), LedgerCause<D>> {
        let position = self.as_read().qualified_configuration_position(dimension)?;
        self.work.header.protected_dimension = Some(position);
        Ok(())
    }

    pub(crate) fn activate_protected_work(&mut self, dimension: D) -> Result<(), LedgerCause<D>> {
        let position = self
            .work
            .catalog
            .binary_search(&dimension)
            .map_err(|_| LedgerCause::UnknownDimension { dimension })?;
        self.seal_ordinary_work()?;
        if self.work.header.mode != WorkMode::OrdinarySealed
            || self.work.header.protected_dimension != Some(position)
        {
            return Err(LedgerCause::InactiveLinearWorkPermit);
        }
        self.work.header.mode = WorkMode::Protected;
        Ok(())
    }

    pub(crate) fn transfer_failure_terminal_work(
        &mut self,
        ordinary_dimension: D,
        terminal_dimension: D,
        additional_work: u64,
    ) -> Result<(), LedgerCause<D>> {
        let position = |dimension| {
            self.work
                .catalog
                .binary_search(&dimension)
                .map_err(|_| LedgerCause::UnknownDimension { dimension })
        };
        let ordinary = position(ordinary_dimension)?;
        let terminal = position(terminal_dimension)?;
        if ordinary == terminal {
            return Err(LedgerCause::DimensionPolicyShapeMismatch {
                dimension: ordinary_dimension,
            });
        }
        self.check_work_quiescent()?;
        let storage = self.storage.state.read();
        Ledger::<O, P, D>::ensure_open_header(&storage)?;
        if self.work.header.mode != WorkMode::Ordinary {
            return Err(LedgerCause::PublicationAlreadyPending);
        }
        let storage = storage.view();
        for index in [ordinary, terminal] {
            let row = &storage.dimensions[index];
            if row.policy != StoragePolicy::from_policy(DimensionPolicy::counter(0, 0, 0)) {
                return Err(LedgerCause::DimensionPolicyShapeMismatch {
                    dimension: row.dimension,
                });
            }
            validate_dimension_state(
                row.dimension,
                row.policy.with_work(self.work.rows[index].policy),
                DimensionState::join(self.work.rows[index].state, row.state),
            )?;
        }
        let ordinary_policy = self.work.rows[ordinary].policy;
        let terminal_policy = self.work.rows[terminal].policy;
        let subtract = |class, limit: u64| {
            limit
                .checked_sub(additional_work)
                .ok_or(LedgerCause::LimitExceeded {
                    class,
                    dimension: ordinary_dimension,
                    value: additional_work,
                    limit,
                })
        };
        let add = |class, limit: u64| {
            limit
                .checked_add(additional_work)
                .ok_or(LedgerCause::ArithmeticOverflow {
                    class,
                    dimension: terminal_dimension,
                })
        };
        let ordinary_policy = WorkPolicy {
            attempted_limit: subtract(CounterClass::Attempted, ordinary_policy.attempted_limit)?,
            consumed_limit: subtract(CounterClass::Consumed, ordinary_policy.consumed_limit)?,
        };
        let terminal_policy = WorkPolicy {
            attempted_limit: add(CounterClass::Attempted, terminal_policy.attempted_limit)?,
            consumed_limit: add(CounterClass::Consumed, terminal_policy.consumed_limit)?,
        };
        for (index, policy) in [(ordinary, ordinary_policy), (terminal, terminal_policy)] {
            let row = &storage.dimensions[index];
            validate_dimension_state(
                row.dimension,
                row.policy.with_work(policy),
                DimensionState::join(self.work.rows[index].state, row.state),
            )?;
        }
        self.work.rows[ordinary].policy = ordinary_policy;
        self.work.rows[terminal].policy = terminal_policy;
        Ok(())
    }
}
