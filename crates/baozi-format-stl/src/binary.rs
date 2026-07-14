use crate::detect::{BINARY_PREAMBLE_BYTES, FACET_BYTES, HEADER_BYTES};
use crate::error::{StlFailure, StlFailureCode};
use crate::parser::{ParsedMesh, ParsedStl};
use baozi_core::{Color, Vec3};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, MeteredString, MeteredVec, ParseFailure, ParseResult,
    ParseScope,
};

const MATERIALISE_COLOR_TOKEN: &[u8] = b"COLOR=";

pub(crate) fn parse<'scope>(
    scope: &'scope ParseScope<'_>,
    bytes: &[u8],
    facets: u32,
) -> ParseResult<ParsedStl<'scope>> {
    let face_count =
        usize::try_from(facets).map_err(|_| scope.allocation_failure("stl-binary-face-count"))?;
    let record_bytes = face_count
        .checked_mul(FACET_BYTES)
        .ok_or_else(|| scope.allocation_failure("stl-binary-size"))?;
    let expected = BINARY_PREAMBLE_BYTES
        .checked_add(record_bytes)
        .ok_or_else(|| scope.allocation_failure("stl-binary-size"))?;
    if expected != bytes.len() {
        return Err(scope.format_failure(
            StlFailure::content(
                StlFailureCode::BinarySizeMismatch,
                "facet-count",
                "declared-count-does-not-match-size",
            )
            .with_counts(
                usize_amount(scope, expected, "stl-expected-size")?,
                usize_amount(scope, bytes.len(), "stl-actual-size")?,
            ),
        ));
    }
    if facets == 0 {
        return Err(scope.format_failure(StlFailure::content(
            StlFailureCode::EmptyGeometry,
            "binary-facets",
            "zero-facets",
        )));
    }

    let faces = scope.begin_work(
        ImportWorkClass::Faces,
        usize_amount(scope, face_count, "stl-binary-faces")?,
    )?;
    let vertex_count = face_count
        .checked_mul(3)
        .ok_or_else(|| scope.allocation_failure("stl-binary-vertex-count"))?;
    let vertices = scope.begin_work(
        ImportWorkClass::Vertices,
        usize_amount(scope, vertex_count, "stl-binary-vertices")?,
    )?;
    let Some(header) = bytes.get(..HEADER_BYTES) else {
        return Err(scope.format_failure(StlFailure::content(
            StlFailureCode::InvalidStructure,
            "binary-header",
            "truncated",
        )));
    };
    let material_color = materialise_default_color(header);
    let materialise = material_color.is_some();
    let attribute_cells = scope.begin_work(
        ImportWorkClass::AttributeCells,
        usize_amount(
            scope,
            vertex_count
                .checked_mul(2)
                .ok_or_else(|| scope.allocation_failure("stl-binary-attribute-cells"))?,
            "stl-binary-attribute-cells",
        )?,
    )?;
    let records = scope.begin_work(
        ImportWorkClass::Records,
        usize_amount(scope, face_count, "stl-binary-records")?,
    )?;

    let mut positions = try_vec_capacity(
        scope,
        ImportStorageClass::SceneConstruction,
        vertex_count,
        "stl-binary-positions",
    )?;
    let mut normals = try_vec_capacity(
        scope,
        ImportStorageClass::SceneConstruction,
        vertex_count,
        "stl-binary-normals",
    )?;
    let mut facet_colors = try_vec_capacity(
        scope,
        ImportStorageClass::FormatNormalization,
        face_count,
        "stl-binary-facet-colors",
    )?;
    let mut has_facet_colors = false;

    for face_index in 0..face_count {
        scope.poll()?;
        let offset = BINARY_PREAMBLE_BYTES
            .checked_add(
                face_index
                    .checked_mul(FACET_BYTES)
                    .ok_or_else(|| scope.allocation_failure("stl-binary-record-offset"))?,
            )
            .ok_or_else(|| scope.allocation_failure("stl-binary-record-offset"))?;
        let end = offset
            .checked_add(FACET_BYTES)
            .ok_or_else(|| scope.allocation_failure("stl-binary-record-end"))?;
        let Some(record) = bytes.get(offset..end) else {
            return Err(scope.format_failure(
                StlFailure::content(
                    StlFailureCode::InvalidStructure,
                    "binary-facet",
                    "truncated-record",
                )
                .at_byte(usize_amount(scope, offset, "stl-binary-offset")?),
            ));
        };
        let normal = vec3_at(record, 0, scope, offset)?;
        let v0 = vec3_at(record, 12, scope, offset + 12)?;
        let v1 = vec3_at(record, 24, scope, offset + 24)?;
        let v2 = vec3_at(record, 36, scope, offset + 36)?;
        positions.try_extend_from_slice(&[v0, v1, v2])?;
        normals.try_extend_from_slice(&[normal, normal, normal])?;

        let Some(attribute_bytes) = record.get(48..50) else {
            return Err(scope.format_failure(
                StlFailure::content(
                    StlFailureCode::InvalidStructure,
                    "binary-facet-attribute",
                    "truncated",
                )
                .at_byte(usize_amount(scope, offset + 48, "stl-binary-offset")?),
            ));
        };
        let mut attribute = [0; 2];
        attribute.copy_from_slice(attribute_bytes);
        let color = facet_color(u16::from_le_bytes(attribute), materialise);
        has_facet_colors |= color.is_some();
        facet_colors.try_push(color)?;
    }

    let colors = if has_facet_colors {
        let color_cells = scope.begin_work(
            ImportWorkClass::AttributeCells,
            usize_amount(scope, vertex_count, "stl-binary-color-cells")?,
        )?;
        let expansion = scope.begin_work(
            ImportWorkClass::WorkUnits,
            usize_amount(scope, face_count, "stl-binary-color-expansion")?,
        )?;
        let fallback = material_color.unwrap_or(Color::WHITE);
        let mut colors = try_vec_capacity(
            scope,
            ImportStorageClass::SceneConstruction,
            vertex_count,
            "stl-binary-colors",
        )?;
        for (index, color) in facet_colors.as_slice().iter().copied().enumerate() {
            if index % 4096 == 0 {
                scope.poll()?;
            }
            colors.try_extend_from_slice(&[color.unwrap_or(fallback); 3])?;
        }
        expansion.complete()?;
        color_cells.complete()?;
        Some(colors)
    } else {
        None
    };

    records.complete()?;
    attribute_cells.complete()?;
    vertices.complete()?;
    faces.complete()?;
    let mesh = scope.begin_work(ImportWorkClass::Meshes, 1)?;
    let name = owned_string(scope, "<STL_BINARY>", "stl-binary-name")?;
    let mut meshes = try_vec_capacity(
        scope,
        ImportStorageClass::FormatNormalization,
        1,
        "stl-binary-meshes",
    )?;
    meshes.try_push(ParsedMesh {
        name: Some(name),
        positions,
        normals,
        colors,
    })?;
    mesh.complete()?;
    Ok(ParsedStl {
        material_color,
        meshes,
    })
}

fn vec3_at(
    record: &[u8],
    offset: usize,
    scope: &ParseScope<'_>,
    byte_offset: usize,
) -> ParseResult<Vec3> {
    Ok(Vec3::new(
        f32_at(record, offset, scope, byte_offset)?,
        f32_at(record, offset + 4, scope, byte_offset + 4)?,
        f32_at(record, offset + 8, scope, byte_offset + 8)?,
    ))
}

fn f32_at(
    record: &[u8],
    offset: usize,
    scope: &ParseScope<'_>,
    byte_offset: usize,
) -> ParseResult<f32> {
    let Some(chunk) = record.get(offset..offset + 4) else {
        return Err(scope.format_failure(
            StlFailure::content(
                StlFailureCode::InvalidStructure,
                "binary-float",
                "truncated",
            )
            .at_byte(usize_amount(scope, byte_offset, "stl-binary-offset")?),
        ));
    };
    let mut bytes = [0; 4];
    bytes.copy_from_slice(chunk);
    Ok(f32::from_le_bytes(bytes))
}

fn try_vec_capacity<'scope, T>(
    scope: &'scope ParseScope<'_>,
    class: ImportStorageClass,
    capacity: usize,
    resource: &'static str,
) -> ParseResult<MeteredVec<'scope, T>> {
    let mut values = scope.metered_vec(class, resource);
    values.try_reserve(capacity)?;
    Ok(values)
}

fn owned_string<'scope>(
    scope: &'scope ParseScope<'_>,
    value: &str,
    resource: &'static str,
) -> ParseResult<MeteredString<'scope>> {
    let permit = scope.begin_work(
        ImportWorkClass::StringBytes,
        usize_amount(scope, value.len(), resource)?,
    )?;
    let mut owned = scope.metered_string(ImportStorageClass::SceneConstruction, resource);
    owned.try_push_str(value)?;
    permit.complete()?;
    Ok(owned)
}

fn usize_amount(
    scope: &ParseScope<'_>,
    value: usize,
    resource: &'static str,
) -> Result<u64, ParseFailure> {
    u64::try_from(value).map_err(|_| scope.allocation_failure(resource))
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
