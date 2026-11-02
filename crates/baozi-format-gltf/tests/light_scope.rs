mod common;

use baozi_core::{MetadataOwnerRef, MetadataValueRef, ProvenanceKindRef, SourceObjectLocatorRef};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    FormatFailureDetailValue, ImportCause, ImportDimension, ImportLossKind, LossPolicy,
    ScopeAbortCause, SourceScopeDisposition, SourceScopeEntityKind,
};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, controlled_import, expected_error,
    glb_with_json, gltf_failure, import_assets, import_assets_result,
    import_assets_result_with_control, sidecar_options, triangle_gltf,
};
use serde_json::{Value, json};

const SOURCE: &str = "models/light-scope.gltf";

fn document() -> Value {
    json!({
        "asset":{"version":"2.0"}, "extensionsUsed":["KHR_lights_punctual"],
        "extensions":{"KHR_lights_punctual":{"lights":[
            {"type":"directional","name":""},
            {"type":"future","spot":null,"color":null,"behavior":true,"extensions":{"VENDOR_behavior":{}}},
            {"type":"point","name":"Shared","range":8}
        ]}},
        "scene":0,"scenes":[{"nodes":[0,1,2]}, {"nodes":[]}],
        "nodes":[
            {"extensions":{"KHR_lights_punctual":{"light":0}}},
            {"extensions":{"KHR_lights_punctual":{"light":2}}},
            {"extensions":{"KHR_lights_punctual":{"light":2}}},
            {"extensions":{"KHR_lights_punctual":null}}
        ]
    })
}

#[test]
fn root_inventory_is_selected_by_attachments_and_shared_definitions_keep_source_ordinals()
-> TestResult<()> {
    let (report, diagnostics) = import_assets(
        SOURCE,
        [(SOURCE, serde_json::to_vec(&document())?)],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    let scene = report.scene();
    let lights = scene.lights().collect::<Vec<_>>();
    assert_eq!(lights.len(), 2);
    assert_eq!(
        scene
            .nodes()
            .filter(|node| node.light().map(|light| light.id()) == Some(lights[0].id()))
            .count(),
        1
    );
    assert_eq!(
        scene
            .nodes()
            .filter(|node| node.light().map(|light| light.id()) == Some(lights[1].id()))
            .count(),
        2
    );
    for (light, index, name) in [(lights[0], 0, ""), (lights[1], 2, "Shared")] {
        assert!(scene.owner_provenance(light.metadata_owner())?.any(|node| {
            matches!(node.kind(), ProvenanceKindRef::Source(source)
                if matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                    if value.starts_with(&format!("kind=light;parent=-;index={index};"))))
        }));
        assert!(scene.metadata_entries().any(|entry| entry.owner()
            == MetadataOwnerRef::Entity(light.metadata_owner())
            && entry.key() == "org.baozi.gltf:source-name"
            && matches!(entry.value(), MetadataValueRef::String(value) if value == name)));
    }
    let scope = report.source_scope_record().unwrap();
    assert_eq!(scope.counts(SourceScopeEntityKind::Light).selected(), 2);
    assert_eq!(scope.counts(SourceScopeEntityKind::Light).omitted(), 1);
    let details = scope
        .details()
        .filter(|detail| detail.kind() == SourceScopeEntityKind::Light)
        .map(|detail| (detail.identity(), detail.disposition()))
        .collect::<Vec<_>>();
    assert_eq!(
        details,
        [
            ("l0", SourceScopeDisposition::Selected),
            ("l1", SourceScopeDisposition::Omitted),
            ("l2", SourceScopeDisposition::Selected)
        ]
    );
    Ok(())
}

#[test]
fn empty_selected_scene_does_not_interpret_or_orphan_declared_lights() -> TestResult<()> {
    let mut json = document();
    json["scene"] = json!(1);
    let (report, _) = import_assets(
        SOURCE,
        [(SOURCE, serde_json::to_vec(&json)?)],
        GltfTestOptions::memory(),
    )?;
    assert_eq!(report.scene().lights().count(), 0);
    assert_eq!(
        report
            .source_scope_record()
            .unwrap()
            .counts(SourceScopeEntityKind::Light)
            .omitted(),
        3
    );
    assert!(!report.scene().source_identities().any(|source| matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. } if value.starts_with("kind=light;"))));
    Ok(())
}

#[test]
fn selected_unsupported_sites_and_bad_references_fail_before_dependency_io_under_both_loss_policies()
-> TestResult<()> {
    for mutation in 0..16 {
        let mut json: Value = serde_json::from_slice(&triangle_gltf())?;
        json["extensionsUsed"] = json!(["KHR_lights_punctual"]);
        json["extensions"] = json!({"KHR_lights_punctual":{"lights":[{"type":"spot","spot":{}}]}});
        json["nodes"][0]["extensions"] = json!({"KHR_lights_punctual":{"light":0}});
        match mutation {
            0 => json["extensions"]["KHR_lights_punctual"]["behavior"] = json!(true),
            1 => {
                json["extensions"]["KHR_lights_punctual"]["extensions"] =
                    json!({"VENDOR_behavior":{}})
            }
            2 => json["nodes"][0]["extensions"]["KHR_lights_punctual"]["behavior"] = json!(true),
            3 => {
                json["nodes"][0]["extensions"]["KHR_lights_punctual"]["extensions"] =
                    json!({"VENDOR_behavior":{}})
            }
            4 => json["extensions"]["KHR_lights_punctual"]["lights"][0]["behavior"] = json!(true),
            5 => {
                json["extensions"]["KHR_lights_punctual"]["lights"][0]["extensions"] =
                    json!({"VENDOR_behavior":{}})
            }
            6 => {
                json["extensions"]["KHR_lights_punctual"]["lights"][0]["spot"]["behavior"] =
                    json!(true)
            }
            7 => {
                json["extensions"]["KHR_lights_punctual"]["lights"][0]["spot"]["extensions"] =
                    json!({"VENDOR_behavior":{}})
            }
            8 => json["extensions"]["KHR_lights_punctual"]["lights"][0]["type"] = json!("future"),
            9 => json["extensions"]["KHR_lights_punctual"]["lights"][0]["type"] = json!("point"),
            10 => json["nodes"][0]["extensions"]["KHR_lights_punctual"] = json!(null),
            11 => json["nodes"][0]["extensions"]["KHR_lights_punctual"] = json!({}),
            12 => json["nodes"][0]["extensions"]["KHR_lights_punctual"]["light"] = json!(null),
            13 => json["nodes"][0]["extensions"]["KHR_lights_punctual"]["light"] = json!(-1),
            14 => json["nodes"][0]["extensions"]["KHR_lights_punctual"]["light"] = json!(1),
            15 => json["nodes"][0]["extensions"]["KHR_lights_punctual"]["light"] = json!(0.5),
            _ => unreachable!(),
        }
        for allow_loss in [false, true] {
            let mut options = sidecar_options();
            if allow_loss {
                options.loss_policy =
                    LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss);
            }
            let (result, _) =
                import_assets_result(SOURCE, [(SOURCE, serde_json::to_vec(&json)?)], options)?;
            let failure = expected_error(result)?;
            if mutation < 10 {
                assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
            }
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0,
                "mutation {mutation}: {failure:?}"
            );
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert!(failure.loss_evidence().is_empty());
        }
    }
    Ok(())
}

#[test]
fn root_shape_and_extension_declarations_are_global_even_with_empty_selection() -> TestResult<()> {
    for mutation in 0..7 {
        let mut json = document();
        json["scene"] = json!(1);
        match mutation {
            0 => json["extensions"]["KHR_lights_punctual"] = json!(null),
            1 => json["extensions"]["KHR_lights_punctual"] = json!({}),
            2 => json["extensions"]["KHR_lights_punctual"]["lights"] = json!([]),
            3 => json["extensions"]["KHR_lights_punctual"]["lights"] = json!({}),
            4 => json["extensionsRequired"] = json!(["VENDOR_behavior"]),
            5 => {
                json.as_object_mut().unwrap().remove("extensionsUsed");
            }
            6 => json["extensions"]["KHR_lights_punctual"]["lights"] = json!([null]),
            _ => unreachable!(),
        }
        let (result, _) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&json)?)],
            GltfTestOptions::memory(),
        )?;
        let failure = expected_error(result)?;
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
    Ok(())
}

#[test]
fn escaped_keys_exact_extras_and_original_definition_ranges_in_json_and_glb() -> TestResult<()> {
    let light =
        r#"{"type":"spot","extras":{"definition":1.2300e+04},"sp\u006ft":{"extr\u0061s":null}}"#;
    let node = r#"{"extensions":{"KHR_lights_punct\u0075al":{"li\u0067ht":0,"extras":[ 1, 2 ]}}}"#;
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"extensionsUsed":["KHR_lights_punctual"],"extensions":{{"KHR_lights_punct\u0075al":{{"li\u0067hts":[{light}],"extras":{{ "root" : true }}}}}},"nodes":[{node}]}}"#
    );
    for glb in [false, true] {
        let (report, _) = import_assets(
            SOURCE,
            [(
                SOURCE,
                if glb {
                    glb_with_json(json.as_bytes())
                } else {
                    json.as_bytes().to_vec()
                },
            )],
            GltfTestOptions::memory(),
        )?;
        let scene = report.scene();
        assert_eq!(scene.lights().count(), 1);
        let retained = scene.lights().next().unwrap();
        assert!(!scene.metadata_entries().any(|entry| entry.owner()
            == MetadataOwnerRef::Entity(retained.metadata_owner())
            && entry.key() == "org.baozi.gltf:source-name"));
        for (key, raw, kind) in [
            (
                "org.baozi.gltf:extras",
                r#"{"definition":1.2300e+04}"#,
                "light",
            ),
            ("org.baozi.gltf:spot-extras", "null", "light"),
            ("org.baozi.gltf:punctual-light-extras", "[ 1, 2 ]", "node"),
            (
                "org.baozi.gltf:punctual-lights-extras",
                r#"{ "root" : true }"#,
                "document",
            ),
        ] {
            let entry = scene
                .metadata_entries()
                .find(|entry| entry.key() == key)
                .unwrap();
            let MetadataOwnerRef::SourceObject(source) = entry.owner() else {
                panic!("source extras owner");
            };
            assert!(
                matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. } if value.starts_with(&format!("kind={kind};")))
            );
            assert!(
                matches!(entry.value(), MetadataValueRef::Bytes { media_type: "application/json", payload } if payload == raw.as_bytes())
            );
        }
        let expected = format!(
            "kind=light;parent=-;index=0;offset={};length={}",
            json.find(light).unwrap() + if glb { 20 } else { 0 },
            light.len()
        );
        assert!(scene.source_identities().any(|source| matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. } if value == expected)));
    }
    Ok(())
}

#[test]
fn lookalike_keys_in_extras_never_create_inventory() -> TestResult<()> {
    let json = br#"{"asset":{"version":"2.0"},"extras":{"extensions":{"KHR_lights_punctual":{"lights":[{"type":"point"}]}}},"nodes":[{"extras":{"extensions":{"KHR_lights_punctual":{"light":0}}}}]}"#;
    let (report, _) = import_assets(SOURCE, [(SOURCE, json.to_vec())], GltfTestOptions::memory())?;
    assert_eq!(report.scene().lights().count(), 0);
    assert_eq!(
        report
            .source_scope_record()
            .unwrap()
            .counts(SourceScopeEntityKind::Light)
            .selected(),
        0
    );
    assert_eq!(
        report
            .source_scope_record()
            .unwrap()
            .counts(SourceScopeEntityKind::Light)
            .omitted(),
        0
    );
    Ok(())
}

#[test]
fn unreferenced_root_definitions_never_create_implicit_placements() -> TestResult<()> {
    let json = br#"{"asset":{"version":"2.0"},"extensionsUsed":["KHR_lights_punctual"],"extensions":{"KHR_lights_punctual":{"lights":[{"type":null,"color":null}]}}}"#;
    let (report, diagnostics) =
        import_assets(SOURCE, [(SOURCE, json.to_vec())], GltfTestOptions::memory())?;
    assert!(diagnostics.is_empty());
    assert_eq!(report.scene().nodes().count(), 0);
    assert_eq!(report.scene().lights().count(), 0);
    let counts = report
        .source_scope_record()
        .unwrap()
        .counts(SourceScopeEntityKind::Light);
    assert_eq!(counts.selected(), 0);
    assert_eq!(counts.omitted(), 1);
    Ok(())
}

#[test]
fn unsupported_light_properties_report_original_value_ranges_before_io() -> TestResult<()> {
    let raw = r#"[null, { "extension" : true }]"#;
    for site in ["root", "node", "light", "spot"] {
        let mut json = document();
        json["extensions"]["KHR_lights_punctual"]["lights"][0] = json!({"type":"spot","spot":{}});
        let owner = match site {
            "root" => &mut json["extensions"]["KHR_lights_punctual"],
            "node" => &mut json["nodes"][0]["extensions"]["KHR_lights_punctual"],
            "light" => &mut json["extensions"]["KHR_lights_punctual"]["lights"][0],
            "spot" => &mut json["extensions"]["KHR_lights_punctual"]["lights"][0]["spot"],
            _ => unreachable!(),
        };
        owner["VENDOR_behavior"] = json!("REPLACE_PAYLOAD");
        let json = serde_json::to_string(&json)?.replace("\"REPLACE_PAYLOAD\"", raw);
        for glb in [false, true] {
            let bytes = if glb {
                glb_with_json(json.as_bytes())
            } else {
                json.as_bytes().to_vec()
            };
            let (result, _) =
                import_assets_result(SOURCE, [(SOURCE, bytes)], GltfTestOptions::memory())?;
            let failure = expected_error(result)?;
            assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
            let envelope = gltf_failure(&failure)?;
            for (key, expected) in [
                (
                    "byte_offset",
                    (json.find(raw).unwrap() + if glb { 20 } else { 0 }) as u64,
                ),
                ("byte_length", raw.len() as u64),
            ] {
                assert!(envelope.details().iter().any(|detail| detail.key() == key
                    && matches!(detail.value(), FormatFailureDetailValue::Unsigned(value) if value == expected)), "{site}: {envelope:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn selected_light_names_are_admitted_together_before_dependency_io() -> TestResult<()> {
    let mut json: Value = serde_json::from_slice(&triangle_gltf())?;
    json["extensionsUsed"] = json!(["KHR_lights_punctual"]);
    json["extensions"] = json!({"KHR_lights_punctual":{"lights":[
        {"type":"point","name":"a".repeat(8192)},
        {"type":"point","name":"b".repeat(8192)}
    ]}});
    json["nodes"][0]["extensions"] = json!({"KHR_lights_punctual":{"light":0}});
    let added_root = json["nodes"].as_array().unwrap().len();
    json["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"extensions":{"KHR_lights_punctual":{"light":1}}}));
    json["scenes"][0]["nodes"] = json!([0, added_root]);
    for (string_limit, canonical_limit, expected) in [
        (4096, 65536, "max_string_bytes"),
        (16384, 12000, "max_canonical_string_bytes"),
    ] {
        let mut options = sidecar_options();
        options.limits.max_string_bytes = string_limit;
        options.limits.max_canonical_string_bytes = canonical_limit;
        let (result, _) =
            import_assets_result(SOURCE, [(SOURCE, serde_json::to_vec(&json)?)], options)?;
        let failure = expected_error(result)?;
        common::assert_resource_limit(&failure, expected);
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}

#[test]
fn light_inventory_limits_and_cancellation_release_temporary_storage() -> TestResult<()> {
    use std::sync::atomic::Ordering;

    let mut json = document();
    json["extensions"]["KHR_lights_punctual"]["lights"] = json!(
        (0..64)
            .map(|index| json!({"type":"point","name":format!("Light {index}")}))
            .collect::<Vec<_>>()
    );
    json["nodes"] = json!(
        (0..64)
            .map(|index| json!({"extensions":{"KHR_lights_punctual":{"light":index}}}))
            .collect::<Vec<_>>()
    );
    json["scenes"][0]["nodes"] = json!((0..64).collect::<Vec<_>>());
    let bytes = serde_json::to_vec(&json)?;
    let (control, polls) = controlled_import(None);
    let (report, _) = import_assets_result_with_control(
        SOURCE,
        [(SOURCE, bytes.clone())],
        GltfTestOptions::memory(),
        control,
    )?;
    let report = report?;
    assert_eq!(report.scene().lights().count(), 64);
    let total_polls = polls.load(Ordering::SeqCst);
    assert!(total_polls > 3);
    for cancel_on in [1, total_polls / 2, total_polls - 1] {
        let (control, _) = controlled_import(Some(cancel_on));
        let (result, _) = import_assets_result_with_control(
            SOURCE,
            [(SOURCE, bytes.clone())],
            GltfTestOptions::memory(),
            control,
        )?;
        let failure = expected_error(result)?;
        assert!(
            matches!(failure.cause(), ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::Control(_))),
            "{failure:?}"
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
    }
    for limit in ["records", "work", "storage"] {
        let mut options = GltfTestOptions::memory();
        match limit {
            "records" => options.limits.max_records = 8,
            "work" => options.limits.max_work_units = 32,
            "storage" => options.limits.max_live_bytes = 32 * 1024,
            _ => unreachable!(),
        }
        let (result, _) = import_assets_result(SOURCE, [(SOURCE, bytes.clone())], options)?;
        let failure = expected_error(result)?;
        assert!(
            matches!(failure.cause(), ImportCause::Scope(_)),
            "{limit}: {failure:?}"
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}
