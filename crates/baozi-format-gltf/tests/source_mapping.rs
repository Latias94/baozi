mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{FormatFailureDetailValue, ImportDimension, SceneSnapshotLimits};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, expected_error, glb_with_json, gltf_failure,
    import_assets, import_assets_result,
};

fn unsigned_detail(envelope: &baozi_import::FormatFailureEnvelope, key: &str) -> Option<u64> {
    envelope.details().iter().find_map(|detail| {
        if detail.key() != key {
            return None;
        }
        match detail.value() {
            FormatFailureDetailValue::Unsigned(value) => Some(value),
            _ => None,
        }
    })
}

#[test]
fn rejected_payload_ranges_preserve_raw_unicode_escapes_and_glb_offsets() -> TestResult<()> {
    let value = r#"{"tag":"扩展"}"#;
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"name":"节点","extr\u0061s":{value}}}]}}"#
    );
    let offset = json.find(value).unwrap();
    for is_glb in [false, true] {
        let bytes = if is_glb {
            glb_with_json(json.as_bytes())
        } else {
            json.as_bytes().to_vec()
        };
        let (result, diagnostics) = import_assets_result(
            "mapping.gltf",
            [("mapping.gltf", bytes)],
            GltfTestOptions::memory(),
        )?;
        assert!(diagnostics.is_empty());
        let error = expected_error(result)?;
        assert_gltf_failure(&error, GltfFailureCode::UnsupportedFeature, Some("extras"))?;
        let failure = gltf_failure(&error)?;
        assert_eq!(
            unsigned_detail(failure, "byte_offset"),
            Some((offset + if is_glb { 20 } else { 0 }) as u64)
        );
        assert_eq!(
            unsigned_detail(failure, "byte_length"),
            Some(value.len() as u64)
        );
        assert_eq!(
            error
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
    }
    Ok(())
}

#[test]
fn unselected_payloads_are_scope_omissions() -> TestResult<()> {
    let json = br#"{
        "asset":{"version":"2.0"},"scene":0,
        "scenes":[{"nodes":[0]},{"nodes":[1],"extras":{"unselected":true}}],
        "nodes":[{"name":"selected"},{"extras":{"unselected":true},"extensions":{"VENDOR_anything":{}}}]
    }"#;
    let (report, diagnostics) = import_assets(
        "mapping.gltf",
        [("mapping.gltf", json.to_vec())],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    assert_eq!(
        report
            .scene()
            .nodes()
            .filter(|node| node.name() == Some("selected"))
            .count(),
        1
    );
    let snapshot = report
        .scene()
        .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?)?;
    assert_eq!(snapshot.profile_source_count(), 3);
    assert_eq!(snapshot.profile_provenance_node_count(), 4);
    assert_eq!(snapshot.profile_operation_count(), 1);
    assert_eq!(snapshot.profile_evidence_count(), 1);
    assert_eq!(snapshot.profile_parent_reference_count(), 2);
    Ok(())
}

#[test]
fn same_names_with_different_source_ordinals_change_the_retained_profile() -> TestResult<()> {
    let mut digests = Vec::new();
    for selected_node in [0, 1] {
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[{selected_node}]}}],"nodes":[{{"name":"same"}},{{"name":"same"}}]}}"#
        );
        let (report, diagnostics) = import_assets(
            "mapping.gltf",
            [("mapping.gltf", json.into_bytes())],
            GltfTestOptions::memory(),
        )?;
        assert!(diagnostics.is_empty());
        assert_eq!(
            report
                .scene()
                .nodes()
                .filter(|node| node.name() == Some("same"))
                .count(),
            1
        );
        let snapshot = report
            .scene()
            .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?)?;
        assert_eq!(snapshot.profile_resource_count(), 1);
        assert_eq!(snapshot.profile_source_count(), 3);
        assert_eq!(snapshot.profile_provenance_node_count(), 4);
        digests.push(snapshot.digest());
    }
    assert_ne!(digests[0], digests[1]);
    Ok(())
}

#[test]
fn glb_raw_source_ranges_survive_in_the_canonical_profile() -> TestResult<()> {
    let json = br#"{"asset":{"version":"2.0"},"nodes":[{"name":"node"}]}"#;
    let mut digests = Vec::new();
    for bytes in [json.to_vec(), glb_with_json(json)] {
        let (report, diagnostics) = import_assets(
            "mapping.gltf",
            [("mapping.gltf", bytes)],
            GltfTestOptions::memory(),
        )?;
        assert!(diagnostics.is_empty());
        let snapshot = report
            .scene()
            .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?)?;
        assert_eq!(snapshot.profile_source_count(), 2);
        digests.push(snapshot.digest());
    }
    assert_ne!(digests[0], digests[1]);
    Ok(())
}

#[test]
fn generated_root_and_implicit_sampler_keep_durable_operations_and_source_parents() -> TestResult<()>
{
    use baozi_import::SourceScopeEntityKind;

    for implicit_sampler in [false, true] {
        let mut document: serde_json::Value = serde_json::from_slice(&common::triangle_gltf())?;
        if implicit_sampler {
            document.as_object_mut().unwrap().remove("samplers");
            document["textures"][0]
                .as_object_mut()
                .unwrap()
                .remove("sampler");
        }
        let (report, diagnostics) = import_assets(
            "models/generated.gltf",
            [
                ("models/generated.gltf", serde_json::to_vec(&document)?),
                ("models/triangle.bin", common::triangle_bin()),
                ("models/textures/base.png", common::triangle_png()),
            ],
            common::sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let snapshot = report
            .scene()
            .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?)?;
        let operations = 1 + usize::from(implicit_sampler);
        assert_eq!(snapshot.profile_operation_count(), operations);
        assert_eq!(snapshot.profile_evidence_count(), operations);
        assert_eq!(
            snapshot.profile_provenance_node_count(),
            snapshot.profile_source_count() + operations
        );
        // Root: document + selected scene. Default sampler: document + referencing texture.
        assert_eq!(
            snapshot.profile_parent_reference_count(),
            (operations * 2) as u64
        );
        assert_eq!(report.scene().sampler_count(), 1);
        assert_eq!(
            report
                .source_scope_record()
                .unwrap()
                .counts(SourceScopeEntityKind::Sampler)
                .selected(),
            u64::from(!implicit_sampler),
        );
    }
    Ok(())
}

#[test]
fn selected_unknown_extension_payloads_are_not_lost_in_backend_decode() -> TestResult<()> {
    for property in [
        r#""extensions":{"VENDOR_anything":{}}"#,
        r#""extensions":{"KHR_texture_transform":{}}"#,
        r#""extras":null"#,
    ] {
        let json = format!(r#"{{"asset":{{"version":"2.0"}},"nodes":[{{{property}}}]}}"#);
        let (result, _) = import_assets_result(
            "mapping.gltf",
            [("mapping.gltf", json.into_bytes())],
            GltfTestOptions::memory(),
        )?;
        let error = expected_error(result)?;
        assert_eq!(
            gltf_failure(&error)?.code(),
            GltfFailureCode::UnsupportedFeature.as_str(),
            "{property}: {error:?}"
        );
    }
    Ok(())
}

#[test]
fn selected_nested_camera_extras_are_explicitly_rejected() -> TestResult<()> {
    let json = br#"{
        "asset":{"version":"2.0"},
        "nodes":[{"camera":0}],
        "cameras":[{"type":"orthographic","orthographic":{"xmag":1,"ymag":1,"znear":0.1,"zfar":10,"extras":{"nested":true}}}]
    }"#;
    let (result, _) = import_assets_result(
        "mapping.gltf",
        [("mapping.gltf", json.to_vec())],
        GltfTestOptions::memory(),
    )?;
    assert_gltf_failure(
        &expected_error(result)?,
        GltfFailureCode::UnsupportedFeature,
        Some("extras"),
    )?;
    Ok(())
}
