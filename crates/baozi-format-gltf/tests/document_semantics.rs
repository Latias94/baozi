mod common;

use baozi_core::{AttributeElement, TextureRole};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::{ImportDimension, ImportTerminalCause, ImportTerminalFailureClass};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, expected_error, glb_with_json, import_assets,
    import_assets_result,
};
use serde_json::{Value, json};

fn encodings(document: &Value) -> TestResult<[(&'static str, Vec<u8>); 2]> {
    let json = serde_json::to_vec(document)?;
    let glb = glb_with_json(&json);
    Ok([("semantics.gltf", json), ("semantics.glb", glb)])
}

fn point_document(selected: bool) -> Value {
    json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [if selected { json!({"nodes": [0]}) } else { json!({}) }],
        "nodes": [{"mesh": 0}],
        "buffers": [{
            "byteLength": 12,
            "uri": if selected {
                "data:application/octet-stream;base64,AAAAAAAAAAAAAAAA"
            } else {
                "never.bin"
            }
        }],
        "bufferViews": [{"buffer": 0, "byteLength": 12}],
        "accessors": [{
            "bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
            "min": [0, 0, 0], "max": [0, 0, 0]
        }],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}]
    })
}

#[test]
fn texture_info_additional_fields_stay_site_specific_when_selected_or_omitted() -> TestResult<()> {
    for selected in [false, true] {
        let mut document: Value = serde_json::from_slice(&common::data_uri_gltf())?;
        if !selected {
            document["scenes"][0]["nodes"] = json!([]);
        }
        document["images"] = json!([{
            "uri": if selected { "data:image/png;base64,AA==" } else { "never.png" }
        }]);
        document["textures"] = json!([{"source": 0}]);
        document["materials"][0] = json!({
            "pbrMetallicRoughness": {
                "baseColorTexture": {"index": 0, "scale": [2, 3], "strength": [4, 5]},
                "metallicRoughnessTexture": {"index": 0, "scale": [2, 3]}
            },
            "emissiveTexture": {"index": 0, "scale": [2, 3]},
            "normalTexture": {"index": 0, "scale": 0.4, "strength": [4, 5]},
            "occlusionTexture": {"index": 0, "strength": 0.625, "scale": [2, 3]}
        });
        for (source, bytes) in encodings(&document)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            if selected {
                let material = report.scene().materials().next().unwrap();
                assert_eq!(material.binding_count(), 5);
                for (binding, (role, strength)) in material.bindings().zip([
                    (TextureRole::BaseColor, 1.0_f32),
                    (TextureRole::MetallicRoughness, 1.0),
                    (TextureRole::Normal, 0.4),
                    (TextureRole::Occlusion, 0.625),
                    (TextureRole::Emissive, 1.0),
                ]) {
                    assert_eq!(binding.role(), role);
                    assert_eq!(binding.strength().get().to_bits(), strength.to_bits());
                }
            } else {
                assert_eq!(report.scene().material_count(), 0);
                assert_eq!(report.scene().image_count(), 0);
            }
            assert_eq!(
                report
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
            assert_eq!(report.ledger().current_live_bytes(), 0);
        }
    }
    Ok(())
}

fn sparse_document(selected: bool) -> Value {
    let mut document = point_document(selected);
    document["buffers"][0]["byteLength"] = json!(16);
    if selected {
        document["buffers"][0]["uri"] =
            json!("data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAAAAAA==");
    }
    document["bufferViews"] = json!([
        {"buffer": 0, "byteLength": 1},
        {"buffer": 0, "byteOffset": 4, "byteLength": 12}
    ]);
    document["accessors"][0]
        .as_object_mut()
        .unwrap()
        .remove("bufferView");
    document["accessors"][0]["sparse"] = json!({
        "count": 1,
        "indices": {"bufferView": 0, "componentType": 5121},
        "values": {"bufferView": 1}
    });
    document
}

#[test]
fn missing_sparse_offsets_default_to_zero_when_selected_or_omitted() -> TestResult<()> {
    for selected in [false, true] {
        for (source, bytes) in encodings(&sparse_document(selected))? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            if selected {
                let primitive = report
                    .scene()
                    .meshes()
                    .next()
                    .unwrap()
                    .primitives()
                    .next()
                    .unwrap();
                let positions = primitive.positions().unwrap();
                assert_eq!(positions.len(), 1);
                let Some(AttributeElement::F32(point)) = positions.element(0) else {
                    panic!("glTF positions must remain F32");
                };
                assert_eq!(point, &[0.0; 3]);
            } else {
                assert_eq!(report.scene().mesh_count(), 0);
            }
            assert_eq!(
                report
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
            assert_eq!(report.ledger().current_live_bytes(), 0);
        }
    }
    Ok(())
}

#[test]
fn explicit_null_sparse_offsets_reject_globally_before_dependencies() -> TestResult<()> {
    for selected in [false, true] {
        for owner in ["indices", "values"] {
            let mut document = sparse_document(selected);
            document["accessors"][0]["sparse"][owner]["byteOffset"] = Value::Null;
            for (source, bytes) in encodings(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(diagnostics.is_empty());
                let failure = expected_error(result)?;
                assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
                assert_eq!(
                    failure
                        .ledger()
                        .dimension(ImportDimension::DependencyReference)
                        .unwrap()
                        .attempted(),
                    0
                );
                assert_eq!(failure.ledger().current_live_bytes(), 0);
            }
        }
    }
    Ok(())
}

#[test]
fn empty_morph_targets_remain_valid_when_selected_or_omitted() -> TestResult<()> {
    for selected in [false, true] {
        let mut document = point_document(selected);
        document["meshes"][0]["primitives"][0]["targets"] = json!([{}, {}, {}, {}]);
        for (source, bytes) in encodings(&document)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            if selected {
                let mesh = report.scene().meshes().next().unwrap();
                assert_eq!(mesh.morph_target_count(), 4);
                let primitive = mesh.primitives().next().unwrap();
                let targets: Vec<_> = primitive.morph_targets().collect();
                assert_eq!(targets.len(), 4);
                assert!(targets.iter().all(|target| target.delta_count() == 0));
            } else {
                assert_eq!(report.scene().mesh_count(), 0);
            }
            assert_eq!(
                report
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
            assert_eq!(report.ledger().current_live_bytes(), 0);
        }
    }
    Ok(())
}

#[test]
fn known_null_morph_references_reject_globally_before_dependencies() -> TestResult<()> {
    for selected in [false, true] {
        for target in [
            json!({"POSITION": null}),
            json!({"NORMAL": null}),
            json!({"TANGENT": null}),
            json!({"POSITION": null, "NORMAL": null, "TANGENT": null}),
        ] {
            let mut document = point_document(selected);
            document["buffers"][0]["uri"] = json!("never.bin");
            document["meshes"][0]["primitives"][0]["targets"] = json!([target]);
            for (source, bytes) in encodings(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(diagnostics.is_empty());
                let failure = expected_error(result)?;
                assert_gltf_failure(
                    &failure,
                    GltfFailureCode::InvalidDocument,
                    Some("requires an unsigned integer"),
                )?;
                assert!(failure.source_scope_record().is_none());
                assert_eq!(failure.ledger().committed_canonical_members(), 0);
                for dimension in [
                    ImportDimension::DependencyReference,
                    ImportDimension::DependencyBytes,
                ] {
                    assert_eq!(
                        failure.ledger().dimension(dimension).unwrap().attempted(),
                        0
                    );
                }
                assert_eq!(failure.ledger().current_live_bytes(), 0);
                assert_eq!(failure.ledger().current_evidence_bytes(), 0);
                assert_eq!(failure.ledger().active_reservations(), 0);
                assert!(matches!(
                    failure.ledger().terminal_cause(),
                    ImportTerminalCause::Failure(terminal)
                        if terminal.class() == ImportTerminalFailureClass::Requested
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn unknown_morph_references_keep_global_indices_and_selected_semantic_rejection() -> TestResult<()>
{
    for selected in [false, true] {
        for reference in [Value::Null, json!(1), json!(0)] {
            let mut document = point_document(selected);
            document["meshes"][0]["primitives"][0]["targets"] = json!([{"_UNKNOWN": reference}]);
            for (source, bytes) in encodings(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(diagnostics.is_empty());
                if reference == json!(0) && !selected {
                    let report = result?;
                    assert_eq!(report.scene().mesh_count(), 0);
                    assert_eq!(
                        report
                            .ledger()
                            .dimension(ImportDimension::DependencyReference)
                            .unwrap()
                            .attempted(),
                        0
                    );
                    assert_eq!(report.ledger().current_live_bytes(), 0);
                } else {
                    let failure = expected_error(result)?;
                    let code = if reference == json!(0) {
                        GltfFailureCode::UnsupportedFeature
                    } else {
                        GltfFailureCode::InvalidDocument
                    };
                    assert_gltf_failure(&failure, code, None)?;
                    assert_eq!(
                        failure
                            .ledger()
                            .dimension(ImportDimension::DependencyReference)
                            .unwrap()
                            .attempted(),
                        0
                    );
                    assert_eq!(failure.ledger().current_live_bytes(), 0);
                }
            }
        }
    }
    Ok(())
}
