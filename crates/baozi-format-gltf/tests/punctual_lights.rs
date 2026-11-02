mod common;

use baozi_core::{LightKindCategory, LocalAxes, Vec3};
use baozi_import::{ImportDimension, ImportReport};
use common::{
    GltfTestOptions, TestResult, expected_error, glb_with_json, import_assets,
    import_assets_result, sidecar_options, triangle_bin, triangle_gltf, triangle_png,
};
use serde_json::{Value, json};

const SOURCE: &str = "models/lights.gltf";

fn light_document(light: &str) -> Vec<u8> {
    format!(
        r#"{{"asset":{{"version":"2.0"}},"extensionsUsed":["KHR_lights_punctual"],"extensions":{{"KHR_lights_punctual":{{"lights":[{light}]}}}},"nodes":[{{"extensions":{{"KHR_lights_punctual":{{"light":0}}}}}}]}}"#,
    )
    .into_bytes()
}

fn import_lights(bytes: Vec<u8>) -> TestResult<ImportReport> {
    let (report, diagnostics) =
        import_assets(SOURCE, [(SOURCE, bytes)], GltfTestOptions::memory())?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .unwrap()
            .attempted(),
        0,
    );
    Ok(report)
}

#[test]
fn light_only_defaults_and_node_attachments_in_json_and_glb() -> TestResult<()> {
    for (payload, category) in [
        (r#"{"type":"directional"}"#, LightKindCategory::Directional),
        (r#"{"type":"point"}"#, LightKindCategory::Point),
        (r#"{"type":"spot","spot":{}}"#, LightKindCategory::Spot),
    ] {
        let json = light_document(payload);
        for glb in [false, true] {
            let report = import_lights(if glb {
                glb_with_json(&json)
            } else {
                json.clone()
            })?;
            let scene = report.scene();
            assert_eq!(scene.lights().count(), 1);
            assert_eq!(scene.meshes().count(), 0);
            assert_eq!(scene.materials().count(), 0);
            let light = scene.lights().next().unwrap();
            let definition = light.definition();
            assert_eq!(definition.kind().category(), category);
            assert_eq!(definition.kind().range(), None);
            assert_eq!(definition.axes(), LocalAxes::STANDARD);
            assert_eq!(definition.tint(), Vec3::new(1.0, 1.0, 1.0));
            assert_eq!(definition.intensity(), 1.0);
            assert_eq!(
                scene
                    .nodes()
                    .filter(|node| node.light().map(|light| light.id()) == Some(light.id()))
                    .count(),
                1
            );
            if category == LightKindCategory::Spot {
                assert_eq!(definition.kind().inner_cone_radians().unwrap().to_bits(), 0);
                assert_eq!(
                    definition.kind().outer_cone_radians().unwrap().to_bits(),
                    0x3f49_0fdb
                );
            }
        }
    }
    Ok(())
}

#[test]
fn photometric_products_use_existing_checked_canonicalization() -> TestResult<()> {
    for (payload, tint, intensity) in [
        (
            r#"{"type":"point","color":[1,0,0],"intensity":10000,"range":8}"#,
            Vec3::new((10000.0_f64 / 2126.0) as f32, 0.0, 0.0),
            2126.0,
        ),
        (
            r#"{"type":"point","color":[0.5,0.5,0.5],"intensity":2}"#,
            Vec3::new(1.0, 1.0, 1.0),
            1.0,
        ),
        (
            r#"{"type":"point","color":[0,0,0],"intensity":100}"#,
            Vec3::new(1.0, 1.0, 1.0),
            0.0,
        ),
        (
            r#"{"type":"point","color":[1,0.5,0.25],"intensity":-0.0}"#,
            Vec3::new(1.0, 1.0, 1.0),
            0.0,
        ),
        (
            r#"{"type":"point","intensity":1.401298464324817e-45}"#,
            Vec3::new(1.0, 1.0, 1.0),
            f32::from_bits(1),
        ),
    ] {
        let report = import_lights(light_document(payload))?;
        let definition = report.scene().lights().next().unwrap().definition();
        assert_eq!(definition.tint(), tint, "{payload}");
        assert_eq!(
            definition.intensity().to_bits(),
            intensity.to_bits(),
            "{payload}"
        );
        assert_eq!(
            definition.kind().range(),
            payload.contains("range").then_some(8.0)
        );
    }
    Ok(())
}

#[test]
fn malformed_source_domains_and_cone_narrowing_fail_before_io() -> TestResult<()> {
    let cases = [
        r#"{}"#,
        r#"{"type":null}"#,
        r#"{"type":0}"#,
        r#"{"type":"point","name":null}"#,
        r#"{"type":"point","color":null}"#,
        r#"{"type":"point","color":[1,1]}"#,
        r#"{"type":"point","color":[1,1,"1"]}"#,
        r#"{"type":"point","color":[-1e-300,1,1]}"#,
        r#"{"type":"point","color":[-1e-999,1,1]}"#,
        r#"{"type":"point","color":[1.0000000001,1,1]}"#,
        r#"{"type":"point","color":[1e-300,1,1]}"#,
        r#"{"type":"point","intensity":null}"#,
        r#"{"type":"point","intensity":-1e-300}"#,
        r#"{"type":"point","intensity":-1e-999}"#,
        r#"{"type":"point","intensity":1e300}"#,
        r#"{"type":"point","intensity":1e999}"#,
        r#"{"type":"point","intensity":1e-300}"#,
        r#"{"type":"point","intensity":1e-999}"#,
        r#"{"type":"point","color":[1,0,0],"intensity":1.401298464324817e-45}"#,
        r#"{"type":"point","range":null}"#,
        r#"{"type":"point","range":0}"#,
        r#"{"type":"point","range":-1}"#,
        r#"{"type":"point","range":1e300}"#,
        r#"{"type":"point","range":1e-300}"#,
        r#"{"type":"directional","range":8}"#,
        r#"{"type":"spot"}"#,
        r#"{"type":"spot","spot":null}"#,
        r#"{"type":"spot","spot":{"innerConeAngle":null}}"#,
        r#"{"type":"spot","spot":{"outerConeAngle":null}}"#,
        r#"{"type":"spot","spot":{"innerConeAngle":0.5,"outerConeAngle":0.5}}"#,
        r#"{"type":"spot","spot":{"innerConeAngle":0.6,"outerConeAngle":0.5}}"#,
        r#"{"type":"spot","spot":{"outerConeAngle":0}}"#,
        r#"{"type":"spot","spot":{"innerConeAngle":-1e-300}}"#,
        r#"{"type":"spot","spot":{"innerConeAngle":-1e-999}}"#,
        r#"{"type":"spot","spot":{"outerConeAngle":1.5707964}}"#,
        r#"{"type":"spot","spot":{"innerConeAngle":0.5000000001,"outerConeAngle":0.5000000002}}"#,
    ];
    for payload in cases {
        let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
        document["extensionsUsed"] = json!(["KHR_lights_punctual"]);
        document["extensions"] = json!({"KHR_lights_punctual":{"lights":[{}]}});
        document["nodes"][0]["extensions"] = json!({"KHR_lights_punctual":{"light":0}});
        let bytes = serde_json::to_string(&document)?
            .replacen("\"lights\":[{}]", &format!("\"lights\":[{payload}]"), 1)
            .into_bytes();
        let (result, _) = import_assets_result(SOURCE, [(SOURCE, bytes)], sidecar_options())?;
        let failure = expected_error(result)?;
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0,
            "{payload}: {failure:?}"
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0, "{payload}");
    }
    Ok(())
}

#[test]
fn mathematical_outer_boundary_and_scaled_placement_keep_definition_parameters() -> TestResult<()> {
    let mut document: Value = serde_json::from_slice(&light_document(
        r#"{"type":"spot","range":8,"intensity":2,"spot":{"innerConeAngle":0.25,"outerConeAngle":1.5707963267948966}}"#,
    ))?;
    document["nodes"][0]["translation"] = json!([1, 2, 3]);
    document["nodes"][0]["scale"] = json!([4, 5, 6]);
    document["nodes"][0]["name"] = json!("Emitter");
    document["nodes"].as_array_mut().unwrap().push(json!({
        "translation":[10,20,30],"scale":[2,3,4],"children":[0]
    }));
    let report = import_lights(serde_json::to_vec(&document)?)?;
    let light = report.scene().lights().next().unwrap().definition();
    assert_eq!(light.kind().range(), Some(8.0));
    assert_eq!(light.intensity(), 2.0);
    assert_eq!(light.kind().inner_cone_radians(), Some(0.25));
    assert_eq!(
        light.kind().outer_cone_radians().unwrap().to_bits(),
        0x3fc9_0fdb
    );
    assert_eq!(light.axes(), LocalAxes::STANDARD);
    let emitter = report
        .scene()
        .nodes()
        .find(|node| node.name() == Some("Emitter"))
        .unwrap();
    assert!(emitter.light().is_some());
    let trs = emitter.authored_transform().authored_trs().unwrap();
    assert_eq!(trs.translation(), Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(trs.scale(), Vec3::new(4.0, 5.0, 6.0));
    assert_eq!(report.scene().nodes().count(), 2);
    assert_eq!(report.scene().space().unit_scale_to_meters, Some(1.0));
    Ok(())
}

#[test]
fn geometry_camera_and_light_share_one_node_without_changing_material_output() -> TestResult<()> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    document["extensionsUsed"] = json!(["KHR_lights_punctual"]);
    document["extensionsRequired"] = json!(["KHR_lights_punctual"]);
    document["extensions"] = json!({"KHR_lights_punctual":{"lights":[{"type":"point"}]}});
    document["nodes"][1]["extensions"] = json!({"KHR_lights_punctual":{"light":0}});
    document["nodes"][1]["camera"] = json!(0);
    document["cameras"] = json!([{"type":"perspective","perspective":{"yfov":1,"znear":0.1}}]);
    let (report, diagnostics) = import_assets(
        SOURCE,
        [
            (SOURCE, serde_json::to_vec(&document)?),
            ("models/triangle.bin", triangle_bin()),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    // triangle_gltf has Root and TriangleNode; the synthetic root is hidden by NodeRef.
    assert_eq!(report.scene().nodes().count(), 2);
    let node = report
        .scene()
        .nodes()
        .find(|node| node.light().is_some())
        .unwrap();
    assert_eq!(node.name(), Some("TriangleNode"));
    assert!(node.light().is_some());
    assert!(node.camera().is_some());
    assert_eq!(report.scene().mesh_instances().count(), 1);
    assert_eq!(
        report.scene().mesh_instances().next().unwrap().node().id(),
        node.id()
    );
    let material = report.scene().materials().next().unwrap();
    assert_eq!(material.name(), Some("Red"));
    let pbr = material.factors().pbr_metallic_roughness().unwrap();
    assert_eq!([pbr.metallic(), pbr.roughness()], [0.5, 0.25]);
    Ok(())
}

#[test]
fn dependency_failure_after_light_preparation_publishes_no_scene_and_releases_storage()
-> TestResult<()> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    document["extensionsUsed"] = json!(["KHR_lights_punctual"]);
    document["extensions"] =
        json!({"KHR_lights_punctual":{"lights":[{"type":"point","name":"Prepared light"}]}});
    document["nodes"][0]["extensions"] = json!({"KHR_lights_punctual":{"light":0}});
    let (result, _) = import_assets_result(
        SOURCE,
        [(SOURCE, serde_json::to_vec(&document)?)],
        sidecar_options(),
    )?;
    let failure = expected_error(result)?;
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .unwrap()
            .attempted(),
        1
    );
    assert_eq!(
        failure
            .source_scope_record()
            .unwrap()
            .counts(baozi_import::SourceScopeEntityKind::Light)
            .selected(),
        1
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    Ok(())
}
