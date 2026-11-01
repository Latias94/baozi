#![cfg(feature = "format-ply")]

use baozi::{
    AttributeData, AttributeDomain, AttributeSemanticRef, ConformancePolicy, DiagnosticDetailValue,
    Error, ImportCause, ImportConfigOverlay, ImportLossDisposition, ImportLossKind,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, Importer, LossPolicy,
    OptionCompileLimits, Result, ScopeAbortCause,
};

#[derive(Clone, Copy)]
enum Encoding {
    Ascii,
    LittleEndian,
    BigEndian,
}

const ENCODINGS: [Encoding; 3] = [Encoding::Ascii, Encoding::LittleEndian, Encoding::BigEndian];
type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn fixture(encoding: Encoding, scalar: &str, rows: &[[f64; 4]], aliases: bool) -> Vec<u8> {
    let format = match encoding {
        Encoding::Ascii => "ascii",
        Encoding::LittleEndian => "binary_little_endian",
        Encoding::BigEndian => "binary_big_endian",
    };
    let mut bytes = format!(
        "ply\nformat {format} 1.0\nelement vertex {}\nproperty float x\nproperty float y\nproperty float z\n",
        rows.len()
    )
    .into_bytes();
    for name in if aliases {
        ["r", "g", "b", "a"]
    } else {
        ["red", "green", "blue", "alpha"]
    } {
        bytes.extend_from_slice(format!("property {scalar} {name}\n").as_bytes());
    }
    bytes.extend_from_slice(b"end_header\n");
    for row in rows {
        if matches!(encoding, Encoding::Ascii) {
            bytes.extend_from_slice(
                format!("0 0 0 {} {} {} {}\n", row[0], row[1], row[2], row[3]).as_bytes(),
            );
            continue;
        }
        bytes.extend_from_slice(&[0; 12]);
        for &value in row {
            macro_rules! append {
                ($value:expr) => {
                    bytes.extend_from_slice(&match encoding {
                        Encoding::LittleEndian => $value.to_le_bytes(),
                        Encoding::BigEndian => $value.to_be_bytes(),
                        Encoding::Ascii => unreachable!(),
                    })
                };
            }
            match scalar {
                "uchar" => bytes.push(value as u8),
                "ushort" => append!(value as u16),
                "uint" => append!(value as u32),
                "char" => bytes.push(value as i8 as u8),
                "short" => append!(value as i16),
                "int" => append!(value as i32),
                "float" => append!(value as f32),
                "double" => append!(value),
                _ => panic!("unexpected fixture scalar"),
            }
        }
    }
    bytes
}

#[allow(
    clippy::result_large_err,
    reason = "the test inspects the facade's complete typed failure without changing its ownership"
)]
fn import(bytes: &[u8], loss: LossPolicy) -> Result<ImportReport> {
    let importer = Importer::new()?;
    let request = importer.prepare_request(
        &ImportRequest::from_layers(
            ImportRequestLayer::default(),
            ImportRequestLayer::new(
                ImportConfigOverlay::default()
                    .with_conformance_policy(ConformancePolicy::Strict)
                    .with_loss_policy(loss),
            ),
        ),
        OptionCompileLimits::default(),
    )?;
    importer.read_bytes_with_request(
        "colors.ply",
        bytes,
        &request,
        ImportOperationControl::default(),
    )
}

fn color_bits(report: &ImportReport) -> Vec<u32> {
    let primitive = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let color = primitive
        .attribute(AttributeSemanticRef::Color(0), AttributeDomain::Vertex)
        .unwrap();
    let AttributeData::DenseF32 { lanes: 4, values } = color.data() else {
        panic!("PLY colors must be dense F32 RGBA")
    };
    values.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn unsigned_color_profile_preserves_midpoints_and_alpha_in_every_encoding() -> TestResult {
    let rows = [
        [0.0, 0.0, 0.0, 4_294_967_295.0],
        [
            2_147_483_648.0,
            1_073_741_824.0,
            3_221_225_472.0,
            2_147_483_648.0,
        ],
        [4_294_967_295.0, 4_294_967_295.0, 4_294_967_295.0, 0.0],
    ];
    let expected = [
        0.0_f32, 0.0, 0.0, 1.0, 0.5, 0.25, 0.75, 0.5, 1.0, 1.0, 1.0, 0.0,
    ]
    .map(f32::to_bits);
    for encoding in ENCODINGS {
        for aliases in [false, true] {
            let report = import(
                &fixture(encoding, "uint", &rows, aliases),
                LossPolicy::preserve_or_fail(),
            )?;
            assert_eq!(color_bits(&report), expected);
            assert!(report.loss_evidence().is_empty());
            assert!(report.diagnostics().is_empty());
        }
        for (scalar, midpoint, expected_bits) in [
            ("uchar", 128.0, 0x3f00_8081),
            ("ushort", 32_768.0, 0x3f00_0080),
            // A F32 cast before the division rounds this numerator down, yielding 0x3f000000.
            ("uint", 2_147_483_776.0, 0x3f00_0001),
        ] {
            let report = import(
                &fixture(encoding, scalar, &[[midpoint; 4]], false),
                LossPolicy::preserve_or_fail(),
            )?;
            assert_eq!(color_bits(&report), [expected_bits; 4]);
        }
    }
    Ok(())
}

#[test]
fn floating_color_profile_keeps_exact_values_and_signed_zero() -> TestResult {
    for encoding in ENCODINGS {
        for scalar in ["float", "double"] {
            let report = import(
                &fixture(encoding, scalar, &[[-0.0, 0.25, 0.5, 1.0]], false),
                LossPolicy::preserve_or_fail(),
            )?;
            assert_eq!(
                color_bits(&report),
                [-0.0_f32, 0.25, 0.5, 1.0].map(f32::to_bits)
            );
            assert!(report.loss_evidence().is_empty());
        }
    }
    Ok(())
}

#[test]
fn signed_or_out_of_range_colors_are_rejected_without_clamping() {
    for encoding in ENCODINGS {
        for (scalar, value) in [
            ("char", 1.0),
            ("short", -1.0),
            ("int", 0.0),
            ("float", -0.25),
            ("float", 1.25),
            ("double", -f64::from_bits(1)),
            ("double", 1.0_f64.next_up()),
            ("double", f64::NAN),
            ("float", f64::INFINITY),
        ] {
            let Error::Import(failure) = import(
                &fixture(encoding, scalar, &[[value, 0.0, 0.0, 1.0]], false),
                LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
            )
            .expect_err("loss authority must not admit invalid color values") else {
                panic!("expected an import operation failure")
            };
            let ImportCause::Format(envelope) = failure.cause() else {
                panic!("expected a PLY format failure")
            };
            assert_eq!(envelope.code(), "ply.invalid_scalar");
            assert!(failure.loss_evidence().is_empty());
        }
    }
}

#[test]
fn inexact_double_colors_require_semantic_loss_permission_under_strict_conformance() -> TestResult {
    for encoding in ENCODINGS {
        for value in [0.1, f64::from_bits(1)] {
            let bytes = fixture(encoding, "double", &[[value, 0.0, 0.0, 1.0]], false);
            for policy in [
                LossPolicy::preserve_or_fail(),
                LossPolicy::preserve_or_fail().allow(ImportLossKind::FormatRecovery),
            ] {
                let Error::Import(failure) = import(&bytes, policy)
                    .expect_err("inexact double must require semantic loss permission")
                else {
                    panic!("expected import failure")
                };
                assert!(matches!(failure.cause(), ImportCause::Scope(abort)
                    if matches!(abort.cause(), ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { .. })));
                assert_eq!(
                    failure.loss_evidence()[0].disposition(),
                    ImportLossDisposition::RejectedByLossPolicy
                );
            }
            let report = import(
                &bytes,
                LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
            )?;
            assert_eq!(
                color_bits(&report),
                [value as f32, 0.0, 0.0, 1.0].map(f32::to_bits)
            );
            assert_eq!(report.loss_evidence().len(), 1);
            let loss = &report.loss_evidence()[0];
            assert_eq!(loss.kind(), ImportLossKind::SourceSemanticLoss);
            assert_eq!(loss.disposition(), ImportLossDisposition::Applied);
            assert_eq!(loss.code().as_str(), "ply.color_precision_loss");
            assert_eq!(
                report.diagnostics()[0].object_path(),
                Some("/elements/vertex/records/0/properties/red")
            );
            for (key, expected) in [
                ("source_f64_bits", value.to_bits()),
                ("output_f32_bits", u64::from((value as f32).to_bits())),
                ("vertex", 0),
            ] {
                assert_eq!(
                    report.diagnostics()[0]
                        .details()
                        .iter()
                        .find(|d| d.key() == key)
                        .unwrap()
                        .value(),
                    &DiagnosticDetailValue::Unsigned(expected)
                );
            }
        }
        let report = import(
            &fixture(
                encoding,
                "double",
                &[[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.1]],
                true,
            ),
            LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
        )?;
        let diagnostic = &report.diagnostics()[0];
        assert_eq!(
            diagnostic.object_path(),
            Some("/elements/vertex/records/1/properties/a")
        );
        for (key, value) in [
            ("vertex", DiagnosticDetailValue::Unsigned(1)),
            ("property", DiagnosticDetailValue::String("a".into())),
            (
                "source_f64_bits",
                DiagnosticDetailValue::Unsigned(0x3fb9_9999_9999_999a),
            ),
            (
                "output_f32_bits",
                DiagnosticDetailValue::Unsigned(0x3dcc_cccd),
            ),
        ] {
            assert_eq!(
                diagnostic
                    .details()
                    .iter()
                    .find(|d| d.key() == key)
                    .unwrap()
                    .value(),
                &value
            );
        }
    }
    Ok(())
}
