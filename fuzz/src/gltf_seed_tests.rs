use baozi::{
    AttributeDomain, AttributeElement, AttributeSemanticRef, LightKindCategory, TopologyKind, Vec3,
};
use baozi_format_gltf::{FORMAT_ID, GltfImporter};
use baozi_import::{
    ImportDimension, ImportOperationControl, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeSelectionOutcome, SourceScopeSelectionRequest, import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::sync::Arc;

use crate::support;

fn seed_io(data: &[u8]) -> MemoryAssetIo {
    // These JSON seeds use the gltf_import target's first-NUL external-buffer protocol.
    let split = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let primary = &data[..split];
    let buffer = data.get(split.saturating_add(1)..).unwrap_or(&[]);
    let io = MemoryAssetIo::new();
    for (path, bytes) in [("fuzz.gltf", primary), ("buffer.bin", buffer)] {
        io.try_insert(AssetPath::new(path).unwrap(), Arc::<[u8]>::from(bytes))
            .expect("the fixed seed asset must fit in memory");
    }
    io
}

#[test]
fn default_scene_one_seed_selects_its_nodes_and_records_the_omission() {
    let request = support::prepared_request(GltfImporter, FORMAT_ID);
    let source = AssetPath::new("fuzz.gltf").unwrap();
    let io = seed_io(include_bytes!(
        "../corpus/gltf_import/scene_one_selection.gltf"
    ));
    let report = import_asset(
        &request,
        &io,
        source.clone(),
        ImportOperationControl::default(),
    )
    .expect("the scene-selection seed must reach a successful import");
    let scene = report.scene();
    assert_eq!(scene.node_count(), 2);
    assert_eq!(scene.root_count(), 1);
    assert_eq!(scene.mesh_count(), 0);
    let root = scene.roots().next().unwrap();
    assert_eq!(root.name(), Some("Chosen"));
    assert_eq!(
        root.authored_transform()
            .authored_trs()
            .unwrap()
            .translation(),
        Vec3::new(2.0, -3.0, 5.0)
    );
    let children = root.children().collect::<Vec<_>>();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name(), Some("Leaf"));
    assert_eq!(
        children[0]
            .authored_transform()
            .authored_trs()
            .unwrap()
            .translation(),
        Vec3::new(0.0, 4.0, 0.0)
    );
    assert!(children[0].children().next().is_none());

    let scope = report
        .source_scope_record()
        .expect("source selection evidence");
    assert_eq!(
        scope.selection_request(),
        &SourceScopeSelectionRequest::DocumentDefault
    );
    assert_eq!(
        scope.selection_outcome(),
        &SourceScopeSelectionOutcome::declared_scene(source, 1)
    );
    assert_eq!(scope.counts(SourceScopeEntityKind::Scene).selected(), 1);
    assert_eq!(scope.counts(SourceScopeEntityKind::Scene).omitted(), 1);
    assert_eq!(scope.counts(SourceScopeEntityKind::Node).selected(), 2);
    assert_eq!(scope.counts(SourceScopeEntityKind::Node).omitted(), 1);
    assert_eq!(
        scope
            .details()
            .filter(|detail| detail.kind() == SourceScopeEntityKind::Node)
            .map(|detail| (detail.disposition(), detail.identity()))
            .collect::<Vec<_>>(),
        [
            (SourceScopeDisposition::Omitted, "n0"),
            (SourceScopeDisposition::Selected, "n1"),
            (SourceScopeDisposition::Selected, "n2"),
        ]
    );
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    support::assert_validated_snapshot(scene);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}

#[test]
fn material_and_light_seed_retains_authored_lobes_and_attached_emission() {
    let request = support::prepared_request(GltfImporter, FORMAT_ID);
    let source = AssetPath::new("fuzz.gltf").unwrap();
    let io = seed_io(include_bytes!(
        "../corpus/gltf_import/material_lobes_light.gltf"
    ));
    let report = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect("the material/light seed must reach a successful import");
    let scene = report.scene();
    assert_eq!(scene.node_count(), 1);
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.material_count(), 1);
    assert_eq!(scene.light_count(), 1);
    assert_eq!((scene.texture_count(), scene.image_count()), (0, 0));
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(
        (
            primitive.vertex_count(),
            primitive.corner_count(),
            primitive.face_count()
        ),
        (3, 3, 1)
    );
    let positions = primitive.positions().unwrap();
    for (vertex, expected) in [[1.0, 2.0, 0.0], [3.0, 2.0, 0.0], [1.0, 5.0, 0.0]]
        .iter()
        .enumerate()
    {
        assert_eq!(
            positions.element(vertex),
            Some(AttributeElement::F32(expected))
        );
    }
    let material = primitive
        .authored_material()
        .expect("selected authored material");
    assert_eq!(material.id(), scene.materials().next().unwrap().id());
    assert_eq!(material.name(), Some("Lobes"));
    assert_eq!(material.binding_count(), 0);
    let emission = material.factors().emission();
    assert_eq!(emission.factor(), Vec3::new(0.25, 0.5, 1.0));
    assert_eq!(emission.strength(), Some(8.0));
    assert_eq!(emission.effective_rgb(), [2.0, 4.0, 8.0]);
    let coat = material.factors().clearcoat().expect("retained clearcoat");
    assert_eq!([coat.factor(), coat.roughness()], [0.75, 0.125]);
    let sheen = material.factors().sheen().expect("retained sheen");
    assert_eq!(sheen.color(), Vec3::new(0.5, 0.25, 0.75));
    assert_eq!(sheen.roughness(), 0.625);

    let node = scene.nodes().next().unwrap();
    assert_eq!(node.name(), Some("Lit"));
    let light = node.light().expect("selected node's light attachment");
    assert_eq!(light.id(), scene.lights().next().unwrap().id());
    let definition = light.definition();
    assert_eq!(definition.kind().category(), LightKindCategory::Point);
    assert_eq!(definition.kind().range(), Some(8.0));
    // Authored gray 0.5 times intensity 6 is white tint with canonical intensity 3.
    assert_eq!(definition.tint(), Vec3::new(1.0, 1.0, 1.0));
    assert_eq!(definition.intensity(), 3.0);
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    support::assert_validated_snapshot(scene);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}

#[test]
fn quantized_seed_decodes_three_vertices_with_explicit_four_byte_strides() {
    let request = support::prepared_request(GltfImporter, FORMAT_ID);
    let source = AssetPath::new("fuzz.gltf").unwrap();
    let io = seed_io(include_bytes!(
        "../corpus/gltf_import/quantized_attributes.gltfbin"
    ));
    let report = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect("the quantized seed must reach a successful import");
    let scene = report.scene();
    assert_eq!((scene.mesh_count(), scene.mesh_instance_count()), (1, 1));
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(
        (
            primitive.vertex_count(),
            primitive.corner_count(),
            primitive.face_count()
        ),
        (3, 3, 1)
    );
    assert_eq!(primitive.indices(), None);
    assert_eq!(primitive.attributes().count(), 3);
    let positions = primitive.positions().unwrap();
    let normals = primitive.normals(AttributeDomain::Vertex).unwrap();
    let uvs = primitive.texcoords(0, AttributeDomain::Vertex).unwrap();
    for attribute in [positions, normals, uvs] {
        assert_eq!(attribute.len(), 3);
        assert_eq!(attribute.domain(), AttributeDomain::Vertex);
        assert_eq!(attribute.element(3), None);
    }
    // The fixed stored samples use signed normalized endpoints, including -128/-32768.
    for (vertex, expected) in [[-3.0, 2.0, 1.0], [5.0, 2.0, 1.0], [-3.0, 6.0, 1.0]]
        .iter()
        .enumerate()
    {
        assert_eq!(
            positions.element(vertex),
            Some(AttributeElement::F32(expected))
        );
    }
    for (vertex, expected) in [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        .iter()
        .enumerate()
    {
        assert_eq!(
            normals.element(vertex),
            Some(AttributeElement::F32(expected))
        );
    }
    for (vertex, expected) in [[-1.0, 1.0], [0.0, -1.0], [1.0, 0.0]].iter().enumerate() {
        assert_eq!(uvs.element(vertex), Some(AttributeElement::F32(expected)));
    }
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    support::assert_validated_snapshot(scene);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}

#[test]
fn implicit_zero_seed_materializes_attributes_and_indices_without_dependencies() {
    let request = support::prepared_request(GltfImporter, FORMAT_ID);
    let source = AssetPath::new("fuzz.gltf").unwrap();
    let io = seed_io(include_bytes!(
        "../corpus/gltf_import/implicit_zero_attributes.gltf"
    ));
    let report = import_asset(&request, &io, source, ImportOperationControl::default())
        .expect("the implicit-zero seed must reach a successful import");
    let scene = report.scene();
    assert_eq!(scene.mesh_count(), 1);
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::Points);
    assert_eq!(
        (
            primitive.vertex_count(),
            primitive.corner_count(),
            primitive.face_count()
        ),
        (3, 3, 3)
    );
    assert_eq!(primitive.indices(), Some([0, 0, 0].as_slice()));
    let colors = primitive
        .attribute(AttributeSemanticRef::Color(0), AttributeDomain::Vertex)
        .unwrap();
    for (attribute, expected_bits) in [
        (primitive.positions().unwrap(), [0_u32, 0, 0].as_slice()),
        (
            primitive.texcoords(0, AttributeDomain::Vertex).unwrap(),
            [0, 0].as_slice(),
        ),
        (colors, [0, 0, 0, 0x3f80_0000].as_slice()),
    ] {
        assert_eq!(attribute.len(), 3);
        assert_eq!(attribute.domain(), AttributeDomain::Vertex);
        for vertex in 0..3 {
            let Some(AttributeElement::F32(values)) = attribute.element(vertex) else {
                panic!("implicit-zero attributes must retain F32 storage");
            };
            assert_eq!(
                values
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                expected_bits
            );
        }
        assert_eq!(attribute.element(3), None);
    }
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
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    support::assert_validated_snapshot(scene);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}
