use crate::error::ScopeAbortCause;
use crate::format::discard_untrusted_panic_payload;
use crate::ledger::{ImportProgressPhase, ImportReservation};
use crate::metered_storage::{AccountedReservationVec, AccountedVec};
use baozi_io::{AssetPath, AuthorityDomain};
use baozi_operation::{CapacityPlan, CapacityPlanError};
use std::error::Error;
use std::fmt::{self, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};

const MAX_DIAGNOSTIC_DETAILS: usize = 32;
const MAX_DIAGNOSTIC_DETAIL_BYTES: u64 = 4 * 1024;
const MAX_DIAGNOSTIC_DETAIL_STRING_BYTES: usize = 512;
const MAX_DIAGNOSTIC_OBJECT_PATH_BYTES: usize = 512;

/// A session-bound source capability used for authoritative byte locations.
pub struct DiagnosticSource<'source> {
    authority: AuthorityDomain,
    path: &'source AssetPath,
    bytes: &'source [u8],
}

impl<'source> DiagnosticSource<'source> {
    pub(crate) fn new(
        authority: AuthorityDomain,
        path: &'source AssetPath,
        bytes: &'source [u8],
    ) -> Self {
        Self {
            authority,
            path,
            bytes,
        }
    }

    pub fn path(&self) -> &AssetPath {
        self.path
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    pub(crate) fn authority(&self) -> &AuthorityDomain {
        &self.authority
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiagnosticCode(&'static str);

impl DiagnosticCode {
    pub fn new(value: &'static str) -> Result<Self, DiagnosticCause> {
        if !is_valid_code(value) {
            return Err(DiagnosticCause::InvalidCode);
        }
        Ok(Self(value))
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

fn is_valid_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiagnosticByteRange {
    start: u64,
    end: u64,
}

impl DiagnosticByteRange {
    pub const fn new(start: u64, end: u64) -> Result<Self, DiagnosticCause> {
        if start > end {
            return Err(DiagnosticCause::InvalidByteRange { start, end });
        }
        Ok(Self { start, end })
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn end(self) -> u64 {
        self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiagnosticLineColumn {
    line: u64,
    column: u64,
}

impl DiagnosticLineColumn {
    pub const fn new(line: u64, column: u64) -> Result<Self, DiagnosticCause> {
        if line == 0 || column == 0 {
            return Err(DiagnosticCause::InvalidLineColumn { line, column });
        }
        Ok(Self { line, column })
    }

    pub const fn line(self) -> u64 {
        self.line
    }

    pub const fn column(self) -> u64 {
        self.column
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticDetailValue {
    String(String),
    Unsigned(u64),
    Signed(i64),
    Boolean(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticDetail {
    key: &'static str,
    value: DiagnosticDetailValue,
}

impl DiagnosticDetail {
    pub const fn key(&self) -> &'static str {
        self.key
    }

    pub const fn value(&self) -> &DiagnosticDetailValue {
        &self.value
    }
}

pub trait DiagnosticDetailSink {
    fn string(&mut self, key: &'static str, value: &str);
    fn unsigned(&mut self, key: &'static str, value: u64);
    fn signed(&mut self, key: &'static str, value: i64);
    fn boolean(&mut self, key: &'static str, value: bool);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDiagnostic {
    severity: DiagnosticSeverity,
    code: DiagnosticCode,
    phase: ImportProgressPhase,
    source: AssetPath,
    byte_range: Option<DiagnosticByteRange>,
    line_column: Option<DiagnosticLineColumn>,
    object_path: Option<String>,
    message: String,
    truncated: bool,
    details: Vec<DiagnosticDetail>,
}

impl ImportDiagnostic {
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    pub const fn phase(&self) -> ImportProgressPhase {
        self.phase
    }

    pub fn source(&self) -> &AssetPath {
        &self.source
    }

    pub const fn byte_range(&self) -> Option<DiagnosticByteRange> {
        self.byte_range
    }

    pub const fn line_column(&self) -> Option<DiagnosticLineColumn> {
        self.line_column
    }

    pub fn object_path(&self) -> Option<&str> {
        self.object_path.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }

    pub fn details(&self) -> &[DiagnosticDetail] {
        &self.details
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiagnosticSummary {
    attempted_count: u64,
    stored_count: u64,
    dropped_count: u64,
    attempted_bytes: u64,
    stored_bytes: u64,
}

impl DiagnosticSummary {
    pub const fn attempted_count(&self) -> u64 {
        self.attempted_count
    }

    pub const fn stored_count(&self) -> u64 {
        self.stored_count
    }

    pub const fn dropped_count(&self) -> u64 {
        self.dropped_count
    }

    pub const fn attempted_bytes(&self) -> u64 {
        self.attempted_bytes
    }

    pub const fn stored_bytes(&self) -> u64 {
        self.stored_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticCause {
    InvalidCode,
    InvalidByteRange { start: u64, end: u64 },
    ByteRangeOutOfBounds { end: u64, source_len: u64 },
    LocationSourceUnavailable,
    InvalidLineColumn { line: u64, column: u64 },
    InvalidObjectPath,
    InvalidDetailKey,
    DetailCountExceeded,
    DetailBytesExceeded,
    DetailStringExceeded,
    MessageFormattingFailed,
    AttemptBudgetExceeded { limit: u64 },
    CapacityExceeded,
    Promoted { code: &'static str },
}

impl fmt::Display for DiagnosticCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCode => formatter.write_str("diagnostic code is not canonical"),
            Self::InvalidByteRange { start, end } => {
                write!(formatter, "diagnostic byte range {start}..{end} is invalid")
            }
            Self::ByteRangeOutOfBounds { end, source_len } => write!(
                formatter,
                "diagnostic byte-range end {end} exceeds source length {source_len}"
            ),
            Self::LocationSourceUnavailable => {
                formatter.write_str("diagnostic location source is unavailable in this session")
            }
            Self::InvalidLineColumn { line, column } => {
                write!(
                    formatter,
                    "diagnostic line {line} column {column} is invalid"
                )
            }
            Self::InvalidObjectPath => formatter.write_str("diagnostic object path is invalid"),
            Self::InvalidDetailKey => formatter.write_str("diagnostic detail key is invalid"),
            Self::DetailCountExceeded => formatter.write_str("diagnostic detail count exceeded"),
            Self::DetailBytesExceeded => formatter.write_str("diagnostic detail bytes exceeded"),
            Self::DetailStringExceeded => formatter.write_str("diagnostic detail string exceeded"),
            Self::MessageFormattingFailed => {
                formatter.write_str("diagnostic message formatting failed")
            }
            Self::AttemptBudgetExceeded { limit } => {
                write!(
                    formatter,
                    "diagnostic attempted-byte budget {limit} exceeded"
                )
            }
            Self::CapacityExceeded => {
                formatter.write_str("diagnostic storage capacity could not be reserved")
            }
            Self::Promoted { code } => {
                write!(formatter, "diagnostic {code} was promoted by strict policy")
            }
        }
    }
}

impl Error for DiagnosticCause {}

#[derive(Debug, Clone, Copy)]
struct DiagnosticCapacityPlans {
    details: CapacityPlan,
    source: CapacityPlan,
    object_path: Option<CapacityPlan>,
    message: CapacityPlan,
}

impl DiagnosticCapacityPlans {
    fn new(
        source_bytes: usize,
        object_path_bytes: Option<usize>,
        message_bytes: usize,
    ) -> Result<Self, CapacityPlanError> {
        Ok(Self {
            details: CapacityPlan::for_type::<DiagnosticDetail>(0, 0, MAX_DIAGNOSTIC_DETAILS)?,
            source: CapacityPlan::for_type::<u8>(0, 0, source_bytes)?,
            object_path: object_path_bytes
                .map(|bytes| CapacityPlan::for_type::<u8>(0, 0, bytes))
                .transpose()?,
            message: CapacityPlan::for_type::<u8>(0, 0, message_bytes)?,
        })
    }

    fn payload_envelope_bytes(self) -> Result<u64, CapacityPlanError> {
        let source = self.source.envelope_delta_bytes()?;
        let object_path = match self.object_path {
            Some(plan) => plan.envelope_delta_bytes()?,
            None => 0,
        };
        let message = self.message.envelope_delta_bytes()?;
        let detail_string = CapacityPlan::for_type::<u8>(0, 0, MAX_DIAGNOSTIC_DETAIL_STRING_BYTES)?
            .envelope_delta_bytes()?
            .checked_mul(MAX_DIAGNOSTIC_DETAILS as u64)
            .ok_or(CapacityPlanError::CapacityOverflow)?;
        source
            .checked_add(object_path)
            .and_then(|bytes| bytes.checked_add(message))
            .and_then(|bytes| bytes.checked_add(detail_string))
            .ok_or(CapacityPlanError::CapacityOverflow)
    }

    fn structural_envelope_bytes(self) -> Result<u64, CapacityPlanError> {
        self.details.envelope_delta_bytes()
    }
}

#[derive(Debug)]
pub(crate) struct DiagnosticAdmission {
    fixed_attempted_bytes: u64,
    attempted_remaining: u64,
    storage_reservation_bytes: u64,
    payload_capacity_envelope: u64,
    structural_capacity_envelope: u64,
    capacity_plans: Option<DiagnosticCapacityPlans>,
    may_store: bool,
    failure: Option<DiagnosticCause>,
}

impl DiagnosticAdmission {
    pub(crate) const fn fixed_attempted_bytes(&self) -> u64 {
        self.fixed_attempted_bytes
    }

    pub(crate) const fn storage_reservation_bytes(&self) -> u64 {
        self.storage_reservation_bytes
    }

    pub(crate) const fn payload_capacity_envelope(&self) -> u64 {
        self.payload_capacity_envelope
    }

    pub(crate) const fn structural_capacity_envelope(&self) -> u64 {
        self.structural_capacity_envelope
    }

    pub(crate) const fn failure(&self) -> Option<&DiagnosticCause> {
        self.failure.as_ref()
    }
}

#[derive(Debug)]
pub(crate) enum DiagnosticPrepareCause {
    Diagnostic(DiagnosticCause),
    Panicked,
}

#[derive(Debug)]
pub(crate) struct DiagnosticPrepareFailure {
    pub cause: DiagnosticPrepareCause,
    pub additional_attempted_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct PreparedDiagnostic {
    diagnostic: Option<ImportDiagnostic>,
    additional_attempted_bytes: u64,
    stored_bytes: u64,
    payload_capacity_bytes: u64,
    structural_bytes: u64,
}

impl PreparedDiagnostic {
    pub(crate) const fn additional_attempted_bytes(&self) -> u64 {
        self.additional_attempted_bytes
    }

    pub(crate) const fn stored_bytes(&self) -> u64 {
        self.stored_bytes
    }

    pub(crate) const fn is_stored(&self) -> bool {
        self.diagnostic.is_some()
    }

    pub(crate) const fn payload_capacity_bytes(&self) -> u64 {
        self.payload_capacity_bytes
    }

    pub(crate) const fn structural_bytes(&self) -> u64 {
        self.structural_bytes
    }
}

pub(crate) struct DiagnosticSink {
    diagnostics: AccountedVec<ImportDiagnostic>,
    reservations: AccountedReservationVec,
    summary: DiagnosticSummary,
    max_count: u64,
    max_stored_bytes: u64,
    max_message_bytes: usize,
    max_attempted_bytes: u64,
}

impl DiagnosticSink {
    pub(crate) fn new(
        max_count: u64,
        max_stored_bytes: u64,
        max_message_bytes: usize,
        max_attempted_bytes: u64,
    ) -> Self {
        Self {
            diagnostics: AccountedVec::new(),
            reservations: AccountedReservationVec::new(),
            summary: DiagnosticSummary::default(),
            max_count,
            max_stored_bytes,
            max_message_bytes,
            max_attempted_bytes,
        }
    }

    pub(crate) fn begin(
        &mut self,
        code: DiagnosticCode,
        source: &AssetPath,
        byte_range: Option<DiagnosticByteRange>,
        line_column: Option<DiagnosticLineColumn>,
        object_path: Option<&str>,
    ) -> DiagnosticAdmission {
        let prior_attempted = self.summary.attempted_bytes;
        let object_path_result = validate_object_path(object_path);
        let object_path_bytes = object_path.map_or(0, |path| path.len() as u64);
        let fixed_attempted_bytes = (code.as_str().len() as u64)
            .saturating_add(source.as_str().len() as u64)
            .saturating_add(u64::from(byte_range.is_some()) * 16)
            .saturating_add(u64::from(line_column.is_some()) * 16)
            .saturating_add(object_path_bytes)
            .saturating_add(1);
        self.summary.attempted_count = self.summary.attempted_count.saturating_add(1);
        self.summary.attempted_bytes = prior_attempted.saturating_add(fixed_attempted_bytes);

        let mut failure = object_path_result.err().or_else(|| {
            (self.summary.attempted_bytes > self.max_attempted_bytes).then_some(
                DiagnosticCause::AttemptBudgetExceeded {
                    limit: self.max_attempted_bytes,
                },
            )
        });
        let stored_remaining = self
            .max_stored_bytes
            .saturating_sub(self.summary.stored_bytes);
        let semantically_storable = failure.is_none()
            && self.summary.stored_count < self.max_count
            && fixed_attempted_bytes <= stored_remaining;
        let maximum_extra =
            MAX_DIAGNOSTIC_DETAIL_BYTES.saturating_add(self.max_message_bytes as u64);
        let candidate_storage_bytes = if semantically_storable {
            fixed_attempted_bytes.saturating_add(
                stored_remaining
                    .saturating_sub(fixed_attempted_bytes)
                    .min(maximum_extra),
            )
        } else {
            0
        };
        let capacity = semantically_storable.then(|| {
            let message_bytes = candidate_storage_bytes
                .saturating_sub(fixed_attempted_bytes)
                .min(self.max_message_bytes as u64) as usize;
            let plans = DiagnosticCapacityPlans::new(
                source.as_str().len(),
                object_path.map(str::len),
                message_bytes,
            )?;
            Ok::<_, CapacityPlanError>((
                plans,
                plans.payload_envelope_bytes()?,
                plans.structural_envelope_bytes()?,
            ))
        });
        let (capacity_plans, payload_capacity_envelope, structural_capacity_envelope) =
            match capacity.transpose() {
                Ok(Some((plans, payload, structural))) => (Some(plans), payload, structural),
                Ok(None) => (None, 0, 0),
                Err(_) => {
                    failure = Some(DiagnosticCause::CapacityExceeded);
                    (None, 0, 0)
                }
            };
        let may_store = semantically_storable && capacity_plans.is_some();
        let storage_reservation_bytes = if may_store {
            candidate_storage_bytes
        } else {
            0
        };
        DiagnosticAdmission {
            fixed_attempted_bytes,
            attempted_remaining: self
                .max_attempted_bytes
                .saturating_sub(self.summary.attempted_bytes),
            storage_reservation_bytes,
            payload_capacity_envelope,
            structural_capacity_envelope,
            capacity_plans,
            may_store,
            failure,
        }
    }

    pub(crate) fn discard_admission(&mut self) {
        self.summary.dropped_count = self.summary.dropped_count.saturating_add(1);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        admission: DiagnosticAdmission,
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        phase: ImportProgressPhase,
        source: &AssetPath,
        byte_range: Option<DiagnosticByteRange>,
        line_column: Option<DiagnosticLineColumn>,
        object_path: Option<&str>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> Result<PreparedDiagnostic, DiagnosticPrepareFailure> {
        if let Some(cause) = admission.failure {
            self.discard_admission();
            return Err(DiagnosticPrepareFailure {
                cause: DiagnosticPrepareCause::Diagnostic(cause),
                additional_attempted_bytes: 0,
            });
        }
        let capacity_plans = admission.capacity_plans;

        let detail_storage_limit = admission
            .storage_reservation_bytes
            .saturating_sub(admission.fixed_attempted_bytes);
        let mut detail_sink = match BoundedDiagnosticDetailSink::new(
            admission.may_store,
            detail_storage_limit,
            admission.attempted_remaining,
            self.max_attempted_bytes,
            capacity_plans.map(|plans| plans.details),
        ) {
            Ok(sink) => sink,
            Err(cause) => {
                self.record_failed_prepare(0);
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Diagnostic(cause),
                    additional_attempted_bytes: 0,
                });
            }
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| details(&mut detail_sink))) {
            discard_untrusted_panic_payload(payload);
            let attempted = detail_sink.attempted_bytes;
            self.record_failed_prepare(attempted);
            return Err(DiagnosticPrepareFailure {
                cause: DiagnosticPrepareCause::Panicked,
                additional_attempted_bytes: attempted,
            });
        }
        let detail_build = match detail_sink.finish() {
            Ok(build) => build,
            Err(failure) => {
                self.record_failed_prepare(failure.attempted_bytes);
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Diagnostic(failure.cause),
                    additional_attempted_bytes: failure.attempted_bytes,
                });
            }
        };

        let message_store_limit = if admission.may_store && detail_build.storage_fits {
            admission
                .storage_reservation_bytes
                .saturating_sub(admission.fixed_attempted_bytes)
                .saturating_sub(detail_build.encoded_bytes)
                .min(self.max_message_bytes as u64) as usize
        } else {
            0
        };
        let message_attempted_limit = admission
            .attempted_remaining
            .saturating_sub(detail_build.attempted_bytes);
        let mut writer =
            match BoundedMessageWriter::new(message_store_limit, message_attempted_limit) {
                Ok(writer) => writer,
                Err(cause) => {
                    self.record_failed_prepare(detail_build.attempted_bytes);
                    return Err(DiagnosticPrepareFailure {
                        cause: DiagnosticPrepareCause::Diagnostic(cause),
                        additional_attempted_bytes: detail_build.attempted_bytes,
                    });
                }
            };
        let write_result = catch_unwind(AssertUnwindSafe(|| writer.write_fmt(message)));
        let message_attempted_bytes = writer.attempted_bytes;
        let additional_attempted_bytes = detail_build
            .attempted_bytes
            .saturating_add(message_attempted_bytes);
        match write_result {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                self.record_failed_prepare(additional_attempted_bytes);
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Panicked,
                    additional_attempted_bytes,
                });
            }
            Ok(Err(_)) if writer.attempt_limit_exceeded => {
                self.record_failed_prepare(additional_attempted_bytes);
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Diagnostic(
                        DiagnosticCause::AttemptBudgetExceeded {
                            limit: self.max_attempted_bytes,
                        },
                    ),
                    additional_attempted_bytes,
                });
            }
            Ok(Err(_)) => {
                self.record_failed_prepare(additional_attempted_bytes);
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Diagnostic(
                        DiagnosticCause::MessageFormattingFailed,
                    ),
                    additional_attempted_bytes,
                });
            }
            Ok(Ok(())) => {}
        }
        let message = writer.finish();
        self.summary.attempted_bytes = self
            .summary
            .attempted_bytes
            .saturating_add(additional_attempted_bytes);

        if !admission.may_store || !detail_build.storage_fits {
            self.discard_admission();
            return Ok(PreparedDiagnostic {
                diagnostic: None,
                additional_attempted_bytes,
                stored_bytes: 0,
                payload_capacity_bytes: 0,
                structural_bytes: 0,
            });
        }

        let (object_path, object_path_capacity_bytes) = match copy_object_path(object_path) {
            Ok(path) => path,
            Err(cause) => {
                self.discard_admission();
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Diagnostic(cause),
                    additional_attempted_bytes,
                });
            }
        };
        let stored_bytes = admission
            .fixed_attempted_bytes
            .saturating_add(detail_build.encoded_bytes)
            .saturating_add(message.text.len() as u64);
        let source = match source.try_clone() {
            Ok(source) => source,
            Err(_) => {
                self.discard_admission();
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Diagnostic(DiagnosticCause::CapacityExceeded),
                    additional_attempted_bytes,
                });
            }
        };
        let Some(capacity_plans) = capacity_plans else {
            self.discard_admission();
            return Err(DiagnosticPrepareFailure {
                cause: DiagnosticPrepareCause::Diagnostic(DiagnosticCause::CapacityExceeded),
                additional_attempted_bytes,
            });
        };
        let source_capacity_bytes = match capacity_plans.source.finish(0, source.capacity_bytes()) {
            Ok(bytes) => bytes,
            Err(_) => {
                self.discard_admission();
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Diagnostic(DiagnosticCause::CapacityExceeded),
                    additional_attempted_bytes,
                });
            }
        };
        if capacity_plans
            .object_path
            .map(|plan| plan.finish(0, object_path_capacity_bytes as usize))
            .transpose()
            .is_err()
        {
            self.discard_admission();
            return Err(DiagnosticPrepareFailure {
                cause: DiagnosticPrepareCause::Diagnostic(DiagnosticCause::CapacityExceeded),
                additional_attempted_bytes,
            });
        }
        let payload_capacity_bytes = match source_capacity_bytes
            .checked_add(object_path_capacity_bytes)
            .and_then(|bytes| bytes.checked_add(message.capacity_bytes))
            .and_then(|bytes| bytes.checked_add(detail_build.string_capacity_bytes))
        {
            Some(bytes) if bytes <= admission.payload_capacity_envelope => bytes,
            Some(_) | None => {
                self.discard_admission();
                return Err(DiagnosticPrepareFailure {
                    cause: DiagnosticPrepareCause::Diagnostic(DiagnosticCause::CapacityExceeded),
                    additional_attempted_bytes,
                });
            }
        };
        let structural_bytes = detail_build.detail_capacity_bytes;
        debug_assert!(structural_bytes <= admission.structural_capacity_envelope);
        Ok(PreparedDiagnostic {
            diagnostic: Some(ImportDiagnostic {
                severity,
                code,
                phase,
                source,
                byte_range,
                line_column,
                object_path,
                message: message.text,
                truncated: message.truncated,
                details: detail_build.details,
            }),
            additional_attempted_bytes,
            stored_bytes,
            payload_capacity_bytes,
            structural_bytes,
        })
    }

    pub(crate) fn discard_prepared(&mut self, _prepared: PreparedDiagnostic) {
        self.discard_admission();
    }

    pub(crate) fn try_reserve_storage_slot(
        &mut self,
        reserve: impl FnOnce(u64) -> Result<ImportReservation, ScopeAbortCause>,
    ) -> Result<(), ScopeAbortCause> {
        self.diagnostics
            .try_reserve(1, "diagnostic-storage", reserve)
    }

    pub(crate) fn try_reserve_reservation_slots(
        &mut self,
        additional: usize,
        reserve: impl FnOnce(u64) -> Result<ImportReservation, ScopeAbortCause>,
    ) -> Result<(), ScopeAbortCause> {
        self.reservations
            .try_reserve(additional, "diagnostic-reservation-list", reserve)
    }

    #[allow(
        clippy::result_large_err,
        reason = "rejection must return the prepared diagnostic and both live reservations without allocating"
    )]
    pub(crate) fn commit(
        &mut self,
        mut prepared: PreparedDiagnostic,
        payload: Option<ImportReservation>,
        structural: Option<ImportReservation>,
    ) -> Result<
        (),
        (
            PreparedDiagnostic,
            Option<ImportReservation>,
            Option<ImportReservation>,
        ),
    > {
        let Some(diagnostic) = prepared.diagnostic.take() else {
            return if payload.is_none() && structural.is_none() {
                Ok(())
            } else {
                Err((prepared, payload, structural))
            };
        };
        let (payload, structural) = match (payload, structural) {
            (Some(payload), Some(structural)) => (payload, structural),
            (payload, structural) => {
                prepared.diagnostic = Some(diagnostic);
                return Err((prepared, payload, structural));
            }
        };
        if !self.diagnostics.has_slots(1) || !self.reservations.has_slots(2) {
            prepared.diagnostic = Some(diagnostic);
            return Err((prepared, Some(payload), Some(structural)));
        }
        if let Err(diagnostic) = self.diagnostics.try_push_reserved(diagnostic) {
            prepared.diagnostic = Some(diagnostic);
            return Err((prepared, Some(payload), Some(structural)));
        }
        if let Err((payload, structural)) = self
            .reservations
            .try_push_pair_reserved(payload, structural)
        {
            prepared.diagnostic = self.diagnostics.pop();
            return Err((prepared, Some(payload), Some(structural)));
        }
        self.summary.stored_count = self.summary.stored_count.saturating_add(1);
        self.summary.stored_bytes = self
            .summary
            .stored_bytes
            .saturating_add(prepared.stored_bytes);
        Ok(())
    }

    fn record_failed_prepare(&mut self, additional_attempted_bytes: u64) {
        self.summary.attempted_bytes = self
            .summary
            .attempted_bytes
            .saturating_add(additional_attempted_bytes);
        self.discard_admission();
    }

    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }

    #[cfg(test)]
    pub(crate) const fn summary(&self) -> DiagnosticSummary {
        self.summary
    }

    pub(crate) fn publication_reservation_count(&self) -> Option<usize> {
        self.diagnostics
            .capacity_reservation_count()
            .checked_add(self.reservations.publication_reservation_count()?)
    }

    pub(crate) fn move_publication_reservations_into(
        &mut self,
        target: &mut AccountedReservationVec,
    ) {
        let additional = self
            .publication_reservation_count()
            .expect("diagnostic reservation count must fit usize");
        assert!(
            target.has_slots(additional),
            "diagnostic publication slots must be admitted before moving tokens"
        );
        self.reservations.move_publication_reservations_into(target);
        self.diagnostics.move_capacity_reservation_into(target);
    }

    pub(crate) fn into_terminal(self) -> TerminalDiagnosticEvidence {
        let Self {
            diagnostics,
            reservations,
            summary,
            max_count: _,
            max_stored_bytes: _,
            max_message_bytes: _,
            max_attempted_bytes: _,
        } = self;
        let diagnostics = diagnostics.into_terminal();
        let terminal = TerminalDiagnosticEvidence {
            diagnostics: diagnostics.into_values(),
            summary,
        };
        drop(reservations);
        terminal
    }
}

pub(crate) struct TerminalDiagnosticEvidence {
    diagnostics: Vec<ImportDiagnostic>,
    summary: DiagnosticSummary,
}

impl TerminalDiagnosticEvidence {
    pub(crate) fn into_parts(self) -> (Vec<ImportDiagnostic>, DiagnosticSummary) {
        (self.diagnostics, self.summary)
    }
}

fn validate_object_path(value: Option<&str>) -> Result<(), DiagnosticCause> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.len() < 2
        || value.len() > MAX_DIAGNOSTIC_OBJECT_PATH_BYTES
        || !value.starts_with('/')
        || value.chars().any(char::is_control)
    {
        return Err(DiagnosticCause::InvalidObjectPath);
    }
    Ok(())
}

fn apply_vec_capacity_plan<T>(
    values: &mut Vec<T>,
    plan: CapacityPlan,
) -> Result<u64, DiagnosticCause> {
    plan.verify_preflight(values.len(), values.capacity())
        .map_err(|_| DiagnosticCause::CapacityExceeded)?;
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| DiagnosticCause::CapacityExceeded)?;
    plan.finish(values.len(), values.capacity())
        .map_err(|_| DiagnosticCause::CapacityExceeded)
}

fn allocate_string(desired_capacity: usize) -> Result<(String, u64), DiagnosticCause> {
    let plan = CapacityPlan::for_type::<u8>(0, 0, desired_capacity)
        .map_err(|_| DiagnosticCause::CapacityExceeded)?;
    let mut value = String::new();
    plan.verify_preflight(value.len(), value.capacity())
        .map_err(|_| DiagnosticCause::CapacityExceeded)?;
    value
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| DiagnosticCause::CapacityExceeded)?;
    let actual = plan
        .finish(value.len(), value.capacity())
        .map_err(|_| DiagnosticCause::CapacityExceeded)?;
    Ok((value, actual))
}

fn copy_object_path(value: Option<&str>) -> Result<(Option<String>, u64), DiagnosticCause> {
    let Some(value) = value else {
        return Ok((None, 0));
    };
    let (mut owned, capacity_bytes) = allocate_string(value.len())?;
    owned.push_str(value);
    Ok((Some(owned), capacity_bytes))
}

struct DetailBuild {
    details: Vec<DiagnosticDetail>,
    attempted_bytes: u64,
    encoded_bytes: u64,
    detail_capacity_bytes: u64,
    string_capacity_bytes: u64,
    storage_fits: bool,
}

struct DetailFailure {
    cause: DiagnosticCause,
    attempted_bytes: u64,
}

struct BoundedDiagnosticDetailSink {
    details: Vec<DiagnosticDetail>,
    count: usize,
    attempted_bytes: u64,
    encoded_bytes: u64,
    attempted_limit: u64,
    storage_limit: u64,
    storage_fits: bool,
    detail_capacity_bytes: u64,
    string_capacity_bytes: u64,
    cause: Option<DiagnosticCause>,
    global_attempted_limit: u64,
}

impl BoundedDiagnosticDetailSink {
    fn new(
        store: bool,
        storage_limit: u64,
        attempted_limit: u64,
        global_attempted_limit: u64,
        capacity_plan: Option<CapacityPlan>,
    ) -> Result<Self, DiagnosticCause> {
        let mut details = Vec::new();
        let detail_capacity_bytes = match capacity_plan {
            Some(plan) if store => apply_vec_capacity_plan(&mut details, plan)?,
            Some(_) | None => 0,
        };
        Ok(Self {
            details,
            count: 0,
            attempted_bytes: 0,
            encoded_bytes: 0,
            attempted_limit,
            storage_limit,
            storage_fits: store,
            detail_capacity_bytes,
            string_capacity_bytes: 0,
            cause: None,
            global_attempted_limit,
        })
    }

    fn finish(self) -> Result<DetailBuild, DetailFailure> {
        match self.cause {
            Some(cause) => Err(DetailFailure {
                cause,
                attempted_bytes: self.attempted_bytes,
            }),
            None => Ok(DetailBuild {
                details: self.details,
                attempted_bytes: self.attempted_bytes,
                encoded_bytes: self.encoded_bytes,
                detail_capacity_bytes: self.detail_capacity_bytes,
                string_capacity_bytes: self.string_capacity_bytes,
                storage_fits: self.storage_fits,
            }),
        }
    }

    fn admit(&mut self, key: &'static str, value_bytes: u64) -> bool {
        if self.cause.is_some() {
            return false;
        }
        let item_bytes = (key.len() as u64).saturating_add(value_bytes);
        self.attempted_bytes = self.attempted_bytes.saturating_add(item_bytes);
        if self.attempted_bytes > self.attempted_limit {
            self.cause = Some(DiagnosticCause::AttemptBudgetExceeded {
                limit: self.global_attempted_limit,
            });
            return false;
        }
        if !valid_detail_key(key) {
            self.cause = Some(DiagnosticCause::InvalidDetailKey);
            return false;
        }
        if self.count >= MAX_DIAGNOSTIC_DETAILS {
            self.cause = Some(DiagnosticCause::DetailCountExceeded);
            return false;
        }
        let Some(next_bytes) = self.encoded_bytes.checked_add(item_bytes) else {
            self.cause = Some(DiagnosticCause::DetailBytesExceeded);
            return false;
        };
        if next_bytes > MAX_DIAGNOSTIC_DETAIL_BYTES {
            self.cause = Some(DiagnosticCause::DetailBytesExceeded);
            return false;
        }
        self.count += 1;
        self.encoded_bytes = next_bytes;
        if self.storage_fits && next_bytes > self.storage_limit {
            self.storage_fits = false;
            self.details.clear();
        }
        self.storage_fits
    }

    fn push_stored(&mut self, detail: DiagnosticDetail) {
        if self.details.len() == self.details.capacity() {
            self.cause = Some(DiagnosticCause::CapacityExceeded);
            return;
        }
        self.details.push(detail);
    }
}

impl DiagnosticDetailSink for BoundedDiagnosticDetailSink {
    fn string(&mut self, key: &'static str, value: &str) {
        let escaped_bytes = value.chars().fold(0_usize, |total, character| {
            total.saturating_add(if character.is_control() {
                '\u{fffd}'.len_utf8()
            } else {
                character.len_utf8()
            })
        });
        if escaped_bytes > MAX_DIAGNOSTIC_DETAIL_STRING_BYTES {
            self.cause = Some(DiagnosticCause::DetailStringExceeded);
            return;
        }
        if !self.admit(key, escaped_bytes as u64) {
            return;
        }
        let (mut escaped, capacity_bytes) = match allocate_string(escaped_bytes) {
            Ok(allocated) => allocated,
            Err(cause) => {
                self.cause = Some(cause);
                return;
            }
        };
        let Some(string_capacity_bytes) = self.string_capacity_bytes.checked_add(capacity_bytes)
        else {
            self.cause = Some(DiagnosticCause::CapacityExceeded);
            return;
        };
        for character in value.chars() {
            escaped.push(if character.is_control() {
                '\u{fffd}'
            } else {
                character
            });
        }
        self.push_stored(DiagnosticDetail {
            key,
            value: DiagnosticDetailValue::String(escaped),
        });
        if self.cause.is_none() {
            self.string_capacity_bytes = string_capacity_bytes;
        }
    }

    fn unsigned(&mut self, key: &'static str, value: u64) {
        if self.admit(key, 8) {
            self.push_stored(DiagnosticDetail {
                key,
                value: DiagnosticDetailValue::Unsigned(value),
            });
        }
    }

    fn signed(&mut self, key: &'static str, value: i64) {
        if self.admit(key, 8) {
            self.push_stored(DiagnosticDetail {
                key,
                value: DiagnosticDetailValue::Signed(value),
            });
        }
    }

    fn boolean(&mut self, key: &'static str, value: bool) {
        if self.admit(key, 1) {
            self.push_stored(DiagnosticDetail {
                key,
                value: DiagnosticDetailValue::Boolean(value),
            });
        }
    }
}

pub(crate) fn valid_detail_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

struct BoundedMessage {
    text: String,
    capacity_bytes: u64,
    truncated: bool,
}

struct BoundedMessageWriter {
    text: String,
    store_limit: usize,
    attempted_limit: u64,
    attempted_bytes: u64,
    capacity_bytes: u64,
    truncated: bool,
    attempt_limit_exceeded: bool,
}

impl BoundedMessageWriter {
    fn new(store_limit: usize, attempted_limit: u64) -> Result<Self, DiagnosticCause> {
        let (text, capacity_bytes) = allocate_string(store_limit)?;
        Ok(Self {
            text,
            store_limit,
            attempted_limit,
            attempted_bytes: 0,
            capacity_bytes,
            truncated: false,
            attempt_limit_exceeded: false,
        })
    }

    fn finish(mut self) -> BoundedMessage {
        if self.truncated && self.store_limit >= 3 {
            while self.text.len() > self.store_limit - 3 {
                self.text.pop();
            }
            self.text.push_str("...");
        }
        BoundedMessage {
            text: self.text,
            capacity_bytes: self.capacity_bytes,
            truncated: self.truncated,
        }
    }

    fn write_escaped(&mut self, value: &str) -> fmt::Result {
        for character in value.chars() {
            let mut escaped = [0_u8; 10];
            let encoded = match character {
                '\n' => "\\n",
                '\r' => "\\r",
                '\t' => "\\t",
                character if character.is_control() => {
                    format_control(character as u32, &mut escaped)
                }
                character => character.encode_utf8(&mut escaped),
            };
            self.attempted_bytes = self.attempted_bytes.saturating_add(encoded.len() as u64);
            if self.attempted_bytes > self.attempted_limit {
                self.attempt_limit_exceeded = true;
                return Err(fmt::Error);
            }
            if self.text.len().saturating_add(encoded.len()) <= self.store_limit {
                self.text.push_str(encoded);
            } else {
                self.truncated = true;
            }
        }
        Ok(())
    }
}

impl fmt::Write for BoundedMessageWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write_escaped(value)
    }
}

fn format_control(code: u32, output: &mut [u8; 10]) -> &str {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    output[0] = b'\\';
    output[1] = b'u';
    output[2] = b'{';
    let mut digits = [0_u8; 6];
    let mut value = code;
    let mut len = 0;
    loop {
        digits[len] = HEX[(value & 0xf) as usize];
        len += 1;
        value >>= 4;
        if value == 0 {
            break;
        }
    }
    for index in 0..len {
        output[3 + index] = digits[len - index - 1];
    }
    output[3 + len] = b'}';
    std::str::from_utf8(&output[..4 + len]).unwrap_or("\\u{fffd}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepare_and_commit(
        sink: &mut DiagnosticSink,
        source: &AssetPath,
        message: fmt::Arguments<'_>,
    ) -> PreparedDiagnostic {
        let code = DiagnosticCode::new("obj.warning").expect("valid code");
        let admission = sink.begin(code, source, None, None, None);
        sink.prepare(
            admission,
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Decode,
            source,
            None,
            None,
            None,
            |_| {},
            message,
        )
        .expect("bounded preparation")
    }

    #[test]
    fn message_is_bounded_and_controls_are_escaped_before_storage() {
        let source = AssetPath::new("model.obj").expect("valid path");
        let mut sink = DiagnosticSink::new(2, 256, 12, 1_024);
        let code = DiagnosticCode::new("obj.bad_token").expect("valid code");
        let range = DiagnosticByteRange::new(3, 7).ok();
        let location = DiagnosticLineColumn::new(2, 4).ok();
        let admission = sink.begin(code, &source, range, location, Some("/materials/0"));
        let prepared = sink
            .prepare(
                admission,
                DiagnosticSeverity::Warning,
                code,
                ImportProgressPhase::Decode,
                &source,
                range,
                location,
                Some("/materials/0"),
                |details| details.unsigned("token", 7),
                format_args!("bad\n{}", "x".repeat(64)),
            )
            .expect("bounded preparation");
        assert!(prepared.is_stored());
        let diagnostic = prepared
            .diagnostic
            .as_ref()
            .expect("stored preparation must retain a diagnostic");
        assert!(diagnostic.is_truncated());
        assert!(!diagnostic.message().contains('\n'));
    }

    #[test]
    fn full_storage_counts_drops_without_storing_messages() {
        let source = AssetPath::new("model.obj").expect("valid path");
        let mut sink = DiagnosticSink::new(0, 0, 0, 128);
        let prepared = prepare_and_commit(&mut sink, &source, format_args!("bounded"));
        assert!(!prepared.is_stored());
        sink.commit(prepared, None, None)
            .expect("non-stored preparation needs no reservations");
        assert_eq!(sink.summary().attempted_count(), 1);
        assert_eq!(sink.summary().dropped_count(), 1);
        assert!(sink.diagnostics().is_empty());
    }

    fn stored_detail_sink() -> BoundedDiagnosticDetailSink {
        let capacity_plan =
            CapacityPlan::for_type::<DiagnosticDetail>(0, 0, MAX_DIAGNOSTIC_DETAILS)
                .expect("static detail capacity must be valid");
        BoundedDiagnosticDetailSink::new(
            true,
            MAX_DIAGNOSTIC_DETAIL_BYTES,
            MAX_DIAGNOSTIC_DETAIL_BYTES,
            MAX_DIAGNOSTIC_DETAIL_BYTES,
            Some(capacity_plan),
        )
        .expect("bounded detail sink")
    }

    #[test]
    fn detail_strings_escape_controls_before_storage() {
        let mut sink = stored_detail_sink();

        sink.string("token", "bad\nvalue");

        let build = match sink.finish() {
            Ok(build) => build,
            Err(_) => panic!("valid detail string must be accepted"),
        };
        assert_eq!(
            build.details.first().expect("stored detail").value(),
            &DiagnosticDetailValue::String("bad\u{fffd}value".into())
        );
    }

    #[test]
    fn detail_string_limit_accepts_512_bytes_and_rejects_513_bytes() {
        let at_limit = "x".repeat(MAX_DIAGNOSTIC_DETAIL_STRING_BYTES);
        let mut accepted = stored_detail_sink();
        accepted.string("token", &at_limit);
        let build = match accepted.finish() {
            Ok(build) => build,
            Err(_) => panic!("512-byte detail string must be accepted"),
        };
        assert_eq!(
            build.details.first().expect("stored detail").value(),
            &DiagnosticDetailValue::String(at_limit)
        );

        let over_limit = "x".repeat(MAX_DIAGNOSTIC_DETAIL_STRING_BYTES + 1);
        let mut rejected = stored_detail_sink();
        rejected.string("token", &over_limit);
        let failure = match rejected.finish() {
            Ok(_) => panic!("513-byte detail string must be rejected"),
            Err(failure) => failure,
        };
        assert_eq!(failure.cause, DiagnosticCause::DetailStringExceeded);
    }
}
