#![allow(dead_code)]
use baozi::*;
use baozi_asset_consumer::*;

pub fn asset(vertices: usize) -> ApplicationAsset {
    let vertices: Vec<Vertex> = (0..vertices)
        .map(|i| Vertex {
            position: match i % 3 {
                0 => [-0.75, -0.75, 0.0],
                1 => [0.75, -0.75, 0.0],
                _ => [0.0, 0.75, 0.0],
            },
            normal: Some([0.0, 0.0, 1.0]),
            tangent: None,
            color: None,
        })
        .collect();
    let mut indices: Vec<_> = (0..vertices.len() as u32).collect();
    while indices.len() % 3 != 0 {
        indices.push(0);
    }
    ApplicationAsset {
        mesh_count: 1,
        mesh_morph_weights: vec![vec![]],
        roots: vec![0],
        nodes: vec![Node {
            name: None,
            transform: AuthoredTransform::identity_trs(),
            children: vec![],
            camera: None,
            light: None,
        }],
        cameras: vec![],
        lights: vec![],
        draws: vec![DrawBuffer {
            mesh: 0,
            primitive: 0,
            material: MaterialAssignment::Authored(0),
            topology: TopologyKind::TriangleList,
            vertices,
            indices,
            uv_streams: vec![],
            tangent_conventions: None,
            influences: None,
            morphs: vec![],
        }],
        instances: vec![Instance {
            node: 0,
            mesh: 0,
            skin: None,
            morph_weights: None,
        }],
        skins: vec![],
        animations: vec![],
        materials: vec![Material {
            name: None,
            shading_model: ShadingModel::Unlit,
            alpha_mode: AlphaMode::Opaque,
            double_sided: false,
            factors: MaterialFactors::empty().with_pbr_metallic_roughness(Some(
                PbrMetallicRoughnessFactors::try_new(
                    Color {
                        r: 0.25,
                        g: 0.5,
                        b: 0.75,
                        a: 1.0,
                    },
                    0.0,
                    1.0,
                )
                .unwrap(),
            )),
            bindings: vec![],
        }],
        textures: vec![],
        samplers: vec![],
        images: vec![],
        space: SceneSpaceRef {
            handedness: Handedness::Right,
            up_axis: Axis::PositiveY,
            front_axis: Axis::NegativeZ,
            unit_scale_to_meters: Some(1.0),
            front_face_winding: FrontFaceWinding::CounterClockwise,
            uv_origin: UvOrigin::UpperLeft,
        },
        source_json_metadata: vec![],
    }
}
pub fn evaluated(asset: &ApplicationAsset) -> EvaluatedAsset {
    evaluate(
        asset,
        None,
        SampleTime::Declared(0.0),
        EvaluationRequest {
            frames: FramePolicy::FrozenWeight,
            limits: EvaluationLimits::default(),
            cancellation: None,
        },
    )
    .unwrap()
}
pub fn camera() -> baozi_render_wgpu::spatial::CameraInput {
    let mut world = baozi_render_wgpu::spatial::IDENTITY;
    world[3][2] = 3.0;
    baozi_render_wgpu::spatial::CameraInput::Explicit {
        projection: CameraProjection::orthographic(LocalAxes::STANDARD, 1.0, 1.0, 1.0, 10.0)
            .unwrap(),
        world,
    }
}

pub fn renderer() -> baozi_render_wgpu::renderer::Renderer {
    use baozi_render_wgpu::renderer::{AdapterSelector, Backend, Renderer};
    let backend = match std::env::var("BAOZI_GPU_BACKEND")
        .expect("explicit BAOZI_GPU_BACKEND required")
        .as_str()
    {
        "dx12" => Backend::Dx12,
        "vulkan" => Backend::Vulkan,
        _ => panic!("expected dx12 or vulkan"),
    };
    Renderer::new(&AdapterSelector {
        backend,
        name: std::env::var("BAOZI_GPU_ADAPTER").expect("exact adapter name required"),
        vendor: std::env::var("BAOZI_GPU_VENDOR").unwrap().parse().unwrap(),
        device: std::env::var("BAOZI_GPU_DEVICE").unwrap().parse().unwrap(),
    })
    .expect("named adapter must exist; missing adapter is a failure")
}

pub fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}

pub fn binding(
    role: TextureRole,
    texture: usize,
    set: u16,
    swizzle: TextureChannelSwizzle,
    transfer: ColorInterpretation,
) -> Binding {
    Binding {
        role,
        texture,
        uv_set: set,
        channel_mapping: swizzle,
        color_interpretation: transfer,
        transform: TextureTransform::IDENTITY,
        strength: TextureStrength::try_new(1.0).unwrap(),
        blend: TextureBlendOperation::Replace,
        source_key: None,
    }
}

pub fn image(asset: &mut ApplicationAsset, rgba: &[u8], width: u32, height: u32) -> usize {
    let image = asset.images.len();
    asset.images.push(Image {
        name: None,
        encoded_bytes: png(width, height, rgba),
        media_type: Some("image/png".into()),
    });
    let sampler = asset.samplers.len();
    asset.samplers.push(Sampler {
        name: None,
        state: SamplerState::new(
            TexelFilter::Nearest,
            TexelFilter::Nearest,
            MipmapFilter::None,
            TextureAddressMode::Repeat,
            TextureAddressMode::Repeat,
            TextureAddressMode::Repeat,
        ),
    });
    let index = asset.textures.len();
    asset.textures.push(Texture {
        name: None,
        image,
        sampler,
    });
    index
}
