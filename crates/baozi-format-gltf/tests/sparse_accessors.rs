mod common;

use baozi_core::{AttributeElement, ProvenanceKindRef};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::authoring::{AttributeDomain, Mat4, MorphDeltaSemantic, Vec3};
use baozi_import::{
    AnimationTarget, EntityMetadataOwner, FormatFailureDetailValue, ImportDimension, ImportFailure,
    ImportReport, SourceIdentityRef, SourceObjectLocatorRef, SourceScopeDisposition,
    SourceScopeEntityKind,
};
use common::{
    TestResult, assert_gltf_failure, assert_resource_limit, expected_error, gltf_failure,
    gltf_failure_reason, import_assets, import_assets_result, sidecar_options,
    skinned_triangle_bin, skinned_triangle_gltf, triangle_bin, triangle_gltf_with_buffer_uri,
};
use serde_json::{Value, json};

const SOURCE: &str = "models/sparse.gltf";
const BASE: &str = "models/base.bin";
const INDICES: &str = "models/sparse-indices.bin";
const VALUES: &str = "models/sparse-values.bin";

fn dense_triangle() -> TestResult<(Value, Vec<u8>)> {
    Ok((
        serde_json::from_slice(&triangle_gltf_with_buffer_uri("base.bin", 104))?,
        triangle_bin(),
    ))
}

fn encoded_indices(indices: &[u32], component: u32) -> Vec<u8> {
    indices
        .iter()
        .flat_map(|&index| match component {
            5121 => vec![u8::try_from(index).expect("fixture index fits U8")],
            5123 => u16::try_from(index)
                .expect("fixture index fits U16")
                .to_le_bytes()
                .to_vec(),
            5125 => index.to_le_bytes().to_vec(),
            _ => panic!("unexpected sparse index component {component}"),
        })
        .collect()
}

fn preserve_first_vertex(binary: &mut [u8]) {
    for (lane, value) in [0.25_f32, 0.25, 0.0].into_iter().enumerate() {
        binary[lane * 4..lane * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn sparse_triangle(component: u32, with_base: bool) -> TestResult<(Value, [Vec<u8>; 3])> {
    let (mut document, mut base) = dense_triangle()?;
    let mut indices = vec![0xa5; 8];
    indices.extend(encoded_indices(&[1, 2], component));
    let mut values = vec![0xa5; 8];
    values.extend_from_slice(&base[12..36]);

    // Both view and sparse offsets are nonzero. Prefix bytes must never become payload.
    document["buffers"].as_array_mut().unwrap().extend([
        json!({"uri": "sparse-indices.bin", "byteLength": indices.len()}),
        json!({"uri": "sparse-values.bin", "byteLength": values.len()}),
    ]);
    document["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 1, "byteOffset": 4, "byteLength": indices.len() - 4}),
        json!({"buffer": 2, "byteOffset": 4, "byteLength": values.len() - 4}),
    ]);
    document["accessors"][0]["sparse"] = json!({
        "count": 2,
        "indices": {"bufferView": 4, "byteOffset": 4, "componentType": component},
        "values": {"bufferView": 5, "byteOffset": 4}
    });
    base[..36].fill(0);
    if with_base {
        preserve_first_vertex(&mut base);
    } else {
        document["accessors"][0]
            .as_object_mut()
            .unwrap()
            .remove("bufferView");
    }
    Ok((document, [base, indices, values]))
}

fn import_sparse(
    document: &Value,
    [base, indices, values]: [Vec<u8>; 3],
) -> TestResult<ImportReport> {
    let (report, diagnostics) = import_assets(
        SOURCE,
        [
            (SOURCE, serde_json::to_vec(document)?),
            (BASE, base),
            (INDICES, indices),
            (VALUES, values),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Ok(report)
}

fn import_dense(document: &Value, binary: Vec<u8>) -> TestResult<ImportReport> {
    let (report, diagnostics) = import_assets(
        SOURCE,
        [(SOURCE, serde_json::to_vec(document)?), (BASE, binary)],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Ok(report)
}

fn assert_geometry_equivalent(actual: &ImportReport, expected: &ImportReport) {
    let actual_mesh = actual.scene().meshes().next().unwrap();
    let expected_mesh = expected.scene().meshes().next().unwrap();
    assert_eq!(actual.scene().mesh_count(), expected.scene().mesh_count());
    assert_eq!(
        actual_mesh.primitive_count(),
        expected_mesh.primitive_count()
    );
    for (actual, expected) in actual_mesh.primitives().zip(expected_mesh.primitives()) {
        assert_eq!(actual.topology(), expected.topology());
        assert_eq!(actual.vertex_count(), expected.vertex_count());
        assert_eq!(actual.corner_count(), expected.corner_count());
        assert_eq!(actual.face_count(), expected.face_count());
        assert_eq!(actual.indices(), expected.indices());
        assert_eq!(actual.attributes().len(), expected.attributes().len());
        for (actual, expected) in actual.attributes().zip(expected.attributes()) {
            assert_eq!(actual.semantic(), expected.semantic());
            assert_eq!(actual.domain(), expected.domain());
            assert_eq!(actual.lanes(), expected.lanes());
            assert_eq!(actual.len(), expected.len());
            assert_eq!(actual.tangent_conventions(), expected.tangent_conventions());
            for vertex in 0..actual.len() {
                match (actual.element(vertex), expected.element(vertex)) {
                    (
                        Some(AttributeElement::F32(actual_lanes)),
                        Some(AttributeElement::F32(expected_lanes)),
                    ) => {
                        let actual_bits = actual_lanes
                            .iter()
                            .map(|lane| lane.to_bits())
                            .collect::<Vec<_>>();
                        let expected_bits = expected_lanes
                            .iter()
                            .map(|lane| lane.to_bits())
                            .collect::<Vec<_>>();
                        assert_eq!(
                            actual_bits,
                            expected_bits,
                            "{:?} vertex {vertex}",
                            actual.semantic()
                        );
                    }
                    (actual_element, expected_element) => {
                        assert_eq!(
                            actual_element,
                            expected_element,
                            "{:?} vertex {vertex}",
                            actual.semantic()
                        );
                    }
                }
                assert!(actual.element(vertex).is_some());
            }
            assert_eq!(actual.element(actual.len()), None);
        }
    }
}

fn source_locator(source: SourceIdentityRef<'_>) -> Option<&str> {
    match source.locator() {
        SourceObjectLocatorRef::Namespaced { value, .. } => Some(value),
        _ => None,
    }
}

fn assert_source_ancestry(
    report: &ImportReport,
    owner: EntityMetadataOwner,
    prefixes: &[&str],
    resources: &[&str],
) -> TestResult<()> {
    let scene = report.scene();
    let mut pending = scene.owner_provenance(owner)?.collect::<Vec<_>>();
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
            sources.push(source);
        }
        pending.extend(node.parents());
    }

    let assert_reached = |source: SourceIdentityRef<'_>| {
        assert!(
            sources.contains(&source),
            "owner did not reach source {:?} from {}",
            source.locator(),
            source.resource().identity()
        );
        assert_eq!(source.generation(), scene.generation());
        assert_eq!(source.resource().generation(), scene.generation());
    };
    for prefix in prefixes {
        let source = scene
            .source_identities()
            .find(|source| source_locator(*source).is_some_and(|value| value.starts_with(prefix)))
            .unwrap_or_else(|| panic!("missing source locator {prefix}"));
        assert_eq!(source.format(), "org.baozi.gltf");
        assert_eq!(source.resource().identity(), SOURCE);
        assert_reached(source);
    }
    for resource in resources {
        let source = scene
            .source_identities()
            .find(|source| source.resource().identity() == *resource)
            .unwrap_or_else(|| panic!("missing source resource {resource}"));
        assert_reached(source);
    }
    Ok(())
}

fn assert_no_dependency_io(failure: &ImportFailure) {
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
    ] {
        assert_eq!(
            failure.ledger().dimension(dimension).unwrap().attempted(),
            0,
            "unexpected {dimension:?} before sparse admission"
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

fn assert_accessor_failure(failure: &ImportFailure, index: u64) -> TestResult<()> {
    assert_gltf_failure(failure, GltfFailureCode::InvalidAccessor, None)?;
    assert!(!gltf_failure_reason(failure)?.trim().is_empty());
    assert!(gltf_failure(failure)?.details().iter().any(|detail| {
        detail.key() == "index"
            && matches!(detail.value(), FormatFailureDetailValue::Unsigned(actual) if actual == index)
    }));
    Ok(())
}

#[test]
fn shared_zero_base_and_base_overlays_match_dense_geometry_for_all_index_encodings()
-> TestResult<()> {
    for component in [5121, 5123, 5125] {
        for with_base in [false, true] {
            let (mut sparse, buffers) = sparse_triangle(component, with_base)?;
            let (mut dense, mut binary) = dense_triangle()?;
            if with_base {
                preserve_first_vertex(&mut binary);
            }
            for document in [&mut sparse, &mut dense] {
                let shared = document["meshes"][0]["primitives"][0].clone();
                document["meshes"][0]["primitives"]
                    .as_array_mut()
                    .unwrap()
                    .push(shared);
            }
            let actual = import_sparse(&sparse, buffers)?;
            let expected = import_dense(&dense, binary)?;
            assert_geometry_equivalent(&actual, &expected);
            let scope = actual.source_scope_record().unwrap();
            assert_eq!(scope.counts(SourceScopeEntityKind::Accessor).selected(), 4);
            assert_eq!(scope.counts(SourceScopeEntityKind::Buffer).selected(), 3);
            assert_eq!(
                scope.counts(SourceScopeEntityKind::BufferView).selected(),
                if with_base { 6 } else { 5 }
            );
        }
    }
    Ok(())
}

#[test]
fn sparse_topology_indices_match_dense_typed_indices() -> TestResult<()> {
    let (mut document, [base, indices, mut values]) = sparse_triangle(5125, false)?;
    let offset = values.len();
    values.extend(encoded_indices(&[1, 2], 5123));
    document["buffers"][2]["byteLength"] = json!(values.len());
    document["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer": 2, "byteOffset": offset, "byteLength": 4
    }));
    document["accessors"][3] = json!({
        "componentType": 5123, "count": 3, "type": "SCALAR",
        "sparse": {
            "count": 2,
            "indices": {"bufferView": 4, "byteOffset": 4, "componentType": 5125},
            "values": {"bufferView": 6}
        }
    });
    let actual = import_sparse(&document, [base, indices, values])?;
    let (dense, binary) = dense_triangle()?;
    let expected = import_dense(&dense, binary)?;
    assert_geometry_equivalent(&actual, &expected);
    assert_eq!(
        actual
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .indices(),
        Some([0, 1, 2].as_slice())
    );
    Ok(())
}

#[test]
fn normalized_sparse_texcoords_match_dense_integer_attributes_exactly() -> TestResult<()> {
    for (component, maximum) in [(5121, 255_u32), (5123, 65535_u32)] {
        let (mut sparse, [mut base, indices, mut values]) = sparse_triangle(5123, true)?;
        let (mut dense, mut binary) = dense_triangle()?;
        preserve_first_vertex(&mut binary);
        let uv = [[maximum, maximum / 4], [maximum / 2, 0], [0, maximum]];
        base[72..96].fill(0);
        let first = encoded_indices(&uv[0], component);
        base[72..72 + first.len()].copy_from_slice(&first);
        binary[72..96].fill(0);
        for (vertex, row) in uv.iter().enumerate() {
            let encoded = encoded_indices(row, component);
            let offset = 72 + vertex * 4;
            binary[offset..offset + encoded.len()].copy_from_slice(&encoded);
        }
        dense["bufferViews"][2]["byteLength"] = json!(12);
        dense["bufferViews"][2]["byteStride"] = json!(4);
        dense["accessors"][2]["componentType"] = json!(component);
        dense["accessors"][2]["normalized"] = json!(true);
        let offset = values.len();
        for row in &uv[1..] {
            values.extend(encoded_indices(row, component));
        }
        sparse["buffers"][2]["byteLength"] = json!(values.len());
        sparse["bufferViews"][2]["byteLength"] = json!(12);
        sparse["bufferViews"][2]["byteStride"] = json!(4);
        sparse["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 2, "byteOffset": offset, "byteLength": values.len() - offset
        }));
        sparse["accessors"][2] = json!({
            "bufferView": 2, "componentType": component, "normalized": true,
            "count": 3, "type": "VEC2",
            "sparse": {
                "count": 2,
                "indices": {"bufferView": 4, "byteOffset": 4, "componentType": 5123},
                "values": {"bufferView": 6}
            }
        });
        let actual = import_sparse(&sparse, [base, indices, values])?;
        let expected = import_dense(&dense, binary)?;
        assert_geometry_equivalent(&actual, &expected);
        let primitive = actual
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        let texcoords = primitive.texcoords(0, AttributeDomain::Vertex).unwrap();
        assert_eq!(texcoords.len(), 3);
        for (vertex, raw) in uv.into_iter().enumerate() {
            let Some(AttributeElement::F32(actual)) = texcoords.element(vertex) else {
                panic!("missing normalized UV value");
            };
            for (&actual, raw) in actual.iter().zip(raw) {
                // Compare with an independently evaluated rational value. The existing
                // normalized conversion may differ by one ULP through F32 multiplication.
                let expected = (f64::from(raw) / f64::from(maximum)) as f32;
                assert!(actual.to_bits().abs_diff(expected.to_bits()) <= 1);
            }
        }
    }
    Ok(())
}

#[test]
fn zero_base_normalized_rgb_colors_keep_opaque_alpha_for_implicit_vertices() -> TestResult<()> {
    let (mut sparse, [base, indices, mut values]) = sparse_triangle(5121, false)?;
    let (mut dense, mut binary) = dense_triangle()?;
    let offset = values.len();
    values.extend_from_slice(&[128, 64, 255, 0, 255, 128]);
    sparse["buffers"][2]["byteLength"] = json!(values.len());
    sparse["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer": 2, "byteOffset": offset, "byteLength": 6
    }));
    sparse["accessors"].as_array_mut().unwrap().push(json!({
        "componentType": 5121, "normalized": true, "count": 3, "type": "VEC3",
        "sparse": {
            "count": 2,
            "indices": {"bufferView": 4, "byteOffset": 4, "componentType": 5121},
            "values": {"bufferView": 6}
        }
    }));
    dense["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer": 0, "byteOffset": binary.len(), "byteLength": 12, "byteStride": 4, "target": 34962
    }));
    for rgba in [[0_u8, 0, 0, 255], [128, 64, 255, 255], [0, 255, 128, 255]] {
        binary.extend_from_slice(&rgba);
    }
    dense["buffers"][0]["byteLength"] = json!(binary.len());
    dense["accessors"].as_array_mut().unwrap().push(json!({
        "bufferView": 4, "componentType": 5121, "normalized": true, "count": 3, "type": "VEC4"
    }));
    for document in [&mut sparse, &mut dense] {
        document["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(4);
    }
    let actual = import_sparse(&sparse, [base, indices, values])?;
    let expected = import_dense(&dense, binary)?;
    // The independent dense VEC4 fixture includes [0, 0, 0, 1] for the implicit RGB row.
    assert_geometry_equivalent(&actual, &expected);
    Ok(())
}

#[test]
fn sparse_position_normal_and_tangent_morphs_match_dense_typed_deltas() -> TestResult<()> {
    for with_base in [false, true] {
        let (mut sparse, [mut base, indices, values]) = sparse_triangle(5121, with_base)?;
        let (mut dense, mut binary) = dense_triangle()?;
        if with_base {
            preserve_first_vertex(&mut binary);
        }
        for buffer in [&mut base, &mut binary] {
            for _ in 0..3 {
                for value in [1.0_f32, 0.0, 0.0, 1.0] {
                    buffer.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        for document in [&mut sparse, &mut dense] {
            document["buffers"][0]["byteLength"] = json!(binary.len());
            // Multiple vertex accessors share this VEC3 base view.
            document["bufferViews"][0]["byteStride"] = json!(12);
            let tangent_view = document["bufferViews"].as_array().unwrap().len();
            document["bufferViews"].as_array_mut().unwrap().push(json!({
                "buffer": 0, "byteOffset": 104, "byteLength": 48, "target": 34962
            }));
            document["accessors"].as_array_mut().unwrap().push(json!({
                "bufferView": tangent_view, "componentType": 5126, "count": 3, "type": "VEC4"
            }));
            for _ in 0..3 {
                let delta = document["accessors"][0].clone();
                document["accessors"].as_array_mut().unwrap().push(delta);
            }
            document["meshes"][0]["weights"] = json!([0.25]);
            document["meshes"][0]["primitives"][0]["attributes"]["TANGENT"] = json!(4);
            document["meshes"][0]["primitives"][0]["targets"] =
                json!([{"POSITION": 5, "NORMAL": 6, "TANGENT": 7}]);
        }
        let actual = import_sparse(&sparse, [base, indices, values])?;
        let expected = import_dense(&dense, binary)?;
        assert_geometry_equivalent(&actual, &expected);
        let actual_mesh = actual.scene().meshes().next().unwrap();
        let expected_mesh = expected.scene().meshes().next().unwrap();
        assert_eq!(actual_mesh.morph_target_count(), 1);
        assert_eq!(actual_mesh.default_morph_weights(), &[0.25]);
        let actual_target = actual_mesh
            .primitives()
            .next()
            .unwrap()
            .morph_targets()
            .next()
            .unwrap();
        let expected_target = expected_mesh
            .primitives()
            .next()
            .unwrap()
            .morph_targets()
            .next()
            .unwrap();
        assert_eq!(actual_target.delta_count(), 3);
        for semantic in [
            MorphDeltaSemantic::Position,
            MorphDeltaSemantic::Normal,
            MorphDeltaSemantic::Tangent,
        ] {
            let actual_delta = actual_target.delta(semantic).unwrap();
            let expected_delta = expected_target.delta(semantic).unwrap();
            assert_eq!(actual_delta.len(), 3);
            for vertex in 0..3 {
                assert_eq!(
                    actual_delta.data().value(vertex),
                    expected_delta.data().value(vertex)
                );
            }
            assert_eq!(
                actual_delta.data().value(0),
                Some(if with_base {
                    Vec3::new(0.25, 0.25, 0.0)
                } else {
                    Vec3::new(0.0, 0.0, 0.0)
                })
            );
            assert_eq!(actual_delta.data().value(1), Some(Vec3::new(1.0, 0.0, 0.0)));
            assert_eq!(actual_delta.data().value(2), Some(Vec3::new(0.0, 1.0, 0.0)));
            assert_eq!(actual_delta.data().value(3), None);
        }
    }
    Ok(())
}

#[test]
fn sparse_animation_inputs_and_outputs_match_dense_typed_keys() -> TestResult<()> {
    for with_base in [false, true] {
        let (mut sparse, [mut base, indices, mut values]) = sparse_triangle(5123, with_base)?;
        let (mut dense, mut binary) = dense_triangle()?;
        if with_base {
            preserve_first_vertex(&mut binary);
        }
        let offset = values.len();
        for time in [1.0_f32, 2.0] {
            values.extend_from_slice(&time.to_le_bytes());
        }
        sparse["buffers"][2]["byteLength"] = json!(values.len());
        sparse["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 2, "byteOffset": offset, "byteLength": 8
        }));
        let mut input = json!({
            "componentType": 5126, "count": 3, "type": "SCALAR", "min": [0], "max": [2],
            "sparse": {
                "count": 2,
                "indices": {"bufferView": 4, "byteOffset": 4, "componentType": 5123},
                "values": {"bufferView": 6}
            }
        });
        if with_base {
            sparse["bufferViews"].as_array_mut().unwrap().push(json!({
                "buffer": 0, "byteOffset": base.len(), "byteLength": 12
            }));
            input["bufferView"] = json!(7);
            for time in [0.0_f32, 9.0, 9.0] {
                base.extend_from_slice(&time.to_le_bytes());
            }
            sparse["buffers"][0]["byteLength"] = json!(base.len());
        }
        sparse["accessors"].as_array_mut().unwrap().push(input);
        let output = sparse["accessors"][0].clone();
        sparse["accessors"].as_array_mut().unwrap().push(output);
        dense["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 0, "byteOffset": binary.len(), "byteLength": 12
        }));
        for time in [0.0_f32, 1.0, 2.0] {
            binary.extend_from_slice(&time.to_le_bytes());
        }
        dense["buffers"][0]["byteLength"] = json!(binary.len());
        dense["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView": 4, "componentType": 5126, "count": 3, "type": "SCALAR",
            "min": [0], "max": [2]
        }));
        let output = dense["accessors"][0].clone();
        dense["accessors"].as_array_mut().unwrap().push(output);
        for document in [&mut sparse, &mut dense] {
            document["animations"] = json!([{
                "name": "Sparse translation",
                "samplers": [{"input": 4, "output": 5, "interpolation": "LINEAR"}],
                "channels": [{"sampler": 0, "target": {"node": 1, "path": "translation"}}]
            }]);
        }
        let actual = import_sparse(&sparse, [base, indices, values])?;
        let expected = import_dense(&dense, binary)?;
        let actual_clip = actual.scene().animation_clips().next().unwrap();
        let expected_clip = expected.scene().animation_clips().next().unwrap();
        assert_eq!(actual_clip.duration(), 2.0);
        assert_eq!(actual_clip.channel_count(), 1);
        let actual_channel = actual_clip.channel(0).unwrap();
        let expected_channel = expected_clip.channel(0).unwrap();
        assert_eq!(actual_channel.key_count(), 3);
        assert_eq!(actual_channel.value_width(), 3);
        assert_eq!(actual_channel.key_times(), &[0.0, 1.0, 2.0]);
        assert_eq!(actual_channel.key_times(), expected_channel.key_times());
        assert_eq!(actual_channel.values(), expected_channel.values());
        assert_eq!(
            actual_channel.interpolation(),
            expected_channel.interpolation()
        );
        assert_eq!(actual_channel.in_tangents(), None);
        assert_eq!(actual_channel.out_tangents(), None);
        let target = actual
            .scene()
            .nodes()
            .find(|node| node.name() == Some("TriangleNode"))
            .unwrap();
        assert_eq!(
            actual_channel.target(),
            AnimationTarget::NodeTranslation(target.id())
        );
    }
    Ok(())
}

#[test]
fn sparse_joint_weight_and_inverse_bind_accessors_match_dense_skin() -> TestResult<()> {
    let mut dense: Value = serde_json::from_slice(&skinned_triangle_gltf())?;
    dense["buffers"][0]["uri"] = json!("base.bin");
    let binary = skinned_triangle_bin();
    let mut sparse = dense.clone();
    let mut base = binary.clone();
    let indices = vec![1, 2, 1];
    let mut values = binary[50..66].to_vec();
    values.extend_from_slice(&binary[84..116]);
    values.extend_from_slice(&binary[180..244]);
    base[42..66].fill(0);
    base[84..116].fill(0);
    base[180..244].fill(0);
    sparse["buffers"].as_array_mut().unwrap().extend([
        json!({"uri": "sparse-indices.bin", "byteLength": indices.len()}),
        json!({"uri": "sparse-values.bin", "byteLength": values.len()}),
    ]);
    sparse["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 1, "byteOffset": 0, "byteLength": 2}),
        json!({"buffer": 2, "byteOffset": 0, "byteLength": 16}),
        json!({"buffer": 2, "byteOffset": 16, "byteLength": 32}),
        json!({"buffer": 1, "byteOffset": 2, "byteLength": 1}),
        json!({"buffer": 2, "byteOffset": 48, "byteLength": 64}),
    ]);
    sparse["accessors"][2]
        .as_object_mut()
        .unwrap()
        .remove("bufferView");
    for (accessor, index_view, value_view, count) in [(2, 5, 6, 2), (3, 5, 7, 2), (4, 8, 9, 1)] {
        sparse["accessors"][accessor]["sparse"] = json!({
            "count": count,
            "indices": {"bufferView": index_view, "componentType": 5121},
            "values": {"bufferView": value_view}
        });
    }
    let actual = import_sparse(&sparse, [base, indices, values])?;
    let expected = import_dense(&dense, binary)?;
    assert_geometry_equivalent(&actual, &expected);
    let actual_skin = actual.scene().skins().next().unwrap();
    let expected_skin = expected.scene().skins().next().unwrap();
    assert_eq!(actual_skin.joint_count(), 2);
    assert_eq!(
        actual_skin.inverse_bind_matrices().collect::<Vec<_>>(),
        vec![Mat4::IDENTITY; 2]
    );
    assert_eq!(
        actual_skin.inverse_bind_matrices().collect::<Vec<_>>(),
        expected_skin.inverse_bind_matrices().collect::<Vec<_>>()
    );
    let actual_rows = actual
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .influences()
        .unwrap();
    let expected_rows = expected
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .influences()
        .unwrap();
    assert_eq!(actual_rows.len(), 3);
    for vertex in 0..3 {
        let actual = actual_rows.get(vertex).unwrap();
        let expected = expected_rows.get(vertex).unwrap();
        assert_eq!(actual.joint_indices(), expected.joint_indices());
        assert_eq!(actual.weights(), expected.weights());
    }
    assert_eq!(actual_rows.get(2).unwrap().joint_indices(), &[0, 1, 0, 0]);
    assert_eq!(actual_rows.get(2).unwrap().weights(), &[0.5, 0.5, 0.0, 0.0]);
    Ok(())
}

#[test]
fn shared_sparse_attribute_owners_reach_all_three_external_resources_and_declarations()
-> TestResult<()> {
    let (mut document, buffers) = sparse_triangle(5125, true)?;
    let shared = document["meshes"][0]["primitives"][0].clone();
    document["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(shared);
    let report = import_sparse(&document, buffers)?;
    let mut resources = report
        .successful_input_manifest()
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    resources.sort_unstable();
    let mut expected_resources = [SOURCE, BASE, INDICES, VALUES];
    expected_resources.sort_unstable();
    assert_eq!(resources, expected_resources);
    let mesh = report.scene().meshes().next().unwrap();
    for (index, primitive) in mesh.primitives().enumerate() {
        let primitive_prefix = format!("kind=primitive;parent=0;index={index};");
        let prefixes = [
            primitive_prefix.as_str(),
            "kind=accessor;parent=-;index=0;",
            "kind=buffer-view;parent=-;index=0;",
            "kind=buffer-view;parent=-;index=4;",
            "kind=buffer-view;parent=-;index=5;",
            "kind=buffer;parent=-;index=0;",
            "kind=buffer;parent=-;index=1;",
            "kind=buffer;parent=-;index=2;",
        ];
        for owner in [
            primitive.metadata_owner(),
            primitive.positions().unwrap().metadata_owner(),
        ] {
            assert_source_ancestry(&report, owner, &prefixes, &[SOURCE, BASE, INDICES, VALUES])?;
        }
    }
    Ok(())
}

#[test]
fn duplicate_decreasing_and_out_of_bounds_sparse_indices_are_content_failures() -> TestResult<()> {
    for component in [5121, 5123, 5125] {
        for bad_indices in [[1, 1], [2, 1], [1, 3]] {
            let (document, [base, mut indices, values]) = sparse_triangle(component, true)?;
            indices[8..].copy_from_slice(&encoded_indices(&bad_indices, component));
            let (result, diagnostics) = import_assets_result(
                SOURCE,
                [
                    (SOURCE, serde_json::to_vec(&document)?),
                    (BASE, base),
                    (INDICES, indices),
                    (VALUES, values),
                ],
                sidecar_options(),
            )?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_accessor_failure(&failure, 0)?;
            assert_eq!(failure.ledger().current_live_bytes(), 0);
            assert_eq!(failure.ledger().active_reservations(), 0);
        }
    }
    Ok(())
}

#[test]
fn zero_or_excessive_sparse_count_fails_before_dependency_io() -> TestResult<()> {
    for count in [0, 4] {
        let (mut document, _) = sparse_triangle(5123, true)?;
        document["accessors"][0]["sparse"]["count"] = json!(count);
        // Nonempty views cover even count=4, isolating the invalid sparse count.
        document["buffers"][1]["byteLength"] = json!(16);
        document["bufferViews"][4]["byteLength"] = json!(12);
        document["buffers"][2]["byteLength"] = json!(56);
        document["bufferViews"][5]["byteLength"] = json!(52);
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_accessor_failure(&failure, 0)?;
        assert_no_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn truncated_and_misaligned_sparse_ranges_fail_before_dependency_io() -> TestResult<()> {
    for defect in [
        "short index view",
        "short value view",
        "short index buffer",
        "short value buffer",
        "misaligned index offset",
        "misaligned value offset",
        "misaligned index view",
        "misaligned value view",
        "index offsets cancel",
        "value offsets cancel",
    ] {
        let (mut document, _) = sparse_triangle(5123, true)?;
        match defect {
            "short index view" => document["bufferViews"][4]["byteLength"] = json!(7),
            "short value view" => document["bufferViews"][5]["byteLength"] = json!(27),
            "short index buffer" => document["buffers"][1]["byteLength"] = json!(11),
            "short value buffer" => document["buffers"][2]["byteLength"] = json!(31),
            "misaligned index offset" => {
                document["accessors"][0]["sparse"]["indices"]["byteOffset"] = json!(3)
            }
            "misaligned value offset" => {
                document["accessors"][0]["sparse"]["values"]["byteOffset"] = json!(3)
            }
            "misaligned index view" => document["bufferViews"][4]["byteOffset"] = json!(3),
            "misaligned value view" => document["bufferViews"][5]["byteOffset"] = json!(3),
            "index offsets cancel" => {
                document["bufferViews"][4]["byteOffset"] = json!(3);
                document["bufferViews"][4]["byteLength"] = json!(9);
                document["accessors"][0]["sparse"]["indices"]["byteOffset"] = json!(5);
            }
            "value offsets cancel" => {
                document["bufferViews"][5]["byteOffset"] = json!(3);
                document["bufferViews"][5]["byteLength"] = json!(29);
                document["accessors"][0]["sparse"]["values"]["byteOffset"] = json!(5);
            }
            _ => unreachable!("all sparse range defects are listed above"),
        }
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty(), "{defect}: {diagnostics:?}");
        let failure = expected_error(result)?;
        assert_accessor_failure(&failure, 0)?;
        assert_no_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn byte_stride_on_either_sparse_view_fails_before_dependency_io() -> TestResult<()> {
    for (view, stride) in [(4, 4), (5, 12)] {
        let (mut document, _) = sparse_triangle(5123, true)?;
        document["bufferViews"][view]["byteStride"] = json!(stride);
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_accessor_failure(&failure, 0)?;
        assert_no_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn targets_on_either_sparse_view_fail_before_dependency_io() -> TestResult<()> {
    for view in [4, 5] {
        for target in [34962, 34963] {
            let (mut document, _) = sparse_triangle(5123, true)?;
            document["bufferViews"][view]["target"] = json!(target);
            let (result, diagnostics) = import_assets_result(
                SOURCE,
                [(SOURCE, serde_json::to_vec(&document)?)],
                sidecar_options(),
            )?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_accessor_failure(&failure, 0)?;
            assert_no_dependency_io(&failure);
        }
    }
    Ok(())
}

#[test]
fn zero_base_accessors_must_omit_byte_offset_even_when_zero() -> TestResult<()> {
    for offset in [0, 4] {
        let (mut document, _) = sparse_triangle(5123, false)?;
        document["accessors"][0]["byteOffset"] = json!(offset);
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_accessor_failure(&failure, 0)?;
        assert_no_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn sparse_logical_vertex_and_morph_budgets_are_admitted_before_dependency_io() -> TestResult<()> {
    for morph in [false, true] {
        let (mut document, _) = sparse_triangle(5125, false)?;
        let mut options = sidecar_options();
        let (dimension, limit) = if morph {
            document["meshes"][0]["weights"] = json!([0.25]);
            document["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION": 0}]);
            // Two definition/default cells, one target cell and nine delta scalars.
            options.limits.max_morph_cells = 11;
            (ImportDimension::MorphCells, "max_morph_cells")
        } else {
            options.limits.max_vertices = 2;
            (ImportDimension::Vertices, "max_vertices")
        };
        let (result, diagnostics) =
            import_assets_result(SOURCE, [(SOURCE, serde_json::to_vec(&document)?)], options)?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_resource_limit(&failure, limit);
        assert_no_dependency_io(&failure);
        let cells = failure.ledger().dimension(dimension).unwrap();
        // Format-local preflight rejects before the materialization ledger action begins.
        assert_eq!(cells.attempted(), 0);
        assert_eq!(cells.consumed(), 0);
    }
    Ok(())
}

#[test]
fn sparse_index_scan_decode_work_is_admitted_once_for_a_shared_accessor() -> TestResult<()> {
    let (mut document, buffers) = sparse_triangle(5125, false)?;
    let shared = document["meshes"][0]["primitives"][0].clone();
    document["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(shared);
    let primary = serde_json::to_vec(&document)?;
    // Syntax/source admission consumes JSON once; typed field decoding uses WorkUnits.
    let primary_decode = u64::try_from(primary.len())?;
    let [base, indices, values] = buffers;
    let execute = |max_decode_work| {
        let mut options = sidecar_options();
        options.limits.max_decode_work = max_decode_work;
        import_assets_result(
            SOURCE,
            [
                (SOURCE, primary.clone()),
                (BASE, base.clone()),
                (INDICES, indices.clone()),
                (VALUES, values.clone()),
            ],
            options,
        )
    };

    let (result, diagnostics) = execute(primary_decode + 1)?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert_resource_limit(&failure, "max_decode_work");
    let decode = failure
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .unwrap();
    assert_eq!(decode.consumed(), primary_decode);
    assert_eq!(decode.attempted(), primary_decode + 2);
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        4
    );
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::Vertices)
            .unwrap()
            .attempted(),
        0
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);

    let (result, diagnostics) = execute(primary_decode + 2)?;
    assert!(diagnostics.is_empty());
    let actual = result?;
    let decode = actual
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .unwrap();
    assert_eq!(decode.consumed(), primary_decode + 2);
    assert_eq!(decode.attempted(), primary_decode + 2);
    let (mut dense, binary) = dense_triangle()?;
    let shared = dense["meshes"][0]["primitives"][0].clone();
    dense["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(shared);
    assert_geometry_equivalent(&actual, &import_dense(&dense, binary)?);
    Ok(())
}

#[test]
fn dense_and_sparse_topology_index_accessors_must_not_be_normalized() -> TestResult<()> {
    for component in [5123, 5125] {
        for sparse in [false, true] {
            let (mut document, _) = dense_triangle()?;
            let element_bytes = if component == 5123 { 2 } else { 4 };
            if sparse {
                document["buffers"].as_array_mut().unwrap().extend([
                    json!({"uri": "sparse-indices.bin", "byteLength": 2}),
                    json!({"uri": "sparse-values.bin", "byteLength": 2 * element_bytes}),
                ]);
                document["bufferViews"].as_array_mut().unwrap().extend([
                    json!({"buffer": 1, "byteLength": 2}),
                    json!({"buffer": 2, "byteLength": 2 * element_bytes}),
                ]);
                document["accessors"][3] = json!({
                    "componentType": component, "normalized": true, "count": 3, "type": "SCALAR",
                    "sparse": {
                        "count": 2,
                        "indices": {"bufferView": 4, "componentType": 5121},
                        "values": {"bufferView": 5}
                    }
                });
            } else {
                document["accessors"][3]["componentType"] = json!(component);
                document["accessors"][3]["normalized"] = json!(true);
                document["bufferViews"][3]["byteLength"] = json!(3 * element_bytes);
                document["buffers"][0]["byteLength"] = json!(96 + 3 * element_bytes);
            }
            let (result, diagnostics) = import_assets_result(
                SOURCE,
                [(SOURCE, serde_json::to_vec(&document)?)],
                sidecar_options(),
            )?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_accessor_failure(&failure, 3)?;
            assert_no_dependency_io(&failure);
        }
    }
    Ok(())
}

#[test]
fn dense_and_sparse_joint_accessors_must_not_be_normalized() -> TestResult<()> {
    for sparse in [false, true] {
        let mut document: Value = serde_json::from_slice(&skinned_triangle_gltf())?;
        document["accessors"][2]["normalized"] = json!(true);
        if sparse {
            document["accessors"][2]
                .as_object_mut()
                .unwrap()
                .remove("bufferView");
            document["buffers"].as_array_mut().unwrap().extend([
                json!({"uri": "sparse-indices.bin", "byteLength": 2}),
                json!({"uri": "sparse-values.bin", "byteLength": 16}),
            ]);
            document["bufferViews"].as_array_mut().unwrap().extend([
                json!({"buffer": 1, "byteLength": 2}),
                json!({"buffer": 2, "byteLength": 16}),
            ]);
            document["accessors"][2]["sparse"] = json!({
                "count": 2,
                "indices": {"bufferView": 5, "componentType": 5121},
                "values": {"bufferView": 6}
            });
        }
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_accessor_failure(&failure, 2)?;
        assert_no_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn rgb_base_stride_requires_four_byte_alignment_while_sparse_values_stay_packed() -> TestResult<()>
{
    let colors = [[64_u8, 128, 255], [255, 0, 64], [0, 128, 255]];
    let (mut sparse, [mut base, indices, mut values]) = sparse_triangle(5121, false)?;
    let (mut dense, mut binary) = dense_triangle()?;
    base.extend_from_slice(&colors[0]);
    base.resize(116, 0);
    for row in colors {
        binary.extend_from_slice(&row);
        binary.push(0);
    }
    let offset = values.len();
    values.extend_from_slice(&colors[1]);
    values.extend_from_slice(&colors[2]);
    sparse["buffers"][0]["byteLength"] = json!(base.len());
    sparse["buffers"][2]["byteLength"] = json!(values.len());
    sparse["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 0, "byteOffset": 104, "byteLength": 12, "byteStride": 4, "target": 34962}),
        json!({"buffer": 2, "byteOffset": offset, "byteLength": 6}),
    ]);
    sparse["accessors"].as_array_mut().unwrap().push(json!({
        "bufferView": 6, "componentType": 5121, "normalized": true, "count": 3, "type": "VEC3",
        "sparse": {
            "count": 2,
            "indices": {"bufferView": 4, "byteOffset": 4, "componentType": 5121},
            "values": {"bufferView": 7}
        }
    }));
    dense["buffers"][0]["byteLength"] = json!(binary.len());
    dense["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer": 0, "byteOffset": 104, "byteLength": 12, "byteStride": 4, "target": 34962
    }));
    dense["accessors"].as_array_mut().unwrap().push(json!({
        "bufferView": 4, "componentType": 5121, "normalized": true, "count": 3, "type": "VEC3"
    }));
    for document in [&mut sparse, &mut dense] {
        document["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(4);
    }
    let actual = import_sparse(&sparse, [base, indices, values])?;
    let expected = import_dense(&dense, binary)?;
    assert_geometry_equivalent(&actual, &expected);

    for (mut document, view) in [(dense, 4), (sparse, 6)] {
        document["bufferViews"][view]["byteStride"] = json!(5);
        // Cover the larger strided range so alignment is the only invalid contract.
        document["bufferViews"][view]["byteLength"] = json!(13);
        document["buffers"][0]["byteLength"] = json!(117);
        let (result, diagnostics) = import_assets_result(
            SOURCE,
            [(SOURCE, serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_accessor_failure(&failure, 4)?;
        assert_no_dependency_io(&failure);
    }
    Ok(())
}

#[test]
fn sparse_u8_selector_255_is_valid_for_256_logical_vertices() -> TestResult<()> {
    let (mut sparse, [base, mut indices, mut values]) = sparse_triangle(5121, false)?;
    let (mut dense, _) = dense_triangle()?;
    indices.truncate(9);
    indices[8] = 255;
    values.truncate(20);
    sparse["buffers"][1]["byteLength"] = json!(indices.len());
    sparse["buffers"][2]["byteLength"] = json!(values.len());
    sparse["bufferViews"][4]["byteLength"] = json!(5);
    sparse["bufferViews"][5]["byteLength"] = json!(16);
    sparse["accessors"][0]["sparse"]["count"] = json!(1);
    let mut binary = vec![0; 256 * 12];
    binary[255 * 12..255 * 12 + 4].copy_from_slice(&1.0_f32.to_le_bytes());
    dense["buffers"][0]["byteLength"] = json!(binary.len());
    dense["bufferViews"][0]["byteLength"] = json!(binary.len());
    for document in [&mut sparse, &mut dense] {
        document["accessors"][0]["count"] = json!(256);
        document["accessors"][0]["max"] = json!([1, 0, 0]);
        let primitive = &mut document["meshes"][0]["primitives"][0];
        primitive["attributes"] = json!({"POSITION": 0});
        primitive["mode"] = json!(0);
        primitive.as_object_mut().unwrap().remove("indices");
    }
    // A sparse selector addresses an accessor element; it is not a primitive-restart token.
    let actual = import_sparse(&sparse, [base, indices, values])?;
    let expected = import_dense(&dense, binary)?;
    assert_geometry_equivalent(&actual, &expected);
    let primitive = actual
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(primitive.vertex_count(), 256);
    assert_eq!(primitive.indices(), None);
    Ok(())
}

#[test]
fn omitted_sparse_accessor_does_not_acquire_its_indices_or_values() -> TestResult<()> {
    let (mut document, _) = sparse_triangle(5123, true)?;
    let mut base = triangle_bin();
    preserve_first_vertex(&mut base);
    let omitted_mesh = document["meshes"][0].clone();
    let mut dense_accessor = document["accessors"][0].clone();
    dense_accessor.as_object_mut().unwrap().remove("sparse");
    document["accessors"]
        .as_array_mut()
        .unwrap()
        .push(dense_accessor);
    document["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(4);
    document["meshes"]
        .as_array_mut()
        .unwrap()
        .push(omitted_mesh);
    document["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"mesh": 1}));
    document["scenes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"nodes": [2]}));

    let (report, diagnostics) = import_assets(
        SOURCE,
        [(SOURCE, serde_json::to_vec(&document)?), (BASE, base)],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    let opens = report
        .ledger()
        .dimension(ImportDimension::AssetOpen)
        .unwrap();
    assert_eq!(opens.attempted(), 2);
    assert_eq!(opens.consumed(), 2);
    let mut resources = report
        .successful_input_manifest()
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    resources.sort_unstable();
    assert_eq!(resources, [BASE, SOURCE]);

    let scope = report.source_scope_record().unwrap();
    for (kind, selected, omitted) in [
        (SourceScopeEntityKind::Scene, 1, 1),
        (SourceScopeEntityKind::Mesh, 1, 1),
        (SourceScopeEntityKind::Primitive, 1, 1),
        (SourceScopeEntityKind::Accessor, 4, 1),
        (SourceScopeEntityKind::BufferView, 4, 2),
        (SourceScopeEntityKind::Buffer, 1, 2),
    ] {
        assert_eq!(scope.counts(kind).selected(), selected, "{kind:?}");
        assert_eq!(scope.counts(kind).omitted(), omitted, "{kind:?}");
    }
    for (kind, expected) in [
        (SourceScopeEntityKind::Accessor, vec!["acc0"]),
        (SourceScopeEntityKind::BufferView, vec!["bv4", "bv5"]),
        (SourceScopeEntityKind::Buffer, vec!["b1", "b2"]),
    ] {
        let omitted = scope
            .details()
            .filter(|detail| {
                detail.kind() == kind && detail.disposition() == SourceScopeDisposition::Omitted
            })
            .map(|detail| detail.identity())
            .collect::<Vec<_>>();
        assert_eq!(omitted, expected, "{kind:?}");
    }
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
    for (vertex, expected) in [[0.25_f32, 0.25, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            positions.element(vertex),
            Some(AttributeElement::F32(&expected))
        );
    }
    Ok(())
}
