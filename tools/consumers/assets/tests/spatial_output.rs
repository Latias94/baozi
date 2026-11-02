use std::sync::Arc;

use baozi::{
    AssetPath, Axis, CameraProjection, FrontFaceWinding, Handedness, Importer, LightKindCategory,
    LocalAxes, MemoryAssetIo, SceneSpaceRef, TargetSpace, UvOrigin,
    postprocess::{PostProcessControl, SpaceConversionOptions, convert_scene_space},
};
use baozi_asset_consumer::{
    ApplicationAsset, Result, SampleTime, UvExtractionError, UvExtractionErrorKind, evaluate,
};
use serde_json::{Value, json};

fn document() -> Value {
    json!({
        "asset": {"version": "2.0"},
        "extensionsUsed": ["KHR_lights_punctual"],
        "extensions": {"KHR_lights_punctual": {"lights": [
            {"name": "omitted-light", "type": "point"},
            {"type": "directional"},
            {"name": "", "type": "point"},
            {"name": "key", "type": "spot", "color": [1, 0, 0], "intensity": 10000,
             "range": 8, "spot": {"innerConeAngle": 0.125, "outerConeAngle": 0.5},
             "extras": {"tag": "spot"}}
        ]}},
        "cameras": [
            {"name": "omitted-camera", "type": "perspective",
             "perspective": {"yfov": 0.75, "znear": 0.25}},
            {"name": "view", "type": "perspective",
             "perspective": {"yfov": 1, "aspectRatio": 1.5, "znear": 0.125, "zfar": 64}},
            {"name": "", "type": "orthographic",
             "orthographic": {"xmag": 2, "ymag": 3, "znear": 0, "zfar": 32}}
        ],
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [
            {"name": "parent", "translation": [10, 20, 30], "scale": [2, 3, 4],
             "children": [1, 2, 3, 4]},
            {"name": "mixed", "translation": [1, 2, 3], "scale": [5, 6, 7], "camera": 1,
             "extensions": {"KHR_lights_punctual": {"light": 3}}},
            {"name": "shared", "translation": [-1, 0, 0], "camera": 1,
             "extensions": {"KHR_lights_punctual": {"light": 3}}},
            {"name": "sun", "extensions": {"KHR_lights_punctual": {"light": 1}}},
            {"name": "point", "camera": 2,
             "extensions": {"KHR_lights_punctual": {"light": 2}}}
        ]
    })
}

fn add_geometry(document: &mut Value) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    document["buffers"] = json!([{"uri": "spatial.bin", "byteLength": 36}]);
    document["bufferViews"] = json!([{"buffer": 0, "byteLength": 36}]);
    document["accessors"] = json!([
        {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
         "min": [0, 0, 0], "max": [1, 1, 0]}
    ]);
    document["meshes"] = json!([{"primitives": [{"attributes": {"POSITION": 0}}]}]);
    document["nodes"][1]["mesh"] = json!(0);
    document["nodes"][2]["mesh"] = json!(0);
    payload
}

fn memory_io(document: &Value, payload: &[u8]) -> Result<MemoryAssetIo> {
    let io = MemoryAssetIo::new();
    io.try_insert(
        AssetPath::new("spatial.gltf")?,
        Arc::from(serde_json::to_vec(document)?),
    )?;
    if !payload.is_empty() {
        io.try_insert(AssetPath::new("spatial.bin")?, Arc::from(payload))?;
    }
    Ok(io)
}

fn copy_document(
    document: Value,
    payload: Vec<u8>,
    target: Option<TargetSpace>,
) -> Result<ApplicationAsset> {
    let io = memory_io(&document, &payload)?;
    let importer = Importer::new()?;
    let report = importer.read_asset(&io, AssetPath::new("spatial.gltf")?)?;
    assert!(report.loss_evidence().is_empty());
    let (mut scene, evidence) = report.into_parts();
    if let Some(target) = target {
        let (converted, report) = convert_scene_space(
            scene,
            SpaceConversionOptions {
                target,
                source_assumptions: None,
            },
            PostProcessControl::default(),
        )?
        .into_parts();
        scene = converted;
        drop(report);
    }
    let asset = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    drop(evidence);
    drop(importer);
    drop(io);
    Ok(asset)
}

fn node_index(asset: &ApplicationAsset, name: &str) -> usize {
    asset
        .nodes
        .iter()
        .position(|node| node.name.as_deref() == Some(name))
        .expect("missing named application node")
}

fn assert_sharing(asset: &ApplicationAsset) {
    assert_eq!(asset.nodes.len(), 5);
    assert_eq!((asset.cameras.len(), asset.lights.len()), (2, 3));
    let parent = node_index(asset, "parent");
    let mixed = node_index(asset, "mixed");
    let shared = node_index(asset, "shared");
    let sun = node_index(asset, "sun");
    let point = node_index(asset, "point");
    assert_eq!(asset.roots, [parent]);
    assert_eq!(asset.nodes[parent].children, [mixed, shared, sun, point]);
    assert_eq!(
        (asset.nodes[parent].camera, asset.nodes[parent].light),
        (None, None)
    );
    // Source camera 1 and light 3 become dense application rows 0 and 2.
    assert_eq!(
        (asset.nodes[mixed].camera, asset.nodes[mixed].light),
        (Some(0), Some(2))
    );
    assert_eq!(
        (asset.nodes[shared].camera, asset.nodes[shared].light),
        (Some(0), Some(2))
    );
    assert_eq!(
        (asset.nodes[sun].camera, asset.nodes[sun].light),
        (None, Some(0))
    );
    assert_eq!(
        (asset.nodes[point].camera, asset.nodes[point].light),
        (Some(1), Some(1))
    );
    assert_eq!(asset.cameras[0].name.as_deref(), Some("view"));
    assert_eq!(asset.cameras[1].name.as_deref(), Some(""));
    assert_eq!(asset.lights[0].name, None);
    assert_eq!(asset.lights[1].name.as_deref(), Some(""));
    assert_eq!(asset.lights[2].name.as_deref(), Some("key"));
}

fn assert_light_parameters(asset: &ApplicationAsset, range: f32) {
    for (light, category) in asset.lights.iter().zip([
        LightKindCategory::Directional,
        LightKindCategory::Point,
        LightKindCategory::Spot,
    ]) {
        let definition = light.definition;
        assert_eq!(definition.kind().category(), category);
        let tint = definition.tint();
        if category == LightKindCategory::Spot {
            // Rec.709 red luminance is 0.2126: 10000 red candela -> 2126 canonical candela.
            // The expected tint is derived from the source product, not a Baozi constructor.
            assert_eq!(definition.intensity().to_bits(), 2126.0_f32.to_bits());
            assert_eq!(
                [tint.x, tint.y, tint.z].map(f32::to_bits),
                [(10000.0_f64 / 2126.0) as f32, 0.0, 0.0].map(f32::to_bits)
            );
            assert_eq!(definition.kind().range(), Some(range));
            assert_eq!(definition.kind().inner_cone_radians(), Some(0.125));
            assert_eq!(definition.kind().outer_cone_radians(), Some(0.5));
        } else {
            assert_eq!([tint.x, tint.y, tint.z], [1.0; 3]);
            assert_eq!(definition.intensity(), 1.0);
            assert_eq!(definition.kind().range(), None);
            assert_eq!(definition.kind().inner_cone_radians(), None);
            assert_eq!(definition.kind().outer_cone_radians(), None);
        }
    }
}

fn assert_cameras(asset: &ApplicationAsset, scale: f32) -> Result<()> {
    let CameraProjection::Perspective(perspective) = asset.cameras[0].projection else {
        return Err("expected perspective camera".into());
    };
    assert_eq!(perspective.vertical_fov_radians(), 1.0);
    assert_eq!(perspective.aspect_ratio(), Some(1.5));
    assert_eq!(perspective.near(), 0.125 * scale);
    assert_eq!(perspective.far(), Some(64.0 * scale));
    let CameraProjection::Orthographic(orthographic) = asset.cameras[1].projection else {
        return Err("expected orthographic camera".into());
    };
    assert_eq!(orthographic.horizontal_magnification(), 2.0 * scale);
    assert_eq!(orthographic.vertical_magnification(), 3.0 * scale);
    assert_eq!(orthographic.near().to_bits(), 0.0_f32.to_bits());
    assert_eq!(orthographic.far(), 32.0 * scale);
    Ok(())
}

#[test]
fn direct_obj_output_retains_unknown_space_after_source_destruction() -> Result<()> {
    let asset = {
        let io = MemoryAssetIo::new();
        io.try_insert(
            AssetPath::new("unknown.obj")?,
            Arc::from(&b"v 0 0 0\nv 2 0 0\nv 0 3 0\nf 1 2 3\n"[..]),
        )?;
        let importer = Importer::new()?;
        let report = importer.read_asset(&io, AssetPath::new("unknown.obj")?)?;
        ApplicationAsset::from_scene(report.scene())?
    };
    assert_eq!(
        asset.space,
        SceneSpaceRef {
            handedness: Handedness::Unknown,
            up_axis: Axis::Unknown,
            front_axis: Axis::Unknown,
            unit_scale_to_meters: None,
            front_face_winding: FrontFaceWinding::Unknown,
            uv_origin: UvOrigin::Unknown,
        }
    );
    assert_eq!(asset.mesh_count, 1);
    assert_eq!(asset.draws.len(), 1);
    assert!(asset.cameras.is_empty());
    assert!(asset.lights.is_empty());
    Ok(())
}

#[test]
fn camera_only_output_keeps_absent_name_optional_planes_and_shared_definition() -> Result<()> {
    let asset = copy_document(
        json!({
            "asset": {"version": "2.0"},
            "cameras": [{"type": "perspective", "perspective": {"yfov": 0.75, "znear": 0.25}}],
            "scene": 0,
            "scenes": [{"nodes": [0, 1]}],
            "nodes": [{"camera": 0}, {"camera": 0, "translation": [1, 2, 3]}]
        }),
        Vec::new(),
        None,
    )?;
    assert_eq!(
        (asset.mesh_count, asset.draws.len(), asset.instances.len()),
        (0, 0, 0)
    );
    assert!(asset.lights.is_empty());
    assert_eq!(asset.cameras.len(), 1);
    assert_eq!(asset.cameras[0].name, None);
    let CameraProjection::Perspective(projection) = asset.cameras[0].projection else {
        return Err("expected perspective camera".into());
    };
    assert_eq!(projection.axes(), LocalAxes::STANDARD);
    assert_eq!(projection.vertical_fov_radians(), 0.75);
    assert_eq!(projection.aspect_ratio(), None);
    assert_eq!(projection.near(), 0.25);
    assert_eq!(projection.far(), None);
    assert_eq!(asset.nodes.len(), 2);
    assert!(
        asset
            .nodes
            .iter()
            .all(|node| node.camera == Some(0) && node.light.is_none())
    );
    let evaluated = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    assert_eq!(evaluated.world_transforms[1][3], [1.0, 2.0, 3.0, 1.0]);
    Ok(())
}

#[test]
fn camera_and_light_only_output_owns_selected_definitions_names_and_metadata() -> Result<()> {
    let asset = copy_document(document(), Vec::new(), None)?;
    assert_eq!(
        (asset.mesh_count, asset.draws.len(), asset.instances.len()),
        (0, 0, 0)
    );
    assert!(asset.materials.is_empty() && asset.images.is_empty());
    assert_sharing(&asset);
    assert_light_parameters(&asset, 8.0);
    assert_cameras(&asset, 1.0)?;
    assert_eq!(
        asset.space,
        SceneSpaceRef {
            handedness: Handedness::Right,
            up_axis: Axis::PositiveY,
            front_axis: Axis::PositiveZ,
            unit_scale_to_meters: Some(1.0),
            front_face_winding: FrontFaceWinding::CounterClockwise,
            uv_origin: UvOrigin::UpperLeft,
        }
    );
    for light in &asset.lights {
        assert_eq!(light.definition.axes(), LocalAxes::STANDARD);
    }
    for camera in &asset.cameras {
        assert_eq!(camera.projection.axes(), LocalAxes::STANDARD);
    }
    let extras = asset
        .source_json_metadata
        .iter()
        .find(|entry| {
            entry.key == "org.baozi.gltf:extras"
                && entry.locator.contains("kind=light;parent=-;index=3;")
        })
        .ok_or("missing selected source light extras")?;
    assert_eq!(extras.resource, "spatial.gltf");
    assert_eq!(extras.format, "org.baozi.gltf");
    assert_eq!(extras.media_type, "application/json");
    assert_eq!(extras.payload, br#"{"tag":"spot"}"#);
    let evaluated = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    assert!(evaluated.draws.is_empty());
    assert_eq!(
        evaluated.world_transforms[node_index(&asset, "mixed")][3],
        [12.0, 26.0, 42.0, 1.0]
    );
    Ok(())
}

#[test]
fn geometry_camera_and_light_coexist_and_parent_scale_moves_only_the_placement() -> Result<()> {
    for (parent_scale, child_scale, origin) in [
        ([1, 1, 1], [1, 1, 1], [11.0, 22.0, 33.0, 1.0]),
        ([2, 3, 4], [1, 1, 1], [12.0, 26.0, 42.0, 1.0]),
        ([2, 3, 4], [5, 6, 7], [12.0, 26.0, 42.0, 1.0]),
    ] {
        let mut document = document();
        document["nodes"][0]["scale"] = json!(parent_scale);
        document["nodes"][1]["scale"] = json!(child_scale);
        let payload = add_geometry(&mut document);
        let asset = copy_document(document, payload, None)?;
        assert_sharing(&asset);
        assert_light_parameters(&asset, 8.0);
        assert_cameras(&asset, 1.0)?;
        assert_eq!(
            (asset.mesh_count, asset.draws.len(), asset.instances.len()),
            (1, 1, 2)
        );
        let mixed = node_index(&asset, "mixed");
        let shared = node_index(&asset, "shared");
        for node in [mixed, shared] {
            let instance = asset
                .instances
                .iter()
                .find(|instance| instance.node == node)
                .ok_or("missing mesh placement on camera/light node")?;
            assert_eq!(instance.mesh, 0);
            assert!(asset.nodes[node].camera.is_some() && asset.nodes[node].light.is_some());
        }
        let evaluated = evaluate(&asset, None, SampleTime::Declared(0.0))?;
        assert_eq!(evaluated.world_transforms[mixed][3], origin);
        assert_eq!(
            evaluated.draws[0].world_positions[0],
            [origin[0], origin[1], origin[2]]
        );
    }
    Ok(())
}

#[test]
fn converted_spatial_output_owns_reflected_frames_and_centimeter_distances() -> Result<()> {
    let mut document = document();
    let payload = add_geometry(&mut document);
    let asset = copy_document(
        document,
        payload,
        Some(TargetSpace::try_new(
            Handedness::Left,
            Axis::PositiveY,
            Axis::NegativeZ,
            0.01,
            FrontFaceWinding::CounterClockwise,
        )?),
    )?;
    assert_sharing(&asset);
    assert_light_parameters(&asset, 800.0);
    assert_cameras(&asset, 100.0)?;
    assert_eq!(
        asset.space,
        SceneSpaceRef {
            handedness: Handedness::Left,
            up_axis: Axis::PositiveY,
            front_axis: Axis::NegativeZ,
            unit_scale_to_meters: Some(0.01),
            front_face_winding: FrontFaceWinding::CounterClockwise,
            uv_origin: UvOrigin::UpperLeft,
        }
    );
    for axes in asset
        .cameras
        .iter()
        .map(|camera| camera.projection.axes())
        .chain(asset.lights.iter().map(|light| light.definition.axes()))
    {
        assert_eq!(
            (axes.right(), axes.up(), axes.forward()),
            (Axis::PositiveX, Axis::PositiveY, Axis::PositiveZ)
        );
    }
    let evaluated = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    assert_eq!(
        evaluated.world_transforms[node_index(&asset, "mixed")][3],
        [1200.0, 2600.0, -4200.0, 1.0]
    );
    assert_eq!(
        evaluated.world_transforms[node_index(&asset, "shared")][3],
        [800.0, 2000.0, -3000.0, 1.0]
    );
    assert_eq!(asset.instances.len(), 2);
    assert_eq!(evaluated.draws.len(), 2);
    let extras = asset
        .source_json_metadata
        .iter()
        .find(|entry| {
            entry.key == "org.baozi.gltf:extras"
                && entry.locator.contains("kind=light;parent=-;index=3;")
        })
        .ok_or("missing converted source light extras")?;
    assert_eq!(extras.payload, br#"{"tag":"spot"}"#);
    Ok(())
}

#[test]
fn detached_direction_uses_local_negative_z_under_a_rigid_y_rotation() -> Result<()> {
    let asset = copy_document(
        json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_lights_punctual"],
            "extensions": {"KHR_lights_punctual": {"lights": [{"type": "directional"}]}},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"rotation": [0, std::f64::consts::FRAC_1_SQRT_2, 0, std::f64::consts::FRAC_1_SQRT_2],
                       "extensions": {"KHR_lights_punctual": {"light": 0}}}]
        }),
        Vec::new(),
        None,
    )?;
    assert_eq!(asset.nodes[0].light, Some(0));
    assert_eq!(asset.lights[0].definition.axes().forward(), Axis::NegativeZ);
    let evaluated = evaluate(&asset, None, SampleTime::Declared(0.0))?;
    // A rigid +90 degree Y rotation maps the reference -Z direction to -X.
    for (actual, expected) in evaluated.world_transforms[0][2][..3]
        .iter()
        .map(|value| -value)
        .zip([-1.0, 0.0, 0.0])
    {
        assert!((actual - expected).abs() < 1.0e-7);
    }
    Ok(())
}

#[test]
fn spatial_records_do_not_publish_partial_output_when_material_uv_extraction_fails() -> Result<()> {
    let mut document = document();
    let payload = add_geometry(&mut document);
    document["images"] = json!([{"uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jC1QAAAAASUVORK5CYII="}]);
    document["textures"] = json!([{"source": 0}]);
    document["materials"] = json!([{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}]);
    document["meshes"][0]["primitives"][0]["material"] = json!(0);
    let io = memory_io(&document, &payload)?;
    let importer = Importer::new()?;
    let report = importer.read_asset(&io, AssetPath::new("spatial.gltf")?)?;
    let scene = report.scene();
    let generation = scene.generation();
    let error = ApplicationAsset::from_scene(scene).unwrap_err();
    assert_eq!(
        error.downcast_ref::<UvExtractionError>(),
        Some(&UvExtractionError {
            mesh: 0,
            primitive: 0,
            binding: Some(0),
            uv_set: 0,
            kind: UvExtractionErrorKind::Missing,
        })
    );
    assert_eq!(scene.generation(), generation);
    assert_eq!(
        (
            scene.node_count(),
            scene.camera_count(),
            scene.light_count()
        ),
        (5, 2, 3)
    );
    let mixed = scene
        .nodes()
        .find(|node| node.name() == Some("mixed"))
        .ok_or("missing unchanged mixed node")?;
    assert_eq!(
        mixed
            .light()
            .ok_or("missing unchanged light")?
            .definition()
            .kind()
            .range(),
        Some(8.0)
    );
    assert!(mixed.camera().is_some());
    Ok(())
}
