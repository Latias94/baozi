use baozi::{
    AssetPath, ExternalReferencePolicy, ImportOptions, Importer, MemoryAssetIo, Result,
    TextureSource,
};

fn main() -> Result<()> {
    let model_path = AssetPath::new("models/triangle.obj")?;
    let material_path = AssetPath::new("models/materials/triangle.mtl")?;

    let mut io = MemoryAssetIo::new();
    io.insert(
        model_path.clone(),
        b"mtllib materials/triangle.mtl
usemtl red
v 0 0 0
v 1 0 0
v 0 1 0
vt 0 0
vt 1 0
vt 0 1
f 1/1 2/2 3/3
"
        .as_slice(),
    );
    io.insert(
        material_path,
        b"newmtl red
Kd 1 0 0
map_Kd textures/red.png
"
        .as_slice(),
    );

    let mut options = ImportOptions::memory();
    options.io.external_references = ExternalReferencePolicy::CustomResolver;

    let report = Importer::new().read_asset_with_options(&io, model_path, options)?;
    let scene = report.scene();
    let texture_uri = scene
        .textures
        .first()
        .and_then(|texture| match &texture.source {
            TextureSource::External { uri } => Some(uri.as_str()),
            TextureSource::Embedded { .. } => None,
        })
        .unwrap_or("<none>");

    println!(
        "format={} materials={} textures={} texture_uri={} opened_assets={}",
        report.format().id(),
        scene.materials.len(),
        scene.textures.len(),
        texture_uri,
        report.stats().opened_assets()
    );
    Ok(())
}
