use crate::detect::{BINARY_PREAMBLE_BYTES, FACET_BYTES, HEADER_BYTES};
use crate::parser::{ParsedMesh, ParsedStl};
use baozi_core::{BaoziError, Color, Result, SourceLocation, Vec3};
use baozi_import::ImportContext;

const MATERIALISE_COLOR_TOKEN: &[u8] = b"COLOR=";

pub(crate) fn parse(ctx: &mut ImportContext<'_>, bytes: &[u8], facets: u32) -> Result<ParsedStl> {
    let expected = BINARY_PREAMBLE_BYTES
        .checked_add(
            (facets as usize)
                .checked_mul(FACET_BYTES)
                .ok_or(BaoziError::LimitExceeded { limit: "max_faces" })?,
        )
        .ok_or(BaoziError::LimitExceeded { limit: "max_faces" })?;
    if expected != bytes.len() {
        return Err(BaoziError::parse(
            ctx.source.to_string(),
            None,
            "binary STL facet count does not match file size",
        ));
    }
    if facets == 0 {
        return Err(BaoziError::parse(
            ctx.source.to_string(),
            None,
            "binary STL contains no facets",
        ));
    }

    let face_count = facets as usize;
    if face_count > ctx.options.limits.max_faces {
        return Err(BaoziError::LimitExceeded { limit: "max_faces" });
    }
    let vertex_count = face_count.checked_mul(3).ok_or(BaoziError::LimitExceeded {
        limit: "max_vertices",
    })?;
    if vertex_count > ctx.options.limits.max_vertices {
        return Err(BaoziError::LimitExceeded {
            limit: "max_vertices",
        });
    }

    let Some(header) = bytes.get(..HEADER_BYTES) else {
        return Err(BaoziError::parse(
            ctx.source.to_string(),
            None,
            "binary STL is too small for header",
        ));
    };
    let material_color = materialise_default_color(header);
    let materialise = material_color.is_some();
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut facet_colors = Vec::with_capacity(face_count);

    for face_index in 0..face_count {
        let offset = BINARY_PREAMBLE_BYTES + face_index * FACET_BYTES;
        let Some(record) = bytes.get(offset..offset + FACET_BYTES) else {
            return Err(BaoziError::parse(
                ctx.source.to_string(),
                Some(SourceLocation::byte(offset as u64)),
                "binary STL ended inside a facet record",
            ));
        };
        let normal = vec3_at(record, 0, ctx, offset)?;
        let v0 = vec3_at(record, 12, ctx, offset + 12)?;
        let v1 = vec3_at(record, 24, ctx, offset + 24)?;
        let v2 = vec3_at(record, 36, ctx, offset + 36)?;
        positions.extend([v0, v1, v2]);
        normals.extend([normal, normal, normal]);

        let Some(attribute_bytes) = record.get(48..50) else {
            return Err(BaoziError::parse(
                ctx.source.to_string(),
                Some(SourceLocation::byte((offset + 48) as u64)),
                "binary STL ended inside a facet attribute",
            ));
        };
        let mut attribute = [0; 2];
        attribute.copy_from_slice(attribute_bytes);
        let attr = u16::from_le_bytes(attribute);
        facet_colors.push(facet_color(attr, materialise));
    }

    let colors = if facet_colors.iter().any(Option::is_some) {
        let fallback = material_color.unwrap_or(Color::WHITE);
        Some(
            facet_colors
                .into_iter()
                .flat_map(|color| [color.unwrap_or(fallback); 3])
                .collect(),
        )
    } else {
        None
    };

    Ok(ParsedStl {
        storage: "binary",
        material_color,
        meshes: vec![ParsedMesh {
            name: Some("<STL_BINARY>".to_owned()),
            positions,
            normals,
            colors,
        }],
    })
}

fn vec3_at(
    record: &[u8],
    offset: usize,
    ctx: &ImportContext<'_>,
    byte_offset: usize,
) -> Result<Vec3> {
    Ok(Vec3::new(
        f32_at(record, offset, ctx, byte_offset)?,
        f32_at(record, offset + 4, ctx, byte_offset + 4)?,
        f32_at(record, offset + 8, ctx, byte_offset + 8)?,
    ))
}

fn f32_at(
    record: &[u8],
    offset: usize,
    ctx: &ImportContext<'_>,
    byte_offset: usize,
) -> Result<f32> {
    let Some(chunk) = record.get(offset..offset + 4) else {
        return Err(BaoziError::parse(
            ctx.source.to_string(),
            Some(SourceLocation::byte(byte_offset as u64)),
            "binary STL ended inside a float",
        ));
    };
    let mut bytes = [0; 4];
    bytes.copy_from_slice(chunk);
    Ok(f32::from_le_bytes(bytes))
}

fn materialise_default_color(header: &[u8]) -> Option<Color> {
    header
        .windows(MATERIALISE_COLOR_TOKEN.len() + 4)
        .find(|window| window.starts_with(MATERIALISE_COLOR_TOKEN))
        .map(|window| {
            let mut rgba = [0; 4];
            rgba.copy_from_slice(
                &window[MATERIALISE_COLOR_TOKEN.len()..MATERIALISE_COLOR_TOKEN.len() + 4],
            );
            let [r, g, b, a] = rgba;
            Color::linear_rgba(byte_color(r), byte_color(g), byte_color(b), byte_color(a))
        })
}

fn facet_color(attribute: u16, materialise: bool) -> Option<Color> {
    if attribute & 0x8000 == 0 {
        return None;
    }

    let scale = 1.0 / 31.0;
    let low = (attribute & 0x1f) as f32 * scale;
    let mid = ((attribute >> 5) & 0x1f) as f32 * scale;
    let high = ((attribute >> 10) & 0x1f) as f32 * scale;

    if materialise {
        Some(Color::linear_rgba(low, mid, high, 1.0))
    } else {
        Some(Color::linear_rgba(high, mid, low, 1.0))
    }
}

fn byte_color(value: u8) -> f32 {
    value as f32 / 255.0
}
