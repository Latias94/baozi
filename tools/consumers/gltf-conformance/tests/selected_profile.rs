use baozi_core::{
    AttributeDomain, AttributeElement, AttributeRef, AttributeSemanticRef, CameraProjection,
    LightKindCategory, MetadataValueRef, ShadingModel, TextureRole, ValidatedScene,
};
use baozi_format_gltf::{FORMAT_ID, GltfFailureCode, GltfSceneSelection};
use baozi_gltf_conformance_consumer::{
    compile, import_embedded, import_from_io, register, selected_fixtures,
};
use baozi_import::{
    DependencyPolicyError, ImportCause, ImportDimension, ImportEvidence, ImportPhase,
    ImportRunError, ScopeAbortCause, SourceScopeDisposition, SourceScopeEntityKind,
    SourceScopeSelectionRequest,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::error::Error;
use std::sync::Arc;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

const POSITIONS: [[f32; 3]; 3] = [[-0.0, 1.25, -2.5], [-3.0, 4.0, 5.0], [-7.25, 8.5, 9.75]];
const INDICES: [u32; 3] = [2, 0, 1];
// Independent rational source values are these numerators divided by 255.
const COLOR_NUMERATORS: [[u8; 4]; 3] = [[0, 64, 128, 255], [255, 0, 127, 255], [3, 200, 17, 255]];
const COLOR_RATIONAL_NEAREST_F32: [[u32; 4]; 3] = [
    [0, 0x3e80_8081, 0x3f00_8081, 0x3f80_0000],
    [0x3f80_0000, 0, 0x3efe_feff, 0x3f80_0000],
    [0x3c40_c0c1, 0x3f48_c8c9, 0x3d88_8889, 0x3f80_0000],
];
// The pinned reference's observed F32 reciprocal products differ from the exact rational.
// They characterize that implementation, not Baozi's single-rounding contract.
const GLTF_RS_1_4_1_COLOR_BITS_OBSERVATION: [[u32; 4]; 3] = [
    [0, 0x3e80_8081, 0x3f00_8081, 0x3f80_0000],
    [0x3f80_0000, 0, 0x3efe_ff00, 0x3f80_0000],
    [0x3c40_c0c2, 0x3f48_c8ca, 0x3d88_8889, 0x3f80_0000],
];
const UVS: [[u32; 2]; 3] = [
    [0, 0x3f80_0000],
    [0x3e80_8081, 0x3f00_8081],
    [0x3f80_0000, 0],
];
// Original 1x1 RGBA PNG, pixel [23, 41, 79, 255], with PNG chunk CRCs.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 16, 215, 244, 255, 15, 0, 2,
    121, 1, 143, 216, 95, 165, 94, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn fixture(name: &str) -> Vec<u8> {
    selected_fixtures()
        .into_iter()
        .find_map(|(path, bytes)| (path == name).then_some(bytes))
        .expect("known fixed fixture")
}

fn attribute_bits<const N: usize>(attribute: AttributeRef<'_>) -> Vec<[u32; N]> {
    assert_eq!(usize::from(attribute.lanes()), N);
    (0..attribute.len())
        .map(|index| {
            let Some(AttributeElement::F32(values)) = attribute.element(index) else {
                panic!("selected profile requires retained F32 attribute values");
            };
            let lanes: [f32; N] = values.try_into().expect("validated attribute lane count");
            lanes.map(f32::to_bits)
        })
        .collect()
}

#[derive(Debug)]
struct GeometryFacts {
    positions: Vec<[u32; 3]>,
    colors: Vec<[u32; 4]>,
    uvs: Vec<[u32; 2]>,
    indices: Vec<u32>,
    material: String,
}

fn geometry_facts(scene: &ValidatedScene) -> Vec<GeometryFacts> {
    scene
        .meshes()
        .flat_map(|mesh| mesh.primitives())
        .map(|primitive| GeometryFacts {
            positions: attribute_bits(primitive.positions().expect("POSITION")),
            colors: attribute_bits(
                primitive
                    .attribute(AttributeSemanticRef::Color(0), AttributeDomain::Vertex)
                    .expect("COLOR_0"),
            ),
            uvs: attribute_bits(
                primitive
                    .texcoords(0, AttributeDomain::Vertex)
                    .expect("TEXCOORD_0"),
            ),
            indices: primitive.indices().expect("authored indices").to_vec(),
            material: primitive
                .authored_material()
                .expect("authored material")
                .name()
                .expect("material name")
                .to_owned(),
        })
        .collect()
}

fn assert_baozi_geometry(facts: &[GeometryFacts]) {
    assert_eq!(facts.len(), 2);
    for (primitive, material) in facts.iter().zip(["Radiant", "Ink"]) {
        assert_eq!(
            primitive.positions,
            POSITIONS.map(|row| row.map(f32::to_bits))
        );
        assert_eq!(primitive.colors, COLOR_RATIONAL_NEAREST_F32);
        assert_u8_normalization_observation("Baozi", &primitive.colors, [[0; 4]; 3]);
        assert_eq!(primitive.uvs, UVS);
        assert_eq!(primitive.indices, INDICES);
        assert_eq!(primitive.material, material);
    }
}

fn assert_u8_normalization_observation(
    engine: &str,
    observed: &[[u32; 4]],
    deviations: [[u32; 4]; 3],
) {
    // This records exact deviations; it does not authorize a one-ULP acceptance tolerance.
    assert_eq!(observed.len(), COLOR_NUMERATORS.len(), "{engine}");
    for (row, actual) in observed.iter().enumerate() {
        for (lane, &bits) in actual.iter().enumerate() {
            let rational = f64::from(COLOR_NUMERATORS[row][lane]) / 255.0;
            let nearest = COLOR_RATIONAL_NEAREST_F32[row][lane];
            assert_eq!(
                (rational as f32).to_bits(),
                nearest,
                "independent n/255 baseline"
            );
            assert_eq!(
                bits.abs_diff(nearest),
                deviations[row][lane],
                "{engine}: source {}/255, observed {:#010x}, nearest {nearest:#010x}",
                COLOR_NUMERATORS[row][lane],
                bits
            );
        }
    }
}

fn assert_reference_geometry(reference: &gltf::Gltf, binary: &[u8]) {
    // Inspect the known source mesh directly; this is not a reference scene importer/resolver.
    let mesh = reference.meshes().next().expect("source mesh zero");
    assert_eq!(mesh.name(), Some("Selected mesh"));
    assert_eq!(mesh.primitives().len(), 2);
    for (primitive, name) in mesh.primitives().zip(["Radiant", "Ink"]) {
        let reader = primitive.reader(|buffer| (buffer.index() == 0).then_some(binary));
        assert_eq!(
            reader
                .read_positions()
                .expect("positions")
                .map(|row| row.map(f32::to_bits))
                .collect::<Vec<_>>(),
            POSITIONS.map(|row| row.map(f32::to_bits))
        );
        assert_eq!(
            reader
                .read_indices()
                .expect("indices")
                .into_u32()
                .collect::<Vec<_>>(),
            INDICES
        );
        let colors = reader
            .read_colors(0)
            .expect("colors")
            .into_rgba_f32()
            .map(|row| row.map(f32::to_bits))
            .collect::<Vec<_>>();
        assert_eq!(colors, GLTF_RS_1_4_1_COLOR_BITS_OBSERVATION);
        assert_u8_normalization_observation(
            "gltf-rs 1.4.1",
            &colors,
            [[0, 0, 0, 0], [0, 0, 1, 0], [1, 1, 0, 0]],
        );
        assert_eq!(
            reader
                .read_tex_coords(0)
                .expect("UVs")
                .into_f32()
                .map(|row| row.map(f32::to_bits))
                .collect::<Vec<_>>(),
            UVS
        );
        assert_eq!(primitive.material().name(), Some(name));
    }
}

fn assert_selected_evidence(evidence: &ImportEvidence) {
    assert_eq!(evidence.format().id().as_str(), FORMAT_ID);
    assert!(evidence.diagnostics().is_empty());
    assert!(evidence.loss_evidence().is_empty());
    assert!(evidence.optional_dependency_evidence().is_empty());
    let scope = evidence
        .source_scope_record()
        .expect("source selection evidence");
    assert_eq!(
        scope.selection_request(),
        &SourceScopeSelectionRequest::DocumentDefault
    );
    assert_eq!(scope.selection_outcome().scene_index(), Some(0));
    for (kind, selected, omitted) in [
        (SourceScopeEntityKind::Scene, 1, 1),
        (SourceScopeEntityKind::Node, 7, 1),
        (SourceScopeEntityKind::Mesh, 1, 1),
        (SourceScopeEntityKind::Buffer, 1, 1),
        (SourceScopeEntityKind::Camera, 3, 0),
        (SourceScopeEntityKind::Light, 1, 1),
    ] {
        let counts = scope.counts(kind);
        assert_eq!(
            (counts.selected(), counts.omitted()),
            (selected, omitted),
            "{kind:?}"
        );
    }
    let light_sites = scope
        .details()
        .filter(|site| site.kind() == SourceScopeEntityKind::Light)
        .map(|site| (site.identity().to_owned(), site.disposition()))
        .collect::<Vec<_>>();
    assert_eq!(
        light_sites,
        [
            ("l0".to_owned(), SourceScopeDisposition::Selected),
            ("l1".to_owned(), SourceScopeDisposition::Omitted)
        ]
    );
}

#[test]
fn original_shared_profile_has_literal_geometry_through_three_resource_paths() -> Result {
    for container in ["glb", "data-uri", "external"] {
        let binary = fixture("selected.bin");
        let bytes = match container {
            "glb" => fixture("selected.glb"),
            "data-uri" => fixture("selected-data.gltf"),
            _ => fixture("selected-external.gltf"),
        };
        let reference = gltf::Gltf::from_slice(&bytes)?;
        if container == "glb" {
            let bin = reference.blob.as_deref().expect("BIN chunk");
            assert_eq!(bin.len(), 92);
            assert_eq!(&bin[..89], binary);
            assert_eq!(&bin[89..], &[0, 0, 0]);
        }
        assert_reference_geometry(&reference, &binary);

        let registry = register()?;
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry
                .descriptors()
                .next()
                .expect("glTF plugin")
                .id()
                .as_str(),
            FORMAT_ID
        );
        let request = compile(&registry, GltfSceneSelection::DocumentDefault)?;
        let io = MemoryAssetIo::new();
        let source = AssetPath::new(if container == "glb" {
            "probes/selected.glb"
        } else {
            "probes/selected.gltf"
        })?;
        let observation = if container == "external" {
            io.try_insert(source.clone(), Arc::from(bytes.clone()))?;
            io.try_insert(
                AssetPath::new("probes/selected.bin")?,
                Arc::from(binary.clone()),
            )?;
            // omitted.bin deliberately does not exist in this authority.
            import_from_io(&request, &io, source, geometry_facts)?
        } else {
            import_embedded(&request, source, &bytes, geometry_facts)?
        };
        drop((request, registry, io, bytes, binary, reference));
        assert_baozi_geometry(&observation.facts);
        assert_selected_evidence(&observation.evidence);
        let opens = observation
            .evidence
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .expect("asset-open dimension")
            .consumed();
        assert_eq!(
            opens,
            if container == "external" { 2 } else { 0 },
            "{container}"
        );
    }
    Ok(())
}

#[test]
fn default_and_explicit_selection_retain_source_identity_and_shared_attachments() -> Result {
    let bytes = fixture("selected-data.gltf");
    let reference = gltf::Gltf::from_slice(&bytes)?;
    assert_eq!(reference.scenes().len(), 2);
    let selected = reference.default_scene().expect("declared default");
    assert_eq!(selected.index(), 0);
    assert_eq!(
        selected
            .nodes()
            .map(|node| node.index())
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 7]
    );
    let nodes = selected.nodes().collect::<Vec<_>>();
    assert_eq!(nodes[0].mesh().expect("first instance").index(), 0);
    assert_eq!(nodes[1].mesh().expect("second instance").index(), 0);
    assert_eq!(nodes[1].transform().matrix()[3], [2.0, 0.0, 0.0, 1.0]);
    assert_eq!(nodes[3].light().expect("spot A").index(), 0);
    assert_eq!(nodes[4].light().expect("spot B").index(), 0);
    assert_eq!(
        nodes[0].extras().as_ref().expect("source extras").get(),
        r#"{"purpose": "owned-consumer", "ordinal": 7}"#
    );

    for selection in [
        GltfSceneSelection::DocumentDefault,
        GltfSceneSelection::SceneIndex(0),
    ] {
        let registry = register()?;
        let request = compile(&registry, selection)?;
        let observation = import_embedded(
            &request,
            AssetPath::new("probes/selection.gltf")?,
            &bytes,
            |scene| {
                let mesh = scene.meshes().next().expect("selected mesh");
                let lights = scene
                    .nodes()
                    .filter_map(|node| node.light().map(|light| light.id()))
                    .collect::<Vec<_>>();
                let shared_light = (lights.len(), lights.first() == lights.get(1));
                let extras = scene
                    .metadata_entries()
                    .filter(|entry| entry.key() == "org.baozi.gltf:extras")
                    .map(|entry| match entry.value() {
                        MetadataValueRef::Bytes {
                            media_type,
                            payload,
                        } => {
                            assert_eq!(media_type, "application/json");
                            payload.to_vec()
                        }
                        value => panic!("expected descriptive JSON bytes, got {value:?}"),
                    })
                    .collect::<Vec<_>>();
                (
                    mesh.name().map(str::to_owned),
                    scene
                        .mesh_instances()
                        .map(|instance| instance.node().name().map(str::to_owned))
                        .collect::<Vec<_>>(),
                    extras,
                    shared_light,
                )
            },
        )?;
        drop((request, registry));
        assert_eq!(observation.facts.0.as_deref(), Some("Selected mesh"));
        assert_eq!(
            observation.facts.1,
            [Some("Original".to_owned()), Some("Instance".to_owned())]
        );
        assert_eq!(
            observation.facts.2,
            [br#"{"purpose": "owned-consumer", "ordinal": 7}"#.to_vec()]
        );
        assert_eq!(observation.facts.3, (2, true));
        let scope = observation
            .evidence
            .source_scope_record()
            .expect("source scope");
        assert_eq!(scope.selection_outcome().scene_index(), Some(0));
        assert_eq!(
            scope.selection_request(),
            &match selection {
                GltfSceneSelection::DocumentDefault => SourceScopeSelectionRequest::DocumentDefault,
                GltfSceneSelection::SceneIndex(index) =>
                    SourceScopeSelectionRequest::ExplicitScene {
                        scene_index: u64::from(index)
                    },
            }
        );
    }
    Ok(())
}

#[test]
fn emission_unlit_uv_transform_and_encoded_image_have_independent_source_expectations() -> Result {
    let bytes = fixture("selected-data.gltf");
    let reference = gltf::Gltf::from_slice(&bytes)?;
    let radiant = reference.materials().next().expect("Radiant");
    assert!(!radiant.unlit());
    assert_eq!(radiant.emissive_factor(), [0.125, 0.5, 0.75]);
    assert_eq!(radiant.emissive_strength(), Some(8.0));
    let texture = radiant
        .pbr_metallic_roughness()
        .base_color_texture()
        .expect("base color texture");
    assert_eq!(texture.tex_coord(), 1);
    let transform = texture.texture_transform().expect("typed UV transform");
    assert_eq!(transform.tex_coord(), Some(0));
    assert_eq!(transform.offset(), [0.25, -0.5]);
    assert_eq!(transform.scale(), [2.0, 0.5]);
    assert_eq!(transform.rotation().to_bits(), 0.0_f32.to_bits());
    let ink = reference.materials().nth(1).expect("Ink");
    assert!(ink.unlit());
    assert_eq!(
        ink.pbr_metallic_roughness().base_color_factor(),
        [0.75, 0.25, 0.125, 0.5]
    );
    assert_eq!(ink.emissive_factor(), [0.0; 3]);
    assert_eq!(ink.emissive_strength(), None);

    let registry = register()?;
    let request = compile(&registry, GltfSceneSelection::DocumentDefault)?;
    let observation = import_embedded(
        &request,
        AssetPath::new("probes/material.gltf")?,
        &bytes,
        |scene| {
            let radiant = scene
                .materials()
                .find(|material| material.name() == Some("Radiant"))
                .expect("Radiant");
            let ink = scene
                .materials()
                .find(|material| material.name() == Some("Ink"))
                .expect("Ink");
            let binding = radiant
                .bindings()
                .find(|binding| binding.role() == TextureRole::BaseColor)
                .expect("base color binding");
            let transform = binding.transform();
            let emission = radiant.factors().emission();
            let ink_pbr = ink
                .factors()
                .pbr_metallic_roughness()
                .expect("unlit base color");
            (
                emission,
                radiant.shading_model(),
                ink.shading_model(),
                ink_pbr.base_color(),
                binding.uv_set(),
                [transform.offset().x, transform.offset().y],
                [transform.scale().x, transform.scale().y],
                transform.rotation_radians(),
                binding.texture().image().encoded_bytes().to_vec(),
                binding.texture().image().media_type().map(str::to_owned),
                ink.factors().emission(),
            )
        },
    )?;
    drop((request, registry, reference, bytes));
    let (
        emission,
        radiant,
        ink,
        color,
        uv_set,
        offset,
        scale,
        rotation,
        image,
        media,
        ink_emission,
    ) = observation.facts;
    let factor = emission.factor();
    assert_eq!([factor.x, factor.y, factor.z], [0.125, 0.5, 0.75]);
    assert_eq!(emission.strength(), Some(8.0));
    assert_eq!(emission.effective_rgb(), [1.0, 4.0, 6.0]);
    assert_eq!(radiant, ShadingModel::PbrMetallicRoughness);
    assert_eq!(ink, ShadingModel::Unlit);
    assert_eq!(
        [color.r, color.g, color.b, color.a],
        [0.75, 0.25, 0.125, 0.5]
    );
    assert_eq!(uv_set, 0);
    assert_eq!(offset, [0.25, -0.5]);
    assert_eq!(scale, [2.0, 0.5]);
    assert_eq!(rotation.to_bits(), 0.0_f32.to_bits());
    assert_eq!(image, PNG);
    assert_eq!(media.as_deref(), Some("image/png"));
    assert_eq!(ink_emission.strength(), None);
    assert_eq!(ink_emission.effective_rgb(), [0.0; 3]);
    assert_selected_evidence(&observation.evidence);
    Ok(())
}

#[test]
fn cameras_and_punctual_lights_keep_source_units_and_distinct_canonical_emission() -> Result {
    let bytes = fixture("selected-data.gltf");
    let reference = gltf::Gltf::from_slice(&bytes)?;
    let cameras = reference.cameras().collect::<Vec<_>>();
    assert_eq!(cameras.len(), 3);
    let gltf::camera::Projection::Perspective(perspective) = cameras[0].projection() else {
        panic!("perspective");
    };
    assert_eq!(
        (
            perspective.yfov(),
            perspective.znear(),
            perspective.zfar(),
            perspective.aspect_ratio()
        ),
        (0.75, 0.125, Some(96.0), Some(1.5))
    );
    let gltf::camera::Projection::Orthographic(orthographic) = cameras[1].projection() else {
        panic!("orthographic");
    };
    assert_eq!(
        (
            orthographic.xmag(),
            orthographic.ymag(),
            orthographic.znear(),
            orthographic.zfar()
        ),
        (2.0, 3.0, 0.0, 32.0)
    );
    let gltf::camera::Projection::Perspective(infinite) = cameras[2].projection() else {
        panic!("perspective");
    };
    assert_eq!(
        (
            infinite.yfov(),
            infinite.znear(),
            infinite.zfar(),
            infinite.aspect_ratio()
        ),
        (1.0, 0.25, None, None)
    );
    let lights = reference
        .lights()
        .expect("root light inventory")
        .collect::<Vec<_>>();
    assert_eq!(lights.len(), 2);
    assert_eq!(lights[0].color(), [1.0, 0.0, 0.0]);
    assert_eq!(lights[0].intensity(), 10000.0);
    assert_eq!(lights[0].range(), Some(8.0));
    let gltf::khr_lights_punctual::Kind::Spot {
        inner_cone_angle,
        outer_cone_angle,
    } = lights[0].kind()
    else {
        panic!("spot");
    };
    assert_eq!((inner_cone_angle, outer_cone_angle), (0.125, 0.5));
    assert_eq!(lights[1].color(), [1.0; 3]);
    assert_eq!(lights[1].intensity(), 1.0);

    let registry = register()?;
    let request = compile(&registry, GltfSceneSelection::DocumentDefault)?;
    let observation = import_embedded(
        &request,
        AssetPath::new("probes/spatial.gltf")?,
        &bytes,
        |scene| {
            (
                scene
                    .cameras()
                    .map(|camera| camera.projection())
                    .collect::<Vec<_>>(),
                scene
                    .lights()
                    .map(|light| light.definition())
                    .collect::<Vec<_>>(),
            )
        },
    )?;
    drop((request, registry, reference, bytes));
    let (cameras, lights) = observation.facts;
    assert_eq!(cameras.len(), 3);
    let CameraProjection::Perspective(perspective) = cameras[0] else {
        panic!("perspective");
    };
    assert_eq!(
        (
            perspective.vertical_fov_radians(),
            perspective.near(),
            perspective.far(),
            perspective.aspect_ratio()
        ),
        (0.75, 0.125, Some(96.0), Some(1.5))
    );
    let CameraProjection::Orthographic(orthographic) = cameras[1] else {
        panic!("orthographic");
    };
    assert_eq!(
        (
            orthographic.horizontal_magnification(),
            orthographic.vertical_magnification(),
            orthographic.near(),
            orthographic.far()
        ),
        (2.0, 3.0, 0.0, 32.0)
    );
    let CameraProjection::Perspective(infinite) = cameras[2] else {
        panic!("perspective");
    };
    assert_eq!(
        (
            infinite.vertical_fov_radians(),
            infinite.near(),
            infinite.far(),
            infinite.aspect_ratio()
        ),
        (1.0, 0.25, None, None)
    );
    assert_eq!(lights.len(), 1);
    assert_eq!(lights[0].kind().category(), LightKindCategory::Spot);
    assert_eq!(lights[0].kind().range(), Some(8.0));
    assert_eq!(lights[0].kind().inner_cone_radians(), Some(0.125));
    assert_eq!(lights[0].kind().outer_cone_radians(), Some(0.5));
    // Source red 10000 cd has Rec.709 luminance 0.2126 * 10000 = 2126 cd.
    // Canonical tint is chromaticity scaled to unit luminance, not source RGB.
    assert_eq!(lights[0].intensity().to_bits(), 2126.0_f32.to_bits());
    let tint = lights[0].tint();
    assert_eq!(
        [tint.x, tint.y, tint.z].map(f32::to_bits),
        [(10000.0_f64 / 2126.0) as f32, 0.0, 0.0].map(f32::to_bits)
    );
    assert_selected_evidence(&observation.evidence);
    Ok(())
}

fn assert_format_failure(error: &ImportRunError, code: GltfFailureCode) {
    let failure = error.operation_failure().expect("typed operation failure");
    let ImportCause::Format(envelope) = failure.cause() else {
        panic!("expected format failure: {failure:?}");
    };
    assert_eq!(envelope.format().as_str(), FORMAT_ID);
    assert_eq!(envelope.code(), code.as_str());
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
}

#[test]
fn baozi_requires_a_choice_for_multiple_scenes_without_default() -> Result {
    // Valid source; this is a Baozi policy outcome, not an invalid-glTF assertion.
    let bytes = fixture("no-default.gltf");
    let registry = register()?;
    let request = compile(&registry, GltfSceneSelection::DocumentDefault)?;
    let error = import_embedded(
        &request,
        AssetPath::new("probes/no-default.gltf")?,
        &bytes,
        geometry_facts,
    )
    .expect_err("explicit choice required");
    assert_format_failure(&error, GltfFailureCode::SceneSelectionRequired);
    let explicit = compile(&registry, GltfSceneSelection::SceneIndex(0))?;
    let observation = import_embedded(
        &explicit,
        AssetPath::new("probes/no-default.gltf")?,
        &bytes,
        geometry_facts,
    )?;
    assert_baozi_geometry(&observation.facts);
    Ok(())
}

#[test]
fn reference_exposes_both_scenes_without_inventing_a_default() -> Result {
    let reference = gltf::Gltf::from_slice(&fixture("no-default.gltf"))?;
    assert!(reference.default_scene().is_none());
    assert_eq!(
        reference
            .scenes()
            .map(|scene| (scene.index(), scene.name()))
            .collect::<Vec<_>>(),
        [(0, Some("Chosen")), (1, Some("Other"))]
    );
    Ok(())
}

#[test]
fn baozi_typed_negatives_do_not_invoke_hazardous_reference_readers() -> Result {
    let registry = register()?;
    let request = compile(&registry, GltfSceneSelection::DocumentDefault)?;
    let cases: &[(&[u8], GltfFailureCode)] = &[
        (br#"{"asset":{"version":"2.0"},"nodes":[{"camera":0}],"scene":0,"scenes":[{"nodes":[0]}],"cameras":[{"type":"perspective"}]}"#, GltfFailureCode::InvalidDocument),
        (br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[9]}]}"#, GltfFailureCode::InvalidDocument),
        (br#"{"asset":{"version":"2.0"},"extensionsUsed":["VENDOR_required"],"extensionsRequired":["VENDOR_required"],"extensions":{"VENDOR_required":{}},"scene":0,"scenes":[{"nodes":[]}]}"#, GltfFailureCode::UnsupportedFeature),
    ];
    for &(bytes, code) in cases {
        let error = import_embedded(
            &request,
            AssetPath::new("probes/rejected.gltf")?,
            bytes,
            |_| (),
        )
        .expect_err("typed rejection");
        assert_format_failure(&error, code);
    }
    Ok(())
}

#[test]
fn external_buffer_requires_asset_authority_and_explicit_selection_can_open_other_scene() -> Result
{
    let bytes = fixture("selected-external.gltf");
    let registry = register()?;
    let default = compile(&registry, GltfSceneSelection::DocumentDefault)?;
    let denied = import_embedded(
        &default,
        AssetPath::new("probes/external.gltf")?,
        &bytes,
        geometry_facts,
    )
    .expect_err("direct bytes have no external authority");
    assert!(
        matches!(denied.operation_failure().expect("operation failure").cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::DependencyPolicy(DependencyPolicyError::Denied { .. })))
    );

    let other = compile(&registry, GltfSceneSelection::SceneIndex(1))?;
    let io = MemoryAssetIo::new();
    io.try_insert(
        AssetPath::new("probes/external.gltf")?,
        Arc::from(bytes.clone()),
    )?;
    let binary = fixture("omitted.bin");
    io.try_insert(
        AssetPath::new("probes/omitted.bin")?,
        Arc::from(binary.clone()),
    )?;
    let observation = import_from_io(
        &other,
        &io,
        AssetPath::new("probes/external.gltf")?,
        |scene| {
            let mesh = scene.meshes().next().expect("other mesh");
            (
                mesh.name().map(str::to_owned),
                attribute_bits::<3>(
                    mesh.primitives()
                        .next()
                        .expect("other primitive")
                        .positions()
                        .expect("positions"),
                ),
            )
        },
    )?;
    drop((other, default, registry, io));
    assert_eq!(observation.facts.0.as_deref(), Some("Other mesh"));
    assert_eq!(
        observation.facts.1,
        [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]].map(|row| row.map(f32::to_bits))
    );
    let scope = observation
        .evidence
        .source_scope_record()
        .expect("source scope");
    assert_eq!(
        scope.selection_request(),
        &SourceScopeSelectionRequest::ExplicitScene { scene_index: 1 }
    );
    assert_eq!(scope.selection_outcome().scene_index(), Some(1));
    assert_eq!(scope.counts(SourceScopeEntityKind::Light).selected(), 0);
    assert_eq!(scope.counts(SourceScopeEntityKind::Light).omitted(), 2);
    let paths = observation
        .evidence
        .successful_input_manifest()
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"probes/omitted.bin"));
    assert!(!paths.contains(&"probes/selected.bin"));

    let reference = gltf::Gltf::from_slice(&bytes)?;
    let scene = reference.scenes().nth(1).expect("source scene one");
    assert_eq!(
        scene.nodes().map(|node| node.index()).collect::<Vec<_>>(),
        [6]
    );
    let mesh = scene
        .nodes()
        .next()
        .expect("other node")
        .mesh()
        .expect("other mesh");
    assert_eq!(mesh.name(), Some("Other mesh"));
    let primitive = mesh.primitives().next().expect("other primitive");
    let reader = primitive.reader(|buffer| (buffer.index() == 1).then_some(binary.as_slice()));
    assert_eq!(
        reader
            .read_positions()
            .expect("other positions")
            .collect::<Vec<_>>(),
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
    );
    Ok(())
}

#[test]
#[ignore = "exports the exact original shared-profile bytes for a separate official Validator run"]
fn export_validator_fixtures() -> Result {
    let directory = std::path::PathBuf::from(
        std::env::var_os("BAOZI_GLTF_FIXTURE_DIR")
            .ok_or("set BAOZI_GLTF_FIXTURE_DIR to an output directory")?,
    );
    std::fs::create_dir_all(&directory)?;
    for (name, bytes) in selected_fixtures() {
        std::fs::write(directory.join(name), bytes)?;
    }
    Ok(())
}
