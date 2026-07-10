mod common;

use baozi_core::{BaoziError, Result};
use baozi_import::ImportOptions;
use common::{ascii_triangle, expected_error, import_bytes_result};

#[test]
fn missing_end_header_is_parse_error() -> Result<()> {
    let bytes = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\n";
    let (result, diagnostics) =
        import_bytes_result("missing-header.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("end_header"));
    Ok(())
}

#[test]
fn face_index_out_of_range_is_parse_error() -> Result<()> {
    let bytes = std::str::from_utf8(ascii_triangle())
        .expect("fixture is utf-8")
        .replace("3 0 1 2", "3 0 1 3");
    let (result, diagnostics) =
        import_bytes_result("bad-index.ply", bytes.as_bytes(), ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("missing vertex"));
    Ok(())
}

#[test]
fn vertex_limit_is_enforced_from_header() -> Result<()> {
    let mut options = ImportOptions::memory();
    options.limits.max_vertices = 2;
    let (result, diagnostics) = import_bytes_result("vertex-limit.ply", ascii_triangle(), options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_vertices"
        }
    ));
    Ok(())
}

#[test]
fn unsupported_format_version_is_parse_error() -> Result<()> {
    let bytes = b"ply
format ascii 2.0
element vertex 1
property float x
property float y
property float z
end_header
0 0 0
";
    let (result, diagnostics) =
        import_bytes_result("version-2.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("unsupported PLY version"));
    Ok(())
}

#[test]
fn duplicate_vertex_element_is_parse_error() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
element vertex 1
property float x
property float y
property float z
end_header
0 0 0
1 1 1
";
    let (result, diagnostics) =
        import_bytes_result("duplicate-vertex.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("multiple PLY vertex elements"));
    Ok(())
}

#[test]
fn duplicate_face_element_is_parse_error() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
element face 1
property list uchar uint vertex_indices
end_header
0 0 0
1 0 0
0 1 0
3 0 1 2
3 0 1 2
";
    let (result, diagnostics) =
        import_bytes_result("duplicate-face.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("multiple PLY face elements"));
    Ok(())
}

#[test]
fn duplicate_face_topology_aliases_are_parse_error() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
property list uchar uint vertex_index
end_header
0 0 0
1 0 0
0 1 0
3 0 1 2 3 0 1 2
";
    let (result, diagnostics) =
        import_bytes_result("duplicate-topology.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("exactly one topology list"));
    Ok(())
}

#[test]
fn zero_count_face_with_topology_schema_preserves_point_cloud() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
element face 0
property list uchar uint vertex_indices
end_header
0 0 0
";
    let (scene, diagnostics) =
        import_bytes_result("zero-face.ply", bytes, ImportOptions::memory())?;
    let scene = scene?;

    assert!(diagnostics.is_empty());
    assert_eq!(
        scene.meshes[0].topology,
        baozi_core::PrimitiveTopology::Points
    );
    assert!(scene.meshes[0].indices.is_empty());
    Ok(())
}

#[test]
fn custom_attribute_cell_limit_is_enforced_before_body_read() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property float temperature
end_header
0 0 0 1
1 0 0 2
0 1 0 3
";
    let mut options = ImportOptions::memory();
    options.limits.max_total_vertex_attribute_cells = 5;
    let (result, diagnostics) = import_bytes_result("custom-cell-limit.ply", bytes, options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_total_vertex_attribute_cells"
        }
    ));
    Ok(())
}

#[test]
fn custom_attribute_stream_limit_is_enforced_before_allocation() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
property float temperature
property float density
end_header
0 0 0 1 2
";
    let mut options = ImportOptions::memory();
    options.limits.max_vertex_attribute_streams_per_mesh = 2;
    let (result, diagnostics) = import_bytes_result("custom-stream-limit.ply", bytes, options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_vertex_attribute_streams_per_mesh"
        }
    ));
    Ok(())
}

#[test]
fn custom_attribute_slots_preserve_schema_order_across_many_rows() -> Result<()> {
    const ROW_COUNT: usize = 64;

    let mut document = format!(
        "ply\nformat ascii 1.0\nelement vertex {ROW_COUNT}\n\
         property float x\nproperty float y\nproperty float z\n\
         property float temperature\nproperty int sample_id\nproperty uint flags\nend_header\n"
    );
    for row in 0..ROW_COUNT {
        let signed = -(row as i32);
        document.push_str(&format!("{row} 0 0 {} {signed} {row}\n", row as f32 + 0.5));
    }

    let (scene, diagnostics) = import_bytes_result(
        "custom-slots.ply",
        document.as_bytes(),
        ImportOptions::memory(),
    )?;
    let scene = scene?;
    let attributes = &scene.meshes[0].custom_attributes;

    assert!(diagnostics.is_empty());
    assert_eq!(
        attributes
            .iter()
            .map(|attribute| attribute.name.as_str())
            .collect::<Vec<_>>(),
        ["ply:temperature", "ply:sample_id", "ply:flags"]
    );
    let baozi_core::VertexAttributeData::F32(temperatures) = &attributes[0].data else {
        panic!("temperature must retain floating-point storage");
    };
    let baozi_core::VertexAttributeData::I32(sample_ids) = &attributes[1].data else {
        panic!("sample_id must retain signed integer storage");
    };
    let baozi_core::VertexAttributeData::U32(flags) = &attributes[2].data else {
        panic!("flags must retain unsigned integer storage");
    };
    assert_eq!(temperatures.len(), ROW_COUNT);
    assert_eq!(sample_ids.len(), ROW_COUNT);
    assert_eq!(flags.len(), ROW_COUNT);
    for row in 0..ROW_COUNT {
        assert_eq!(temperatures[row], row as f32 + 0.5);
        assert_eq!(sample_ids[row], -(row as i32));
        assert_eq!(flags[row], row as u32);
    }
    Ok(())
}

#[test]
fn topology_cell_limit_is_checked_before_face_indices_are_stored() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
end_header
0 0 0
1 0 0
0 1 0
3 0 1 2
";

    let mut exact = ImportOptions::memory();
    exact.limits.max_total_topology_cells = 3;
    let (result, diagnostics) = import_bytes_result("topology-exact.ply", bytes, exact)?;
    let scene = result?;
    assert!(diagnostics.is_empty());
    assert_eq!(scene.meshes[0].indices.len(), 3);

    let mut excess = ImportOptions::memory();
    excess.limits.max_total_topology_cells = 2;
    let (result, diagnostics) = import_bytes_result("topology-excess.ply", bytes, excess)?;
    let error = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_eq!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_total_topology_cells"
        }
    );
    Ok(())
}

#[test]
fn polygon_topology_limit_counts_indices_and_face_cardinality() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
end_header
0 0 0
1 0 0
1 1 0
0 1 0
4 0 1 2 3
";

    let mut exact = ImportOptions::memory();
    exact.limits.max_total_topology_cells = 5;
    let (result, diagnostics) = import_bytes_result("polygon-topology-exact.ply", bytes, exact)?;
    let scene = result?;
    assert!(diagnostics.is_empty());
    assert_eq!(scene.meshes[0].indices.len(), 4);
    assert_eq!(scene.meshes[0].face_vertex_counts, [4]);

    let malformed_excess = std::str::from_utf8(bytes)
        .expect("fixture is valid UTF-8")
        .replace("4 0 1 2 3", "4 invalid 1 2 3");
    let mut excess = ImportOptions::memory();
    excess.limits.max_total_topology_cells = 4;
    let (result, diagnostics) = import_bytes_result(
        "polygon-topology-excess.ply",
        malformed_excess.as_bytes(),
        excess,
    )?;
    let error = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_eq!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_total_topology_cells"
        }
    );
    Ok(())
}

#[test]
fn mixed_topology_limit_is_order_independent() -> Result<()> {
    let header = "ply\nformat ascii 1.0\nelement vertex 4\n\
                  property float x\nproperty float y\nproperty float z\n\
                  element face 2\nproperty list uchar uint vertex_indices\nend_header\n\
                  0 0 0\n1 0 0\n1 1 0\n0 1 0\n";
    for (label, first, second, invalid_second, expected_counts) in [
        (
            "triangle-quad",
            "3 0 1 2",
            "4 0 1 2 3",
            "4 invalid 1 2 3",
            [3, 4],
        ),
        (
            "quad-triangle",
            "4 0 1 2 3",
            "3 0 1 2",
            "3 invalid 1 2",
            [4, 3],
        ),
    ] {
        let document = format!("{header}{first}\n{second}\n");
        let mut exact = ImportOptions::memory();
        exact.limits.max_total_topology_cells = 9;
        let (scene, diagnostics) = import_bytes_result(
            &format!("mixed-{label}-exact.ply"),
            document.as_bytes(),
            exact,
        )?;
        let scene = scene?;
        assert!(diagnostics.is_empty());
        assert_eq!(scene.meshes[0].indices.len(), 7);
        assert_eq!(scene.meshes[0].face_vertex_counts, expected_counts);

        let malformed_excess = format!("{header}{first}\n{invalid_second}\n");
        let mut excess = ImportOptions::memory();
        excess.limits.max_total_topology_cells = 8;
        let (result, diagnostics) = import_bytes_result(
            &format!("mixed-{label}-excess.ply"),
            malformed_excess.as_bytes(),
            excess,
        )?;
        let error = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert_eq!(
            error,
            BaoziError::LimitExceeded {
                limit: "max_total_topology_cells"
            }
        );
    }
    Ok(())
}

#[test]
fn binary_polygon_topology_limit_precedes_item_reads() -> Result<()> {
    let mut exact_bytes = b"ply
format binary_little_endian 1.0
element vertex 4
property float x
property float y
property float z
element face 1
property list uchar uint vertex_indices
end_header
"
    .to_vec();
    for position in [
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ] {
        for value in position {
            exact_bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    exact_bytes.push(4);
    let topology_start = exact_bytes.len();
    for index in [0u32, 1, 2, 3] {
        exact_bytes.extend_from_slice(&index.to_le_bytes());
    }

    let mut exact = ImportOptions::memory();
    exact.limits.max_total_topology_cells = 5;
    let (scene, diagnostics) =
        import_bytes_result("binary-polygon-exact.ply", &exact_bytes, exact)?;
    let scene = scene?;
    assert!(diagnostics.is_empty());
    assert_eq!(scene.meshes[0].indices, [0, 1, 2, 3]);
    assert_eq!(scene.meshes[0].face_vertex_counts, [4]);

    let truncated_excess = &exact_bytes[..topology_start];
    let mut excess = ImportOptions::memory();
    excess.limits.max_total_topology_cells = 4;
    let (result, diagnostics) =
        import_bytes_result("binary-polygon-excess.ply", truncated_excess, excess)?;
    let error = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_eq!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_total_topology_cells"
        }
    );
    Ok(())
}

#[test]
fn binary_unknown_element_without_properties_is_rejected_before_rows() -> Result<()> {
    let mut bytes = b"ply
format binary_little_endian 1.0
element metadata 4294967295
element vertex 1
property float x
property float y
property float z
end_header
"
    .to_vec();
    for value in [0.0f32, 0.0, 0.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let (result, diagnostics) =
        import_bytes_result("zero-width-records.ply", &bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("has no properties"));
    Ok(())
}

#[test]
fn binary_element_records_are_preflighted_against_the_aggregate_payload() -> Result<()> {
    let bytes = b"ply
format binary_little_endian 1.0
element metadata 2
property uchar first
element metadata_extra 2
property uchar second
element vertex 1
property float x
property float y
property float z
end_header
\x01\x02\x03\x04
";

    let (result, diagnostics) =
        import_bytes_result("aggregate-records.ply", bytes, ImportOptions::memory())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("declared element records"));
    Ok(())
}

#[test]
fn aggregate_binary_record_limit_is_checked_before_body_iteration() -> Result<()> {
    let mut bytes = b"ply
format binary_little_endian 1.0
element metadata 2
property uchar first
element metadata_extra 1
property uchar second
element vertex 1
property float x
property float y
property float z
end_header
"
    .to_vec();
    bytes.extend_from_slice(&[1, 2, 3]);
    for value in [0.0f32, 0.0, 0.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let mut options = ImportOptions::memory();
    options.limits.max_records = 3;

    let (result, diagnostics) = import_bytes_result("record-limit.ply", &bytes, options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_records"
        }
    ));
    Ok(())
}

#[test]
fn aggregate_ascii_record_limit_is_checked_before_body_iteration() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element metadata 2
element vertex 1
property float x
property float y
property float z
end_header
";
    let mut options = ImportOptions::memory();
    options.limits.max_records = 2;

    let (result, diagnostics) = import_bytes_result("ascii-record-limit.ply", bytes, options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_records"
        }
    ));
    Ok(())
}

#[test]
fn exact_ascii_record_budget_allows_zero_property_rows_and_advances_lines() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element metadata 2
element vertex 1
property float x
property float y
property float z
end_header


1 2 3
";
    let mut options = ImportOptions::memory();
    options.limits.max_records = 3;

    let (scene, diagnostics) = import_bytes_result("ascii-record-exact.ply", bytes, options)?;
    let scene = scene?;

    assert_eq!(
        scene.meshes[0].positions,
        vec![baozi_core::Vec3::new(1.0, 2.0, 3.0)]
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.0 == "ply.element_ignored")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn exact_binary_record_budget_is_allowed() -> Result<()> {
    let mut bytes = b"ply
format binary_little_endian 1.0
element metadata 1
property uchar value
element vertex 1
property float x
property float y
property float z
end_header
"
    .to_vec();
    bytes.push(7);
    for value in [1.0f32, 2.0, 3.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let mut options = ImportOptions::memory();
    options.limits.max_records = 2;

    let (scene, diagnostics) = import_bytes_result("binary-record-exact.ply", &bytes, options)?;
    let scene = scene?;

    assert_eq!(
        scene.meshes[0].positions,
        vec![baozi_core::Vec3::new(1.0, 2.0, 3.0)]
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.0 == "ply.element_ignored")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn aggregate_record_count_overflow_is_reported_as_record_limit() -> Result<()> {
    let bytes = format!(
        "ply\nformat ascii 1.0\nelement metadata {}\nproperty uchar value\nelement metadata_extra 1\nproperty uchar value\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n",
        usize::MAX
    );
    let mut options = ImportOptions::memory();
    options.limits.max_records = usize::MAX;

    let (result, diagnostics) =
        import_bytes_result("record-overflow.ply", bytes.as_bytes(), options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error,
        BaoziError::LimitExceeded {
            limit: "max_records"
        }
    ));
    Ok(())
}

#[test]
fn zero_width_record_fuzz_corpus_terminates_with_expected_outcomes() -> Result<()> {
    let ascii = include_bytes!("../../../fuzz/corpus/ply_import/ascii_zero_width_records.ply");
    let (ascii_result, diagnostics) =
        import_bytes_result("ascii-zero-width.ply", ascii, ImportOptions::memory())?;
    let scene = ascii_result?;
    assert_eq!(scene.meshes[0].positions.len(), 1);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.0 == "ply.element_ignored")
            .count(),
        1
    );

    let binary = include_bytes!("../../../fuzz/corpus/ply_import/binary_zero_width_records.ply");
    let (binary_result, diagnostics) =
        import_bytes_result("binary-zero-width.ply", binary, ImportOptions::memory())?;
    let error = expected_error(binary_result)?;
    assert!(diagnostics.is_empty());
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("has no properties"));
    Ok(())
}

#[test]
fn ignored_binary_element_emits_one_diagnostic_per_element() -> Result<()> {
    let mut bytes = b"ply
format binary_little_endian 1.0
element metadata 2
property uchar value
element vertex 1
property float x
property float y
property float z
end_header
"
    .to_vec();
    bytes.extend_from_slice(&[1, 2]);
    for value in [0.0f32, 0.0, 0.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let (scene, diagnostics) =
        import_bytes_result("bounded-diagnostics.ply", &bytes, ImportOptions::memory())?;
    let scene = scene?;

    assert_eq!(scene.meshes[0].positions.len(), 1);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.0 == "ply.element_ignored")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn partial_normal_properties_emit_diagnostics_and_omit_channel() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property float nx
element face 1
property list uchar uint vertex_indices
end_header
0 0 0 0
1 0 0 0
0 1 0 0
3 0 1 2
";
    let (scene, diagnostics) =
        import_bytes_result("partial-normal.ply", bytes, ImportOptions::memory())?;
    let scene = scene?;

    assert!(scene.meshes[0].normals.is_empty());
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.0 == "ply.partial_normals_ignored")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn vertex_schema_warnings_are_emitted_once_instead_of_once_per_row() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property float nx
property float s
property uchar red
property uchar green
property list uchar int neighbors
end_header
0 0 0 1 0 255 128 0
1 0 0 1 0 255 128 0
0 1 0 1 0 255 128 0
";

    let (scene, diagnostics) =
        import_bytes_result("vertex-schema-warnings.ply", bytes, ImportOptions::memory())?;
    let scene = scene?;

    assert_eq!(scene.meshes[0].positions.len(), 3);
    assert!(scene.meshes[0].normals.is_empty());
    assert!(scene.meshes[0].texcoords.is_empty());
    assert!(scene.meshes[0].colors.is_empty());
    for code in [
        "ply.vertex_list_property_ignored",
        "ply.partial_normals_ignored",
        "ply.partial_texcoords_ignored",
        "ply.partial_colors_ignored",
    ] {
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code.0 == code)
                .count(),
            1,
            "diagnostic {code} must be emitted once per vertex schema"
        );
    }
    assert_eq!(diagnostics.len(), 4);
    Ok(())
}

#[test]
fn face_ignored_property_warnings_are_emitted_once_per_schema() -> Result<()> {
    let bytes = b"ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 2
property list uchar uint vertex_indices
property int material_id
property list uchar float texcoord
end_header
0 0 0
1 0 0
1 1 0
0 1 0
3 0 1 2 7 6 0 0 1 0 1 1
3 0 2 3 8 6 0 0 1 1 0 1
";

    let (scene, diagnostics) =
        import_bytes_result("face-schema-warnings.ply", bytes, ImportOptions::memory())?;
    let scene = scene?;

    assert_eq!(
        scene.meshes[0].topology,
        baozi_core::PrimitiveTopology::Triangles
    );
    assert_eq!(scene.meshes[0].indices.len(), 6);
    for code in [
        "ply.face_scalar_property_ignored",
        "ply.face_list_property_ignored",
    ] {
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code.0 == code)
                .count(),
            1,
            "diagnostic {code} must be emitted once per face schema"
        );
    }
    assert_eq!(diagnostics.len(), 2);
    Ok(())
}
