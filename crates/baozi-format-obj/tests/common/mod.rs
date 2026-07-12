#![allow(dead_code)]

use baozi_core::{FormatId, ValidatedScene};
use baozi_format_obj::{FORMAT_ID, ObjImportOptions, ObjImporter};
use baozi_import::{
    ConformancePolicy, FormatSelection, ImportConfigOverlay, ImportFailure, ImportLossKind,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, ImportResult,
    LossPolicy, OptionCompileLimits, RegistryBuildLimits, RegistryBuilder, ResourceLimits,
    ResourceLimitsOverlay, import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::error::Error;
use std::sync::Arc;

pub type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub fn import_bytes(source: &str, bytes: &[u8]) -> TestResult<ImportReport> {
    import_bytes_result(source, bytes, ResourceLimits::default()).map(expected_success)
}

pub fn import_bytes_result(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
) -> TestResult<Result<ImportReport, ImportFailure>> {
    import_assets_result(source, [(source, bytes)], limits, false)
}

pub fn import_bytes_result_with_text_policy(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
    options: Option<ObjImportOptions>,
    conformance_policy: ConformancePolicy,
    allow_format_recovery: bool,
) -> TestResult<Result<ImportReport, ImportFailure>> {
    import_assets_result_with_text_policy(
        source,
        [(source, bytes)],
        limits,
        false,
        options,
        conformance_policy,
        allow_format_recovery,
    )
}

pub fn import_assets<'asset>(
    source: &str,
    assets: impl IntoIterator<Item = (&'asset str, &'asset [u8])>,
    limits: ResourceLimits,
) -> TestResult<ImportReport> {
    import_assets_result(source, assets, limits, true).map(expected_success)
}

pub fn import_assets_result<'asset>(
    source: &str,
    assets: impl IntoIterator<Item = (&'asset str, &'asset [u8])>,
    limits: ResourceLimits,
    allow_unavailable_dependencies: bool,
) -> TestResult<Result<ImportReport, ImportFailure>> {
    import_assets_result_with_control(
        source,
        assets,
        limits,
        allow_unavailable_dependencies,
        ImportOperationControl::default(),
    )
}

pub fn import_assets_result_with_control<'asset>(
    source: &str,
    assets: impl IntoIterator<Item = (&'asset str, &'asset [u8])>,
    limits: ResourceLimits,
    allow_unavailable_dependencies: bool,
    control: ImportOperationControl,
) -> TestResult<Result<ImportReport, ImportFailure>> {
    import_assets_result_with_text_policy_and_control(
        source,
        assets,
        limits,
        allow_unavailable_dependencies,
        None,
        ConformancePolicy::Strict,
        false,
        control,
    )
}

pub fn import_assets_result_with_text_policy<'asset>(
    source: &str,
    assets: impl IntoIterator<Item = (&'asset str, &'asset [u8])>,
    limits: ResourceLimits,
    allow_unavailable_dependencies: bool,
    options: Option<ObjImportOptions>,
    conformance_policy: ConformancePolicy,
    allow_format_recovery: bool,
) -> TestResult<Result<ImportReport, ImportFailure>> {
    import_assets_result_with_text_policy_and_control(
        source,
        assets,
        limits,
        allow_unavailable_dependencies,
        options,
        conformance_policy,
        allow_format_recovery,
        ImportOperationControl::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn import_assets_result_with_text_policy_and_control<'asset>(
    source: &str,
    assets: impl IntoIterator<Item = (&'asset str, &'asset [u8])>,
    limits: ResourceLimits,
    allow_unavailable_dependencies: bool,
    options: Option<ObjImportOptions>,
    conformance_policy: ConformancePolicy,
    allow_format_recovery: bool,
    control: ImportOperationControl,
) -> TestResult<Result<ImportReport, ImportFailure>> {
    let source_path = AssetPath::new(source)?;
    let io = MemoryAssetIo::new();
    for (path, bytes) in assets {
        io.try_insert(AssetPath::new(path)?, Arc::<[u8]>::from(bytes))?;
    }

    let mut registry = RegistryBuilder::new();
    registry.register(ObjImporter)?;
    let snapshot = registry.freeze(RegistryBuildLimits::default())?;
    let mut loss_policy = LossPolicy::preserve_or_fail();
    if allow_unavailable_dependencies {
        loss_policy = loss_policy.allow(ImportLossKind::OptionalDependencyUnavailable);
    }
    if allow_format_recovery {
        loss_policy = loss_policy.allow(ImportLossKind::FormatRecovery);
    }
    let config = ImportConfigOverlay::default()
        .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
        .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT_ID)?))
        .with_conformance_policy(conformance_policy)
        .with_loss_policy(loss_policy);
    let mut caller = ImportRequestLayer::new(config);
    if let Some(options) = options {
        caller = caller.try_with_format_option(options.to_envelope()?)?;
    }
    let request = ImportRequest::new().with_caller_layer(caller);
    let request = snapshot.compile_request(&request, OptionCompileLimits::default())?;

    operation_result(import_asset(&request, &io, source_path, control))
}

pub fn operation_result(
    result: ImportResult<ImportReport>,
) -> TestResult<Result<ImportReport, ImportFailure>> {
    match result {
        Ok(report) => Ok(Ok(report)),
        Err(error) => match error.into_operation_failure() {
            Ok(failure) => Ok(Err(failure)),
            Err(source) => Err(Box::new(source)),
        },
    }
}

pub fn scene(report: &ImportReport) -> &ValidatedScene {
    report.scene()
}

pub fn expected_success(result: Result<ImportReport, ImportFailure>) -> ImportReport {
    result.unwrap_or_else(|failure| panic!("expected OBJ import success, got {failure:?}"))
}

pub fn expected_failure(result: Result<ImportReport, ImportFailure>) -> ImportFailure {
    result.expect_err("expected OBJ import to fail")
}
