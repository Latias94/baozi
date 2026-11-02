mod common;

use baozi_core::{AttributeElement, ProvenanceKindRef};
use baozi_format_gltf::GltfFailureCode;
use baozi_import::authoring::{Quat, Vec3};
use baozi_import::{
    ImportDimension, ImportFailure, ImportLedgerSnapshot, ImportReport, ImportTerminalCause,
    SourceObjectLocatorRef,
};
use common::{
    GltfTestOptions, TestResult, append_glb_chunk, assert_gltf_failure, expected_error,
    glb_with_json, import_assets, import_assets_result,
};
use serde_json::{Value, json};

fn encodings(document: &Value) -> TestResult<[(&'static str, Vec<u8>); 2]> {
    let json = serde_json::to_vec(document)?;
    let glb = glb_with_json(&json);
    Ok([("compatibility.gltf", json), ("compatibility.glb", glb)])
}

fn assert_released_without_dependencies(ledger: &ImportLedgerSnapshot) {
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(ledger.current_evidence_bytes(), 0);
    assert_eq!(ledger.active_reservations(), 0);
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
    ] {
        assert_eq!(ledger.dimension(dimension).unwrap().attempted(), 0);
    }
    assert_eq!(
        ledger
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        1
    );
}

fn assert_content_failure(
    failure: &ImportFailure,
    code: GltfFailureCode,
    reason: Option<&str>,
) -> TestResult<()> {
    assert_gltf_failure(failure, code, reason)?;
    assert!(failure.diagnostics().is_empty());
    assert_released_without_dependencies(failure.ledger());
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert!(matches!(
        failure.ledger().terminal_cause(),
        ImportTerminalCause::Failure(_)
    ));
    Ok(())
}

fn core_document(selected: bool) -> Value {
    json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"name": "TypedScene", "nodes": if selected { vec![0] } else { vec![] }}],
        "nodes": [{
            "name": "TypedNode", "mesh": 0,
            "translation": [0, 0, 0], "scale": [1, 1, 1]
        }],
        "buffers": [{
            "byteLength": 60,
            "uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAIA/"
        }],
        "bufferViews": [
            {"buffer": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 24}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0, 0, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"}
        ],
        "images": [{"uri": "data:image/png;base64,AA=="}],
        "textures": [{"source": 0}],
        "materials": [{
            "doubleSided": false,
            "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}
        }],
        "meshes": [{"primitives": [{
            "attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "material": 0
        }]}]
    })
}

#[test]
fn core_wrong_types_reject_selected_and_omitted_rows_before_dependency_io() -> TestResult<()> {
    let cases = [
        ("/meshes/0/primitives", json!({})),
        ("/materials/0/pbrMetallicRoughness", json!([])),
        ("/scenes/0/name", json!(false)),
        ("/buffers/0/byteLength", json!(-1)),
        (
            "/materials/0/pbrMetallicRoughness/baseColorTexture/index",
            json!("0"),
        ),
        ("/materials/0/doubleSided", json!(1)),
        ("/nodes/0/translation", json!([0, 0])),
        ("/nodes/0/scale", json!([1, "1", 1])),
    ];
    for selected in [false, true] {
        let baseline = core_document(selected);
        for (source, bytes) in encodings(&baseline)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            assert_eq!(report.scene().mesh_count(), usize::from(selected));
            assert_eq!(report.scene().material_count(), usize::from(selected));
            assert_released_without_dependencies(report.ledger());
        }
        for (pointer, wrong_type) in &cases {
            let mut document = baseline.clone();
            *document.pointer_mut(pointer).expect("fixture field exists") = wrong_type.clone();
            document["buffers"][0]["uri"] = json!("never.bin");
            document["images"][0]["uri"] = json!("never.png");
            for (source, bytes) in encodings(&document)? {
                let (result, diagnostics) =
                    import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(diagnostics.is_empty(), "{source}: {pointer}");
                let failure = expected_error(result)?;
                assert_content_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
                assert!(failure.source_scope_record().is_none(), "{pointer}");
            }
        }
    }
    Ok(())
}

#[test]
fn absent_and_explicit_core_defaults_retain_trs_and_material_values() -> TestResult<()> {
    for (authored, translation, scale, double_sided) in [
        (None, Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0), false),
        (
            Some(json!({"translation": [0, 0, 0], "scale": [1, 1, 1], "doubleSided": false})),
            Vec3::ZERO,
            Vec3::new(1.0, 1.0, 1.0),
            false,
        ),
        (
            Some(json!({"translation": [2, -3, 5], "scale": [0.5, 2, -1], "doubleSided": true})),
            Vec3::new(2.0, -3.0, 5.0),
            Vec3::new(0.5, 2.0, -1.0),
            true,
        ),
    ] {
        let mut document = core_document(true);
        for key in ["translation", "scale"] {
            document["nodes"][0].as_object_mut().unwrap().remove(key);
        }
        document["materials"][0]
            .as_object_mut()
            .unwrap()
            .remove("doubleSided");
        if let Some(authored) = authored {
            for key in ["translation", "scale"] {
                document["nodes"][0][key] = authored[key].clone();
            }
            document["materials"][0]["doubleSided"] = authored["doubleSided"].clone();
        }
        for (source, bytes) in encodings(&document)? {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            let scene = report.scene();
            let node = scene.nodes().next().expect("selected authored node");
            assert_eq!(node.name(), Some("TypedNode"));
            let trs = node
                .authored_transform()
                .authored_trs()
                .expect("omitted components retain authored TRS form");
            assert_eq!(trs.translation(), translation);
            assert_eq!(trs.rotation(), Quat::IDENTITY);
            assert_eq!(trs.scale(), scale);
            assert_eq!(
                scene.materials().next().unwrap().double_sided(),
                double_sided
            );
            assert!(report.loss_evidence().is_empty());
            assert_released_without_dependencies(report.ledger());
        }
    }
    Ok(())
}

#[test]
fn sparse_mat4_replaces_only_the_second_bind_and_preserves_every_lane_bit() -> TestResult<()> {
    let mut binary = Vec::new();
    for value in [1.0_f32, 2.0, 3.0] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    binary.extend_from_slice(&[0, 1, 0, 0]);
    for value in [0.0_f32, 1.0, 0.0, 0.0] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(binary.len(), 32);
    for _ in 0..2 {
        for value in [
            1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ] {
            binary.extend_from_slice(&value.to_le_bytes());
        }
    }
    assert_eq!(binary.len(), 160);
    binary.push(1);
    binary.resize(164, 0);
    for value in [
        2.0_f32, 0.0, 0.0, 0.0, 0.5, 3.0, 0.0, 0.0, -0.0, 0.25, -4.0, 0.0, 5.0, -6.0, 7.0, 1.0,
    ] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(binary.len(), 228);
    let document = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0, 1, 2]}],
        "nodes": [{"mesh": 0, "skin": 0}, {"name": "FirstJoint"}, {"name": "PatchedJoint"}],
        "buffers": [{"byteLength": 228}],
        "bufferViews": [
            {"buffer": 0, "byteLength": 12},
            {"buffer": 0, "byteOffset": 12, "byteLength": 4, "byteStride": 4},
            {"buffer": 0, "byteOffset": 16, "byteLength": 16},
            {"buffer": 0, "byteOffset": 32, "byteLength": 128},
            {"buffer": 0, "byteOffset": 160, "byteLength": 1},
            {"buffer": 0, "byteOffset": 164, "byteLength": 64}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
             "min": [1, 2, 3], "max": [1, 2, 3]},
            {"bufferView": 1, "componentType": 5121, "count": 1, "type": "VEC4"},
            {"bufferView": 2, "componentType": 5126, "count": 1, "type": "VEC4"},
            {"bufferView": 3, "componentType": 5126, "count": 2, "type": "MAT4",
             "sparse": {"count": 1,
                "indices": {"bufferView": 4, "componentType": 5121},
                "values": {"bufferView": 5}}}
        ],
        "skins": [{"joints": [1, 2], "inverseBindMatrices": 3}],
        "meshes": [{"primitives": [{
            "attributes": {"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}, "mode": 0
        }]}]
    });
    const SOURCE: &str = "sparse-binds.glb";
    let bytes = append_glb_chunk(
        glb_with_json(&serde_json::to_vec(&document)?),
        b"BIN\0",
        &binary,
    );
    let (report, diagnostics) =
        import_assets(SOURCE, [(SOURCE, bytes)], GltfTestOptions::memory())?;
    assert!(diagnostics.is_empty());
    let scene = report.scene();
    let skin = scene.skins().next().expect("selected skin");
    assert_eq!(skin.joint_count(), 2);
    let matrices = skin.inverse_bind_matrices().collect::<Vec<_>>();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let influences = primitive.influences().unwrap().get(0).unwrap();
    assert_eq!(influences.joint_indices(), &[0, 1, 0, 0]);
    assert_eq!(influences.weights(), &[0.0, 1.0, 0.0, 0.0]);
    assert_eq!(
        primitive.positions().unwrap().element(0),
        Some(AttributeElement::F32(&[1.0, 2.0, 3.0]))
    );
    {
        let mut pending = scene
            .owner_provenance(skin.metadata_owner())?
            .collect::<Vec<_>>();
        let mut visited = Vec::new();
        let mut sources = Vec::new();
        while let Some(node) = pending.pop() {
            if visited.contains(&node.id()) {
                continue;
            }
            visited.push(node.id());
            assert!(visited.len() <= scene.provenance_node_count());
            if let ProvenanceKindRef::Source(source) = node.kind() {
                sources.push(source);
            }
            pending.extend(node.parents());
        }
        for selector in [
            "kind=accessor;parent=-;index=3;",
            "kind=buffer-view;parent=-;index=3;",
            "kind=buffer-view;parent=-;index=4;",
            "kind=buffer-view;parent=-;index=5;",
        ] {
            assert!(sources.iter().any(|source| {
                source.resource().identity() == SOURCE
                    && matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                        if value.starts_with(selector))
            }), "skin must own the base, sparse selector and replacement sources: {selector}");
        }
    }
    assert_released_without_dependencies(report.ledger());
    drop(report);

    let expected: [[[f32; 4]; 4]; 2] = [
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        [
            [2.0, 0.0, 0.0, 0.0],
            [0.5, 3.0, 0.0, 0.0],
            [-0.0, 0.25, -4.0, 0.0],
            [5.0, -6.0, 7.0, 1.0],
        ],
    ];
    assert_eq!(matrices.len(), 2);
    for (actual, expected) in matrices.iter().zip(expected) {
        assert_eq!(
            actual.cols.map(|column| column.map(f32::to_bits)),
            expected.map(|column| column.map(f32::to_bits))
        );
    }
    let transformed: [f64; 4] = std::array::from_fn(|row| {
        matrices[1]
            .cols
            .iter()
            .zip([1.0, 2.0, 3.0, 1.0])
            .map(|(column, component)| f64::from(column[row]) * component)
            .sum()
    });
    assert_eq!(transformed, [8.0, 0.75, -5.0, 1.0]);
    Ok(())
}

fn indexed_point_document(byte_length: usize) -> Value {
    json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "buffers": [{"byteLength": byte_length}],
        "bufferViews": [
            {"buffer": 0, "byteLength": 12},
            {"buffer": 0, "byteOffset": 12, "byteLength": 1}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
             "min": [1, 2, 3], "max": [1, 2, 3]},
            {"bufferView": 1, "componentType": 5121, "count": 1, "type": "SCALAR"}
        ],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "mode": 0}]}]
    })
}

fn point_binary() -> Vec<u8> {
    let mut binary = Vec::new();
    for value in [1.0_f32, 2.0, 3.0] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    binary.push(0);
    binary
}

fn point_glb(document: &Value, binary: &[u8]) -> TestResult<Vec<u8>> {
    let bytes = append_glb_chunk(
        glb_with_json(&serde_json::to_vec(document)?),
        b"BIN\0",
        binary,
    );
    assert!(bytes.len() < 1024);
    Ok(bytes)
}

fn assert_one_indexed_point(report: &ImportReport) {
    assert_eq!(report.scene().mesh_count(), 1);
    let mesh = report.scene().meshes().next().unwrap();
    assert_eq!(mesh.primitive_count(), 1);
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.vertex_count(), 1);
    assert_eq!(primitive.corner_count(), 1);
    assert_eq!(primitive.face_count(), 1);
    assert_eq!(primitive.indices(), Some([0_u32].as_slice()));
    let positions = primitive.positions().unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(
        positions.element(0),
        Some(AttributeElement::F32(&[1.0, 2.0, 3.0]))
    );
    assert_eq!(positions.element(1), None);
    assert_released_without_dependencies(report.ledger());
}

#[test]
fn glb_accepts_zero_to_three_physical_padding_bytes_without_extra_cells() -> TestResult<()> {
    for declared_length in 13..=16 {
        let binary = point_binary();
        assert_eq!(binary.len(), 13);
        let bytes = point_glb(&indexed_point_document(declared_length), &binary)?;
        assert_eq!(&bytes[bytes.len() - 3..], &[0, 0, 0]);
        let (report, diagnostics) = import_assets(
            "padding.glb",
            [("padding.glb", bytes)],
            GltfTestOptions::memory(),
        )?;
        assert!(diagnostics.is_empty());
        assert_one_indexed_point(&report);
    }
    Ok(())
}

#[test]
fn glb_physical_tail_is_addressable_only_when_the_buffer_declares_it() -> TestResult<()> {
    for accessor_offset in [0, 3] {
        for declared_length in [16, 13] {
            let mut document = indexed_point_document(declared_length);
            document["bufferViews"][1]["byteLength"] = json!(4);
            document["accessors"][1]["byteOffset"] = json!(accessor_offset);
            let bytes = point_glb(&document, &point_binary())?;
            let (result, diagnostics) = import_assets_result(
                "logical-tail.glb",
                [("logical-tail.glb", bytes)],
                GltfTestOptions::memory(),
            )?;
            assert!(diagnostics.is_empty());
            if declared_length == 16 {
                assert_one_indexed_point(&result?);
            } else {
                let failure = expected_error(result)?;
                assert_content_failure(
                    &failure,
                    GltfFailureCode::InvalidAccessor,
                    Some("exceeds declared buffer byteLength"),
                )?;
            }
        }
    }
    Ok(())
}

#[test]
fn glb_bin_shorter_than_the_declared_prefix_has_a_content_failure() -> TestResult<()> {
    let bytes = point_glb(&indexed_point_document(13), &point_binary()[..12])?;
    let (result, diagnostics) = import_assets_result(
        "short-bin.glb",
        [("short-bin.glb", bytes)],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    assert_content_failure(
        &expected_error(result)?,
        GltfFailureCode::InvalidDocument,
        Some("declares 13 bytes but only 12 bytes are available"),
    )
}
