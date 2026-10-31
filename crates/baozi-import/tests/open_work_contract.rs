mod support;

use std::io::Cursor;
use std::num::NonZeroU64;
use std::sync::Mutex;

use baozi_core::FormatId;
use baozi_import::{
    DependencyRequirement, DependencyRoleId, DetectionOutcome, DetectionResult, DetectionScope,
    FormatDescriptor, FormatEncoding, FormatImporter, ImportCause, ImportDimension, ImportFailure,
    ImportOperationControl, ImportReport, ImportRequest, OptionCompileLimits, ParseResult,
    ParseScope, PreparedImportRequest, Probe, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuilder, SceneDraft, ScopeAbortCause, TrustedExtensionBoundary, import_asset,
};
use baozi_io::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetOpenWorkError,
    AssetPath, AuthorityDomain, OpenedAsset,
};
use baozi_operation::{CancellationToken, ControlCause, CounterClass, LedgerCause};

const PRIMARY: &str = "primary.open-work";
const DEPENDENCY: &str = "sidecar.bin";
const DEFAULT_WEIGHT: u64 = 17;

struct TestImporter {
    dependency: bool,
}

impl FormatImporter for TestImporter {
    fn descriptor(&self) -> FormatDescriptor {
        let builder = FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.open-work").unwrap(),
            "Open Work Contract",
        )
        .extensions(["open-work"])
        .encoding(FormatEncoding::Binary);
        let builder = if self.dependency {
            builder.dependency_roles([(
                DependencyRoleId::new("test.sidecar").unwrap(),
                DependencyRequirement::Required,
            )])
        } else {
            builder
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

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        if self.dependency {
            let role = DependencyRoleId::new("test.sidecar").unwrap();
            // Both references authorize; the physical cache must query/open only once.
            for _ in 0..2 {
                let dependency = support::required_root_dependency(scope, &role, DEPENDENCY)?;
                assert_eq!(dependency.bytes(), b"payload");
            }
        }
        support::submit_minimal_scene(scope)
    }
}

enum Behavior {
    Return,
    BoundError(AssetOpenWorkError),
    BoundPanic,
    CancelDuringBound(CancellationToken),
    OpenError,
    OpenPanic,
    WrongIdentity,
}

struct TestIo {
    domain: AuthorityDomain,
    selected_path: &'static str,
    weight: u64,
    behavior: Behavior,
    calls: Mutex<Vec<(&'static str, String)>>,
}

impl TestIo {
    fn new(dependency: bool, weight: u64, behavior: Behavior) -> Self {
        Self {
            domain: AuthorityDomain::new(),
            selected_path: if dependency { DEPENDENCY } else { PRIMARY },
            weight,
            behavior,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls_for(&self, action: &str, path: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, requested)| *kind == action && requested == path)
            .count()
    }
}

impl AssetIo for TestIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        self.calls.lock().unwrap().push(("bound", path.to_string()));
        if path.as_str() == self.selected_path {
            match &self.behavior {
                Behavior::BoundError(cause) => return Err(*cause),
                Behavior::BoundPanic => panic!("bound query panic"),
                Behavior::CancelDuringBound(cancellation) => cancellation.cancel(),
                _ => {}
            }
            Ok(NonZeroU64::new(self.weight).unwrap())
        } else {
            Ok(NonZeroU64::new(DEFAULT_WEIGHT).unwrap())
        }
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.calls.lock().unwrap().push(("open", path.to_string()));
        let mut domain = self.domain.clone();
        if path.as_str() == self.selected_path {
            match self.behavior {
                Behavior::OpenError => {
                    return Err(AssetOpenError::new(
                        AssetOpenStage::Open,
                        AssetOpenErrorKind::NotFound,
                    ));
                }
                Behavior::OpenPanic => panic!("open panic"),
                Behavior::WrongIdentity => domain = AuthorityDomain::new(),
                _ => {}
            }
        }
        Ok(OpenedAsset::new(
            domain,
            path.clone(),
            Cursor::new(b"payload"),
            Some(AssetLength::Exact(7)),
        ))
    }
}

fn request(dependency: bool) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    registry.register(TestImporter { dependency }).unwrap();
    registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
        .unwrap()
}

#[allow(
    clippy::result_large_err,
    reason = "contract tests inspect complete terminal evidence without changing its owner"
)]
fn run(
    dependency: bool,
    io: &TestIo,
    control: ImportOperationControl,
) -> Result<ImportReport, ImportFailure> {
    import_asset(
        &request(dependency),
        io,
        AssetPath::new(PRIMARY).unwrap(),
        control,
    )
    .map_err(|error| error.into_operation_failure().unwrap())
}

fn extension_code(failure: &ImportFailure) -> &'static str {
    match failure.cause() {
        ImportCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::AssetIo,
            code,
        } => code,
        ImportCause::Scope(abort) => match abort.cause() {
            ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::AssetIo,
                code,
            } => code,
            cause => panic!("unexpected scope failure: {cause:?}"),
        },
        cause => panic!("unexpected import failure: {cause:?}"),
    }
}

#[test]
fn primary_and_dependency_open_weights_complete_and_cache_hits_do_not_repeat_them() {
    for dependency in [false, true] {
        let first = TestIo::new(dependency, DEFAULT_WEIGHT, Behavior::Return);
        let second = TestIo::new(dependency, DEFAULT_WEIGHT + 13, Behavior::Return);
        let first_report = run(dependency, &first, ImportOperationControl::default()).unwrap();
        let second_report = run(dependency, &second, ImportOperationControl::default()).unwrap();
        let first_work = first_report
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap();
        let second_work = second_report
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap();
        assert_eq!(second_work.attempted() - first_work.attempted(), 13);
        assert_eq!(second_work.consumed() - first_work.consumed(), 13);
        for io in [&first, &second] {
            assert_eq!(io.calls_for("bound", io.selected_path), 1);
            assert_eq!(io.calls_for("open", io.selected_path), 1);
        }
    }
}

#[test]
fn open_work_is_admitted_before_either_backend_entry() {
    for dependency in [false, true] {
        let io = TestIo::new(dependency, 1_000_000_000, Behavior::Return);
        let failure = run(dependency, &io, ImportOperationControl::default()).unwrap_err();
        assert_eq!(io.calls_for("bound", io.selected_path), 1);
        assert_eq!(io.calls_for("open", io.selected_path), 0);
        let ImportCause::Scope(abort) = failure.cause() else {
            panic!("expected work admission failure");
        };
        assert!(matches!(
            abort.cause(),
            ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
                class: CounterClass::Attempted,
                dimension: ImportDimension::WorkUnits,
                ..
            })
        ));
        let opens = failure
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap();
        assert_eq!(opens.attempted(), if dependency { 2 } else { 1 });
        assert_eq!(opens.consumed(), u64::from(dependency));
        let work = failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap();
        assert!(work.attempted() >= 1_000_000_000);
        assert!(work.consumed() < 1_000_000_000);
    }
}

#[test]
fn normal_open_errors_complete_work_and_panics_leave_only_attempted_work() {
    for dependency in [false, true] {
        let returned = TestIo::new(dependency, 701, Behavior::OpenError);
        let panicked = TestIo::new(dependency, 701, Behavior::OpenPanic);
        let returned = run(dependency, &returned, ImportOperationControl::default()).unwrap_err();
        let panicked = run(dependency, &panicked, ImportOperationControl::default()).unwrap_err();
        let ImportCause::Scope(abort) = returned.cause() else {
            panic!("expected original open error");
        };
        assert!(matches!(abort.cause(), ScopeAbortCause::AssetOpen(cause)
            if cause.kind() == AssetOpenErrorKind::NotFound));
        assert_eq!(
            extension_code(&panicked),
            if dependency {
                "asset-open-panicked"
            } else {
                "primary-open-panicked"
            }
        );
        for (failure, completed) in [(&returned, true), (&panicked, false)] {
            let work = failure
                .ledger()
                .dimension(ImportDimension::WorkUnits)
                .unwrap();
            if completed {
                assert_eq!(work.attempted(), work.consumed());
            } else {
                assert_eq!(work.attempted() - work.consumed(), 701);
            }
        }
    }
}

#[test]
fn bound_failures_are_terminal_and_never_enter_open() {
    for dependency in [false, true] {
        for (behavior, suffix) in [
            (
                Behavior::BoundError(AssetOpenWorkError::Unbounded),
                "unbounded",
            ),
            (
                Behavior::BoundError(AssetOpenWorkError::Overflow),
                "overflow",
            ),
            (Behavior::BoundPanic, "panicked"),
        ] {
            let io = TestIo::new(dependency, DEFAULT_WEIGHT, behavior);
            let failure = run(dependency, &io, ImportOperationControl::default()).unwrap_err();
            let prefix = if dependency { "asset" } else { "primary" };
            assert_eq!(
                extension_code(&failure),
                format!("{prefix}-open-work-{suffix}")
            );
            assert_eq!(io.calls_for("open", io.selected_path), 0);
        }
    }
}

#[test]
fn cancellation_raised_during_bound_query_precedes_open() {
    for dependency in [false, true] {
        let cancellation = CancellationToken::new();
        let io = TestIo::new(
            dependency,
            DEFAULT_WEIGHT,
            Behavior::CancelDuringBound(cancellation.clone()),
        );
        let failure = run(
            dependency,
            &io,
            ImportOperationControl::default().with_cancellation(cancellation),
        )
        .unwrap_err();
        let ImportCause::Scope(abort) = failure.cause() else {
            panic!("expected cancellation");
        };
        assert!(matches!(
            abort.cause(),
            ScopeAbortCause::Control(ControlCause::Cancelled { .. })
        ));
        assert_eq!(io.calls_for("open", io.selected_path), 0);
    }
}

#[test]
fn returned_object_identity_is_checked_after_completing_opaque_open_work() {
    for dependency in [false, true] {
        let io = TestIo::new(dependency, 701, Behavior::WrongIdentity);
        let failure = run(dependency, &io, ImportOperationControl::default()).unwrap_err();
        assert_eq!(
            extension_code(&failure),
            if dependency {
                "opened-object-identity-mismatch"
            } else {
                "primary-open-identity-mismatch"
            }
        );
        let work = failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap();
        assert_eq!(work.attempted(), work.consumed());
        let opens = failure
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap();
        assert_eq!(opens.consumed(), u64::from(dependency));
    }
}
