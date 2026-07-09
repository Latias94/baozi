use crate::{mesh_builder, mtl, obj};
use baozi_core::{BaoziError, Result, Scene, SourceLocation};
use baozi_import::ImportContext;

pub(crate) fn read_obj(ctx: &mut ImportContext<'_>) -> Result<Scene> {
    let bytes = ctx.read_primary_to_end()?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        BaoziError::parse(
            ctx.source().to_string(),
            Some(SourceLocation::byte(error.valid_up_to() as u64)),
            "OBJ is not valid UTF-8",
        )
    })?;
    let parsed = obj::parse(ctx, text)?;
    let materials = mtl::load_libraries(ctx, &parsed.mtllibs)?;

    mesh_builder::scene_from_parsed(ctx, parsed, &materials)
}
