#![allow(
    clippy::result_large_err,
    reason = "facade tests preserve complete bounded terminal evidence by value"
)]
#![cfg(feature = "format-obj")]

mod common;

use baozi::{
    AssetPath, CapabilityStatus, FormatCapability, ImportConfigOverlay, ImportDimension,
    ImportLossKind, ImportOperationControl, ImportRequest, ImportRequestLayer, ImportTerminalCause,
    ImportTerminalFailureClass, Importer, LossPolicy, MemoryAssetIo, OptionCompileLimits,
    ResourceLimits, ResourceLimitsOverlay, Result,
};
use common::{scene_snapshot, snapshot_line};
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
    let snapshot = scene_snapshot(&report);

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains("topology=triangle_list"));
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
    let first_snapshot = scene_snapshot(&first);
    let second_snapshot = scene_snapshot(&second);

    assert_eq!(first.format().id().as_str(), "org.baozi.obj");
    assert_eq!(second.format().id().as_str(), "org.baozi.obj");
    assert_eq!(first_snapshot.logical_mesh_count(), 1);
    assert_eq!(second_snapshot.logical_mesh_count(), 1);
    Ok(())
}

#[test]
fn facade_detects_obj_content_with_unknown_extension() -> Result<()> {
    let report = Importer::new()?.read_bytes("facade.mesh", triangle_obj())?;
    let snapshot = scene_snapshot(&report);

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert_eq!(snapshot.logical_mesh_count(), 1);
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
    assert_eq!(scene_snapshot(&report).logical_mesh_count(), 1);
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
fn repeated_oversized_mtl_references_preserve_non_refundable_terminal_budget() -> Result<()> {
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

    let error = importer
        .read_asset_with_request(&io, obj_path, &request, ImportOperationControl::default())
        .expect_err("dependency-byte exhaustion must remain the terminal ledger cause");
    let baozi::Error::Import(failure) = error else {
        panic!("dependency-byte exhaustion must retain a complete import failure")
    };
    let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
        panic!("dependency-byte exhaustion cannot publish a successful terminal")
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
    assert_eq!(terminal.code(), 1);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .expect("asset-open accounting should exist")
            .consumed(),
        2
    );
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency-reference accounting should exist")
            .consumed(),
        2
    );
    let dependency_bytes = failure
        .ledger()
        .dimension(ImportDimension::DependencyBytes)
        .expect("dependency-byte accounting should exist");
    assert_eq!(dependency_bytes.attempted(), mtl.len() as u64);
    assert_eq!(dependency_bytes.consumed(), 0);
    assert_eq!(failure.optional_dependency_evidence().len(), 2);
    assert_eq!(
        failure
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "obj.mtl_unavailable")
            .count(),
        2
    );
    Ok(())
}

#[test]
fn facade_reads_obj_with_memory_mtl_sidecar() -> Result<()> {
    let obj_path = AssetPath::new("models/cube.obj")?;
    let mtl_path = AssetPath::new("models/materials/cube.mtl")?;
    let texture_path = AssetPath::new("models/materials/textures/red.png")?;
    let primary_relative_decoy = AssetPath::new("models/models/materials/textures/red.png")?;
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
    let texture = b"facade-red-texture";
    let decoy = b"wrong-primary-relative-texture-payload";
    let io = MemoryAssetIo::new();
    io.try_insert(obj_path.clone(), Arc::from(obj.as_slice()))
        .expect("the in-memory OBJ index should reserve one entry");
    io.try_insert(mtl_path, Arc::from(mtl.as_slice()))
        .expect("the in-memory MTL index should reserve one entry");
    io.try_insert(texture_path, Arc::from(texture.as_slice()))
        .expect("the in-memory texture index should reserve one entry");
    io.try_insert(primary_relative_decoy, Arc::from(decoy.as_slice()))
        .expect("the in-memory decoy texture index should reserve one entry");

    let report = Importer::new()?.read_asset(&io, obj_path)?;

    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert!(report.diagnostics().is_empty());
    let snapshot = scene_snapshot(&report);
    assert_eq!(snapshot.material_count(), 1);
    assert_eq!(snapshot.image_count(), 1);
    assert_eq!(snapshot.sampler_count(), 1);
    assert_eq!(snapshot.texture_count(), 1);
    assert_eq!(snapshot.material_binding_count(), 1);
    assert!(snapshot_line(&snapshot, "material[0]").contains("name=red"));
    assert!(snapshot_line(&snapshot, "binding[0]").contains("source_key=map_Kd"));
    assert!(
        snapshot_line(&snapshot, "image[0]").contains(&format!("payload_bytes={}", texture.len()))
    );
    Ok(())
}
