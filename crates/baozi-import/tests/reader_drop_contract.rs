mod support;

use std::io::Cursor;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use baozi_core::FormatId;
use baozi_import::{
    CompiledOptions, DependencyCacheDisposition, DependencyRequirement, DependencyRoleId,
    DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor, FormatEncoding,
    ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure, ImportLossKind,
    ImportOperationControl, ImportPlan, ImportReport, ImportRequest, ImportRequestLayer,
    LossPolicy, ManifestRelationOutcome, NoFormatOptions, OptionCompileError, OptionCompileLimits,
    ParseContext, ParseResult, Probe, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuildScope, RegistryBuilder, ResourceLimits, ResourceLimitsOverlay, ScopeAbortCause,
    TrustedExtensionBoundary,
};
use baozi_io::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenWorkError, AssetPath, AssetReadError,
    AssetReadErrorKind, AssetReader, AuthorityDomain, OpenedAsset,
};
use baozi_operation::{CancellationToken, ControlCause, CounterClass, LedgerCause};

const PRIMARY: &str = "primary.reader-drop";
const DEPENDENCY: &str = "sidecar.bin";
const PAYLOAD: &[u8] = b"payload";
const OPEN_WORK: u64 = 101;

#[derive(Debug, Clone, Copy)]
enum Lane {
    Primary,
    Required,
    Optional,
}

impl Lane {
    const ALL: [Self; 3] = [Self::Primary, Self::Required, Self::Optional];

    fn selected_path(self) -> &'static str {
        match self {
            Self::Primary => PRIMARY,
            Self::Required | Self::Optional => DEPENDENCY,
        }
    }
}

struct TestImporter {
    lane: Lane,
    available_snapshots: Arc<AtomicUsize>,
}

impl Format for TestImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        let builder = FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.reader-drop").unwrap(),
            "Reader Drop Contract",
        )
        .extensions(["reader-drop"])
        .encoding(FormatEncoding::Binary);
        let builder = match self.lane {
            Lane::Primary => builder,
            Lane::Required | Lane::Optional => builder.dependency_roles([(
                DependencyRoleId::new("test.sidecar").unwrap(),
                if matches!(self.lane, Lane::Required) {
                    DependencyRequirement::Required
                } else {
                    DependencyRequirement::Optional
                },
            )]),
        };
        builder.build().unwrap()
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, _options: &Self::Options, context: &mut ParseContext<'_>) -> ParseResult<()> {
        context.with_execution(|exec| -> ParseResult<()> {
            assert_eq!(exec.primary_bytes(), PAYLOAD);
            if !matches!(self.lane, Lane::Primary) {
                let role = DependencyRoleId::new("test.sidecar").unwrap();
                for _ in 0..2 {
                    let outcome = support::root_dependency(exec, &role, DEPENDENCY)?;
                    if let Some(snapshot) = outcome.snapshot() {
                        assert_eq!(snapshot.bytes(), PAYLOAD);
                        self.available_snapshots.fetch_add(1, Ordering::AcqRel);
                    } else {
                        assert!(matches!(self.lane, Lane::Optional));
                        assert!(outcome.unavailable_evidence().is_some());
                    }
                }
            }
            Ok(())
        })?;
        support::submit_minimal_scene(context)
    }
}

#[derive(Debug, Clone, Copy)]
enum Fault {
    Eof,
    ReadError,
    ReadPanic,
    CancelDuringRead,
    CancelDuringOpen,
    WrongAuthority,
    WrongPath,
    BackendBudget,
}

#[derive(Clone, Copy)]
enum DropFault {
    None,
    String,
    Opaque,
}

#[derive(Default)]
struct Observations {
    opens: AtomicUsize,
    reads: AtomicUsize,
    eof: AtomicUsize,
    drops: AtomicUsize,
    payload_drops: AtomicUsize,
}

struct HostilePayload(Arc<Observations>);

impl Drop for HostilePayload {
    fn drop(&mut self) {
        self.0.payload_drops.fetch_add(1, Ordering::AcqRel);
        panic!("hostile panic payload destructor");
    }
}

struct TestReader {
    fault: Fault,
    drop_fault: DropFault,
    offset: usize,
    cancellation: CancellationToken,
    observations: Arc<Observations>,
}

impl AssetReader for TestReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        self.observations.reads.fetch_add(1, Ordering::AcqRel);
        if self.offset != 0 {
            match self.fault {
                Fault::ReadError => {
                    return Err(AssetReadError::new(AssetReadErrorKind::InvalidData));
                }
                Fault::ReadPanic => {
                    std::panic::panic_any(HostilePayload(Arc::clone(&self.observations)))
                }
                _ => {}
            }
        }
        if matches!(self.fault, Fault::CancelDuringRead) {
            self.cancellation.cancel();
        }
        let count = buffer.len().min(PAYLOAD.len() - self.offset);
        buffer[..count].copy_from_slice(&PAYLOAD[self.offset..self.offset + count]);
        self.offset += count;
        if count == 0 {
            self.observations.eof.fetch_add(1, Ordering::AcqRel);
        }
        Ok(count)
    }
}

impl Drop for TestReader {
    fn drop(&mut self) {
        self.observations.drops.fetch_add(1, Ordering::AcqRel);
        match self.drop_fault {
            DropFault::None => {}
            DropFault::String => panic!("reader destructor panic"),
            DropFault::Opaque => {
                std::panic::panic_any(HostilePayload(Arc::clone(&self.observations)))
            }
        }
    }
}

struct TestIo {
    domain: AuthorityDomain,
    lane: Lane,
    fault: Fault,
    drop_fault: DropFault,
    cancellation: CancellationToken,
    observations: Arc<Observations>,
}

impl TestIo {
    fn new(lane: Lane, fault: Fault, drop_fault: DropFault) -> Self {
        Self {
            domain: AuthorityDomain::new(),
            lane,
            fault,
            drop_fault,
            cancellation: CancellationToken::new(),
            observations: Arc::new(Observations::default()),
        }
    }

    fn control(&self) -> ImportOperationControl {
        ImportOperationControl::default().with_cancellation(self.cancellation.clone())
    }

    fn assert_released(&self, expected_reads: usize) {
        assert_eq!(self.observations.opens.load(Ordering::Acquire), 1);
        assert_eq!(
            self.observations.reads.load(Ordering::Acquire),
            expected_reads
        );
        assert_eq!(self.observations.drops.load(Ordering::Acquire), 1);
        assert_eq!(self.observations.payload_drops.load(Ordering::Acquire), 0);
    }
}

impl AssetIo for TestIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, _path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // Covers the bounded fixture path clone, identity setup and reader construction.
        Ok(NonZeroU64::new(OPEN_WORK).unwrap())
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        if path.as_str() != self.lane.selected_path() {
            return Ok(OpenedAsset::new(
                self.domain.clone(),
                path.clone(),
                Cursor::new(PAYLOAD),
                Some(AssetLength::Exact(PAYLOAD.len() as u64)),
            ));
        }
        self.observations.opens.fetch_add(1, Ordering::AcqRel);
        let mut opened = OpenedAsset::new(
            if matches!(self.fault, Fault::WrongAuthority) {
                AuthorityDomain::new()
            } else {
                self.domain.clone()
            },
            if matches!(self.fault, Fault::WrongPath) {
                AssetPath::new("wrong.bin").unwrap()
            } else {
                path.clone()
            },
            TestReader {
                fault: self.fault,
                drop_fault: self.drop_fault,
                offset: 0,
                cancellation: self.cancellation.clone(),
                observations: Arc::clone(&self.observations),
            },
            Some(AssetLength::Exact(PAYLOAD.len() as u64)),
        );
        if matches!(self.fault, Fault::BackendBudget) {
            opened = opened.with_backend_hard_limit(0);
        }
        if matches!(self.fault, Fault::CancelDuringOpen) {
            self.cancellation.cancel();
        }
        Ok(opened)
    }
}

fn requests(lane: Lane, limits: ResourceLimits) -> (ImportPlan, ImportPlan, Arc<AtomicUsize>) {
    let available = Arc::new(AtomicUsize::new(0));
    let mut registry = RegistryBuilder::new();
    registry
        .register(TestImporter {
            lane,
            available_snapshots: Arc::clone(&available),
        })
        .unwrap();
    let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let request = ImportRequest::from_layers(
        ImportRequestLayer::default(),
        ImportRequestLayer::new(
            ImportConfigOverlay::default()
                .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
                .with_loss_policy(
                    LossPolicy::preserve_or_fail()
                        .allow(ImportLossKind::OptionalDependencyUnavailable),
                ),
        ),
    );
    (
        registry
            .plan(&request, OptionCompileLimits::default())
            .unwrap(),
        registry
            .plan(&ImportRequest::new(), OptionCompileLimits::default())
            .unwrap(),
        available,
    )
}

#[allow(
    clippy::result_large_err,
    reason = "contract assertions retain the public terminal evidence"
)]
fn run(request: &ImportPlan, io: &TestIo) -> Result<ImportReport, ImportFailure> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        request.read_asset(io, AssetPath::new(PRIMARY).unwrap(), io.control())
    }));
    match result {
        Ok(result) => result.map_err(|error| error.into_operation_failure().unwrap()),
        Err(payload) => {
            // Keep the red regression safe even when the escaped payload has hostile Drop.
            std::mem::forget(payload);
            panic!("reader destruction escaped the AssetIo boundary");
        }
    }
}

fn assert_extension(failure: &ImportFailure, expected: &'static str) {
    match failure.cause() {
        ImportCause::TrustedExtension { boundary, code } => {
            assert_eq!(*boundary, TrustedExtensionBoundary::AssetIo);
            assert_eq!(*code, expected);
        }
        ImportCause::Scope(abort) => assert!(
            matches!(
                abort.cause(),
                ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code,
                } if *code == expected
            ),
            "unexpected scope cause: {:?}",
            abort.cause()
        ),
        cause => panic!("unexpected import cause: {cause:?}"),
    }
}

fn scope_cause(failure: &ImportFailure) -> &ScopeAbortCause {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected a scope cause: {:?}", failure.cause());
    };
    abort.cause()
}

fn assert_healthy_retry(request: &ImportPlan, lane: Lane) {
    let io = TestIo::new(lane, Fault::Eof, DropFault::None);
    let report =
        run(request, &io).expect("the same registry must remain healthy after an IO fault");
    assert!(report.optional_dependency_evidence().is_empty());
    io.assert_released(2);
}

fn assert_optional_omission(report: &ImportReport, expected_code: &str) {
    let evidence = report.optional_dependency_evidence();
    assert_eq!(evidence.len(), 2);
    assert_eq!(
        evidence[0].cache_disposition(),
        DependencyCacheDisposition::Fresh
    );
    assert_eq!(
        evidence[1].cache_disposition(),
        DependencyCacheDisposition::Reused
    );
    let manifest = report.successful_input_manifest();
    let relation = &manifest.node_classes()[0].outgoing_relations()[0];
    assert!(matches!(
        relation.outcome(),
        ManifestRelationOutcome::Unavailable { code, .. } if code.as_str() == expected_code
    ));
}

#[test]
fn reader_drop_at_successful_eof_is_an_io_fault_without_publishing_a_snapshot() {
    for lane in Lane::ALL {
        let (request, retry, available) = requests(lane, ResourceLimits::default());
        let io = TestIo::new(lane, Fault::Eof, DropFault::String);
        let failure =
            run(&request, &io).expect_err("reader cleanup must complete before publication");
        io.assert_released(2);
        assert_eq!(io.observations.eof.load(Ordering::Acquire), 1);
        assert_eq!(available.load(Ordering::Acquire), 0);
        assert_extension(&failure, "asset-reader-drop-panicked");
        assert_healthy_retry(&retry, lane);
    }
}

#[test]
fn reader_drop_preserves_the_first_read_error_and_observed_bytes() {
    for lane in [Lane::Primary, Lane::Required] {
        let (request, retry, _) = requests(lane, ResourceLimits::default());
        let io = TestIo::new(lane, Fault::ReadError, DropFault::String);
        let failure = run(&request, &io).unwrap_err();
        io.assert_released(2);
        assert!(matches!(
            scope_cause(&failure),
            ScopeAbortCause::AssetRead(error) if error.kind() == AssetReadErrorKind::InvalidData
        ));
        let dimension = if matches!(lane, Lane::Primary) {
            ImportDimension::PrimaryBytes
        } else {
            ImportDimension::DependencyBytes
        };
        assert_eq!(failure.ledger().dimension(dimension).unwrap().consumed(), 7);
        assert_healthy_retry(&retry, lane);
    }
}

#[test]
fn reader_drop_preserves_the_first_read_panic_without_dropping_its_payload() {
    for lane in Lane::ALL {
        let (request, retry, _) = requests(lane, ResourceLimits::default());
        let io = TestIo::new(lane, Fault::ReadPanic, DropFault::String);
        let failure = run(&request, &io).unwrap_err();
        io.assert_released(2);
        assert_extension(&failure, "asset-read-panicked");
        assert_healthy_retry(&retry, lane);
    }
}

#[test]
fn reader_drop_preserves_cancellation_after_a_read() {
    for lane in Lane::ALL {
        let (request, retry, _) = requests(lane, ResourceLimits::default());
        let io = TestIo::new(lane, Fault::CancelDuringRead, DropFault::String);
        let failure = run(&request, &io).unwrap_err();
        io.assert_released(1);
        assert!(matches!(
            scope_cause(&failure),
            ScopeAbortCause::Control(ControlCause::Cancelled { .. })
        ));
        assert_healthy_retry(&retry, lane);
    }
}

#[test]
fn reader_drop_preserves_cancellation_after_open_before_any_read() {
    for lane in Lane::ALL {
        let (request, retry, _) = requests(lane, ResourceLimits::default());
        let io = TestIo::new(lane, Fault::CancelDuringOpen, DropFault::String);
        let failure = run(&request, &io).unwrap_err();
        io.assert_released(0);
        assert!(matches!(
            scope_cause(&failure),
            ScopeAbortCause::Control(ControlCause::Cancelled { .. })
        ));
        let work = failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap();
        assert!(work.consumed() >= OPEN_WORK);
        assert_eq!(work.attempted(), work.consumed());
        assert_healthy_retry(&retry, lane);
    }
}

#[test]
fn reader_drop_preserves_early_authority_and_path_identity_rejection() {
    for lane in Lane::ALL {
        for fault in [Fault::WrongAuthority, Fault::WrongPath] {
            let (request, retry, _) = requests(lane, ResourceLimits::default());
            let io = TestIo::new(lane, fault, DropFault::String);
            let failure = run(&request, &io).unwrap_err();
            io.assert_released(0);
            assert_extension(
                &failure,
                if matches!(lane, Lane::Primary) {
                    "primary-open-identity-mismatch"
                } else {
                    "opened-object-identity-mismatch"
                },
            );
            let opens = failure
                .ledger()
                .dimension(ImportDimension::AssetOpen)
                .unwrap();
            assert_eq!(opens.consumed(), u64::from(!matches!(lane, Lane::Primary)));
            assert_healthy_retry(&retry, lane);
        }
    }
}

#[test]
fn reader_drop_preserves_a_byte_budget_failure_before_reading() {
    for lane in [Lane::Primary, Lane::Required] {
        let (request, retry, _) = requests(lane, ResourceLimits::default());
        let io = TestIo::new(lane, Fault::BackendBudget, DropFault::String);
        let failure = run(&request, &io).unwrap_err();
        io.assert_released(0);
        assert!(matches!(
            scope_cause(&failure),
            ScopeAbortCause::ResourceLimit {
                resource: "asset-bytes",
                limit: 0
            }
        ));
        assert_healthy_retry(&retry, lane);
    }
}

#[test]
fn reader_drop_preserves_post_open_dependency_acquisition_rejection() {
    let mut limits = ResourceLimits::default();
    limits.max_dependency_acquisition_work = 0;
    let (request, retry, _) = requests(Lane::Required, limits);
    let io = TestIo::new(Lane::Required, Fault::Eof, DropFault::String);
    let failure = run(&request, &io).unwrap_err();
    io.assert_released(0);
    assert!(matches!(
        scope_cause(&failure),
        ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: ImportDimension::DependencyAcquisitionWork,
            value: 1,
            limit: 0,
        })
    ));
    assert_healthy_retry(&retry, Lane::Required);
}

#[test]
fn optional_read_error_keeps_its_cached_omission_when_reader_drop_panics() {
    let (request, retry, available) = requests(Lane::Optional, ResourceLimits::default());
    let io = TestIo::new(Lane::Optional, Fault::ReadError, DropFault::String);
    let report =
        run(&request, &io).expect("cleanup cannot replace the allowed read-error omission");
    io.assert_released(2);
    assert_eq!(available.load(Ordering::Acquire), 0);
    assert_optional_omission(&report, "read-failed");
    assert_healthy_retry(&retry, Lane::Optional);
}

#[test]
fn optional_budget_failure_keeps_its_cached_omission_when_reader_drop_panics() {
    for acquisition_budget in [false, true] {
        let mut limits = ResourceLimits::default();
        let fault = if acquisition_budget {
            limits.max_dependency_acquisition_work = 0;
            Fault::Eof
        } else {
            Fault::BackendBudget
        };
        let (request, retry, available) = requests(Lane::Optional, limits);
        let io = TestIo::new(Lane::Optional, fault, DropFault::String);
        let report =
            run(&request, &io).expect("cleanup cannot replace the allowed budget omission");
        io.assert_released(0);
        assert_eq!(available.load(Ordering::Acquire), 0);
        assert_optional_omission(&report, "resource-limit");
        assert_healthy_retry(&retry, Lane::Optional);
    }
}

#[test]
fn reader_drop_never_runs_a_hostile_panic_payload_destructor() {
    for lane in Lane::ALL {
        for fault in [Fault::Eof, Fault::ReadPanic] {
            let (request, retry, _) = requests(lane, ResourceLimits::default());
            let io = TestIo::new(lane, fault, DropFault::Opaque);
            let failure = run(&request, &io).unwrap_err();
            io.assert_released(2);
            assert_extension(
                &failure,
                if matches!(fault, Fault::ReadPanic) {
                    "asset-read-panicked"
                } else {
                    "asset-reader-drop-panicked"
                },
            );
            assert_healthy_retry(&retry, lane);
        }
    }
}
