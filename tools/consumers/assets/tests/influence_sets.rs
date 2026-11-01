use std::sync::Arc;

use baozi::{
    AssetPath, Axis, FrontFaceWinding, Handedness, Importer, InfluenceRowRef, Mat4, MemoryAssetIo,
    ProvenanceKindRef, TargetSpace, TopologyKind, ValidatedScene,
    postprocess::{
        PostProcessControl, PostProcessRunDisposition, SpaceConversionOptions, convert_scene_space,
        list_vertices,
    },
};
use baozi_asset_consumer::{ApplicationAsset, Result};
use serde_json::json;

const POSITIONS: [[f32; 3]; 5] = [
    [0.25, 0.5, 0.75],
    [1.0, 0.0, 0.5],
    [0.0, 1.0, 1.0],
    [0.25, 0.5, 0.75],
    [1.0, 1.0, 1.0],
];
const UVS: [[f32; 2]; 5] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 0.5], [1.0, 1.0]];
// Vertices 0 and 3 form a UV seam with identical positions and deformation data.
// The first encounter order differs from source order, and several indices are reused.
const INDICES: [u32; 9] = [2, 0, 1, 2, 1, 4, 4, 3, 2];
const CONVERTED_INDICES: [u32; 9] = [2, 1, 0, 2, 4, 1, 4, 2, 3];
const JOINTS: [[u8; 12]; 5] = [
    [9, 2, 11, 4, 1, 8, 3, 12, 7, 5, 10, 6],
    [4, 12, 2, 9, 7, 1, 11, 5, 10, 6, 3, 8],
    [8, 3, 10, 1, 12, 6, 4, 9, 2, 11, 5, 7],
    [9, 2, 11, 4, 1, 8, 3, 12, 7, 5, 10, 6],
    [6, 11, 1, 8, 3, 10, 12, 2, 5, 9, 7, 4],
];
// Binary fractions sum exactly to one. Zero lanes, including signed zero, must survive.
// Only vertices 1 and 4 have nonzero third-set weights; its sparse accessor has no base.
const WEIGHTS: [[f32; 12]; 5] = [
    [
        0.125, 0.25, 0.0625, 0.1875, 0.125, 0.0625, 0.125, 0.0625, 0.0, 0.0, 0.0, 0.0,
    ],
    [
        0.03125, 0.0625, 0.125, 0.09375, 0.15625, 0.03125, 0.0625, 0.125, 0.09375, 0.0625, 0.15625,
        -0.0,
    ],
    [
        -0.0, 0.125, 0.1875, 0.0625, 0.3125, 0.125, 0.0625, 0.125, 0.0, 0.0, 0.0, 0.0,
    ],
    [
        0.125, 0.25, 0.0625, 0.1875, 0.125, 0.0625, 0.125, 0.0625, 0.0, 0.0, 0.0, 0.0,
    ],
    [
        0.0625, 0.03125, 0.125, 0.03125, 0.09375, 0.0625, 0.125, 0.15625, 0.0625, 0.1875, 0.0,
        0.0625,
    ],
];
// Both skins reference the same joint hierarchy in different, nonidentity palette orders.
// Palette entry zero is unused; every authored influence has a distinct nonzero joint.
const PALETTES: [[usize; 13]; 2] = [
    [2, 8, 4, 14, 6, 3, 12, 5, 11, 7, 13, 9, 10],
    [14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2],
];

fn influence_sets_io(sets: usize, weights: &[[f32; 12]; 5]) -> Result<MemoryAssetIo> {
    // Self-authored glTF and little-endian bytes, independent of importer test fixtures.
    let mut payload = Vec::new();
    for value in POSITIONS.iter().flatten().chain(UVS.iter().flatten()) {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    let mut views = vec![
        json!({"buffer": 0, "byteOffset": 0, "byteLength": 60}),
        json!({"buffer": 0, "byteOffset": 60, "byteLength": 40}),
    ];
    let mut accessors = vec![
        json!({"bufferView": 0, "componentType": 5126, "count": 5, "type": "VEC3",
               "min": [0.0, 0.0, 0.5], "max": [1.0, 1.0, 1.0]}),
        json!({"bufferView": 1, "componentType": 5126, "count": 5, "type": "VEC2"}),
    ];
    let mut attributes = json!({"POSITION": 0, "TEXCOORD_0": 1});
    for set in 0..sets {
        let lanes = set * 4..set * 4 + 4;
        let offset = payload.len();
        for joints in JOINTS {
            payload.extend_from_slice(&joints[lanes.clone()]);
        }
        attributes[format!("JOINTS_{set}")] = json!(accessors.len());
        accessors.push(json!({
            "bufferView": views.len(), "componentType": 5121, "count": 5, "type": "VEC4"
        }));
        views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": 20}));

        attributes[format!("WEIGHTS_{set}")] = json!(accessors.len());
        if set == 2 {
            // Separate external resources make loss of higher-set sparse provenance observable.
            accessors.push(json!({
                "componentType": 5126, "count": 5, "type": "VEC4",
                "sparse": {"count": 2,
                    "indices": {"bufferView": views.len(), "componentType": 5121},
                    "values": {"bufferView": views.len() + 1}}
            }));
            views.push(json!({"buffer": 1, "byteLength": 2}));
            views.push(json!({"buffer": 2, "byteLength": 32}));
        } else {
            let offset = payload.len();
            for row in weights {
                for weight in &row[lanes.clone()] {
                    payload.extend_from_slice(&weight.to_le_bytes());
                }
            }
            accessors.push(json!({
                "bufferView": views.len(), "componentType": 5126, "count": 5, "type": "VEC4"
            }));
            views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": 80}));
        }
    }
    let indices = accessors.len();
    let offset = payload.len();
    for index in INDICES {
        payload.extend_from_slice(&u16::try_from(index)?.to_le_bytes());
    }
    accessors.push(json!({
        "bufferView": views.len(), "componentType": 5123, "count": 9, "type": "SCALAR"
    }));
    views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": 18}));

    let mut buffers = vec![json!({"uri": "influence-sets.bin", "byteLength": payload.len()})];
    let mut sparse_values = Vec::new();
    if sets == 3 {
        buffers.push(json!({"uri": "influence-indices.bin", "byteLength": 2}));
        buffers.push(json!({"uri": "influence-values.bin", "byteLength": 32}));
        for vertex in [1, 4] {
            for weight in &weights[vertex][8..12] {
                sparse_values.extend_from_slice(&weight.to_le_bytes());
            }
        }
    }
    let mut nodes = vec![
        json!({"name": "first", "mesh": 0, "skin": 0}),
        json!({"name": "second", "mesh": 0, "skin": 1, "translation": [4.0, 2.0, -2.0]}),
    ];
    nodes.extend((0..13).map(|joint| json!({"name": format!("joint-{joint}")})));
    nodes[2]["children"] = json!((3..15).collect::<Vec<_>>());
    let document = json!({
        "asset": {"version": "2.0"},
        "scene": 0, "scenes": [{"nodes": [0, 1, 2]}], "nodes": nodes,
        "skins": [
            {"name": "palette-a", "skeleton": 2, "joints": PALETTES[0]},
            {"name": "palette-b", "skeleton": 2, "joints": PALETTES[1]}
        ],
        "meshes": [{"primitives": [{"attributes": attributes, "indices": indices, "mode": 4}]}],
        "buffers": buffers, "bufferViews": views, "accessors": accessors
    });
    let io = MemoryAssetIo::new();
    io.try_insert(
        AssetPath::new("influence-sets.gltf")?,
        Arc::from(serde_json::to_vec(&document)?),
    )?;
    io.try_insert(AssetPath::new("influence-sets.bin")?, Arc::from(payload))?;
    if sets == 3 {
        io.try_insert(
            AssetPath::new("influence-indices.bin")?,
            Arc::from([1_u8, 4]),
        )?;
        io.try_insert(
            AssetPath::new("influence-values.bin")?,
            Arc::from(sparse_values),
        )?;
    }
    Ok(io)
}

fn assert_row(row: InfluenceRowRef<'_>, source: usize, weights: &[f32]) -> Result<()> {
    assert_eq!(usize::try_from(row.vertex())?, source);
    assert_eq!(row.len(), weights.len());
    assert_eq!(
        row.joint_indices(),
        &JOINTS[source].map(u32::from)[..weights.len()]
    );
    assert_eq!(
        row.weights()
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        weights
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        row.source_orders(),
        (0..u32::try_from(weights.len())?).collect::<Vec<_>>()
    );
    for (lane, influence) in row.iter().enumerate() {
        assert_eq!(influence.palette_joint(), u32::from(JOINTS[source][lane]));
        assert_eq!(influence.weight().to_bits(), weights[lane].to_bits());
        assert_eq!(influence.source_order(), u32::try_from(lane)?);
    }
    assert!(row.get(weights.len()).is_none());
    Ok(())
}

fn assert_scene(
    scene: &ValidatedScene,
    sets: usize,
    weights: &[[f32; 12]; 5],
    indices: &[u32; 9],
) -> Result<()> {
    assert_eq!(
        (
            scene.mesh_count(),
            scene.primitive_count(),
            scene.mesh_instance_count(),
            scene.skin_count()
        ),
        (1, 1, 2, 2)
    );
    let mesh = scene.meshes().next().ok_or("missing shared mesh")?;
    let primitive = mesh.primitives().next().ok_or("missing primitive")?;
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(
        (
            primitive.vertex_count(),
            primitive.corner_count(),
            primitive.face_count()
        ),
        (5, 9, 3)
    );
    assert_eq!(primitive.indices(), Some(indices.as_slice()));
    let rows = primitive
        .influences()
        .ok_or("missing canonical influence rows")?;
    assert_eq!(rows.len(), POSITIONS.len());
    for (source, row) in rows.iter().enumerate() {
        assert_row(row, source, &weights[source][..sets * 4])?;
    }
    assert!(rows.get(POSITIONS.len()).is_none());
    let tuples = list_vertices(primitive)?;
    assert_eq!(tuples.len(), indices.len());
    for (tuple, &source) in tuples.vertices().zip(indices) {
        assert_eq!(tuple.source_vertex().primitive, primitive.id());
        assert_eq!(tuple.source_vertex().index, source);
        let source = usize::try_from(source)?;
        assert_row(
            tuple.influences().ok_or("missing tuple influences")?,
            source,
            &weights[source][..sets * 4],
        )?;
    }
    for (index, name) in ["first", "second"].into_iter().enumerate() {
        let instance = scene
            .mesh_instances()
            .find(|instance| instance.node().name() == Some(name))
            .ok_or("missing shared-mesh instance")?;
        assert_eq!(instance.mesh().id(), mesh.id());
        assert_eq!(
            instance.skin().ok_or("missing instance skin")?.name(),
            Some(["palette-a", "palette-b"][index])
        );
    }

    // Traverse only this primitive's ancestry. Sparse resources are used exclusively by WEIGHTS_2.
    let mut pending: Vec<_> = scene
        .owner_provenance(primitive.metadata_owner())?
        .collect();
    let mut visited = Vec::new();
    let mut resources = Vec::new();
    while let Some(node) = pending.pop() {
        if visited.contains(&node.id()) {
            continue;
        }
        visited.push(node.id());
        assert!(visited.len() <= scene.provenance_node_count());
        if let ProvenanceKindRef::Source(source) = node.kind() {
            resources.push(source.resource().identity());
        }
        pending.extend(node.parents());
    }
    assert!(resources.contains(&"influence-sets.bin"));
    if sets == 3 {
        assert!(resources.contains(&"influence-indices.bin"));
        assert!(resources.contains(&"influence-values.bin"));
    }
    Ok(())
}

fn assert_owned_asset(
    asset: &ApplicationAsset,
    sets: usize,
    weights: &[[f32; 12]; 5],
    converted: bool,
) -> Result<()> {
    assert_eq!(
        (
            asset.mesh_count,
            asset.draws.len(),
            asset.instances.len(),
            asset.skins.len()
        ),
        (1, 1, 2, 2)
    );
    let draw = &asset.draws[0];
    assert_eq!((draw.mesh, draw.primitive, draw.material), (0, 0, None));
    assert_eq!(draw.topology, TopologyKind::TriangleList);
    assert_eq!(draw.vertices.len(), POSITIONS.len());
    let (sources, packed, seam) = if converted {
        (CONVERTED_INDICES, [0, 1, 2, 0, 3, 1, 3, 0, 4], [2, 4])
    } else {
        (INDICES, [0, 1, 2, 0, 2, 3, 3, 4, 0], [1, 4])
    };
    assert_eq!(draw.indices, packed);
    assert_eq!(
        draw.vertices[seam[0]].position,
        draw.vertices[seam[1]].position
    );
    assert_ne!(draw.vertices[seam[0]].uv, draw.vertices[seam[1]].uv);
    let rows = draw
        .influences
        .as_ref()
        .ok_or("missing owned influence rows")?;
    let width = sets * 4;
    assert_eq!(
        rows.offsets,
        (0..=draw.vertices.len())
            .map(|vertex| vertex * width)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        (rows.joints.len(), rows.weights.len()),
        (draw.vertices.len() * width, draw.vertices.len() * width)
    );

    // Reconstruct every corner through application indices, rather than assuming source order.
    // ApplicationAsset stores order in its parallel arrays; source_orders was checked on rows.
    for (&index, source) in draw.indices.iter().zip(sources) {
        let index = usize::try_from(index)?;
        let source = usize::try_from(source)?;
        let [x, y, z] = POSITIONS[source].map(f64::from);
        let expected = if converted {
            [x * 2.0, y * 2.0, -z * 2.0]
        } else {
            [x, y, z]
        };
        assert_eq!(draw.vertices[index].position, expected);
        assert_eq!(draw.vertices[index].uv, Some(UVS[source].map(f64::from)));
        let range = rows.offsets[index]..rows.offsets[index + 1];
        assert_eq!(
            &rows.joints[range.clone()],
            &JOINTS[source].map(u32::from)[..width]
        );
        assert_eq!(
            rows.weights[range]
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            weights[source][..width]
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
    }
    for (palette, name) in ["first", "second"].into_iter().enumerate() {
        let instance = asset
            .instances
            .iter()
            .find(|instance| asset.nodes[instance.node].name.as_deref() == Some(name))
            .ok_or("missing owned instance")?;
        assert_eq!((instance.mesh, instance.skin), (0, Some(palette)));
        let skin = &asset.skins[palette];
        assert_eq!(skin.joints.len(), 13);
        assert_eq!(skin.inverse_binds, [Mat4::IDENTITY; 13]);
        // Resolve each reconstructed influence through this instance's palette after Scene/IO drop.
        for (&index, source) in draw.indices.iter().zip(sources) {
            let index = usize::try_from(index)?;
            let source = usize::try_from(source)?;
            let range = rows.offsets[index]..rows.offsets[index + 1];
            for (lane, &joint) in rows.joints[range].iter().enumerate() {
                let node = skin.joints[usize::try_from(joint)?];
                let expected = format!(
                    "joint-{}",
                    PALETTES[palette][usize::from(JOINTS[source][lane])] - 2
                );
                assert_eq!(asset.nodes[node].name.as_deref(), Some(expected.as_str()));
            }
        }
    }
    Ok(())
}

fn exercise_influence_sets(sets: usize) -> Result<()> {
    let mut weights = WEIGHTS;
    if sets == 2 {
        // Author a separate valid eight-lane fixture, also summing exactly to one.
        weights[1][7] = 0.4375;
        weights[4][7] = 0.46875;
    }
    for (joints, row) in JOINTS.iter().zip(&weights) {
        let row = &row[..sets * 4];
        assert!(row.iter().filter(|&&weight| weight > 0.0).count() > 4);
        assert_eq!(row.iter().sum::<f32>(), 1.0);
        for lane in 0..row.len() {
            assert_ne!(joints[lane], 0);
            assert!(!joints[..lane].contains(&joints[lane]));
        }
    }
    let io = influence_sets_io(sets, &weights)?;
    let report = Importer::new()?.read_asset(&io, AssetPath::new("influence-sets.gltf")?)?;
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    let (scene, evidence) = report.into_parts();
    assert_scene(&scene, sets, &weights, &INDICES)?;
    let original = ApplicationAsset::from_scene(&scene)?;
    let generation = scene.generation();
    let (scene, conversion) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: TargetSpace::try_new(
                Handedness::Left,
                Axis::PositiveY,
                Axis::NegativeZ,
                0.5,
                FrontFaceWinding::CounterClockwise,
            )?,
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    assert_eq!(conversion.disposition(), PostProcessRunDisposition::Changed);
    assert_ne!(scene.generation(), generation);
    assert!(conversion.matches_scene(&scene));
    assert!(
        conversion
            .space_conversion_effects()
            .ok_or("missing conversion effects")?
            .reverses_triangle_winding()
    );
    assert_scene(&scene, sets, &weights, &CONVERTED_INDICES)?;
    let converted = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    drop(io);
    drop(evidence);
    drop(conversion);

    assert_owned_asset(&original, sets, &weights, false)?;
    assert_owned_asset(&converted, sets, &weights, true)?;
    Ok(())
}

#[test]
fn two_contiguous_sets_preserve_complete_owned_rows_through_space_conversion() -> Result<()> {
    exercise_influence_sets(2)
}

#[test]
fn sparse_third_set_preserves_complete_owned_rows_and_external_provenance() -> Result<()> {
    exercise_influence_sets(3)
}
