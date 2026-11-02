mod common;

use baozi_core::{AttributeElement, TopologyKind};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::ImportDimension;
use base64::Engine as _;
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, assert_gltf_failure, expected_error,
    glb_with_json, import_assets, import_assets_result,
};

// Literal little-endian F32 rows: (0,0,0), (1,2,3), (4,-1,2).
const POSITIONS: [u8; 36] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 63, 0, 0, 0, 64, 0, 0, 64, 64, 0, 0, 128, 64, 0,
    0, 128, 191, 0, 0, 0, 64,
];

fn document(count: &str, reference: &str, mode: &str) -> String {
    format!(
        r#"{{
          "asset":{{"version":"2.0"}},
          "scene":0,"scenes":[{{"nodes":[{reference}]}}],
          "nodes":[{{}},{{}},{{}},{{"mesh":0}}],
          "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"mode":{mode}}}]}}],
          "buffers":[{{"byteLength":36}}],
          "bufferViews":[{{"buffer":0,"byteLength":36}}],
          "accessors":[{{"bufferView":0,"componentType":5126,"count":{count},
                         "type":"VEC3","min":[0,-1,0],"max":[4,2,3]}}]
        }}"#
    )
}

fn containers(json: &str) -> [(&'static str, Vec<u8>); 2] {
    let uri = format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(POSITIONS)
    );
    [
        (
            "integer-encoding.gltf",
            json.replace(
                r#""buffers":[{"byteLength":36}]"#,
                &format!(r#""buffers":[{{"byteLength":36,"uri":"{uri}"}}]"#),
            )
            .into_bytes(),
        ),
        (
            "integer-encoding.glb",
            append_glb_chunk(glb_with_json(json.as_bytes()), b"BIN\0", &POSITIONS),
        ),
    ]
}

fn assert_geometry(json: &str) -> TestResult<()> {
    for (source, bytes) in containers(json) {
        let (report, diagnostics) =
            import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty(), "{source}");
        assert_eq!(report.scene().mesh_count(), 1, "{source}");
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert_eq!(primitive.topology(), TopologyKind::LineStrip, "{source}");
        assert_eq!(primitive.vertex_count(), 3, "{source}");
        assert_eq!(primitive.corner_count(), 3, "{source}");
        assert_eq!(primitive.face_count(), 2, "{source}");
        let positions = primitive.positions().unwrap();
        for (vertex, expected) in [
            [0x00000000, 0x00000000, 0x00000000],
            [0x3f800000, 0x40000000, 0x40400000],
            [0x40800000, 0xbf800000, 0x40000000],
        ]
        .into_iter()
        .enumerate()
        {
            let Some(AttributeElement::F32(actual)) = positions.element(vertex) else {
                panic!("expected F32 POSITION at {vertex} in {source}");
            };
            assert_eq!(
                actual
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                expected,
                "{source} vertex {vertex}"
            );
        }
        assert_eq!(positions.element(3), None);
        assert_eq!(report.ledger().current_live_bytes(), 0);
        assert_eq!(report.ledger().active_reservations(), 0);
    }
    Ok(())
}

fn assert_rejected(json: &str, code: GltfFailureCode, reason: Option<&str>) -> TestResult<()> {
    for (source, bytes) in containers(json) {
        let (result, diagnostics) =
            import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty(), "{source}");
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, code, reason)?;
        assert!(failure.source_scope_record().is_none(), "{source}");
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0,
            "{source}"
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}

#[test]
fn decimal_and_exponent_counts_references_and_enums_preserve_geometry() -> TestResult<()> {
    assert_geometry(&document("3", "3", "3"))?;
    for token in ["3.0", "3e0"] {
        for (count, reference, mode) in [(token, "3", "3"), ("3", token, "3"), ("3", "3", token)] {
            assert_geometry(&document(count, reference, mode))?;
        }
    }
    Ok(())
}

#[test]
fn integer_component_and_reference_spellings_do_not_change_typed_positions() -> TestResult<()> {
    let json = document("3e0", "3.0", "3e0")
        .replace(r#""componentType":5126"#, r#""componentType":5126.0"#)
        .replace(r#""bufferView":0"#, r#""bufferView":-0e999"#)
        .replace(
            r#""buffer":0,"byteLength":36"#,
            r#""buffer":0.0,"byteLength":36e0"#,
        );
    assert_geometry(&json)
}

#[cfg(target_pointer_width = "64")]
#[test]
fn large_integral_exponents_in_unused_accessors_preserve_selected_geometry() -> TestResult<()> {
    for token in ["10000000000000000000", "1e19"] {
        // Unselected counts stay within the U64/64-bit host domain without allocating rows.
        let json = document("3", "3", "3").replace(
            r#""max":[4,2,3]}]"#,
            &format!(
                r#""max":[4,2,3]}},{{"componentType":5126,"count":{token},"type":"SCALAR"}}]"#
            ),
        );
        assert_geometry(&json)?;
    }
    Ok(())
}

#[test]
fn fractions_underflow_negative_values_and_overrange_tokens_fail_before_dependencies()
-> TestResult<()> {
    for token in [
        "3.0000000000000000001",
        "2.9999999999999999999",
        "12e-1",
        "1e-999",
        "1e-18446744073709551616",
        "-3.0",
        "-1e-999",
        "18446744073709551616",
        "18446744073709551616.0",
        "1e999",
        "1e18446744073709551616",
    ] {
        for (count, reference, mode) in [(token, "3", "3"), ("3", token, "3"), ("3", "3", token)] {
            assert_rejected(
                &document(count, reference, mode),
                GltfFailureCode::InvalidDocument,
                None,
            )?;
        }
    }
    Ok(())
}

#[test]
fn nonscalar_and_malformed_tokens_never_become_unsigned_defaults() -> TestResult<()> {
    for token in [
        "null", "\"3\"", "true", "false", "[]", "{}", "3.", "1e", "+1", "01",
    ] {
        for (count, reference, mode) in [(token, "3", "3"), ("3", token, "3"), ("3", "3", token)] {
            assert_rejected(
                &document(count, reference, mode),
                GltfFailureCode::InvalidDocument,
                None,
            )?;
        }
    }
    Ok(())
}

#[test]
fn exact_zero_offsets_require_a_real_view_and_preserve_presence() -> TestResult<()> {
    for zero in ["0.0", "-0", "-0.000", "0e999", "-0e-18446744073709551616"] {
        let json = document("3.0", "3e0", "3e0").replace(
            r#""bufferView":0,"componentType"#,
            &format!(r#""bufferView":0,"byteOffset":{zero},"componentType"#),
        );
        assert_geometry(&json)?;
        let no_view = json.replace(r#""bufferView":0,"#, "");
        assert_rejected(
            &no_view,
            GltfFailureCode::InvalidAccessor,
            Some("without bufferView must not define byteOffset"),
        )?;

        let view_offset = document("3", "3", "3").replace(
            r#""buffer":0,"byteLength"#,
            &format!(r#""buffer":0,"byteOffset":{zero},"byteLength"#),
        );
        assert_geometry(&view_offset)?;
    }
    Ok(())
}

#[test]
fn an_absent_view_and_offset_remain_valid_for_an_implicit_zero_accessor() -> TestResult<()> {
    let json = document("3.0", "3e0", "3e0")
        .replace(r#""bufferView":0,"#, "")
        .replace(
            r#""min":[0,-1,0],"max":[4,2,3]"#,
            r#""min":[0,0,0],"max":[0,0,0]"#,
        );
    for (source, bytes) in containers(&json) {
        let (report, diagnostics) =
            import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty());
        let mesh = report.scene().meshes().next().unwrap();
        let primitive = mesh.primitives().next().unwrap();
        assert_eq!(primitive.vertex_count(), 3);
        for vertex in 0..3 {
            assert_eq!(
                primitive.positions().unwrap().element(vertex),
                Some(AttributeElement::F32(&[0.0, 0.0, 0.0]))
            );
        }
        assert_eq!(report.ledger().current_live_bytes(), 0);
        assert_eq!(report.ledger().active_reservations(), 0);
    }
    Ok(())
}

#[test]
fn exact_zero_counts_still_fail_the_positive_accessor_count_constraint() -> TestResult<()> {
    for zero in ["0", "0.0", "-0", "0e999", "-0e-18446744073709551616"] {
        assert_rejected(
            &document(zero, "3", "3"),
            GltfFailureCode::InvalidAccessor,
            Some("count must be positive"),
        )?;
    }
    Ok(())
}

#[test]
fn null_offsets_and_nonnumeric_reference_zeros_are_rejected() -> TestResult<()> {
    for token in ["null", "\"0\"", "false", "[]", "{}"] {
        for json in [
            document("3", "3", "3").replace(
                r#""bufferView":0,"componentType"#,
                &format!(r#""bufferView":0,"byteOffset":{token},"componentType"#),
            ),
            document("3", "3", "3").replace(
                r#""buffer":0,"byteLength"#,
                &format!(r#""buffer":0,"byteOffset":{token},"byteLength"#),
            ),
            document("3", "3", "3")
                .replace(r#""bufferView":0"#, &format!(r#""bufferView":{token}"#)),
        ] {
            assert_rejected(&json, GltfFailureCode::InvalidDocument, None)?;
        }
    }
    Ok(())
}
