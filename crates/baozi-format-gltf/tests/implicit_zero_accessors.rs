mod common;

use baozi_core::{AttributeElement, AttributeSemanticRef, TopologyKind};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::authoring::{MorphDeltaSemantic, Vec3};
use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportReport, ImportTerminalFailureClass,
    ScopeAbortCause,
};
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, assert_gltf_failure, assert_resource_limit,
    controlled_import, expected_error, glb_with_json, import_assets, import_assets_result,
    import_assets_result_with_control,
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

fn points(count: u64) -> Value {
    json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"mode": 0, "attributes": {"POSITION": 0}}]}],
        "accessors": [{
            "componentType": 5126, "count": count, "type": "VEC3",
            "min": [-0.0, 0.0, -0.0], "max": [0.0, -0.0, 0.0]
        }]
    })
}

fn containers(document: &Value) -> TestResult<[(&'static str, Vec<u8>); 2]> {
    let json = serde_json::to_vec(document)?;
    Ok([
        ("implicit-zero.gltf", json.clone()),
        ("implicit-zero.glb", glb_with_json(&json)),
    ])
}

fn assert_released(report: &ImportReport) {
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}

fn assert_failure_released(failure: &ImportFailure) {
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

fn assert_no_dependencies(report: &ImportReport) {
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
    ] {
        assert_eq!(report.ledger().dimension(dimension).unwrap().attempted(), 0);
    }
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        1
    );
}

#[test]
fn buffer_free_points_import_the_origin_with_positive_zero_bits_in_json_and_glb() -> TestResult<()>
{
    for (source, bytes) in containers(&points(1))? {
        let (report, diagnostics) =
            import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty());
        assert_eq!(report.scene().mesh_count(), 1);
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert_eq!(primitive.topology(), TopologyKind::Points);
        assert_eq!(
            (
                primitive.vertex_count(),
                primitive.corner_count(),
                primitive.face_count()
            ),
            (1, 1, 1)
        );
        assert_eq!(primitive.indices(), None);
        let positions = primitive.positions().unwrap();
        assert_eq!((positions.lanes(), positions.len()), (3, 1));
        let Some(AttributeElement::F32(value)) = positions.element(0) else {
            panic!("POSITION must retain F32 storage")
        };
        assert_eq!(
            value
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            [0, 0, 0]
        );
        assert_eq!(positions.element(1), None);
        assert_no_dependencies(&report);
        assert_released(&report);
    }
    Ok(())
}

#[test]
fn unused_zero_accessor_is_the_valid_control_for_a_missing_view_reference() -> TestResult<()> {
    let mut document = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{}],
        "accessors": [{"componentType": 5126, "count": 1, "type": "VEC3"}]
    });
    for (source, bytes) in containers(&document)? {
        let (report, diagnostics) =
            import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty());
        assert_eq!(report.scene().mesh_count(), 0);
        assert_no_dependencies(&report);
        assert_released(&report);
    }
    document["accessors"][0]["bufferView"] = json!(0);
    for (source, bytes) in containers(&document)? {
        let (result, diagnostics) =
            import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::InvalidDocument,
            Some("core reference is out of range"),
        )?;
        assert_failure_released(&failure);
    }
    Ok(())
}

#[test]
fn zero_uv_colors_indices_and_morph_deltas_keep_their_typed_values() -> TestResult<()> {
    for (component, color_type, index_component, alpha) in [
        (5126, "VEC3", 5121, 1.0_f32),
        (5126, "VEC4", 5123, 0.0),
        (5121, "VEC3", 5125, 1.0),
        (5121, "VEC4", 5121, 0.0),
        (5123, "VEC3", 5123, 1.0),
        (5123, "VEC4", 5125, 0.0),
    ] {
        let mut document = points(3);
        document["accessors"].as_array_mut().unwrap().extend([
            json!({"componentType": component, "normalized": component != 5126,
                   "count": 3, "type": "VEC2"}),
            json!({"componentType": component, "normalized": component != 5126,
                   "count": 3, "type": color_type}),
            json!({"componentType": index_component, "count": 3, "type": "SCALAR"}),
        ]);
        let primitive = &mut document["meshes"][0]["primitives"][0];
        primitive["attributes"]["TEXCOORD_0"] = json!(1);
        primitive["attributes"]["COLOR_0"] = json!(2);
        primitive["indices"] = json!(3);
        primitive["targets"] = json!([{"POSITION": 0}]);
        for (source, bytes) in containers(&document)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let primitive = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap();
            assert_eq!(primitive.vertex_count(), 3);
            assert_eq!(primitive.indices(), Some([0, 0, 0].as_slice()));
            for (semantic, expected_bits) in [
                (AttributeSemanticRef::Texcoord(0), vec![0, 0]),
                (
                    AttributeSemanticRef::Color(0),
                    vec![0, 0, 0, alpha.to_bits()],
                ),
            ] {
                let attribute = primitive
                    .attributes()
                    .find(|a| a.semantic() == semantic)
                    .unwrap();
                assert_eq!(attribute.len(), 3);
                for vertex in 0..3 {
                    let Some(AttributeElement::F32(value)) = attribute.element(vertex) else {
                        panic!("zero vertex attribute must retain F32 storage")
                    };
                    assert_eq!(
                        value
                            .iter()
                            .map(|value| value.to_bits())
                            .collect::<Vec<_>>(),
                        expected_bits
                    );
                }
                assert_eq!(attribute.element(3), None);
            }
            let target = primitive.morph_targets().next().unwrap();
            assert_eq!(target.delta_count(), 1);
            let values = target.delta(MorphDeltaSemantic::Position).unwrap().data();
            for vertex in 0..3 {
                assert_eq!(values.value(vertex), Some(Vec3::new(0.0, 0.0, 0.0)));
            }
            assert_eq!(values.value(3), None);
            assert_no_dependencies(&report);
            assert_released(&report);
        }
    }
    Ok(())
}

#[test]
fn authored_byte_offset_is_invalid_without_a_view_even_for_unused_or_sparse_accessors()
-> TestResult<()> {
    for selected in [false, true] {
        for sparse in [false, true] {
            for offset in [0, 4] {
                let mut document = points(1);
                if !selected {
                    document["scenes"][0] = json!({});
                }
                document["accessors"][0]["byteOffset"] = json!(offset);
                if sparse {
                    document["buffers"] = json!([{"uri": "missing.bin", "byteLength": 16}]);
                    document["bufferViews"] = json!([
                        {"buffer": 0, "byteLength": 1},
                        {"buffer": 0, "byteOffset": 4, "byteLength": 12}
                    ]);
                    document["accessors"][0]["sparse"] = json!({
                        "count": 1, "indices": {"bufferView": 0, "componentType": 5121},
                        "values": {"bufferView": 1}
                    });
                }
                for (source, bytes) in containers(&document)? {
                    let (result, diagnostics) =
                        import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
                    assert!(diagnostics.is_empty());
                    let failure = expected_error(result)?;
                    assert_gltf_failure(
                        &failure,
                        GltfFailureCode::InvalidAccessor,
                        Some("without bufferView must not define byteOffset"),
                    )?;
                    assert_eq!(
                        failure
                            .ledger()
                            .dimension(ImportDimension::DependencyReference)
                            .unwrap()
                            .attempted(),
                        0
                    );
                    assert_failure_released(&failure);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn zero_initialization_does_not_bypass_vertex_type_or_normalization_contracts() -> TestResult<()> {
    for (semantic, component, dimension, normalized, reason, code) in [
        (
            "TEXCOORD_0",
            5126,
            "VEC3",
            false,
            "TEXCOORD accessor type",
            GltfFailureCode::InvalidAccessor,
        ),
        (
            "COLOR_0",
            5120,
            "VEC3",
            true,
            "COLOR accessor type",
            GltfFailureCode::InvalidAccessor,
        ),
        (
            "TEXCOORD_0",
            5123,
            "VEC2",
            false,
            "integer TEXCOORD accessor",
            GltfFailureCode::InvalidDocument,
        ),
    ] {
        let mut document = points(1);
        document["accessors"].as_array_mut().unwrap().push(json!({
            "componentType": component, "count": 1, "type": dimension, "normalized": normalized
        }));
        document["meshes"][0]["primitives"][0]["attributes"][semantic] = json!(1);
        for (source, bytes) in containers(&document)? {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_gltf_failure(&failure, code, Some(reason))?;
            assert_failure_released(&failure);
        }
    }
    Ok(())
}

#[test]
fn implicit_descriptor_bounds_may_differ_from_zero_values_but_stored_morph_bounds_must_match()
-> TestResult<()> {
    let mut document = points(1);
    document["accessors"][0]["min"] = json!([-10, -20, -30]);
    document["accessors"][0]["max"] = json!([10, 20, 30]);
    document["accessors"].as_array_mut().unwrap().push(json!({
        "componentType": 5126, "count": 1, "type": "VEC3",
        "min": [0, 0, 0], "max": [0, 0, 1]
    }));
    document["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION": 1}]);
    for (source, bytes) in containers(&document)? {
        let (report, diagnostics) =
            import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty());
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        let position = primitive.positions().unwrap().element(0).unwrap();
        assert!(matches!(position, AttributeElement::F32(values) if values == [0.0, 0.0, 0.0]));
        let delta = primitive
            .morph_targets()
            .next()
            .unwrap()
            .delta(MorphDeltaSemantic::Position)
            .unwrap();
        assert_eq!(delta.data().value(0), Some(Vec3::ZERO));
        assert_no_dependencies(&report);
        assert_released(&report);
    }
    // Supplying actual dense or sparse bytes removes the descriptor-only bounds exception.
    document["buffers"] = json!([{
        "byteLength": 16, "uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAAAAAA=="
    }]);
    document["bufferViews"] = json!([
        {"buffer": 0, "byteLength": 1},
        {"buffer": 0, "byteOffset": 4, "byteLength": 12}
    ]);
    for sparse in [false, true] {
        let mut sourced = document.clone();
        if sparse {
            sourced["accessors"][1]["sparse"] = json!({
                "count": 1, "indices": {"bufferView": 0, "componentType": 5121},
                "values": {"bufferView": 1}
            });
        } else {
            sourced["accessors"][1]["bufferView"] = json!(1);
        }
        for (source, bytes) in containers(&sourced)? {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_gltf_failure(
                &failure,
                GltfFailureCode::InvalidAccessor,
                Some("mesh accessor bounds do not match"),
            )?;
            assert_failure_released(&failure);
        }
    }
    Ok(())
}

#[test]
fn sparse_positions_overlay_implicit_zeros_and_preserve_authored_negative_zero() -> TestResult<()> {
    let mut document = points(3);
    document["accessors"][0]["max"] = json!([2, 0, 0]);
    document["buffers"] = json!([{"byteLength": 16}]);
    document["bufferViews"] = json!([
        {"buffer": 0, "byteLength": 1},
        {"buffer": 0, "byteOffset": 4, "byteLength": 12}
    ]);
    document["accessors"][0]["sparse"] = json!({
        "count": 1, "indices": {"bufferView": 0, "componentType": 5121},
        "values": {"bufferView": 1}
    });
    let mut binary = vec![1, 0, 0, 0];
    for value in [2.0_f32, -0.0, 0.0] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    let bytes = append_glb_chunk(
        glb_with_json(&serde_json::to_vec(&document)?),
        b"BIN\0",
        &binary,
    );
    let (report, diagnostics) = import_assets(
        "sparse-zero.glb",
        [("sparse-zero.glb", bytes)],
        GltfTestOptions::memory(),
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
    let positions = primitive.positions().unwrap();
    for (vertex, expected) in [[0, 0, 0], [0x4000_0000, 0x8000_0000, 0], [0, 0, 0]]
        .into_iter()
        .enumerate()
    {
        let Some(AttributeElement::F32(value)) = positions.element(vertex) else {
            panic!("sparse POSITION must retain F32 storage")
        };
        assert_eq!(
            value
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            expected
        );
    }
    assert_eq!(positions.element(3), None);
    assert_released(&report);
    Ok(())
}

#[test]
fn zero_direction_deltas_keep_valid_complete_sparse_base_normals_and_tangents() -> TestResult<()> {
    let mut document = points(3);
    document["buffers"] = json!([{"byteLength": 88}]);
    document["bufferViews"] = json!([
        {"buffer": 0, "byteLength": 3},
        {"buffer": 0, "byteOffset": 4, "byteLength": 36},
        {"buffer": 0, "byteOffset": 40, "byteLength": 48}
    ]);
    document["accessors"].as_array_mut().unwrap().extend([
        json!({"componentType": 5126, "count": 3, "type": "VEC3", "sparse": {
            "count": 3, "indices": {"bufferView": 0, "componentType": 5121},
            "values": {"bufferView": 1}
        }}),
        json!({"componentType": 5126, "count": 3, "type": "VEC4", "sparse": {
            "count": 3, "indices": {"bufferView": 0, "componentType": 5121},
            "values": {"bufferView": 2}
        }}),
    ]);
    document["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(1);
    document["meshes"][0]["primitives"][0]["attributes"]["TANGENT"] = json!(2);
    document["meshes"][0]["primitives"][0]["targets"] =
        json!([{"POSITION": 0, "NORMAL": 0, "TANGENT": 0}]);
    let mut binary = vec![0, 1, 2, 0];
    for value in [0.0_f32, 0.0, 1.0].repeat(3) {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    for value in [1.0_f32, 0.0, 0.0, 1.0].repeat(3) {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    let bytes = append_glb_chunk(
        glb_with_json(&serde_json::to_vec(&document)?),
        b"BIN\0",
        &binary,
    );
    let (report, diagnostics) = import_assets(
        "direction-deltas.glb",
        [("direction-deltas.glb", bytes)],
        GltfTestOptions::memory(),
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
    for (semantic, expected) in [
        (AttributeSemanticRef::Normal, vec![0.0_f32, 0.0, 1.0]),
        (AttributeSemanticRef::Tangent, vec![1.0, 0.0, 0.0, 1.0]),
    ] {
        let attribute = primitive
            .attributes()
            .find(|a| a.semantic() == semantic)
            .unwrap();
        for vertex in 0..3 {
            assert_eq!(
                attribute.element(vertex),
                Some(AttributeElement::F32(&expected))
            );
        }
    }
    let target = primitive.morph_targets().next().unwrap();
    assert_eq!(target.delta_count(), 3);
    for semantic in [
        MorphDeltaSemantic::Position,
        MorphDeltaSemantic::Normal,
        MorphDeltaSemantic::Tangent,
    ] {
        let values = target.delta(semantic).unwrap().data();
        for vertex in 0..3 {
            assert_eq!(values.value(vertex), Some(Vec3::new(0.0, 0.0, 0.0)));
        }
        assert_eq!(values.value(3), None);
    }
    assert_released(&report);

    // Each incomplete direction stream would leave an invalid zero base element.
    for index in [1, 2] {
        let mut partial = document.clone();
        partial["accessors"][index]["sparse"]["count"] = json!(2);
        let bytes = append_glb_chunk(
            glb_with_json(&serde_json::to_vec(&partial)?),
            b"BIN\0",
            &binary,
        );
        let (result, diagnostics) = import_assets_result(
            "partial-directions.glb",
            [("partial-directions.glb", bytes)],
            GltfTestOptions::memory(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::InvalidAccessor,
            Some("implicit zero direction"),
        )?;
        assert_failure_released(&failure);
    }
    Ok(())
}

#[test]
fn implicit_animation_times_translations_and_normalized_signed_weights_keep_zero_bits()
-> TestResult<()> {
    for (path, dimension, component, expected) in [
        ("translation", "VEC3", 5126, vec![0, 0, 0]),
        ("scale", "VEC3", 5126, vec![0, 0, 0]),
        ("weights", "SCALAR", 5120, vec![0]),
        ("weights", "SCALAR", 5122, vec![0]),
    ] {
        let mut document = points(1);
        document["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION": 0}]);
        document["accessors"].as_array_mut().unwrap().extend([
            json!({"componentType": 5126, "count": 1, "type": "SCALAR", "min": [0], "max": [0]}),
            json!({"componentType": component, "count": 1, "type": dimension, "normalized": component != 5126}),
        ]);
        document["animations"] = json!([{
            "samplers": [{"input": 1, "output": 2}],
            "channels": [{"sampler": 0, "target": {"node": 0, "path": path}}]
        }]);
        for (source, bytes) in containers(&document)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let channel = report
                .scene()
                .animation_clips()
                .next()
                .unwrap()
                .channel(0)
                .unwrap();
            assert_eq!(channel.key_times(), [0.0]);
            assert_eq!(
                channel
                    .values()
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_no_dependencies(&report);
            assert_released(&report);
        }
    }
    Ok(())
}

#[test]
fn zero_normal_and_rotation_values_remain_semantically_invalid() -> TestResult<()> {
    let mut normal = points(1);
    normal["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(0);
    for (source, bytes) in containers(&normal)? {
        let (result, diagnostics) =
            import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidAccessor, Some("NORMAL"))?;
        assert_failure_released(&failure);
    }

    let mut rotation = points(1);
    rotation["accessors"].as_array_mut().unwrap().extend([
        json!({"componentType": 5126, "count": 1, "type": "SCALAR", "min": [0], "max": [0]}),
        json!({"componentType": 5126, "count": 1, "type": "VEC4"}),
    ]);
    rotation["animations"] = json!([{
        "samplers": [{"input": 1, "output": 2}],
        "channels": [{"sampler": 0, "target": {"node": 0, "path": "rotation"}}]
    }]);
    for (source, bytes) in containers(&rotation)? {
        let (result, diagnostics) =
            import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::InvalidDocument,
            Some("non-unit animation rotation"),
        )?;
        assert_failure_released(&failure);
    }
    Ok(())
}

#[test]
fn implicit_logical_counts_are_admitted_before_any_vertex_materialization() -> TestResult<()> {
    let document = points(17);
    for (source, bytes) in containers(&document)? {
        let mut exact = GltfTestOptions::memory();
        exact.limits.max_vertices = 17;
        let (report, diagnostics) =
            import_assets(source, [(source, bytes.clone())], exact.clone())?;
        assert!(diagnostics.is_empty());
        assert_eq!(
            report
                .ledger()
                .dimension(ImportDimension::Vertices)
                .unwrap()
                .consumed(),
            17
        );
        assert_released(&report);

        exact.limits.max_vertices = 16;
        let (result, diagnostics) = import_assets_result(source, [(source, bytes)], exact)?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_resource_limit(&failure, "max_vertices");
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::Vertices)
                .unwrap()
                .attempted(),
            0
        );
        assert_failure_released(&failure);
    }

    let count = (usize::MAX as u64) / 12 + 1;
    let bytes = serde_json::to_vec(&points(count))?;
    let (result, diagnostics) = import_assets_result(
        "logical-overflow.gltf",
        [("logical-overflow.gltf", bytes)],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::Allocation { resource: "gltf-accessor-decoded-size" })));
    assert_failure_released(&failure);
    Ok(())
}

#[test]
fn implicit_accessors_still_require_a_positive_count_even_when_unused() -> TestResult<()> {
    for selected in [false, true] {
        let mut document = points(0);
        if !selected {
            document["scenes"][0] = json!({});
        }
        for (source, bytes) in containers(&document)? {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let failure = expected_error(result)?;
            assert_gltf_failure(
                &failure,
                GltfFailureCode::InvalidAccessor,
                Some("count must be positive"),
            )?;
            assert_failure_released(&failure);
        }
    }
    Ok(())
}

#[test]
fn zero_materialization_obeys_live_storage_and_work_limits_and_releases_failure_storage()
-> TestResult<()> {
    const SOURCE: &str = "zero-limits.gltf";
    let bytes = serde_json::to_vec(&points(4097))?;
    let (report, diagnostics) =
        import_assets(SOURCE, [(SOURCE, bytes.clone())], GltfTestOptions::memory())?;
    assert!(diagnostics.is_empty());
    let peak = report.ledger().peak_live_bytes();
    let work = report
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .unwrap()
        .consumed();
    assert!(peak > 0 && work > 0);
    assert_released(&report);
    drop(report);

    let mut exact = GltfTestOptions::memory();
    exact.limits.max_live_bytes = peak;
    let (report, diagnostics) = import_assets(SOURCE, [(SOURCE, bytes.clone())], exact.clone())?;
    assert!(diagnostics.is_empty());
    assert_eq!(report.ledger().peak_live_bytes(), peak);
    assert_released(&report);
    drop(report);

    exact.limits.max_live_bytes = peak - 1;
    let (result, diagnostics) = import_assets_result(SOURCE, [(SOURCE, bytes.clone())], exact)?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    // The final owner admission happens during central publication, after parser scope teardown.
    assert!(
        matches!(failure.cause(), ImportCause::OperationTerminal(terminal)
        if terminal.class() == ImportTerminalFailureClass::Ledger)
    );
    assert!(failure.ledger().peak_live_bytes() < peak);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert_failure_released(&failure);

    let mut exact_work = GltfTestOptions::memory();
    exact_work.limits.max_work_units = work;
    let (report, diagnostics) = import_assets(SOURCE, [(SOURCE, bytes.clone())], exact_work)?;
    assert!(diagnostics.is_empty());
    assert_released(&report);

    let mut below_work = GltfTestOptions::memory();
    below_work.limits.max_work_units = work - 1;
    let (result, diagnostics) = import_assets_result(SOURCE, [(SOURCE, bytes)], below_work)?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert!(
        matches!(failure.cause(), ImportCause::OperationTerminal(terminal)
        if terminal.class() == ImportTerminalFailureClass::Ledger)
    );
    assert!(
        failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap()
            .consumed()
            < work
    );
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert_failure_released(&failure);
    Ok(())
}

#[test]
fn implicit_vertex_iteration_remains_cancellable_with_no_partial_scene() -> TestResult<()> {
    const SOURCE: &str = "cancel-zero.gltf";
    let bytes = serde_json::to_vec(&points(8193))?;
    let (control, polls) = controlled_import(None);
    let (result, diagnostics) = import_assets_result_with_control(
        SOURCE,
        [(SOURCE, bytes.clone())],
        GltfTestOptions::memory(),
        control,
    )?;
    assert!(diagnostics.is_empty());
    let report = result?;
    let total_polls = polls.load(Ordering::SeqCst);
    assert!(total_polls > 4);
    assert_released(&report);
    drop(report);
    for cancel_on in [total_polls / 4, total_polls / 2, 3 * (total_polls / 4)] {
        let (control, polls) = controlled_import(Some(cancel_on));
        let (result, diagnostics) = import_assets_result_with_control(
            SOURCE,
            [(SOURCE, bytes.clone())],
            GltfTestOptions::memory(),
            control,
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert!(matches!(failure.cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled {
            origin: CancellationOrigin::Signal,
        }))));
        assert_eq!(polls.load(Ordering::SeqCst), cancel_on);
        assert_failure_released(&failure);
    }
    Ok(())
}
