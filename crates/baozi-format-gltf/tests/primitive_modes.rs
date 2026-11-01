mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::authoring::{AttributeDomain, MorphDeltaSemantic, Vec3};
use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportReport, ImportTerminalFailureClass,
    MetadataOwnerRef, MetadataValueRef, SourceIdentityRef, SourceObjectLocatorRef,
    SourceScopeDisposition, SourceScopeEntityKind,
};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, assert_resource_limit, expected_error,
    glb_with_json, import_assets, import_assets_result, sidecar_options,
};
use serde_json::{Value, json};

// Six corners exercise every mode without making either list incomplete.
// The importer API exposes the topology reader without re-exporting its enum.
const MODES: [(u32, &str, u32); 7] = [
    (0, "Points", 6),
    (1, "LineList", 3),
    (2, "LineLoop", 6),
    (3, "LineStrip", 5),
    (4, "TriangleList", 2),
    (5, "TriangleStrip", 4),
    (6, "TriangleFan", 4),
];
const INDICES: [u32; 6] = [4, 1, 5, 0, 3, 2];

fn fixture(mode: u32, vertices: usize, indices: Option<(&[u32], u32)>) -> (Value, Vec<u8>) {
    // Keep nonempty storage even for invalid zero-count accessors: the intended error is
    // draw cardinality, not a zero-length buffer or buffer view.
    let mut buffer = Vec::new();
    for vertex in 0..vertices.max(1) {
        for value in [(vertex % 2) as f32, (vertex / 2) as f32, 0.0] {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
    }
    let position_bytes = buffer.len();
    let mut document = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"name": "modes", "primitives": [{
            "attributes": {"POSITION": 0}, "mode": mode
        }]}],
        "buffers": [{"uri": "modes.bin", "byteLength": position_bytes}],
        "bufferViews": [{"buffer": 0, "byteLength": position_bytes, "target": 34962}],
        "accessors": [{
            "bufferView": 0, "componentType": 5126, "count": vertices, "type": "VEC3",
            "min": [0.0, 0.0, 0.0],
            "max": [if vertices > 1 { 1.0 } else { 0.0 }, ((vertices.max(1) - 1) / 2) as f32, 0.0]
        }]
    });
    if let Some((indices, component)) = indices {
        for &index in indices {
            match component {
                5121 => buffer.push(u8::try_from(index).expect("fixture index fits U8")),
                5123 => buffer.extend_from_slice(
                    &u16::try_from(index)
                        .expect("fixture index fits U16")
                        .to_le_bytes(),
                ),
                5125 => buffer.extend_from_slice(&index.to_le_bytes()),
                _ => panic!("unsupported fixture index component {component}"),
            }
        }
        if indices.is_empty() {
            buffer.extend_from_slice(&[0; 4]);
        }
        document["meshes"][0]["primitives"][0]["indices"] = json!(1);
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 0, "byteOffset": position_bytes,
            "byteLength": buffer.len() - position_bytes, "target": 34963
        }));
        document["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView": 1, "componentType": component, "count": indices.len(), "type": "SCALAR"
        }));
        document["buffers"][0]["byteLength"] = json!(buffer.len());
    }
    (document, buffer)
}

fn import_fixture(
    document: &Value,
    buffer: &[u8],
    options: GltfTestOptions,
) -> TestResult<ImportReport> {
    let (report, diagnostics) = import_assets(
        "modes.gltf",
        [
            ("modes.gltf", serde_json::to_vec(document)?),
            ("modes.bin", buffer.to_vec()),
        ],
        options,
    )?;
    assert!(diagnostics.is_empty());
    assert!(report.loss_evidence().is_empty());
    Ok(report)
}

fn reject_before_dependency_io(
    document: &Value,
    options: GltfTestOptions,
) -> TestResult<ImportFailure> {
    let (result, diagnostics) = import_assets_result(
        "modes.gltf",
        [("modes.gltf", serde_json::to_vec(document)?)],
        options,
    )?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    assert!(failure.source_scope_record().is_none());
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .unwrap()
            .attempted(),
        0
    );
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        1
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    Ok(failure)
}

fn assert_counter(report: &ImportReport, dimension: ImportDimension, expected: u64) {
    let counter = report.ledger().dimension(dimension).unwrap();
    assert_eq!(counter.attempted(), expected, "{dimension:?}");
    assert_eq!(counter.consumed(), expected, "{dimension:?}");
}

#[test]
fn all_seven_modes_preserve_indexed_and_nonindexed_draws() -> TestResult<()> {
    for (mode, topology, faces) in MODES {
        for component in [None, Some(5121), Some(5123), Some(5125)] {
            // Eight POSITION rows with six indices prove that indexed cardinality comes
            // from indices, including when the POSITION count cannot form a triangle list.
            let vertices = if component.is_some() { 8 } else { 6 };
            let (document, buffer) = fixture(
                mode,
                vertices,
                component.map(|component| (INDICES.as_slice(), component)),
            );
            let report = import_fixture(&document, &buffer, sidecar_options())?;
            let scene = report.scene();
            assert_eq!((scene.mesh_count(), scene.primitive_count()), (1, 1));
            let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
            assert_eq!(
                format!("{:?}", primitive.topology()),
                topology,
                "mode {mode}/{component:?}"
            );
            assert_eq!(primitive.vertex_count(), vertices as u32);
            assert_eq!(primitive.corner_count(), 6);
            assert_eq!(primitive.face_count(), faces);
            assert_eq!(
                primitive.indices(),
                component.map(|_| INDICES.as_slice()),
                "mode {mode}/{component:?}"
            );
            assert_eq!(
                primitive.positions().unwrap().domain(),
                AttributeDomain::Vertex
            );
            assert_counter(&report, ImportDimension::Faces, u64::from(faces));
            assert_counter(
                &report,
                ImportDimension::TopologyCells,
                if component.is_some() { 6 } else { 0 },
            );
        }
    }
    Ok(())
}

#[test]
fn omitted_mode_defaults_to_triangle_list_without_synthesizing_indices() -> TestResult<()> {
    for indexed in [false, true] {
        let (mut document, buffer) = fixture(4, 6, indexed.then_some((INDICES.as_slice(), 5123)));
        document["meshes"][0]["primitives"][0]
            .as_object_mut()
            .unwrap()
            .remove("mode");
        let report = import_fixture(&document, &buffer, sidecar_options())?;
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert_eq!(format!("{:?}", primitive.topology()), "TriangleList");
        assert_eq!((primitive.corner_count(), primitive.face_count()), (6, 2));
        assert_eq!(primitive.indices(), indexed.then_some(INDICES.as_slice()));
    }
    Ok(())
}

#[test]
fn minimum_draw_lengths_are_valid_for_indexed_and_nonindexed_modes() -> TestResult<()> {
    for (mode, topology, corners, faces) in [
        (0, "Points", 1, 1),
        (1, "LineList", 2, 1),
        (2, "LineLoop", 2, 2),
        (3, "LineStrip", 2, 1),
        (4, "TriangleList", 3, 1),
        (5, "TriangleStrip", 3, 1),
        (6, "TriangleFan", 3, 1),
    ] {
        let indices: Vec<_> = (0..corners as u32).rev().collect();
        for indexed in [false, true] {
            let (document, buffer) = fixture(
                mode,
                if indexed { 6 } else { corners },
                indexed.then_some((indices.as_slice(), 5123)),
            );
            let report = import_fixture(&document, &buffer, sidecar_options())?;
            let primitive = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap();
            assert_eq!(format!("{:?}", primitive.topology()), topology);
            assert_eq!(primitive.corner_count(), corners as u32);
            assert_eq!(primitive.face_count(), faces);
            assert_eq!(primitive.indices(), indexed.then_some(indices.as_slice()));
        }
    }
    Ok(())
}

#[test]
fn empty_short_and_incomplete_draws_fail_before_dependency_io() -> TestResult<()> {
    let invalid: &[(u32, &[usize])] = &[
        (0, &[0]),
        (1, &[0, 1, 3]),
        (2, &[0, 1]),
        (3, &[0, 1]),
        (4, &[0, 1, 2, 4, 5]),
        (5, &[0, 1, 2]),
        (6, &[0, 1, 2]),
    ];
    for &(mode, counts) in invalid {
        for &corners in counts {
            let indices: Vec<_> = (0..corners as u32).collect();
            for indexed in [false, true] {
                let (document, _) = fixture(
                    mode,
                    if indexed { 6 } else { corners },
                    indexed.then_some((indices.as_slice(), 5123)),
                );
                let failure = reject_before_dependency_io(&document, sidecar_options())?;
                let code = if corners == 0 {
                    GltfFailureCode::InvalidAccessor
                } else {
                    GltfFailureCode::InvalidDocument
                };
                assert_gltf_failure(&failure, code, None)?;
            }
        }
    }
    Ok(())
}

#[test]
fn invalid_numeric_modes_are_content_errors_before_dependency_io() -> TestResult<()> {
    for mode in [json!(7), json!(u32::MAX), json!(-1), json!(4.5)] {
        let (mut document, _) = fixture(4, 6, None);
        document["meshes"][0]["primitives"][0]["mode"] = mode;
        let failure = reject_before_dependency_io(&document, sidecar_options())?;
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    }
    Ok(())
}

#[test]
fn every_mode_rejects_out_of_range_indices_and_source_width_restart_values() -> TestResult<()> {
    for (mode, _, _) in MODES {
        for (component, invalid) in [(5121, 255), (5123, 65535), (5125, u32::MAX)] {
            for invalid in [6, invalid] {
                let indices = [0, 1, 2, 3, 4, invalid];
                let (document, buffer) = fixture(mode, 6, Some((&indices, component)));
                let (result, diagnostics) = import_assets_result(
                    "modes.gltf",
                    [
                        ("modes.gltf", serde_json::to_vec(&document)?),
                        ("modes.bin", buffer),
                    ],
                    sidecar_options(),
                )?;
                assert!(diagnostics.is_empty());
                let failure = expected_error(result)?;
                assert_gltf_failure(
                    &failure,
                    GltfFailureCode::InvalidDocument,
                    Some("invalid vertex index"),
                )?;
                assert!(failure.scene_validation_error().is_none());
                assert_eq!(failure.ledger().current_live_bytes(), 0);
                assert_eq!(failure.ledger().active_reservations(), 0);
            }
        }
    }
    Ok(())
}

#[test]
fn restart_values_remain_forbidden_when_they_address_existing_positions() -> TestResult<()> {
    // These payloads are 3 KiB / 768 KiB of positions, not allocations near U32::MAX.
    // The same numeric indices are valid when encoded in the next wider source type.
    for (vertices, restart, narrow, wide) in [(256, 255, 5121, 5123), (65536, 65535, 5123, 5125)] {
        let indices = [0, 1, restart, 2, 3, 4];
        let (document, buffer) = fixture(5, vertices, Some((&indices, narrow)));
        let (result, diagnostics) = import_assets_result(
            "modes.gltf",
            [
                ("modes.gltf", serde_json::to_vec(&document)?),
                ("modes.bin", buffer),
            ],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let failure = expected_error(result)?;
        assert_gltf_failure(
            &failure,
            GltfFailureCode::InvalidDocument,
            Some("invalid vertex index"),
        )?;

        let (document, buffer) = fixture(5, vertices, Some((&indices, wide)));
        let report = import_fixture(&document, &buffer, sidecar_options())?;
        let primitive = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert_eq!(primitive.vertex_count(), vertices as u32);
        assert_eq!(primitive.indices(), Some(indices.as_slice()));
        assert_eq!((primitive.corner_count(), primitive.face_count()), (6, 4));
    }
    Ok(())
}

#[test]
fn degenerate_positions_and_repeated_indices_are_retained_in_every_mode() -> TestResult<()> {
    let repeated = [2, 2, 0, 0, 1, 1];
    for (mode, topology, faces) in MODES {
        for indexed in [false, true] {
            let (mut document, mut buffer) =
                fixture(mode, 6, indexed.then_some((repeated.as_slice(), 5123)));
            buffer[..72].fill(0);
            document["accessors"][0]["max"] = json!([0.0, 0.0, 0.0]);
            let report = import_fixture(&document, &buffer, sidecar_options())?;
            let primitive = report
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap();
            assert_eq!(format!("{:?}", primitive.topology()), topology);
            assert_eq!((primitive.vertex_count(), primitive.corner_count()), (6, 6));
            assert_eq!(primitive.face_count(), faces);
            assert_eq!(primitive.indices(), indexed.then_some(repeated.as_slice()));
            let position = primitive.positions().unwrap();
            assert_eq!((position.lanes(), position.len()), (3, 6));
            for vertex in 0..6 {
                assert_eq!(position.element(vertex), position.element(0));
            }
        }
    }
    Ok(())
}

#[test]
fn face_and_topology_limits_admit_exact_counts_and_reject_one_cell_less_before_io() -> TestResult<()>
{
    for (mode, _, faces) in MODES {
        for indexed in [false, true] {
            let (document, buffer) =
                fixture(mode, 6, indexed.then_some((INDICES.as_slice(), 5123)));
            let topology_cells = if indexed { 6 } else { 0 };
            let mut exact = sidecar_options();
            exact.limits.max_faces = faces as usize;
            exact.limits.max_total_topology_cells = topology_cells;
            let report = import_fixture(&document, &buffer, exact.clone())?;
            assert_counter(&report, ImportDimension::Faces, u64::from(faces));
            assert_counter(
                &report,
                ImportDimension::TopologyCells,
                topology_cells as u64,
            );

            let mut fewer_faces = exact.clone();
            fewer_faces.limits.max_faces -= 1;
            let failure = reject_before_dependency_io(&document, fewer_faces)?;
            assert_resource_limit(&failure, "max_faces");

            if indexed {
                let mut fewer_cells = exact;
                fewer_cells.limits.max_total_topology_cells -= 1;
                let failure = reject_before_dependency_io(&document, fewer_cells)?;
                assert_resource_limit(&failure, "max_total_topology_cells");
            }
        }
    }
    Ok(())
}

fn source_locator(source: SourceIdentityRef<'_>) -> Option<&str> {
    match source.locator() {
        SourceObjectLocatorRef::Namespaced {
            schema,
            schema_version,
            value,
        } => {
            assert_eq!(schema, "org.baozi.gltf:source-object");
            assert_eq!(schema_version, 1);
            Some(value)
        }
        SourceObjectLocatorRef::Indexed { .. } => None,
    }
}

#[test]
fn mixed_modes_keep_order_shared_accessors_deformation_and_source_owned_extras() -> TestResult<()> {
    let (mut document, mut buffer) = fixture(4, 6, Some((&INDICES, 5123)));
    let mut offset = buffer.len();
    for (bytes, component, kind) in [
        (72, 5126, "VEC3"),
        (24, 5121, "VEC4"),
        (96, 5126, "VEC4"),
        (72, 5126, "VEC3"),
    ] {
        let view = document["bufferViews"].as_array().unwrap().len();
        document["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 0, "byteOffset": offset, "byteLength": bytes, "target": 34962
        }));
        document["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView": view, "componentType": component, "count": 6, "type": kind
        }));
        offset += bytes;
    }
    for _ in 0..6 {
        for value in [0.0_f32, 0.0, 1.0] {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
    }
    buffer.extend_from_slice(&[0; 24]);
    for _ in 0..6 {
        for value in [1.0_f32, 0.0, 0.0, 0.0] {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
    }
    for vertex in 0..6 {
        for value in [vertex as f32 * 0.125, -0.0, 0.25] {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
    }
    assert_eq!(buffer.len(), offset);
    document["buffers"][0]["byteLength"] = json!(buffer.len());
    document["buffers"][0]["extras"] = json!({"shared": "buffer"});
    document["bufferViews"][0]["extras"] = json!({"shared": "view"});
    document["accessors"][0]["extras"] = json!({"shared": "position"});
    document["accessors"][5]["min"] = json!([0.0, 0.0, 0.25]);
    document["accessors"][5]["max"] = json!([0.625, 0.0, 0.25]);
    // An unselected earlier mesh prevents canonical mesh ordinals from masquerading as
    // source mesh ordinals. Its unavailable dependency must never be opened.
    document["buffers"].as_array_mut().unwrap().push(json!({
        "uri": "omitted.bin", "byteLength": 36
    }));
    document["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer": 1, "byteLength": 36, "target": 34962
    }));
    document["accessors"].as_array_mut().unwrap().push(json!({
        "bufferView": 6, "componentType": 5126, "count": 3, "type": "VEC3",
        "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]
    }));
    let order = [5, 0, 3, 4, 2, 6, 1];
    let primitives: Vec<_> = order
        .iter()
        .enumerate()
        .map(|(slot, mode)| {
            json!({
                "attributes": {"POSITION": 0, "NORMAL": 2, "JOINTS_0": 3, "WEIGHTS_0": 4},
                "indices": 1, "mode": mode, "material": slot % 2,
                "targets": [{"POSITION": 5}], "extras": {"slot": slot, "mode": mode}
            })
        })
        .collect();
    document["meshes"] = json!([
        {"primitives": [{"attributes": {"POSITION": 6}, "extras": "omitted"}]},
        {"name": "mixed", "weights": [0.25], "primitives": primitives}
    ]);
    document["nodes"] = json!([
        {"mesh": 1, "skin": 0, "weights": [0.75]}, {"mesh": 1, "skin": 0},
        {"name": "joint"}
    ]);
    document["scenes"][0]["nodes"] = json!([0, 1, 2]);
    document["skins"] = json!([{"joints": [2]}]);
    document["materials"] = json!([{"name": "first"}, {"name": "second"}]);
    let json_bytes = serde_json::to_vec(&document)?;

    for is_glb in [false, true] {
        let primary = if is_glb {
            glb_with_json(&json_bytes)
        } else {
            json_bytes.clone()
        };
        let (report, diagnostics) = import_assets(
            "modes.gltf",
            [("modes.gltf", primary), ("modes.bin", buffer.clone())],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        assert!(report.loss_evidence().is_empty());
        let scene = report.scene();
        assert_eq!(
            (
                scene.mesh_count(),
                scene.primitive_count(),
                scene.mesh_instance_count()
            ),
            (1, 7, 2)
        );
        assert_eq!((scene.skin_count(), scene.material_count()), (1, 2));
        assert_eq!(scene.profile_resource_count(), 2);
        assert_counter(&report, ImportDimension::AssetOpen, 2);
        assert_counter(&report, ImportDimension::Faces, 30);
        assert_counter(&report, ImportDimension::TopologyCells, 42);
        let manifest = report.successful_input_manifest();
        assert_eq!(manifest.counts().node_class_count(), 2);
        let mut resources: Vec<_> = manifest
            .node_classes()
            .iter()
            .map(|class| class.identity().value().as_str())
            .collect();
        resources.sort_unstable();
        assert_eq!(resources, ["modes.bin", "modes.gltf"]);

        let scope = report.source_scope_record().unwrap();
        for (kind, selected, omitted) in [
            (SourceScopeEntityKind::Mesh, 1, 1),
            (SourceScopeEntityKind::Primitive, 7, 1),
            (SourceScopeEntityKind::Accessor, 6, 1),
            (SourceScopeEntityKind::BufferView, 6, 1),
            (SourceScopeEntityKind::Buffer, 1, 1),
        ] {
            assert_eq!(scope.counts(kind).selected(), selected);
            assert_eq!(scope.counts(kind).omitted(), omitted);
        }
        let selected_primitives: Vec<_> = scope
            .details()
            .filter(|detail| {
                detail.kind() == SourceScopeEntityKind::Primitive
                    && detail.disposition() == SourceScopeDisposition::Selected
            })
            .map(|detail| detail.identity())
            .collect();
        assert_eq!(
            selected_primitives,
            ["m1p0", "m1p1", "m1p2", "m1p3", "m1p4", "m1p5", "m1p6"]
        );
        let position_source = scene
            .source_identities()
            .find(|source| {
                source_locator(*source)
                    .is_some_and(|value| value.starts_with("kind=accessor;parent=-;index=0;"))
            })
            .unwrap();
        let delta_source = scene
            .source_identities()
            .find(|source| {
                source_locator(*source)
                    .is_some_and(|value| value.starts_with("kind=accessor;parent=-;index=5;"))
            })
            .unwrap();
        let mesh = scene.meshes().next().unwrap();
        assert_eq!(mesh.name(), Some("mixed"));
        assert_eq!(mesh.default_morph_weights(), &[0.25]);
        let definition = mesh.morph_target_definitions().next().unwrap();
        let instances: Vec<_> = scene.mesh_instances().collect();
        assert!(
            instances
                .iter()
                .all(|instance| instance.mesh().id() == mesh.id())
        );
        assert_eq!(instances[0].morph_weights(), Some([0.75].as_slice()));
        assert_eq!(instances[1].morph_weights(), None);
        assert_eq!(
            instances[0].skin().unwrap().id(),
            instances[1].skin().unwrap().id()
        );
        let first = mesh.primitives().next().unwrap();
        let first_positions: Vec<_> = scene
            .owner_provenance(first.positions().unwrap().metadata_owner())?
            .map(|node| node.id())
            .collect();
        let first_delta = first
            .morph_target(definition.id())
            .unwrap()
            .delta(MorphDeltaSemantic::Position)
            .unwrap();
        let first_deltas: Vec<_> = scene
            .owner_provenance(first_delta.metadata_owner())?
            .map(|node| node.id())
            .collect();

        for (slot, primitive) in mesh.primitives().enumerate() {
            let (_, topology, faces) = MODES[order[slot] as usize];
            assert_eq!(format!("{:?}", primitive.topology()), topology);
            assert_eq!(
                (
                    primitive.vertex_count(),
                    primitive.corner_count(),
                    primitive.face_count()
                ),
                (6, 6, faces)
            );
            assert_eq!(primitive.indices(), Some(INDICES.as_slice()));
            assert_eq!(
                primitive.authored_material().unwrap().name(),
                Some(if slot % 2 == 0 { "first" } else { "second" })
            );
            let position = primitive.positions().unwrap();
            let primitive_sources: Vec<_> = scene
                .owner_provenance(primitive.metadata_owner())?
                .map(|node| node.id())
                .collect();
            let position_sources: Vec<_> = scene
                .owner_provenance(position.metadata_owner())?
                .map(|node| node.id())
                .collect();
            assert_eq!(position_sources.len(), 6);
            assert!(
                position_sources
                    .iter()
                    .all(|node| primitive_sources.contains(node))
            );
            // Only the primitive source leaf differs; document, external resource,
            // shared accessor, buffer view and buffer leaves remain the same handles.
            assert_eq!(
                position_sources
                    .iter()
                    .filter(|node| first_positions.contains(node))
                    .count(),
                if slot == 0 { 6 } else { 5 }
            );
            for id in primitive_sources {
                assert_eq!(id.generation(), scene.generation());
                assert_eq!(scene.provenance(id)?.id(), id);
            }
            let prefix = format!("kind=primitive;parent=1;index={slot};");
            let source = scene
                .source_identities()
                .find(|source| {
                    source_locator(*source).is_some_and(|value| value.starts_with(&prefix))
                })
                .unwrap();
            assert_eq!(source.generation(), scene.generation());
            assert_eq!(source.format(), "org.baozi.gltf");
            assert_eq!(source.resource().identity(), "modes.gltf");
            let original = serde_json::to_vec(&document["meshes"][1]["primitives"][slot])?;
            let offset = json_bytes
                .windows(original.len())
                .position(|bytes| bytes == original)
                .unwrap()
                + if is_glb { 20 } else { 0 };
            assert_eq!(
                source_locator(source).unwrap(),
                format!(
                    "kind=primitive;parent=1;index={slot};offset={offset};length={}",
                    original.len()
                )
            );
            let extras = source.metadata_entries().next().unwrap();
            assert_eq!(extras.owner(), MetadataOwnerRef::SourceObject(source));
            assert_eq!(extras.key(), "org.baozi.gltf:extras");
            let MetadataValueRef::Bytes {
                media_type,
                payload,
            } = extras.value()
            else {
                panic!("primitive extras must retain their JSON bytes");
            };
            assert_eq!(media_type, "application/json");
            assert_eq!(
                payload,
                serde_json::to_vec(&document["meshes"][1]["primitives"][slot]["extras"])?
            );
            let influences = primitive.influences().unwrap();
            assert_eq!(influences.len(), 6);
            for vertex in 0..6 {
                let row = influences.get(vertex).unwrap();
                assert_eq!(row.joint_indices(), &[0, 0, 0, 0]);
                assert_eq!(row.weights(), &[1.0, 0.0, 0.0, 0.0]);
            }
            let delta = primitive
                .morph_target(definition.id())
                .unwrap()
                .delta(MorphDeltaSemantic::Position)
                .unwrap();
            assert_eq!(delta.len(), 6);
            let delta_sources: Vec<_> = scene
                .owner_provenance(delta.metadata_owner())?
                .map(|node| node.id())
                .collect();
            assert_eq!(delta_sources.len(), 6);
            assert_eq!(
                delta_sources
                    .iter()
                    .filter(|node| first_deltas.contains(node))
                    .count(),
                if slot == 0 { 6 } else { 5 }
            );
            for vertex in 0..6 {
                let value = delta.data().value(vertex).unwrap();
                assert_eq!(value, Vec3::new(vertex as f32 * 0.125, -0.0, 0.25));
                assert_eq!(value.y.to_bits(), (-0.0_f32).to_bits());
            }
        }
        // Shared source extras occur once each, not once per canonical attribute/primitive.
        assert_eq!(scene.metadata_count(), 10);
        assert_eq!(position_source.metadata_entries().len(), 1);
        assert_eq!(delta_source.generation(), scene.generation());
        assert!(
            scene
                .metadata_entries()
                .all(|entry| matches!(entry.owner(), MetadataOwnerRef::SourceObject(_)))
        );
    }
    Ok(())
}

#[test]
fn fan_import_admits_exact_total_work_and_rejects_one_less_at_publication() -> TestResult<()> {
    let (document, buffer) = fixture(6, 6, Some((&INDICES, 5123)));
    let baseline = import_fixture(&document, &buffer, sidecar_options())?;
    let work = baseline
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .unwrap();
    let (attempted, consumed) = (work.attempted(), work.consumed());
    assert!(consumed > 0);
    assert_eq!(attempted, consumed);
    let mut exact = sidecar_options();
    exact.limits.max_work_units = consumed;
    let report = import_fixture(&document, &buffer, exact.clone())?;
    assert_counter(&report, ImportDimension::WorkUnits, consumed);

    exact.limits.max_work_units -= 1;
    let (result, diagnostics) = import_assets_result(
        "modes.gltf",
        [
            ("modes.gltf", serde_json::to_vec(&document)?),
            ("modes.bin", buffer),
        ],
        exact,
    )?;
    assert!(diagnostics.is_empty());
    let failure = expected_error(result)?;
    // This final work unit belongs to central publication, after the parser scope is gone.
    assert!(
        matches!(failure.cause(), ImportCause::OperationTerminal(terminal)
        if terminal.class() == ImportTerminalFailureClass::Ledger)
    );
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    assert!(
        failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap()
            .consumed()
            < consumed
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    Ok(())
}
