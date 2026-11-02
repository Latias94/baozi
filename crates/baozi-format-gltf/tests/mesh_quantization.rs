mod common;

use std::sync::atomic::Ordering;

use baozi_core::{
    AttributeData, AttributeElement, AttributeSemanticRef, ProvenanceKindRef, TopologyKind,
};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::authoring::{AttributeDomain, MorphDeltaSemantic, Vec3};
use baozi_import::{
    FormatFailureDetailValue, ImportCause, ImportDimension, ScopeAbortCause,
    SourceObjectLocatorRef, SourceScopeDisposition, SourceScopeEntityKind,
};
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    TestResult, assert_gltf_failure, assert_resource_limit, controlled_import, expected_error,
    glb_with_json, gltf_failure, gltf_failure_reason, import_assets, import_assets_result,
    import_assets_result_with_control, sidecar_options,
};
use serde_json::{Value, json};

const SOURCE: &str = "models/quantized.gltf";
const BINARY: &str = "models/quantized.bin";
const QUANTIZATION: &str = "KHR_mesh_quantization";

#[test]
fn float_bounds_order_and_extrema_use_the_accessor_component_rounding() -> TestResult<()> {
    for morph in [false, true] {
        let mut binary: Vec<u8> = [if morph { 0.0_f32 } else { 1.0 }, 0.0, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let mut document = json!({
            "asset": {"version": "2.0"}, "scene": 0,
            "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"mode": 0, "attributes": {"POSITION": 0}}]}],
            "buffers": [{"uri": "quantized.bin", "byteLength": 12}],
            "bufferViews": [{"buffer": 0, "byteLength": 12}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 1,
                "type": "VEC3", "min": [0,0,0], "max": [0,0,0]}]
        });
        let selected = usize::from(morph);
        if morph {
            binary.extend([1.0_f32, 0.0, 0.0].into_iter().flat_map(f32::to_le_bytes));
            document["bufferViews"].as_array_mut().unwrap().push(json!({
                "buffer": 0, "byteOffset": 12, "byteLength": 12
            }));
            document["accessors"].as_array_mut().unwrap().push(json!({
                "bufferView": 1, "componentType": 5126, "count": 1, "type": "VEC3"
            }));
            document["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION": 1}]);
            document["buffers"][0]["byteLength"] = json!(24);
        }
        // Independently known nearest-F32 bits: both X bounds and stored 1.0
        // are 0x3f800000, although their source decimal order is reversed.
        document["accessors"][selected]["min"] = json!([1.00000001, 0, 0]);
        document["accessors"][selected]["max"] = json!([0.99999999, 0, 0]);
        let (report, diagnostics) = import_assets(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?), (BINARY, binary)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        if morph {
            assert_eq!(
                primitive
                    .morph_targets()
                    .next()
                    .unwrap()
                    .delta(MorphDeltaSemantic::Position)
                    .unwrap()
                    .data()
                    .value(0),
                Some(Vec3::new(1.0, 0.0, 0.0))
            );
        } else {
            assert_eq!(
                primitive.positions().unwrap().element(0),
                Some(AttributeElement::F32(&[1.0, 0.0, 0.0]))
            );
        }
        // A genuinely reversed F32 pair must still fail before missing IO.
        document["accessors"][selected]["min"] = json!([1.0000002, 0, 0]);
        let (result, _) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        let failure = expected_error(result)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::InvalidAccessor,
            Some("ordered min <= max"),
        )?;
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
    }
    Ok(())
}

#[test]
fn optional_influence_bounds_compare_stored_lanes_without_normalizing_their_domain()
-> TestResult<()> {
    for integer_weights in [false, true] {
        let mut document: Value = serde_json::from_slice(&common::skinned_triangle_gltf())?;
        let mut binary = common::skinned_triangle_bin();
        document["accessors"][2]["min"] = json!([0, 0, 0, 0]);
        document["accessors"][2]["max"] = json!([1, 1, 0, 0]);
        if integer_weights {
            binary[68..80].copy_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 128, 127, 0, 0]);
            document["accessors"][3]["componentType"] = json!(5121);
            document["accessors"][3]["normalized"] = json!(true);
            document["bufferViews"][3]["byteStride"] = json!(4);
            document["bufferViews"][3]["byteLength"] = json!(12);
            document["accessors"][3]["min"] = json!([128, 0, 0, 0]);
            document["accessors"][3]["max"] = json!([255, 127, 0, 0]);
        } else {
            document["accessors"][3]["min"] = json!([0.5, 0, 0, 0]);
            document["accessors"][3]["max"] = json!([1, 0.5, 0, 0]);
        }
        let (report, diagnostics) = import_assets(
            SOURCE,
            [
                (SOURCE, serde_json::to_vec(&document)?),
                ("models/skin.bin", binary.clone()),
            ],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        let row = primitive.influences().unwrap().get(2).unwrap();
        assert_eq!(row.joint_indices(), &[0, 1, 0, 0]);
        let expected = if integer_weights {
            [0x3f00_8081, 0x3efe_feff, 0, 0]
        } else {
            [0x3f00_0000, 0x3f00_0000, 0, 0]
        };
        assert_eq!(
            row.weights()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            expected
        );
        for accessor in [2, 3] {
            let mut invalid = document.clone();
            invalid["accessors"][accessor]["max"][0] = if accessor == 2 {
                json!(0)
            } else if integer_weights {
                json!(254)
            } else {
                json!(0.75)
            };
            let (result, _) = import_assets_result(
                SOURCE,
                [
                    (SOURCE, serde_json::to_vec(&invalid)?),
                    ("models/skin.bin", binary.clone()),
                ],
                sidecar_options(),
            )?;
            let failure = expected_error(result)?;
            assert_gltf_failure(
                &failure,
                GltfFailureCode::InvalidAccessor,
                Some("stored values"),
            )?;
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }
    Ok(())
}

// Each row is an independent Khronos tuple: semantic, component, normalized,
// packed bytes, source-domain sample/bounds, decoded F32 sample, dense stride.
type EncodingCase = (
    &'static str,
    u32,
    bool,
    &'static [u8],
    &'static [i32],
    &'static [f32],
    usize,
);

const BASE_CASES: [EncodingCase; 18] = [
    (
        "POSITION",
        5120,
        false,
        &[128, 64, 0],
        &[-128, 64, 0],
        &[-128.0, 64.0, 0.0],
        4,
    ),
    (
        "POSITION",
        5120,
        true,
        &[128, 64, 0],
        &[-128, 64, 0],
        &[-1.0, 0.503937, 0.0],
        4,
    ),
    (
        "POSITION",
        5121,
        false,
        &[0, 128, 255],
        &[0, 128, 255],
        &[0.0, 128.0, 255.0],
        4,
    ),
    (
        "POSITION",
        5121,
        true,
        &[0, 128, 255],
        &[0, 128, 255],
        &[0.0, 0.5019608, 1.0],
        4,
    ),
    (
        "POSITION",
        5122,
        false,
        &[0, 128, 0, 64, 255, 255],
        &[-32768, 16384, -1],
        &[-32768.0, 16384.0, -1.0],
        8,
    ),
    (
        "POSITION",
        5122,
        true,
        &[0, 128, 0, 64, 255, 255],
        &[-32768, 16384, -1],
        &[-1.0, 0.50001526, -0.00003051851],
        8,
    ),
    (
        "POSITION",
        5123,
        false,
        &[0, 0, 0, 128, 255, 255],
        &[0, 32768, 65535],
        &[0.0, 32768.0, 65535.0],
        8,
    ),
    (
        "POSITION",
        5123,
        true,
        &[0, 0, 0, 128, 255, 255],
        &[0, 32768, 65535],
        &[0.0, 0.5000076, 1.0],
        8,
    ),
    (
        "NORMAL",
        5120,
        true,
        &[64, 64, 64],
        &[64, 64, 64],
        &[0.503937, 0.503937, 0.503937],
        4,
    ),
    (
        "NORMAL",
        5122,
        true,
        &[0, 64, 0, 64, 0, 64],
        &[16384, 16384, 16384],
        &[0.50001526, 0.50001526, 0.50001526],
        8,
    ),
    (
        "TANGENT",
        5120,
        true,
        &[64, 192, 0, 129],
        &[64, -64, 0, -127],
        &[0.503937, -0.503937, 0.0, -1.0],
        4,
    ),
    (
        "TANGENT",
        5122,
        true,
        &[0, 64, 0, 192, 0, 0, 1, 128],
        &[16384, -16384, 0, -32767],
        &[0.50001526, -0.50001526, 0.0, -1.0],
        8,
    ),
    (
        "TEXCOORD_0",
        5120,
        false,
        &[128, 64],
        &[-128, 64],
        &[-128.0, 64.0],
        4,
    ),
    (
        "TEXCOORD_0",
        5120,
        true,
        &[128, 64],
        &[-128, 64],
        &[-1.0, 0.503937],
        4,
    ),
    (
        "TEXCOORD_0",
        5122,
        false,
        &[0, 128, 255, 255],
        &[-32768, -1],
        &[-32768.0, -1.0],
        4,
    ),
    (
        "TEXCOORD_0",
        5122,
        true,
        &[0, 128, 255, 255],
        &[-32768, -1],
        &[-1.0, -0.00003051851],
        4,
    ),
    (
        "TEXCOORD_0",
        5121,
        false,
        &[128, 255],
        &[128, 255],
        &[128.0, 255.0],
        4,
    ),
    (
        "TEXCOORD_0",
        5123,
        false,
        &[0, 128, 255, 255],
        &[32768, 65535],
        &[32768.0, 65535.0],
        4,
    ),
];

const MORPH_CASES: [EncodingCase; 8] = [
    (
        "POSITION",
        5120,
        false,
        &[128, 64, 0],
        &[-128, 64, 0],
        &[-128.0, 64.0, 0.0],
        4,
    ),
    (
        "POSITION",
        5120,
        true,
        &[128, 64, 0],
        &[-128, 64, 0],
        &[-1.0, 0.503937, 0.0],
        4,
    ),
    (
        "POSITION",
        5122,
        false,
        &[0, 128, 0, 64, 255, 255],
        &[-32768, 16384, -1],
        &[-32768.0, 16384.0, -1.0],
        8,
    ),
    (
        "POSITION",
        5122,
        true,
        &[0, 128, 0, 64, 255, 255],
        &[-32768, 16384, -1],
        &[-1.0, 0.50001526, -0.00003051851],
        8,
    ),
    (
        "NORMAL",
        5120,
        true,
        &[64, 64, 64],
        &[64, 64, 64],
        &[0.503937, 0.503937, 0.503937],
        4,
    ),
    (
        "NORMAL",
        5122,
        true,
        &[0, 64, 0, 64, 0, 64],
        &[16384, 16384, 16384],
        &[0.50001526, 0.50001526, 0.50001526],
        8,
    ),
    (
        "TANGENT",
        5120,
        true,
        &[64, 192, 0],
        &[64, -64, 0],
        &[0.503937, -0.503937, 0.0],
        4,
    ),
    (
        "TANGENT",
        5122,
        true,
        &[0, 64, 0, 192, 0, 0],
        &[16384, -16384, 0],
        &[0.50001526, -0.50001526, 0.0],
        8,
    ),
];

fn append_f32(bytes: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn fixture(case: EncodingCase, morph: bool) -> (Value, Vec<u8>) {
    let (semantic, component, normalized, sample, stored, _, stride) = case;
    let mut binary = Vec::new();
    append_f32(&mut binary, &[1.0, 2.0, 3.0, 3.0, 2.0, 3.0, 1.0, 4.0, 3.0]);
    append_f32(&mut binary, &[0.0, 0.0, 1.0].repeat(3));
    append_f32(&mut binary, &[1.0, 0.0, 0.0, 1.0].repeat(3));
    append_f32(&mut binary, &[0.25, 0.75].repeat(3));
    assert_eq!(binary.len(), 144);
    for _ in 0..3 {
        binary.extend_from_slice(sample);
        binary.resize(binary.len() + stride - sample.len(), 0xa5);
    }
    let mut document = json!({
        "asset": {"version": "2.0"},
        "extensionsUsed": [QUANTIZATION], "extensionsRequired": [QUANTIZATION],
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"mode": 0,
            "attributes": {"POSITION": 0, "NORMAL": 1, "TANGENT": 2, "TEXCOORD_0": 3}}]}],
        "buffers": [{"uri": "quantized.bin", "byteLength": binary.len()}],
        "bufferViews": [
            {"buffer": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 36},
            {"buffer": 0, "byteOffset": 72, "byteLength": 48},
            {"buffer": 0, "byteOffset": 120, "byteLength": 24},
            {"buffer": 0, "byteOffset": 144, "byteLength": stride * 3, "byteStride": stride}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [1, 2, 3], "max": [3, 4, 3]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"},
            {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2"},
            {"bufferView": 4, "componentType": component, "normalized": normalized, "count": 3,
             "type": format!("VEC{}", stored.len()), "min": stored, "max": stored}
        ]
    });
    if morph {
        document["meshes"][0]["weights"] = json!([0.25]);
        document["meshes"][0]["primitives"][0]["targets"] = json!([{semantic: 4}]);
    } else {
        document["meshes"][0]["primitives"][0]["attributes"][semantic] = json!(4);
    }
    (document, binary)
}

#[test]
fn every_added_tuple_retains_f32_values_in_dense_zero_base_and_sparse_overlay() -> TestResult<()> {
    for (morph, cases) in [
        (false, BASE_CASES.as_slice()),
        (true, MORPH_CASES.as_slice()),
    ] {
        for &case in cases {
            let (semantic, _, _, sample, _, expected, _) = case;
            for storage in ["dense", "zero-base", "overlay"] {
                let (mut document, mut binary) = fixture(case, morph);
                if storage == "overlay" {
                    for vertex in 1..3 {
                        let offset = 144 + vertex * case.6;
                        binary[offset..offset + sample.len()].fill(0);
                    }
                }
                if storage != "dense" {
                    let start = binary.len();
                    let selectors: &[u8] = if storage == "zero-base" {
                        &[0, 1, 2]
                    } else {
                        &[1, 2]
                    };
                    binary.extend_from_slice(selectors);
                    let values = binary.len();
                    // Sparse VEC3 values are packed, without the dense stream's sentinels.
                    for _ in selectors {
                        binary.extend_from_slice(sample);
                    }
                    document["buffers"][0]["byteLength"] = json!(binary.len());
                    document["bufferViews"].as_array_mut().unwrap().extend([
                        json!({"buffer": 0, "byteOffset": start, "byteLength": selectors.len()}),
                        json!({"buffer": 0, "byteOffset": values, "byteLength": sample.len() * selectors.len()}),
                    ]);
                    // SHORT replacements need component alignment, never vertex padding.
                    if (case.1 == 5122 || case.1 == 5123) && !values.is_multiple_of(2) {
                        binary.insert(values, 0x5a);
                        document["bufferViews"][6]["byteOffset"] = json!(values + 1);
                        document["buffers"][0]["byteLength"] = json!(binary.len());
                    }
                    document["accessors"][4]["sparse"] = json!({
                        "count": selectors.len(),
                        "indices": {"bufferView": 5, "componentType": 5121},
                        "values": {"bufferView": 6}
                    });
                    if storage == "zero-base" {
                        document["accessors"][4]
                            .as_object_mut()
                            .unwrap()
                            .remove("bufferView");
                    }
                }
                let (report, diagnostics) = import_assets(
                    SOURCE,
                    [(SOURCE, serde_json::to_vec(&document)?), (BINARY, binary)],
                    sidecar_options(),
                )?;
                assert!(diagnostics.is_empty(), "{case:?} {storage}");
                assert!(report.loss_evidence().is_empty());
                let scene = report.scene();
                let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
                assert_eq!(primitive.topology(), TopologyKind::Points);
                assert_eq!((primitive.vertex_count(), primitive.corner_count()), (3, 3));
                assert_eq!(primitive.indices(), None);
                let owner = if morph {
                    let target = primitive.morph_targets().next().unwrap();
                    let semantic = match semantic {
                        "POSITION" => MorphDeltaSemantic::Position,
                        "NORMAL" => MorphDeltaSemantic::Normal,
                        "TANGENT" => MorphDeltaSemantic::Tangent,
                        _ => unreachable!(),
                    };
                    let delta = target.delta(semantic).unwrap();
                    assert_eq!((target.delta_count(), delta.len()), (1, 3));
                    for vertex in 0..3 {
                        let value = delta.data().value(vertex).unwrap();
                        assert_eq!(
                            [value.x, value.y, value.z].map(f32::to_bits),
                            [expected[0], expected[1], expected[2]].map(f32::to_bits),
                            "{case:?} {storage} vertex {vertex}"
                        );
                    }
                    assert_eq!(delta.data().value(3), None);
                    delta.metadata_owner()
                } else {
                    let semantic = match semantic {
                        "POSITION" => AttributeSemanticRef::Position,
                        "NORMAL" => AttributeSemanticRef::Normal,
                        "TANGENT" => AttributeSemanticRef::Tangent,
                        "TEXCOORD_0" => AttributeSemanticRef::Texcoord(0),
                        _ => unreachable!(),
                    };
                    let attribute = primitive
                        .attributes()
                        .find(|a| a.semantic() == semantic)
                        .unwrap();
                    assert_eq!(attribute.domain(), AttributeDomain::Vertex);
                    assert_eq!(
                        (usize::from(attribute.lanes()), attribute.len()),
                        (expected.len(), 3)
                    );
                    assert!(matches!(attribute.data(), AttributeData::DenseF32 { .. }));
                    for vertex in 0..3 {
                        let Some(AttributeElement::F32(value)) = attribute.element(vertex) else {
                            panic!("expected F32 {case:?} {storage}");
                        };
                        assert_eq!(
                            value.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                            expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                        );
                    }
                    assert_eq!(attribute.element(3), None);
                    attribute.metadata_owner()
                };
                let mut pending = scene.owner_provenance(owner)?.collect::<Vec<_>>();
                let mut visited = Vec::new();
                let mut locators = Vec::new();
                let mut resources = Vec::new();
                while let Some(node) = pending.pop() {
                    if visited.contains(&node.id()) {
                        continue;
                    }
                    assert_eq!(node.id().generation(), scene.generation());
                    visited.push(node.id());
                    assert!(visited.len() <= scene.provenance_node_count());
                    if let ProvenanceKindRef::Source(source) = node.kind() {
                        assert_eq!(source.generation(), scene.generation());
                        resources.push(source.resource().identity());
                        if let SourceObjectLocatorRef::Namespaced { value, .. } = source.locator() {
                            locators.push(value);
                        }
                    }
                    pending.extend(node.parents());
                }
                assert!(
                    locators
                        .iter()
                        .any(|value| value.starts_with("kind=accessor;parent=-;index=4;"))
                );
                assert!(resources.contains(&BINARY));
                if storage != "zero-base" {
                    assert!(
                        locators
                            .iter()
                            .any(|value| value.starts_with("kind=buffer-view;parent=-;index=4;"))
                    );
                }
                if storage != "dense" {
                    for prefix in [
                        "kind=buffer-view;parent=-;index=5;",
                        "kind=buffer-view;parent=-;index=6;",
                    ] {
                        assert!(
                            locators.iter().any(|value| value.starts_with(prefix)),
                            "{prefix}"
                        );
                    }
                }
                assert_eq!(report.ledger().current_live_bytes(), 0);
                assert_eq!(report.ledger().active_reservations(), 0);
            }
        }
    }
    Ok(())
}

#[test]
fn scalar_boundary_lists_have_independent_literal_normalization_bits() -> TestResult<()> {
    // F32 oracle bits were calculated from the specification's rational values, not Baozi.
    for (component, source, decoded) in [
        (
            5120,
            vec![-128, -127, -64, -1, 0, 1, 64, 127],
            vec![
                0xbf800000, 0xbf800000, 0xbf010204, 0xbc010204, 0, 0x3c010204, 0x3f010204,
                0x3f800000,
            ],
        ),
        (
            5122,
            vec![-32768, -32767, -16384, -1, 0, 1, 16384, 32767],
            vec![
                0xbf800000, 0xbf800000, 0xbf000100, 0xb8000100, 0, 0x38000100, 0x3f000100,
                0x3f800000,
            ],
        ),
        (
            5121,
            vec![0, 1, 127, 128, 254, 255],
            vec![
                0, 0x3b808081, 0x3efefeff, 0x3f008081, 0x3f7efeff, 0x3f800000,
            ],
        ),
        (
            5123,
            vec![0, 1, 32767, 32768, 65534, 65535],
            vec![
                0, 0x37800080, 0x3effff00, 0x3f000080, 0x3f7fff00, 0x3f800000,
            ],
        ),
    ] {
        for normalized in [false, true] {
            let (mut document, mut binary) = fixture(BASE_CASES[0], false);
            binary.truncate(144);
            for &value in &source {
                match component {
                    5120 => binary.extend_from_slice(&i8::try_from(value)?.to_le_bytes()),
                    5121 => binary.extend_from_slice(&u8::try_from(value)?.to_le_bytes()),
                    5122 => binary.extend_from_slice(&i16::try_from(value)?.to_le_bytes()),
                    5123 => binary.extend_from_slice(&u16::try_from(value)?.to_le_bytes()),
                    _ => unreachable!(),
                }
                let stride = if component <= 5121 { 4 } else { 8 };
                let width = if component <= 5121 { 1 } else { 2 };
                binary.resize(binary.len() + 2 * width, 0);
                binary.resize(144 + (binary.len() - 144).div_ceil(stride) * stride, 0xa5);
            }
            document["meshes"][0]["primitives"][0]["attributes"] = json!({"POSITION": 4});
            document["buffers"][0]["byteLength"] = json!(binary.len());
            document["bufferViews"][4] = json!({"buffer": 0, "byteOffset": 144,
                "byteLength": binary.len() - 144, "byteStride": if component <= 5121 {4} else {8}});
            document["accessors"][4] = json!({"bufferView": 4, "componentType": component,
                "normalized": normalized, "count": source.len(), "type": "VEC3",
                "min": [source[0], 0, 0], "max": [source[source.len() - 1], 0, 0]});
            let (report, _) = import_assets(
                SOURCE,
                [(SOURCE, serde_json::to_vec(&document)?), (BINARY, binary)],
                sidecar_options(),
            )?;
            let positions = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .positions()
                .unwrap();
            for (index, (&stored, &bits)) in source.iter().zip(&decoded).enumerate() {
                let Some(AttributeElement::F32(value)) = positions.element(index) else {
                    panic!("F32");
                };
                assert_eq!(
                    value[0].to_bits(),
                    if normalized {
                        bits
                    } else {
                        (stored as f32).to_bits()
                    }
                );
                assert_eq!([value[1].to_bits(), value[2].to_bits()], [0, 0]);
            }
        }
    }
    Ok(())
}

#[test]
fn declarations_are_global_and_used_only_is_invalid_even_for_float_geometry() -> TestResult<()> {
    for float_only in [false, true] {
        for (used, required) in [
            (None, None),
            (Some(json!([QUANTIZATION])), None),
            (None, Some(json!([QUANTIZATION]))),
            (
                Some(json!([QUANTIZATION, QUANTIZATION])),
                Some(json!([QUANTIZATION])),
            ),
            (
                Some(json!([QUANTIZATION])),
                Some(json!([QUANTIZATION, QUANTIZATION])),
            ),
            (Some(json!([QUANTIZATION, ""])), Some(json!([QUANTIZATION]))),
            (
                Some(json!([QUANTIZATION, "VENDOR_required"])),
                Some(json!([QUANTIZATION, "VENDOR_required"])),
            ),
        ] {
            if float_only && used.is_none() && required.is_none() {
                continue;
            }
            let unsupported_required = required.as_ref().is_some_and(|names| {
                names
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|name| name == "VENDOR_required")
            });
            let (mut document, _) = fixture(BASE_CASES[0], false);
            if float_only {
                document["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(0);
            }
            for (name, value) in [("extensionsUsed", used), ("extensionsRequired", required)] {
                document.as_object_mut().unwrap().remove(name);
                if let Some(value) = value {
                    document[name] = value;
                }
            }
            let (result, _) = import_assets_result(
                SOURCE,
                [(SOURCE, serde_json::to_vec(&document)?)],
                sidecar_options(),
            )?;
            let failure = expected_error(result)?;
            assert_gltf_failure(
                &failure,
                if unsupported_required {
                    GltfFailureCode::UnsupportedFeature
                } else {
                    GltfFailureCode::InvalidDocument
                },
                None,
            )?;
            assert!(failure.source_scope_record().is_none());
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
            assert_eq!(failure.ledger().current_live_bytes(), 0);
        }
    }
    // JSON escapes cannot conceal a duplicate declaration name.
    let (mut document, _) = fixture(BASE_CASES[0], false);
    document["extensionsUsed"] = json!([QUANTIZATION, "escaped-name"]);
    let primary =
        serde_json::to_string(&document)?.replace("escaped-name", r"KHR_mesh_\u0071uantization");
    let (result, _) =
        import_assets_result(SOURCE, [(SOURCE, primary.into_bytes())], sidecar_options())?;
    let failure = expected_error(result)?;
    assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .unwrap()
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn omitted_quantized_mesh_needs_declarations_but_never_opens_its_buffer() -> TestResult<()> {
    for declared in [false, true] {
        let (mut document, _) = fixture(BASE_CASES[4], false);
        document["nodes"] = json!([{}]);
        if !declared {
            document.as_object_mut().unwrap().remove("extensionsUsed");
            document
                .as_object_mut()
                .unwrap()
                .remove("extensionsRequired");
        }
        let (result, _) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        if declared {
            let report = result?;
            assert_eq!(report.scene().mesh_count(), 0);
            let scope = report.source_scope_record().unwrap();
            assert_eq!(scope.counts(SourceScopeEntityKind::Mesh).omitted(), 1);
            assert_eq!(scope.counts(SourceScopeEntityKind::Buffer).selected(), 0);
            assert_eq!(scope.counts(SourceScopeEntityKind::Buffer).omitted(), 1);
            assert!(
                scope
                    .details()
                    .any(|row| row.kind() == SourceScopeEntityKind::Buffer
                        && row.identity() == "b0"
                        && row.disposition() == SourceScopeDisposition::Omitted)
            );
            assert_eq!(
                report
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
        } else {
            let failure = expected_error(result)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
        }
    }
    // An unused accessor row does not establish extension use by itself.
    let (mut document, binary) = fixture(BASE_CASES[4], false);
    document["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(0);
    document.as_object_mut().unwrap().remove("extensionsUsed");
    document
        .as_object_mut()
        .unwrap()
        .remove("extensionsRequired");
    let (report, _) = import_assets(
        SOURCE,
        [(SOURCE, serde_json::to_vec(&document)?), (BINARY, binary)],
        sidecar_options(),
    )?;
    assert_eq!(report.scene().mesh_count(), 1);
    Ok(())
}

#[test]
fn core_float_and_unsigned_normalized_uvs_need_no_quantization_declaration() -> TestResult<()> {
    for component in [5126, 5121, 5123] {
        for declared in [false, true] {
            let (mut document, mut binary) = fixture(BASE_CASES[16], false);
            let sample: &[u8] = match component {
                5121 => &[128, 255],
                5123 => &[0, 128, 255, 255],
                5126 => &[0, 0, 128, 62, 0, 0, 64, 63],
                _ => unreachable!(),
            };
            let stride = if component == 5126 { 8 } else { 4 };
            binary.truncate(144);
            for _ in 0..3 {
                binary.extend_from_slice(sample);
                binary.resize(binary.len() + stride - sample.len(), 0xa5);
            }
            document["buffers"][0]["byteLength"] = json!(binary.len());
            document["bufferViews"][4]["byteLength"] = json!(3 * stride);
            document["bufferViews"][4]["byteStride"] = json!(stride);
            document["accessors"][4] = json!({"bufferView": 4, "componentType": component,
                "normalized": component != 5126, "count": 3, "type": "VEC2"});
            if !declared {
                document.as_object_mut().unwrap().remove("extensionsUsed");
                document
                    .as_object_mut()
                    .unwrap()
                    .remove("extensionsRequired");
            }
            let (report, _) = import_assets(
                SOURCE,
                [(SOURCE, serde_json::to_vec(&document)?), (BINARY, binary)],
                sidecar_options(),
            )?;
            let uv = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .texcoords(0, AttributeDomain::Vertex)
                .unwrap();
            let Some(AttributeElement::F32(value)) = uv.element(0) else {
                panic!("F32 UV");
            };
            let expected: [f32; 2] = match component {
                5121 => [0.5019608, 1.0],
                5123 => [0.5000076, 1.0],
                5126 => [0.25, 0.75],
                _ => unreachable!(),
            };
            assert_eq!(
                value.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                expected.map(f32::to_bits)
            );
        }
    }
    Ok(())
}

#[test]
fn declaration_support_does_not_admit_a_fake_object_payload_or_lose_its_raw_range() -> TestResult<()>
{
    for site in ["", "/accessors/4", "/meshes/0/primitives/0", "/materials/0"] {
        for escaped in [false, true] {
            let (mut document, _) = fixture(BASE_CASES[0], false);
            document["materials"] = json!([{}]);
            document["meshes"][0]["primitives"][0]["material"] = json!(0);
            document.pointer_mut(site).unwrap()["extensions"] = json!({QUANTIZATION: {"fake": 17}});
            let mut primary = serde_json::to_string(&document)?;
            let key = if escaped {
                r#""KHR_mesh_\u0071uantization""#
            } else {
                r#""KHR_mesh_quantization""#
            };
            if escaped {
                primary = primary.replace(r#""KHR_mesh_quantization":{"#, &format!("{key}:{{"));
            }
            let marker = format!("{key}:{{\"fake\":17}}");
            let key_offset = primary.find(&marker).unwrap();
            let value_offset = key_offset + key.len() + 1;
            for (source, bytes, container_offset) in [
                (SOURCE, primary.as_bytes().to_vec(), 0_u64),
                (
                    "models/quantized.glb",
                    glb_with_json(primary.as_bytes()),
                    20,
                ),
            ] {
                let (result, _) =
                    import_assets_result(source, [(source, bytes)], sidecar_options())?;
                let failure = expected_error(result)?;
                assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
                let details = gltf_failure(&failure)?.details();
                for (name, expected) in [
                    ("byte_offset", value_offset as u64 + container_offset),
                    ("byte_length", 11),
                ] {
                    assert!(details.iter().any(|detail| detail.key() == name
                        && matches!(detail.value(), FormatFailureDetailValue::Unsigned(value) if value == expected)), "{name} {failure:?}");
                }
                assert_eq!(
                    &primary.as_bytes()[key_offset..key_offset + key.len()],
                    key.as_bytes()
                );
                assert_eq!(
                    failure
                        .ledger()
                        .dimension(ImportDimension::DependencyReference)
                        .unwrap()
                        .attempted(),
                    0
                );
            }
        }
    }
    Ok(())
}

#[test]
fn wrong_mesh_encodings_still_fail_with_the_extension_enabled() -> TestResult<()> {
    for (morph, semantic, component, normalized, shape) in [
        (false, "POSITION", 5125, false, "VEC3"),
        (false, "POSITION", 5126, true, "VEC3"),
        (false, "NORMAL", 5120, true, "VEC2"),
        (false, "NORMAL", 5121, true, "VEC3"),
        (false, "NORMAL", 5123, true, "VEC3"),
        (false, "NORMAL", 5120, false, "VEC3"),
        (false, "NORMAL", 5122, false, "VEC3"),
        (false, "TANGENT", 5121, true, "VEC4"),
        (false, "TANGENT", 5120, false, "VEC4"),
        (false, "TANGENT", 5122, false, "VEC4"),
        (false, "TANGENT", 5120, true, "VEC3"),
        (false, "TEXCOORD_0", 5125, false, "VEC2"),
        (false, "COLOR_0", 5120, true, "VEC4"),
        (false, "WEIGHTS_0", 5120, true, "VEC4"),
        (false, "JOINTS_0", 5121, true, "VEC4"),
        (true, "POSITION", 5121, false, "VEC3"),
        (true, "POSITION", 5121, true, "VEC3"),
        (true, "POSITION", 5123, false, "VEC3"),
        (true, "POSITION", 5123, true, "VEC3"),
        (true, "NORMAL", 5120, false, "VEC3"),
        (true, "TANGENT", 5120, true, "VEC4"),
    ] {
        let (mut document, _) = fixture(BASE_CASES[0], false);
        document["bufferViews"][4] =
            json!({"buffer": 0, "byteOffset": 144, "byteLength": 96, "byteStride": 32});
        document["buffers"][0]["byteLength"] = json!(240);
        document["accessors"][4] = json!({"bufferView": 4, "componentType": component,
            "normalized": normalized, "count": 3, "type": shape});
        if semantic == "POSITION" {
            document["accessors"][4]["min"] = json!(if shape == "VEC2" {
                vec![0, 0]
            } else {
                vec![0, 0, 0]
            });
            document["accessors"][4]["max"] = document["accessors"][4]["min"].clone();
        }
        document["meshes"][0]["primitives"][0]["attributes"] =
            json!({"POSITION": 0, "NORMAL": 1, "TANGENT": 2});
        if semantic == "WEIGHTS_0" || semantic == "JOINTS_0" {
            let companion = if semantic == "WEIGHTS_0" {
                "JOINTS_0"
            } else {
                "WEIGHTS_0"
            };
            document["accessors"].as_array_mut().unwrap().push(json!({
                "componentType": if companion == "JOINTS_0" { 5121 } else { 5126 }, "count": 3, "type": "VEC4"
            }));
            document["meshes"][0]["primitives"][0]["attributes"][companion] = json!(5);
        }
        if morph {
            document["meshes"][0]["primitives"][0]["targets"] = json!([{semantic: 4}]);
        } else {
            document["meshes"][0]["primitives"][0]["attributes"][semantic] = json!(4);
        }
        let (result, _) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidAccessor, None)?;
        assert!(
            gltf_failure(&failure)?
                .details()
                .iter()
                .any(|detail| detail.key() == "index"
                    && matches!(detail.value(), FormatFailureDetailValue::Unsigned(4)))
        );
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
    }
    Ok(())
}

#[test]
fn tangent_w_accepts_only_the_two_normalized_signs() -> TestResult<()> {
    for (stored_w, expected) in [
        (128, Some(-1.0_f32)),
        (129, Some(-1.0)),
        (127, Some(1.0)),
        (0, None),
        (126, None),
    ] {
        let (mut document, mut binary) = fixture(BASE_CASES[10], false);
        document["accessors"][4]
            .as_object_mut()
            .unwrap()
            .remove("min");
        document["accessors"][4]
            .as_object_mut()
            .unwrap()
            .remove("max");
        for vertex in 0..3 {
            binary[144 + vertex * 4 + 3] = stored_w;
        }
        let (result, _) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?), (BINARY, binary)],
            sidecar_options(),
        )?;
        if let Some(expected) = expected {
            let report = result?;
            let tangent = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
                .unwrap();
            let Some(AttributeElement::F32(value)) = tangent.element(0) else {
                panic!("F32 tangent");
            };
            assert_eq!(value[3].to_bits(), expected.to_bits());
        } else {
            let failure = expected_error(result)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidAccessor, None)?;
            assert_eq!(failure.ledger().current_live_bytes(), 0);
        }
    }
    Ok(())
}

#[test]
fn short_component_stride_alignment_and_ranges_are_checked_before_io() -> TestResult<()> {
    for (stride, accessor_offset, view_offset, view_length) in [
        (None, 0, 144, 96),
        (Some(3), 0, 144, 96),
        (Some(6), 0, 144, 96),
        (Some(4), 0, 144, 96),
        (Some(256), 0, 144, 768),
        (Some(8), 2, 144, 96),
        (Some(8), 0, 145, 96),
        (Some(8), 2, 146, 96),
        (Some(8), 0, 144, 21),
        (Some(8), u64::MAX - 3, 144, 96),
    ] {
        let (mut document, _) = fixture(BASE_CASES[4], false);
        document["buffers"][0]["byteLength"] = json!(1024);
        document["bufferViews"][4] =
            json!({"buffer": 0, "byteOffset": view_offset, "byteLength": view_length});
        if let Some(stride) = stride {
            document["bufferViews"][4]["byteStride"] = json!(stride);
        }
        document["accessors"][4]["byteOffset"] = json!(accessor_offset);
        let (result, _) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        let failure = expected_error(result)?;
        // The decoder checks the schema range for every bufferView, before a
        // selected accessor owns the remaining layout checks.
        let code = if stride.is_some_and(|stride| !(4..=252).contains(&stride)) {
            GltfFailureCode::InvalidDocument
        } else {
            GltfFailureCode::InvalidAccessor
        };
        assert_gltf_failure(&failure, code, None)?;
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
    }
    // No trailing padding after the last element; view-relative alignment remains authoritative.
    for (case, count, view_offset, view_length, expected_ok) in [
        (BASE_CASES[0], 2, 144, 7, true),
        (BASE_CASES[0], 1, 144, 3, false),
        (BASE_CASES[17], 3, 146, 12, true),
        (BASE_CASES[17], 3, 145, 12, false),
    ] {
        let (mut document, mut binary) = fixture(case, false);
        document["meshes"][0]["primitives"][0]["attributes"] =
            json!({case.0: 4, "POSITION": if case.0 == "POSITION" {4} else {0}});
        document["accessors"][4]["count"] = json!(count);
        if view_offset == 146 {
            binary.splice(144..144, [0x5a, 0x5a]);
        }
        binary.resize(binary.len().max(view_offset + view_length), 0x5a);
        document["buffers"][0]["byteLength"] = json!(binary.len());
        document["bufferViews"][4]["byteOffset"] = json!(view_offset);
        document["bufferViews"][4]["byteLength"] = json!(view_length);
        let (result, _) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?), (BINARY, binary)],
            sidecar_options(),
        )?;
        if expected_ok {
            let report = result?;
            let primitive = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap();
            assert_eq!(primitive.vertex_count(), count);
        } else {
            let failure = expected_error(result)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidAccessor, None)?;
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
        }
    }
    Ok(())
}

#[test]
fn normalized_position_bounds_remain_exact_source_integers_for_base_and_morph() -> TestResult<()> {
    for morph in [false, true] {
        for (min, max, metadata_failure, accepted) in [
            (json!([-128, 0, 0]), json!([-127, 0, 0]), false, true),
            (json!([-127, 0, 0]), json!([-127, 0, 0]), false, false),
            (json!([-128, 0, 0]), json!([-128, 0, 0]), false, false),
            (json!([-1, 0, 0]), json!([-1, 0, 0]), false, false),
            (
                json!([-128.0000001, 0, 0]),
                json!([-127, 0, 0]),
                true,
                false,
            ),
            (
                json!([-128, 0, 0]),
                json!([-126.9999999, 0, 0]),
                true,
                false,
            ),
            (json!([-129, 0, 0]), json!([-127, 0, 0]), true, false),
            (json!([-128, 0, 0]), json!([128, 0, 0]), true, false),
            (json!([-127, 0, 0]), json!([-128, 0, 0]), true, false),
            (json!([-128, 0]), json!([-127, 0, 0]), true, false),
            (Value::Null, json!([-127, 0, 0]), true, false),
        ] {
            let (mut document, mut binary) = fixture(BASE_CASES[1], morph);
            binary[144..156].copy_from_slice(&[128, 0, 0, 0xa5, 129, 0, 0, 0xa5, 129, 0, 0, 0xa5]);
            document["accessors"][4]["min"] = min.clone();
            if min.is_null() {
                document["accessors"][4]
                    .as_object_mut()
                    .unwrap()
                    .remove("min");
            }
            document["accessors"][4]["max"] = max;
            let (result, _) = import_assets_result(
                SOURCE,
                [(SOURCE, serde_json::to_vec(&document)?), (BINARY, binary)],
                sidecar_options(),
            )?;
            if accepted {
                let report = result?;
                let primitive = report
                    .scene()
                    .meshes()
                    .next()
                    .unwrap()
                    .primitives()
                    .next()
                    .unwrap();
                if morph {
                    let data = primitive
                        .morph_targets()
                        .next()
                        .unwrap()
                        .delta(MorphDeltaSemantic::Position)
                        .unwrap()
                        .data();
                    assert_eq!(data.value(0), Some(Vec3::new(-1.0, 0.0, 0.0)));
                    assert_eq!(data.value(1), Some(Vec3::new(-1.0, 0.0, 0.0)));
                } else {
                    let positions = primitive.positions().unwrap();
                    assert_eq!(positions.element(0), positions.element(1));
                    assert_eq!(
                        positions.element(0),
                        Some(AttributeElement::F32(&[-1.0, 0.0, 0.0]))
                    );
                }
            } else {
                let failure = expected_error(result)?;
                assert_gltf_failure(&failure, GltfFailureCode::InvalidAccessor, None)?;
                let attempted = failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted();
                assert_eq!(attempted, if metadata_failure { 0 } else { 1 });
                assert_eq!(failure.ledger().current_live_bytes(), 0);
                assert_eq!(failure.ledger().active_reservations(), 0);
            }
        }
    }
    Ok(())
}

#[test]
fn packed_sparse_positions_merge_in_the_source_domain_in_three_separate_buffers() -> TestResult<()>
{
    for (selector_component, selectors) in [
        (5121, vec![1, 3]),
        (5123, vec![1, 0, 3, 0]),
        (5125, vec![1, 0, 0, 0, 3, 0, 0, 0]),
    ] {
        for with_base in [false, true] {
            let (mut document, mut binary) = fixture(BASE_CASES[1], false);
            document["meshes"][0]["primitives"][0]["attributes"] = json!({"POSITION": 4});
            document["accessors"][4]["count"] = json!(4);
            document["accessors"][4]["min"] = json!([-128, -127, 0]);
            document["accessors"][4]["max"] = json!([127, 64, 2]);
            binary.truncate(144);
            binary.extend_from_slice(&[
                0, 0, 0, 0xa5, 128, 0, 0, 0xa5, 0, 0, 0, 0xa5, 128, 0, 0, 0xa5,
            ]);
            document["bufferViews"][4]["byteLength"] = json!(16);
            document["buffers"] = json!([
                {"uri": "quantized.bin", "byteLength": binary.len()},
                {"uri": "selectors.bin", "byteLength": selectors.len()},
                {"uri": "replacements.bin", "byteLength": 7}
            ]);
            document["bufferViews"].as_array_mut().unwrap().extend([
                json!({"buffer": 1, "byteLength": selectors.len()}),
                json!({"buffer": 2, "byteOffset": 1, "byteLength": 6}),
            ]);
            document["accessors"][4]["sparse"] = json!({"count": 2,
                "indices": {"bufferView": 5, "componentType": selector_component},
                "values": {"bufferView": 6}});
            if !with_base {
                document["accessors"][4]
                    .as_object_mut()
                    .unwrap()
                    .remove("bufferView");
            }
            let (report, _) = import_assets(
                SOURCE,
                [
                    (SOURCE, serde_json::to_vec(&document)?),
                    (BINARY, binary),
                    ("models/selectors.bin", selectors.clone()),
                    (
                        "models/replacements.bin",
                        vec![0x5a, 128, 64, 0, 127, 129, 2],
                    ),
                ],
                sidecar_options(),
            )?;
            let positions = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .positions()
                .unwrap();
            for (vertex, expected) in [
                [0.0_f32, 0.0, 0.0],
                [-1.0, 0.503937, 0.0],
                [0.0, 0.0, 0.0],
                [1.0, -1.0, 0.015748031],
            ]
            .into_iter()
            .enumerate()
            {
                let Some(AttributeElement::F32(actual)) = positions.element(vertex) else {
                    panic!("F32");
                };
                assert_eq!(
                    actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    expected.map(f32::to_bits)
                );
            }
            assert_eq!(
                report
                    .source_scope_record()
                    .unwrap()
                    .counts(SourceScopeEntityKind::Buffer)
                    .selected(),
                if with_base { 3 } else { 2 }
            );
            assert_eq!(report.ledger().current_live_bytes(), 0);
            assert_eq!(report.ledger().active_reservations(), 0);
        }
    }
    Ok(())
}

#[test]
fn valid_uv_morphs_have_a_named_unsupported_profile_failure() -> TestResult<()> {
    for (component, normalized) in [
        (5126, false),
        (5120, true),
        (5122, true),
        (5121, true),
        (5123, true),
        (5120, false),
        (5122, false),
    ] {
        let (mut document, _) = fixture(BASE_CASES[12], false);
        document["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(3);
        document["meshes"][0]["primitives"][0]["targets"] = json!([{"TEXCOORD_0": 4}]);
        document["bufferViews"][4] =
            json!({"buffer": 0, "byteOffset": 144, "byteLength": 24, "byteStride": 8});
        document["buffers"][0]["byteLength"] = json!(168);
        document["accessors"][4] = json!({"bufferView": 4, "componentType": component,
            "normalized": normalized, "count": 3, "type": "VEC2"});
        let (result, _) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
        assert!(gltf_failure_reason(&failure)?.contains("TEXCOORD_0"));
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
    }
    Ok(())
}

#[test]
fn quantized_logical_vertex_limits_do_not_shrink_with_the_source_scalar_width() -> TestResult<()> {
    let (document, binary) = fixture(BASE_CASES[0], false);
    for limit in [3, 2] {
        let mut options = sidecar_options();
        options.limits.max_vertices = limit;
        let (result, _) = import_assets_result(
            SOURCE,
            [
                (SOURCE, serde_json::to_vec(&document)?),
                (BINARY, binary.clone()),
            ],
            options,
        )?;
        if limit == 3 {
            let report = result?;
            assert_eq!(
                report
                    .ledger()
                    .dimension(ImportDimension::Vertices)
                    .unwrap()
                    .consumed(),
                3
            );
            let positions = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .positions()
                .unwrap();
            let AttributeData::DenseF32 { values, lanes: 3 } = positions.data() else {
                panic!("F32x3");
            };
            assert_eq!(std::mem::size_of_val(values), 36);
        } else {
            let failure = expected_error(result)?;
            assert_resource_limit(&failure, "max_vertices");
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }
    Ok(())
}

#[test]
fn quantized_morph_cancellation_uses_calibrated_polls_and_releases_storage() -> TestResult<()> {
    let (mut document, mut binary) = fixture(MORPH_CASES[0], true);
    const COUNT: usize = 1025;
    // A finite larger sample reaches the accessor/bounds/morph loops without large allocations.
    binary.clear();
    for _ in 0..COUNT {
        binary.extend_from_slice(&[128, 64, 0, 0xa5]);
    }
    document["meshes"][0]["primitives"][0]["attributes"] = json!({"POSITION": 0});
    document["accessors"][0] = json!({"componentType": 5126, "count": COUNT,
        "type": "VEC3", "min": [0,0,0], "max": [0,0,0]});
    document["accessors"][4]["bufferView"] = json!(0);
    document["accessors"][4]["count"] = json!(COUNT);
    document["bufferViews"] = json!([{"buffer": 0, "byteLength": binary.len(), "byteStride": 4}]);
    // Remove the unused dense controls before replacing the view table.
    document["accessors"] = json!([
        document["accessors"][0].clone(),
        document["accessors"][4].clone()
    ]);
    document["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION": 1}]);
    document["buffers"][0]["byteLength"] = json!(binary.len());
    let primary = serde_json::to_vec(&document)?;
    let (control, polls) = controlled_import(None);
    let (result, _) = import_assets_result_with_control(
        SOURCE,
        [(SOURCE, primary.clone()), (BINARY, binary.clone())],
        sidecar_options(),
        control,
    )?;
    let report = result?;
    let total = polls.load(Ordering::SeqCst);
    assert!(total > 4);
    drop(report);
    for cancel_on in [total / 4, total / 2, 3 * (total / 4)] {
        let (control, polls) = controlled_import(Some(cancel_on));
        let (result, _) = import_assets_result_with_control(
            SOURCE,
            [(SOURCE, primary.clone()), (BINARY, binary.clone())],
            sidecar_options(),
            control,
        )?;
        let failure = expected_error(result)?;
        assert!(matches!(failure.cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        }))));
        assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
    Ok(())
}
