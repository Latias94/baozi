mod common;

use baozi_core::{ColorInterpretation, ShadingModel, TextureRole};
use common::{
    TestResult, import_assets, sidecar_options, triangle_bin, triangle_gltf, triangle_png,
};
use serde_json::{Value, json};

#[test]
fn selected_emissive_strength_imports_with_existing_material_reads() -> TestResult<()> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    document["extensionsUsed"] = json!(["KHR_materials_emissive_strength"]);
    document["materials"][0]["extensions"] = json!({
        "KHR_materials_emissive_strength": {"emissiveStrength": 8}
    });
    document["materials"][0]["emissiveFactor"] = json!([0.25, 0.5, 1]);
    document["materials"][0]["emissiveTexture"] = json!({"index": 0});
    let (report, diagnostics) = import_assets(
        "models/emission.gltf",
        [
            ("models/emission.gltf", serde_json::to_vec(&document)?),
            ("models/triangle.bin", triangle_bin()),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    assert!(report.loss_evidence().is_empty());
    let material = report.scene().materials().next().unwrap();
    assert_eq!(material.name(), Some("Red"));
    assert_eq!(material.shading_model(), ShadingModel::PbrMetallicRoughness);
    let pbr = material.factors().pbr_metallic_roughness().unwrap();
    assert_eq!([pbr.metallic(), pbr.roughness()], [0.5, 0.25]);
    let binding = material
        .bindings()
        .find(|binding| binding.role() == TextureRole::Emissive)
        .unwrap();
    assert_eq!(binding.strength().get(), 1.0);
    assert_eq!(
        binding.color_interpretation(),
        ColorInterpretation::SrgbColor
    );
    assert_eq!(binding.texture().image().encoded_bytes(), triangle_png());
    Ok(())
}
