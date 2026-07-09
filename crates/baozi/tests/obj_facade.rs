use baozi::{
    AssetPath, BaoziError, CapabilityStatus, ExternalReferencePolicy, FormatCapability,
    ImportOptions, ImportStage, Importer, MemoryAssetIo, PostProcessPipeline, PostProcessStep,
    PrimitiveTopology, Result, TextureSource,
};

fn triangle_obj() -> &'static [u8] {
    b"o facade
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
"
}

#[test]
fn facade_reads_obj_from_bytes() -> Result<()> {
    let report = Importer::new().read_bytes("facade.obj", triangle_obj())?;

    assert_eq!(report.format().id(), "obj");
    assert_eq!(report.scene().meshes.len(), 1);
    assert_eq!(
        report.scene().meshes[0].topology,
        PrimitiveTopology::Triangles
    );
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn facade_reports_obj_capabilities() -> Result<()> {
    let report = Importer::new().read_bytes("facade.obj", triangle_obj())?;

    assert!(
        report
            .format()
            .capabilities()
            .contains(&(FormatCapability::Geometry, CapabilityStatus::Supported))
    );
    assert!(
        report
            .format()
            .capabilities()
            .contains(&(FormatCapability::Materials, CapabilityStatus::Partial))
    );
    assert!(
        report
            .format()
            .capabilities()
            .contains(&(FormatCapability::Diagnostics, CapabilityStatus::Supported))
    );
    assert!(report.format().capabilities().contains(&(
        FormatCapability::ResourceLimits,
        CapabilityStatus::Supported
    )));
    Ok(())
}

#[test]
fn same_importer_reads_obj_repeatedly() -> Result<()> {
    let importer = Importer::new();
    let first = importer.read_bytes("first.obj", triangle_obj())?;
    let second = importer.read_bytes("second.obj", triangle_obj())?;

    assert_eq!(first.format().id(), "obj");
    assert_eq!(second.format().id(), "obj");
    assert_eq!(first.scene().meshes.len(), 1);
    assert_eq!(second.scene().meshes.len(), 1);
    Ok(())
}

#[test]
fn facade_detects_obj_content_with_unknown_extension() -> Result<()> {
    let report = Importer::new().read_bytes("facade.mesh", triangle_obj())?;

    assert_eq!(report.format().id(), "obj");
    assert_eq!(report.scene().meshes.len(), 1);
    Ok(())
}

#[test]
fn facade_preserves_obj_diagnostics() -> Result<()> {
    let obj_path = AssetPath::new("models/missing-mtl.obj")?;
    let obj = b"mtllib missing.mtl
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let mut io = MemoryAssetIo::new();
    io.insert(obj_path.clone(), obj.as_slice());
    let mut options = ImportOptions::memory();
    options.io.external_references = ExternalReferencePolicy::CustomResolver;

    let report = Importer::new().read_asset_with_options(&io, obj_path, options)?;

    assert_eq!(report.format().id(), "obj");
    assert_eq!(report.scene().meshes.len(), 1);
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "obj.mtl_missing")
    );
    Ok(())
}

#[cfg(feature = "native-fs")]
#[test]
fn facade_reads_obj_from_path() -> Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| baozi::BaoziError::io("time", error.to_string()))?
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("baozi-obj-facade-{}-{stamp}", std::process::id()));
    let obj_path = root.join("path.obj");
    std::fs::create_dir(&root)
        .map_err(|error| baozi::BaoziError::io(root.display().to_string(), error.to_string()))?;
    std::fs::write(&obj_path, triangle_obj()).map_err(|error| {
        baozi::BaoziError::io(obj_path.display().to_string(), error.to_string())
    })?;

    let report = Importer::new().read_path(&obj_path);
    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_dir(&root);
    let report = report?;

    assert_eq!(report.format().id(), "obj");
    assert_eq!(report.scene().meshes.len(), 1);
    Ok(())
}

#[test]
fn facade_reads_obj_with_memory_mtl_sidecar() -> Result<()> {
    let obj_path = AssetPath::new("models/cube.obj")?;
    let mtl_path = AssetPath::new("models/materials/cube.mtl")?;
    let obj = b"mtllib materials/cube.mtl
usemtl red
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let mtl = b"newmtl red
Kd 1 0 0
map_Kd textures/red.png
";
    let mut io = MemoryAssetIo::new();
    io.insert(obj_path.clone(), obj.as_slice());
    io.insert(mtl_path, mtl.as_slice());
    let mut options = ImportOptions::memory();
    options.io.external_references = ExternalReferencePolicy::CustomResolver;

    let report = Importer::new().read_asset_with_options(&io, obj_path, options)?;

    assert_eq!(report.format().id(), "obj");
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.scene().materials[0].name.as_deref(), Some("red"));
    match &report.scene().textures[0].source {
        TextureSource::External { uri } => {
            assert_eq!(uri, "models/materials/textures/red.png");
        }
        other => panic!("expected external texture, got {other:?}"),
    }
    Ok(())
}

#[test]
fn facade_obj_quad_can_be_triangulated_by_postprocess() -> Result<()> {
    let source = b"v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
f 1 2 3 4
";
    let importer = Importer::new();
    let report = importer.read_bytes("quad.obj", source)?;
    assert_eq!(report.stage(), ImportStage::ValidatedImported);
    assert_eq!(
        report.scene().meshes[0].topology,
        PrimitiveTopology::Polygons
    );

    let pipeline = PostProcessPipeline::new([PostProcessStep::Triangulate]);
    let report = importer.read_bytes_with_postprocess(
        "quad.obj",
        source,
        ImportOptions::memory(),
        &pipeline,
    )?;

    assert_eq!(report.stage(), ImportStage::PostProcessed);
    assert_eq!(
        report.scene().meshes[0].topology,
        PrimitiveTopology::Triangles
    );
    assert_eq!(report.scene().meshes[0].indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(report.stats().generated_faces(), 2);
    Ok(())
}

#[test]
fn postprocess_output_must_still_obey_resource_limits() -> Result<()> {
    let source = b"v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
f 1 2 3 4
";
    let mut options = ImportOptions::memory();
    options.limits.max_faces = 1;
    let pipeline = PostProcessPipeline::new([PostProcessStep::Triangulate]);

    let error = Importer::new()
        .read_bytes_with_postprocess("quad.obj", source, options, &pipeline)
        .unwrap_err();

    assert!(matches!(
        error,
        BaoziError::LimitExceeded { limit: "max_faces" }
    ));
    Ok(())
}
