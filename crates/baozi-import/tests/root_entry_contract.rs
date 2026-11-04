use std::num::NonZeroU64;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use baozi_core::FormatId;
use baozi_import::{
    CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor,
    FormatEncoding, ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure,
    ImportOperationControl, ImportPhase, ImportPlan, ImportReport, ImportRequest,
    ImportRequestLayer, ImportResult, ImportTerminalCause, ImportTerminalFailureClass,
    NoFormatOptions, OptionCompileError, OptionCompileLimits, ParseContext, ParseResult, Probe,
    ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuildScope, RegistryBuilder,
    ResourceLimits, ResourceLimitsOverlay, ScopeAbortCause,
};
use baozi_io::{
    AssetIo, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetOpenWorkError, AssetPath,
    AuthorityDomain, OpenedAsset,
};
use baozi_operation::{
    CancellationOrigin, CancellationSignal, ControlCause, CounterClass, LedgerCause,
};

struct UnreachedParser {
    calls: Arc<AtomicUsize>,
}

impl Format for UnreachedParser {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.root-entry").expect("valid root entry format id"),
            "Root Entry Contract",
        )
        .extensions(["root-entry"])
        .encoding(FormatEncoding::Binary)
        .build()
        .expect("valid root entry descriptor")
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        _context: &mut ParseContext<'_, '_>,
    ) -> ParseResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        panic!("root entry failure must precede parser invocation")
    }
}

fn plan(parse_calls: &Arc<AtomicUsize>, limits: ResourceLimits) -> ImportPlan {
    let mut registry = RegistryBuilder::new();
    registry
        .register(UnreachedParser {
            calls: Arc::clone(parse_calls),
        })
        .expect("register root entry parser");
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits)),
    ));
    registry
        .freeze(RegistryBuildLimits::default())
        .expect("freeze root entry registry")
        .plan(&request, OptionCompileLimits::default())
        .expect("compile root entry request")
}

fn operation_failure(result: ImportResult<ImportReport>) -> ImportFailure {
    result
        .expect_err("root entry must fail before parsing")
        .into_operation_failure()
        .expect("root entry fixture must initialize and retain terminal evidence")
}

fn assert_counters(
    failure: &ImportFailure,
    dimension: ImportDimension,
    attempted: u64,
    consumed: u64,
) {
    let snapshot = failure
        .ledger()
        .dimension(dimension)
        .expect("import dimension is present");
    assert_eq!(snapshot.attempted(), attempted, "{dimension:?}");
    assert_eq!(snapshot.consumed(), consumed, "{dimension:?}");
    assert_eq!(snapshot.current_live_bytes(), 0, "{dimension:?}");
}

fn assert_stopped_before_parser(failure: &ImportFailure, parse_calls: &AtomicUsize) {
    assert_eq!(parse_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SessionCreated
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

struct CountingCancellation {
    checks: Arc<AtomicUsize>,
    cancel_on: Option<usize>,
}

impl CancellationSignal for CountingCancellation {
    fn is_cancelled(&self) -> bool {
        let check = self.checks.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel_on.is_some_and(|cancel_on| check >= cancel_on)
    }
}

struct OpenFailureIo {
    domain: AuthorityDomain,
    checks: Arc<AtomicUsize>,
    frontier: Mutex<Vec<(&'static str, usize)>>,
}

impl OpenFailureIo {
    fn new(checks: &Arc<AtomicUsize>) -> Self {
        Self {
            domain: AuthorityDomain::new(),
            checks: Arc::clone(checks),
            frontier: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, entry: &'static str) {
        self.frontier
            .lock()
            .expect("IO frontier lock")
            .push((entry, self.checks.load(Ordering::SeqCst)));
    }
}

impl AssetIo for OpenFailureIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.record("authority");
        self.domain.clone()
    }

    fn open_work_bound(&self, _path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        self.record("bound");
        Ok(NonZeroU64::new(17).expect("nonzero bounded open work"))
    }

    fn open(&self, _path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.record("open");
        Err(AssetOpenError::new(
            AssetOpenStage::Open,
            AssetOpenErrorKind::NotFound,
        ))
    }
}

#[test]
fn primary_byte_limit_preserves_original_ledger_failure_and_attempted_prefix() {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = 3;
    let request = plan(&parse_calls, limits);

    let failure = operation_failure(request.read_bytes(
        AssetPath::new("primary.root-entry").expect("valid primary path"),
        b"123456789",
        ImportOperationControl::default(),
    ));

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!(
            "expected original primary byte ledger failure: {:?}",
            failure.cause()
        );
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: ImportDimension::PrimaryBytes,
            value: 9,
            limit: 3,
        })
    ));
    assert!(matches!(
        failure.ledger().terminal_cause(),
        ImportTerminalCause::Failure(terminal)
            if terminal.class() == ImportTerminalFailureClass::Ledger
    ));
    assert_counters(&failure, ImportDimension::PrimaryBytes, 9, 0);
    assert_counters(&failure, ImportDimension::TotalAssetBytes, 0, 0);
    assert_counters(&failure, ImportDimension::AssetOpen, 0, 0);
    assert_counters(&failure, ImportDimension::WorkUnits, 10, 10);
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::PrimaryBytes)
            .expect("primary byte dimension")
            .peak_live_bytes(),
        0
    );
    assert_stopped_before_parser(&failure, &parse_calls);
}

#[test]
fn ordinary_root_entry_preserves_io_poll_frontier_and_returned_error_work() {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = plan(&parse_calls, ResourceLimits::default());
    let checks = Arc::new(AtomicUsize::new(0));
    let io = OpenFailureIo::new(&checks);
    let control = ImportOperationControl::default().with_cancellation(CountingCancellation {
        checks: Arc::clone(&checks),
        cancel_on: None,
    });

    let failure = operation_failure(request.read_asset(
        &io,
        AssetPath::new("primary.root-entry").expect("valid primary path"),
        control,
    ));

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected backend open failure: {:?}", failure.cause());
    };
    let ScopeAbortCause::AssetOpen(cause) = abort.cause() else {
        panic!("expected returned backend open error: {:?}", abort.cause());
    };
    assert_eq!(cause.stage(), AssetOpenStage::Open);
    assert_eq!(cause.kind(), AssetOpenErrorKind::NotFound);
    assert_eq!(
        io.frontier.lock().expect("IO frontier lock").as_slice(),
        &[("authority", 3), ("bound", 4), ("open", 5)]
    );
    assert_counters(&failure, ImportDimension::AssetOpen, 1, 0);
    assert_counters(&failure, ImportDimension::WorkUnits, 27, 27);
    assert_counters(&failure, ImportDimension::PrimaryBytes, 0, 0);
    assert_stopped_before_parser(&failure, &parse_calls);
}

#[test]
fn cancellation_after_boundary_preserves_open_attempt_before_observed_poll() {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let request = plan(&parse_calls, ResourceLimits::default());
    let checks = Arc::new(AtomicUsize::new(0));
    let io = OpenFailureIo::new(&checks);
    let control = ImportOperationControl::default().with_cancellation(CountingCancellation {
        checks: Arc::clone(&checks),
        cancel_on: Some(5),
    });

    let failure = operation_failure(request.read_asset(
        &io,
        AssetPath::new("primary.root-entry").expect("valid primary path"),
        control,
    ));

    let ImportCause::Scope(abort) = failure.cause() else {
        panic!(
            "expected cancellation at observed open admission: {:?}",
            failure.cause()
        );
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        })
    ));
    assert_eq!(checks.load(Ordering::SeqCst), 5);
    assert_eq!(
        io.frontier.lock().expect("IO frontier lock").as_slice(),
        &[("authority", 3), ("bound", 4)]
    );
    assert_counters(&failure, ImportDimension::AssetOpen, 1, 0);
    assert_counters(&failure, ImportDimension::WorkUnits, 10, 10);
    assert_counters(&failure, ImportDimension::PrimaryBytes, 0, 0);
    assert_stopped_before_parser(&failure, &parse_calls);
}
