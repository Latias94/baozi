mod common;

use baozi_core::{
    AlphaMode, MaterialAssignment, MaterialRef, MetadataOwnerRef, ProvenanceKindRef,
    ProvenanceOperationKindRef, SceneSnapshotLimits, ShadingModel, SourceObjectLocatorRef,
    ValidatedScene,
};
use baozi_import::{ImportDimension, SourceScopeEntityKind};
use common::{
    GltfTestOptions, TestResult, assert_resource_limit, expected_error, glb_with_json,
    import_assets, import_assets_result,
};
use serde_json::{Value, json};

fn missing_material_document() -> TestResult<Value> {
    let mut document: Value = serde_json::from_slice(&common::data_uri_gltf())?;
    document.as_object_mut().unwrap().remove("materials");
    document["meshes"][0]["primitives"][0]
        .as_object_mut()
        .unwrap()
        .remove("material");
    Ok(document)
}

fn assert_generated_default(
    scene: &ValidatedScene,
    material: MaterialRef<'_>,
) -> TestResult<Vec<u8>> {
    assert_eq!(material.name(), None);
    assert_eq!(material.shading_model(), ShadingModel::PbrMetallicRoughness);
    assert_eq!(material.alpha_mode(), AlphaMode::Opaque);
    assert!(!material.double_sided());
    assert_eq!(material.binding_count(), 0);
    let factors = material.factors();
    let pbr = factors.pbr_metallic_roughness().unwrap();
    let color = pbr.base_color();
    assert_eq!([color.r, color.g, color.b, color.a], [1.0; 4]);
    assert_eq!([pbr.metallic(), pbr.roughness()], [1.0; 2]);
    let emissive = factors.emission().factor();
    assert_eq!([emissive.x, emissive.y, emissive.z], [0.0; 3]);
    assert!(factors.pbr_specular_glossiness().is_none());
    assert!(factors.legacy().is_none());
    assert!(
        !scene
            .metadata_entries()
            .any(|entry| { entry.owner() == MetadataOwnerRef::Entity(material.metadata_owner()) })
    );

    let lineage: Vec<_> = scene.owner_provenance(material.metadata_owner())?.collect();
    assert_eq!(lineage.len(), 1);
    let generated = lineage[0];
    assert_eq!(generated.depth(), 2);
    let ProvenanceKindRef::Operation(operation) = generated.kind() else {
        panic!("the implicit material must have generated provenance");
    };
    assert_eq!(operation.kind(), ProvenanceOperationKindRef::Generated);
    assert_eq!(operation.evidence_count(), 1);
    let evidence = operation.evidence().next().unwrap();
    assert_eq!(evidence.code(), "org.baozi.gltf:implicit-material-v1");
    assert_eq!(evidence.occurrence_count(), 1);
    assert_eq!(evidence.digest().unwrap(), operation.options_digest());
    assert_eq!(generated.parents().len(), 1);
    let parent = generated.parents().next().unwrap();
    let ProvenanceKindRef::Source(source) = parent.kind() else {
        panic!("the generated default must be parented by the source document");
    };
    assert!(
        matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
        if value.starts_with("kind=document;"))
    );
    assert_eq!(parent.parents().len(), 0);
    Ok(operation.options_digest().1.to_vec())
}

#[test]
fn missing_bindings_share_one_default_only_within_selected_geometry() -> TestResult<()> {
    let mut generated_identities = Vec::new();
    for selected in [false, true] {
        for omitted_material in [false, true] {
            let mut document = missing_material_document()?;
            let primitive = document["meshes"][0]["primitives"][0].clone();
            document["meshes"][0]["primitives"] = json!([primitive, primitive]);
            document["meshes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"primitives": [primitive]}));
            document["nodes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"mesh": 1}));
            document["nodes"][0]["children"] = json!([1, 2]);
            if omitted_material {
                document["materials"] = json!([{}]);
            }
            if !selected {
                document["scenes"][0]["nodes"] = json!([]);
                document["buffers"][0]["uri"] = json!("never.bin");
            }
            let json = serde_json::to_vec(&document)?;
            for (source, bytes) in [
                ("default.gltf", json.clone()),
                ("default.glb", glb_with_json(&json)),
            ] {
                let (report, diagnostics) =
                    import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(diagnostics.is_empty());
                assert!(report.loss_evidence().is_empty());
                let scene = report.scene();
                assert_eq!(scene.material_count(), usize::from(selected));
                assert_eq!(
                    (
                        scene.image_count(),
                        scene.texture_count(),
                        scene.sampler_count()
                    ),
                    (0, 0, 0)
                );
                let counts = report
                    .source_scope_record()
                    .unwrap()
                    .counts(SourceScopeEntityKind::Material);
                assert_eq!(counts.selected(), 0);
                assert_eq!(counts.omitted(), u64::from(omitted_material));
                assert!(!scene.source_identities().any(|source| {
                    matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                        if value.starts_with("kind=material;"))
                }));
                if selected {
                    let default = scene.materials().next().unwrap();
                    generated_identities.push(assert_generated_default(scene, default)?);
                    let primitives: Vec<_> =
                        scene.meshes().flat_map(|mesh| mesh.primitives()).collect();
                    assert_eq!(primitives.len(), 3);
                    for primitive in primitives {
                        assert!(primitive.authored_material().is_none());
                        assert_eq!(primitive.effective_material().unwrap().id(), default.id());
                        assert!(matches!(primitive.material_assignment(),
                            MaterialAssignment::ImplicitDefault(material) if material.id() == default.id()));
                    }
                }
                let snapshot = scene.capture_snapshot(SceneSnapshotLimits::try_new(
                    64 * 1024,
                    256,
                    1_000_000,
                )?)?;
                assert_eq!(
                    snapshot.profile_operation_count(),
                    1 + usize::from(selected)
                );
                assert_eq!(snapshot.profile_evidence_count(), 1 + usize::from(selected));
                assert_eq!(
                    snapshot.profile_source_count(),
                    scene.source_identity_count()
                );
                assert_eq!(
                    snapshot.profile_provenance_node_count(),
                    snapshot.profile_source_count() + snapshot.profile_operation_count()
                );
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
    }
    assert!(
        generated_identities
            .windows(2)
            .all(|pair| pair[0] == pair[1])
    );
    Ok(())
}

#[test]
fn authored_empty_materials_keep_source_order_and_stay_distinct_from_the_default() -> TestResult<()>
{
    for include_missing in [false, true] {
        let mut document = missing_material_document()?;
        document["materials"] = json!([{"name": "omitted"}, {}, {"name": "selected"}]);
        let missing = document["meshes"][0]["primitives"][0].clone();
        let mut named = missing.clone();
        named["material"] = json!(2);
        let mut empty = missing.clone();
        empty["material"] = json!(1);
        document["meshes"][0]["primitives"] = if include_missing {
            json!([missing, named, empty, missing])
        } else {
            json!([named, empty])
        };
        let json = serde_json::to_vec(&document)?;
        for (source, bytes) in [
            ("authored.gltf", json.clone()),
            ("authored.glb", glb_with_json(&json)),
        ] {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let scene = report.scene();
            let materials: Vec<_> = scene.materials().collect();
            assert_eq!(materials.len(), 2 + usize::from(include_missing));
            assert_eq!(materials[0].name(), None);
            assert_eq!(materials[1].name(), Some("selected"));
            let counts = report
                .source_scope_record()
                .unwrap()
                .counts(SourceScopeEntityKind::Material);
            assert_eq!((counts.selected(), counts.omitted()), (2, 1));
            let source_materials: Vec<_> = scene
                .source_identities()
                .filter(|source| {
                    matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                    if value.starts_with("kind=material;"))
                })
                .collect();
            assert_eq!(source_materials.len(), 2);
            for (material, index) in materials.iter().take(2).zip([1, 2]) {
                let lineage: Vec<_> = scene.owner_provenance(material.metadata_owner())?.collect();
                assert!(lineage.iter().any(|node| {
                    matches!(node.kind(), ProvenanceKindRef::Source(source)
                        if matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                            if value.starts_with(&format!("kind=material;parent=-;index={index};"))))
                }));
            }
            let primitives: Vec<_> = scene.meshes().next().unwrap().primitives().collect();
            let offset = usize::from(include_missing);
            for (primitive, material) in primitives[offset..offset + 2]
                .iter()
                .zip([materials[1], materials[0]])
            {
                assert!(matches!(primitive.material_assignment(),
                    MaterialAssignment::Authored(authored) if authored.id() == material.id()));
                assert_eq!(primitive.authored_material().unwrap().id(), material.id());
                assert_eq!(primitive.effective_material().unwrap().id(), material.id());
            }
            if include_missing {
                let default = materials[2];
                assert_generated_default(scene, default)?;
                assert_eq!(default.factors(), materials[0].factors());
                assert_ne!(default.id(), materials[0].id());
                for primitive in [primitives[0], primitives[3]] {
                    assert!(matches!(primitive.material_assignment(),
                        MaterialAssignment::ImplicitDefault(material) if material.id() == default.id()));
                }
            }
            assert_eq!(report.ledger().current_live_bytes(), 0);
        }
    }
    Ok(())
}

#[test]
fn generated_material_storage_and_lineage_fit_the_exact_live_memory_budget() -> TestResult<()> {
    const SOURCE: &str = "default-budget.gltf";
    let document = serde_json::to_vec(&missing_material_document()?)?;
    let (report, diagnostics) = import_assets(
        SOURCE,
        [(SOURCE, document.clone())],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    let peak = report.ledger().peak_live_bytes();
    assert_generated_default(report.scene(), report.scene().materials().next().unwrap())?;
    drop(report);

    let mut options = GltfTestOptions::memory();
    options.limits.max_live_bytes = peak;
    let (report, diagnostics) = import_assets(SOURCE, [(SOURCE, document.clone())], options)?;
    assert!(diagnostics.is_empty());
    assert_eq!(report.ledger().peak_live_bytes(), peak);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    drop(report);

    let mut options = GltfTestOptions::memory();
    options.limits.max_live_bytes = peak - 1;
    let (result, diagnostics) = import_assets_result(SOURCE, [(SOURCE, document)], options)?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert_resource_limit(&failure, "max_live_bytes");
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    Ok(())
}
