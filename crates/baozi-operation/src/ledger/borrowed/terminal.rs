use super::super::snapshot::SnapshotRows;
use super::super::storage::SelectedStorageRead;
use super::*;

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord + fmt::Debug> LedgerReadView<'_, O, P, D> {
    pub(crate) fn validate_typed_batch_pair<Destination>(
        &self,
        first: &AttributionReservationBatch<O, P, D, Destination>,
        second: &AttributionReservationBatch<O, P, D, Destination>,
    ) -> Result<(), LedgerCause<D>>
    where
        Destination: DestinationState,
    {
        if !first.validate_owner_view(self) || !second.validate_owner_view(self) {
            return Err(LedgerCause::OwnerMismatch);
        }
        validate_typed_commitment_pair(self.storage.read().view().dimensions, first, second)
    }

    pub(crate) fn try_bind_fallback_rows(
        &self,
        baseline: FallbackBatchView<'_, O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.try_bind_fallback_rows_with(
            baseline.as_read(),
            FallbackBaselineQualification::Terminal,
        )
    }

    pub(crate) fn try_attach_fallback_rows(
        &self,
        baseline: FallbackBatchView<'_, O, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        self.try_bind_fallback_rows_with(
            baseline.as_read(),
            FallbackBaselineQualification::Attachment,
        )
    }

    fn try_bind_fallback_rows_with(
        &self,
        baseline: FallbackBatchReadView<'_, O, P, D>,
        qualification: FallbackBaselineQualification,
    ) -> Result<(), LedgerCause<D>> {
        match qualification {
            FallbackBaselineQualification::Attachment => self.work.ensure_ordinary_open()?,
            FallbackBaselineQualification::Terminal => self.work.check_integrity()?,
        }
        let selected = self.storage.read();
        self.validate_fallback_baseline_locked(&selected, &baseline, qualification)?;
        let state = selected.view();
        preflight_typed_publication_batch(&state, baseline.canonical_assembly)?;
        preflight_typed_publication_batch(&state, baseline.evidence_assembly)?;
        preflight_typed_publication_batch(&state, baseline.fallback)?;
        if let Some(evidence) = baseline.numerical_evidence {
            preflight_typed_publication_batch(&state, evidence)?;
        }
        Ok(())
    }

    fn validate_fallback_baseline_locked(
        &self,
        selected: &SelectedStorageRead<'_, P, D>,
        baseline: &FallbackBatchReadView<'_, O, P, D>,
        qualification: FallbackBaselineQualification,
    ) -> Result<(), LedgerCause<D>> {
        let state = selected.view();
        if !baseline.canonical_assembly.validate_owner_view(self)
            || !baseline.evidence_assembly.validate_owner_view(self)
            || !baseline.fallback.validate_owner_view(self)
            || baseline
                .numerical_evidence
                .is_some_and(|batch| !batch.validate_owner_view(self))
        {
            return Err(LedgerCause::OwnerMismatch);
        }
        if !baseline
            .evidence_assembly
            .can_absorb_sources([baseline.numerical_evidence, Some(baseline.fallback)])
        {
            return Err(LedgerCause::ActiveReservationsRemain { count: u64::MAX });
        }
        let expected_active = baseline
            .canonical_assembly
            .active_count()
            .and_then(|count| {
                baseline
                    .evidence_assembly
                    .active_count()?
                    .checked_add(count)
            })
            .and_then(|count| baseline.fallback.active_count()?.checked_add(count))
            .and_then(|count| {
                baseline
                    .numerical_evidence
                    .map_or(Some(0), AttributionReservationBatch::active_count)?
                    .checked_add(count)
            })
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let expected_live = baseline
            .canonical_assembly
            .live_bytes()
            .and_then(|bytes| baseline.evidence_assembly.live_bytes()?.checked_add(bytes))
            .and_then(|bytes| baseline.fallback.live_bytes()?.checked_add(bytes))
            .and_then(|bytes| {
                baseline
                    .numerical_evidence
                    .map_or(Some(0), AttributionReservationBatch::live_bytes)?
                    .checked_add(bytes)
            })
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        match qualification {
            FallbackBaselineQualification::Attachment => {
                Ledger::<O, P, D>::ensure_open_header(state.header)?;
                let (dimension_staging_len, phase_staging_len) =
                    selected.snapshot_staging_lengths();
                if state.dimensions.len() != self.work.rows.len()
                    || state.dimensions.len() != dimension_staging_len
                    || !storage::phase_positions_are_valid(
                        selected.phases(),
                        state.phase_states.len(),
                        phase_staging_len,
                        state
                            .last_completed_phase
                            .as_ref()
                            .map(|completed| completed.state_position),
                    )
                {
                    return Err(LedgerCause::FinalSnapshotShapeMismatch);
                }
            }
            FallbackBaselineQualification::Terminal => {
                Ledger::<O, P, D>::ensure_terminal_preflight(state.header, self.work.header)?;
                self.validate_terminal_snapshot_shape(selected)?;
            }
        }
        let covered_active = expected_active
            .checked_add(state.retained_output_reservations)
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        if !qualification.accepts_coverage(state.active_reservations, covered_active) {
            return Err(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            });
        }
        let covered_live = expected_live
            .checked_add(state.retained_output_bytes)
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        if !qualification.accepts_coverage(state.current_live_bytes, covered_live) {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: expected_live,
                ledger_bytes: state.current_live_bytes,
            });
        }
        validate_typed_commitments(state.dimensions, baseline.canonical_assembly)?;
        validate_typed_commitment_batches(
            state.dimensions,
            [
                Some(baseline.evidence_assembly),
                baseline.numerical_evidence,
                Some(baseline.fallback),
            ],
        )
    }

    /// Attempts to bind the typed primary publication graph without consuming its owners.
    ///
    /// An ordinary `Err` is transactional: the ledger, `baseline`, and every supplied
    /// canonical or outcome-evidence source batch remain unchanged. In particular, their
    /// reservations, capacity covers, and commitment entries remain owned by the caller and may
    /// be reused for a later compatible bind attempt. The first mutation is source absorption,
    /// which occurs only after every validation and preflight check succeeds.
    ///
    /// A panic is not an ordinary rejection; callers must contain it before deciding whether any
    /// retained physical ownership can be released.
    pub(crate) fn try_bind_primary_rows(
        &self,
        baseline: FallbackBatchView<'_, O, P, D>,
        canonical_source: Option<&mut AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<
            &mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        >,
        publish_canonical: bool,
    ) -> Result<(), LedgerCause<D>> {
        self.work.check_integrity()?;
        let state = self.storage.read();
        self.preflight_primary_publication_locked(
            &state,
            &baseline.as_read(),
            canonical_source.as_deref(),
            evidence_source.as_deref(),
            publish_canonical,
            None,
            None,
        )?;
        drop(state);

        if publish_canonical && let Some(source) = canonical_source {
            baseline
                .canonical_assembly
                .absorb_proven_retaining_source_storage(source);
        }
        if let Some(evidence) = baseline.numerical_evidence {
            baseline
                .evidence_assembly
                .absorb_proven_retaining_source_storage(evidence);
        }
        if let Some(source) = evidence_source {
            baseline
                .evidence_assembly
                .absorb_proven_retaining_source_storage(source);
        }
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the private preflight keeps the exact primary graph and optional U4 pair explicit"
    )]
    fn preflight_primary_publication_locked(
        &self,
        selected: &SelectedStorageRead<'_, P, D>,
        baseline: &FallbackBatchReadView<'_, O, P, D>,
        canonical_source: Option<&AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<&AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
        publish_canonical: bool,
        canonical_extra: Option<&OutputCommitment<D, CanonicalOutput>>,
        evidence_extra: Option<&OutputCommitment<D, OutcomeEvidenceDestination>>,
    ) -> Result<(), LedgerCause<D>> {
        self.validate_primary_publication_locked(
            selected,
            baseline,
            canonical_source,
            evidence_source,
            publish_canonical,
            canonical_extra,
            evidence_extra,
        )?;
        let state = selected.view();
        preflight_typed_publication_batch(&state, baseline.canonical_assembly)?;
        preflight_typed_publication_batch(&state, baseline.evidence_assembly)?;
        if let Some(evidence) = baseline.numerical_evidence {
            preflight_typed_publication_batch(&state, evidence)?;
        }
        preflight_typed_discard_batch(&state, baseline.fallback)?;
        if let Some(batch) = canonical_source {
            if publish_canonical {
                preflight_typed_publication_batch(&state, batch)?;
            } else {
                preflight_typed_discard_batch(&state, batch)?;
            }
        }
        if let Some(batch) = evidence_source {
            preflight_typed_publication_batch(&state, batch)?;
        }
        Ok(())
    }

    pub(crate) fn try_bind_fallback_discard_rows(
        &self,
        baseline: FallbackBatchView<'_, O, P, D>,
        canonical_discard: Option<&mut AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_discard: Option<
            &mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        >,
    ) -> Result<(), LedgerCause<D>> {
        if !baseline.canonical_assembly.validate_owner_view(self)
            || !baseline.evidence_assembly.validate_owner_view(self)
            || !baseline.fallback.validate_owner_view(self)
            || baseline
                .numerical_evidence
                .as_deref()
                .is_some_and(|batch| !batch.validate_owner_view(self))
            || canonical_discard
                .as_deref()
                .is_some_and(|batch| !batch.validate_owner_view(self))
            || evidence_discard
                .as_deref()
                .is_some_and(|batch| !batch.validate_owner_view(self))
        {
            return Err(LedgerCause::OwnerMismatch);
        }
        if !baseline.evidence_assembly.can_absorb_sources([
            baseline.numerical_evidence.as_deref(),
            Some(&*baseline.fallback),
        ]) {
            return Err(LedgerCause::ActiveReservationsRemain { count: u64::MAX });
        }

        self.work.check_integrity()?;
        let selected = self.storage.read();
        Ledger::<O, P, D>::ensure_terminal_preflight(&selected, self.work.header)?;
        self.validate_terminal_snapshot_shape(&selected)?;
        let state = selected.view();
        if let Some(batch) = canonical_discard.as_deref() {
            preflight_typed_discard_batch(&state, batch)?;
        }
        if let Some(batch) = evidence_discard.as_deref() {
            preflight_typed_discard_batch(&state, batch)?;
        }
        drop(selected);

        if let Some(evidence) = baseline.numerical_evidence {
            baseline
                .evidence_assembly
                .absorb_proven_retaining_source_storage(evidence);
        }
        baseline
            .evidence_assembly
            .absorb_proven_retaining_source_storage(baseline.fallback);
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the private validation keeps the exact primary graph and optional U4 pair explicit"
    )]
    fn validate_primary_publication_locked(
        &self,
        selected: &SelectedStorageRead<'_, P, D>,
        baseline: &FallbackBatchReadView<'_, O, P, D>,
        canonical_source: Option<&AttributionReservationBatch<O, P, D, CanonicalOutput>>,
        evidence_source: Option<&AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
        publish_canonical: bool,
        canonical_extra: Option<&OutputCommitment<D, CanonicalOutput>>,
        evidence_extra: Option<&OutputCommitment<D, OutcomeEvidenceDestination>>,
    ) -> Result<(), LedgerCause<D>> {
        let state = selected.view();
        if !baseline.canonical_assembly.validate_owner_view(self)
            || !baseline.evidence_assembly.validate_owner_view(self)
            || !baseline.fallback.validate_owner_view(self)
            || baseline
                .numerical_evidence
                .is_some_and(|batch| !batch.validate_owner_view(self))
            || canonical_source.is_some_and(|batch| !batch.validate_owner_view(self))
            || evidence_source.is_some_and(|batch| !batch.validate_owner_view(self))
        {
            return Err(LedgerCause::OwnerMismatch);
        }
        if (publish_canonical
            && !can_absorb_sources_with_additional_commitments(
                baseline.canonical_assembly,
                [canonical_source],
                usize::from(canonical_extra.is_some()),
            ))
            || !can_absorb_sources_with_additional_commitments(
                baseline.evidence_assembly,
                [baseline.numerical_evidence, evidence_source],
                usize::from(evidence_extra.is_some()),
            )
        {
            return Err(LedgerCause::ActiveReservationsRemain { count: u64::MAX });
        }

        let canonical_attribution_count = baseline
            .canonical_assembly
            .combined_attribution_count([publish_canonical.then_some(canonical_source).flatten()])
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let evidence_attribution_count = baseline
            .evidence_assembly
            .combined_attribution_count([baseline.numerical_evidence, evidence_source])
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let projected_active = canonical_attribution_count
            .checked_add(evidence_attribution_count)
            .and_then(|count| {
                count.checked_add(u64::from(
                    baseline.canonical_assembly.capacity_cover.is_some(),
                ))
            })
            .and_then(|count| {
                count.checked_add(u64::from(
                    baseline.evidence_assembly.capacity_cover.is_some(),
                ))
            })
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let canonical_attribution_bytes = baseline
            .canonical_assembly
            .combined_attribution_bytes([publish_canonical.then_some(canonical_source).flatten()])
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        let evidence_attribution_bytes = baseline
            .evidence_assembly
            .combined_attribution_bytes([baseline.numerical_evidence, evidence_source])
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        let projected_live = canonical_attribution_bytes
            .checked_add(evidence_attribution_bytes)
            .and_then(|bytes| {
                bytes.checked_add(baseline.canonical_assembly.account_capacity_bytes())
            })
            .and_then(|bytes| {
                bytes.checked_add(baseline.evidence_assembly.account_capacity_bytes())
            })
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        let released_canonical_active = if publish_canonical {
            canonical_source.map_or(Some(0), |batch| {
                Some(u64::from(batch.capacity_cover.is_some()))
            })
        } else {
            canonical_source.map_or(Some(0), AttributionReservationBatch::active_count)
        }
        .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let released_active = released_canonical_active
            .checked_add(u64::from(
                baseline
                    .numerical_evidence
                    .is_some_and(|batch| batch.capacity_cover.is_some()),
            ))
            .and_then(|count| {
                count.checked_add(u64::from(
                    evidence_source.is_some_and(|batch| batch.capacity_cover.is_some()),
                ))
            })
            .and_then(|count| count.checked_add(baseline.fallback.active_count()?))
            .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?;
        let released_canonical_live = if publish_canonical {
            Some(canonical_source.map_or(0, AttributionReservationBatch::account_capacity_bytes))
        } else {
            canonical_source.map_or(Some(0), AttributionReservationBatch::live_bytes)
        }
        .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;
        let released_live = released_canonical_live
            .checked_add(
                baseline
                    .numerical_evidence
                    .map_or(0, AttributionReservationBatch::account_capacity_bytes),
            )
            .and_then(|bytes| {
                bytes.checked_add(
                    evidence_source.map_or(0, AttributionReservationBatch::account_capacity_bytes),
                )
            })
            .and_then(|bytes| bytes.checked_add(baseline.fallback.live_bytes()?))
            .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?;

        let sealed_destination_pair = canonical_extra.is_some()
            && evidence_extra.is_some()
            && state.publication == PublicationState::Closing
            && self.work.header.mode == WorkMode::OrdinarySealed;
        if !sealed_destination_pair {
            Ledger::<O, P, D>::ensure_terminal_preflight(state.header, self.work.header)?;
        }
        self.validate_terminal_snapshot_shape(selected)?;
        let active_after_transition = state
            .active_reservations
            .checked_sub(released_active)
            .ok_or(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            })?;
        if active_after_transition
            != projected_active
                .checked_add(state.retained_output_reservations)
                .ok_or(LedgerCause::ActiveReservationsRemain { count: u64::MAX })?
        {
            return Err(LedgerCause::ActiveReservationsRemain {
                count: state.active_reservations,
            });
        }
        let live_after_transition = state.current_live_bytes.checked_sub(released_live).ok_or(
            LedgerCause::LiveReservationsRemain {
                bytes: state.current_live_bytes,
            },
        )?;
        if live_after_transition
            != projected_live
                .checked_add(state.retained_output_bytes)
                .ok_or(LedgerCause::LiveReservationsRemain { bytes: u64::MAX })?
        {
            return Err(LedgerCause::PublicationLiveMismatch {
                transaction_bytes: projected_live,
                ledger_bytes: live_after_transition,
            });
        }
        validate_typed_commitment_batches_with_extra(
            state.dimensions,
            [
                Some(baseline.canonical_assembly),
                publish_canonical.then_some(canonical_source).flatten(),
            ],
            publish_canonical.then_some(canonical_extra).flatten(),
        )?;
        validate_typed_commitment_batches_with_extra(
            state.dimensions,
            [
                Some(baseline.evidence_assembly),
                baseline.numerical_evidence,
                evidence_source,
            ],
            evidence_extra,
        )
    }

    pub(crate) fn try_qualify_and_push_exact_primary_commitment_pair_rows(
        &self,
        baseline: FallbackBatchReadView<'_, O, P, D>,
        canonical_batch: &mut AttributionReservationBatch<O, P, D, CanonicalOutput>,
        mut canonical: OutputCommitment<D, CanonicalOutput>,
        evidence_batch: &mut AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>,
        mut evidence: OutputCommitment<D, OutcomeEvidenceDestination>,
    ) -> Result<(), CommitmentPairPushError<D>> {
        let canonical_position = self
            .work
            .catalog
            .binary_search(&canonical.dimension)
            .map_err(|_| LedgerCause::UnknownDimension {
                dimension: canonical.dimension,
            });
        let evidence_position = self
            .work
            .catalog
            .binary_search(&evidence.dimension)
            .map_err(|_| LedgerCause::UnknownDimension {
                dimension: evidence.dimension,
            });
        self.work
            .check_integrity()
            .map_err(CommitmentPairPushError::Ledger)?;
        let state = self.storage.read();
        if !canonical_batch.validate_owner_view(self) || !evidence_batch.validate_owner_view(self) {
            return Err(CommitmentPairPushError::Ledger(LedgerCause::OwnerMismatch));
        }
        if !canonical_batch.has_commitment_slot() {
            return Err(CommitmentPairPushError::CanonicalBatchFull);
        }
        if !evidence_batch.has_commitment_slot() {
            return Err(CommitmentPairPushError::OutcomeEvidenceBatchFull);
        }

        canonical.dimension_position =
            canonical_position.map_err(CommitmentPairPushError::Ledger)?;
        evidence.dimension_position = evidence_position.map_err(CommitmentPairPushError::Ledger)?;
        self.preflight_primary_publication_locked(
            &state,
            &baseline,
            Some(canonical_batch),
            Some(evidence_batch),
            true,
            Some(&canonical),
            Some(&evidence),
        )
        .map_err(CommitmentPairPushError::Ledger)?;

        canonical_batch.push_positioned_commitment(canonical);
        evidence_batch.push_positioned_commitment(evidence);
        Ok(())
    }

    fn validate_terminal_snapshot_shape(
        &self,
        selected: &SelectedStorageRead<'_, P, D>,
    ) -> Result<(), LedgerCause<D>> {
        let (dimension_staging_len, phase_staging_len) = selected.snapshot_staging_lengths();
        SnapshotRows {
            storage: selected.view(),
            phases: selected.phases(),
            work_rows: self.work.rows,
            work_header: self.work.header,
        }
        .validate_final_shape(dimension_staging_len, phase_staging_len)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod finite_baseline_tests;
