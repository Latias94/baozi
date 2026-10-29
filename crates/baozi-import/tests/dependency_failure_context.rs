use baozi_import::authoring::FormatId;
use baozi_import::{
    AuthorizedDependency, CapabilityStatus, CauseOwner, DependencyPolicyError, DependencyReference,
    DependencyRequirement, DependencyResolver, DependencyRoleId, DetectionOutcome, DetectionResult,
    DetectionScope, FormatCapability, FormatDescriptor, FormatEncoding,
    FormatFailureEvidenceProducer, FormatImporter, ImportCause, ImportConfigOverlay,
    ImportDimension, ImportFailure, ImportLossKind, ImportOperationControl, ImportRequest,
    ImportRequestLayer, LossPolicy, OptionCompileLimits, ParseResult, ParseScope,
    PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    SceneDraft, ScopeAbortCause, import_asset, import_asset_with_resolver,
};
use baozi_io::{
    AssetIo, AssetOpenError, AssetOpenErrorKind, AssetPath, AuthorityDomain,
    ImmutableMemoryAssetIo, OpenedAsset, PathError,
};
use std::error::Error;
use std::sync::{Arc, Mutex};

const SOURCE: &str = "models/asset.context";
const ORIGIN: &str = "models/materials/library.side";
const REFERENCE: &str = "../textures/./missing.bin";
const NORMALIZED: &str = "models/textures/missing.bin";

struct NestedRequiredImporter {
    reference: &'static str,
    catch_failure: bool,
}

struct UnexpectedAvailability;

impl FormatFailureEvidenceProducer for UnexpectedAvailability {
    fn code(&self) -> &str {
        "test.required_dependency_unexpectedly_available"
    }
}

impl FormatImporter for NestedRequiredImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.dependency-context").unwrap(),
            "Required dependency context test",
        )
        .extensions(["context"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .dependency_roles([
            (role("sidecar.required"), DependencyRequirement::Required),
            (role("payload.required"), DependencyRequirement::Required),
        ])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let sidecar = scope.dependency(
            &scope.root_origin(),
            &role("sidecar.required"),
            "materials/library.side",
        )?;
        let sidecar = scope.into_available_dependency(sidecar)?;
        let payload =
            scope.dependency(&sidecar.origin(), &role("payload.required"), self.reference);
        if self.catch_failure {
            // A plugin cannot erase the latched required failure by returning its own error.
            drop(payload);
        } else {
            let _ = payload?;
        }
        Err(scope.format_failure(UnexpectedAvailability))
    }
}

struct LoggedIo {
    inner: ImmutableMemoryAssetIo,
    opens: Mutex<Vec<AssetPath>>,
}

impl LoggedIo {
    fn new(with_documents: bool) -> Self {
        let assets = if with_documents {
            vec![
                (AssetPath::new(SOURCE).unwrap(), Arc::from(&b"primary"[..])),
                (AssetPath::new(ORIGIN).unwrap(), Arc::from(&b"sidecar"[..])),
            ]
        } else {
            Vec::new()
        };
        Self {
            inner: ImmutableMemoryAssetIo::try_from_entries(assets).unwrap(),
            opens: Mutex::new(Vec::new()),
        }
    }

    fn opened_paths(&self) -> Vec<AssetPath> {
        self.opens.lock().unwrap().clone()
    }
}

impl AssetIo for LoggedIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.inner.authority_domain()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.opens.lock().unwrap().push(path.clone());
        self.inner.open(path)
    }
}

enum NestedResolver {
    Deny,
    Redirect(LoggedIo),
}

impl DependencyResolver for NestedResolver {
    fn authorize<'a>(
        &'a self,
        origin: &'a dyn AssetIo,
        origin_authority: &AuthorityDomain,
        _origin_path: &AssetPath,
        role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        if role.as_str() == "payload.required" {
            return match self {
                Self::Deny => Err(DependencyPolicyError::Denied {
                    path: reference.normalized().try_clone().unwrap(),
                }),
                Self::Redirect(target) => Ok(AuthorizedDependency::new(
                    target,
                    target.authority_domain(),
                    AssetPath::new("private/payload.bin").unwrap(),
                )),
            };
        }
        Ok(AuthorizedDependency::new(
            origin,
            origin_authority.clone(),
            reference.normalized().try_clone().unwrap(),
        ))
    }
}

fn role(value: &str) -> DependencyRoleId {
    DependencyRoleId::new(value).unwrap()
}

fn request(reference: &'static str, catch_failure: bool) -> PreparedImportRequest {
    let mut builder = RegistryBuilder::new();
    builder
        .register(NestedRequiredImporter {
            reference,
            catch_failure,
        })
        .unwrap();
    builder
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(
                    ImportConfigOverlay::default().with_loss_policy(
                        LossPolicy::preserve_or_fail()
                            .allow(ImportLossKind::OptionalDependencyUnavailable),
                    ),
                ),
            ),
            OptionCompileLimits::default(),
        )
        .unwrap()
}

fn assert_context(failure: &ImportFailure, origin_io: &LoggedIo, reference: &str) {
    assert_eq!(failure.source_identity().as_str(), SOURCE);
    let context = failure
        .cause()
        .dependency_context()
        .expect("dependency context");
    assert_eq!(context.origin().path().as_str(), ORIGIN);
    assert_eq!(context.origin().authority(), &origin_io.authority_domain());
    assert_eq!(context.role().as_str(), "payload.required");
    assert_eq!(context.requested_reference(), reference);
    assert_eq!(context.requested_reference_bytes(), reference.len());
    assert!(!context.requested_reference_is_truncated());
    assert!(failure.optional_dependency_evidence().is_empty());
    assert!(failure.loss_evidence().is_empty());
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    for dimension in [
        ImportDimension::ScratchBytes,
        ImportDimension::DependencyBytes,
    ] {
        assert_eq!(
            failure
                .ledger()
                .dimension(dimension)
                .unwrap()
                .current_live_bytes(),
            0
        );
    }
    let display = failure.to_string();
    assert!(display.contains(ORIGIN), "{display}");
    assert!(display.contains("payload.required"), "{display}");
    assert!(display.contains(reference), "{display}");
}

fn scope_cause(failure: &ImportFailure) -> &ScopeAbortCause {
    let ImportCause::Scope(abort) = failure.cause() else {
        panic!("expected the original dependency failure: {failure:?}");
    };
    assert!(std::ptr::eq(
        abort.cause(),
        abort
            .source()
            .unwrap()
            .downcast_ref::<ScopeAbortCause>()
            .unwrap()
    ));
    abort.cause()
}

#[test]
fn missing_nested_required_dependency_retains_origin_role_and_both_targets() {
    let io = LoggedIo::new(true);
    let failure = import_asset(
        &request(REFERENCE, false),
        &io,
        AssetPath::new(SOURCE).unwrap(),
        ImportOperationControl::default(),
    )
    .unwrap_err()
    .into_operation_failure()
    .unwrap();
    assert_context(&failure, &io, REFERENCE);
    let context = failure.cause().dependency_context().unwrap();
    assert_eq!(context.normalized_path().unwrap().as_str(), NORMALIZED);
    let target = context.target_identity().unwrap();
    assert_eq!(target.path().as_str(), NORMALIZED);
    assert_eq!(target.authority(), &io.authority_domain());
    assert_eq!(failure.cause().owner(), CauseOwner::Io);
    let ScopeAbortCause::AssetOpen(error) = scope_cause(&failure) else {
        panic!("expected the backend's open error");
    };
    assert_eq!(error.kind(), AssetOpenErrorKind::NotFound);
    assert!(
        scope_cause(&failure)
            .source()
            .unwrap()
            .is::<AssetOpenError>()
    );
    assert_eq!(
        io.opened_paths(),
        [SOURCE, ORIGIN, NORMALIZED].map(|path| AssetPath::new(path).unwrap())
    );
}

#[test]
fn denied_nested_required_dependency_remains_terminal_when_the_plugin_catches_it() {
    let io = LoggedIo::new(true);
    let failure = import_asset_with_resolver(
        &request(REFERENCE, true),
        &io,
        AssetPath::new(SOURCE).unwrap(),
        &NestedResolver::Deny,
        ImportOperationControl::default(),
    )
    .unwrap_err()
    .into_operation_failure()
    .unwrap();
    assert_context(&failure, &io, REFERENCE);
    let context = failure.cause().dependency_context().unwrap();
    assert_eq!(context.normalized_path().unwrap().as_str(), NORMALIZED);
    assert!(context.target_identity().is_none());
    assert_eq!(failure.cause().owner(), CauseOwner::DependencyPolicy);
    assert!(
        matches!(scope_cause(&failure), ScopeAbortCause::DependencyPolicy(
        DependencyPolicyError::Denied { path }
    ) if path.as_str() == NORMALIZED)
    );
    assert!(
        scope_cause(&failure)
            .source()
            .unwrap()
            .is::<DependencyPolicyError>()
    );
    assert_eq!(
        io.opened_paths(),
        [SOURCE, ORIGIN].map(|path| AssetPath::new(path).unwrap())
    );
}

#[test]
fn redirected_required_dependency_retains_the_authorized_backend_identity() {
    let io = LoggedIo::new(true);
    let resolver = NestedResolver::Redirect(LoggedIo::new(false));
    let failure = import_asset_with_resolver(
        &request(REFERENCE, false),
        &io,
        AssetPath::new(SOURCE).unwrap(),
        &resolver,
        ImportOperationControl::default(),
    )
    .unwrap_err()
    .into_operation_failure()
    .unwrap();
    assert_context(&failure, &io, REFERENCE);
    let context = failure.cause().dependency_context().unwrap();
    assert_eq!(context.normalized_path().unwrap().as_str(), NORMALIZED);
    let target = context.target_identity().unwrap();
    let NestedResolver::Redirect(target_io) = &resolver else {
        unreachable!()
    };
    assert_eq!(target.path().as_str(), "private/payload.bin");
    assert_eq!(target.authority(), &target_io.authority_domain());
    assert_ne!(target.authority(), context.origin().authority());
    assert_eq!(
        target_io.opened_paths(),
        [AssetPath::new("private/payload.bin").unwrap()]
    );
    assert_eq!(
        io.opened_paths(),
        [SOURCE, ORIGIN].map(|path| AssetPath::new(path).unwrap())
    );
    assert!(matches!(
        scope_cause(&failure),
        ScopeAbortCause::AssetOpen(_)
    ));
}

#[test]
fn rejected_nested_reference_keeps_its_spelling_and_path_rule_without_opening_it() {
    const ESCAPE: &str = "../../../outside.bin";
    let io = LoggedIo::new(true);
    let failure = import_asset(
        &request(ESCAPE, false),
        &io,
        AssetPath::new(SOURCE).unwrap(),
        ImportOperationControl::default(),
    )
    .unwrap_err()
    .into_operation_failure()
    .unwrap();
    assert_context(&failure, &io, ESCAPE);
    let context = failure.cause().dependency_context().unwrap();
    assert!(context.normalized_path().is_none());
    assert!(context.target_identity().is_none());
    assert!(matches!(
        scope_cause(&failure),
        ScopeAbortCause::Path(PathError::Escape)
    ));
    assert!(scope_cause(&failure).source().unwrap().is::<PathError>());
    assert_eq!(
        io.opened_paths(),
        [SOURCE, ORIGIN].map(|path| AssetPath::new(path).unwrap())
    );
}
