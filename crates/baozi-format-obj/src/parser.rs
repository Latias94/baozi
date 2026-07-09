use crate::{mesh_builder, mtl, obj};
use baozi_core::{BaoziError, Result, Scene, SourceLocation};
use baozi_import::ImportContext;
use std::io::Read;

pub(crate) fn read_obj(ctx: &mut ImportContext<'_>) -> Result<Scene> {
    let bytes = read_primary_bytes(ctx)?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        BaoziError::parse(
            ctx.source.to_string(),
            Some(SourceLocation::byte(error.valid_up_to() as u64)),
            "OBJ is not valid UTF-8",
        )
    })?;
    let parsed = obj::parse(ctx, text)?;
    let materials = mtl::load_libraries(ctx, &parsed.mtllibs)?;

    mesh_builder::scene_from_parsed(ctx, parsed, &materials)
}

fn read_primary_bytes(ctx: &ImportContext<'_>) -> Result<Vec<u8>> {
    let limit = ctx.options.limits.max_primary_asset_bytes;
    let mut reader = ctx.io.open(&ctx.source)?;
    let mut bytes = Vec::new();
    let mut limited = reader.by_ref().take(limit.saturating_add(1));
    limited
        .read_to_end(&mut bytes)
        .map_err(|error| BaoziError::io(ctx.source.to_string(), error.to_string()))?;

    if bytes.len() as u64 > limit {
        return Err(BaoziError::LimitExceeded {
            limit: "max_primary_asset_bytes",
        });
    }

    Ok(bytes)
}
