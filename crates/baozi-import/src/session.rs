use crate::SceneDraft;
use crate::asset_open::{AssetOpenContext, open_asset};
use crate::config::FormatSelection;
use crate::dependency::{
    DenyDependencies, DependencyOriginIssuer, DependencyResolver, ResolverBinding,
};
use crate::detection::DetectionScope;
use crate::diagnostic::DiagnosticSink;
use crate::error::{
    DetectionFailure, ImportCause, ScopeAbort, ScopeAbortCause, TrustedExtensionBoundary,
};
use crate::format::{DetectionOutcome, Probe, ReadConfidence, discard_untrusted_panic_payload};
use crate::ledger::{
    ImportDimension, ImportLedgerPhase, ImportOperationControl, ImportProgressPhase,
    ImportProgressUnit, ImportTerminalCause, import_ledger_limits, tighten_selected_format_limits,
};
use crate::limits::ResourceLimits;
use crate::loss::ImportLossEvidence;
use crate::metered_storage::AccountedVec;
use crate::outcome::{
    AccountedErrorContext, DetectionEvidence, ImportFailure, ImportFailureParts,
    ImportFailureStoreOwner, ImportPhase, ImportPhaseEvidence, ImportReport, ImportReportParts,
    ImportResult, ImportRunError,
};
use crate::registry::{ParseInvocationError, RegistryEntry};
use crate::request::PreparedImportRequest;
use crate::resource_graph::ResourceGraph;
use crate::scope::{
    FailureEvidenceStorage, ParseFailureKind, ParseScope, ParseScopeParts, read_opened_asset,
};
use crate::source_scope::{LiveSourceScopeRecord, SourceScopeRecord};
use crate::store::{
    ImportStoreBootstrapRequirements, ImportStoreFailureCauseAuthority, ImportStoreReservation,
    ImportStoreSceneBoundary, ImportStoreSessionOwner, ImportStoreTerminalEvidence,
    ImportStoreTerminalOutcome, try_import_store_bootstrap,
};
use baozi_core::FormatId;
use baozi_io::{
    __semantic_identity::same_authority_resolver_v1, AssetIo, AssetOpenError, AssetOpenErrorKind,
    AssetOpenStage, AssetPath, AuthorityDomain, OpenedAsset,
};
use baozi_operation::{ControlCause, LedgerLimits, LedgerLimitsError};
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
    import_asset_with_resolver_and_ledger_limits(
        request,
        io,
        source,
        ResolverBinding::registered(same_authority_resolver_v1()),
        control,
        import_ledger_limits(request.limits()),
    )
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
        ResolverBinding::custom(resolver),
        control,
        import_ledger_limits(request.limits()),
    )
}

#[allow(
    clippy::result_large_err,
    reason = "terminal import failures retain complete bounded session evidence without heap indirection"
)]
fn import_asset_with_resolver_and_ledger_limits<'a>(
    request: &PreparedImportRequest,
    io: &dyn AssetIo,
    source: AssetPath,
    resolver: ResolverBinding<'a>,
    control: ImportOperationControl,
    ledger_limits: Result<LedgerLimits<ImportDimension>, LedgerLimitsError<ImportDimension>>,
) -> ImportResult<ImportReport> {
    let store = try_start_store(request, control, ledger_limits)?;
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.import").entered();

    let mut session = ImportSession::new(request, io, source, resolver, store);
    let execution = session.execute(SessionCreatedProof::new());
    match execution {
        Ok(draft) => session.finalize_success(draft),
        Err(cause) => session.finalize_failure(cause),
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
    let ledger_limits = import_ledger_limits(request.limits());
    let store = try_start_store(request, control, ledger_limits)?;
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.import").entered();

    let primary_io = DirectBytesAuthority::new();
    let mut session = ImportSession::new(
        request,
        &primary_io,
        source,
        ResolverBinding::custom(&DenyDependencies),
        store,
    );
    let execution = session.execute_bytes(bytes, SessionCreatedProof::new());
    match execution {
        Ok(draft) => session.finalize_success(draft),
        Err(cause) => session.finalize_failure(cause),
    }
}

#[allow(
    clippy::result_large_err,
    reason = "import errors retain the linear terminal owner without post-terminal allocation"
)]
fn try_start_store(
    request: &PreparedImportRequest,
    control: ImportOperationControl,
    ledger_limits: Result<LedgerLimits<ImportDimension>, LedgerLimitsError<ImportDimension>>,
) -> Result<ImportStoreSessionOwner, ImportRunError> {
    ledger_limits.map_err(ImportRunError::initialization)?;
    let account = request.try_create_storage_account().map_err(|_| {
        ImportRunError::retained_store_initialization("physical-storage-account-unavailable")
    })?;
    let requirements =
        ImportStoreBootstrapRequirements::for_node_scene(request.limits()).map_err(|_| {
            ImportRunError::retained_store_initialization("retained-store-bootstrap-rejected")
        })?;
    let bootstrap = try_import_store_bootstrap(request.limits(), requirements).map_err(|_| {
        ImportRunError::retained_store_initialization("retained-store-plan-rejected")
    })?;
    ImportStoreSessionOwner::try_start(control, &account, bootstrap).map_err(|_cause| {
        ImportRunError::retained_store_initialization("retained-store-start-rejected")
    })
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

    fn open_work_bound(
        &self,
        _path: &AssetPath,
    ) -> Result<std::num::NonZeroU64, baozi_io::AssetOpenWorkError> {
        // Direct bytes have no backend-open entry point, even for failed requests.
        Err(baozi_io::AssetOpenWorkError::Unbounded)
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
    resolver: ResolverBinding<'a>,
    dependency_origin_issuer: DependencyOriginIssuer,
    store: ImportStoreSessionOwner,
    diagnostics: Option<RefCell<DiagnosticSink>>,
    resource_graph: Option<RefCell<ResourceGraph>>,
    optional_dependency_evidence: RefCell<AccountedVec<crate::OptionalDependencyEvidence>>,
    loss_evidence: RefCell<AccountedVec<ImportLossEvidence>>,
    source_scope_record: RefCell<Option<LiveSourceScopeRecord>>,
    terminal: RefCell<Option<ScopeAbortCause>>,
    primary_authority: Option<AuthorityDomain>,
    primary_backend_hard_limit: Option<u64>,
    primary_snapshot: Option<AccountedVec<u8>>,
    format_failure_storage: RefCell<FailureEvidenceStorage>,
    selected: Option<RegistryEntry>,
    effective_limits: Option<ResourceLimits>,
    detection: AccountedVec<DetectionEvidence>,
    last_completed: ImportPhaseEvidence,
}

impl<'a> ImportSession<'a> {
    fn new(
        request: &'a PreparedImportRequest,
        primary_io: &'a dyn AssetIo,
        source: AssetPath,
        resolver: ResolverBinding<'a>,
        store: ImportStoreSessionOwner,
    ) -> Self {
        Self {
            request,
            primary_io,
            source,
            resolver,
            dependency_origin_issuer: DependencyOriginIssuer::new(),
            store,
            diagnostics: Some(RefCell::new(DiagnosticSink::new(
                request.limits().max_diagnostics,
                request.limits().max_diagnostic_bytes,
                request.limits().max_diagnostic_message_bytes,
                request.limits().max_attempted_diagnostic_bytes,
            ))),
            resource_graph: Some(RefCell::new(ResourceGraph::new(request.limits().clone()))),
            optional_dependency_evidence: RefCell::new(AccountedVec::new()),
            loss_evidence: RefCell::new(AccountedVec::new()),
            source_scope_record: RefCell::new(None),
            terminal: RefCell::new(None),
            primary_authority: None,
            primary_backend_hard_limit: None,
            primary_snapshot: None,
            format_failure_storage: RefCell::new(FailureEvidenceStorage::default()),
            selected: None,
            effective_limits: None,
            detection: AccountedVec::new(),
            last_completed: ImportPhaseEvidence::new(ImportPhase::SessionCreated),
        }
    }

    fn execute(&mut self, created: SessionCreatedProof) -> Result<SceneDraft, ImportCause> {
        let primary = self.bind_primary(created)?;
        let selected = self.select_format(primary)?;
        let (draft, _) = self.parse_selected(selected)?;
        Ok(draft)
    }

    fn execute_bytes(
        &mut self,
        bytes: &[u8],
        created: SessionCreatedProof,
    ) -> Result<SceneDraft, ImportCause> {
        let primary = self.bind_primary_bytes(bytes, created)?;
        let selected = self.select_format(primary)?;
        let (draft, _) = self.parse_selected(selected)?;
        Ok(draft)
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
        let opened = open_asset(
            self.host(),
            self.primary_io,
            &self.source,
            AssetOpenContext::Primary,
        )
        .map_err(|cause| match cause {
            ScopeAbortCause::TrustedExtension { boundary, code } => {
                ImportCause::TrustedExtension { boundary, code }
            }
            cause => self.scope_cause(cause),
        })?;
        if opened.authority_domain() != &expected_domain || opened.path() != &self.source {
            return Err(ImportCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code: "primary-open-identity-mismatch",
            });
        }
        let backend_hard_limit = opened.backend_hard_limit();
        self.charge_open_consumed()?;
        let snapshot = read_opened_asset(
            self.host(),
            opened,
            ImportLedgerPhase::PrimarySnapshot,
            ImportProgressPhase::AcquirePrimary,
            ImportDimension::PrimaryBytes,
            self.request.limits().max_primary_asset_bytes,
            self.request.limits().max_total_asset_bytes,
        )
        .map_err(|cause| self.scope_cause(cause))?;
        self.resource_graph()
            .borrow_mut()
            .bind_primary(self.host(), &expected_domain, &self.source, &snapshot)
            .map_err(|cause| self.scope_cause(cause.into_scope_cause()))?;
        self.primary_authority = Some(expected_domain);
        self.primary_backend_hard_limit = backend_hard_limit;
        self.primary_snapshot = Some(snapshot);
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

        let mut snapshot = AccountedVec::new();
        snapshot
            .try_reserve(bytes.len(), "direct-primary-snapshot", |capacity_bytes| {
                reserve_required_bytes(
                    self.host(),
                    ImportLedgerPhase::PrimarySnapshot,
                    ImportDimension::PrimaryBytes,
                    capacity_bytes,
                )
            })
            .map_err(|cause| self.scope_cause(cause))?;
        snapshot
            .try_extend_from_slice_reserved(bytes)
            .map_err(|()| {
                self.scope_cause(ScopeAbortCause::DescriptorContract {
                    code: "direct-primary-snapshot-slots-not-admitted",
                })
            })?;
        self.charge_primary_bytes(byte_count, true)?;
        self.resource_graph()
            .borrow_mut()
            .bind_primary(self.host(), &primary_authority, &self.source, &snapshot)
            .map_err(|cause| self.scope_cause(cause.into_scope_cause()))?;
        self.primary_snapshot = Some(snapshot);
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
        let bytes = self.primary_snapshot.as_ref().ok_or_else(|| {
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
            self.host(),
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
        let mut entries = AccountedVec::new();
        entries
            .try_reserve(entry_count, "detection-candidates", |bytes| {
                reserve_required_bytes(
                    self.host(),
                    ImportLedgerPhase::Detection,
                    ImportDimension::ScratchBytes,
                    bytes,
                )
            })
            .map_err(|cause| self.scope_cause(cause))?;
        match self.request.config().format_selection() {
            FormatSelection::Forced(id) => {
                if let Some(entry) = self.request.entry(id) {
                    entries.try_push_reserved(entry).map_err(|_| {
                        self.scope_cause(ScopeAbortCause::DescriptorContract {
                            code: "detection-candidate-slots-not-admitted",
                        })
                    })?;
                }
            }
            FormatSelection::Automatic | FormatSelection::Preferred(_) => {
                for entry in self.request.snapshot().entries() {
                    entries.try_push_reserved(entry).map_err(|_| {
                        self.scope_cause(ScopeAbortCause::DescriptorContract {
                            code: "detection-candidate-slots-not-admitted",
                        })
                    })?;
                }
            }
        }
        if !self.detection.is_empty() || self.detection.capacity_bytes() != 0 {
            return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "detection-storage-reserved-more-than-once",
            }));
        }
        let host = self.store.host();
        self.detection
            .try_reserve(entries.len(), "detection-evidence", |bytes| {
                reserve_retained_bytes(
                    host,
                    ImportLedgerPhase::OutcomeFinalization,
                    ImportDimension::ScratchBytes,
                    bytes,
                )
            })
            .map_err(|cause| self.scope_cause(cause))?;
        let total = entries.len() as u64;
        for (index, entry) in entries.iter().enumerate() {
            self.poll_boundary()?;
            let (outcome, latched) = {
                let scope = DetectionScope::new(self.host(), &self.terminal);
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
                .try_push_reserved(DetectionEvidence::new(entry.format_id().clone(), outcome))
                .map_err(|_| {
                    self.scope_cause(ScopeAbortCause::DescriptorContract {
                        code: "detection-evidence-slots-not-admitted",
                    })
                })?;
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

        let mut candidates = AccountedVec::new();
        candidates
            .try_reserve(entries.len(), "detection-candidate-ranking", |bytes| {
                reserve_required_bytes(
                    self.host(),
                    ImportLedgerPhase::Detection,
                    ImportDimension::ScratchBytes,
                    bytes,
                )
            })
            .map_err(|cause| self.scope_cause(cause))?;
        for candidate in
            entries
                .iter()
                .zip(self.detection.iter())
                .filter_map(|(entry, evidence)| match evidence.outcome() {
                    DetectionOutcome::Match(confidence) => Some((entry.clone(), *confidence)),
                    _ => None,
                })
        {
            candidates.try_push_reserved(candidate).map_err(|_| {
                self.scope_cause(ScopeAbortCause::DescriptorContract {
                    code: "detection-ranking-slots-not-admitted",
                })
            })?;
        }
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
        candidates: AccountedVec<(RegistryEntry, ReadConfidence)>,
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
        let mut extension_matches = AccountedVec::new();
        extension_matches
            .try_reserve(
                candidates.len(),
                "extension-detection-candidates",
                |bytes| {
                    reserve_required_bytes(
                        self.host(),
                        ImportLedgerPhase::Detection,
                        ImportDimension::ScratchBytes,
                        bytes,
                    )
                },
            )
            .map_err(|cause| self.scope_cause(cause))?;
        if let Some(extension) = self.source.extension() {
            for candidate in candidates.iter().filter(|(entry, _)| {
                entry
                    .descriptor()
                    .extensions()
                    .any(|candidate| candidate.eq_ignore_ascii_case(extension))
            }) {
                extension_matches
                    .try_push_reserved(candidate)
                    .map_err(|_| {
                        self.scope_cause(ScopeAbortCause::DescriptorContract {
                            code: "extension-detection-slots-not-admitted",
                        })
                    })?;
            }
        }
        match &*extension_matches {
            [(entry, _)] => Ok((*entry).clone()),
            [] if candidates.len() == 1 && unique_unhinted_allowed => Ok(candidates[0].0.clone()),
            [] if unique_unhinted_allowed => {
                drop(extension_matches);
                self.ambiguous(candidates)
            }
            [] => Err(self.no_selection_cause()),
            _ => {
                drop(extension_matches);
                self.ambiguous(candidates)
            }
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
        mut candidates: AccountedVec<(RegistryEntry, ReadConfidence)>,
    ) -> Result<RegistryEntry, ImportCause> {
        let mut format_ids = AccountedVec::new();
        format_ids
            .try_reserve(candidates.len(), "ambiguous-format-candidates", |bytes| {
                reserve_retained_bytes(
                    self.host(),
                    ImportLedgerPhase::OutcomeFinalization,
                    ImportDimension::ScratchBytes,
                    bytes,
                )
            })
            .map_err(|cause| self.scope_cause(cause))?;
        while let Some((entry, _)) = candidates.pop() {
            format_ids
                .try_push_reserved(entry.format_id().clone())
                .map_err(|_| {
                    self.scope_cause(ScopeAbortCause::DescriptorContract {
                        code: "ambiguous-format-candidate-slots-not-admitted",
                    })
                })?;
        }
        drop(candidates);
        format_ids.sort_unstable();
        format_ids.dedup();
        let candidates = crate::error::DetectionCandidates::try_new(format_ids, self.host())
            .map_err(|cause| self.scope_cause(cause))?;
        Err(ImportCause::Detection(DetectionFailure::Ambiguous {
            candidates,
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
        if let Some(limits) = self.effective_limits.as_ref() {
            self.resource_graph()
                .borrow_mut()
                .set_limits(limits.clone())
                .map_err(|cause| self.scope_cause(cause.into_scope_cause()))?;
        }
        let tightening = {
            let effective_limits = self.effective_limits.as_ref().ok_or_else(|| {
                self.scope_cause(ScopeAbortCause::DescriptorContract {
                    code: "selected-limits-not-frozen",
                })
            })?;
            tighten_selected_format_limits(self.host(), effective_limits)
        };
        tightening.map_err(|cause| self.scope_cause(cause.into_scope_abort_cause()))
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
                primary_bytes: self.primary_snapshot.as_ref().ok_or_else(|| {
                    self.scope_cause(ScopeAbortCause::DescriptorContract {
                        code: "selection-proof-without-primary-snapshot",
                    })
                })?,
                resolver: self.resolver,
                dependency_origin_issuer: &self.dependency_origin_issuer,
                conformance_policy: self.request.config().conformance_policy(),
                loss_policy: self.request.config().loss_policy(),
                limits: effective_limits,
                store_owner: &self.store,
                host: self.host(),
                diagnostics: self.diagnostics(),
                resource_graph: self.resource_graph(),
                optional_dependency_evidence: &self.optional_dependency_evidence,
                loss_evidence: &self.loss_evidence,
                source_scope_record: &self.source_scope_record,
                format_failure_storage: &self.format_failure_storage,
                terminal: &self.terminal,
            });
            let parsed = entry.parse(&mut scope);
            let latched = scope.take_terminal_abort();
            (parsed, latched)
        };
        match self
            .store
            .completed_scene_boundary()
            .map_err(|cause| self.scope_cause(cause.into_scope_abort_cause()))?
        {
            Some(ImportStoreSceneBoundary::DraftBuilt) => {
                self.last_completed = ImportPhaseEvidence::new(ImportPhase::DraftBuilt);
            }
            Some(ImportStoreSceneBoundary::Validated) => {
                self.last_completed = ImportPhaseEvidence::new(ImportPhase::Validated);
            }
            None => {}
        }
        let post_parse_poll = self
            .store
            .poll_parser_boundary()
            .map_err(|cause| self.scope_cause(cause.into_scope_abort_cause()));
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
                    if !self.format_failure_storage.borrow().format_captured
                        || !envelope.has_storage()
                    {
                        return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                            code: "format-failure-without-storage-reservation",
                        }));
                    }
                    return Err(ImportCause::Format(envelope));
                }
            },
            Ok(Ok(draft)) => {
                if self.format_failure_storage.borrow().is_active() {
                    return Err(self.scope_cause(ScopeAbortCause::DescriptorContract {
                        code: "format-failure-was-swallowed",
                    }));
                }
                draft
            }
        };
        debug_assert_eq!(self.last_completed.phase(), ImportPhase::Validated);
        Ok((draft, DraftBuiltProof::new()))
    }

    #[allow(
        clippy::result_large_err,
        reason = "terminal import failures retain complete bounded session evidence without heap indirection"
    )]
    fn finalize_success(mut self, draft: SceneDraft) -> ImportResult<ImportReport> {
        if self.format_failure_storage.get_mut().is_active() {
            drop(draft);
            let cause = self.scope_cause(ScopeAbortCause::DescriptorContract {
                code: "format-failure-was-swallowed",
            });
            return self.finalize_failure(cause);
        }
        let Some(format) = self.selected.as_ref().map(RegistryEntry::descriptor_handle) else {
            drop(draft);
            return self.finalize_failure(ImportCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::Importer,
                code: "scene-draft-without-selected-format",
            });
        };
        let successful_input_manifest = match match self.resource_graph.as_ref().map(|graph| {
            graph
                .borrow_mut()
                .take_manifest()
                .map_err(|cause| cause.into_scope_cause())
        }) {
            Some(result) => result,
            None => Err(ScopeAbortCause::DescriptorContract {
                code: "resource-graph-missing-at-success",
            }),
        } {
            Ok(manifest) => manifest,
            Err(cause) => {
                drop(draft);
                let cause = self.scope_cause(cause);
                return self.finalize_failure(cause);
            }
        };
        let staging = self.take_report_staging();
        self.release_transient_live();
        let staging = staging.into_terminal();
        let source = self.source;
        let mut last_completed = self.last_completed;
        let terminal = self
            .store
            .finish_scene(Some(draft))
            .map_err(|_| ImportRunError::finalization("canonical-scene-finalization-contained"))?;
        match terminal {
            ImportStoreTerminalOutcome::Success {
                scene,
                report,
                evidence,
            } => {
                last_completed = ImportPhaseEvidence::new(ImportPhase::Validated);
                let TerminalReportEvidence {
                    detection,
                    diagnostics,
                    diagnostic_summary,
                    optional_dependency_evidence,
                    loss_evidence,
                    source_scope_record,
                } = staging;
                Ok(ImportReport::from_parts(ImportReportParts {
                    scene,
                    scene_report: report,
                    source,
                    format,
                    detection,
                    diagnostics,
                    diagnostic_summary,
                    optional_dependency_evidence,
                    loss_evidence,
                    source_scope_record,
                    successful_input_manifest,
                    ledger: crate::ImportLedgerSnapshot::from_store(evidence),
                    last_completed,
                }))
            }
            ImportStoreTerminalOutcome::RichFailure {
                failure,
                evidence,
                control_cause,
                cause_authority: _,
            } => {
                let cause = control_cause.map_or_else(
                    || operation_terminal_cause(&evidence),
                    |cause| ImportCause::Scope(ScopeAbort::new(ScopeAbortCause::Control(cause))),
                );
                Err(ImportRunError::operation(terminal_failure(
                    TerminalFailureCause {
                        context: AccountedErrorContext::new(cause, None),
                        terminal_control_cause: None,
                    },
                    source,
                    Some(format.id().clone()),
                    staging,
                    crate::ImportLedgerSnapshot::from_store(evidence),
                    ImportFailureStoreOwner::Rich { _owner: failure },
                    last_completed,
                )))
            }
            ImportStoreTerminalOutcome::MinimalFallback {
                fallback,
                evidence,
                control_cause,
                cause_authority: _,
            } => {
                let cause = control_cause.map_or_else(
                    || operation_terminal_cause(&evidence),
                    |cause| ImportCause::Scope(ScopeAbort::new(ScopeAbortCause::Control(cause))),
                );
                Err(ImportRunError::operation(terminal_failure(
                    TerminalFailureCause {
                        context: AccountedErrorContext::new(cause, None),
                        terminal_control_cause: None,
                    },
                    source,
                    Some(format.id().clone()),
                    staging,
                    crate::ImportLedgerSnapshot::from_store(evidence),
                    ImportFailureStoreOwner::Minimal { _owner: fallback },
                    last_completed,
                )))
            }
        }
    }

    #[allow(
        clippy::result_large_err,
        reason = "import failures retain the linear terminal owner without post-terminal allocation"
    )]
    fn finalize_failure(mut self, cause: ImportCause) -> ImportResult<ImportReport> {
        let selected_format = self
            .selected
            .as_ref()
            .map(|entry| entry.format_id().clone());
        let staging = self.take_report_staging();
        let cause = self.take_failure_cause(cause);
        self.release_transient_live();
        let staging = staging.into_terminal();
        let source = self.source;
        let last_completed = self.last_completed;
        let terminal = self.store.finish_scene(None).map_err(|_cause| {
            ImportRunError::finalization("canonical-failure-finalization-contained")
        })?;
        let failure = match terminal {
            ImportStoreTerminalOutcome::Success { .. } => {
                return Err(ImportRunError::finalization(
                    "failed-import-produced-canonical-success",
                ));
            }
            ImportStoreTerminalOutcome::RichFailure {
                failure,
                evidence,
                control_cause,
                cause_authority,
            } => {
                let cause =
                    select_terminal_failure_cause(cause, &evidence, control_cause, cause_authority);
                terminal_failure(
                    cause,
                    source,
                    selected_format,
                    staging,
                    crate::ImportLedgerSnapshot::from_store(evidence),
                    ImportFailureStoreOwner::Rich { _owner: failure },
                    last_completed,
                )
            }
            ImportStoreTerminalOutcome::MinimalFallback {
                fallback,
                evidence,
                control_cause,
                cause_authority,
            } => {
                let cause =
                    select_terminal_failure_cause(cause, &evidence, control_cause, cause_authority);
                terminal_failure(
                    cause,
                    source,
                    selected_format,
                    staging,
                    crate::ImportLedgerSnapshot::from_store(evidence),
                    ImportFailureStoreOwner::Minimal { _owner: fallback },
                    last_completed,
                )
            }
        };
        Err(ImportRunError::operation(failure))
    }

    fn release_transient_live(&mut self) {
        drop(self.primary_snapshot.take());
        drop(self.resource_graph.take());
    }

    fn take_report_staging(&mut self) -> LiveReportStaging {
        let diagnostics = self
            .diagnostics
            .take()
            .map(RefCell::into_inner)
            .unwrap_or_else(|| DiagnosticSink::new(0, 0, 0, 0));
        LiveReportStaging {
            detection: self.detection.take(),
            diagnostics,
            optional_dependency_evidence: std::mem::take(
                self.optional_dependency_evidence.get_mut(),
            ),
            loss_evidence: std::mem::take(self.loss_evidence.get_mut()),
            source_scope_record: self.source_scope_record.get_mut().take(),
        }
    }

    fn take_failure_cause(&mut self, cause: ImportCause) -> TerminalFailureCause {
        TerminalFailureCause {
            context: AccountedErrorContext::new(
                cause,
                self.format_failure_storage.get_mut().reservation.take(),
            ),
            terminal_control_cause: None,
        }
    }

    fn host(&self) -> crate::store::ImportStoreHost<'_> {
        self.store.host()
    }

    fn diagnostics(&self) -> &RefCell<DiagnosticSink> {
        self.diagnostics.as_ref().expect("session is active")
    }

    fn resource_graph(&self) -> &RefCell<ResourceGraph> {
        self.resource_graph.as_ref().expect("session is active")
    }

    fn poll_boundary(&self) -> Result<(), ImportCause> {
        self.host()
            .poll()
            .map_err(|cause| self.scope_cause(cause.into_scope_abort_cause()))
    }

    fn report_boundary_progress(
        &self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> Result<(), ImportCause> {
        self.host()
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.scope_cause(cause.into_scope_abort_cause()))
    }

    fn charge_open_attempt(&self) -> Result<(), ImportCause> {
        self.host()
            .charge_attempted(ImportDimension::AssetOpen, 1)
            .map_err(|cause| self.scope_cause(cause.into_scope_abort_cause()))
    }

    fn charge_open_consumed(&self) -> Result<(), ImportCause> {
        self.host()
            .charge_consumed(ImportDimension::AssetOpen, 1)
            .map_err(|cause| self.scope_cause(cause.into_scope_abort_cause()))
    }

    fn charge_primary_bytes(&self, amount: u64, consumed: bool) -> Result<(), ImportCause> {
        let result = if consumed {
            self.host().charge_consumed_batch(&[
                (ImportDimension::PrimaryBytes, amount),
                (ImportDimension::TotalAssetBytes, amount),
            ])
        } else {
            self.host()
                .charge_attempted(ImportDimension::PrimaryBytes, amount)
                .and_then(|()| {
                    self.host()
                        .charge_attempted(ImportDimension::TotalAssetBytes, amount)
                })
        };
        result.map_err(|cause| self.scope_cause(cause.into_scope_abort_cause()))
    }

    fn scope_cause(&self, cause: ScopeAbortCause) -> ImportCause {
        ImportCause::Scope(ScopeAbort::new(cause))
    }
}

struct LiveReportStaging {
    detection: AccountedVec<DetectionEvidence>,
    diagnostics: DiagnosticSink,
    optional_dependency_evidence: AccountedVec<crate::OptionalDependencyEvidence>,
    loss_evidence: AccountedVec<ImportLossEvidence>,
    source_scope_record: Option<LiveSourceScopeRecord>,
}

impl LiveReportStaging {
    fn into_terminal(self) -> TerminalReportEvidence {
        let detection = self.detection;
        let diagnostics = self.diagnostics.into_terminal();
        let optional_dependency_evidence = self.optional_dependency_evidence;
        let loss_evidence = self.loss_evidence;
        let source_scope_record = self
            .source_scope_record
            .map(LiveSourceScopeRecord::into_terminal);
        let (diagnostics, diagnostic_summary) = diagnostics.into_parts();
        TerminalReportEvidence {
            detection,
            diagnostics,
            diagnostic_summary,
            optional_dependency_evidence,
            loss_evidence,
            source_scope_record,
        }
    }
}

struct TerminalReportEvidence {
    detection: AccountedVec<DetectionEvidence>,
    diagnostics: crate::diagnostic::RetainedDiagnostics,
    diagnostic_summary: crate::DiagnosticSummary,
    optional_dependency_evidence: AccountedVec<crate::OptionalDependencyEvidence>,
    loss_evidence: AccountedVec<ImportLossEvidence>,
    source_scope_record: Option<SourceScopeRecord>,
}

fn operation_terminal_cause(evidence: &ImportStoreTerminalEvidence) -> ImportCause {
    match ImportTerminalCause::from_store(evidence.cause()) {
        ImportTerminalCause::Failure(failure) => ImportCause::OperationTerminal(failure),
        ImportTerminalCause::Success => {
            unreachable!("a failure owner cannot retain a successful terminal cause")
        }
    }
}

fn select_terminal_failure_cause(
    mut original: TerminalFailureCause,
    evidence: &ImportStoreTerminalEvidence,
    control_cause: Option<ControlCause>,
    cause_authority: ImportStoreFailureCauseAuthority,
) -> TerminalFailureCause {
    let validation_error = match cause_authority {
        ImportStoreFailureCauseAuthority::SceneValidation(detail) => Some(detail),
        _ => original.context.cause().scene_validation_error().copied(),
    };
    if let Some(detail) = validation_error {
        original
            .context
            .replace_cause(ImportCause::SceneValidation(detail));
        original.terminal_control_cause = control_cause;
        return original;
    }
    if let Some(cause) = control_cause {
        original
            .context
            .replace_cause(ImportCause::Scope(ScopeAbort::new(
                ScopeAbortCause::Control(cause),
            )));
        return original;
    }
    match cause_authority {
        ImportStoreFailureCauseAuthority::OriginalImport => {}
        ImportStoreFailureCauseAuthority::OperationTerminal => original
            .context
            .replace_cause(operation_terminal_cause(evidence)),
        ImportStoreFailureCauseAuthority::SceneValidation(detail) => {
            original
                .context
                .replace_cause(ImportCause::SceneValidation(detail));
        }
    }
    original
}

fn terminal_failure(
    cause: TerminalFailureCause,
    source: AssetPath,
    selected_format: Option<FormatId>,
    report: TerminalReportEvidence,
    ledger: crate::ImportLedgerSnapshot,
    store_owner: ImportFailureStoreOwner,
    last_completed: ImportPhaseEvidence,
) -> ImportFailure {
    let TerminalFailureCause {
        context: cause,
        terminal_control_cause,
    } = cause;
    let TerminalReportEvidence {
        detection,
        diagnostics,
        diagnostic_summary,
        optional_dependency_evidence,
        loss_evidence,
        source_scope_record,
    } = report;
    ImportFailure::from_parts(ImportFailureParts {
        cause,
        terminal_control_cause,
        source,
        selected_format,
        detection,
        diagnostics,
        diagnostic_summary,
        optional_dependency_evidence,
        loss_evidence,
        source_scope_record,
        ledger,
        store_owner,
        last_completed,
    })
}

struct TerminalFailureCause {
    context: AccountedErrorContext,
    terminal_control_cause: Option<ControlCause>,
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
    host: crate::store::ImportStoreHost<'_>,
    phase: ImportLedgerPhase,
    dimension: ImportDimension,
    bytes: u64,
) -> Result<Option<ImportStoreReservation>, ScopeAbortCause> {
    if bytes == 0 {
        return Ok(None);
    }
    host.reserve(phase, dimension, bytes)
        .map(Some)
        .map_err(crate::store::ImportStoreHostError::into_scope_abort_cause)
}

fn reserve_required_bytes(
    host: crate::store::ImportStoreHost<'_>,
    phase: ImportLedgerPhase,
    dimension: ImportDimension,
    bytes: u64,
) -> Result<ImportStoreReservation, ScopeAbortCause> {
    if bytes == 0 {
        return Err(ScopeAbortCause::DescriptorContract {
            code: "zero-byte-required-reservation",
        });
    }
    host.reserve(phase, dimension, bytes)
        .map_err(crate::store::ImportStoreHostError::into_scope_abort_cause)
}

fn reserve_retained_bytes(
    host: crate::store::ImportStoreHost<'_>,
    phase: ImportLedgerPhase,
    dimension: ImportDimension,
    bytes: u64,
) -> Result<ImportStoreReservation, ScopeAbortCause> {
    host.reserve_retained(phase, dimension, bytes, u64::from(bytes != 0))
        .map_err(|cause| cause.into_scope_abort_cause())
}

#[cfg(test)]
mod retained_output_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::ImportInitializationError;
    use crate::{
        CapabilityStatus, FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter,
        ImportConfigOverlay, ImportRequest, ImportRequestLayer, OptionCompileLimits, ParseResult,
        ParseScope, RegistryBuildLimits, RegistryBuilder, ResourceLimitsOverlay,
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

        fn open_work_bound(
            &self,
            _path: &AssetPath,
        ) -> Result<std::num::NonZeroU64, baozi_io::AssetOpenWorkError> {
            panic!("initialization failure must precede backend work planning")
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
        initialization_test_request_with_limits(ResourceLimits::default())
    }

    fn initialization_test_request_with_limits(limits: ResourceLimits) -> PreparedImportRequest {
        let mut registry = RegistryBuilder::new();
        registry
            .register(NeverInvokedImporter)
            .expect("test importer registration");
        registry
            .freeze(RegistryBuildLimits::default())
            .expect("test registry freeze")
            .compile_request(
                &ImportRequest::from_layers(
                    ImportRequestLayer::default(),
                    ImportRequestLayer::new(
                        ImportConfigOverlay::default()
                            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits)),
                    ),
                ),
                OptionCompileLimits::default(),
            )
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
                        ResolverBinding::custom(&DenyDependencies),
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
                ResolverBinding::custom(&DenyDependencies),
                control,
                Err(injected.clone()),
            )
        }));
        let error = outcome
            .expect("ledger initialization failure must not unwind")
            .expect_err("injected ledger initialization must fail");

        assert!(matches!(
            &error,
            ImportRunError::Initialization {
                source: ImportInitializationError::LedgerLimits { source }
            } if source == &injected
        ));
        assert!(error.operation_failure().is_none());
        let initialization_source = Error::source(&error)
            .and_then(|source| source.downcast_ref::<ImportInitializationError>())
            .expect("run error must retain the typed initialization source");
        let limits_source = Error::source(initialization_source)
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
    fn direct_primary_binding_rejects_payload_before_copying_when_live_budget_is_too_small() {
        const LIVE_LIMIT: u64 = 1024 * 1024;
        let limits = ResourceLimits {
            max_live_bytes: LIVE_LIMIT,
            ..ResourceLimits::default()
        };
        let request = initialization_test_request_with_limits(limits);
        let io = DirectBytesAuthority::new();
        let store = try_start_store(
            &request,
            ImportOperationControl::default(),
            import_ledger_limits(request.limits()),
        )
        .expect("the retained import store must start");
        let mut session = ImportSession::new(
            &request,
            &io,
            AssetPath::new("asset.direct-reservation").expect("valid test path"),
            ResolverBinding::custom(&DenyDependencies),
            store,
        );
        let payload =
            vec![0; usize::try_from(LIVE_LIMIT + 1).expect("the test payload length fits usize")];

        assert!(
            session
                .bind_primary_bytes(&payload, SessionCreatedProof::new())
                .is_err(),
            "the payload allocation must be admitted before copying"
        );
        assert!(session.primary_snapshot.is_none());
        assert_eq!(session.last_completed.phase(), ImportPhase::SessionCreated);
    }
}
