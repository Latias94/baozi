#![forbid(unsafe_code)]

mod detect;
mod parser;

use baozi_core::{Result, Scene};
use baozi_import::{DetectionOutcome, ImportContext, ImportOptions, ReadHint};
use baozi_io::{AssetIo, AssetPath};

/// Classifies bytes for the non-admitted development harness.
#[doc(hidden)]
pub fn detect_for_development(bytes: &[u8], source_len: u64, hint: &ReadHint) -> DetectionOutcome {
    detect::can_read(bytes, source_len, hint)
}

/// Runs the non-admitted parser through an existing test context.
#[doc(hidden)]
pub fn read_for_development(ctx: &mut ImportContext<'_>) -> Result<Scene> {
    parser::read_gltf(ctx)
}

/// Runs the non-admitted parser without registering it in a facade registry.
#[doc(hidden)]
pub fn read_asset_for_development(
    io: &dyn AssetIo,
    source: AssetPath,
    options: ImportOptions,
) -> Result<Scene> {
    let mut ctx = ImportContext::with_options(io, source, options);
    read_for_development(&mut ctx)
}
