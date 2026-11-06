mod common;

use baozi_core::{AttributeElement, AttributeSemanticRef};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::authoring::{AttributeDomain, MorphDeltaSemantic, Vec3};
use baozi_import::{
    FormatFailureDetailValue, ImportCause, ImportDimension, ImportFailure, ImportReport,
    ScopeAbortCause,
};
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    TestResult, assert_gltf_failure, controlled_import, expected_error, gltf_failure,
    import_assets, import_assets_result, import_assets_result_with_control, sidecar_options,
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

const SOURCE: &str = "models/directions.gltf";
const BINARY: &str = "models/directions.bin";

fn fixture() -> (Value, Vec<u8>) {
    fixture_with_count(3)
}

fn fixture_with_count(count: usize) -> (Value, Vec<u8>) {
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"mode": 4,
            "attributes": {"POSITION": 0, "NORMAL": 1, "TANGENT": 2}}]}],
        "buffers": [{"uri": "directions.bin", "byteLength": count * 40}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": count * 12},
            {"buffer": 0, "byteOffset": count * 12, "byteLength": count * 12},
            {"buffer": 0, "byteOffset": count * 24, "byteLength": count * 16}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
                "min": [0, 0, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": count, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": count, "type": "VEC4"}
        ]
    });
    let positions = (0..count).flat_map(|row| match row {
        1 => [1.0_f32, 0.0, 0.0],
        2 => [0.0, 1.0, 0.0],
        _ => [0.0; 3],
    });
    let binary = positions
        .chain(std::iter::repeat_n([0.0, 0.0, 1.0], count).flatten())
        .chain(std::iter::repeat_n([1.0, 0.0, 0.0, 1.0], count).flatten())
        .flat_map(f32::to_le_bytes)
        .collect();
    (document, binary)
}

fn direction_offset(accessor: usize, row: usize) -> usize {
    match accessor {
        1 => 36 + row * 12,
        2 => 72 + row * 16,
        _ => panic!("unexpected fixture direction accessor"),
    }
}

fn set_xyz(binary: &mut [u8], accessor: usize, row: usize, value: [f32; 3]) {
    let offset = direction_offset(accessor, row);
    for (lane, value) in value.into_iter().enumerate() {
        binary[offset + lane * 4..offset + lane * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn sparsify(document: &mut Value, binary: &mut Vec<u8>, accessor: usize, base: bool) {
    let offset = direction_offset(accessor, 0);
    let length = if accessor == 1 { 36 } else { 48 };
    let values = binary[offset..offset + length].to_vec();
    let indices_offset = binary.len();
    binary.extend_from_slice(&[0, 1, 2, 0]);
    let values_offset = binary.len();
    binary.extend_from_slice(&values);
    document["buffers"][0]["byteLength"] = json!(binary.len());
    document["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 0, "byteOffset": indices_offset, "byteLength": 3}),
        json!({"buffer": 0, "byteOffset": values_offset, "byteLength": length}),
    ]);
    document["accessors"][accessor]["sparse"] = json!({
        "count": 3, "indices": {"bufferView": 3, "componentType": 5121},
        "values": {"bufferView": 4}
    });
    if !base {
        document["accessors"][accessor]
            .as_object_mut()
            .unwrap()
            .remove("bufferView");
    }
}

fn import(document: &Value, binary: Vec<u8>) -> TestResult<ImportReport> {
    let (report, diagnostics) = import_assets(
        SOURCE,
        [(SOURCE, serde_json::to_vec(document)?), (BINARY, binary)],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    Ok(report)
}

fn failure(document: &Value, binary: Vec<u8>) -> TestResult<ImportFailure> {
    let (result, diagnostics) = import_assets_result(
        SOURCE,
        [(SOURCE, serde_json::to_vec(document)?), (BINARY, binary)],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    expected_error(result)
}

fn consumed(failure: &ImportFailure, dimension: ImportDimension) -> u64 {
    failure.ledger().dimension(dimension).unwrap().consumed()
}

fn assert_released(failure: &ImportFailure) {
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert!(
        failure.ledger().current_administrative_bytes()
            <= failure.ledger().peak_administrative_bytes()
    );
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
}

fn assert_accessor_index(failure: &ImportFailure, accessor: usize) -> TestResult<()> {
    assert!(gltf_failure(failure)?.details().iter().any(|detail| {
        detail.key() == "index"
            && matches!(detail.value(), FormatFailureDetailValue::Unsigned(value) if value == accessor as u64)
    }));
    Ok(())
}

#[test]
fn float_base_directions_reject_zero_and_nonunit_dense_and_sparse_values() -> TestResult<()> {
    for accessor in [1, 2] {
        for storage in 0..3 {
            for xyz in [
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 2.0],
                [0.5, 0.0, 0.0],
                [f32::MAX, f32::MAX, f32::MAX],
                [f32::MIN_POSITIVE, 0.0, 0.0],
                [f32::from_bits(1), 0.0, 0.0],
            ] {
                let (mut document, mut binary) = fixture();
                set_xyz(&mut binary, accessor, 2, xyz);
                if storage != 0 {
                    sparsify(&mut document, &mut binary, accessor, storage == 1);
                }
                let failure = failure(&document, binary)?;
                let label = if accessor == 1 {
                    "NORMAL"
                } else {
                    "TANGENT XYZ"
                };
                assert_gltf_failure(
                    &failure,
                    GltfFailureCode::InvalidAccessor,
                    Some(&format!("{label} must be unit length")),
                )?;
                assert_accessor_index(&failure, accessor)?;
                assert_eq!(consumed(&failure, ImportDimension::Vertices), 3);
                assert_eq!(
                    consumed(&failure, ImportDimension::AttributeCells),
                    if accessor == 1 { 5 } else { 8 }
                );
                assert_eq!(
                    consumed(&failure, ImportDimension::AttributeStreams),
                    accessor as u64
                );
                assert_eq!(consumed(&failure, ImportDimension::Primitives), 0);
                assert_released(&failure);
            }
        }
    }
    Ok(())
}

#[test]
fn float_direction_rejection_keeps_only_the_completed_sample_work_prefix() -> TestResult<()> {
    for accessor in [1, 2] {
        let mut prefixes = Vec::new();
        for row in [0, 2] {
            let (document, mut binary) = fixture();
            set_xyz(&mut binary, accessor, row, [0.0, 0.0, 2.0]);
            let failure = failure(&document, binary)?;
            assert_gltf_failure(
                &failure,
                GltfFailureCode::InvalidAccessor,
                Some("unit length"),
            )?;
            prefixes.push(consumed(&failure, ImportDimension::WorkUnits));
            assert_released(&failure);
        }
        // Two complete samples, each using the existing fixed 32-unit mesh envelope.
        assert_eq!(prefixes[1] - prefixes[0], 64);
    }
    Ok(())
}

#[test]
fn float_direction_tolerance_has_adjacent_f32_controls_on_both_boundaries() -> TestResult<()> {
    // Adjacent F32 values bracket sqrt(1 - 1e-4) and sqrt(1 + 1e-4).
    for accessor in [1, 2] {
        for (bits, accepted) in [
            (0x3f7f_fcb9, false),
            (0x3f7f_fcba, true),
            (0x3f80_01a3, true),
            (0x3f80_01a4, false),
            (0xbf7f_fcb9, false),
            (0xbf7f_fcba, true),
            (0xbf80_01a3, true),
            (0xbf80_01a4, false),
        ] {
            let (document, mut binary) = fixture();
            set_xyz(&mut binary, accessor, 1, [f32::from_bits(bits), 0.0, -0.0]);
            if accepted {
                let report = import(&document, binary)?;
                let primitive = report
                    .scene()
                    .meshes()
                    .next()
                    .unwrap()
                    .primitives()
                    .next()
                    .unwrap();
                let semantic = if accessor == 1 {
                    AttributeSemanticRef::Normal
                } else {
                    AttributeSemanticRef::Tangent
                };
                let Some(AttributeElement::F32(value)) = primitive
                    .attribute(semantic, AttributeDomain::Vertex)
                    .unwrap()
                    .element(1)
                else {
                    panic!("fixture direction must remain F32");
                };
                assert_eq!(value[0].to_bits(), bits);
                assert_eq!(value[2].to_bits(), (-0.0_f32).to_bits());
            } else {
                let failure = failure(&document, binary)?;
                assert_gltf_failure(
                    &failure,
                    GltfFailureCode::InvalidAccessor,
                    Some("unit length"),
                )?;
                assert_accessor_index(&failure, accessor)?;
                assert_released(&failure);
            }
        }
    }
    Ok(())
}

#[test]
fn cancellation_at_sample_256_preserves_prefix_before_direction_validation() -> TestResult<()> {
    const COUNT: usize = 257;
    for accessor in [1, 2] {
        let (mut document, valid_binary) = fixture_with_count(COUNT);
        document["meshes"][0]["primitives"][0]["mode"] = json!(0);
        let mut binary = valid_binary.clone();
        let offset = if accessor == 1 {
            COUNT * 12 + 256 * 12
        } else {
            COUNT * 24 + 256 * 16
        };
        for (lane, value) in [0.0_f32, 0.0, 2.0].into_iter().enumerate() {
            binary[offset + lane * 4..offset + lane * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let document = serde_json::to_vec(&document)?;
        let (control, polls) = controlled_import(None);
        let (result, _) = import_assets_result_with_control(
            SOURCE,
            [(SOURCE, document.clone()), (BINARY, binary.clone())],
            sidecar_options(),
            control,
        )?;
        let malformed = expected_error(result)?;
        assert_gltf_failure(
            &malformed,
            GltfFailureCode::InvalidAccessor,
            Some("unit length"),
        )?;
        let mut low = 1;
        let mut high = polls.load(Ordering::SeqCst);
        assert!(high > 0);
        let expected_cells = if accessor == 1 { 513 } else { 770 };
        // Locate the collector's row-256 poll rather than the later known-error exit poll.
        // The iterator may decode row 256; validation/storage/completion must not run.
        while low < high {
            let middle = low + (high - low) / 2;
            let (control, _) = controlled_import(Some(middle));
            let (result, _) = import_assets_result_with_control(
                SOURCE,
                [(SOURCE, document.clone()), (BINARY, binary.clone())],
                sidecar_options(),
                control,
            )?;
            let cancelled = expected_error(result)?;
            if consumed(&cancelled, ImportDimension::AttributeCells) >= expected_cells {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        let cancel_on = low;
        let mut work_prefix = None;
        for binary in [valid_binary, binary] {
            let (control, polls) = controlled_import(Some(cancel_on));
            let (result, diagnostics) = import_assets_result_with_control(
                SOURCE,
                [(SOURCE, document.clone()), (BINARY, binary)],
                sidecar_options(),
                control,
            )?;
            assert!(diagnostics.is_empty());
            let cancelled = expected_error(result)?;
            assert!(matches!(
                cancelled.cause(),
                ImportCause::Scope(abort) if matches!(
                    abort.cause(),
                    ScopeAbortCause::Control(ControlCause::Cancelled {
                        origin: CancellationOrigin::Signal,
                    })
                )
            ));
            assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
            assert_eq!(consumed(&cancelled, ImportDimension::Vertices), 257);
            assert_eq!(
                consumed(&cancelled, ImportDimension::AttributeCells),
                expected_cells
            );
            assert_eq!(
                consumed(&cancelled, ImportDimension::AttributeStreams),
                accessor as u64
            );
            assert_eq!(consumed(&cancelled, ImportDimension::Primitives), 0);
            let current = consumed(&cancelled, ImportDimension::WorkUnits);
            if let Some(expected) = work_prefix {
                assert_eq!(current, expected);
            }
            work_prefix = Some(current);
            assert_released(&cancelled);
        }
    }
    Ok(())
}

#[test]
fn declaring_quantization_does_not_exempt_float_base_directions() -> TestResult<()> {
    for accessor in [1, 2] {
        let (mut document, mut binary) = fixture();
        document["extensionsUsed"] = json!(["KHR_mesh_quantization"]);
        document["extensionsRequired"] = json!(["KHR_mesh_quantization"]);
        set_xyz(&mut binary, accessor, 2, [0.0, 0.0, 2.0]);
        let failure = failure(&document, binary)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::InvalidAccessor,
            Some("unit length"),
        )?;
        assert_accessor_index(&failure, accessor)?;
        assert_released(&failure);
    }
    Ok(())
}

#[test]
fn float_base_directions_preserve_authored_bits_and_sparse_replacement() -> TestResult<()> {
    for accessor in [1, 2] {
        for storage in 0..3 {
            let (mut document, mut binary) = fixture();
            let xyz = [0.6, -0.8, -0.0];
            set_xyz(&mut binary, accessor, 2, xyz);
            if accessor == 2 {
                binary[116..120].copy_from_slice(&(-1.0_f32).to_le_bytes());
            }
            if storage != 0 {
                sparsify(&mut document, &mut binary, accessor, storage == 1);
                // A replaced dense value is not the logical source direction.
                set_xyz(&mut binary, accessor, 2, [0.0; 3]);
            }
            let report = import(&document, binary)?;
            let primitive = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap();
            let semantic = if accessor == 1 {
                AttributeSemanticRef::Normal
            } else {
                AttributeSemanticRef::Tangent
            };
            let attribute = primitive
                .attribute(semantic, AttributeDomain::Vertex)
                .unwrap();
            let Some(AttributeElement::F32(value)) = attribute.element(2) else {
                panic!("fixture direction must remain F32");
            };
            for (actual, expected) in value.iter().zip(xyz) {
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
            if accessor == 2 {
                assert_eq!(value[3].to_bits(), (-1.0_f32).to_bits());
            }
        }
    }
    Ok(())
}

#[test]
fn float_tangent_sign_and_finite_errors_precede_unit_length_errors() -> TestResult<()> {
    for (xyz, sign, reason) in [
        ([0.0; 3], 0.0, "TANGENT W"),
        ([f32::NAN, 0.0, 0.0], 0.0, "finite"),
        ([1.0, 0.0, 0.0], f32::NAN, "finite"),
    ] {
        let (document, mut binary) = fixture();
        set_xyz(&mut binary, 2, 0, xyz);
        binary[84..88].copy_from_slice(&sign.to_le_bytes());
        let failure = failure(&document, binary)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidAccessor, Some(reason))?;
        assert_released(&failure);
    }
    Ok(())
}

#[test]
fn morph_direction_deltas_keep_zero_signed_and_nonunit_values() -> TestResult<()> {
    let (mut document, mut binary) = fixture();
    let deltas = [[0.0_f32, -0.0, 0.0], [-2.0, 0.5, 0.0], [0.0, 0.0, 2.0]];
    binary.extend(deltas.into_iter().flatten().flat_map(f32::to_le_bytes));
    document["buffers"][0]["byteLength"] = json!(binary.len());
    document["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer": 0, "byteOffset": 120, "byteLength": 36, "byteStride": 12
    }));
    document["accessors"].as_array_mut().unwrap().push(json!({
        "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC3"
    }));
    document["meshes"][0]["primitives"][0]["targets"] = json!([{"NORMAL": 3, "TANGENT": 3}]);
    let report = import(&document, binary)?;
    let primitive = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let target = primitive.morph_targets().next().unwrap();
    for semantic in [MorphDeltaSemantic::Normal, MorphDeltaSemantic::Tangent] {
        let values = target.delta(semantic).unwrap().data();
        for (row, [x, y, z]) in deltas.into_iter().enumerate() {
            let actual = values.value(row).unwrap();
            assert_eq!(actual, Vec3::new(x, y, z));
            assert_eq!(actual.x.to_bits(), x.to_bits());
            assert_eq!(actual.y.to_bits(), y.to_bits());
            assert_eq!(actual.z.to_bits(), z.to_bits());
        }
    }
    Ok(())
}
