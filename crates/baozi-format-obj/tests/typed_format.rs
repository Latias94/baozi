mod common;

use baozi_format_obj::{FORMAT_ID, ObjImportOptions, ObjImporter, ObjTextDecodingMode};
use baozi_import::authoring::FormatId;
use baozi_import::{
    CompiledOptions, ConformancePolicy, DetectionOutcome, DetectionResult, DetectionScope, Format,
    FormatDescriptor, FormatOptionLayers, FormatOptionSchema, FormatSelection, ImportConfigOverlay,
    ImportLossKind, ImportOperationControl, ImportRequest, ImportRequestLayer, LossPolicy,
    OptionCompileError, OptionCompileLimits, OptionCompileScope, ParseContext, ParseResult, Probe,
    ReadHint, RegistryBuildLimits, RegistryBuildScope, RegistryBuilder,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use common::TestResult;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TypedObj {
    invocations: Arc<AtomicUsize>,
}

impl Format for TypedObj {
    type Options = ObjImportOptions;

    fn descriptor(&self) -> FormatDescriptor {
        ObjImporter.descriptor()
    }

    fn option_schema(&self) -> Option<FormatOptionSchema> {
        ObjImporter.option_schema()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        ObjImporter.compile_defaults(scope)
    }

    fn compile_options(
        &self,
        scope: &mut OptionCompileScope<'_>,
        defaults: &Self::Options,
        layers: FormatOptionLayers<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        ObjImporter.compile_options(scope, defaults, layers)
    }

    fn detect(
        &self,
        probe: &Probe<'_>,
        hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        ObjImporter.detect(probe, hint, scope)
    }

    fn parse(&self, options: &Self::Options, context: &mut ParseContext<'_>) -> ParseResult<()> {
        assert_eq!(
            options.text_decoding_mode(),
            ObjTextDecodingMode::ReplaceInvalidUtf8
        );
        self.invocations.fetch_add(1, Ordering::Relaxed);
        ObjImporter.parse(options, context)
    }
}

#[test]
fn typed_wrapper_and_cloned_plan_share_options_with_primary_and_mtl() -> TestResult<()> {
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut registry = RegistryBuilder::new();
    registry.register(TypedObj {
        invocations: Arc::clone(&invocations),
    })?;
    let registry = registry.freeze(RegistryBuildLimits::default())?;
    let options = ObjImportOptions::default()
        .with_text_decoding_mode(ObjTextDecodingMode::ReplaceInvalidUtf8);
    let config = ImportConfigOverlay::default()
        .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT_ID)?))
        .with_conformance_policy(ConformancePolicy::AllowDeclaredRecovery)
        .with_loss_policy(LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery));
    let layer = ImportRequestLayer::new(config).try_with_format_option(options.to_envelope()?)?;
    let request = ImportRequest::new().with_caller_layer(layer);
    let plan = registry.plan(&request, OptionCompileLimits::default())?;

    let primary = plan.read_bytes(
        AssetPath::new("primary.obj")?,
        b"o pri\xffmary\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        ImportOperationControl::default(),
    )?;
    assert_eq!(primary.loss_evidence().len(), 1);

    let source = AssetPath::new("sidecar.obj")?;
    let io = MemoryAssetIo::new();
    io.try_insert(
        source.clone(),
        Arc::<[u8]>::from(
            b"mtllib material.mtl\no mesh\nusemtl mat\xff\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n"
                .as_slice(),
        ),
    )?;
    io.try_insert(
        AssetPath::new("material.mtl")?,
        Arc::<[u8]>::from(b"newmtl mat\xff\nKd 0.2 0.3 0.4\n".as_slice()),
    )?;
    let sidecar = plan
        .clone()
        .read_asset(&io, source, ImportOperationControl::default())?;
    assert_eq!(sidecar.loss_evidence().len(), 2);
    assert!(
        !sidecar
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "obj.material_missing" })
    );
    assert_eq!(invocations.load(Ordering::Relaxed), 2);
    Ok(())
}
