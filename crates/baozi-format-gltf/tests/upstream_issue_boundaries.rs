mod common;

use baozi_core::{AttributeData, AttributeDomain, AttributeSemanticRef};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::ImportDimension;
use common::{
    TestResult, assert_gltf_failure, expected_error, glb_with_json, import_assets,
    import_assets_result, sidecar_options, triangle_bin, triangle_gltf, triangle_png,
};
use serde_json::{Value, json};

fn document_with_color() -> TestResult<Value> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    // Original positive control: source POSITION's zero/one F32 tuples are also legal RGB.
    let color = document["accessors"][0].clone();
    document["accessors"].as_array_mut().unwrap().push(color);
    document["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(4);
    Ok(document)
}

#[test]
fn valid_color_uv_and_index_shapes_reach_public_geometry_in_json_and_glb() -> TestResult<()> {
    let json = serde_json::to_vec(&document_with_color()?)?;
    for (source, bytes) in [
        ("models/issues.gltf", json.clone()),
        ("models/issues.glb", glb_with_json(&json)),
    ] {
        let (report, diagnostics) = import_assets(
            source,
            [
                (source, bytes),
                ("models/triangle.bin", triangle_bin()),
                ("models/textures/base.png", triangle_png()),
            ],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]));
        assert_eq!(primitive.vertex_count(), 3);
        let color = primitive
            .attribute(AttributeSemanticRef::Color(0), AttributeDomain::Vertex)
            .unwrap();
        let AttributeData::DenseF32 { lanes: 4, values } = color.data() else {
            panic!("expected RGB expanded with the defined alpha one")
        };
        assert_eq!(
            values,
            &[0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0]
        );
        let uv = primitive.texcoords(0, AttributeDomain::Vertex).unwrap();
        let AttributeData::DenseF32 { lanes: 2, values } = uv.data() else {
            panic!("expected authored F32 UV0")
        };
        assert_eq!(values, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert_eq!(report.ledger().current_live_bytes(), 0);
        assert_eq!(report.ledger().active_reservations(), 0);
    }
    Ok(())
}

#[test]
fn forbidden_reader_shapes_reject_before_missing_sidecars_can_mask_them() -> TestResult<()> {
    // Regression classes reported in gltf-rs #478; no upstream code or asset is copied.
    for (index, field, value, reason) in [
        (4, "componentType", json!(5120), "COLOR accessor type"),
        (2, "type", json!("VEC3"), "TEXCOORD accessor type"),
        (3, "componentType", json!(5126), "indices accessor type"),
    ] {
        let mut document = document_with_color()?;
        document["accessors"][index][field] = value;
        let json = serde_json::to_vec(&document)?;
        for (source, bytes) in [
            ("models/issues.gltf", json.clone()),
            ("models/issues.glb", glb_with_json(&json)),
        ] {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], sidecar_options())?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, Some(reason))?;
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
            assert_eq!(failure.ledger().committed_canonical_members(), 0);
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }
    Ok(())
}

#[test]
fn aligned_near_u64_max_accessor_offset_rejects_with_checked_range_arithmetic() -> TestResult<()> {
    // Regression class reported in gltf-rs #479. Alignment is valid, but offset + 36 overflows.
    let mut document = document_with_color()?;
    document["accessors"][0]["byteOffset"] = json!(u64::MAX - 3);
    let json = serde_json::to_vec(&document)?;
    for (source, bytes) in [
        ("models/issues.gltf", json.clone()),
        ("models/issues.glb", glb_with_json(&json)),
    ] {
        let (result, diagnostics) =
            import_assets_result(source, [(source, bytes)], sidecar_options())?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::InvalidDocument,
            Some(if usize::BITS == 64 {
                "POSITION accessor range overflow"
            } else {
                "glTF byte length, count or offset exceeds the host index domain"
            }),
        )?;
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
        assert_eq!(failure.ledger().committed_canonical_members(), 0);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}
