use crate::GltfFailure;
use crate::json_syntax;
use baozi_import::{ImportWorkClass, ParseResult, ParseScope};

const GLB_MAGIC: &[u8; 4] = b"glTF";
const GLB_VERSION: u32 = 2;
const GLB_HEADER_BYTES: usize = 12;
const GLB_CHUNK_HEADER_BYTES: usize = 8;
const GLB_JSON_CHUNK: &[u8; 4] = b"JSON";
const GLB_BIN_CHUNK: &[u8; 4] = b"BIN\0";

#[derive(Clone, Copy)]
pub(crate) struct JsonSurface<'bytes> {
    document_bytes: &'bytes [u8],
    json_bytes: &'bytes [u8],
    json_raw_base: u64,
}

impl<'bytes> JsonSurface<'bytes> {
    pub(crate) const fn document_bytes(self) -> &'bytes [u8] {
        self.document_bytes
    }

    pub(crate) const fn json_bytes(self) -> &'bytes [u8] {
        self.json_bytes
    }

    pub(crate) const fn json_raw_base(self) -> u64 {
        self.json_raw_base
    }
}

pub(crate) fn admit<'bytes>(ctx: &'bytes ParseScope<'_>) -> ParseResult<JsonSurface<'bytes>> {
    let document_bytes = ctx.primary_bytes();
    let (json_bytes, json_raw_base, is_glb) = if document_bytes.starts_with(GLB_MAGIC) {
        let (json_bytes, json_raw_base) = locate_glb_json(ctx, document_bytes)?;
        (json_bytes, json_raw_base, true)
    } else {
        (document_bytes, 0, false)
    };
    let surface = JsonSurface {
        document_bytes,
        json_bytes,
        json_raw_base,
    };
    json_syntax::admit(ctx, surface.json_bytes(), surface.json_raw_base(), is_glb)?;
    Ok(surface)
}

fn locate_glb_json<'bytes>(
    ctx: &ParseScope<'_>,
    document_bytes: &'bytes [u8],
) -> ParseResult<(&'bytes [u8], u64)> {
    if document_bytes.len() < GLB_HEADER_BYTES || !document_bytes.len().is_multiple_of(4) {
        return Err(invalid_glb(
            ctx,
            "GLB header or total length is truncated or unaligned",
        ));
    }
    let version = read_u32(document_bytes, 4)
        .ok_or_else(|| invalid_glb(ctx, "GLB version field is truncated"))?;
    if version != GLB_VERSION {
        return Err(invalid_glb(ctx, "GLB version is not 2"));
    }
    let declared_length = usize::try_from(
        read_u32(document_bytes, 8)
            .ok_or_else(|| invalid_glb(ctx, "GLB declared length field is truncated"))?,
    )
    .map_err(|_| invalid_glb(ctx, "GLB declared length does not fit the host index type"))?;
    if declared_length != document_bytes.len() || declared_length < GLB_HEADER_BYTES {
        return Err(invalid_glb(
            ctx,
            "GLB declared length does not match the input",
        ));
    }

    if declared_length == GLB_HEADER_BYTES {
        return Err(invalid_glb(ctx, "GLB does not contain a JSON chunk"));
    }

    let mut cursor = GLB_HEADER_BYTES;
    let mut chunk_index = 0usize;
    let mut json_range = None;
    ctx.with_work(ImportWorkClass::WorkUnits, 1, |work| -> ParseResult<()> {
        while cursor < declared_length {
            let chunk = read_glb_chunk(ctx, document_bytes, cursor, declared_length)?;
            // Unknown chunk types are extension points. JSON stays first and BIN, when
            // present, stays second; every remaining header is still range-checked.
            if chunk_index == 0 {
                if chunk.chunk_type != GLB_JSON_CHUNK {
                    return Err(invalid_glb(ctx, "the first GLB chunk is not JSON"));
                }
                json_range = Some((chunk.data_start, chunk.end));
            } else if chunk.chunk_type == GLB_JSON_CHUNK {
                return Err(invalid_glb(ctx, "GLB contains more than one JSON chunk"));
            } else if chunk.chunk_type == GLB_BIN_CHUNK && chunk_index != 1 {
                return Err(invalid_glb(ctx, "the GLB BIN chunk is not second"));
            }

            cursor = chunk.end;
            chunk_index = chunk_index
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-glb-chunk-count"))?;
            work.complete([1])?;
            if cursor < declared_length {
                work.replenish(1)?;
            }
        }
        Ok(())
    })?;

    let (json_start, json_end) =
        json_range.ok_or_else(|| invalid_glb(ctx, "GLB does not contain a JSON chunk"))?;
    let json_bytes = document_bytes
        .get(json_start..json_end)
        .ok_or_else(|| invalid_glb(ctx, "GLB JSON range is outside the input"))?;
    let json_raw_base =
        u64::try_from(json_start).map_err(|_| ctx.allocation_failure("gltf-json-byte-offset"))?;
    Ok((json_bytes, json_raw_base))
}

struct GlbChunk<'bytes> {
    chunk_type: &'bytes [u8],
    data_start: usize,
    end: usize,
}

fn read_glb_chunk<'bytes>(
    ctx: &ParseScope<'_>,
    document_bytes: &'bytes [u8],
    cursor: usize,
    declared_length: usize,
) -> ParseResult<GlbChunk<'bytes>> {
    let chunk_header_end = cursor
        .checked_add(GLB_CHUNK_HEADER_BYTES)
        .ok_or_else(|| invalid_glb(ctx, "GLB chunk header offset overflowed"))?;
    if chunk_header_end > declared_length {
        return Err(invalid_glb(ctx, "GLB chunk header is truncated"));
    }
    let chunk_length = usize::try_from(
        read_u32(document_bytes, cursor)
            .ok_or_else(|| invalid_glb(ctx, "GLB chunk length field is truncated"))?,
    )
    .map_err(|_| invalid_glb(ctx, "GLB chunk length does not fit the host index type"))?;
    if !chunk_length.is_multiple_of(4) {
        return Err(invalid_glb(
            ctx,
            "GLB chunk length is not four-byte aligned",
        ));
    }
    let chunk_end = chunk_header_end
        .checked_add(chunk_length)
        .ok_or_else(|| invalid_glb(ctx, "GLB chunk end offset overflowed"))?;
    if chunk_end > declared_length {
        return Err(invalid_glb(
            ctx,
            "GLB chunk extends beyond the declared input",
        ));
    }
    let chunk_type_start = cursor
        .checked_add(4)
        .ok_or_else(|| invalid_glb(ctx, "GLB chunk type offset overflowed"))?;
    let chunk_type = document_bytes
        .get(chunk_type_start..chunk_header_end)
        .ok_or_else(|| invalid_glb(ctx, "GLB chunk type field is truncated"))?;
    Ok(GlbChunk {
        chunk_type,
        data_start: chunk_header_end,
        end: chunk_end,
    })
}

fn invalid_glb(ctx: &ParseScope<'_>, reason: &'static str) -> baozi_import::ParseFailure {
    ctx.format_failure(GltfFailure::invalid_document(reason))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let encoded: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(encoded))
}
