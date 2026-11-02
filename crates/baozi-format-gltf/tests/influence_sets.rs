mod common;

use baozi_core::ProvenanceKindRef;
use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    DiagnosticSeverity, ImportCause, ImportDimension, ImportFailure, ImportReport,
    SceneValidationError, ScopeAbortCause, SourceObjectLocatorRef, SourceScopeDisposition,
    SourceScopeEntityKind,
};
use common::{
    TestResult, assert_gltf_failure, assert_resource_limit, expected_error, import_assets,
    import_assets_result, sidecar_options, skinned_triangle_bin, skinned_triangle_gltf,
};
use serde_json::{Value, json};

const SOURCE: &str = "models/influence-sets.gltf";
const BASE: &str = "models/skin.bin";
const INDICES: &str = "models/higher-set-indices.bin";
const VALUES: &str = "models/higher-set-values.bin";

const JOINT_ROWS: [[u32; 8]; 3] = [
    [0, 1, 2, 3, 4, 5, 6, 7],
    [7, 6, 0, 0, 5, 4, 3, 0],
    [1, 0, 2, 0, 7, 6, 5, 0],
];
const WEIGHT_ROWS: [[f32; 8]; 3] = [
    [0.125, 0.25, 0.0625, 0.0625, 0.1875, 0.125, 0.0625, 0.125],
    [0.25, 0.125, -0.0, 0.0, 0.375, 0.125, 0.125, 0.0],
    [0.25, 0.0, 0.125, -0.0, 0.0625, 0.5, 0.0625, 0.0],
];
// Exact-rational nearest-F32 bits for the pinned spec's mixed-width weight example.
const SPEC_U8_WEIGHT_BITS: [u32; 4] = [0x3b80_8081, 0x3c80_8081, 0x3d80_8081, 0x3e80_8081];
const SPEC_U16_WEIGHT_BITS: [u32; 4] = [0x3c20_00a0, 0x3d02_0082, 0x3e00_2080, 0x3f00_0280];

fn two_set_triangle() -> TestResult<(Value, Vec<u8>)> {
    let mut document: Value = serde_json::from_slice(&skinned_triangle_gltf())?;
    let mut binary = skinned_triangle_bin();
    // Reuse the positions, indices, and first influence accessors of the skin fixture.
    // Identity bind matrices let the palette grow without unrelated matrix payloads.
    document["skins"][0]
        .as_object_mut()
        .unwrap()
        .remove("inverseBindMatrices");
    document["accessors"].as_array_mut().unwrap().truncate(4);
    document["bufferViews"].as_array_mut().unwrap().truncate(4);
    binary.truncate(116);
    document["skins"][0]["skeleton"] = json!(0);
    document["skins"][0]["joints"] = json!([1, 3, 4, 5, 6, 7, 8, 9]);
    for joint in 2..8 {
        let node = document["nodes"].as_array().unwrap().len();
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": format!("Joint{joint}")}));
        document["nodes"][0]["children"]
            .as_array_mut()
            .unwrap()
            .push(json!(node));
    }
    for (vertex, (joints, weights)) in JOINT_ROWS.iter().zip(&WEIGHT_ROWS).enumerate() {
        for lane in 0..4 {
            let joint = u16::try_from(joints[lane])?;
            let joint_offset = 42 + vertex * 8 + lane * 2;
            binary[joint_offset..joint_offset + 2].copy_from_slice(&joint.to_le_bytes());
            let weight_offset = 68 + vertex * 16 + lane * 4;
            binary[weight_offset..weight_offset + 4].copy_from_slice(&weights[lane].to_le_bytes());
        }
    }
    for row in JOINT_ROWS {
        for joint in &row[4..] {
            binary.extend_from_slice(&u16::try_from(*joint)?.to_le_bytes());
        }
    }
    for row in WEIGHT_ROWS {
        for weight in &row[4..] {
            binary.extend_from_slice(&weight.to_le_bytes());
        }
    }
    document["buffers"][0]["byteLength"] = json!(binary.len());
    document["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 0, "byteOffset": 116, "byteLength": 24, "target": 34962}),
        json!({"buffer": 0, "byteOffset": 140, "byteLength": 48, "target": 34962}),
    ]);
    document["accessors"].as_array_mut().unwrap().extend([
        json!({"bufferView": 4, "componentType": 5123, "count": 3, "type": "VEC4"}),
        json!({"bufferView": 5, "componentType": 5126, "count": 3, "type": "VEC4"}),
    ]);
    document["meshes"][0]["primitives"][0]["attributes"]["JOINTS_1"] = json!(4);
    document["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_1"] = json!(5);
    Ok((document, binary))
}

fn import_triangle(document: &Value, binary: Vec<u8>) -> TestResult<ImportReport> {
    let (report, diagnostics) = import_assets(
        SOURCE,
        [(SOURCE, serde_json::to_vec(document)?), (BASE, binary)],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Ok(report)
}

fn assert_rows(report: &ImportReport, expected: &[(&[u32], &[f32])]) {
    let primitive = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let rows = primitive.influences().expect("retained influence rows");
    assert_eq!(
        primitive.vertex_count(),
        u32::try_from(expected.len()).unwrap()
    );
    assert_eq!(rows.len(), expected.len());
    assert_eq!(rows.iter().len(), expected.len());
    for (vertex, (joints, weights)) in expected.iter().enumerate() {
        let row = rows.get(vertex).expect("vertex influence row");
        let orders = (0..u32::try_from(joints.len()).unwrap()).collect::<Vec<_>>();
        assert_eq!(row.vertex(), u32::try_from(vertex).unwrap());
        assert_eq!(row.len(), joints.len());
        assert_eq!(row.joint_indices(), *joints);
        assert_eq!(row.source_orders(), orders.as_slice());
        assert_eq!(
            row.weights()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            weights
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            "vertex {vertex} must preserve every authored weight bit"
        );
        for (lane, influence) in row.iter().enumerate() {
            assert_eq!(influence.palette_joint(), joints[lane]);
            assert_eq!(influence.weight().to_bits(), weights[lane].to_bits());
            assert_eq!(influence.source_order(), orders[lane]);
        }
        assert!(row.get(joints.len()).is_none());
    }
    assert!(rows.get(expected.len()).is_none());
}

fn assert_no_dependency_io(failure: &ImportFailure) {
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
    ] {
        assert_eq!(
            failure.ledger().dimension(dimension).unwrap().attempted(),
            0
        );
    }
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        1
    );
}

#[test]
fn two_sets_preserve_typed_rows_lane_order_and_signed_zero() -> TestResult<()> {
    let (document, binary) = two_set_triangle()?;
    let report = import_triangle(&document, binary)?;
    assert_rows(
        &report,
        &[
            (&JOINT_ROWS[0], &WEIGHT_ROWS[0]),
            (&JOINT_ROWS[1], &WEIGHT_ROWS[1]),
            (&JOINT_ROWS[2], &WEIGHT_ROWS[2]),
        ],
    );
    assert_eq!(report.scene().attribute_count(), 1);
    Ok(())
}

#[test]
fn three_and_seventeen_sets_retain_all_lanes_in_numeric_set_order() -> TestResult<()> {
    const JOINTS: [[u32; 12]; 3] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        [7, 6, 0, 0, 5, 4, 3, 0, 8, 9, 10, 11],
        [1, 0, 2, 0, 7, 6, 5, 0, 8, 9, 10, 11],
    ];
    const WEIGHTS: [[f32; 12]; 3] = [
        [
            0.0625, 0.125, 0.03125, 0.03125, 0.09375, 0.0625, 0.03125, 0.0625, 0.125, 0.125, 0.125,
            0.125,
        ],
        [
            0.125, 0.0625, -0.0, 0.0, 0.1875, 0.0625, 0.0625, 0.0, 0.125, 0.125, 0.125, 0.125,
        ],
        [
            0.125, 0.0, 0.0625, -0.0, 0.03125, 0.25, 0.03125, 0.0, 0.125, 0.125, 0.125, 0.125,
        ],
    ];
    for set_count in [3_usize, 17] {
        let (mut document, mut binary) = two_set_triangle()?;
        for vertex in 0..3 {
            for lane in 0..4 {
                for offset in [68 + vertex * 16 + lane * 4, 140 + vertex * 16 + lane * 4] {
                    let weight = f32::from_le_bytes(binary[offset..offset + 4].try_into()?);
                    binary[offset..offset + 4].copy_from_slice(&(weight * 0.5).to_le_bytes());
                }
            }
        }
        for _ in 0..3 {
            for joint in [8_u16, 9, 10, 11] {
                binary.extend_from_slice(&joint.to_le_bytes());
            }
        }
        for _ in 0..12 {
            binary.extend_from_slice(&0.125_f32.to_le_bytes());
        }
        for joint in 8..12 {
            let node = document["nodes"].as_array().unwrap().len();
            document["nodes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name": format!("Joint{joint}")}));
            document["nodes"][0]["children"]
                .as_array_mut()
                .unwrap()
                .push(json!(node));
            document["skins"][0]["joints"]
                .as_array_mut()
                .unwrap()
                .push(json!(node));
        }
        document["bufferViews"].as_array_mut().unwrap().extend([
            json!({"buffer": 0, "byteOffset": 188, "byteLength": 24, "target": 34962}),
            json!({"buffer": 0, "byteOffset": 212, "byteLength": 48, "target": 34962}),
        ]);
        document["accessors"].as_array_mut().unwrap().extend([
            json!({"bufferView": 6, "componentType": 5123, "count": 3, "type": "VEC4"}),
            json!({"bufferView": 7, "componentType": 5126, "count": 3, "type": "VEC4"}),
        ]);
        document["meshes"][0]["primitives"][0]["attributes"]["JOINTS_2"] = json!(6);
        document["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_2"] = json!(7);
        if set_count > 3 {
            // The remaining sets share zero accessors; their lanes must still be retained.
            binary.resize(332, 0);
            document["bufferViews"].as_array_mut().unwrap().extend([
                json!({"buffer": 0, "byteOffset": 260, "byteLength": 24, "target": 34962}),
                json!({"buffer": 0, "byteOffset": 284, "byteLength": 48, "target": 34962}),
            ]);
            document["accessors"].as_array_mut().unwrap().extend([
                json!({"bufferView": 8, "componentType": 5123, "count": 3, "type": "VEC4"}),
                json!({"bufferView": 9, "componentType": 5126, "count": 3, "type": "VEC4"}),
            ]);
            for set in 3..set_count {
                document["meshes"][0]["primitives"][0]["attributes"][format!("JOINTS_{set}")] =
                    json!(8);
                document["meshes"][0]["primitives"][0]["attributes"][format!("WEIGHTS_{set}")] =
                    json!(9);
            }
        }
        document["buffers"][0]["byteLength"] = json!(binary.len());
        let report = import_triangle(&document, binary)?;
        let expected_joints = JOINTS.map(|row| {
            let mut row = row.to_vec();
            row.resize(set_count * 4, 0);
            row
        });
        let expected_weights = WEIGHTS.map(|row| {
            let mut row = row.to_vec();
            row.resize(set_count * 4, 0.0);
            row
        });
        assert_rows(
            &report,
            &[
                (&expected_joints[0], &expected_weights[0]),
                (&expected_joints[1], &expected_weights[1]),
                (&expected_joints[2], &expected_weights[2]),
            ],
        );
        let cells = report
            .ledger()
            .dimension(ImportDimension::InfluenceCells)
            .unwrap();
        assert_eq!(cells.attempted(), u64::try_from(12 * set_count)?);
        assert_eq!(cells.consumed(), u64::try_from(12 * set_count)?);
    }
    Ok(())
}

#[test]
fn later_set_strided_accessors_skip_padding_and_keep_exact_weight_bits() -> TestResult<()> {
    let (mut document, mut binary) = two_set_triangle()?;
    binary.truncate(116);
    for (joints, weights) in JOINT_ROWS.iter().zip(&WEIGHT_ROWS) {
        binary.extend_from_slice(&[0xa5; 4]);
        for joint in &joints[4..] {
            binary.extend_from_slice(&u16::try_from(*joint)?.to_le_bytes());
        }
        binary.extend_from_slice(&[0xa5; 4]);
        for weight in &weights[4..] {
            binary.extend_from_slice(&weight.to_le_bytes());
        }
    }
    document["buffers"][0]["byteLength"] = json!(binary.len());
    document["bufferViews"][4] = json!({
        "buffer": 0, "byteOffset": 116, "byteLength": 96, "byteStride": 32, "target": 34962
    });
    document["accessors"][4]["byteOffset"] = json!(4);
    document["accessors"][5]["bufferView"] = json!(4);
    document["accessors"][5]["byteOffset"] = json!(16);
    document["bufferViews"].as_array_mut().unwrap().truncate(5);
    let report = import_triangle(&document, binary)?;
    assert_rows(
        &report,
        &[
            (&JOINT_ROWS[0], &WEIGHT_ROWS[0]),
            (&JOINT_ROWS[1], &WEIGHT_ROWS[1]),
            (&JOINT_ROWS[2], &WEIGHT_ROWS[2]),
        ],
    );
    Ok(())
}

#[test]
fn sparse_normalized_later_sets_retain_values_and_complete_typed_provenance() -> TestResult<()> {
    // The mixed U8/U16 numerators are the exact-sum example in the pinned glTF spec.
    for (low_component, high_component, joint_component, low, high, expected_bits) in [
        (
            5121,
            5123,
            5121,
            [1_u16, 4, 16, 64],
            [640_u16, 2080, 8200, 32770],
            [SPEC_U8_WEIGHT_BITS, SPEC_U16_WEIGHT_BITS],
        ),
        (
            5123,
            5121,
            5123,
            [640_u16, 2080, 8200, 32770],
            [1_u16, 4, 16, 64],
            [SPEC_U16_WEIGHT_BITS, SPEC_U8_WEIGHT_BITS],
        ),
    ] {
        let (mut document, mut binary) = two_set_triangle()?;
        let low_max = if low_component == 5121 {
            255_u16
        } else {
            65535
        };
        let mut low_bytes = Vec::new();
        for row in [[low_max, 0, 0, 0], low, low] {
            for value in row {
                if low_component == 5121 {
                    low_bytes.push(u8::try_from(value)?);
                } else {
                    low_bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        binary[68..68 + low_bytes.len()].copy_from_slice(&low_bytes);
        for vertex in 0..3 {
            for (lane, joint) in [0_u16, 1, 2, 3].into_iter().enumerate() {
                let offset = 42 + vertex * 8 + lane * 2;
                binary[offset..offset + 2].copy_from_slice(&joint.to_le_bytes());
            }
        }
        document["accessors"][3]["componentType"] = json!(low_component);
        document["accessors"][3]["normalized"] = json!(true);
        document["bufferViews"][3]["byteLength"] = json!(low_bytes.len());

        let indices = vec![0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 1, 2];
        let mut values = vec![0xa5; 8];
        for _ in 0..2 {
            for joint in [4_u16, 5, 6, 7] {
                if joint_component == 5121 {
                    values.push(u8::try_from(joint)?);
                } else {
                    values.extend_from_slice(&joint.to_le_bytes());
                }
            }
        }
        let weight_start = values.len();
        for _ in 0..2 {
            for value in high {
                if high_component == 5121 {
                    values.push(u8::try_from(value)?);
                } else {
                    values.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        document["buffers"].as_array_mut().unwrap().extend([
            json!({"uri": "higher-set-indices.bin", "byteLength": indices.len()}),
            json!({"uri": "higher-set-values.bin", "byteLength": values.len()}),
        ]);
        document["bufferViews"].as_array_mut().unwrap().extend([
            json!({"buffer": 1, "byteOffset": 4, "byteLength": 6}),
            json!({"buffer": 2, "byteOffset": 4, "byteLength": weight_start - 4}),
            json!({"buffer": 2, "byteOffset": weight_start - 4, "byteLength": values.len() - weight_start + 4}),
        ]);
        for (accessor, value_view) in [(4, 7), (5, 8)] {
            document["accessors"][accessor]
                .as_object_mut()
                .unwrap()
                .remove("bufferView");
            document["accessors"][accessor]["sparse"] = json!({
                "count": 2,
                "indices": {"bufferView": 6, "byteOffset": 4, "componentType": 5121},
                "values": {"bufferView": value_view, "byteOffset": 4}
            });
        }
        document["accessors"][4]["componentType"] = json!(joint_component);
        document["accessors"][5]["componentType"] = json!(high_component);
        document["accessors"][5]["normalized"] = json!(true);
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "SecondMesh", "mesh": 0, "skin": 0}));
        document["nodes"][0]["children"]
            .as_array_mut()
            .unwrap()
            .push(json!(10));
        let (report, diagnostics) = import_assets(
            SOURCE,
            [
                (SOURCE, serde_json::to_vec(&document)?),
                (BASE, binary),
                (INDICES, indices),
                (VALUES, values),
            ],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let scene = report.scene();
        assert_eq!(scene.mesh_count(), 1);
        assert_eq!(scene.mesh_instance_count(), 2);
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        let expected_weights = expected_bits
            .into_iter()
            .flatten()
            .map(f32::from_bits)
            .collect::<Vec<_>>();
        assert_rows(
            &report,
            &[
                (
                    &[0, 1, 2, 3, 0, 0, 0, 0],
                    &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                ),
                (&[0, 1, 2, 3, 4, 5, 6, 7], &expected_weights),
                (&[0, 1, 2, 3, 4, 5, 6, 7], &expected_weights),
            ],
        );

        let mut pending = scene
            .owner_provenance(primitive.metadata_owner())?
            .collect::<Vec<_>>();
        assert!(!pending.is_empty());
        let mut visited = Vec::new();
        let mut sources = Vec::new();
        while let Some(node) = pending.pop() {
            if visited.contains(&node.id()) {
                continue;
            }
            assert_eq!(node.id().generation(), scene.generation());
            assert_eq!(scene.provenance(node.id())?, node);
            visited.push(node.id());
            assert!(visited.len() <= scene.provenance_node_count());
            if let ProvenanceKindRef::Source(source) = node.kind() {
                assert_eq!(source.generation(), scene.generation());
                assert_eq!(source.resource().generation(), scene.generation());
                sources.push(source);
            }
            pending.extend(node.parents());
        }
        for prefix in [
            "kind=primitive;parent=0;index=0;",
            "kind=accessor;parent=-;index=4;",
            "kind=accessor;parent=-;index=5;",
            "kind=buffer-view;parent=-;index=6;",
            "kind=buffer-view;parent=-;index=7;",
            "kind=buffer-view;parent=-;index=8;",
            "kind=buffer;parent=-;index=1;",
            "kind=buffer;parent=-;index=2;",
        ] {
            let source = scene
                .source_identities()
                .find(|source| {
                    matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                    if value.starts_with(prefix))
                })
                .unwrap_or_else(|| panic!("missing higher-set source {prefix}"));
            assert_eq!(source.resource().identity(), SOURCE);
            assert!(
                sources.contains(&source),
                "primitive lost ancestry for {prefix}"
            );
        }
        for resource in [SOURCE, BASE, INDICES, VALUES] {
            let source = scene
                .source_identities()
                .find(|source| source.resource().identity() == resource)
                .unwrap_or_else(|| panic!("missing source resource {resource}"));
            assert!(
                sources.contains(&source),
                "primitive lost resource {resource}"
            );
        }
    }
    Ok(())
}

#[test]
fn shared_mesh_instances_keep_all_sets_with_independent_joint_palettes() -> TestResult<()> {
    let (mut document, binary) = two_set_triangle()?;
    let mut reversed_skin = document["skins"][0].clone();
    reversed_skin["name"] = json!("ReversedSkin");
    reversed_skin["joints"].as_array_mut().unwrap().reverse();
    document["skins"]
        .as_array_mut()
        .unwrap()
        .push(reversed_skin);
    document["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "SecondMesh", "mesh": 0, "skin": 1}));
    document["nodes"][0]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!(10));
    let report = import_triangle(&document, binary)?;
    let scene = report.scene();
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    assert_eq!(scene.mesh_instance_count(), 2);
    assert_eq!(scene.skin_count(), 2);
    let instances = scene.mesh_instances().collect::<Vec<_>>();
    assert_eq!(instances[0].mesh().id(), instances[1].mesh().id());
    let first = instances[0].skin().unwrap();
    let second = instances[1].skin().unwrap();
    assert_ne!(first.id(), second.id());
    assert_eq!(first.joint_count(), 8);
    assert_eq!(
        second.joints().collect::<Vec<_>>(),
        first.joints().rev().collect::<Vec<_>>()
    );
    assert_rows(
        &report,
        &[
            (&JOINT_ROWS[0], &WEIGHT_ROWS[0]),
            (&JOINT_ROWS[1], &WEIGHT_ROWS[1]),
            (&JOINT_ROWS[2], &WEIGHT_ROWS[2]),
        ],
    );
    Ok(())
}

#[test]
fn missing_pairs_and_gaps_fail_before_dependency_io() -> TestResult<()> {
    for defect in [
        "missing joints 0",
        "missing weights 0",
        "missing joints 1",
        "missing weights 1",
        "missing set 0",
        "gap at set 1",
        "unpaired set 2",
    ] {
        let (mut document, _) = two_set_triangle()?;
        let attributes = document["meshes"][0]["primitives"][0]["attributes"]
            .as_object_mut()
            .unwrap();
        match defect {
            "missing joints 0" => {
                attributes.remove("JOINTS_0");
            }
            "missing weights 0" => {
                attributes.remove("WEIGHTS_0");
            }
            "missing joints 1" => {
                attributes.remove("JOINTS_1");
            }
            "missing weights 1" => {
                attributes.remove("WEIGHTS_1");
            }
            "missing set 0" => {
                attributes.remove("JOINTS_0");
                attributes.remove("WEIGHTS_0");
            }
            "gap at set 1" => {
                let joints = attributes.remove("JOINTS_1").unwrap();
                let weights = attributes.remove("WEIGHTS_1").unwrap();
                attributes.insert("JOINTS_2".into(), joints);
                attributes.insert("WEIGHTS_2".into(), weights);
            }
            "unpaired set 2" => {
                attributes.insert("WEIGHTS_2".into(), json!(5));
            }
            _ => unreachable!("all pair defects are listed above"),
        }
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty(), "{defect}: {diagnostics:?}");
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        assert_no_dependency_io(&failure);
        assert!(failure.source_scope_record().is_none());
    }
    Ok(())
}

#[test]
fn later_set_count_type_and_normalization_failures_precede_dependency_io() -> TestResult<()> {
    for (accessor, field, value, code) in [
        (4, "count", json!(2), GltfFailureCode::InvalidDocument),
        (5, "count", json!(2), GltfFailureCode::InvalidDocument),
        (4, "type", json!("VEC3"), GltfFailureCode::InvalidDocument),
        (5, "type", json!("VEC3"), GltfFailureCode::InvalidDocument),
        (
            4,
            "componentType",
            json!(5125),
            GltfFailureCode::InvalidDocument,
        ),
        (
            5,
            "componentType",
            json!(5122),
            GltfFailureCode::InvalidDocument,
        ),
        (
            5,
            "componentType",
            json!(5121),
            GltfFailureCode::InvalidDocument,
        ),
        (
            5,
            "normalized",
            json!(true),
            GltfFailureCode::InvalidDocument,
        ),
        (
            4,
            "normalized",
            json!(true),
            GltfFailureCode::InvalidAccessor,
        ),
    ] {
        let (mut document, _) = two_set_triangle()?;
        document["accessors"][accessor][field] = value;
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, code, None)?;
        assert_no_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn later_set_layout_failures_precede_dependency_io() -> TestResult<()> {
    for (table, index, field, value, code) in [
        (
            "accessors",
            4,
            "byteOffset",
            2,
            GltfFailureCode::InvalidAccessor,
        ),
        (
            "bufferViews",
            4,
            "byteStride",
            4,
            GltfFailureCode::InvalidDocument,
        ),
        (
            "bufferViews",
            5,
            "byteStride",
            20,
            GltfFailureCode::InvalidDocument,
        ),
        (
            "bufferViews",
            5,
            "byteLength",
            47,
            GltfFailureCode::InvalidDocument,
        ),
    ] {
        let (mut document, _) = two_set_triangle()?;
        document[table][index][field] = json!(value);
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, code, None)?;
        assert_no_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn later_set_joint_bounds_include_zero_weight_lanes_for_every_instance() -> TestResult<()> {
    for invalid_instance in [0, 1] {
        for zero_weight in [false, true] {
            let (mut document, mut binary) = two_set_triangle()?;
            let mut smaller_skin = document["skins"][0].clone();
            smaller_skin["name"] = json!("SmallerPalette");
            smaller_skin["joints"].as_array_mut().unwrap().pop();
            document["skins"].as_array_mut().unwrap().push(smaller_skin);
            document["nodes"][2]["skin"] = json!(if invalid_instance == 0 { 1 } else { 0 });
            document["nodes"].as_array_mut().unwrap().push(json!({
                "name": "SecondMesh", "mesh": 0, "skin": if invalid_instance == 1 { 1 } else { 0 }
            }));
            document["nodes"][0]["children"]
                .as_array_mut()
                .unwrap()
                .push(json!(10));
            if zero_weight {
                binary[140..144].copy_from_slice(&0.3125_f32.to_le_bytes());
                binary[152..156].copy_from_slice(&0.0_f32.to_le_bytes());
            }
            let (result, diagnostics) = import_assets_result(
                SOURCE,
                [(SOURCE, serde_json::to_vec(&document)?), (BASE, binary)],
                sidecar_options(),
            )?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_eq!(
                failure.scene_validation_error(),
                Some(&SceneValidationError::PaletteJointOutOfRange {
                    instance: invalid_instance,
                    logical_mesh: 0,
                    skin: 1,
                    joint_count: 7,
                    primitive: 0,
                    influence: 7,
                    palette_joint: 7,
                }),
                "instance {invalid_instance}, zero weight {zero_weight}"
            );
        }
    }
    Ok(())
}

#[test]
fn later_set_negative_and_nonfinite_weights_are_invalid_documents() -> TestResult<()> {
    for weight in [-0.125_f32, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let (document, mut binary) = two_set_triangle()?;
        // Vertex 2, set 1, lane 2 is influence 22 in the retained eight-wide rows.
        binary[180..184].copy_from_slice(&weight.to_le_bytes());
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?), (BASE, binary)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn duplicate_positive_weight_joints_are_invalid_within_and_across_sets() -> TestResult<()> {
    for (set_count, joint_offset) in [(1, 44), (2, 44), (2, 116), (2, 122)] {
        let (mut document, mut binary) = two_set_triangle()?;
        if set_count == 1 {
            let attributes = document["meshes"][0]["primitives"][0]["attributes"]
                .as_object_mut()
                .unwrap();
            attributes.remove("JOINTS_1");
            attributes.remove("WEIGHTS_1");
            for (vertex, weights) in [
                [0.25_f32, 0.5, 0.125, 0.125],
                [0.25, 0.75, -0.0, 0.0],
                [0.5, 0.0, 0.5, -0.0],
            ]
            .into_iter()
            .enumerate()
            {
                for (lane, weight) in weights.into_iter().enumerate() {
                    let offset = 68 + vertex * 16 + lane * 4;
                    binary[offset..offset + 4].copy_from_slice(&weight.to_le_bytes());
                }
            }
        }
        // Joint zero already has positive weight in vertex zero's first lane.
        binary[joint_offset..joint_offset + 2].copy_from_slice(&0_u16.to_le_bytes());
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?), (BASE, binary)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn duplicate_zero_weight_joints_keep_both_zero_signs() -> TestResult<()> {
    for zero in [0.0_f32, -0.0] {
        let (document, mut binary) = two_set_triangle()?;
        binary[122..124].copy_from_slice(&0_u16.to_le_bytes());
        binary[68..72].copy_from_slice(&0.25_f32.to_le_bytes());
        binary[152..156].copy_from_slice(&zero.to_le_bytes());
        let report = import_triangle(&document, binary)?;
        assert_rows(
            &report,
            &[
                (
                    &[0, 1, 2, 3, 4, 5, 6, 0],
                    &[0.25, 0.25, 0.0625, 0.0625, 0.1875, 0.125, 0.0625, zero],
                ),
                (&JOINT_ROWS[1], &WEIGHT_ROWS[1]),
                (&JOINT_ROWS[2], &WEIGHT_ROWS[2]),
            ],
        );
    }
    Ok(())
}

#[test]
fn normalized_integer_witnesses_keep_exact_bits_in_dense_and_sparse_sets() -> TestResult<()> {
    // Literal results use integer quotient/remainder rounding, independent of the decoder.
    for (component, maximum, witness, expected_bits) in [
        (
            5121,
            255_u16,
            [3_u16, 252, 0, 0],
            [0x3c40_c0c1, 0x3f7c_fcfd, 0x0000_0000, 0x0000_0000],
        ),
        (
            5123,
            65535,
            [257, 65278, 0, 0],
            [0x3b80_8081, 0x3f7e_feff, 0x0000_0000, 0x0000_0000],
        ),
    ] {
        for (sparse, with_base) in [(false, true), (true, true), (true, false)] {
            let (mut document, mut binary) = two_set_triangle()?;
            for vertex in 0..3 {
                for (offset, joints) in [
                    (42 + vertex * 8, [3_u16, 0, 2, 1]),
                    (116 + vertex * 8, [7_u16, 4, 6, 5]),
                ] {
                    for (lane, joint) in joints.into_iter().enumerate() {
                        binary[offset + lane * 2..offset + lane * 2 + 2]
                            .copy_from_slice(&joint.to_le_bytes());
                    }
                }
            }
            for (accessor, offset, rows) in [
                (3, 68, [[maximum, 0, 0, 0], [0; 4], [0; 4]]),
                (5, 140, [[0; 4], witness, witness]),
            ] {
                let mut encoded = Vec::new();
                for row in rows {
                    for value in row {
                        if component == 5121 {
                            encoded.push(u8::try_from(value)?);
                        } else {
                            encoded.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
                binary[offset..offset + encoded.len()].copy_from_slice(&encoded);
                document["bufferViews"][accessor]["byteLength"] = json!(encoded.len());
                document["accessors"][accessor]["componentType"] = json!(component);
                document["accessors"][accessor]["normalized"] = json!(true);
            }
            if sparse {
                let row_bytes = if component == 5121 { 4 } else { 8 };
                let values = binary[140 + row_bytes..140 + 3 * row_bytes].to_vec();
                // A valid unit-weight base must be replaced, never added or renormalized.
                binary[140 + row_bytes..140 + 3 * row_bytes].fill(0);
                for vertex in 1..3 {
                    let offset = 140 + vertex * row_bytes;
                    if component == 5121 {
                        binary[offset] = u8::try_from(maximum)?;
                    } else {
                        binary[offset..offset + 2].copy_from_slice(&maximum.to_le_bytes());
                    }
                }
                let index_offset = binary.len();
                binary.extend_from_slice(&[1, 2]);
                binary.resize(binary.len().next_multiple_of(4), 0);
                let value_offset = binary.len();
                binary.extend_from_slice(&values);
                document["bufferViews"].as_array_mut().unwrap().extend([
                    json!({"buffer": 0, "byteOffset": index_offset, "byteLength": 2}),
                    json!({"buffer": 0, "byteOffset": value_offset, "byteLength": values.len()}),
                ]);
                document["accessors"][5]["sparse"] = json!({
                    "count": 2,
                    "indices": {"bufferView": 6, "componentType": 5121},
                    "values": {"bufferView": 7}
                });
                if !with_base {
                    document["accessors"][5]
                        .as_object_mut()
                        .unwrap()
                        .remove("bufferView");
                }
            }
            document["buffers"][0]["byteLength"] = json!(binary.len());
            let report = import_triangle(&document, binary)?;
            let [a, b, c, d] = expected_bits.map(f32::from_bits);
            let expected_weights = [0.0, 0.0, 0.0, 0.0, a, b, c, d];
            let joints = [3, 0, 2, 1, 7, 4, 6, 5];
            assert_rows(
                &report,
                &[
                    (&joints, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
                    (&joints, &expected_weights),
                    (&joints, &expected_weights),
                ],
            );
        }
    }
    Ok(())
}

#[test]
fn integer_weight_sets_require_an_exact_sum_for_equal_and_mixed_widths() -> TestResult<()> {
    const POWERS_OF_TWO_BITS: [[u32; 4]; 2] = [
        [0x3b80_8081, 0x3c00_8081, 0x3c80_8081, 0x3d00_8081],
        [0x3d80_8081, 0x3e00_8081, 0x3e80_8081, 0x3f00_8081],
    ];
    for (components, numerators, expected_bits) in [
        (
            [5121, 5121],
            [[1_u16, 2, 4, 8], [16_u16, 32, 64, 128]],
            POWERS_OF_TWO_BITS,
        ),
        (
            [5123, 5123],
            [[257, 514, 1028, 2056], [4112, 8224, 16448, 32896]],
            POWERS_OF_TWO_BITS,
        ),
        (
            [5121, 5123],
            [[1, 4, 16, 64], [640, 2080, 8200, 32770]],
            [SPEC_U8_WEIGHT_BITS, SPEC_U16_WEIGHT_BITS],
        ),
        (
            [5123, 5121],
            [[640, 2080, 8200, 32770], [1, 4, 16, 64]],
            [SPEC_U16_WEIGHT_BITS, SPEC_U8_WEIGHT_BITS],
        ),
    ] {
        let (mut document, mut binary) = two_set_triangle()?;
        for vertex in 0..3 {
            for (offset, joints) in [
                (42 + vertex * 8, [0_u16, 1, 2, 3]),
                (116 + vertex * 8, [4_u16, 5, 6, 7]),
            ] {
                for (lane, joint) in joints.into_iter().enumerate() {
                    binary[offset + lane * 2..offset + lane * 2 + 2]
                        .copy_from_slice(&joint.to_le_bytes());
                }
            }
        }
        for (set, offset, accessor) in [(0, 68, 3), (1, 140, 5)] {
            let mut encoded = Vec::new();
            for _ in 0..3 {
                for value in numerators[set] {
                    if components[set] == 5121 {
                        encoded.push(u8::try_from(value)?);
                    } else {
                        encoded.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            binary[offset..offset + encoded.len()].copy_from_slice(&encoded);
            document["bufferViews"][accessor]["byteLength"] = json!(encoded.len());
            document["accessors"][accessor]["componentType"] = json!(components[set]);
            document["accessors"][accessor]["normalized"] = json!(true);
        }
        let report = import_triangle(&document, binary.clone())?;
        let expected_weights = expected_bits
            .into_iter()
            .flatten()
            .map(f32::from_bits)
            .collect::<Vec<_>>();
        assert_rows(
            &report,
            &[
                (&[0, 1, 2, 3, 4, 5, 6, 7], &expected_weights),
                (&[0, 1, 2, 3, 4, 5, 6, 7], &expected_weights),
                (&[0, 1, 2, 3, 4, 5, 6, 7], &expected_weights),
            ],
        );
        // Change only the final later-set component by one source integer unit.
        for delta in [-1_i32, 1] {
            let mut invalid = binary.clone();
            let value = u16::try_from(i32::from(numerators[1][3]) + delta)?;
            if components[1] == 5121 {
                invalid[151] = u8::try_from(value)?;
            } else {
                invalid[162..164].copy_from_slice(&value.to_le_bytes());
            }
            let (result, diagnostics) = import_assets_result(
                SOURCE,
                [(SOURCE, serde_json::to_vec(&document)?), (BASE, invalid)],
                sidecar_options(),
            )?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        }
    }
    Ok(())
}

#[test]
fn float_weight_sums_warn_once_per_primitive_and_preserve_source_bits() -> TestResult<()> {
    for integer_later_set in [false, true] {
        let (mut document, mut binary) = two_set_triangle()?;
        for vertex in 0..3 {
            let offset = 68 + vertex * 16;
            binary[offset..offset + 4].copy_from_slice(&0.375_f32.to_le_bytes());
        }
        if integer_later_set {
            document["accessors"][5]["componentType"] = json!(5121);
            document["accessors"][5]["normalized"] = json!(true);
            document["bufferViews"][5]["byteLength"] = json!(12);
            binary[140..152].copy_from_slice(&[127, 0, 0, 0, 127, 0, 0, 0, 127, 0, 0, 0]);
        }
        // Multiple offending rows and two instances still produce one warning per definition.
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "SecondMesh", "mesh": 0, "skin": 0}));
        document["nodes"][0]["children"]
            .as_array_mut()
            .unwrap()
            .push(json!(10));
        let second = document["meshes"][0]["primitives"][0].clone();
        document["meshes"][0]["primitives"]
            .as_array_mut()
            .unwrap()
            .push(second);
        let (report, diagnostics) = import_assets(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?), (BASE, binary)],
            sidecar_options(),
        )?;
        assert_eq!(diagnostics.len(), 2);
        for diagnostic in &diagnostics {
            assert_eq!(diagnostic.code().as_str(), "gltf.influence_weight_sum");
            assert_eq!(diagnostic.severity(), DiagnosticSeverity::Warning);
        }
        for primitive in 0..2 {
            let owner = format!("mesh 0 primitive {primitive}");
            assert_eq!(
                diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.message().contains(&owner))
                    .count(),
                1,
                "one warning for {owner}"
            );
        }
        assert!(report.loss_evidence().is_empty());
        let mesh = report.scene().meshes().next().unwrap();
        for primitive in mesh.primitives() {
            let rows = primitive.influences().unwrap();
            for (vertex, row) in rows.iter().enumerate() {
                assert_eq!(row.joint_indices(), &JOINT_ROWS[vertex]);
                assert_eq!(row.source_orders(), &[0, 1, 2, 3, 4, 5, 6, 7]);
                for (lane, &actual) in row.weights().iter().enumerate() {
                    if integer_later_set && lane == 4 {
                        assert_eq!(actual.to_bits(), 0x3efe_feff);
                    } else {
                        let expected = if lane == 0 {
                            0.375
                        } else if integer_later_set && lane > 4 {
                            0.0
                        } else {
                            WEIGHT_ROWS[vertex][lane]
                        };
                        assert_eq!(actual.to_bits(), expected.to_bits());
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn float_sum_within_the_spec_tolerance_keeps_bits_without_a_warning() -> TestResult<()> {
    let (document, mut binary) = two_set_triangle()?;
    let weight = f32::from_bits(0.125_f32.to_bits() + 1);
    binary[68..72].copy_from_slice(&weight.to_le_bytes());
    let report = import_triangle(&document, binary)?;
    let mut expected = WEIGHT_ROWS[0];
    expected[0] = weight;
    assert_rows(
        &report,
        &[
            (&JOINT_ROWS[0], &expected),
            (&JOINT_ROWS[1], &WEIGHT_ROWS[1]),
            (&JOINT_ROWS[2], &WEIGHT_ROWS[2]),
        ],
    );
    assert!(report.loss_evidence().is_empty());
    Ok(())
}

#[test]
fn influence_csr_endpoint_overflow_is_rejected_before_dependency_io() -> TestResult<()> {
    let (mut document, _) = two_set_triangle()?;
    let vertex_count = u64::from(u32::MAX) / 8 + 1;
    document["buffers"][0]["byteLength"] = json!(189);
    document["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer": 0, "byteOffset": 188, "byteLength": 1
    }));
    // Sparse zero bases keep every declared byte range small while the logical CSR
    // endpoint exceeds u32. No large allocation or external fixture is needed.
    for (accessor, value_view) in [(0, 0), (2, 2), (3, 3), (4, 4), (5, 5)] {
        document["accessors"][accessor]
            .as_object_mut()
            .unwrap()
            .remove("bufferView");
        document["accessors"][accessor]["count"] = json!(vertex_count);
        document["accessors"][accessor]["sparse"] = json!({
            "count": 1,
            "indices": {"bufferView": 6, "componentType": 5121},
            "values": {"bufferView": value_view}
        });
        document["bufferViews"][value_view]
            .as_object_mut()
            .unwrap()
            .remove("target");
    }
    let mut options = sidecar_options();
    options.limits.max_vertices = usize::try_from(vertex_count)?;
    options.limits.max_total_vertex_attribute_cells = usize::try_from(vertex_count * 5)?;
    options.limits.max_influence_cells = usize::try_from(vertex_count * 8)?;
    let (result, diagnostics) =
        import_assets_result(SOURCE, [(SOURCE, serde_json::to_vec(&document)?)], options)?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::Allocation { resource: "gltf-influence-endpoint" })));
    assert_no_dependency_io(&failure);
    Ok(())
}

#[test]
fn aggregate_influence_cell_budget_counts_definitions_and_all_zero_lanes_once() -> TestResult<()> {
    let (mut document, mut binary) = two_set_triangle()?;
    binary.resize(260, 0);
    document["buffers"][0]["byteLength"] = json!(binary.len());
    document["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 0, "byteOffset": 188, "byteLength": 24, "target": 34962}),
        json!({"buffer": 0, "byteOffset": 212, "byteLength": 48, "target": 34962}),
    ]);
    document["accessors"].as_array_mut().unwrap().extend([
        json!({"bufferView": 6, "componentType": 5123, "count": 3, "type": "VEC4"}),
        json!({"bufferView": 7, "componentType": 5126, "count": 3, "type": "VEC4"}),
    ]);
    let mut second = document["meshes"][0]["primitives"][0].clone();
    second["attributes"]["JOINTS_2"] = json!(6);
    second["attributes"]["WEIGHTS_2"] = json!(7);
    document["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(second);
    document["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "SecondMesh", "mesh": 0, "skin": 0}));
    document["nodes"][0]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!(10));

    // Two definitions with 3 vertices and 2/3 sets: 24 + 36 = 60, across two instances.
    let mut options = sidecar_options();
    options.limits.max_influence_cells = 59;
    let (result, diagnostics) =
        import_assets_result(SOURCE, [(SOURCE, serde_json::to_vec(&document)?)], options)?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert_resource_limit(&failure, "max_influence_cells");
    assert_no_dependency_io(&failure);

    let mut options = sidecar_options();
    options.limits.max_influence_cells = 60;
    let (report, diagnostics) = import_assets(
        SOURCE,
        [(SOURCE, serde_json::to_vec(&document)?), (BASE, binary)],
        options,
    )?;
    assert!(diagnostics.is_empty());
    assert_eq!(report.scene().mesh_instance_count(), 2);
    assert_eq!(report.scene().primitive_count(), 2);
    let cells = report
        .ledger()
        .dimension(ImportDimension::InfluenceCells)
        .unwrap();
    assert_eq!(cells.attempted(), 60);
    assert_eq!(cells.consumed(), 60);
    let mesh = report.scene().meshes().next().unwrap();
    let widths = mesh
        .primitives()
        .map(|primitive| primitive.influences().unwrap().get(0).unwrap().len())
        .collect::<Vec<_>>();
    assert_eq!(widths, [8, 12]);
    Ok(())
}

#[test]
fn selected_scene_omits_invalid_higher_sets_and_their_missing_external_buffer() -> TestResult<()> {
    let (mut document, binary) = two_set_triangle()?;
    document["buffers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"uri": "omitted.bin", "byteLength": 72}));
    document["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 1, "byteOffset": 0, "byteLength": 24, "target": 34962}),
        json!({"buffer": 1, "byteOffset": 24, "byteLength": 48, "target": 34962}),
    ]);
    document["accessors"].as_array_mut().unwrap().extend([
        json!({"bufferView": 6, "componentType": 5123, "count": 3, "type": "VEC4"}),
        json!({"bufferView": 7, "componentType": 5126, "count": 3, "type": "VEC4"}),
    ]);
    document["meshes"].as_array_mut().unwrap().push(json!({
        "name": "OmittedMesh", "primitives": [{"attributes": {
            "POSITION": 0, "JOINTS_0": 2, "WEIGHTS_0": 3, "JOINTS_2": 6, "WEIGHTS_2": 7
        }, "indices": 1, "mode": 4}]
    }));
    document["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "OmittedNode", "mesh": 1, "skin": 0}));
    document["scenes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"nodes": [10]}));
    let report = import_triangle(&document, binary)?;
    assert_eq!(report.scene().mesh_count(), 1);
    assert_eq!(report.scene().mesh_instance_count(), 1);
    assert_rows(
        &report,
        &[
            (&JOINT_ROWS[0], &WEIGHT_ROWS[0]),
            (&JOINT_ROWS[1], &WEIGHT_ROWS[1]),
            (&JOINT_ROWS[2], &WEIGHT_ROWS[2]),
        ],
    );
    let record = report.source_scope_record().unwrap();
    for (kind, selected, omitted) in [
        (SourceScopeEntityKind::Mesh, 1, 1),
        (SourceScopeEntityKind::Primitive, 1, 1),
        (SourceScopeEntityKind::Buffer, 1, 1),
        (SourceScopeEntityKind::BufferView, 6, 2),
        (SourceScopeEntityKind::Accessor, 6, 2),
    ] {
        assert_eq!(record.counts(kind).selected(), selected);
        assert_eq!(record.counts(kind).omitted(), omitted);
    }
    assert!(
        record
            .details()
            .any(|detail| detail.kind() == SourceScopeEntityKind::Buffer
                && detail.disposition() == SourceScopeDisposition::Omitted
                && detail.identity() == "b1")
    );
    let cells = report
        .ledger()
        .dimension(ImportDimension::InfluenceCells)
        .unwrap();
    assert_eq!(cells.attempted(), 24);
    assert_eq!(cells.consumed(), 24);
    assert!(
        report
            .successful_input_manifest()
            .node_classes()
            .iter()
            .all(|class| class.identity().value().as_str() != "models/omitted.bin")
    );
    Ok(())
}
