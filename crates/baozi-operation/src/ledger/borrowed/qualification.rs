use super::super::snapshot::SnapshotRows;
use super::*;

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> TerminalBatchView<'_, O, P, D> {
    #[cfg(test)]
    pub(in crate::ledger) fn validate(
        &self,
        ledger: &LedgerReadView<'_, O, P, D>,
        rows: SnapshotRows<'_, P, D>,
        staging_lengths: (usize, usize),
    ) -> Result<Option<P>, TypedFreezeQualificationError> {
        self.validate_rows(rows, staging_lengths, || self.owners_match(ledger))
    }

    fn owners_match(&self, ledger: &LedgerReadView<'_, O, P, D>) -> bool {
        self.canonical.validate_owner_view(ledger)
            && self.evidence.validate_owner_view(ledger)
            && self.deferred_canonical_storage.is_none_or(|batch| {
                batch.validate_owner_view(ledger) && batch.is_empty_storage_shell()
            })
            && self.deferred_evidence_storage_first.is_none_or(|batch| {
                batch.validate_owner_view(ledger) && batch.is_empty_storage_shell()
            })
            && self.deferred_evidence_storage_second.is_none_or(|batch| {
                batch.validate_owner_view(ledger) && batch.is_empty_storage_shell()
            })
            && self
                .discard_canonical
                .is_none_or(|batch| batch.validate_owner_view(ledger))
            && self
                .discard_evidence
                .is_none_or(|batch| batch.validate_owner_view(ledger))
    }

    fn validate_rows(
        &self,
        rows: SnapshotRows<'_, P, D>,
        staging_lengths: (usize, usize),
        owners_match: impl FnOnce() -> bool,
    ) -> Result<Option<P>, TypedFreezeQualificationError> {
        if rows.storage.publication != PublicationState::Closing
            || rows.work_header.mode != WorkMode::Closed
        {
            return Err(TypedFreezeQualificationError::PublicationState);
        }
        if rows
            .validate_final_shape(staging_lengths.0, staging_lengths.1)
            .is_err()
        {
            return Err(TypedFreezeQualificationError::SnapshotShape);
        }
        if !owners_match() {
            return Err(TypedFreezeQualificationError::Ownership);
        }
        let storage = rows.storage;
        if preflight_typed_publication_batch(&storage, self.canonical).is_err()
            || preflight_typed_publication_batch(&storage, self.evidence).is_err()
            || self
                .deferred_canonical_storage
                .is_some_and(|batch| preflight_typed_publication_batch(&storage, batch).is_err())
            || self
                .deferred_evidence_storage_first
                .is_some_and(|batch| preflight_typed_publication_batch(&storage, batch).is_err())
            || self
                .deferred_evidence_storage_second
                .is_some_and(|batch| preflight_typed_publication_batch(&storage, batch).is_err())
            || self
                .discard_canonical
                .is_some_and(|batch| preflight_typed_discard_batch(&storage, batch).is_err())
            || self
                .discard_evidence
                .is_some_and(|batch| preflight_typed_discard_batch(&storage, batch).is_err())
        {
            return Err(TypedFreezeQualificationError::BoundPositionOutOfBounds);
        }

        let expected_live = self
            .canonical
            .live_bytes()
            .and_then(|value| value.checked_add(self.evidence.live_bytes()?))
            .and_then(|value| {
                value.checked_add(
                    self.deferred_canonical_storage
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.deferred_evidence_storage_first
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.deferred_evidence_storage_second
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.discard_canonical
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.discard_evidence
                        .map_or(Some(0), AttributionReservationBatch::live_bytes)?,
                )
            })
            .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
        let expected_live = expected_live
            .checked_add(storage.retained_output_bytes)
            .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
        if storage.current_live_bytes != expected_live {
            return Err(TypedFreezeQualificationError::GlobalLiveMismatch {
                expected: expected_live,
                actual: storage.current_live_bytes,
            });
        }

        let expected_active = self
            .canonical
            .active_count()
            .and_then(|value| value.checked_add(self.evidence.active_count()?))
            .and_then(|value| {
                value.checked_add(
                    self.deferred_canonical_storage
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.deferred_evidence_storage_first
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.deferred_evidence_storage_second
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.discard_canonical
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .and_then(|value| {
                value.checked_add(
                    self.discard_evidence
                        .map_or(Some(0), AttributionReservationBatch::active_count)?,
                )
            })
            .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
        let expected_active = expected_active
            .checked_add(storage.retained_output_reservations)
            .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
        if storage.active_reservations != expected_active {
            return Err(TypedFreezeQualificationError::ActiveReservationMismatch {
                expected: expected_active,
                actual: storage.active_reservations,
            });
        }

        for (index, entry) in storage.dimensions.iter().enumerate() {
            let expected = self
                .canonical
                .positioned_dimension_bytes(index)
                .and_then(|value| {
                    value.checked_add(self.evidence.positioned_dimension_bytes(index)?)
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_canonical_storage
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_evidence_storage_first
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_evidence_storage_second
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.discard_canonical
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.discard_evidence
                            .map_or(Some(0), |batch| batch.positioned_dimension_bytes(index))?,
                    )
                })
                .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
            let expected = expected
                .checked_add(entry.state.retained_output_bytes)
                .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
            if entry.state.current_live_bytes != expected {
                return Err(TypedFreezeQualificationError::DimensionLiveMismatch {
                    index,
                    expected,
                    actual: entry.state.current_live_bytes,
                });
            }
            if entry.state.pending_publication != 0 {
                return Err(TypedFreezeQualificationError::PendingPublication {
                    index,
                    amount: entry.state.pending_publication,
                });
            }

            let canonical_increment = self
                .canonical
                .positioned_commitment_amount(index)
                .ok_or(TypedFreezeQualificationError::CanonicalCommitmentOverflow { index })?;
            let committed = entry
                .state
                .committed
                .checked_add(canonical_increment)
                .ok_or(TypedFreezeQualificationError::CanonicalCommitmentOverflow { index })?;
            let committed_canonical_output = entry
                .state
                .committed_canonical_output
                .checked_add(canonical_increment)
                .ok_or(TypedFreezeQualificationError::CanonicalCommitmentOverflow { index })?;
            if committed > entry.policy.committed_limit {
                return Err(
                    TypedFreezeQualificationError::CanonicalCommitmentLimitExceeded {
                        index,
                        value: committed,
                        limit: entry.policy.committed_limit,
                    },
                );
            }
            if committed_canonical_output > committed {
                return Err(
                    TypedFreezeQualificationError::CanonicalCommitmentSubsetViolation {
                        index,
                        canonical: committed_canonical_output,
                        committed,
                    },
                );
            }

            let outcome_evidence_increment =
                self.evidence.positioned_commitment_amount(index).ok_or(
                    TypedFreezeQualificationError::OutcomeEvidenceCommitmentOverflow { index },
                )?;
            let published_outcome_evidence = entry
                .state
                .published_outcome_evidence
                .checked_add(outcome_evidence_increment)
                .ok_or(
                    TypedFreezeQualificationError::OutcomeEvidenceCommitmentOverflow { index },
                )?;
            if published_outcome_evidence > entry.policy.published_outcome_evidence_limit {
                return Err(
                    TypedFreezeQualificationError::OutcomeEvidenceCommitmentLimitExceeded {
                        index,
                        value: published_outcome_evidence,
                        limit: entry.policy.published_outcome_evidence_limit,
                    },
                );
            }
        }

        for (index, phase) in storage.phase_states.iter().enumerate() {
            let expected = self
                .canonical
                .positioned_phase_bytes(index)
                .and_then(|value| value.checked_add(self.evidence.positioned_phase_bytes(index)?))
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_canonical_storage
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_evidence_storage_first
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.deferred_evidence_storage_second
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.discard_canonical
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .and_then(|value| {
                    value.checked_add(
                        self.discard_evidence
                            .map_or(Some(0), |batch| batch.positioned_phase_bytes(index))?,
                    )
                })
                .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
            let expected = expected
                .checked_add(phase.retained_output_bytes)
                .ok_or(TypedFreezeQualificationError::ArithmeticOverflow)?;
            if phase.current_live_bytes != expected {
                return Err(TypedFreezeQualificationError::PhaseLiveMismatch {
                    index,
                    expected,
                    actual: phase.current_live_bytes,
                });
            }
        }

        Ok(storage.last_completed_phase.and_then(|completed| {
            rows.phases
                .iter()
                .find(|entry| entry.state_position == completed.state_position)
                .map(|entry| entry.phase)
        }))
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> LedgerView<'_, O, P, D> {
    /// Qualifies and closes the actual storage owner in one guarded interval.
    pub(crate) fn try_qualify_terminal(
        &mut self,
        batches: TerminalBatchView<'_, O, P, D>,
        known_administrative_bytes: Option<u64>,
    ) -> Result<Option<P>, TypedFreezeQualificationError> {
        let known_administrative_bytes =
            known_administrative_bytes.ok_or(TypedFreezeQualificationError::SnapshotShape)?;
        self.work
            .as_read()
            .check_integrity()
            .map_err(|_| TypedFreezeQualificationError::SnapshotShape)?;
        // These identity/token fields stay exclusively borrowed throughout qualification.
        // Observe them before mutably selecting Inline or locking the actual Shared owner.
        let owners_match = batches.owners_match(&self.as_read());
        let mut selected = self.storage.state.write();
        if selected.current_administrative_bytes != known_administrative_bytes {
            return Err(TypedFreezeQualificationError::SnapshotShape);
        }
        let last_completed_phase = batches.validate_rows(
            SnapshotRows {
                storage: selected.view(),
                phases: selected.phases(),
                work_rows: self.work.rows,
                work_header: self.work.header,
            },
            selected.snapshot_staging_lengths(),
            || owners_match,
        )?;
        selected.publication = PublicationState::TerminalQualified;
        Ok(last_completed_phase)
    }
}

#[cfg(test)]
mod tests;
