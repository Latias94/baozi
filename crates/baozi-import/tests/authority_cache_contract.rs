mod support;

use baozi_core::FormatId;
use baozi_import::{
    AuthorizedDependency, CapabilityStatus, CauseOwner, DependencyCacheDisposition,
    DependencyPolicyError, DependencyReference, DependencyRequirement, DependencyResolver,
    DependencyRoleId, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, ImportCause, ImportConfigOverlay,
    ImportDimension, ImportFailure, ImportLedgerSnapshot, ImportLossKind, ImportOperationControl,
    ImportReport, ImportRequest, ImportRequestLayer, ImportResult, LossPolicy, OptionCompileLimits,
    ParseResult, ParseScope, PreparedImportRequest, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuilder, SceneDraft, ScopeAbortCause,
    import_asset_with_resolver as run_import_asset_with_resolver,
};
use baozi_io::{AssetIo, AssetOpenError, AssetPath, AuthorityDomain, MemoryAssetIo, OpenedAsset};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct CountingMemoryIo {
    inner: MemoryAssetIo,
    opens: AtomicUsize,
}

impl CountingMemoryIo {
    fn empty() -> Self {
        Self {
            inner: MemoryAssetIo::new(),
            opens: AtomicUsize::new(0),
        }
    }

    fn with_asset(path: AssetPath, bytes: &'static [u8]) -> Self {
        let backend = Self::empty();
        backend
            .inner
            .try_insert(path, Arc::from(bytes))
            .expect("test dependency insertion must succeed");
        backend
    }

    fn open_count(&self) -> usize {
        self.opens.load(Ordering::Acquire)
    }
}

impl AssetIo for CountingMemoryIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.inner.authority_domain()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.opens.fetch_add(1, Ordering::AcqRel);
        self.inner.open(path)
    }
}

struct ResolverRoute {
    role: DependencyRoleId,
    backend: usize,
}

struct RoutingResolver {
    backends: Vec<CountingMemoryIo>,
    routes: Vec<ResolverRoute>,
    denied_raw_reference: Option<&'static str>,
    calls: AtomicUsize,
}

impl RoutingResolver {
    fn new(
        backends: Vec<CountingMemoryIo>,
        routes: Vec<ResolverRoute>,
        denied_raw_reference: Option<&'static str>,
    ) -> Self {
        Self {
            backends,
            routes,
            denied_raw_reference,
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }

    fn backend_open_count(&self, index: usize) -> usize {
        self.backends
            .get(index)
            .expect("test resolver backend index must exist")
            .open_count()
    }
}

impl DependencyResolver for RoutingResolver {
    fn authorize<'a>(
        &'a self,
        _primary: &'a dyn AssetIo,
        _primary_authority: &AuthorityDomain,
        _primary_path: &AssetPath,
        role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let path = reference
            .normalized()
            .try_clone()
            .map_err(|_| DependencyPolicyError::AllocationFailed)?;
        if self.denied_raw_reference == Some(reference.raw()) {
            return Err(DependencyPolicyError::Denied { path });
        }

        let route = self.routes.iter().find(|route| &route.role == role).ok_or(
            DependencyPolicyError::ResolverContract {
                code: "test-role-not-routed",
            },
        )?;
        let backend =
            self.backends
                .get(route.backend)
                .ok_or(DependencyPolicyError::ResolverContract {
                    code: "test-backend-not-routed",
                })?;
        Ok(AuthorizedDependency::new(
            backend,
            backend.authority_domain(),
            path,
        ))
    }
}

struct AlternatingNamespaceResolver {
    backends: [CountingMemoryIo; 2],
    calls: AtomicUsize,
}

struct MisboundAuthorityResolver {
    backends: [CountingMemoryIo; 2],
    claimed_authority: AuthorityDomain,
    calls: AtomicUsize,
}

#[repr(C)]
struct AddressAliasedPrimaryIo {
    backend: CountingMemoryIo,
    authority: AuthorityDomain,
    primary_source: AssetPath,
}

impl AddressAliasedPrimaryIo {
    fn new(source: AssetPath, dependency: AssetPath) -> Self {
        Self {
            backend: CountingMemoryIo::with_asset(dependency, b"dependency"),
            authority: AuthorityDomain::new(),
            primary_source: source,
        }
    }
}

impl AssetIo for AddressAliasedPrimaryIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.authority.clone()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        if path == &self.primary_source {
            return Ok(OpenedAsset::new(
                self.authority.clone(),
                path.clone(),
                std::io::Cursor::new(b"primary".to_vec()),
                None,
            ));
        }
        self.backend.open(path)
    }
}

struct AddressAliasResolver<'backend> {
    backend: &'backend CountingMemoryIo,
    claimed_authority: AuthorityDomain,
}

impl DependencyResolver for AddressAliasResolver<'_> {
    fn authorize<'a>(
        &'a self,
        _primary: &'a dyn AssetIo,
        _primary_authority: &AuthorityDomain,
        _primary_path: &AssetPath,
        _role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        let path = reference
            .normalized()
            .try_clone()
            .map_err(|_| DependencyPolicyError::AllocationFailed)?;
        Ok(AuthorizedDependency::new(
            self.backend,
            self.claimed_authority.clone(),
            path,
        ))
    }
}

impl MisboundAuthorityResolver {
    fn new(first: CountingMemoryIo, second: CountingMemoryIo) -> Self {
        let claimed_authority = first.authority_domain();
        Self {
            backends: [first, second],
            claimed_authority,
            calls: AtomicUsize::new(0),
        }
    }

    fn backend_open_count(&self, index: usize) -> usize {
        self.backends
            .get(index)
            .expect("test resolver backend index must exist")
            .open_count()
    }
}

impl DependencyResolver for MisboundAuthorityResolver {
    fn authorize<'a>(
        &'a self,
        _primary: &'a dyn AssetIo,
        _primary_authority: &AuthorityDomain,
        _primary_path: &AssetPath,
        _role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        let index = self.calls.fetch_add(1, Ordering::AcqRel);
        let backend = self
            .backends
            .get(index)
            .ok_or(DependencyPolicyError::ResolverContract {
                code: "test-misbound-resolver-exhausted",
            })?;
        let path = reference
            .normalized()
            .try_clone()
            .map_err(|_| DependencyPolicyError::AllocationFailed)?;
        Ok(AuthorizedDependency::new(
            backend,
            self.claimed_authority.clone(),
            path,
        ))
    }
}

impl AlternatingNamespaceResolver {
    fn new(first: CountingMemoryIo, second: CountingMemoryIo) -> Self {
        Self {
            backends: [first, second],
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }

    fn backend_open_count(&self, index: usize) -> usize {
        self.backends
            .get(index)
            .expect("test resolver backend index must exist")
            .open_count()
    }
}

impl DependencyResolver for AlternatingNamespaceResolver {
    fn authorize<'a>(
        &'a self,
        _primary: &'a dyn AssetIo,
        _primary_authority: &AuthorityDomain,
        _primary_path: &AssetPath,
        _role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        let index = self.calls.fetch_add(1, Ordering::AcqRel);
        let backend = self
            .backends
            .get(index)
            .ok_or(DependencyPolicyError::ResolverContract {
                code: "test-alternating-resolver-exhausted",
            })?;
        let path = reference
            .normalized()
            .try_clone()
            .map_err(|_| DependencyPolicyError::AllocationFailed)?;
        Ok(AuthorizedDependency::new(
            backend,
            backend.authority_domain(),
            path,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotObservation {
    bytes: Vec<u8>,
    cache_disposition: DependencyCacheDisposition,
}

#[derive(Clone)]
struct DependencyStep {
    role: DependencyRoleId,
    requirement: DependencyRequirement,
    raw_reference: &'static str,
}

#[derive(Clone)]
struct DependencySequenceImporter {
    id: &'static str,
    extension: &'static str,
    roles: Vec<(DependencyRoleId, DependencyRequirement)>,
    steps: Vec<DependencyStep>,
    observations: Arc<Mutex<Vec<SnapshotObservation>>>,
}

impl DependencySequenceImporter {
    fn new(
        id: &'static str,
        extension: &'static str,
        roles: Vec<(DependencyRoleId, DependencyRequirement)>,
        steps: Vec<DependencyStep>,
    ) -> Self {
        Self {
            id,
            extension,
            roles,
            steps,
            observations: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn observation_log(&self) -> Arc<Mutex<Vec<SnapshotObservation>>> {
        Arc::clone(&self.observations)
    }
}

impl FormatImporter for DependencySequenceImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new(self.id).expect("test format id must be valid"),
            "Authority Cache Contract",
        )
        .extensions([self.extension])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .dependency_roles(self.roles.iter().cloned())
        .build()
        .expect("test descriptor must be valid")
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
        for step in &self.steps {
            let observation = match step.requirement {
                DependencyRequirement::Required => {
                    let snapshot =
                        support::required_root_dependency(scope, &step.role, step.raw_reference)?;
                    SnapshotObservation {
                        bytes: snapshot.bytes().to_vec(),
                        cache_disposition: snapshot.cache_disposition(),
                    }
                }
                DependencyRequirement::Optional => {
                    let outcome = support::root_dependency(scope, &step.role, step.raw_reference)?;
                    let Some(snapshot) = outcome.snapshot() else {
                        continue;
                    };
                    SnapshotObservation {
                        bytes: snapshot.bytes().to_vec(),
                        cache_disposition: snapshot.cache_disposition(),
                    }
                }
            };
            self.observations
                .lock()
                .expect("observation mutex must remain usable")
                .push(observation);
        }
        support::submit_minimal_scene(scope)
    }
}

fn role(value: &str) -> DependencyRoleId {
    DependencyRoleId::new(value).expect("test dependency role must be valid")
}

fn required(role: &DependencyRoleId, raw_reference: &'static str) -> DependencyStep {
    DependencyStep {
        role: role.clone(),
        requirement: DependencyRequirement::Required,
        raw_reference,
    }
}

fn optional(role: &DependencyRoleId, raw_reference: &'static str) -> DependencyStep {
    DependencyStep {
        role: role.clone(),
        requirement: DependencyRequirement::Optional,
        raw_reference,
    }
}

fn prepared_request(
    importer: impl FormatImporter,
    config: ImportConfigOverlay,
) -> PreparedImportRequest {
    let mut builder = RegistryBuilder::new();
    builder
        .register(importer)
        .expect("test importer registration must succeed");
    let snapshot = builder
        .freeze(RegistryBuildLimits::default())
        .expect("test registry freeze must succeed");
    snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(config),
            ),
            OptionCompileLimits::default(),
        )
        .expect("test request compilation must succeed")
}

fn primary_io(source: &AssetPath) -> MemoryAssetIo {
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::from(&b"primary"[..]))
        .expect("test primary insertion must succeed");
    io
}

#[allow(
    clippy::result_large_err,
    reason = "authority cache assertions inspect complete bounded import failure evidence"
)]
fn operation_result<T>(result: ImportResult<T>) -> Result<T, ImportFailure> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match error.into_operation_failure() {
            Ok(failure) => Err(failure),
            Err(source) => {
                panic!("import test setup failed before a session was created: {source}")
            }
        },
    }
}

#[allow(
    clippy::result_large_err,
    reason = "authority cache assertions inspect complete bounded import failure evidence"
)]
fn import_with_resolver(
    request: &PreparedImportRequest,
    io: &dyn AssetIo,
    source: AssetPath,
    resolver: &dyn DependencyResolver,
) -> Result<ImportReport, ImportFailure> {
    operation_result(run_import_asset_with_resolver(
        request,
        io,
        source,
        resolver,
        ImportOperationControl::default(),
    ))
}

fn observations(log: &Arc<Mutex<Vec<SnapshotObservation>>>) -> Vec<SnapshotObservation> {
    log.lock()
        .expect("observation mutex must remain usable")
        .clone()
}

fn assert_dimension(
    ledger: &ImportLedgerSnapshot,
    dimension: ImportDimension,
    attempted: u64,
    consumed: u64,
) {
    let evidence = ledger
        .dimension(dimension)
        .expect("test dimension must be present");
    assert_eq!(evidence.attempted(), attempted, "{dimension:?} attempted");
    assert_eq!(evidence.consumed(), consumed, "{dimension:?} consumed");
}

#[test]
fn same_logical_path_in_distinct_authority_domains_has_distinct_snapshots() {
    let source = AssetPath::new("models/asset.authority-domains").unwrap();
    let dependency_path = source.resolve_from_file("shared.bin").unwrap();
    let buffer_role = role("buffer.required");
    let importer = DependencySequenceImporter::new(
        "dev.baozi.test.authority-domains",
        "authority-domains",
        vec![(buffer_role.clone(), DependencyRequirement::Required)],
        vec![
            required(&buffer_role, "shared.bin"),
            required(&buffer_role, "shared.bin"),
        ],
    );
    let observation_log = importer.observation_log();
    let request = prepared_request(importer, ImportConfigOverlay::default());
    let resolver = AlternatingNamespaceResolver::new(
        CountingMemoryIo::with_asset(dependency_path.clone(), b"authority-a"),
        CountingMemoryIo::with_asset(dependency_path, b"authority-b"),
    );
    assert_ne!(
        resolver.backends[0].authority_domain(),
        resolver.backends[1].authority_domain(),
        "the fixture must exercise distinct authority domains"
    );
    let primary = primary_io(&source);

    let report = import_with_resolver(&request, &primary, source, &resolver)
        .expect("distinct authority domains must both resolve");

    assert_eq!(
        observations(&observation_log),
        [
            SnapshotObservation {
                bytes: b"authority-a".to_vec(),
                cache_disposition: DependencyCacheDisposition::Fresh,
            },
            SnapshotObservation {
                bytes: b"authority-b".to_vec(),
                cache_disposition: DependencyCacheDisposition::Fresh,
            },
        ]
    );
    assert_eq!(resolver.call_count(), 2);
    assert_eq!(resolver.backend_open_count(0), 1);
    assert_eq!(resolver.backend_open_count(1), 1);
    assert_dimension(report.ledger(), ImportDimension::DependencyReference, 2, 2);
    assert_dimension(report.ledger(), ImportDimension::AssetOpen, 3, 3);
    assert_dimension(report.ledger(), ImportDimension::DependencyBytes, 22, 22);
}

#[test]
fn cache_hit_rejects_backend_that_claims_another_authority_domain() {
    let source = AssetPath::new("models/asset.misbound-authority").unwrap();
    let dependency_path = source.resolve_from_file("shared.bin").unwrap();
    let buffer_role = role("buffer.required");
    let importer = DependencySequenceImporter::new(
        "dev.baozi.test.misbound-authority",
        "misbound-authority",
        vec![(buffer_role.clone(), DependencyRequirement::Required)],
        vec![
            required(&buffer_role, "shared.bin"),
            required(&buffer_role, "shared.bin"),
        ],
    );
    let observation_log = importer.observation_log();
    let request = prepared_request(importer, ImportConfigOverlay::default());
    let resolver = MisboundAuthorityResolver::new(
        CountingMemoryIo::with_asset(dependency_path.clone(), b"authority-a"),
        CountingMemoryIo::with_asset(dependency_path, b"authority-b"),
    );
    assert_ne!(
        resolver.backends[0].authority_domain(),
        resolver.backends[1].authority_domain(),
        "the fixture must exercise a backend-authority mismatch"
    );
    let primary = primary_io(&source);

    let failure = import_with_resolver(&request, &primary, source, &resolver)
        .expect_err("a cache hit must still validate the authorized backend authority");

    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::TrustedExtension {
                    boundary: baozi_import::TrustedExtensionBoundary::AssetIo,
                    code: "authorized-backend-authority-mismatch",
                }
            )
    ));
    assert_eq!(
        observations(&observation_log),
        [SnapshotObservation {
            bytes: b"authority-a".to_vec(),
            cache_disposition: DependencyCacheDisposition::Fresh,
        }]
    );
    assert_eq!(resolver.backend_open_count(0), 1);
    assert_eq!(
        resolver.backend_open_count(1),
        0,
        "the misbound backend must be rejected before open or cache reuse"
    );
}

#[test]
fn primary_identity_includes_the_asset_io_vtable() {
    let source = AssetPath::new("models/asset.address-alias").unwrap();
    let dependency_path = source.resolve_from_file("shared.bin").unwrap();
    let buffer_role = role("buffer.required");
    let importer = DependencySequenceImporter::new(
        "dev.baozi.test.address-alias",
        "address-alias",
        vec![(buffer_role.clone(), DependencyRequirement::Required)],
        vec![required(&buffer_role, "shared.bin")],
    );
    let request = prepared_request(importer, ImportConfigOverlay::default());
    let primary = AddressAliasedPrimaryIo::new(source.clone(), dependency_path);
    let primary_io: &dyn AssetIo = &primary;
    let aliased_backend_io: &dyn AssetIo = &primary.backend;
    assert!(
        std::ptr::addr_eq(primary_io, aliased_backend_io),
        "repr(C) fixture must share its data address with the first field"
    );
    assert!(
        !std::ptr::eq(primary_io, aliased_backend_io),
        "different AssetIo implementations must retain distinct vtable identity"
    );
    assert_ne!(
        primary.authority_domain(),
        primary.backend.authority_domain(),
        "the fixture must expose distinct authority domains"
    );
    let resolver = AddressAliasResolver {
        backend: &primary.backend,
        claimed_authority: primary.authority_domain(),
    };

    let failure = import_with_resolver(&request, &primary, source, &resolver)
        .expect_err("an address-alias backend must not inherit primary authority");

    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::TrustedExtension {
                    boundary: baozi_import::TrustedExtensionBoundary::AssetIo,
                    code: "authorized-backend-authority-mismatch",
                }
            )
    ));
    assert_eq!(
        primary.backend.open_count(),
        0,
        "authority mismatch must be rejected before dependency IO"
    );
}

#[test]
fn distinct_resolver_namespaces_do_not_share_snapshots_between_sessions() {
    let source = AssetPath::new("models/asset.resolver-namespaces").unwrap();
    let dependency_path = source.resolve_from_file("shared.bin").unwrap();
    let buffer_role = role("buffer.required");
    let importer = DependencySequenceImporter::new(
        "dev.baozi.test.resolver-namespaces",
        "resolver-namespaces",
        vec![(buffer_role.clone(), DependencyRequirement::Required)],
        vec![required(&buffer_role, "shared.bin")],
    );
    let observation_log = importer.observation_log();
    let request = prepared_request(importer, ImportConfigOverlay::default());
    let first_resolver = RoutingResolver::new(
        vec![CountingMemoryIo::with_asset(
            dependency_path.clone(),
            b"first-namespace",
        )],
        vec![ResolverRoute {
            role: buffer_role.clone(),
            backend: 0,
        }],
        None,
    );
    let second_resolver = RoutingResolver::new(
        vec![CountingMemoryIo::with_asset(
            dependency_path,
            b"second-namespace",
        )],
        vec![ResolverRoute {
            role: buffer_role,
            backend: 0,
        }],
        None,
    );
    assert_ne!(
        first_resolver.backends[0].authority_domain(),
        second_resolver.backends[0].authority_domain(),
        "the fixture must exercise distinct resolver namespaces"
    );
    let primary = primary_io(&source);

    let first_report = import_with_resolver(&request, &primary, source.clone(), &first_resolver)
        .expect("first resolver namespace must import");
    let second_report = import_with_resolver(&request, &primary, source, &second_resolver)
        .expect("second resolver namespace must import");

    assert_eq!(
        observations(&observation_log),
        [
            SnapshotObservation {
                bytes: b"first-namespace".to_vec(),
                cache_disposition: DependencyCacheDisposition::Fresh,
            },
            SnapshotObservation {
                bytes: b"second-namespace".to_vec(),
                cache_disposition: DependencyCacheDisposition::Fresh,
            },
        ]
    );
    assert_eq!(first_resolver.call_count(), 1);
    assert_eq!(second_resolver.call_count(), 1);
    assert_eq!(first_resolver.backend_open_count(0), 1);
    assert_eq!(second_resolver.backend_open_count(0), 1);
    assert_dimension(
        first_report.ledger(),
        ImportDimension::DependencyReference,
        1,
        1,
    );
    assert_dimension(
        second_report.ledger(),
        ImportDimension::DependencyReference,
        1,
        1,
    );
}

#[test]
fn denied_alias_is_authorized_before_an_allowed_cache_entry_can_be_reused() {
    let source = AssetPath::new("models/asset.denied-alias").unwrap();
    let dependency_path = source.resolve_from_file("shared.bin").unwrap();
    let buffer_role = role("material.optional");
    let importer = DependencySequenceImporter::new(
        "dev.baozi.test.denied-alias",
        "denied-alias",
        vec![(buffer_role.clone(), DependencyRequirement::Optional)],
        vec![
            optional(&buffer_role, "shared.bin"),
            optional(&buffer_role, "./shared.bin"),
        ],
    );
    let observation_log = importer.observation_log();
    let request = prepared_request(
        importer,
        ImportConfigOverlay::default().with_loss_policy(
            LossPolicy::preserve_or_fail().allow(ImportLossKind::OptionalDependencyUnavailable),
        ),
    );
    let resolver = RoutingResolver::new(
        vec![CountingMemoryIo::with_asset(
            dependency_path.clone(),
            b"allowed",
        )],
        vec![ResolverRoute {
            role: buffer_role,
            backend: 0,
        }],
        Some("./shared.bin"),
    );
    let primary = primary_io(&source);

    let report = import_with_resolver(&request, &primary, source, &resolver)
        .expect("an explicitly allowed optional loss must remain observable");

    assert_eq!(
        observations(&observation_log),
        [SnapshotObservation {
            bytes: b"allowed".to_vec(),
            cache_disposition: DependencyCacheDisposition::Fresh,
        }]
    );
    assert_eq!(report.optional_dependency_evidence().len(), 1);
    let denial = &report.optional_dependency_evidence()[0];
    assert!(denial.identity().is_none());
    assert_eq!(denial.normalized_path(), &dependency_path);
    assert_eq!(denial.owner(), &CauseOwner::DependencyPolicy);
    assert_eq!(
        denial.cache_disposition(),
        DependencyCacheDisposition::Fresh
    );
    assert_eq!(
        resolver.call_count(),
        2,
        "both aliases require authorization"
    );
    assert_eq!(resolver.backend_open_count(0), 1);
    assert_dimension(report.ledger(), ImportDimension::DependencyReference, 2, 2);
    assert_dimension(report.ledger(), ImportDimension::AssetOpen, 2, 2);
    assert_dimension(report.ledger(), ImportDimension::DependencyBytes, 7, 7);
    assert_eq!(report.ledger().current_live_bytes(), 0);
}

#[test]
fn requiredness_and_loss_disposition_share_one_physical_failure_but_bill_each_reference() {
    let source = AssetPath::new("models/asset.requiredness").unwrap();
    let dependency_path = source.resolve_from_file("shared.bin").unwrap();
    let optional_role = role("material.optional");
    let required_role = role("buffer.required");
    let importer = DependencySequenceImporter::new(
        "dev.baozi.test.requiredness",
        "requiredness",
        vec![
            (optional_role.clone(), DependencyRequirement::Optional),
            (required_role.clone(), DependencyRequirement::Required),
        ],
        vec![
            optional(&optional_role, "shared.bin"),
            required(&required_role, "./shared.bin"),
        ],
    );
    let request = prepared_request(
        importer,
        ImportConfigOverlay::default().with_loss_policy(
            LossPolicy::preserve_or_fail().allow(ImportLossKind::OptionalDependencyUnavailable),
        ),
    );
    let resolver = RoutingResolver::new(
        vec![CountingMemoryIo::empty()],
        vec![
            ResolverRoute {
                role: optional_role,
                backend: 0,
            },
            ResolverRoute {
                role: required_role,
                backend: 0,
            },
        ],
        None,
    );
    let primary = primary_io(&source);

    let failure = import_with_resolver(&request, &primary, source, &resolver)
        .expect_err("the required reference must promote the cached physical failure");

    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::AssetOpen(_))
    ));
    assert_eq!(
        resolver.call_count(),
        2,
        "each logical reference is authorized"
    );
    assert_eq!(
        resolver.backend_open_count(0),
        1,
        "requiredness and loss disposition must not split the physical cache key"
    );
    assert_eq!(failure.optional_dependency_evidence().len(), 1);
    let optional_failure = &failure.optional_dependency_evidence()[0];
    assert!(optional_failure.identity().is_some());
    assert_eq!(optional_failure.normalized_path(), &dependency_path);
    assert_eq!(optional_failure.owner(), &CauseOwner::Io);
    assert_eq!(
        optional_failure.cache_disposition(),
        DependencyCacheDisposition::Fresh
    );
    assert_dimension(failure.ledger(), ImportDimension::DependencyReference, 2, 2);
    assert_dimension(failure.ledger(), ImportDimension::AssetOpen, 2, 1);
    assert_dimension(failure.ledger(), ImportDimension::DependencyBytes, 0, 0);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}
