#![allow(dead_code)]

use baozi_core::{BaoziError, Diagnostic, Result, Scene};
use baozi_format_obj::ObjImporter;
use baozi_import::{FormatImporter, ImportContext, ImportOptions};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::sync::Arc;

pub fn import_bytes(source: &str, bytes: &[u8]) -> Result<(Scene, Vec<Diagnostic>)> {
    let (scene, diagnostics) = import_bytes_result(source, bytes, ImportOptions::memory())?;
    Ok((scene?, diagnostics))
}

pub fn import_bytes_result(
    source: &str,
    bytes: &[u8],
    options: ImportOptions,
) -> Result<(Result<Scene>, Vec<Diagnostic>)> {
    let path = AssetPath::new(source)?;
    let mut io = MemoryAssetIo::new();
    io.insert(path.clone(), Arc::<[u8]>::from(bytes));
    let mut ctx = ImportContext::with_options(&io, path, options);
    let result = ObjImporter.read(&mut ctx);
    let diagnostics = ctx.into_diagnostics();
    Ok((result, diagnostics))
}

pub fn import_assets(
    source: &str,
    assets: impl IntoIterator<Item = (&'static str, &'static [u8])>,
    options: ImportOptions,
) -> Result<(Scene, Vec<Diagnostic>)> {
    let (scene, diagnostics) = import_assets_result(source, assets, options)?;
    Ok((scene?, diagnostics))
}

pub fn import_assets_result(
    source: &str,
    assets: impl IntoIterator<Item = (&'static str, &'static [u8])>,
    options: ImportOptions,
) -> Result<(Result<Scene>, Vec<Diagnostic>)> {
    let source_path = AssetPath::new(source)?;
    let mut io = MemoryAssetIo::new();
    for (path, bytes) in assets {
        io.insert(AssetPath::new(path)?, Arc::<[u8]>::from(bytes));
    }
    let mut ctx = ImportContext::with_options(&io, source_path, options);
    let result = ObjImporter.read(&mut ctx);
    let diagnostics = ctx.into_diagnostics();
    Ok((result, diagnostics))
}

pub fn expected_error(result: Result<Scene>) -> Result<BaoziError> {
    match result {
        Ok(_) => Err(BaoziError::parse(
            "test",
            None,
            "expected OBJ import to fail",
        )),
        Err(error) => Ok(error),
    }
}
