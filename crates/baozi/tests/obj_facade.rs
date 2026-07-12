#![allow(
    clippy::result_large_err,
    reason = "facade tests preserve complete bounded terminal evidence by value"
)]
#![cfg(feature = "format-obj")]

use baozi::{
    AssetPath, CapabilityStatus, FormatCapability, ImportConfigOverlay, ImportDimension,
    ImportLossKind, ImportOperationControl, ImportPhase, ImportRequest, ImportRequestLayer,
    Importer, LossPolicy, MemoryAssetIo, OptionCompileLimits, PostProcessLimits,
    PostProcessPipeline, PostProcessStep, PrimitiveTopology, ResourceLimits, ResourceLimitsOverlay,
    Result, TextureSource,
};
use std::sync::Arc;

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
    let report = Importer::new()?.read_bytes("facade.obj", triangle_obj())?;

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert_eq!(report.scene().meshes().len(), 1);
    assert_eq!(
        report.scene().meshes()[0].topology,
        PrimitiveTopology::Triangles
    );
    assert!(report.diagnostics().is_empty());
    Ok(())
}

#[test]
fn facade_reports_obj_capabilities() -> Result<()> {
    let report = Importer::new()?.read_bytes("facade.obj", triangle_obj())?;

    assert_eq!(
        report
            .format()
            .capability_status(FormatCapability::Geometry),
        Some(CapabilityStatus::Supported)
    );
    assert_eq!(
        report
            .format()
            .capability_status(FormatCapability::Materials),
        Some(CapabilityStatus::Partial)
    );
    assert_eq!(
        report
            .format()
            .capability_status(FormatCapability::Diagnostics),
        Some(CapabilityStatus::Supported)
    );
    assert_eq!(
        report
            .format()
            .capability_status(FormatCapability::ResourceLimits),
        Some(CapabilityStatus::Supported)
    );
    Ok(())
}

#[test]
fn same_importer_reads_obj_repeatedly() -> Result<()> {
    let importer = Importer::new()?;
    let first = importer.read_bytes("first.obj", triangle_obj())?;
    let second = importer.read_bytes("second.obj", triangle_obj())?;

    assert_eq!(first.format().id().as_str(), "org.baozi.obj");
    assert_eq!(second.format().id().as_str(), "org.baozi.obj");
    assert_eq!(first.scene().meshes().len(), 1);
    assert_eq!(second.scene().meshes().len(), 1);
    Ok(())
}

#[test]
fn facade_detects_obj_content_with_unknown_extension() -> Result<()> {
    let report = Importer::new()?.read_bytes("facade.mesh", triangle_obj())?;

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert_eq!(report.scene().meshes().len(), 1);
    Ok(())
}

#[test]
fn facade_preserves_optional_dependency_evidence() -> Result<()> {
    let obj_path = AssetPath::new("models/missing-mtl.obj")?;
    let obj = b"mtllib missing.mtl
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
    let io = MemoryAssetIo::new();
    io.try_insert(obj_path.clone(), Arc::from(obj.as_slice()))
        .expect("the in-memory OBJ index should reserve one entry");

    let importer = Importer::new()?;
    let options = ImportRequest::from_layers(
        ImportRequestLayer::default(),
        ImportRequestLayer::new(ImportConfigOverlay::default().with_loss_policy(
            LossPolicy::preserve_or_fail().allow(ImportLossKind::OptionalDependencyUnavailable),
        )),
    );
    let request = importer.prepare_request(&options, OptionCompileLimits::default())?;
    let report = importer.read_asset_with_request(
        &io,
        obj_path,
        &request,
        ImportOperationControl::default(),
    )?;

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert_eq!(report.scene().meshes().len(), 1);
    assert_eq!(report.optional_dependency_evidence().len(), 1);
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "obj.mtl_unavailable" })
    );
    Ok(())
}

#[test]
fn repeated_oversized_mtl_references_do_not_refund_read_budget() -> Result<()> {
    let obj_path = AssetPath::new("models/repeated-mtl.obj")?;
    let mtl_path = AssetPath::new("models/material.mtl")?;
    let obj = b"mtllib material.mtl\nmtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl red\nKd 1 0 0\n";
    let io = MemoryAssetIo::new();
    io.try_insert(obj_path.clone(), Arc::from(obj.as_slice()))
        .expect("the in-memory OBJ index should reserve one entry");
    io.try_insert(mtl_path, Arc::from(mtl.as_slice()))
        .expect("the in-memory MTL index should reserve one entry");
    let importer = Importer::new()?;
    let mut limits = ResourceLimits::default();
    limits.max_dependency_asset_bytes = 4;
    limits.max_total_asset_bytes = obj.len() as u64 + 4;
    let options = ImportRequest::from_layers(
        ImportRequestLayer::default(),
        ImportRequestLayer::new(
            ImportConfigOverlay::default()
                .with_loss_policy(
                    LossPolicy::preserve_or_fail()
                        .allow(ImportLossKind::OptionalDependencyUnavailable),
                )
                .with_resource_limits(ResourceLimitsOverlay::from_limits(limits)),
        ),
    );
    let request = importer.prepare_request(&options, OptionCompileLimits::default())?;

    let report = importer.read_asset_with_request(
        &io,
        obj_path,
        &request,
        ImportOperationControl::default(),
    )?;

    assert_eq!(report.scene().meshes().len(), 1);
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .expect("asset-open accounting should exist")
            .consumed(),
        2
    );
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency-reference accounting should exist")
            .consumed(),
        2
    );
    assert_eq!(report.optional_dependency_evidence().len(), 2);
    assert_eq!(
        report
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "obj.mtl_unavailable")
            .count(),
        2
    );
    Ok(())
}

#[cfg(feature = "native-fs-candidate")]
#[test]
fn facade_reads_obj_from_path() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("baozi-obj-facade-{}-{stamp}", std::process::id()));
    let obj_path = root.join("path.obj");
    std::fs::create_dir(&root).expect("temporary OBJ directory should be creatable");
    std::fs::write(&obj_path, triangle_obj()).expect("temporary OBJ should be writable");

    let report = Importer::new()
        .expect("default registry should freeze")
        .read_path(&obj_path)
        .expect("native OBJ import should succeed");
    std::fs::remove_file(&obj_path).expect("temporary OBJ should be removable");
    std::fs::remove_dir(&root).expect("temporary OBJ directory should be removable");

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert_eq!(report.scene().meshes().len(), 1);
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
    let io = MemoryAssetIo::new();
    io.try_insert(obj_path.clone(), Arc::from(obj.as_slice()))
        .expect("the in-memory OBJ index should reserve one entry");
    io.try_insert(mtl_path, Arc::from(mtl.as_slice()))
        .expect("the in-memory MTL index should reserve one entry");

    let report = Importer::new()?.read_asset(&io, obj_path)?;

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.scene().materials()[0].name.as_deref(), Some("red"));
    match &report.scene().textures()[0].source {
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
    let importer = Importer::new()?;
    let report = importer.read_bytes("quad.obj", source)?;
    assert_eq!(report.last_completed().phase(), ImportPhase::Validated);
    assert_eq!(
        report.scene().meshes()[0].topology,
        PrimitiveTopology::Polygons
    );
    let detection_evidence = report.detection_evidence().to_vec();

    let pipeline = PostProcessPipeline::new([PostProcessStep::Triangulate])
        .expect("the static test post-process plan must be valid");
    let processed = importer
        .read_bytes_with_postprocess("quad.obj", source, &pipeline)
        .expect("post-process import should succeed");

    assert_eq!(
        processed.scene().meshes()[0].topology,
        PrimitiveTopology::Triangles
    );
    assert_eq!(
        processed.scene().meshes()[0].indices,
        vec![0, 1, 2, 0, 2, 3]
    );
    assert_eq!(processed.import().detection_evidence(), detection_evidence);
    assert_ne!(
        processed.import().scene().generation(),
        processed.scene().generation()
    );
    Ok(())
}

#[test]
fn postprocess_failure_retains_completed_import_evidence() -> Result<()> {
    let source = b"v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
f 1 2 3 4
";
    let importer = Importer::new()?;
    let report = importer.read_bytes("quad.obj", source)?;
    let imported_generation = report.scene().generation();
    let pipeline = PostProcessPipeline::new([PostProcessStep::Triangulate])
        .expect("the static test post-process plan must be valid")
        .with_limits(PostProcessLimits::new(0, 1024));

    let failure = Importer::process(report, &pipeline)
        .expect_err("zero post-process work budget should fail");

    assert_eq!(
        failure
            .completed_import()
            .expect("completed import should be retained")
            .scene()
            .generation(),
        imported_generation
    );
    assert!(failure.postprocess_failure().is_some());
    Ok(())
}
