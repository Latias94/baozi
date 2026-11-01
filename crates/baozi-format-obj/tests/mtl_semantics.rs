mod common;

use baozi_core::{
    MaterialBindingRef, MetadataValueRef, SamplerState, TextureAddressMode, TextureTransform, Vec2,
    Vec3,
};
use baozi_format_obj::ObjFailureCode;
use baozi_import::{
    ConformancePolicy, DiagnosticDetailValue, ImportCause, ImportFailure, ImportLossDisposition,
    ImportLossKind, ImportLossSubject, ImportOperationControl, ImportReport, LossPolicy,
    ResourceLimits, ScopeAbortCause, SourceScopeEntityKind,
};
use common::{TestResult, expected_failure, expected_success, import_assets_result_with_policies};

const OBJ: &[u8] = b"mtllib materials/palette.mtl\nusemtl paint\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0.25 0.75\nf 1/1 2/1 3/1\n";
const MAP_PATH: &str = "../textures/diffuse  tile.png";

fn import_material(
    statements: &str,
    conformance: ConformancePolicy,
    policy: LossPolicy,
) -> TestResult<Result<ImportReport, ImportFailure>> {
    let mtl = format!("newmtl paint\n{statements}\n");
    import_assets_result_with_policies(
        "model.obj",
        [
            ("model.obj", OBJ),
            ("materials/palette.mtl", mtl.as_bytes()),
            (
                "textures/diffuse  tile.png",
                b"encoded-test-image".as_slice(),
            ),
        ],
        ResourceLimits::default(),
        None,
        conformance,
        policy,
        ImportOperationControl::default(),
    )
}

fn binding(report: &ImportReport) -> MaterialBindingRef<'_> {
    assert_eq!(report.scene().material_count(), 1);
    let material = report.scene().materials().next().unwrap();
    assert_eq!(material.binding_count(), 1);
    material.bindings().next().unwrap()
}

fn assert_text_metadata(report: &ImportReport, key: &str, expected: &str) {
    let entry = report
        .scene()
        .metadata_entries()
        .find(|entry| entry.key() == key)
        .unwrap();
    assert!(matches!(entry.value(), MetadataValueRef::String(value) if value == expected));
}

fn assert_transform(actual: TextureTransform, offset: [f32; 2], scale: [f32; 2]) {
    assert_eq!(actual.offset(), Vec2::new(offset[0], offset[1]));
    assert_eq!(actual.scale(), Vec2::new(scale[0], scale[1]));
    assert_eq!(actual.rotation_radians(), 0.0);
}

#[test]
fn texture_defaults_partial_operands_and_order_have_typed_values() -> TestResult<()> {
    // The named Baozi OBJ/MTL UV Sampling Profile retains authored offset/scale directly.
    for (options, offset, scale) in [
        ("", [0.0, 0.0], [1.0, 1.0]),
        ("-o 0.25", [0.25, 0.0], [1.0, 1.0]),
        ("-o -0.25 0.5", [-0.25, 0.5], [1.0, 1.0]),
        ("-o 0.25 -0.5 0", [0.25, -0.5], [1.0, 1.0]),
        ("-s 2", [0.0, 0.0], [2.0, 1.0]),
        ("-s 2 0.5", [0.0, 0.0], [2.0, 0.5]),
        ("-s -2 0.5 1", [0.0, 0.0], [-2.0, 0.5]),
        ("-o 0.25 -0.5 -s 0.5 2", [0.25, -0.5], [0.5, 2.0]),
        ("-s 0.5 2 -o 0.25 -0.5", [0.25, -0.5], [0.5, 2.0]),
        // A repeated option replaces its whole tuple, including omitted-component defaults.
        ("-o 4 8 9 -s 2 4 8 -o 0.25 -s 0.5", [0.25, 0.0], [0.5, 1.0]),
    ] {
        let source = format!("{options} {MAP_PATH}");
        let source = source.trim_start();
        let report = expected_success(import_material(
            &format!("map_Kd {source}"),
            ConformancePolicy::Strict,
            LossPolicy::preserve_or_fail(),
        )?);
        assert!(report.loss_evidence().is_empty(), "{options}");
        assert!(report.diagnostics().is_empty(), "{options}");
        let binding = binding(&report);
        assert_transform(binding.transform(), offset, scale);
        assert_eq!(binding.uv_set(), 0);
        assert_eq!(binding.source_key(), Some("map_Kd"));
        assert_eq!(
            binding.texture().sampler().state(),
            SamplerState::CANONICAL_DEFAULT
        );
        assert_eq!(report.scene().image_count(), 1);
        assert_text_metadata(&report, "obj:map_kd", source);
    }
    Ok(())
}

#[test]
fn named_uv_sampling_profile_scales_source_uv_before_adding_offset() -> TestResult<()> {
    // This verifies the chosen Baozi profile, not an equation uniquely specified by the manual.
    let report = expected_success(import_material(
        &format!("map_Kd -o 0.25 -0.5 -s 2 0.5 {MAP_PATH}"),
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    )?);
    let transform = binding(&report).transform();
    assert_transform(transform, [0.25, -0.5], [2.0, 0.5]);
    // The fixture's authored vt (.25, .75) samples at (.75, -.125): q = o + s * p.
    let sampled_u = transform.offset().x + transform.scale().x * 0.25;
    let sampled_v = transform.offset().y + transform.scale().y * 0.75;
    assert_eq!(sampled_u, 0.75);
    assert_eq!(sampled_v, -0.125);
    // The source UV origin samples at the authored offset, with no V flip or center pivot.
    assert_eq!(transform.offset(), Vec2::new(0.25, -0.5));
    assert_text_metadata(
        &report,
        "obj:map_kd",
        &format!("-o 0.25 -0.5 -s 2 0.5 {MAP_PATH}"),
    );
    Ok(())
}

#[test]
fn zero_and_extreme_finite_uv_components_are_retained_without_loss() -> TestResult<()> {
    for (options, offset, scale) in [
        ("-s 0", [0.0, 0.0], [0.0, 1.0]),
        ("-s 0 0", [0.0, 0.0], [0.0, 0.0]),
        ("-o -0 -s -0 1e-45", [-0.0, 0.0], [-0.0, f32::from_bits(1)]),
        ("-s 1.17549435e-38", [0.0, 0.0], [f32::MIN_POSITIVE, 1.0]),
        ("-s 3.4028235e38", [0.0, 0.0], [f32::MAX, 1.0]),
        ("-o 3.4028235e38 -s 0.5", [f32::MAX, 0.0], [0.5, 1.0]),
    ] {
        for conformance in [
            ConformancePolicy::Strict,
            ConformancePolicy::AllowDeclaredRecovery,
        ] {
            let source = format!("{options} {MAP_PATH}");
            let report = expected_success(import_material(
                &format!("map_Kd {source}"),
                conformance,
                LossPolicy::preserve_or_fail(),
            )?);
            let transform = binding(&report).transform();
            assert_transform(transform, offset, scale);
            assert_eq!(transform.offset().x.to_bits(), offset[0].to_bits());
            assert_eq!(transform.offset().y.to_bits(), offset[1].to_bits());
            assert_eq!(transform.scale().x.to_bits(), scale[0].to_bits());
            assert_eq!(transform.scale().y.to_bits(), scale[1].to_bits());
            assert!(report.loss_evidence().is_empty(), "{options}");
            assert!(report.diagnostics().is_empty(), "{options}");
            assert_text_metadata(&report, "obj:map_kd", &source);
        }
    }
    Ok(())
}

#[test]
fn clamp_on_uses_decal_coverage_and_off_repeats_in_uv() -> TestResult<()> {
    for (options, expected) in [
        ("", TextureAddressMode::Repeat),
        ("-clamp off", TextureAddressMode::Repeat),
        ("-clamp on", TextureAddressMode::Decal),
        ("-clamp on -clamp off", TextureAddressMode::Repeat),
        ("-clamp off -clamp on", TextureAddressMode::Decal),
    ] {
        let report = expected_success(import_material(
            &format!("map_Kd {options} {MAP_PATH}"),
            ConformancePolicy::Strict,
            LossPolicy::preserve_or_fail(),
        )?);
        let sampler = binding(&report).texture().sampler().state();
        let default = SamplerState::CANONICAL_DEFAULT;
        assert_eq!(sampler.address_u(), expected);
        assert_eq!(sampler.address_v(), expected);
        assert_eq!(sampler.address_w(), default.address_w());
        assert_eq!(sampler.minification(), default.minification());
        assert_eq!(sampler.magnification(), default.magnification());
        assert_eq!(sampler.mipmap(), default.mipmap());
        assert!(report.loss_evidence().is_empty());
    }
    Ok(())
}

#[test]
fn decal_addressing_is_distinct_in_snapshot() -> TestResult<()> {
    let report = expected_success(import_material(
        &format!("map_Kd -clamp on {MAP_PATH}"),
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    )?);
    let repeating = expected_success(import_material(
        &format!("map_Kd -clamp off {MAP_PATH}"),
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    )?);
    let before = common::scene_snapshot(&report).unwrap();
    let repeating_snapshot = common::scene_snapshot(&repeating).unwrap();
    assert_ne!(before.digest(), repeating_snapshot.digest());
    assert!(before.preview().contains("u=decal v=decal"));
    Ok(())
}

#[test]
fn recognized_texture_omissions_require_semantic_permission() -> TestResult<()> {
    for (options, code, option) in [
        ("-t 0.1 0.2 0.3", "obj.mtl_texture_turbulence_omitted", "-t"),
        ("-t 0", "obj.mtl_texture_turbulence_omitted", "-t"),
        ("-mm 0.25 2", "obj.mtl_texture_range_omitted", "-mm"),
        ("-mm 0 1", "obj.mtl_texture_range_omitted", "-mm"),
        ("-bm -0.5", "obj.mtl_texture_bump_multiplier_omitted", "-bm"),
        ("-boost 2", "obj.mtl_texture_boost_omitted", "-boost"),
        ("-type sphere", "obj.mtl_texture_type_omitted", "-type"),
        (
            "-texres 256",
            "obj.mtl_texture_resolution_omitted",
            "-texres",
        ),
        ("-o 0 0 0.5", "obj.mtl_texture_offset_w_omitted", "-o"),
        ("-s 1 1 2", "obj.mtl_texture_scale_w_omitted", "-s"),
    ] {
        let statement = format!("map_Kd {options} {MAP_PATH}");
        for conformance in [
            ConformancePolicy::Strict,
            ConformancePolicy::AllowDeclaredRecovery,
        ] {
            for policy in [
                LossPolicy::preserve_or_fail(),
                LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery),
            ] {
                let failure = expected_failure(import_material(&statement, conformance, policy)?);
                assert_eq!(failure.ledger().committed_canonical_members(), 0);
                assert_eq!(failure.loss_evidence().len(), 1, "{options}");
                let loss = &failure.loss_evidence()[0];
                assert_eq!(loss.kind(), ImportLossKind::SourceSemanticLoss);
                assert_eq!(loss.code().as_str(), code);
                assert_eq!(
                    loss.disposition(),
                    ImportLossDisposition::RejectedByLossPolicy
                );
                assert!(
                    matches!(failure.cause(), ImportCause::Scope(abort) if matches!(
                        abort.cause(), ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { .. }
                    ))
                );
            }
            let report = expected_success(import_material(
                &statement,
                conformance,
                LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
            )?);
            assert_eq!(report.loss_evidence().len(), 1, "{options}");
            let loss = &report.loss_evidence()[0];
            assert_eq!(loss.kind(), ImportLossKind::SourceSemanticLoss);
            assert_eq!(loss.code().as_str(), code);
            assert_eq!(loss.disposition(), ImportLossDisposition::Applied);
            assert_eq!(
                loss.subject(),
                ImportLossSubject::SourceEntity(SourceScopeEntityKind::Material)
            );
            // The public omission recorder is bound to the primary source; dependency identity
            // is retained explicitly in the diagnostic rather than inferred from this field.
            assert_eq!(loss.source().as_str(), "model.obj");
            let diagnostic = report
                .diagnostics()
                .iter()
                .find(|item| item.code().as_str() == code)
                .unwrap();
            let path = format!("/dependencies/materials/palette.mtl/records/2/{option}");
            assert_eq!(diagnostic.object_path(), Some(path.as_str()));
            assert!(diagnostic.details().iter().any(|detail| {
                detail.key() == "dependency"
                    && matches!(detail.value(), DiagnosticDetailValue::String(value) if value == "materials/palette.mtl")
            }));
            assert!(diagnostic.details().iter().any(|detail| {
                detail.key() == "line"
                    && matches!(detail.value(), DiagnosticDetailValue::Unsigned(2))
            }));
            assert_eq!(report.scene().texture_count(), 1);
            assert_eq!(binding(&report).transform(), TextureTransform::IDENTITY);
            assert_text_metadata(&report, "obj:map_kd", &format!("{options} {MAP_PATH}"));
        }
    }
    Ok(())
}

#[test]
fn authorized_omissions_keep_supported_components_and_each_loss() -> TestResult<()> {
    let report = expected_success(import_material(
        &format!("map_Kd -t 1 -mm 0 2 -o 0.25 -0.5 3 -s 0.5 2 4 -clamp on {MAP_PATH}"),
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
    )?);
    assert_eq!(report.loss_evidence().len(), 4);
    assert!(
        report
            .loss_evidence()
            .iter()
            .all(|loss| { loss.disposition() == ImportLossDisposition::Applied })
    );
    assert_transform(binding(&report).transform(), [0.25, -0.5], [0.5, 2.0]);
    assert_eq!(
        binding(&report).texture().sampler().state().address_u(),
        TextureAddressMode::Decal
    );
    Ok(())
}

#[test]
fn malformed_map_arguments_remain_format_failures_with_loss_permission() -> TestResult<()> {
    for arguments in [
        "",
        "-o",
        "-s",
        "-clamp",
        "-t",
        "-mm 1",
        "-bm",
        "-boost",
        "-type",
        "-texres",
        "-o nope image.png",
        "-s -clamp on image.png",
        "-clamp yes image.png",
        "-o NaN image.png",
        "-o 1e100 image.png",
        "-s 3.4028236e38 image.png",
        "-s 1 inf image.png",
        "-s 1 1 -inf image.png",
        "-t NaN image.png",
        "-mm 0 NaN image.png",
        "-bm inf image.png",
        "-boost -1 image.png",
        "-boost NaN image.png",
        "-type cylinder image.png",
        "-texres 0 image.png",
        "-texres 1.5 image.png",
        "-texres -1 image.png",
        "-texres 4294967296 image.png",
        "-o 0 0 0 4 image.png",
        "-o 0 0 0 -1 2 image.png",
        "-o 0 0 0 -inf 2 image.png",
        "-mm 0 1 2 image.png",
        // Complete syntax validation precedes even the first recognized omission.
        "-t 1 -clamp maybe image.png",
    ] {
        for conformance in [
            ConformancePolicy::Strict,
            ConformancePolicy::AllowDeclaredRecovery,
        ] {
            let failure = expected_failure(import_material(
                &format!("map_Kd {arguments}"),
                conformance,
                LossPolicy::preserve_or_fail()
                    .allow(ImportLossKind::SourceSemanticLoss)
                    .allow(ImportLossKind::FormatRecovery),
            )?);
            let ImportCause::Format(envelope) = failure.cause() else {
                panic!(
                    "expected malformed map format failure for {arguments}: {:?}",
                    failure.cause()
                );
            };
            assert_eq!(envelope.code(), ObjFailureCode::InvalidStatement.as_str());
            assert!(failure.loss_evidence().is_empty(), "{arguments}");
            assert_eq!(failure.ledger().committed_canonical_members(), 0);
        }
    }
    Ok(())
}

#[test]
fn ambient_and_specular_rgb_factors_and_exact_source_metadata_are_retained() -> TestResult<()> {
    let report = expected_success(import_material(
        "Ka 0.2500 0.5 2.0\nKs 0.75\nKd 0.125 0.25 0.5",
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    )?);
    let factors = report
        .scene()
        .materials()
        .next()
        .unwrap()
        .factors()
        .legacy()
        .unwrap();
    assert_eq!(factors.ambient(), Vec3::new(0.25, 0.5, 2.0));
    assert_eq!(factors.specular(), Vec3::new(0.75, 0.75, 0.75));
    assert_eq!(factors.diffuse(), Vec3::new(0.125, 0.25, 0.5));
    assert_text_metadata(&report, "obj:ka", "0.2500 0.5 2.0");
    assert_text_metadata(&report, "obj:ks", "0.75");
    assert!(report.loss_evidence().is_empty());
    Ok(())
}

#[test]
fn optional_legacy_rgb_components_use_red_as_their_default() -> TestResult<()> {
    for (statement, ambient, specular) in [
        ("Ka 0.25", Vec3::new(0.25, 0.25, 0.25), Vec3::ZERO),
        ("Ks 0.25 0.5", Vec3::ZERO, Vec3::new(0.25, 0.5, 0.25)),
    ] {
        let report = expected_success(import_material(
            statement,
            ConformancePolicy::Strict,
            LossPolicy::preserve_or_fail(),
        )?);
        let factors = report
            .scene()
            .materials()
            .next()
            .unwrap()
            .factors()
            .legacy()
            .unwrap();
        assert_eq!(factors.ambient(), ambient);
        assert_eq!(factors.specular(), specular);
        assert!(report.loss_evidence().is_empty());
    }
    Ok(())
}

#[test]
fn nonrepresentable_legacy_colors_require_semantic_authority() -> TestResult<()> {
    for (statement, code) in [
        ("Ka -0.25 0.5 1", "obj.mtl_ambient_omitted"),
        ("Ks 0.25 -0.5 1", "obj.mtl_specular_omitted"),
    ] {
        for conformance in [
            ConformancePolicy::Strict,
            ConformancePolicy::AllowDeclaredRecovery,
        ] {
            for policy in [
                LossPolicy::preserve_or_fail(),
                LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery),
            ] {
                let failure = expected_failure(import_material(statement, conformance, policy)?);
                assert_eq!(failure.loss_evidence()[0].code().as_str(), code);
                assert_eq!(
                    failure.loss_evidence()[0].disposition(),
                    ImportLossDisposition::RejectedByLossPolicy
                );
            }
            let report = expected_success(import_material(
                statement,
                conformance,
                LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
            )?);
            assert_eq!(
                report.loss_evidence()[0].disposition(),
                ImportLossDisposition::Applied
            );
            let factors = report
                .scene()
                .materials()
                .next()
                .unwrap()
                .factors()
                .legacy()
                .unwrap();
            assert_eq!(factors.ambient(), Vec3::ZERO);
            assert_eq!(factors.specular(), Vec3::ZERO);
            let (keyword, source) = statement.split_once(' ').unwrap();
            assert_text_metadata(
                &report,
                if keyword == "Ka" { "obj:ka" } else { "obj:ks" },
                source,
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_legacy_colors_are_not_authorized_as_semantic_loss() -> TestResult<()> {
    for statement in ["Ka", "Ks 1 nope 0", "Ka NaN", "Ks 0 1 inf", "Ka 1 2 3 4"] {
        for conformance in [
            ConformancePolicy::Strict,
            ConformancePolicy::AllowDeclaredRecovery,
        ] {
            let failure = expected_failure(import_material(
                statement,
                conformance,
                LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
            )?);
            assert!(matches!(failure.cause(), ImportCause::Format(_)));
            assert!(failure.loss_evidence().is_empty());
        }
    }
    Ok(())
}

#[test]
fn duplicate_materials_replace_the_whole_definition_instead_of_merging_fields() -> TestResult<()> {
    let first = format!("Ka 0.25\nKs 0.5\nmap_Kd -o 0.25 -s 2 -clamp on {MAP_PATH}");
    let last = "newmtl paint\nKd 0.125 0.25 0.5";
    let report = expected_success(import_material(
        &format!("{first}\n{last}"),
        ConformancePolicy::Strict,
        LossPolicy::preserve_or_fail(),
    )?);
    let material = report.scene().materials().next().unwrap();
    let factors = material.factors().legacy().unwrap();
    assert_eq!(factors.ambient(), Vec3::ZERO);
    assert_eq!(factors.specular(), Vec3::ZERO);
    assert_eq!(factors.diffuse(), Vec3::new(0.125, 0.25, 0.5));
    assert_eq!(material.binding_count(), 0);
    assert_eq!(report.scene().texture_count(), 0);
    assert!(
        !report
            .scene()
            .metadata_entries()
            .any(|entry| { matches!(entry.key(), "obj:ka" | "obj:ks" | "obj:map_kd") })
    );
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "obj.mtl_duplicate_material"
    );
    Ok(())
}
