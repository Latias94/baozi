use crate::detect::{BINARY_PREAMBLE_BYTES, FACET_BYTES, HEADER_BYTES};
use crate::error::{StlFailure, StlFailureCode};
use crate::parser::{ParsedMesh, ParsedStl};
use baozi_import::authoring::Color;
use baozi_import::{
    ImportStorageClass, ImportWorkClass, MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS, ParseFailure,
    ParseResult, ParseRoot,
};

const MATERIALISE_COLOR_TOKEN: &[u8] = b"COLOR=";

pub(crate) fn parse<'session>(
    root: &mut ParseRoot<'_, '_, 'session>,
    bytes: &[u8],
    facets: u32,
) -> ParseResult<ParsedStl<'session>> {
    let face_count =
        usize::try_from(facets).map_err(|_| root.allocation_failure("stl-binary-face-count"))?;
    let record_bytes = face_count
        .checked_mul(FACET_BYTES)
        .ok_or_else(|| root.allocation_failure("stl-binary-size"))?;
    let expected = BINARY_PREAMBLE_BYTES
        .checked_add(record_bytes)
        .ok_or_else(|| root.allocation_failure("stl-binary-size"))?;
    if expected != bytes.len() {
        let expected = usize_amount(root, expected, "stl-expected-size")?;
        let actual = usize_amount(root, bytes.len(), "stl-actual-size")?;
        return Err(root.format_failure(
            StlFailure::content(
                StlFailureCode::BinarySizeMismatch,
                "facet-count",
                "declared-count-does-not-match-size",
            )
            .with_counts(expected, actual),
        ));
    }
    if facets == 0 {
        return Err(root.format_failure(StlFailure::content(
            StlFailureCode::EmptyGeometry,
            "binary-facets",
            "zero-facets",
        )));
    }
    let vertex_count = face_count
        .checked_mul(3)
        .ok_or_else(|| root.allocation_failure("stl-binary-vertex-count"))?;
    let Some(header) = bytes.get(..HEADER_BYTES) else {
        return Err(root.format_failure(StlFailure::content(
            StlFailureCode::InvalidStructure,
            "binary-header",
            "truncated",
        )));
    };
    let attribute_cell_count = vertex_count
        .checked_mul(2)
        .ok_or_else(|| root.allocation_failure("stl-binary-attribute-cells"))?;
    let decode_bytes = usize_amount(root, bytes.len(), "stl-decode-work")?;
    let preamble_bytes = usize_amount(root, BINARY_PREAMBLE_BYTES, "stl-binary-preamble")?;
    let facet_bytes = usize_amount(root, FACET_BYTES, "stl-binary-facet-bytes")?;
    let faces = usize_amount(root, face_count, "stl-binary-faces")?;
    let vertices = usize_amount(root, vertex_count, "stl-binary-vertices")?;
    let cells = usize_amount(root, attribute_cell_count, "stl-binary-attribute-cells")?;
    let records_count = usize_amount(root, face_count, "stl-binary-records")?;
    let decode_work = root.bind_group([
        ImportWorkClass::DecodeWork,
        ImportWorkClass::Faces,
        ImportWorkClass::Vertices,
        ImportWorkClass::AttributeCells,
        ImportWorkClass::Records,
    ])?;
    let (positions, normals, facet_colors, object_color, has_facet_colors, has_fallback_facets) =
        root.child(
            &decode_work,
            [decode_bytes, faces, vertices, cells, records_count],
            |child| -> ParseResult<_> {
                let object_color = child.run([preamble_bytes, 0, 0, 0, 0], || {
                    Ok(materialise_default_color(header))
                })?;
                let materialise = object_color.is_some();
                let (mut positions, mut normals, mut facet_colors) = {
                    let mut storage = child.storage()?;
                    let mut positions = storage.metered_vec(
                        ImportStorageClass::SceneConstruction,
                        "stl-binary-positions",
                    );
                    storage.reserve_vec(&mut positions, vertex_count)?;
                    let mut normals = storage
                        .metered_vec(ImportStorageClass::SceneConstruction, "stl-binary-normals");
                    storage.reserve_vec(&mut normals, vertex_count)?;
                    let mut facet_colors = storage.metered_vec(
                        ImportStorageClass::FormatNormalization,
                        "stl-binary-facet-colors",
                    );
                    storage.reserve_vec(&mut facet_colors, face_count)?;
                    (positions, normals, facet_colors)
                };
                let mut has_facet_colors = false;
                let mut has_fallback_facets = false;
                // The immutable exact-size proof above establishes exactly F complete 50-byte records.
                for record in bytes[BINARY_PREAMBLE_BYTES..].chunks_exact(FACET_BYTES) {
                    child.poll()?;
                    child.run([facet_bytes, 1, 3, 6, 1], || -> ParseResult<()> {
                        let record: &[u8; FACET_BYTES] =
                            record.try_into().expect("exact binary STL record size");
                        let facet = decode_facet(record, materialise);
                        positions.try_extend_from_slice_reserved(&facet.positions)?;
                        normals.try_extend_from_slice_reserved(&[facet.normal; 3])?;
                        has_facet_colors |= facet.color.is_some();
                        has_fallback_facets |= facet.color.is_none();
                        facet_colors.try_push_reserved(facet.color)
                    })?;
                    #[cfg(test)]
                    crate::control_tests::facet_completed(
                        crate::control_tests::CompletionSite::BinaryFacet,
                        facet_colors.len(),
                        positions.as_slice(),
                        normals.as_slice(),
                    );
                }
                Ok((
                    positions,
                    normals,
                    facet_colors,
                    object_color,
                    has_facet_colors,
                    has_fallback_facets,
                ))
            },
        )?;
    let colors = if has_facet_colors {
        let color_work =
            root.bind_group([ImportWorkClass::AttributeCells, ImportWorkClass::WorkUnits])?;
        let color_cells = usize_amount(root, vertex_count, "stl-binary-color-cells")?;
        let color_rows = usize_amount(root, face_count, "stl-binary-color-expansion")?;
        Some(root.child(
            &color_work,
            [color_cells, color_rows],
            |child| -> ParseResult<_> {
                let fallback = object_color.unwrap_or(Color::WHITE);
                let mut colors = {
                    let mut storage = child.storage()?;
                    let mut colors = storage
                        .metered_vec(ImportStorageClass::SceneConstruction, "stl-binary-colors");
                    storage.reserve_vec(&mut colors, vertex_count)?;
                    colors
                };
                for (index, color) in facet_colors.as_slice().iter().copied().enumerate() {
                    if index % MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS == 0 {
                        child.poll()?;
                    }
                    child.run([3, 1], || {
                        colors.try_extend_from_slice_reserved(
                            &[color_components(color.unwrap_or(fallback)); 3],
                        )
                    })?;
                    #[cfg(test)]
                    crate::control_tests::color_row_completed(colors.as_slice());
                }
                Ok(colors)
            },
        )?)
    } else {
        None
    };
    let name_value = "<STL_BINARY>";
    let name_byte_count = usize_amount(root, name_value.len(), "stl-binary-name")?;
    let mesh_work = root.bind_group([ImportWorkClass::Meshes, ImportWorkClass::StringBytes])?;
    let meshes = root.child(
        &mesh_work,
        [1, name_byte_count],
        |child| -> ParseResult<_> {
            let mut name = child
                .storage()?
                .metered_string(ImportStorageClass::SceneConstruction, "stl-binary-name");
            child.append_string([0, name_byte_count], &mut name, name_value)?;
            let mut meshes = child
                .storage()?
                .metered_vec(ImportStorageClass::FormatNormalization, "stl-binary-meshes");
            child.push_growing(
                [1, 0],
                &mut meshes,
                ParsedMesh {
                    name: Some(name),
                    positions,
                    normals,
                    colors,
                },
            )?;
            Ok(meshes)
        },
    )?;
    Ok(ParsedStl {
        material_color: object_color.filter(|_| has_fallback_facets),
        meshes,
    })
}

struct DecodedFacet {
    normal: [f32; 3],
    positions: [[f32; 3]; 3],
    color: Option<Color>,
}

fn decode_facet(record: &[u8; FACET_BYTES], materialise: bool) -> DecodedFacet {
    DecodedFacet {
        normal: vec3_at(record, 0),
        positions: [
            vec3_at(record, 12),
            vec3_at(record, 24),
            vec3_at(record, 36),
        ],
        color: facet_color(u16::from_le_bytes([record[48], record[49]]), materialise),
    }
}
fn vec3_at(record: &[u8; FACET_BYTES], offset: usize) -> [f32; 3] {
    [
        f32_at(record, offset),
        f32_at(record, offset + 4),
        f32_at(record, offset + 8),
    ]
}
fn f32_at(record: &[u8; FACET_BYTES], offset: usize) -> f32 {
    let mut bytes = [0; 4];
    bytes.copy_from_slice(&record[offset..offset + 4]);
    f32::from_le_bytes(bytes)
}
fn usize_amount(
    root: &ParseRoot<'_, '_, '_>,
    value: usize,
    resource: &'static str,
) -> Result<u64, ParseFailure> {
    u64::try_from(value).map_err(|_| root.allocation_failure(resource))
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

const fn color_components(value: Color) -> [f32; 4] {
    [value.r, value.g, value.b, value.a]
}
