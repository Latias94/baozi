use crate::config::FormatSelection;
use crate::dependency::{
    DenyDependencies, DependencyCache, DependencyResolver, SameAuthorityResolver,
};
use crate::detection::DetectionScope;
use crate::diagnostic::DiagnosticSink;
use crate::error::{
    DetectionFailure, ImportCause, ScopeAbort, ScopeAbortCause, TrustedExtensionBoundary,
};
use crate::format::{DetectionOutcome, Probe, ReadConfidence, discard_untrusted_panic_payload};
use crate::ledger::{
    ImportCoordinator, ImportDimension, ImportLedgerPhase, ImportOperationControl,
    ImportProgressPhase, ImportProgressUnit, ImportReservation, import_ledger_limits,
    tighten_selected_format_limits,
};
use crate::limits::ResourceLimits;
use crate::loss::ImportLossEvidence;
use crate::outcome::{
    DetectionEvidence, ImportFailure, ImportFailureParts, ImportPhase, ImportPhaseEvidence,
    ImportReport, ImportReportParts, ImportResult, ImportRunError,
};
use crate::registry::{ParseInvocationError, RegistryEntry};
use crate::request::PreparedImportRequest;
use crate::scope::{
    DiagnosticLineIndex, ParseFailureKind, ParseScope, ParseScopeParts,
    prepare_snapshot_reservation_list, read_opened_asset,
};
use baozi_core::{
    FormatId, SceneDraft, SceneReservationKind, SceneStorageFootprint, SceneValidationControl,
    SceneValidationFailure, SceneValidationLimits, SceneValidationSuccess, SceneValidationWork,
    validate_scene_with,
};
use baozi_io::{
    AssetIo, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetPath, AuthorityDomain,
    OpenedAsset,
};
use baozi_operation::{
    CapacityPlan, CapacityPlanError, Commitment, FinalProgress, LedgerCause, LedgerLimits,
    LedgerLimitsError, PublicationCause,
};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadHint {
    source: Option<AssetPath>,
}

impl ReadHint {
    pub fn from_source(source: AssetPath) -> Self {
        Self {
            source: Some(source),
        }
    }

    pub fn source(&self) -> Option<&AssetPath> {
        self.source.as_ref()
    }

    pub fn extension(&self) -> Option<&str> {
        self.source.as_ref().and_then(AssetPath::extension)
    }
}

/// Imports through one session using the primary authority for dependencies.
#[allow(
    clippy::result_large_err,
    reason = "terminal import failures retain complete bounded session evidence without heap indirection"
)]
pub fn import_asset(
    request: &PreparedImportRequest,
    io: &dyn AssetIo,
    source: AssetPath,
    control: ImportOperationControl,
) -> ImportResult<ImportReport> {
    import_asset_with_resolver(request, io, source, &SameAuthorityResolver, control)
}

/// Imports through one session and an explicit dependency authority policy.
#[allow(
    clippy::result_large_err,
    reason = "terminal import failures retain complete bounded session evidence without heap indirection"
)]
pub fn import_asset_with_resolver(
    request: &PreparedImportRequest,
    io: &dyn AssetIo,
    source: AssetPath,
    resolver: &dyn DependencyResolver,
    control: ImportOperationControl,
) -> ImportResult<ImportReport> {
    import_asset_with_resolver_and_ledger_limits(
        request,
        io,
        source,
        resolver,
        control,
        import_ledger_limits(request.limits()),
    )
}

#[allow(
    clippy::result_large_err,
    reason = "terminal import failures retain complete bounded session evidence without heap indirection"
)]
fn import_asset_with_resolver_and_ledger_limits(
    request: &PreparedImportRequest,
    io: &dyn AssetIo,
    source: AssetPath,
    resolver: &dyn DependencyResolver,
    control: ImportOperationControl,
    ledger_limits: Result<LedgerLimits<ImportDimension>, LedgerLimitsError<ImportDimension>>,
) -> ImportResult<ImportReport> {
    let ledger_limits = ledger_limits.map_err(ImportRunError::initialization)?;
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.import").entered();

    let mut session = ImportSession::new(request, io, source, resolver, control, ledger_limits);
    let execution = session.execute(SessionCreatedProof::new());
    match execution {
        Ok(validation) => session
            .finalize_success(validation)
            .map_err(ImportRunError::operation),
        Err(cause) => Err(ImportRunError::operation(session.finalize_failure(cause))),
    }
}

/// Imports an immutable caller-owned byte slice through one directly admitted primary snapshot.
///
/// Direct byte imports have no secondary asset authority. Dependency access is denied, and callers
/// must allow [`ImportLossKind::OptionalDependencyUnavailable`](crate::ImportLossKind) in the
/// uncompiled request to authorize lossy omission of unavailable optional sidecars.
#[allow(
    clippy::result_large_err,
    reason = "terminal import failures retain complete bounded session evidence without heap indirection"
)]
pub fn import_bytes(
    request: &PreparedImportRequest,
    source: AssetPath,
    bytes: &[u8],
    control: ImportOperationControl,
) -> ImportResult<ImportReport> {
    let ledger_limits =
        import_ledger_limits(request.limits()).map_err(ImportRunError::initialization)?;
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.import").entered();

    let primary_io = DirectBytesAuthority::new();
    let mut session = ImportSession::new(
        request,
        &primary_io,
        source,
        &DenyDependencies,
        control,
        ledger_limits,
    );
    let execution = session.execute_bytes(bytes, SessionCreatedProof::new());
    match execution {
        Ok(validation) => session
            .finalize_success(validation)
            .map_err(ImportRunError::operation),
        Err(cause) => Err(ImportRunError::operation(session.finalize_failure(cause))),
    }
}

struct DirectBytesAuthority {
    domain: AuthorityDomain,
}

impl DirectBytesAuthority {
    fn new() -> Self {
        Self {
            domain: AuthorityDomain::new(),
        }
    }
}

impl AssetIo for DirectBytesAuthority {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open(&self, _path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        Err(AssetOpenError::new(
            AssetOpenStage::Open,
            AssetOpenErrorKind::CapabilityUnavailable,
        ))
    }
}

struct ImportSession<'a> {
    request: &'a PreparedImportRequest,
    primary_io: &'a dyn AssetIo,
    source: AssetPath,
    resolver: &'a dyn DependencyResolver,
    coordinator: Option<RefCell<ImportCoordinator>>,
    diagnostics: Option<RefCell<DiagnosticSink>>,
    diagnostic_reservations: RefCell<Vec<ImportReservation>>,
    diagnostic_line_indexes: RefCell<Vec<DiagnosticLineIndex>>,
    dependencies: Option<RefCell<DependencyCache>>,
    optional_dependency_evidence: RefCell<Vec<crate::OptionalDependencyEvidence>>,
    loss_evidence: RefCell<Vec<ImportLossEvidence>>,
    terminal: RefCell<Option<ScopeAbortCause>>,
    primary_authority: Option<AuthorityDomain>,
    primary_backend_hard_limit: Option<u64>,
    primary_bytes: Option<Vec<u8>>,
    primary_reservations: Vec<ImportReservation>,
    draft_reservations: RefCell<Vec<ImportReservation>>,
    draft_reservation_list_capacity: RefCell<Option<ImportReservation>>,
    format_failure_storage: RefCell<Option<ImportReservation>>,
    selected: Option<RegistryEntry>,
    effective_limits: Option<ResourceLimits>,
    detection: Vec<DetectionEvidence>,
    detection_reservation: Option<ImportReservation>,
    detection_cause_reservation: Option<ImportReservation>,
    last_completed: ImportPhaseEvidence,
}

impl<'a> ImportSession<'a> {
    fn new(
        request: &'a PreparedImportRequest,
        primary_io: &'a dyn AssetIo,
        source: AssetPath,
        resolver: &'a dyn DependencyResolver,
        control: ImportOperationControl,
        ledger_limits: LedgerLimits<ImportDimension>,
    ) -> Self {
        Self {
            request,
            primary_io,
            source,
            resolver,
            coordinator: Some(RefCell::new(ImportCoordinator::new(control, ledger_limits))),
            diagnostics: Some(RefCell::new(DiagnosticSink::new(
                request.limits().max_diagnostics,
                request.limits().max_diagnostic_bytes,
                request.limits().max_diagnostic_message_bytes,
                request.limits().max_attempted_diagnostic_bytes,
            ))),
            diagnostic_reservations: RefCell::new(Vec::new()),
            diagnostic_line_indexes: RefCell::new(Vec::new()),
            dependencies: Some(RefCell::new(DependencyCache::new())),
            optional_dependency_evidence: RefCell::new(Vec::new()),
            loss_evidence: RefCell::new(Vec::new()),
            terminal: RefCell::new(None),
            primary_authority: None,
            primary_backend_hard_limit: None,
            primary_bytes: None,
            primary_reservations: Vec::new(),
            draft_reservations: RefCell::new(Vec::new()),
            draft_reservation_list_capacity: RefCell::new(None),
            format_failure_storage: RefCell::new(None),
            selected: None,
            effective_limits: None,
            detection: Vec::new(),
            detection_reservation: None,
            detection_cause_reservation: None,
            last_completed: ImportPhaseEvidence::new(ImportPhase::SessionCreated),
        }
    }

    fn execute(
        &mut self,
        created: SessionCreatedProof,
    ) -> Result<SceneValidationSuccess<ImportValidationReservation>, ImportCause> {
        let primary = self.bind_primary(created)?;
        let selected = self.select_format(primary)?;
        let (draft, draft_proof) = self.parse_selected(selected)?;
        self.validate_draft(draft, draft_proof)
    }

    fn execute_bytes(
        &mut self,
        bytes: &[u8],
        created: SessionCreatedProof,
    ) -> Result<SceneValidationSuccess<ImportValidationReservation>, ImportCause> {
        let primary = self.bind_primary_bytes(bytes, created)?;
        let selected = self.select_format(primary)?;
        let (draft, draft_proof) = self.parse_selected(selected)?;
        self.validate_draft(draft, draft_proof)
    }

    fn bind_primary(
        &mut self,
        _proof: SessionCreatedProof,
    ) -> Result<PrimaryBoundProof, ImportCause> {
        self.poll_boundary()?;
        self.charge_open_attempt()?;
        let expected_domain = catch_unwind(AssertUnwindSafe(|| self.primary_io.authority_domain()))
            .map_err(|payload| {
                discard_untrusted_panic_payload(payload);
                ImportCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code: "primary-authority-panicked",
                }
            })?;
        let opened = catch_unwind(AssertUnwindSafe(|| self.primary_io.open(&self.source)))
            .map_err(|payload| {
                discard_untrusted_panic_payload(payload);
                ImportCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code: "primary-open-panicked",
                }
            })?
            .map_err(|cause| self.scope_cause(ScopeAbortCause::AssetOpen(cause)))?;
        if opened.authority_domain() != &expected_domain || opened.path() != &self.source {
            return Err(ImportCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code: "primary-open-identity-mismatch",
            });
        }
        let backend_hard_limit = opened.backend_hard_limit();
        self.charge_open_consumed()?;
        let (bytes, reservations) = read_opened_asset(
            self.coordinator(),
            opened,
            ImportLedgerPhase::PrimarySnapshot,
            ImportProgressPhase::AcquirePrimary,
            ImportDimension::PrimaryBytes,
            self.request.limits().max_primary_asset_bytes,
            self.request.limits().max_total_asset_bytes,
        )
        .map_err(|cause| self.scope_cause(cause))?;
        self.primary_authority = Some(expected_domain);
        self.primary_backend_hard_limit = backend_hard_limit;
        self.primary_bytes = Some(bytes);
        self.primary_reservations = reservations;
        self.last_completed = ImportPhaseEvidence::new(ImportPhase::PrimaryBound);
        Ok(PrimaryBoundProof::new())
    }

    fn bind_primary_bytes(
        &mut self,
        bytes: &[u8],
        _proof: SessionCreatedProof,
    ) -> Result<PrimaryBoundProof, ImportCause> {
        self.poll_boundary()?;
        let primary_authority =
            catch_unwind(AssertUnwindSafe(|| self.primary_io.authority_domain())).map_err(
                |payload| {
                    discard_untrusted_panic_payload(payload);
                    ImportCause::TrustedExtension {
                        boundary: TrustedExtensionBoundary::AssetIo,
                        code: "primary-authority-panicked",
                    }
                },
            )?;
        let byte_count = u64::try_from(bytes.len()).map_err(|_| {
            self.scope_cause(ScopeAbortCause::Allocation {
                resource: "direct-primary-snapshot",
            })
        })?;
        self.charge_primary_bytes(byte_count, false)?;
        if byte_count > self.request.limits().max_primary_asset_bytes {
            return Err(self.scope_cause(ScopeAbortCause::ResourceLimit {
                resource: "asset-bytes",
                limit: self.request.limits().max_primary_asset_bytes,
            }));
        }

        let mut primary_reservations = if byte_count == 0 {
            Vec::new()
        } else {
            prepare_snapshot_reservation_list(
                self.coordinator(),
                ImportLedgerPhase::PrimarySnapshot,
            )
            .map_err(|cause| self.scope_cause(cause))?
        };
        let mut snapshot = Vec::new();
        let reservation = reserve_vec_capacity(
            self.coordinator(),
            &mut snapshot,
            bytes.len(),
            ImportLedgerPhase::PrimarySnapshot,
            ImportDimension::PrimaryBytes,
            "direct-primary-snapshot",
        )
        .map_err(|cause| self.scope_cause(cause))?;
        snapshot.extend_from_slice(bytes);
        self.charge_primary_bytes(byte_count, true)?;
        if let Some(reservation) = reservation {
            primary_reservations.push(reservation);
        }
        self.primary_reservations = primary_reservations;
        self.primary_bytes = Some(snapshot);
        self.report_boundary_progress(
            ImportProgressPhase::AcquirePrimary,
            ImportProgressUnit::Bytes,
            byte_count,
            Some(byte_count),
        )?;
        self.primary_authority = Some(primary_authority);
        self.last_completed = ImportPhaseEvidence::new(ImportPhase::PrimaryBound);
        Ok(PrimaryBoundProof::new())
    }

    fn select_format(
        &mut self,
        _proof: PrimaryBoundProof,
    ) -> Result<SelectionCompletedProof, ImportCause> {
        #[cfg(feature = "tracing")]
        let _span = tracing::info_span!("baozi.detect").entered();

        self.poll_boundary()?;
        let bytes = self.primary_bytes.as_ref().ok_or_else(|| {
            self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "primary-proof-without-snapshot",
            })
        })?;
        let probe_len =
            usize::try_from(self.request.config().detection_options().max_probe_bytes())
                .unwrap_or(usize::MAX)
                .min(bytes.len());
        let probe = Probe::new(&bytes[..probe_len], bytes.len() as u64);
        let _hint_reservation = reserve_exact_bytes(
            self.coordinator(),
            ImportLedgerPhase::Detection,
            ImportDimension::ScratchBytes,
            self.source.as_str().len() as u64,
        )
        .map_err(|cause| self.scope_cause(cause))?;
        let hint = ReadHint::from_source(self.source.try_clone().map_err(|_| {
            self.scope_cause(ScopeAbortCause::Allocation {
                resource: "detection-source-hint",
            })
        })?);
        let entry_count = match self.request.config().format_selection() {
            FormatSelection::Forced(id) => usize::from(self.request.entry(id).is_some()),
            FormatSelection::Automatic | FormatSelection::Preferred(_) => {
                self.request.snapshot().entries().count()
            }
        };
        let mut entries = Vec::new();
        let _entries_reservation = reserve_vec_capacity(
            self.coordinator(),
            &mut entries,
            entry_count,
            ImportLedgerPhase::Detection,
            ImportDimension::ScratchBytes,
            "detection-candidates",
        )
        .map_err(|cause| self.scope_cause(cause))?;
        match self.request.config().format_selection() {
            FormatSelection::Forced(id) => {
                if let Some(entry) = self.request.entry(id) {
                    entries.push(entry);
                }
            }
            FormatSelection::Automatic | FormatSelection::Preferred(_) => {
                entries.extend(self.request.snapshot().entries())
            }
        }
        let coordinator = self.coordinator.as_ref().ok_or_else(|| {
            self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "detection-without-active-coordinator",
            })
        })?;
        let detection_reservation = reserve_vec_capacity(
            coordinator,
            &mut self.detection,
            entries.len(),
            ImportLedgerPhase::OutcomeFinalization,
            ImportDimension::ScratchBytes,
            "detection-evidence",
        )
        .map_err(|cause| self.scope_cause(cause))?;
        if self.detection_reservation.is_some() {
            return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "detection-storage-reserved-more-than-once",
            }));
        }
        self.detection_reservation = detection_reservation;
        let total = entries.len() as u64;
        for (index, entry) in entries.iter().enumerate() {
            self.poll_boundary()?;
            let (outcome, latched) = {
                let scope = DetectionScope::new(self.coordinator(), &self.terminal);
                let outcome = entry.detect(&probe, &hint, &scope);
                let latched = scope.into_terminal_abort();
                (outcome, latched)
            };
            if let Some(abort) = latched {
                return Err(ImportCause::Scope(abort));
            }
            self.poll_boundary()?;
            let outcome = outcome.map_err(|_| {
                self.scope_cause(ScopeAbortCause::DescriptorContract {
                    code: "detection-abort-without-terminal-cause",
                })
            })?;
            self.detection
                .push(DetectionEvidence::new(entry.format_id().clone(), outcome));
            self.report_boundary_progress(
                ImportProgressPhase::Detect,
                ImportProgressUnit::Candidates,
                (index + 1) as u64,
                Some(total),
            )?;
        }

        let selected = self.choose_candidate(&entries)?;
        self.selected = Some(selected);
        self.last_completed = ImportPhaseEvidence::new(ImportPhase::SelectionCompleted);
        Ok(SelectionCompletedProof::new())
    }

    fn choose_candidate(
        &mut self,
        entries: &[RegistryEntry],
    ) -> Result<RegistryEntry, ImportCause> {
        if matches!(
            self.request.config().format_selection(),
            FormatSelection::Forced(_)
        ) {
            let entry = entries
                .first()
                .cloned()
                .ok_or(ImportCause::UnsupportedFormat)?;
            return match self.detection.first().map(DetectionEvidence::outcome) {
                Some(DetectionOutcome::Match(_)) | Some(DetectionOutcome::Inconclusive(_)) => {
                    Ok(entry)
                }
                Some(DetectionOutcome::NoMatch) => Err(ImportCause::Detection(
                    DetectionFailure::ForcedFormatContradiction {
                        format: entry.format_id().clone(),
                    },
                )),
                Some(DetectionOutcome::Fault(_)) => Err(ImportCause::Detection(
                    DetectionFailure::ForcedDetectorFault {
                        format: entry.format_id().clone(),
                    },
                )),
                None => Err(ImportCause::UnsupportedFormat),
            };
        }

        let mut candidates = Vec::new();
        let _candidates_reservation = reserve_vec_capacity(
            self.coordinator(),
            &mut candidates,
            entries.len(),
            ImportLedgerPhase::Detection,
            ImportDimension::ScratchBytes,
            "detection-candidate-ranking",
        )
        .map_err(|cause| self.scope_cause(cause))?;
        candidates.extend(
            entries
                .iter()
                .zip(&self.detection)
                .filter_map(|(entry, evidence)| match evidence.outcome() {
                    DetectionOutcome::Match(confidence) => Some((entry.clone(), *confidence)),
                    _ => None,
                }),
        );
        let strong = candidates
            .iter()
            .map(|(_, confidence)| *confidence)
            .filter(|confidence| *confidence >= ReadConfidence::Likely)
            .max();
        if let Some(strongest) = strong {
            candidates.retain(|(_, confidence)| *confidence == strongest);
            return self.break_tie(candidates, true);
        }

        candidates.retain(|(_, confidence)| *confidence == ReadConfidence::Maybe);
        self.break_tie(candidates, false)
    }

    fn break_tie(
        &mut self,
        candidates: Vec<(RegistryEntry, ReadConfidence)>,
        unique_unhinted_allowed: bool,
    ) -> Result<RegistryEntry, ImportCause> {
        if candidates.is_empty() {
            return Err(self.no_selection_cause());
        }
        if candidates.len() == 1 && unique_unhinted_allowed {
            return Ok(candidates[0].0.clone());
        }
        if let FormatSelection::Preferred(preferred) = self.request.config().format_selection()
            && let Some((entry, _)) = candidates
                .iter()
                .find(|(entry, _)| entry.format_id() == preferred)
        {
            return Ok(entry.clone());
        }
        let mut extension_matches = Vec::new();
        let _extension_matches_reservation = reserve_vec_capacity(
            self.coordinator(),
            &mut extension_matches,
            candidates.len(),
            ImportLedgerPhase::Detection,
            ImportDimension::ScratchBytes,
            "extension-detection-candidates",
        )
        .map_err(|cause| self.scope_cause(cause))?;
        if let Some(extension) = self.source.extension() {
            extension_matches.extend(candidates.iter().filter(|(entry, _)| {
                entry
                    .descriptor()
                    .extensions()
                    .any(|candidate| candidate.eq_ignore_ascii_case(extension))
            }));
        }
        match extension_matches.as_slice() {
            [(entry, _)] => Ok((*entry).clone()),
            [] if candidates.len() == 1 && unique_unhinted_allowed => Ok(candidates[0].0.clone()),
            [] if unique_unhinted_allowed => self.ambiguous(candidates),
            [] => Err(self.no_selection_cause()),
            _ => self.ambiguous(candidates),
        }
    }

    fn no_selection_cause(&self) -> ImportCause {
        let had_match = self
            .detection
            .iter()
            .any(|evidence| matches!(evidence.outcome(), DetectionOutcome::Match(_)));
        let had_inconclusive = self
            .detection
            .iter()
            .any(|evidence| matches!(evidence.outcome(), DetectionOutcome::Inconclusive(_)));
        let had_fault = self
            .detection
            .iter()
            .any(|evidence| matches!(evidence.outcome(), DetectionOutcome::Fault(_)));
        if !had_match && !had_inconclusive && !had_fault {
            ImportCause::UnsupportedFormat
        } else {
            ImportCause::Detection(DetectionFailure::NoSelection {
                had_match,
                had_inconclusive,
                had_fault,
            })
        }
    }

    fn ambiguous(
        &mut self,
        candidates: Vec<(RegistryEntry, ReadConfidence)>,
    ) -> Result<RegistryEntry, ImportCause> {
        let mut format_ids = Vec::new();
        let cause_reservation = reserve_vec_capacity(
            self.coordinator(),
            &mut format_ids,
            candidates.len(),
            ImportLedgerPhase::OutcomeFinalization,
            ImportDimension::ScratchBytes,
            "ambiguous-format-candidates",
        )
        .map_err(|cause| self.scope_cause(cause))?;
        format_ids.extend(
            candidates
                .into_iter()
                .map(|(entry, _)| entry.format_id().clone()),
        );
        format_ids.sort();
        format_ids.dedup();
        if self.detection_cause_reservation.is_some() {
            return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "detection-cause-storage-reserved-more-than-once",
            }));
        }
        self.detection_cause_reservation = cause_reservation;
        Err(ImportCause::Detection(DetectionFailure::Ambiguous {
            candidates: format_ids,
        }))
    }

    fn freeze_selected_limits(&mut self, entry: &RegistryEntry) -> Result<(), ImportCause> {
        if self.effective_limits.is_some() {
            return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "selected-limits-frozen-more-than-once",
            }));
        }
        self.effective_limits = Some(self.request.limits().effective_for_selected_format(
            entry.descriptor().text_profile(),
            self.primary_backend_hard_limit,
        ));
        let tightening = {
            let effective_limits = self.effective_limits.as_ref().ok_or_else(|| {
                self.scope_cause(ScopeAbortCause::DescriptorContract {
                    code: "selected-limits-not-frozen",
                })
            })?;
            let mut coordinator = self.coordinator().borrow_mut();
            tighten_selected_format_limits(&mut coordinator, effective_limits)
        };
        tightening.map_err(|cause| self.scope_cause(ScopeAbortCause::Ledger(cause)))
    }

    fn parse_selected(
        &mut self,
        _proof: SelectionCompletedProof,
    ) -> Result<(SceneDraft, DraftBuiltProof), ImportCause> {
        #[cfg(feature = "tracing")]
        let _span = tracing::info_span!("baozi.parse").entered();

        self.poll_boundary()?;
        let entry = self.selected.as_ref().cloned().ok_or_else(|| {
            self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "selection-proof-without-entry",
            })
        })?;
        self.freeze_selected_limits(&entry)?;
        let options = self.request.options_for(&entry).ok_or_else(|| {
            self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "foreign-compiled-options",
            })
        })?;
        let effective_limits = self.effective_limits.as_ref().ok_or_else(|| {
            self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "selection-proof-without-effective-limits",
            })
        })?;
        let (parsed, latched) = {
            let mut scope = ParseScope::new(ParseScopeParts {
                entry: entry.clone(),
                options,
                primary_io: self.primary_io,
                primary_authority: self.primary_authority.as_ref().ok_or_else(|| {
                    self.scope_cause(ScopeAbortCause::DescriptorContract {
                        code: "selection-proof-without-primary-authority",
                    })
                })?,
                primary_source: &self.source,
                primary_bytes: self.primary_bytes.as_ref().ok_or_else(|| {
                    self.scope_cause(ScopeAbortCause::DescriptorContract {
                        code: "selection-proof-without-primary-snapshot",
                    })
                })?,
                resolver: self.resolver,
                conformance_policy: self.request.config().conformance_policy(),
                loss_policy: self.request.config().loss_policy(),
                limits: effective_limits,
                coordinator: self.coordinator(),
                diagnostics: self.diagnostics(),
                diagnostic_reservations: &self.diagnostic_reservations,
                diagnostic_line_indexes: &self.diagnostic_line_indexes,
                dependencies: self.dependencies(),
                optional_dependency_evidence: &self.optional_dependency_evidence,
                loss_evidence: &self.loss_evidence,
                draft_reservations: &self.draft_reservations,
                draft_reservation_list_capacity: &self.draft_reservation_list_capacity,
                format_failure_storage: &self.format_failure_storage,
                terminal: &self.terminal,
            });
            let parsed = entry.parse(&mut scope);
            let latched = scope.take_terminal_abort();
            (parsed, latched)
        };
        let post_parse_poll = self.poll_boundary();
        if let Some(abort) = latched {
            return Err(ImportCause::Scope(abort));
        }
        post_parse_poll?;
        let draft = match parsed {
            Err(ParseInvocationError::ExtensionPanicked) => {
                return Err(ImportCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::Importer,
                    code: "importer-panicked",
                });
            }
            Err(ParseInvocationError::ExtensionPoisoned) => {
                return Err(ImportCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::Importer,
                    code: "importer-poisoned",
                });
            }
            Ok(Err(failure)) => match failure.into_kind() {
                ParseFailureKind::Scope => {
                    return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                        code: "scope-failure-without-terminal-cause",
                    }));
                }
                ParseFailureKind::Format(envelope) => {
                    if self.format_failure_storage.borrow().is_none() {
                        return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                            code: "format-failure-without-storage-reservation",
                        }));
                    }
                    return Err(ImportCause::Format(envelope));
                }
            },
            Ok(Ok(draft)) => {
                if self.format_failure_storage.borrow().is_some() {
                    return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                        code: "format-failure-was-swallowed",
                    }));
                }
                draft
            }
        };
        let reserved_bytes =
            self.draft_reservations
                .borrow()
                .iter()
                .try_fold(0_u64, |total, reservation| {
                    total.checked_add(reservation.bytes()).ok_or_else(|| {
                        self.scope_cause(ScopeAbortCause::DescriptorContract {
                            code: "draft-reservation-bytes-overflow",
                        })
                    })
                })?;
        let required_bytes = draft
            .storage_footprint()
            .map_err(|cause| self.scope_cause(ScopeAbortCause::SceneAccounting(cause)))?
            .accounted_payload_bytes();
        if reserved_bytes < required_bytes {
            return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "draft-returned-without-complete-reservation",
            }));
        }
        self.last_completed = ImportPhaseEvidence::new(ImportPhase::DraftBuilt);
        Ok((draft, DraftBuiltProof::new()))
    }

    fn validate_draft(
        &mut self,
        draft: SceneDraft,
        _proof: DraftBuiltProof,
    ) -> Result<SceneValidationSuccess<ImportValidationReservation>, ImportCause> {
        self.poll_boundary()?;
        self.report_boundary_progress(
            ImportProgressPhase::Validate,
            ImportProgressUnit::CanonicalItems,
            0,
            Some(1),
        )?;
        let validation_limits =
            scene_validation_limits(self.effective_limits.as_ref().ok_or_else(|| {
                self.scope_cause(ScopeAbortCause::DescriptorContract {
                    code: "draft-proof-without-effective-limits",
                })
            })?);
        let validation = {
            let mut control = ImportValidationAdapter {
                coordinator: self.coordinator(),
                draft_reservations: &self.draft_reservations,
                draft_reservation_list_capacity: &self.draft_reservation_list_capacity,
            };
            validate_scene_with(draft, validation_limits, &mut control)
        };
        let validation = match validation {
            Ok(validation) => validation,
            Err(SceneValidationFailure::Control(cause)) => {
                return Err(self.scope_cause(cause));
            }
            Err(SceneValidationFailure::Scene(cause)) => {
                return Err(ImportCause::Validation(cause));
            }
        };
        self.poll_boundary()?;
        self.report_boundary_progress(
            ImportProgressPhase::Validate,
            ImportProgressUnit::CanonicalItems,
            1,
            Some(1),
        )?;
        self.last_completed = ImportPhaseEvidence::new(ImportPhase::Validated);
        Ok(validation)
    }

    #[allow(
        clippy::result_large_err,
        reason = "terminal import failures retain complete bounded session evidence without heap indirection"
    )]
    fn finalize_success(
        mut self,
        validation: SceneValidationSuccess<ImportValidationReservation>,
    ) -> Result<ImportReport, ImportFailure> {
        if self.format_failure_storage.get_mut().is_some() {
            let cause = self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "format-failure-was-swallowed",
            });
            drop(validation);
            return Err(self.finalize_failure(cause));
        }
        let output_bytes = validation.stats().footprint().accounted_payload_bytes();
        let Some(format) = self.selected.as_ref().map(RegistryEntry::descriptor_handle) else {
            drop(validation);
            return Err(self.finalize_failure(ImportCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::Importer,
                code: "validated-proof-without-selected-format",
            }));
        };
        let mut staging = self.take_report_staging();
        self.release_transient_live();
        if !self.draft_reservations.get_mut().is_empty()
            || self.draft_reservation_list_capacity.get_mut().is_some()
        {
            let cause = self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "validated-output-left-draft-reservations",
            });
            return Err(self.finalize_failure_with_staging(cause, staging));
        }
        let scene = match validation.try_publish(|output| {
            let mut reservations = output.into_reservations()?;
            self.append_staging_reservations(&mut reservations, &mut staging)?;
            let mut coordinator = self.coordinator().borrow_mut();
            let publication = coordinator
                .prepare_publication(
                    reservations,
                    &[Commitment::new(
                        ImportDimension::CanonicalOutputBytes,
                        output_bytes,
                    )],
                )
                .map_err(ScopeAbortCause::Ledger)?;
            coordinator
                .publish(
                    publication,
                    FinalProgress::new(
                        ImportProgressPhase::Finalize,
                        ImportProgressUnit::CanonicalItems,
                        1,
                    ),
                )
                .map_err(publication_scope_cause)
        }) {
            Ok(scene) => scene,
            Err(cause) => {
                let cause = self.scope_cause(cause);
                return Err(self.finalize_failure_with_staging(cause, staging));
            }
        };
        let (ledger, finalization_cause) = self.finish_ledger();
        let ReportStaging {
            detection,
            diagnostics,
            diagnostic_summary,
            optional_dependency_evidence,
            loss_evidence,
        } = staging;
        if let Some(cause) = finalization_cause {
            let finalization_cause = Some(cause.clone());
            return Err(ImportFailure::from_parts(ImportFailureParts {
                cause: self.scope_cause(ScopeAbortCause::Ledger(cause)),
                source: self.source,
                selected_format: Some(format.id().clone()),
                detection,
                diagnostics,
                diagnostic_summary,
                optional_dependency_evidence,
                loss_evidence,
                ledger,
                finalization_cause,
                last_completed: self.last_completed,
            }));
        }
        Ok(ImportReport::from_parts(ImportReportParts {
            scene,
            source: self.source,
            format,
            detection,
            diagnostics,
            diagnostic_summary,
            optional_dependency_evidence,
            loss_evidence,
            ledger,
            last_completed: self.last_completed,
        }))
    }

    fn finalize_failure(mut self, cause: ImportCause) -> ImportFailure {
        let staging = self.take_report_staging();
        self.finalize_failure_with_staging(cause, staging)
    }

    fn finalize_failure_with_staging(
        mut self,
        cause: ImportCause,
        staging: ReportStaging,
    ) -> ImportFailure {
        let selected_format = self
            .selected
            .as_ref()
            .map(|entry| entry.format_id().clone());
        let mut failure = FailureStaging {
            cause,
            selected_format,
            report: staging,
            last_completed: self.last_completed,
        };
        self.release_transient_live();
        self.release_draft_live();
        self.release_staging_reservations(&mut failure.report);
        drop(self.format_failure_storage.get_mut().take());
        let (ledger, finalization_cause) = self.finish_ledger();
        failure.finish(self.source, ledger, finalization_cause)
    }

    fn release_transient_live(&mut self) {
        self.primary_bytes = None;
        drop(std::mem::take(&mut self.primary_reservations));
        drop(std::mem::take(self.diagnostic_line_indexes.get_mut()));
        drop(self.dependencies.take());
    }

    fn release_draft_live(&mut self) {
        let reservations = std::mem::take(self.draft_reservations.get_mut());
        let reservation_list_capacity = self.draft_reservation_list_capacity.get_mut().take();
        drop(reservations);
        drop(reservation_list_capacity);
    }

    fn take_report_staging(&mut self) -> ReportStaging {
        let (diagnostics, diagnostic_summary) = self
            .diagnostics
            .take()
            .map(|diagnostics| diagnostics.into_inner().into_parts())
            .unwrap_or_default();
        ReportStaging {
            detection: std::mem::take(&mut self.detection),
            diagnostics,
            diagnostic_summary,
            optional_dependency_evidence: std::mem::take(
                self.optional_dependency_evidence.get_mut(),
            ),
            loss_evidence: std::mem::take(self.loss_evidence.get_mut()),
        }
    }

    fn append_staging_reservations(
        &mut self,
        reservations: &mut Vec<ImportReservation>,
        staging: &mut ReportStaging,
    ) -> Result<(), ScopeAbortCause> {
        let optional_count = staging
            .optional_dependency_evidence
            .iter()
            .try_fold(0_usize, |total, evidence| {
                total.checked_add(evidence.reservation_count())
            })
            .ok_or(ScopeAbortCause::Allocation {
                resource: "publication-reservations",
            })?;
        let loss_count = staging
            .loss_evidence
            .iter()
            .try_fold(0_usize, |total, evidence| {
                total.checked_add(evidence.reservation_count())
            })
            .ok_or(ScopeAbortCause::Allocation {
                resource: "publication-reservations",
            })?;
        let retained_count = self
            .diagnostic_reservations
            .get_mut()
            .len()
            .checked_add(optional_count)
            .and_then(|count| count.checked_add(loss_count))
            .and_then(|count| count.checked_add(usize::from(self.detection_reservation.is_some())))
            .and_then(|count| {
                count.checked_add(usize::from(self.detection_cause_reservation.is_some()))
            })
            .ok_or(ScopeAbortCause::Allocation {
                resource: "publication-reservations",
            })?;
        let list_capacity_reservation = if retained_count == 0 {
            None
        } else {
            reserve_vec_capacity(
                self.coordinator(),
                reservations,
                retained_count
                    .checked_add(1)
                    .ok_or(ScopeAbortCause::Allocation {
                        resource: "publication-reservations",
                    })?,
                ImportLedgerPhase::OutcomeFinalization,
                ImportDimension::ScratchBytes,
                "publication-reservations",
            )?
        };
        if let Some(list_capacity_reservation) = list_capacity_reservation {
            reservations.push(list_capacity_reservation);
        }
        reservations.append(self.diagnostic_reservations.get_mut());
        if let Some(reservation) = self.detection_reservation.take() {
            reservations.push(reservation);
        }
        if let Some(reservation) = self.detection_cause_reservation.take() {
            reservations.push(reservation);
        }
        for evidence in &mut staging.optional_dependency_evidence {
            evidence.drain_reservations(reservations);
        }
        for evidence in &mut staging.loss_evidence {
            evidence.drain_reservation(reservations);
        }
        Ok(())
    }

    fn release_staging_reservations(&mut self, staging: &mut ReportStaging) {
        drop(std::mem::take(self.diagnostic_reservations.get_mut()));
        drop(self.detection_reservation.take());
        drop(self.detection_cause_reservation.take());
        for evidence in &mut staging.optional_dependency_evidence {
            evidence.release_reservations();
        }
        for evidence in &mut staging.loss_evidence {
            evidence.release_reservation();
        }
    }

    fn finish_ledger(
        &mut self,
    ) -> (
        crate::ImportLedgerSnapshot,
        Option<LedgerCause<crate::ImportDimension>>,
    ) {
        let coordinator = self
            .coordinator
            .take()
            .expect("outcome finalizes once")
            .into_inner();
        match coordinator.finish() {
            Ok(snapshot) => (snapshot, None),
            Err(error) => {
                let (cause, snapshot) = error.into_parts();
                (snapshot, Some(cause))
            }
        }
    }

    fn coordinator(&self) -> &RefCell<ImportCoordinator> {
        self.coordinator.as_ref().expect("session is active")
    }

    fn diagnostics(&self) -> &RefCell<DiagnosticSink> {
        self.diagnostics.as_ref().expect("session is active")
    }

    fn dependencies(&self) -> &RefCell<DependencyCache> {
        self.dependencies.as_ref().expect("session is active")
    }

    fn poll_boundary(&self) -> Result<(), ImportCause> {
        self.coordinator()
            .borrow_mut()
            .poll()
            .map_err(|cause| self.scope_cause(ScopeAbortCause::Control(cause)))
    }

    fn report_boundary_progress(
        &self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ImportCause> {
        self.coordinator()
            .borrow_mut()
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.scope_cause(ScopeAbortCause::Control(cause)))
    }

    fn charge_open_attempt(&self) -> Result<(), ImportCause> {
        self.coordinator()
            .borrow_mut()
            .checkpoint()
            .charge_attempted(ImportDimension::AssetOpen, 1)
            .map_err(|cause| self.scope_cause(ScopeAbortCause::Ledger(cause)))
    }

    fn charge_open_consumed(&self) -> Result<(), ImportCause> {
        self.coordinator()
            .borrow_mut()
            .checkpoint()
            .charge_consumed(ImportDimension::AssetOpen, 1)
            .map_err(|cause| self.scope_cause(ScopeAbortCause::Ledger(cause)))
    }

    fn charge_primary_bytes(&self, amount: u64, consumed: bool) -> Result<(), ImportCause> {
        let mut coordinator = self.coordinator().borrow_mut();
        let mut checkpoint = coordinator.checkpoint();
        let result = if consumed {
            checkpoint
                .charge_consumed(ImportDimension::PrimaryBytes, amount)
                .and_then(|()| checkpoint.charge_consumed(ImportDimension::TotalAssetBytes, amount))
        } else {
            checkpoint
                .charge_attempted(ImportDimension::PrimaryBytes, amount)
                .and_then(|()| {
                    checkpoint.charge_attempted(ImportDimension::TotalAssetBytes, amount)
                })
        };
        result.map_err(|cause| self.scope_cause(ScopeAbortCause::Ledger(cause)))
    }

    fn scope_cause(&self, cause: ScopeAbortCause) -> ImportCause {
        ImportCause::Scope(ScopeAbort::new(cause))
    }
}

struct ReportStaging {
    detection: Vec<DetectionEvidence>,
    diagnostics: Vec<crate::ImportDiagnostic>,
    diagnostic_summary: crate::DiagnosticSummary,
    optional_dependency_evidence: Vec<crate::OptionalDependencyEvidence>,
    loss_evidence: Vec<ImportLossEvidence>,
}

struct FailureStaging {
    cause: ImportCause,
    selected_format: Option<FormatId>,
    report: ReportStaging,
    last_completed: ImportPhaseEvidence,
}

impl FailureStaging {
    fn finish(
        self,
        source: AssetPath,
        ledger: crate::ImportLedgerSnapshot,
        finalization_cause: Option<LedgerCause<ImportDimension>>,
    ) -> ImportFailure {
        let ReportStaging {
            detection,
            diagnostics,
            diagnostic_summary,
            optional_dependency_evidence,
            loss_evidence,
        } = self.report;
        ImportFailure::from_parts(ImportFailureParts {
            cause: self.cause,
            source,
            selected_format: self.selected_format,
            detection,
            diagnostics,
            diagnostic_summary,
            optional_dependency_evidence,
            loss_evidence,
            ledger,
            finalization_cause,
            last_completed: self.last_completed,
        })
    }
}

trait PhaseState {}

struct SessionCreated;
struct PrimaryBound;
struct SelectionCompleted;
struct DraftBuilt;

impl PhaseState for SessionCreated {}
impl PhaseState for PrimaryBound {}
impl PhaseState for SelectionCompleted {}
impl PhaseState for DraftBuilt {}

struct PhaseProof<S: PhaseState>(PhantomData<fn() -> S>);

impl<S: PhaseState> PhaseProof<S> {
    const fn new() -> Self {
        Self(PhantomData)
    }
}

type SessionCreatedProof = PhaseProof<SessionCreated>;
type PrimaryBoundProof = PhaseProof<PrimaryBound>;
type SelectionCompletedProof = PhaseProof<SelectionCompleted>;
type DraftBuiltProof = PhaseProof<DraftBuilt>;

fn reserve_exact_bytes(
    coordinator: &RefCell<ImportCoordinator>,
    phase: ImportLedgerPhase,
    dimension: ImportDimension,
    bytes: u64,
) -> Result<Option<ImportReservation>, ScopeAbortCause> {
    if bytes == 0 {
        return Ok(None);
    }
    coordinator
        .borrow_mut()
        .checkpoint()
        .reserve(phase, dimension, bytes)
        .map(Some)
        .map_err(ScopeAbortCause::Ledger)
}

#[allow(clippy::too_many_arguments)]
fn reserve_vec_capacity<T>(
    coordinator: &RefCell<ImportCoordinator>,
    values: &mut Vec<T>,
    additional: usize,
    phase: ImportLedgerPhase,
    dimension: ImportDimension,
    resource: &'static str,
) -> Result<Option<ImportReservation>, ScopeAbortCause> {
    let desired_capacity = values
        .len()
        .checked_add(additional)
        .ok_or(ScopeAbortCause::Allocation { resource })?;
    let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), desired_capacity)
        .map_err(|cause| capacity_plan_cause(resource, cause))?;
    let admitted_bytes = plan
        .envelope_delta_bytes()
        .map_err(|cause| capacity_plan_cause(resource, cause))?;
    let upper_bound = reserve_exact_bytes(coordinator, phase, dimension, admitted_bytes)?;
    plan.verify_preflight(values.len(), values.capacity())
        .map_err(|cause| capacity_plan_cause(resource, cause))?;
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| ScopeAbortCause::Allocation { resource })?;
    let actual_bytes = plan
        .finish(values.len(), values.capacity())
        .map_err(|cause| capacity_plan_cause(resource, cause))?;
    match (upper_bound, actual_bytes) {
        (Some(upper_bound), 0) => {
            drop(upper_bound);
            Ok(None)
        }
        (Some(upper_bound), actual_bytes) => coordinator
            .borrow_mut()
            .checkpoint()
            .transition_reservation(upper_bound, phase, dimension, actual_bytes)
            .map(Some)
            .map_err(ScopeAbortCause::Ledger),
        (None, 0) => Ok(None),
        (None, _) => Err(ScopeAbortCause::DescriptorContract {
            code: "vector-capacity-growth-without-reservation",
        }),
    }
}

fn capacity_plan_cause(resource: &'static str, cause: CapacityPlanError) -> ScopeAbortCause {
    match cause {
        CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
            ScopeAbortCause::ResourceLimit {
                resource,
                limit: u64::MAX,
            }
        }
        CapacityPlanError::InvalidState { .. }
        | CapacityPlanError::StalePlan { .. }
        | CapacityPlanError::CapacityPlatformUnqualified { .. } => {
            ScopeAbortCause::DescriptorContract {
                code: "capacity-plan-contract-failed",
            }
        }
        _ => ScopeAbortCause::DescriptorContract {
            code: "capacity-plan-error-unknown",
        },
    }
}

struct ImportValidationAdapter<'a> {
    coordinator: &'a RefCell<ImportCoordinator>,
    draft_reservations: &'a RefCell<Vec<ImportReservation>>,
    draft_reservation_list_capacity: &'a RefCell<Option<ImportReservation>>,
}

enum ImportValidationReservation {
    One {
        kind: SceneReservationKind,
        reservation: ImportReservation,
    },
    Bundle(Vec<ImportReservation>),
}

impl ImportValidationReservation {
    fn one(kind: SceneReservationKind, reservation: ImportReservation) -> Self {
        Self::One { kind, reservation }
    }

    fn into_reservations(self) -> Result<Vec<ImportReservation>, ScopeAbortCause> {
        match self {
            Self::Bundle(reservations) => Ok(reservations),
            Self::One { .. } => Err(ScopeAbortCause::DescriptorContract {
                code: "publication-received-single-validation-reservation",
            }),
        }
    }
}

impl SceneValidationControl for ImportValidationAdapter<'_> {
    type Error = ScopeAbortCause;
    type Reservation = ImportValidationReservation;

    fn poll(&mut self) -> Result<(), Self::Error> {
        self.coordinator
            .borrow_mut()
            .poll()
            .map_err(ScopeAbortCause::Control)
    }

    fn charge_work(&mut self, _work: SceneValidationWork, amount: u64) -> Result<(), Self::Error> {
        let mut coordinator = self.coordinator.borrow_mut();
        let mut checkpoint = coordinator.checkpoint();
        checkpoint
            .charge_attempted(ImportDimension::WorkUnits, amount)
            .and_then(|()| checkpoint.charge_consumed(ImportDimension::WorkUnits, amount))
            .map_err(ScopeAbortCause::Ledger)
    }

    fn reserve(
        &mut self,
        kind: SceneReservationKind,
        bytes: u64,
    ) -> Result<Self::Reservation, Self::Error> {
        let (phase, dimension) = validation_reservation_accounting(kind)?;
        {
            let mut coordinator = self.coordinator.borrow_mut();
            let mut checkpoint = coordinator.checkpoint();
            if kind == SceneReservationKind::CanonicalOutput {
                checkpoint
                    .charge_attempted(dimension, bytes)
                    .and_then(|()| checkpoint.charge_consumed(dimension, bytes))
                    .map_err(ScopeAbortCause::Ledger)?;
            }
            if kind != SceneReservationKind::CanonicalOutput {
                return checkpoint
                    .reserve(phase, dimension, bytes)
                    .map(|reservation| ImportValidationReservation::one(kind, reservation))
                    .map_err(ScopeAbortCause::Ledger);
            }
        }

        let mut reservations = std::mem::take(&mut *self.draft_reservations.borrow_mut());
        let _reservation_list_capacity = self.draft_reservation_list_capacity.borrow_mut().take();
        let source_count = reservations.len();
        let source_bytes = reservations.iter().try_fold(0_u64, |total, reservation| {
            total
                .checked_add(reservation.bytes())
                .ok_or(ScopeAbortCause::DescriptorContract {
                    code: "draft-reservation-bytes-overflow",
                })
        })?;
        let needs_delta = source_bytes < bytes;
        let payload_token_count = source_count.checked_add(usize::from(needs_delta)).ok_or(
            ScopeAbortCause::Allocation {
                resource: "validation-reservation-bundle",
            },
        )?;
        let mut transitioned = Vec::new();
        let list_capacity_reservation = reserve_vec_capacity(
            self.coordinator,
            &mut transitioned,
            payload_token_count
                .checked_add(1)
                .ok_or(ScopeAbortCause::Allocation {
                    resource: "validation-reservation-bundle",
                })?,
            ImportLedgerPhase::Validation,
            ImportDimension::ScratchBytes,
            "validation-reservation-bundle",
        )?;
        if let Some(list_capacity_reservation) = list_capacity_reservation {
            transitioned.push(list_capacity_reservation);
        }
        while let Some(reservation) = reservations.pop() {
            let reservation_bytes = reservation.bytes();
            let replacement = self
                .coordinator
                .borrow_mut()
                .checkpoint()
                .transition_reservation(reservation, phase, dimension, reservation_bytes)
                .map_err(ScopeAbortCause::Ledger)?;
            transitioned.push(replacement);
        }
        if needs_delta {
            let delta = bytes - source_bytes;
            let reservation = self
                .coordinator
                .borrow_mut()
                .checkpoint()
                .reserve(phase, dimension, delta)
                .map_err(ScopeAbortCause::Ledger)?;
            transitioned.push(reservation);
        }
        Ok(ImportValidationReservation::Bundle(transitioned))
    }

    fn transition_reservation(
        &mut self,
        reservation: Self::Reservation,
        from: SceneReservationKind,
        to: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<Self::Reservation, Self::Error> {
        validation_reservation_accounting(from)?;
        let (phase, dimension) = validation_reservation_accounting(to)?;
        let ImportValidationReservation::One { kind, reservation } = reservation else {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "multi-token-validation-resize",
            });
        };
        if kind != from {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "validation-reservation-kind-mismatch",
            });
        }
        self.coordinator
            .borrow_mut()
            .checkpoint()
            .transition_reservation(reservation, phase, dimension, required_bytes)
            .map(|reservation| ImportValidationReservation::one(to, reservation))
            .map_err(ScopeAbortCause::Ledger)
    }

    fn resize_reservation(
        &mut self,
        reservation: &mut Self::Reservation,
        kind: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        validation_reservation_accounting(kind)?;
        let ImportValidationReservation::One {
            kind: reserved_kind,
            reservation,
        } = reservation
        else {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "multi-token-validation-resize",
            });
        };
        if *reserved_kind != kind {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "validation-reservation-kind-mismatch",
            });
        }
        self.coordinator
            .borrow_mut()
            .checkpoint()
            .resize_reservation(reservation, required_bytes)
            .map_err(ScopeAbortCause::Ledger)
    }
}

fn validation_reservation_accounting(
    kind: SceneReservationKind,
) -> Result<(ImportLedgerPhase, ImportDimension), ScopeAbortCause> {
    match kind {
        SceneReservationKind::ValidationScratch => {
            Ok((ImportLedgerPhase::Validation, ImportDimension::ScratchBytes))
        }
        SceneReservationKind::CanonicalOutput => Ok((
            ImportLedgerPhase::Validation,
            ImportDimension::CanonicalOutputBytes,
        )),
        SceneReservationKind::EditCandidate => Err(ScopeAbortCause::DescriptorContract {
            code: "edit-candidate-reservation-in-import-validation",
        }),
    }
}

fn scene_validation_limits(limits: &ResourceLimits) -> SceneValidationLimits {
    let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
    let payload_cells = [
        limits.max_total_vertex_attribute_cells,
        limits.max_total_topology_cells,
        limits.max_influence_cells,
        limits.max_morph_cells,
        limits.max_animation_cells,
    ]
    .into_iter()
    .fold(0_u64, |total, value| total.saturating_add(count(value)));

    SceneValidationLimits::default()
        .with_max_nodes(count(limits.max_graph_nodes))
        .with_max_edges(count(limits.max_graph_edges))
        .with_max_depth(count(limits.max_graph_depth))
        // Parser records are an input-work budget, not a canonical entity bound. Every retained
        // entity contributes at least one accounted payload byte, so the canonical byte ceiling is
        // the non-conflicting aggregate guard for entity traversal.
        .with_max_entities(limits.max_canonical_payload_bytes)
        .with_max_references(count(limits.max_graph_work))
        .with_max_payload_cells(payload_cells)
        .with_max_work(limits.max_work_units)
        .with_max_metadata_entries(count(limits.max_records))
        .with_max_metadata_bytes(count(limits.max_canonical_string_bytes))
        .with_max_metadata_key_bytes(limits.max_string_bytes)
        .with_max_metadata_string_bytes(limits.max_canonical_string_bytes)
        .with_max_canonical_bytes(limits.max_canonical_payload_bytes)
        .with_max_scratch_bytes(limits.max_live_bytes)
}

fn publication_scope_cause(cause: PublicationCause<ImportDimension>) -> ScopeAbortCause {
    match cause {
        PublicationCause::Control(cause) => ScopeAbortCause::Control(cause),
        PublicationCause::Ledger(cause) => ScopeAbortCause::Ledger(cause),
        _ => ScopeAbortCause::DescriptorContract {
            code: "unknown-publication-cause",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CapabilityStatus, FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter,
        ImportRequest, OptionCompileLimits, ParseResult, ParseScope, RegistryBuildLimits,
        RegistryBuilder,
    };
    use baozi_operation::{
        LedgerEvidenceError, LedgerEvidencePurpose, LedgerLimitsError, ProgressDecision,
    };
    use std::error::Error;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[cfg(feature = "tracing")]
    use tracing::span::{Attributes, Id, Record};
    #[cfg(feature = "tracing")]
    use tracing::{Event, Metadata, Subscriber};

    #[cfg(feature = "tracing")]
    struct SpanCountingSubscriber {
        span_calls: Arc<AtomicUsize>,
    }

    #[cfg(feature = "tracing")]
    impl Subscriber for SpanCountingSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _attributes: &Attributes<'_>) -> Id {
            let id = self.span_calls.fetch_add(1, Ordering::SeqCst) + 1;
            Id::from_u64(u64::try_from(id).expect("test span id should fit in u64"))
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, _event: &Event<'_>) {}

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    struct NeverInvokedImporter;

    impl FormatImporter for NeverInvokedImporter {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.initialization").expect("valid test format id"),
                "Initialization Test",
            )
            .extensions(["init"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .expect("valid test descriptor")
        }

        fn parse(&self, _scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            panic!("ledger initialization failure must not invoke the importer")
        }
    }

    struct InitializationSpyIo {
        domain: AuthorityDomain,
        authority_calls: Arc<AtomicUsize>,
        open_calls: Arc<AtomicUsize>,
    }

    impl AssetIo for InitializationSpyIo {
        fn authority_domain(&self) -> AuthorityDomain {
            self.authority_calls.fetch_add(1, Ordering::SeqCst);
            self.domain.clone()
        }

        fn open(&self, _path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
            self.open_calls.fetch_add(1, Ordering::SeqCst);
            Err(AssetOpenError::new(
                AssetOpenStage::Open,
                AssetOpenErrorKind::NotFound,
            ))
        }
    }

    fn initialization_test_request() -> PreparedImportRequest {
        let mut registry = RegistryBuilder::new();
        registry
            .register(NeverInvokedImporter)
            .expect("test importer registration");
        registry
            .freeze(RegistryBuildLimits::default())
            .expect("test registry freeze")
            .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
            .expect("test request compilation")
    }

    #[test]
    #[allow(
        clippy::result_large_err,
        reason = "the unwind boundary exercises the complete by-value import run error"
    )]
    fn ledger_initialization_failure_is_typed_and_has_no_session_or_host_side_effects() {
        let request = initialization_test_request();
        let authority_calls = Arc::new(AtomicUsize::new(0));
        let open_calls = Arc::new(AtomicUsize::new(0));
        let observer_calls = Arc::new(AtomicUsize::new(0));
        let io = InitializationSpyIo {
            domain: AuthorityDomain::new(),
            authority_calls: Arc::clone(&authority_calls),
            open_calls: Arc::clone(&open_calls),
        };
        let observed_calls = Arc::clone(&observer_calls);
        let control = ImportOperationControl::default().with_progress_observer(move |_| {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            ProgressDecision::Continue
        });
        let injected = LedgerLimitsError::Evidence {
            cause: LedgerEvidenceError::AllocationFailed {
                purpose: LedgerEvidencePurpose::Dimensions,
                requested_bytes: 128,
            },
        };
        let source = AssetPath::new("asset.init").expect("valid test path");

        #[cfg(feature = "tracing")]
        let span_calls = Arc::new(AtomicUsize::new(0));
        #[cfg(feature = "tracing")]
        let outcome = tracing::subscriber::with_default(
            SpanCountingSubscriber {
                span_calls: Arc::clone(&span_calls),
            },
            || {
                catch_unwind(AssertUnwindSafe(|| {
                    import_asset_with_resolver_and_ledger_limits(
                        &request,
                        &io,
                        source,
                        &DenyDependencies,
                        control,
                        Err(injected.clone()),
                    )
                }))
            },
        );
        #[cfg(not(feature = "tracing"))]
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            import_asset_with_resolver_and_ledger_limits(
                &request,
                &io,
                source,
                &DenyDependencies,
                control,
                Err(injected.clone()),
            )
        }));
        let error = outcome
            .expect("ledger initialization failure must not unwind")
            .expect_err("injected ledger initialization must fail");

        assert!(matches!(
            &error,
            ImportRunError::Initialization { source } if source == &injected
        ));
        assert!(error.operation_failure().is_none());
        let limits_source = Error::source(&error)
            .and_then(|source| source.downcast_ref::<LedgerLimitsError<ImportDimension>>())
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
        assert_eq!(authority_calls.load(Ordering::SeqCst), 0);
        assert_eq!(open_calls.load(Ordering::SeqCst), 0);
        assert_eq!(observer_calls.load(Ordering::SeqCst), 0);
        #[cfg(feature = "tracing")]
        assert_eq!(span_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn validation_resize_failure_preserves_the_existing_live_token() {
        let limits = ResourceLimits {
            max_live_bytes: 4,
            ..ResourceLimits::default()
        };
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let draft_reservations = RefCell::new(Vec::new());
        let draft_reservation_list_capacity = RefCell::new(None);
        let reservation = {
            let mut adapter = ImportValidationAdapter {
                coordinator: &coordinator,
                draft_reservations: &draft_reservations,
                draft_reservation_list_capacity: &draft_reservation_list_capacity,
            };
            let mut reservation = adapter
                .reserve(SceneReservationKind::ValidationScratch, 4)
                .expect("the initial scratch token fits the live limit");

            let failure = adapter
                .resize_reservation(&mut reservation, SceneReservationKind::ValidationScratch, 5)
                .expect_err("growth above the live limit must fail atomically");
            assert!(matches!(failure, ScopeAbortCause::Ledger(_)));
            let ImportValidationReservation::One {
                kind,
                reservation: token,
            } = &reservation
            else {
                panic!("validation scratch uses one live token")
            };
            assert_eq!(*kind, SceneReservationKind::ValidationScratch);
            assert_eq!(token.bytes(), 4);
            reservation
        };

        drop(reservation);
        let ledger = coordinator
            .into_inner()
            .finish()
            .expect("dropping the preserved token permits clean finalization");
        assert_eq!(ledger.current_live_bytes(), 0);
    }

    #[test]
    fn direct_primary_binding_reserves_its_reservation_list_before_copying() {
        let request = initialization_test_request();
        let io = DirectBytesAuthority::new();
        let limits = ResourceLimits {
            max_live_bytes: 64,
            ..ResourceLimits::default()
        };
        let mut session = ImportSession::new(
            &request,
            &io,
            AssetPath::new("asset.direct-reservation").expect("valid test path"),
            &DenyDependencies,
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid ledger limits"),
        );

        assert!(
            session
                .bind_primary_bytes(&[0; 64], SessionCreatedProof::new())
                .is_err(),
            "the reservation list and payload must be admitted together"
        );
        assert!(session.primary_bytes.is_none());
        assert!(session.primary_reservations.is_empty());
        assert_eq!(session.last_completed.phase(), ImportPhase::SessionCreated);
    }
}
