use crate::detect::{BINARY_PREAMBLE_BYTES, FACET_BYTES, HEADER_BYTES};
use crate::error::{StlFailure, StlFailureCode};
use crate::parser::{ParsedMesh, ParsedStl};
use baozi_import::authoring::{Color, Vec3};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS, MeteredVec,
    ParseExecution, ParseFailure, ParseResult,
};

const MATERIALISE_COLOR_TOKEN: &[u8] = b"COLOR=";

pub(crate) fn parse<'session>(
    exec: &mut ParseExecution<'_, 'session>,
    bytes: &[u8],
    facets: u32,
) -> ParseResult<ParsedStl<'session>> {
    let face_count =
        usize::try_from(facets).map_err(|_| exec.allocation_failure("stl-binary-face-count"))?;
    let record_bytes = face_count
        .checked_mul(FACET_BYTES)
        .ok_or_else(|| exec.allocation_failure("stl-binary-size"))?;
    let expected = BINARY_PREAMBLE_BYTES
        .checked_add(record_bytes)
        .ok_or_else(|| exec.allocation_failure("stl-binary-size"))?;
    if expected != bytes.len() {
        return Err(exec.format_failure(
            StlFailure::content(
                StlFailureCode::BinarySizeMismatch,
                "facet-count",
                "declared-count-does-not-match-size",
            )
            .with_counts(
                usize_amount(exec, expected, "stl-expected-size")?,
                usize_amount(exec, bytes.len(), "stl-actual-size")?,
            ),
        ));
    }
    if facets == 0 {
        return Err(exec.format_failure(StlFailure::content(
            StlFailureCode::EmptyGeometry,
            "binary-facets",
            "zero-facets",
        )));
    }

    let vertex_count = face_count
        .checked_mul(3)
        .ok_or_else(|| exec.allocation_failure("stl-binary-vertex-count"))?;
    let Some(header) = bytes.get(..HEADER_BYTES) else {
        return Err(exec.format_failure(StlFailure::content(
            StlFailureCode::InvalidStructure,
            "binary-header",
            "truncated",
        )));
    };
    let object_color = materialise_default_color(header);
    let materialise = object_color.is_some();
    let attribute_cell_count = vertex_count
        .checked_mul(2)
        .ok_or_else(|| exec.allocation_failure("stl-binary-attribute-cells"))?;
    let decode_bytes = usize_amount(exec, bytes.len(), "stl-decode-work")?;
    let preamble_bytes = usize_amount(exec, BINARY_PREAMBLE_BYTES, "stl-binary-preamble")?;
    let facet_bytes = usize_amount(exec, FACET_BYTES, "stl-binary-facet-bytes")?;
    let mut has_facet_colors = false;
    let mut has_fallback_facets = false;
    let decode_work = exec.bind_work_group([
        ImportWorkClass::DecodeWork,
        ImportWorkClass::Faces,
        ImportWorkClass::Vertices,
        ImportWorkClass::AttributeCells,
        ImportWorkClass::Records,
    ])?;
    let (positions, normals, facet_colors) = exec.with_work(
        &decode_work,
        [
            decode_bytes,
            usize_amount(exec, face_count, "stl-binary-faces")?,
            usize_amount(exec, vertex_count, "stl-binary-vertices")?,
            usize_amount(exec, attribute_cell_count, "stl-binary-attribute-cells")?,
            usize_amount(exec, face_count, "stl-binary-records")?,
        ],
        |exec, work| -> ParseResult<_> {
            work.complete([preamble_bytes, 0, 0, 0, 0])?;
            let mut positions = try_vec_capacity(
                exec,
                ImportStorageClass::SceneConstruction,
                vertex_count,
                "stl-binary-positions",
            )?;
            let mut normals = try_vec_capacity(
                exec,
                ImportStorageClass::SceneConstruction,
                vertex_count,
                "stl-binary-normals",
            )?;
            let mut facet_colors = try_vec_capacity(
                exec,
                ImportStorageClass::FormatNormalization,
                face_count,
                "stl-binary-facet-colors",
            )?;
            for face_index in 0..face_count {
                exec.poll()?;
                work.run([facet_bytes, 1, 3, 6, 1], || -> ParseResult<()> {
                    let offset =
                        BINARY_PREAMBLE_BYTES
                            .checked_add(face_index.checked_mul(FACET_BYTES).ok_or_else(|| {
                                exec.allocation_failure("stl-binary-record-offset")
                            })?)
                            .ok_or_else(|| exec.allocation_failure("stl-binary-record-offset"))?;
                    let end = offset
                        .checked_add(FACET_BYTES)
                        .ok_or_else(|| exec.allocation_failure("stl-binary-record-end"))?;
                    let Some(record) = bytes.get(offset..end) else {
                        return Err(exec.format_failure(
                            StlFailure::content(
                                StlFailureCode::InvalidStructure,
                                "binary-facet",
                                "truncated-record",
                            )
                            .at_byte(usize_amount(
                                exec,
                                offset,
                                "stl-binary-offset",
                            )?),
                        ));
                    };
                    let normal = vec3_at(record, 0, exec, offset)?;
                    let v0 = vec3_at(record, 12, exec, offset + 12)?;
                    let v1 = vec3_at(record, 24, exec, offset + 24)?;
                    let v2 = vec3_at(record, 36, exec, offset + 36)?;
                    positions.try_extend_from_slice_reserved(&[
                        vec3_components(v0),
                        vec3_components(v1),
                        vec3_components(v2),
                    ])?;
                    normals.try_extend_from_slice_reserved(&[vec3_components(normal); 3])?;

                    let Some(attribute_bytes) = record.get(48..50) else {
                        return Err(exec.format_failure(
                            StlFailure::content(
                                StlFailureCode::InvalidStructure,
                                "binary-facet-attribute",
                                "truncated",
                            )
                            .at_byte(usize_amount(
                                exec,
                                offset + 48,
                                "stl-binary-offset",
                            )?),
                        ));
                    };
                    let mut attribute = [0; 2];
                    attribute.copy_from_slice(attribute_bytes);
                    let color = facet_color(u16::from_le_bytes(attribute), materialise);
                    has_facet_colors |= color.is_some();
                    has_fallback_facets |= color.is_none();
                    facet_colors.try_push_reserved(color)?;
                    Ok(())
                })?;
                #[cfg(test)]
                crate::control_tests::facet_completed(
                    crate::control_tests::CompletionSite::BinaryFacet,
                    facet_colors.len(),
                    positions.as_slice(),
                    normals.as_slice(),
                );
            }
            Ok((positions, normals, facet_colors))
        },
    )?;

    let colors = if has_facet_colors {
        let color_work =
            exec.bind_work_group([ImportWorkClass::AttributeCells, ImportWorkClass::WorkUnits])?;
        Some(exec.with_work(
            &color_work,
            [
                usize_amount(exec, vertex_count, "stl-binary-color-cells")?,
                usize_amount(exec, face_count, "stl-binary-color-expansion")?,
            ],
            |exec, work| -> ParseResult<_> {
                let fallback = object_color.unwrap_or(Color::WHITE);
                let mut colors = try_vec_capacity(
                    exec,
                    ImportStorageClass::SceneConstruction,
                    vertex_count,
                    "stl-binary-colors",
                )?;
                for (index, color) in facet_colors.as_slice().iter().copied().enumerate() {
                    if index % MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS == 0 {
                        exec.poll()?;
                    }
                    work.run([3, 1], || {
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
    let name_byte_count = usize_amount(exec, name_value.len(), "stl-binary-name")?;
    let mesh_work =
        exec.bind_work_group([ImportWorkClass::Meshes, ImportWorkClass::StringBytes])?;
    let meshes = exec.with_work(
        &mesh_work,
        [1, name_byte_count],
        |exec, work| -> ParseResult<_> {
            let name = work.run([0, name_byte_count], || -> ParseResult<_> {
                let mut owned =
                    exec.metered_string(ImportStorageClass::SceneConstruction, "stl-binary-name");
                owned.try_push_str(exec, name_value)?;
                Ok(owned)
            })?;
            work.run([1, 0], || -> ParseResult<_> {
                let mut meshes = try_vec_capacity(
                    exec,
                    ImportStorageClass::FormatNormalization,
                    1,
                    "stl-binary-meshes",
                )?;
                meshes.try_push_reserved(ParsedMesh {
                    name: Some(name),
                    positions,
                    normals,
                    colors,
                })?;
                Ok(meshes)
            })
        },
    )?;
    Ok(ParsedStl {
        // An overridden object fallback must not tint facets or enable transparency.
        material_color: object_color.filter(|_| has_fallback_facets),
        meshes,
    })
}

fn vec3_at(
    record: &[u8],
    offset: usize,
    exec: &mut ParseExecution<'_, '_>,
    byte_offset: usize,
) -> ParseResult<Vec3> {
    Ok(Vec3::new(
        f32_at(record, offset, exec, byte_offset)?,
        f32_at(record, offset + 4, exec, byte_offset + 4)?,
        f32_at(record, offset + 8, exec, byte_offset + 8)?,
    ))
}

fn f32_at(
    record: &[u8],
    offset: usize,
    exec: &mut ParseExecution<'_, '_>,
    byte_offset: usize,
) -> ParseResult<f32> {
    let Some(chunk) = record.get(offset..offset + 4) else {
        return Err(exec.format_failure(
            StlFailure::content(
                StlFailureCode::InvalidStructure,
                "binary-float",
                "truncated",
            )
            .at_byte(usize_amount(exec, byte_offset, "stl-binary-offset")?),
        ));
    };
    let mut bytes = [0; 4];
    bytes.copy_from_slice(chunk);
    Ok(f32::from_le_bytes(bytes))
}

fn try_vec_capacity<'session, T>(
    exec: &mut ParseExecution<'_, 'session>,
    class: ImportStorageClass,
    capacity: usize,
    resource: &'static str,
) -> ParseResult<MeteredVec<'session, T>> {
    let mut values = exec.metered_vec(class, resource);
    values.try_reserve(exec, capacity)?;
    Ok(values)
}

fn usize_amount(
    exec: &ParseExecution<'_, '_>,
    value: usize,
    resource: &'static str,
) -> Result<u64, ParseFailure> {
    u64::try_from(value).map_err(|_| exec.allocation_failure(resource))
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

const fn vec3_components(value: Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

const fn color_components(value: Color) -> [f32; 4] {
    [value.r, value.g, value.b, value.a]
}
