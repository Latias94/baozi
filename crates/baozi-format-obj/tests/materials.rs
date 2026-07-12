mod common;

use baozi_core::{AlphaMode, ColorSpace, MetadataValue, TextureRole, TextureSource, Vec3};
use baozi_import::{ImportCause, ResourceLimits, ScopeAbortCause};
use common::{TestResult, expected_failure, import_assets, import_assets_result, scene};

fn sidecar_limits() -> ResourceLimits {
    ResourceLimits::default()
}

#[test]
fn loads_mtllib_sidecar_and_binds_usemtl() -> TestResult<()> {
    let obj = b"mtllib materials/cube.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\n";
    let mtl = b"newmtl red\nKd 0.8 0.1 0.2\nd 0.5\nKe 0.1 0.2 0.3\nNs 32\nNi 1.5\nillum 2\nmap_Kd -s 1 1 1 textures/diffuse.png\n";

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("materials/cube.mtl", mtl.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    assert_eq!(scene.materials().len(), 1);
    assert_eq!(scene.textures().len(), 1);
    assert_eq!(scene.meshes()[0].material.map(|id| id.index()), Some(0));

    let material = &scene.materials()[0];
    assert_eq!(material.name.as_deref(), Some("red"));
    assert_eq!(material.base_color.r, 0.8);
    assert_eq!(material.base_color.g, 0.1);
    assert_eq!(material.base_color.b, 0.2);
    assert_eq!(material.base_color.a, 0.5);
    assert_eq!(material.alpha_mode, AlphaMode::Blend);
    assert_eq!(
        material.emissive,
        baozi_core::Color::linear_rgba(0.1, 0.2, 0.3, 1.0)
    );
    assert_eq!(
        material.metadata.get("obj:illum"),
        Some(&MetadataValue::I64(2))
    );
    assert_eq!(
        material.metadata.get("obj:ns"),
        Some(&MetadataValue::F64(32.0))
    );
    assert_eq!(
        material.metadata.get("obj:ni"),
        Some(&MetadataValue::F64(1.5))
    );
    assert_eq!(material.texture_slots.len(), 1);
    assert_eq!(material.texture_slots[0].role, TextureRole::Diffuse);
    assert_eq!(material.texture_slots[0].color_space, ColorSpace::Srgb);
    assert_eq!(
        material.texture_slots[0].source_key.as_deref(),
        Some("map_Kd")
    );

    match &scene.textures()[0].source {
        TextureSource::External { uri } => {
            assert_eq!(uri, "materials/textures/diffuse.png");
        }
        other => panic!("expected external texture, got {other:?}"),
    }
    assert_eq!(scene.meshes()[0].normals[0], Vec3::new(0.0, 0.0, 1.0));
    Ok(())
}

#[test]
fn loads_multiple_mtllib_tokens_from_one_statement() -> TestResult<()> {
    let obj = b"mtllib unused.mtl material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let unused = b"newmtl blue\nKd 0 0 1\n";
    let material = b"newmtl red\nKd 1 0 0\n";

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("unused.mtl", unused.as_slice()),
            ("material.mtl", material.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    assert_eq!(scene.materials()[0].name.as_deref(), Some("red"));
    assert_eq!(scene.materials()[0].base_color.r, 1.0);
    Ok(())
}

#[test]
fn duplicate_materials_across_libraries_keep_the_last_definition() -> TestResult<()> {
    let obj = b"mtllib first.mtl second.mtl\nusemtl shared\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let first = b"newmtl shared\nKd 1 0 0\n";
    let second = b"newmtl shared\nKd 0 1 0\n";

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("first.mtl", first.as_slice()),
            ("second.mtl", second.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let scene = scene(&report);

    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "obj.mtl_duplicate_material"
    );
    assert_eq!(scene.materials().len(), 1);
    assert_eq!(scene.materials()[0].base_color.r, 0.0);
    assert_eq!(scene.materials()[0].base_color.g, 1.0);
    Ok(())
}

#[test]
fn mtl_utf8_bom_is_accepted() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let material = b"\xEF\xBB\xBFnewmtl red\nKd 1 0 0\n";

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", material.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    assert_eq!(scene.materials()[0].name.as_deref(), Some("red"));
    Ok(())
}

#[test]
fn unknown_mtl_statement_warns_and_keeps_geometry() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let material = b"newmtl red\nfoo bar\nKd 1 0 0\n";

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", material.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let scene = scene(&report);
    let diagnostics = report.diagnostics();

    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code().as_str(), "obj.mtl_unknown_statement");
    assert_eq!(diagnostics[0].source().as_str(), "material.mtl");
    Ok(())
}

#[test]
fn unsupported_texture_map_option_warns_but_finds_texture_path() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let material = b"newmtl red\nmap_Kd -unknown ignored textures/red.png\n";

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", material.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let scene = scene(&report);
    let diagnostics = report.diagnostics();

    assert_eq!(scene.textures().len(), 1);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code().as_str(),
        "obj.mtl_unsupported_texture_option"
    );
    Ok(())
}

#[test]
fn variable_length_texture_transform_options_do_not_swallow_texture_path() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let material = b"newmtl red\nmap_Kd -s 1 1 textures/red.png\n";

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", material.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    assert_eq!(scene.textures().len(), 1);
    match &scene.textures()[0].source {
        TextureSource::External { uri } => assert_eq!(uri, "textures/red.png"),
        other => panic!("expected external texture, got {other:?}"),
    }
    Ok(())
}

#[test]
fn negative_texture_transform_option_values_do_not_swallow_texture_path() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let material = b"newmtl red\nmap_Kd -o -1 0 textures/red.png\n";

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", material.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let scene = scene(&report);

    assert!(report.diagnostics().is_empty());
    assert_eq!(scene.textures().len(), 1);
    match &scene.textures()[0].source {
        TextureSource::External { uri } => assert_eq!(uri, "textures/red.png"),
        other => panic!("expected external texture, got {other:?}"),
    }
    Ok(())
}

#[test]
fn unavailable_mtl_is_explicit_warning_not_geometry_failure() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let report = import_assets(
        "model.obj",
        [("model.obj", obj.as_slice())],
        sidecar_limits(),
    )?;
    let scene = scene(&report);
    let diagnostics = report.diagnostics();

    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(diagnostics.len(), 3);
    assert!(
        diagnostics.iter().any(
            |diagnostic| diagnostic.code().as_str() == "import.optional_dependency_unavailable"
        )
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "obj.mtl_unavailable")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "obj.material_missing")
    );
    assert_eq!(scene.materials()[0].name.as_deref(), Some("red"));
    Ok(())
}

#[test]
fn missing_mtl_is_warning_not_geometry_failure() -> TestResult<()> {
    let obj = b"mtllib missing.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let report = import_assets(
        "model.obj",
        [("model.obj", obj.as_slice())],
        sidecar_limits(),
    )?;
    let scene = scene(&report);
    let diagnostics = report.diagnostics();

    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(
        diagnostics[0].code().as_str(),
        "import.optional_dependency_unavailable"
    );
    assert_eq!(diagnostics[1].code().as_str(), "obj.mtl_unavailable");
    assert_eq!(diagnostics[1].source().as_str(), "model.obj");
    Ok(())
}

#[test]
fn missing_mtl_remains_terminal_when_loss_is_not_authorized() -> TestResult<()> {
    let obj = b"mtllib missing.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let failure = expected_failure(import_assets_result(
        "model.obj",
        [("model.obj", obj.as_slice())],
        sidecar_limits(),
        false,
    )?);

    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::AssetOpen(_))
    ));
    assert!(failure.optional_dependency_evidence().is_empty());
    assert!(failure.diagnostics().is_empty());
    Ok(())
}

#[test]
fn sidecar_byte_limit_warns_and_keeps_geometry() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl red\nKd 1 0 0\n";
    let mut limits = sidecar_limits();
    limits.max_dependency_asset_bytes = 4;

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        limits,
    )?;
    assert_eq!(scene(&report).meshes().len(), 1);
    assert_eq!(report.diagnostics().len(), 2);
    assert_eq!(
        report.diagnostics()[1].code().as_str(),
        "obj.mtl_unavailable"
    );
    Ok(())
}

#[test]
fn sidecar_total_byte_limit_warns_and_keeps_geometry() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl red\nKd 1 0 0\n";
    let mut limits = sidecar_limits();
    limits.max_total_asset_bytes = obj.len() as u64 + 4;

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        limits,
    )?;
    assert_eq!(scene(&report).meshes().len(), 1);
    assert_eq!(report.diagnostics().len(), 2);
    assert_eq!(
        report.diagnostics()[1].code().as_str(),
        "obj.mtl_unavailable"
    );
    Ok(())
}

#[test]
fn sidecar_open_asset_limit_warns_and_keeps_geometry() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl red\nKd 1 0 0\n";
    let mut limits = sidecar_limits();
    limits.max_open_assets = 1;

    let report = import_assets(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        limits,
    )?;
    assert_eq!(scene(&report).meshes().len(), 1);
    assert_eq!(report.diagnostics().len(), 2);
    assert_eq!(
        report.diagnostics()[1].code().as_str(),
        "obj.mtl_unavailable"
    );
    Ok(())
}
