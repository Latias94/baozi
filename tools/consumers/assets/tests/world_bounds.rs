use std::{path::PathBuf, sync::Arc};

use baozi::{
    Aabb, AssetPath, CancellationToken, Importer, MemoryAssetIo, SceneEntityKind, SceneLookupError,
    SpatialQueryError, SpatialQueryLimit, SpatialQueryLimits, TargetSpace, ValidatedScene,
    postprocess::{
        PostProcessControl, PostProcessRunDisposition, SpaceConversionOptions, convert_scene_space,
    },
};
use baozi_asset_consumer::{ApplicationAsset, BasePoseSpatialSnapshot, Result};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};
use serde_json::json;

fn limits() -> SpatialQueryLimits {
    SpatialQueryLimits::new(100_000, 1_048_576)
}

fn shared_static_scene() -> Result<ValidatedScene> {
    // This independently authored glTF has two instances of the same mesh under a transformed
    // ancestor. Vertex 3 is deliberately outside the indexed triangle.
    let positions: [[f32; 3]; 4] = [
        [-1.0, -2.0, 0.0],
        [2.0, 0.0, 1.0],
        [0.0, 3.0, -1.0],
        [4.0, -5.0, 2.0],
    ];
    let mut payload = Vec::new();
    for value in positions.into_iter().flatten() {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for index in [0_u16, 1, 2] {
        payload.extend_from_slice(&index.to_le_bytes());
    }
    assert_eq!(payload.len(), 54);
    let document = serde_json::to_vec(&json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [
            {"name": "ancestor", "children": [1, 2],
             "translation": [10.0, 20.0, 30.0], "scale": [2.0, 3.0, -1.0]},
            {"name": "first", "mesh": 0,
             "translation": [1.0, -2.0, 4.0], "scale": [-1.0, 2.0, 1.0]},
            {"name": "second", "mesh": 0,
             "matrix": [1.0, 0.0, 0.0, 0.0,
                        1.0, 1.0, 0.0, 0.0,
                        0.0, 0.0, -2.0, 0.0,
                        -3.0, 2.0, -1.0, 1.0]}
        ],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "buffers": [{"uri": "world-bounds.bin", "byteLength": payload.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48},
            {"buffer": 0, "byteOffset": 48, "byteLength": 6}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
             "min": [-1.0, -5.0, -1.0], "max": [4.0, 3.0, 2.0]},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ]
    }))?;
    let source = AssetPath::new("world-bounds.gltf")?;
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::from(document))?;
    io.try_insert(AssetPath::new("world-bounds.bin")?, Arc::from(payload))?;
    let report = Importer::new()?.read_asset(&io, source)?;
    assert!(report.loss_evidence().is_empty());
    Ok(report.into_parts().0)
}

fn upstream_scene(model: &str) -> Result<ValidatedScene> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let io = FileSystemAssetIo::from_capability(NativeDirectoryCapability::open_ambient(root)?);
    let report = Importer::new()?.read_asset(
        &io,
        AssetPath::new(format!("upstream/Models/{model}/glTF/{model}.gltf"))?,
    )?;
    assert!(report.loss_evidence().is_empty());
    Ok(report.into_parts().0)
}

#[test]
fn shared_instance_world_bounds_and_transforms_survive_scene_destruction() -> Result<()> {
    let scene = shared_static_scene()?;
    assert_eq!(scene.node_count(), 3);
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.mesh_instance_count(), 2);
    let generation = scene.generation();
    let root = scene.roots().next().ok_or("missing ancestor")?.id();
    let first = scene
        .mesh_instances()
        .find(|i| i.node().name() == Some("first"))
        .ok_or("missing first instance")?;
    let second = scene
        .mesh_instances()
        .find(|i| i.node().name() == Some("second"))
        .ok_or("missing second instance")?;
    assert_eq!(first.mesh().id(), second.mesh().id());
    let first_node = first.node().id();
    let second_node = second.node().id();
    let snapshot = BasePoseSpatialSnapshot::from_scene(&scene, limits(), None)?;
    let first_bounds = snapshot
        .world_transforms
        .instance_bounds(first, limits(), None)?;
    let second_bounds = snapshot
        .world_transforms
        .instance_bounds(second, limits(), None)?;
    let asset = ApplicationAsset::from_scene(&scene)?;
    // The draw layout copies referenced vertices only; the spatial query includes all four.
    assert!(asset.draws.iter().all(|draw| draw.vertices.len() == 3));
    drop(scene);

    assert_eq!(snapshot.world_transforms.generation(), generation);
    assert_eq!(snapshot.world_transforms.node_count(), 3);
    assert_eq!(
        snapshot.world_transforms.node_transform(root)?,
        [
            [2.0, 0.0, 0.0, 0.0],
            [0.0, 3.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, 0.0],
            [10.0, 20.0, 30.0, 1.0],
        ]
    );
    // parent_world * local gives (12 - 2x, 14 + 6y, 26 - z).
    assert_eq!(
        snapshot.world_transforms.node_transform(first_node)?,
        [
            [-2.0, 0.0, 0.0, 0.0],
            [0.0, 6.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, 0.0],
            [12.0, 14.0, 26.0, 1.0],
        ]
    );
    assert_eq!(
        first_bounds,
        Some(Aabb {
            min: [4.0, -16.0, 24.0],
            max: [14.0, 32.0, 27.0],
        })
    );
    // The authored shear gives (4 + 2x + 2y, 26 + 3y, 31 + 2z). Transforming a local AABB's
    // corners would incorrectly widen X to [-8, 18]; actual vertices give [-2, 10].
    assert_eq!(
        snapshot.world_transforms.node_transform(second_node)?,
        [
            [2.0, 0.0, 0.0, 0.0],
            [2.0, 3.0, 0.0, 0.0],
            [0.0, 0.0, 2.0, 0.0],
            [4.0, 26.0, 31.0, 1.0],
        ]
    );
    assert_eq!(
        second_bounds,
        Some(Aabb {
            min: [-2.0, 11.0, 29.0],
            max: [10.0, 35.0, 35.0],
        })
    );
    assert_eq!(
        snapshot.scene_bounds,
        Some(Aabb {
            min: [-2.0, -16.0, 24.0],
            max: [14.0, 35.0, 35.0],
        })
    );
    assert_eq!(asset.instances.len(), 2);
    Ok(())
}

#[test]
fn coordinate_conversion_requires_a_fresh_world_transform_table() -> Result<()> {
    let scene = shared_static_scene()?;
    let old_node = scene.roots().next().ok_or("missing ancestor")?.id();
    let old_generation = scene.generation();
    let old_table = scene.base_pose_world_transforms(limits(), None)?;
    let old_transform = old_table.node_transform(old_node)?;
    let (scene, converted) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: TargetSpace::STANDARD,
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    assert_eq!(converted.disposition(), PostProcessRunDisposition::Changed);
    assert!(converted.matches_scene(&scene));
    let new_generation = scene.generation();
    assert_ne!(new_generation, old_generation);
    let new_node = scene
        .roots()
        .next()
        .ok_or("missing converted ancestor")?
        .id();
    let mismatch = SceneLookupError::GenerationMismatch {
        kind: SceneEntityKind::Node,
        expected: old_generation,
        actual: new_generation,
    };
    assert_eq!(old_table.node_transform(new_node), Err(mismatch));
    assert_eq!(
        old_table.scene_bounds(&scene, limits(), None),
        Err(SpatialQueryError::Lookup(mismatch))
    );
    assert_eq!(old_table.node_transform(old_node)?, old_transform);

    let fresh_table = scene.base_pose_world_transforms(limits(), None)?;
    assert_eq!(fresh_table.generation(), new_generation);
    assert_eq!(
        fresh_table.node_transform(old_node),
        Err(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Node,
            expected: new_generation,
            actual: old_generation,
        })
    );
    // glTF +Z front to the right-handed -Z standard transports (x, y, z) to (-x, y, -z).
    // Apply those signs to the original [-2, -16, 24] .. [14, 35, 35] world bounds.
    assert_eq!(
        fresh_table.scene_bounds(&scene, limits(), None)?,
        Some(Aabb {
            min: [-14.0, -16.0, -35.0],
            max: [2.0, 35.0, -24.0],
        })
    );
    Ok(())
}

#[test]
fn ply_double_positions_and_world_bounds_survive_application_copy_and_scene_drop() -> Result<()> {
    let source = AssetPath::new("double-points.ply")?;
    let bytes = b"ply\nformat ascii 1.0\nelement vertex 2\nproperty double x\nproperty double y\nproperty double z\nend_header\n16777216 -0 5e-324\n16777217 0 1e-100\n";
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::<[u8]>::from(bytes.as_slice()))?;
    let (scene, evidence) = Importer::new()?.read_asset(&io, source)?.into_parts();
    assert!(evidence.loss_evidence().is_empty());
    let spatial = BasePoseSpatialSnapshot::from_scene(&scene, limits(), None)?;
    let asset = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    drop(evidence);

    assert_eq!(asset.draws.len(), 1);
    let vertices = &asset.draws[0].vertices;
    assert_eq!(vertices.len(), 2);
    let expected = [
        [16_777_216.0, -0.0, f64::from_bits(1)],
        [16_777_217.0, 0.0, 1e-100],
    ];
    for (vertex, expected) in vertices.iter().zip(expected) {
        assert_eq!(
            vertex.position.map(f64::to_bits),
            expected.map(f64::to_bits)
        );
    }
    assert_eq!(
        spatial.scene_bounds,
        Some(Aabb {
            min: [16_777_216.0, 0.0, f64::from_bits(1)],
            max: [16_777_217.0, 0.0, 1e-100],
        })
    );
    Ok(())
}

#[test]
fn deformed_upstream_assets_keep_buffer_extraction_and_reject_base_vertex_bounds() -> Result<()> {
    for model in ["RiggedSimple", "MorphPrimitivesTest"] {
        let scene = upstream_scene(model)?;
        let instance = scene
            .mesh_instances()
            .next()
            .ok_or("missing deformed instance")?;
        let expected_error = if model == "RiggedSimple" {
            assert!(instance.skin().is_some());
            assert!(scene.animation_clip_count() > 0);
            SpatialQueryError::SkinUnsupported {
                instance: instance.id(),
            }
        } else {
            assert!(
                instance
                    .morph_weights()
                    .unwrap_or_else(|| instance.mesh().default_morph_weights())
                    .iter()
                    .any(|weight| *weight != 0.0)
            );
            SpatialQueryError::NonzeroMorphWeight {
                instance: instance.id(),
                target: 0,
            }
        };
        // Computing authored node transforms does not evaluate geometry deformation.
        let transforms = scene.base_pose_world_transforms(limits(), None)?;
        let node = instance.node().id();
        assert!(
            transforms
                .node_transform(node)?
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        );
        assert_eq!(
            transforms
                .instance_bounds(instance, limits(), None)
                .unwrap_err(),
            expected_error
        );
        assert_eq!(
            transforms.scene_bounds(&scene, limits(), None).unwrap_err(),
            expected_error
        );
        assert_eq!(
            BasePoseSpatialSnapshot::from_scene(&scene, limits(), None).unwrap_err(),
            expected_error
        );
        let asset = ApplicationAsset::from_scene(&scene)?;
        drop(scene);
        assert!(
            transforms
                .node_transform(node)?
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        );
        assert!(!asset.draws.is_empty());
        if model == "RiggedSimple" {
            assert!(
                asset
                    .instances
                    .iter()
                    .any(|instance| instance.skin.is_some())
            );
            assert!(!asset.animations.is_empty());
        } else {
            assert!(asset.draws.iter().any(|draw| !draw.morphs.is_empty()));
        }
    }
    Ok(())
}

#[test]
fn animated_zero_weight_base_pose_can_be_queried_without_sampling() -> Result<()> {
    let scene = upstream_scene("AnimatedMorphCube")?;
    let asset = ApplicationAsset::from_scene(&scene)?;
    assert!(!asset.animations.is_empty());
    assert!(
        asset
            .instances
            .iter()
            .flat_map(|instance| {
                instance
                    .morph_weights
                    .as_deref()
                    .unwrap_or(&asset.mesh_morph_weights[instance.mesh])
            })
            .all(|weight| *weight == 0.0)
    );
    let snapshot = BasePoseSpatialSnapshot::from_scene(&scene, limits(), None)?;
    let generation = scene.generation();
    let node = scene.nodes().next().ok_or("missing animated node")?.id();
    drop(scene);
    assert_eq!(snapshot.world_transforms.generation(), generation);
    assert!(
        snapshot
            .world_transforms
            .node_transform(node)?
            .iter()
            .flatten()
            .all(|value| value.is_finite())
    );
    let bounds = snapshot.scene_bounds.ok_or("missing base-pose bounds")?;
    assert!(bounds.min.into_iter().chain(bounds.max).all(f64::is_finite));
    assert!(!asset.draws[0].morphs.is_empty());
    Ok(())
}

#[test]
fn opt_in_snapshot_preserves_query_budget_and_cancellation_errors() -> Result<()> {
    let scene = shared_static_scene()?;
    for (limits, expected_limit) in [
        (
            SpatialQueryLimits::new(0, 1_048_576),
            SpatialQueryLimit::Work,
        ),
        (
            SpatialQueryLimits::new(100_000, 0),
            SpatialQueryLimit::Bytes,
        ),
    ] {
        let exhausted = BasePoseSpatialSnapshot::from_scene(&scene, limits, None).unwrap_err();
        assert!(matches!(
            exhausted,
            SpatialQueryError::LimitExceeded { limit, .. } if limit == expected_limit
        ));
    }
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        BasePoseSpatialSnapshot::from_scene(&scene, limits(), Some(&cancellation)).unwrap_err(),
        SpatialQueryError::Cancelled
    );
    assert_eq!(ApplicationAsset::from_scene(&scene)?.instances.len(), 2);
    Ok(())
}
