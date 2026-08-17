mod common;

use baozi_import::{
    ImportCause, ImportDimension, ImportTerminalCause, ImportTerminalFailureClass, ResourceLimits,
    SceneSnapshot, ScopeAbortCause,
};
use common::{
    TestResult, expected_failure, import_assets, import_assets_result, scene_snapshot,
    snapshot_line, snapshot_line_containing,
};

const TEXTURE_BYTES: &[u8] = b"snapshotted-obj-texture";

fn material_factor_bits(snapshot: &SceneSnapshot, material: usize, factor: usize) -> u32 {
    let line = snapshot_line(snapshot, &format!("material[{material}]"));
    let factors = line
        .split_once("factors(present,total=39)=[")
        .expect("material snapshot must expose canonical factors")
        .1
        .split_once(']')
        .expect("material factor list must terminate")
        .0;
    let encoded = factors
        .split(',')
        .nth(factor)
        .unwrap_or_else(|| panic!("material factor {factor} is missing"))
        .trim()
        .strip_prefix("0x")
        .expect("material factor must be hexadecimal");
    u32::from_str_radix(encoded, 16).expect("material factor must be valid hexadecimal")
}

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
            ("materials/textures/diffuse.png", TEXTURE_BYTES),
        ],
        sidecar_limits(),
    )?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.material_count(), 1);
    assert_eq!(snapshot.image_count(), 1);
    assert_eq!(snapshot.sampler_count(), 1);
    assert_eq!(snapshot.texture_count(), 1);
    assert_eq!(snapshot.material_binding_count(), 1);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains("material=0"));

    let material = snapshot_line(&snapshot, "material[0]");
    assert!(material.contains("shading=2"));
    assert!(material.contains("alpha=2"));
    assert!(material.contains("binding_offset=0 binding_count=1"));
    assert!(material.contains("name=red"));
    assert_eq!(material_factor_bits(&snapshot, 0, 0), 0.1_f32.to_bits());
    assert_eq!(material_factor_bits(&snapshot, 0, 1), 0.2_f32.to_bits());
    assert_eq!(material_factor_bits(&snapshot, 0, 2), 0.3_f32.to_bits());
    assert_eq!(material_factor_bits(&snapshot, 0, 20), 0.8_f32.to_bits());
    assert_eq!(material_factor_bits(&snapshot, 0, 21), 0.1_f32.to_bits());
    assert_eq!(material_factor_bits(&snapshot, 0, 22), 0.2_f32.to_bits());
    assert_eq!(material_factor_bits(&snapshot, 0, 29), 32.0_f32.to_bits());
    assert_eq!(material_factor_bits(&snapshot, 0, 30), 0.5_f32.to_bits());

    let binding = snapshot_line(&snapshot, "binding[0]");
    assert!(binding.contains("texture=0 role=1"));
    assert!(binding.contains("coordinate_set=0 channels=4 color=0"));
    assert!(binding.contains("source_key=map_Kd"));
    assert!(snapshot_line(&snapshot, "texture[0]").contains("image=0 sampler=0"));
    assert!(
        snapshot_line(&snapshot, "image[0]")
            .contains(&format!("payload_bytes={}", TEXTURE_BYTES.len()))
    );
    assert!(snapshot_line_containing(&snapshot, "key=obj:illum").contains("i64=2"));
    assert!(
        snapshot_line_containing(&snapshot, "key=obj:ns").contains("f64_bits=0x4040000000000000")
    );
    assert!(
        snapshot_line_containing(&snapshot, "key=obj:ni").contains("f64_bits=0x3ff8000000000000")
    );
    let normal = snapshot_line_containing(&snapshot, "semantic=normal");
    assert!(normal.contains("lanes=3"));
    assert!(normal.contains("elements=3"));
    Ok(())
}

#[test]
fn texture_dependency_resolves_from_the_declaring_mtl_origin() -> TestResult<()> {
    let obj = b"mtllib ../materials/ship.mtl\nusemtl hull\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl hull\nmap_Kd ../textures/hull.png\n";
    let decoy = b"incorrect-primary-relative-texture";

    let report = import_assets(
        "assets/models/ship.obj",
        [
            ("assets/models/ship.obj", obj.as_slice()),
            ("assets/materials/ship.mtl", mtl.as_slice()),
            ("assets/textures/hull.png", TEXTURE_BYTES),
            ("assets/models/assets/textures/hull.png", decoy.as_slice()),
        ],
        sidecar_limits(),
    )?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.image_count(), 1);
    assert!(
        snapshot_line(&snapshot, "image[0]")
            .contains(&format!("payload_bytes={}", TEXTURE_BYTES.len()))
    );
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
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert!(snapshot_line(&snapshot, "material[0]").contains("name=red"));
    assert_eq!(material_factor_bits(&snapshot, 0, 20), 1.0_f32.to_bits());
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
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "obj.mtl_duplicate_material"
    );
    assert_eq!(snapshot.material_count(), 1);
    assert_eq!(material_factor_bits(&snapshot, 0, 20), 0.0_f32.to_bits());
    assert_eq!(material_factor_bits(&snapshot, 0, 21), 1.0_f32.to_bits());
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
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert!(snapshot_line(&snapshot, "material[0]").contains("name=red"));
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
    let snapshot = scene_snapshot(&report)?;
    let diagnostics = report.diagnostics();

    assert_eq!(snapshot.logical_mesh_count(), 1);
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
            ("textures/red.png", TEXTURE_BYTES),
        ],
        sidecar_limits(),
    )?;
    let snapshot = scene_snapshot(&report)?;
    let diagnostics = report.diagnostics();

    assert_eq!(snapshot.texture_count(), 1);
    assert!(snapshot_line(&snapshot, "binding[0]").contains("source_key=map_Kd"));
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
            ("textures/red.png", TEXTURE_BYTES),
        ],
        sidecar_limits(),
    )?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.texture_count(), 1);
    assert!(
        snapshot_line(&snapshot, "image[0]")
            .contains(&format!("payload_bytes={}", TEXTURE_BYTES.len()))
    );
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
            ("textures/red.png", TEXTURE_BYTES),
        ],
        sidecar_limits(),
    )?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.texture_count(), 1);
    assert!(snapshot_line(&snapshot, "binding[0]").contains("role=1"));
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
    let snapshot = scene_snapshot(&report)?;
    let diagnostics = report.diagnostics();

    assert_eq!(snapshot.logical_mesh_count(), 1);
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
    assert!(snapshot_line(&snapshot, "material[0]").contains("name=red"));
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
    let snapshot = scene_snapshot(&report)?;
    let diagnostics = report.diagnostics();

    assert_eq!(snapshot.logical_mesh_count(), 1);
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
    assert_eq!(scene_snapshot(&report)?.logical_mesh_count(), 1);
    assert_eq!(report.diagnostics().len(), 2);
    assert_eq!(
        report.diagnostics()[1].code().as_str(),
        "obj.mtl_unavailable"
    );
    Ok(())
}

#[test]
fn sidecar_total_byte_limit_is_terminal_after_recording_optional_loss_evidence() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl red\nKd 1 0 0\n";
    let mut limits = sidecar_limits();
    limits.max_total_asset_bytes = obj.len() as u64 + 4;

    let failure = expected_failure(import_assets_result(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        limits,
        true,
    )?);
    let terminal = match failure.cause() {
        ImportCause::OperationTerminal(terminal) => *terminal,
        cause => panic!("expected terminal ledger failure, got {cause:?}"),
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
    assert_eq!(
        failure.ledger().terminal_cause(),
        ImportTerminalCause::Failure(terminal)
    );
    let total = failure
        .ledger()
        .dimension(ImportDimension::TotalAssetBytes)
        .expect("aggregate asset-byte evidence remains queryable");
    assert_eq!(total.attempted(), (obj.len() + mtl.len()) as u64);
    assert_eq!(total.consumed(), obj.len() as u64);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert_eq!(failure.optional_dependency_evidence().len(), 1);
    assert_eq!(failure.diagnostics().len(), 2);
    assert_eq!(
        failure.diagnostics()[1].code().as_str(),
        "obj.mtl_unavailable"
    );
    Ok(())
}

#[test]
fn sidecar_open_asset_limit_is_terminal_after_recording_optional_loss_evidence() -> TestResult<()> {
    let obj = b"mtllib material.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let mtl = b"newmtl red\nKd 1 0 0\n";
    let mut limits = sidecar_limits();
    limits.max_open_assets = 1;

    let failure = expected_failure(import_assets_result(
        "model.obj",
        [
            ("model.obj", obj.as_slice()),
            ("material.mtl", mtl.as_slice()),
        ],
        limits,
        true,
    )?);
    let terminal = match failure.cause() {
        ImportCause::OperationTerminal(terminal) => *terminal,
        cause => panic!("expected terminal ledger failure, got {cause:?}"),
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
    assert_eq!(
        failure.ledger().terminal_cause(),
        ImportTerminalCause::Failure(terminal)
    );
    let opens = failure
        .ledger()
        .dimension(ImportDimension::AssetOpen)
        .expect("asset-open evidence remains queryable");
    assert_eq!(opens.attempted(), 2);
    assert_eq!(opens.consumed(), 1);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert_eq!(failure.optional_dependency_evidence().len(), 1);
    assert_eq!(failure.diagnostics().len(), 2);
    assert_eq!(
        failure.diagnostics()[1].code().as_str(),
        "obj.mtl_unavailable"
    );
    Ok(())
}
