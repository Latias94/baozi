//! Actual session services shared by source-local execution tests.

use super::*;

struct FixtureFormat;

pub(crate) fn storage_observation<'region>(
    exec: &ParseExecution<'region, '_>,
    dimension: ImportDimension,
) -> baozi_operation::__retained_store_integration::authority::StoreStorageObservation<'region> {
    exec.store.storage_observation(dimension)
}

impl crate::Format for FixtureFormat {
    type Options = crate::NoFormatOptions;

    fn descriptor(&self) -> crate::FormatDescriptor {
        crate::FormatDescriptor::builder(
            baozi_core::FormatId::new("org.baozi.test.execution-storage").unwrap(),
            "Execution storage fixture",
        )
        .encoding(crate::FormatEncoding::Binary)
        .build()
        .unwrap()
    }

    fn compile_defaults(
        &self,
        scope: &mut crate::RegistryBuildScope<'_>,
    ) -> Result<crate::CompiledOptions<Self::Options>, crate::OptionCompileError> {
        crate::NoFormatOptions::compile_defaults(scope)
    }

    fn parse(&self, _: &Self::Options, _: &mut ParseContext<'_>) -> ParseResult<()> {
        unreachable!("storage tests invoke the real execution region directly")
    }
}

pub(crate) fn with_context<R>(
    owner: &mut ImportStoreSessionOwner,
    action: impl FnOnce(&mut ParseContext<'_>) -> R,
) -> R {
    let mut registry = crate::RegistryBuilder::new();
    registry.register(FixtureFormat).unwrap();
    let registry = registry
        .freeze(crate::RegistryBuildLimits::default())
        .unwrap();
    let entry = registry.entries().next().unwrap();
    let io = baozi_io::MemoryAssetIo::new();
    let authority = io.authority_domain();
    let source = AssetPath::new("fixture.bin").unwrap();
    let issuer = DependencyOriginIssuer::new();
    let limits = ResourceLimits::default();
    let diagnostics = RefCell::new(DiagnosticSink::new(16, 4096, 512, 8192));
    let graph = RefCell::new(ResourceGraph::new(limits.clone()));
    let optional_evidence = RefCell::new(AccountedVec::new());
    let loss_evidence = RefCell::new(AccountedVec::new());
    let source_scope = RefCell::new(None);
    let failure_storage = RefCell::new(FailureEvidenceStorage::default());
    let terminal = RefCell::new(None);
    let mut context = ParseContext::new(ParseContextParts {
        entry,
        primary_io: &io,
        primary_authority: &authority,
        primary_source: &source,
        primary_bytes: b"fixture",
        resolver: ResolverBinding::registered(
            baozi_io::__semantic_identity::same_authority_resolver_v1(),
        ),
        dependency_origin_issuer: &issuer,
        conformance_policy: ConformancePolicy::Strict,
        loss_policy: LossPolicy::preserve_or_fail(),
        limits: &limits,
        store_owner: owner,
        diagnostics: &diagnostics,
        resource_graph: &graph,
        optional_dependency_evidence: &optional_evidence,
        loss_evidence: &loss_evidence,
        source_scope_record: &source_scope,
        format_failure_storage: &failure_storage,
        terminal: &terminal,
    });
    action(&mut context)
}
