mod support;

use baozi::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetOpenWorkError,
    AssetPath, AssetReadError, AssetReader, AuthorityDomain, AuthorizedDependency,
    DependencyIdentity, DependencyPolicyError, DependencyReference, DependencyResolver,
    DependencyRoleId, DependencyRoleIdError, FormatId, FormatOptionEnvelope,
    FormatOptionEnvelopeError, FormatOptionSchemaId, FormatOptionSchemaIdError,
    FormatOptionSchemaKey, ImportRequest, ImportRequestLayer, Importer, OpenedAsset,
    OptionCompileLimits, OptionScopeLimits, RegistryBuildLimits,
};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const TRIANGLE: &[u8] = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
const MATERIAL_TRIANGLE: &[u8] =
    b"mtllib ./materials/triangle.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl red\nf 1 2 3\n";
const MATERIAL: &[u8] = b"newmtl red\nKd 1 0 0\n";

// Deliberately implements AssetReader directly, without the std::io::Read blanket impl.
struct ChunkedReader {
    remaining: &'static [u8],
    bytes_read: Arc<AtomicUsize>,
}

impl AssetReader for ChunkedReader {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        let count = buffer.len().min(self.remaining.len()).min(7);
        buffer[..count].copy_from_slice(&self.remaining[..count]);
        self.remaining = &self.remaining[count..];
        self.bytes_read.fetch_add(count, Ordering::Relaxed);
        Ok(count)
    }
}

struct CustomAssetIo {
    domain: AuthorityDomain,
    material: Option<&'static [u8]>,
    opens: AtomicUsize,
    bytes_read: Arc<AtomicUsize>,
}

impl CustomAssetIo {
    fn new() -> Self {
        Self {
            domain: AuthorityDomain::new(),
            material: Some(MATERIAL),
            opens: AtomicUsize::new(0),
            bytes_read: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl AssetIo for CustomAssetIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // Three literal comparisons, one path clone, and fixed counter/reader setup.
        Ok(NonZeroU64::new(4 * path.as_str().len() as u64 + 16).expect("bounded fixture work"))
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        self.opens.fetch_add(1, Ordering::Relaxed);
        let bytes = match path.as_str() {
            "triangle.obj" => TRIANGLE,
            "models/triangle.obj" => MATERIAL_TRIANGLE,
            "models/materials/triangle.mtl" => self.material.ok_or_else(|| {
                AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::NotFound)
            })?,
            _ => {
                return Err(AssetOpenError::new(
                    AssetOpenStage::Open,
                    AssetOpenErrorKind::NotFound,
                ));
            }
        };
        Ok(OpenedAsset::new(
            self.domain.clone(),
            path.clone(),
            ChunkedReader {
                remaining: bytes,
                bytes_read: Arc::clone(&self.bytes_read),
            },
            Some(AssetLength::Exact(bytes.len() as u64)),
        ))
    }
}

struct MaterialResolver {
    calls: AtomicUsize,
}

impl DependencyResolver for MaterialResolver {
    fn authorize<'a>(
        &'a self,
        origin: &'a dyn AssetIo,
        origin_authority: &AuthorityDomain,
        origin_path: &AssetPath,
        role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(*origin_authority, origin.authority_domain());
        assert_eq!(origin_path.as_str(), "models/triangle.obj");
        assert_eq!(role.as_str(), "material-library");
        assert_eq!(reference.raw(), "./materials/triangle.mtl");
        assert_eq!(
            reference.normalized().as_str(),
            "models/materials/triangle.mtl"
        );

        let path = reference
            .normalized()
            .try_clone()
            .map_err(|_| DependencyPolicyError::AllocationFailed)?;
        let authorized = AuthorizedDependency::new(origin, origin_authority.clone(), path);
        let identity: &DependencyIdentity = authorized.identity();
        assert_eq!(identity.path(), reference.normalized());
        Ok(authorized)
    }
}

#[test]
fn dependency_resolver_and_role_types_are_available_without_formats() {
    let _: &dyn DependencyResolver = &MaterialResolver {
        calls: AtomicUsize::new(0),
    };
    let role: Result<DependencyRoleId, DependencyRoleIdError> =
        DependencyRoleId::new("material-library");
    assert_eq!(role.expect("valid role").as_str(), "material-library");
    assert_eq!(DependencyRoleId::new(""), Err(DependencyRoleIdError::Empty));
    assert_eq!(
        DependencyRoleId::new("MaterialLibrary"),
        Err(DependencyRoleIdError::InvalidGrammar)
    );
    assert_eq!(
        DependencyRoleId::new(&"x".repeat(65)),
        Err(DependencyRoleIdError::TooLong)
    );
}

#[test]
fn format_option_envelope_can_be_constructed_from_facade_types() {
    let local: Result<FormatOptionSchemaId, FormatOptionSchemaIdError> =
        FormatOptionSchemaId::new("consumer-options");
    let schema = FormatOptionSchemaKey::new(
        FormatId::new("org.example.consumer").expect("valid format ID"),
        local.expect("valid schema ID"),
    );
    let version = NonZeroU32::new(1).expect("nonzero schema version");
    let envelope: Result<FormatOptionEnvelope, FormatOptionEnvelopeError> =
        FormatOptionEnvelope::new(schema.owner().clone(), schema.clone(), version, b"options");
    let layer = ImportRequestLayer::default()
        .try_with_format_option(envelope.expect("valid envelope"))
        .expect("one option fits the request layer");
    let request = ImportRequest::new().with_caller_layer(layer);
    let envelope = &request.caller().format_options()[0];

    assert_eq!(envelope.target(), schema.owner());
    assert_eq!(envelope.schema(), &schema);
    assert_eq!(envelope.schema().local().as_str(), "consumer-options");
    assert_eq!(envelope.version(), version);
    assert_eq!(envelope.payload_len(), 7);
    assert_eq!(
        FormatOptionSchemaId::new(""),
        Err(FormatOptionSchemaIdError::Empty)
    );
}

#[test]
fn registry_and_request_limits_can_be_constructed_from_facade_types() {
    let scope = OptionScopeLimits::new(1_000_000, 4096, 4096, 1000, 16, 4096, 4096);
    let registry_limits = RegistryBuildLimits::new(scope, 1_000_000, 4096, 4096);
    let importer = Importer::builder()
        .with_registry_build_limits(registry_limits)
        .build()
        .expect("empty registry accepts explicit codec limits");
    let request = ImportRequest::new();
    let compile_limits = OptionCompileLimits::new(scope, 4, 1_000_000, 4096, 4096, 4096);
    let prepared = importer
        .prepare_request(&request, compile_limits)
        .expect("empty request accepts explicit target limits");

    assert_eq!(prepared.overridden_formats().len(), 0);
}

#[test]
fn custom_reader_and_asset_io_use_only_facade_types() {
    let io = CustomAssetIo::new();
    let path = AssetPath::new("triangle.obj").expect("valid source path");
    let weight: NonZeroU64 = io
        .open_work_bound(&path)
        .expect("facade exposes work bounds");
    assert!(weight.get() >= path.as_str().len() as u64);
    assert_eq!(io.opens.load(Ordering::Relaxed), 0);
    assert_eq!(io.bytes_read.load(Ordering::Relaxed), 0);
    let mut opened = io.open(&path).expect("custom IO opens the triangle");
    assert!(opened.matches_identity(&io.authority_domain(), &path));
    assert_eq!(
        opened.length(),
        Some(AssetLength::Exact(TRIANGLE.len() as u64))
    );
    let mut bytes = Vec::new();
    let mut buffer = [0; 16];
    loop {
        let count = opened.read_chunk(&mut buffer).expect("custom reader chunk");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }

    assert_eq!(bytes, TRIANGLE);
    assert_eq!(opened.bytes_read(), TRIANGLE.len() as u64);
    assert_eq!(io.opens.load(Ordering::Relaxed), 1);
    assert_eq!(io.bytes_read.load(Ordering::Relaxed), TRIANGLE.len());
}

#[cfg(feature = "format-obj")]
#[test]
fn custom_asset_io_imports_bytes_on_the_test_stack() {
    let importer = Importer::new().expect("default formats register");
    let io = CustomAssetIo::new();
    let path = AssetPath::new("triangle.obj").expect("valid source path");
    let report = importer
        .read_asset(&io, path.clone())
        .expect("custom reader bytes import on the ordinary test stack");

    assert_eq!(report.source(), &path);
    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert_eq!(io.opens.load(Ordering::Relaxed), 1);
    assert_eq!(io.bytes_read.load(Ordering::Relaxed), TRIANGLE.len());
    let snapshot = report
        .scene()
        .capture_snapshot(
            baozi::SceneSnapshotLimits::try_new(16 * 1024, 64, 1_000_000)
                .expect("valid snapshot limits"),
        )
        .expect("the imported triangle is observable");
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 3);
}

#[test]
fn custom_dependency_resolver_acquires_a_required_dependency() {
    let importer = required_dependency_fixture::importer();
    let request = importer
        .prepare_default_request()
        .expect("default request compiles");
    let io = CustomAssetIo::new();
    let resolver = MaterialResolver {
        calls: AtomicUsize::new(0),
    };
    let report = importer
        .read_asset_with_resolver(
            &io,
            AssetPath::new("models/triangle.obj").expect("valid source path"),
            &request,
            &resolver,
            baozi::ImportOperationControl::default(),
        )
        .expect("the custom resolver authorizes the material library");

    assert_eq!(
        report.format().id().as_str(),
        "org.example.required-material"
    );
    assert!(report.optional_dependency_evidence().is_empty());
    assert!(report.loss_evidence().is_empty());
    assert!(report.diagnostics().is_empty());
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    assert_eq!(io.opens.load(Ordering::Relaxed), 2);
    assert_eq!(
        io.bytes_read.load(Ordering::Relaxed),
        MATERIAL_TRIANGLE.len() + MATERIAL.len()
    );
}

#[test]
fn missing_required_dependency_preserves_the_facade_io_error() {
    let importer = required_dependency_fixture::importer();
    let request = importer
        .prepare_default_request()
        .expect("default request compiles");
    let mut io = CustomAssetIo::new();
    io.material = None;
    let resolver = MaterialResolver {
        calls: AtomicUsize::new(0),
    };
    let error = importer
        .read_asset_with_resolver(
            &io,
            AssetPath::new("models/triangle.obj").expect("valid source path"),
            &request,
            &resolver,
            baozi::ImportOperationControl::default(),
        )
        .expect_err("a missing required dependency must reject the import");

    let baozi::Error::Import(failure) = error else {
        panic!("required dependency failure must retain import evidence");
    };
    assert!(matches!(
        failure.cause(),
        baozi::ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                baozi::ScopeAbortCause::AssetOpen(error)
                    if error.stage() == AssetOpenStage::Open
                        && error.kind() == AssetOpenErrorKind::NotFound
            )
    ));
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    assert_eq!(io.opens.load(Ordering::Relaxed), 2);
    assert_eq!(
        io.bytes_read.load(Ordering::Relaxed),
        MATERIAL_TRIANGLE.len()
    );
}

// Only this private parser fixture uses the deliberate plugin tier. The consumer tests and
// custom IO/resolver implementations above name their public signatures through baozi alone.
mod required_dependency_fixture {
    use baozi::{
        DependencyRequirement, DependencyRoleId, DetectionOutcome, FormatDescriptor,
        FormatEncoding, FormatId, FormatImporter, Importer, ReadConfidence,
    };
    use baozi_import::{
        DetectionResult, DetectionScope, ParseResult, ParseScope, Probe, ReadHint, SceneDraft,
    };

    struct RequiredMaterialImporter;

    impl FormatImporter for RequiredMaterialImporter {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("org.example.required-material").expect("valid fixture format ID"),
                "Required material fixture",
            )
            .extensions(["obj"])
            .encoding(FormatEncoding::Binary)
            .dependency_roles([(
                DependencyRoleId::new("material-library").expect("valid dependency role"),
                DependencyRequirement::Required,
            )])
            .build()
            .expect("valid fixture descriptor")
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
            {
                let root = scope.root_origin();
                let role =
                    DependencyRoleId::new("material-library").expect("valid dependency role");
                let outcome = scope.dependency(&root, &role, "./materials/triangle.mtl")?;
                assert_eq!(
                    outcome
                        .snapshot()
                        .expect("required dependency is available")
                        .bytes(),
                    super::MATERIAL
                );
            }
            super::support::submit_minimal_scene(scope)
        }
    }

    pub(super) fn importer() -> Importer {
        let mut builder = Importer::builder();
        builder
            .register(RequiredMaterialImporter)
            .expect("required-dependency fixture registers");
        builder.build().expect("fixture registry freezes")
    }
}
