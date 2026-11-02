//! Native, named-adapter headless upload and finite-deadline readback.

use crate::device::{DevicePlan, IndexWidth, ScalarError, ScalarLocation};
use crate::images::{ImageLimits, PreparedImages, sampler_descriptor};
use crate::material::{MATERIAL_BYTES, SLOT_COUNT, linear_to_srgb, sheen_albedo, uniform_scalar};
use crate::spatial::{self, CameraInput, LightFrame};
use crate::{Control, Error, Result, add, cap, f32_at, mul, u32_at, vec_with_capacity};
use baozi::{AlphaMode, ColorInterpretation, FrontFaceWinding};
use half::f16;
use std::{
    num::NonZeroU64,
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Dx12,
    Vulkan,
}
impl Backend {
    pub fn mask(self) -> wgpu::Backends {
        match self {
            Self::Dx12 => wgpu::Backends::DX12,
            Self::Vulkan => wgpu::Backends::VULKAN,
        }
    }
}
#[derive(Debug, Clone)]
pub struct AdapterSelector {
    pub backend: Backend,
    pub name: String,
    pub vendor: u32,
    pub device: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    pub clear: [f64; 4],
    pub exposure: f64,
    pub images: ImageLimits,
    /// Renderer-owned host arrays and upload/readback overlap; excludes resident input/evaluator and codec scratch.
    pub max_application_bytes: u64,
    pub max_gpu_bytes: u64,
    pub max_lights: u32,
    pub max_lut_work: u64,
    pub timeout: Duration,
}
impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            width: 64,
            height: 64,
            clear: [0.0, 0.0, 0.0, 1.0],
            exposure: 1.0,
            images: ImageLimits::default(),
            max_application_bytes: 512 * 1024 * 1024,
            max_gpu_bytes: 512 * 1024 * 1024,
            max_lights: 16,
            max_lut_work: 2_097_152,
            timeout: Duration::from_secs(10),
        }
    }
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    frame_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    chunk_layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::RenderPipeline>,
    uncaptured: Arc<Mutex<Vec<String>>>,
}

struct GpuChunk {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    index_bytes: u64,
    material: usize,
    group: wgpu::BindGroup,
    pipeline: usize,
    blend: bool,
}

/// Fully uploaded resources. No borrow of ApplicationAsset, Scene, evaluation or IO survives.
pub struct GpuAsset {
    chunks: Vec<GpuChunk>,
    materials: Vec<wgpu::BindGroup>,
    frame: wgpu::BindGroup,
    failure: wgpu::Buffer,
    settings: RenderSettings,
    index_width: IndexWidth,
    report: serde_json::Value,
}

#[derive(Debug)]
pub struct RenderedFrame {
    pub width: u32,
    pub height: u32,
    /// Top-to-bottom, linear, straight RGBA; no preview transfer or tone map.
    pub linear_rgba: Vec<[f32; 4]>,
    pub diagnostics: Vec<u32>,
    pub report: serde_json::Value,
    exposure: f64,
}

fn instance(backend: Backend) -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backend.mask();
    wgpu::Instance::new(descriptor)
}

pub fn adapters(backend: Backend) -> Vec<wgpu::AdapterInfo> {
    pollster::block_on(instance(backend).enumerate_adapters(backend.mask()))
        .iter()
        .map(|a| a.get_info())
        .collect()
}

pub fn adapter_report(info: &wgpu::AdapterInfo) -> serde_json::Value {
    serde_json::json!({ "name":info.name,"vendor":info.vendor,"device":info.device,"backend":format!("{:?}",info.backend),
        "device_type":format!("{:?}",info.device_type),"driver":info.driver,"driver_info":info.driver_info })
}

impl Renderer {
    pub fn new(selector: &AdapterSelector) -> Result<Self> {
        let instance = instance(selector.backend);
        let adapters = pollster::block_on(instance.enumerate_adapters(selector.backend.mask()));
        let mut matching = adapters.into_iter().filter(|a| {
            let info = a.get_info();
            info.name == selector.name
                && info.vendor == selector.vendor
                && info.device == selector.device
        });
        let first = matching.next();
        let count = usize::from(first.is_some()) + matching.count();
        if count != 1 {
            return Err(Error::AdapterSelection { matches: count });
        }
        let adapter = first.ok_or(Error::AdapterSelection { matches: 0 })?;
        let info = adapter.get_info();
        if !adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::FRAGMENT_WRITABLE_STORAGE)
        {
            return Err(Error::UnsupportedDeviceProfile(
                "fragment atomic diagnostic storage is required".into(),
            ));
        }
        let required = wgpu::Limits {
            max_texture_dimension_2d: 4096,
            max_buffer_size: 128 * 1024 * 1024,
            max_storage_buffer_binding_size: 128 * 1024 * 1024,
            max_inter_stage_shader_variables: 8,
            max_bind_groups: 3,
            max_bindings_per_bind_group: 21,
            max_sampled_textures_per_shader_stage: 11,
            max_samplers_per_shader_stage: 11,
            max_uniform_buffer_binding_size: 560,
            max_vertex_buffers: 1,
            max_vertex_attributes: 4,
            max_vertex_buffer_array_stride: 64,
            max_color_attachments: 2,
            max_color_attachment_bytes_per_sample: 12,
            ..wgpu::Limits::downlevel_defaults()
        };
        if !required.check_limits(&adapter.limits()) {
            return Err(Error::UnsupportedDeviceProfile(
                "requested finite limits exceed adapter".into(),
            ));
        }
        for (format, usage, blend) in [
            (
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                true,
            ),
            (
                wgpu::TextureFormat::R32Uint,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                false,
            ),
            (
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                false,
            ),
            (
                wgpu::TextureFormat::R16Float,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                false,
            ),
            (
                wgpu::TextureFormat::Rgba8UnormSrgb,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                false,
            ),
            (
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                false,
            ),
        ] {
            let features = adapter.get_texture_format_features(format);
            if !features.allowed_usages.contains(usage)
                || (blend
                    && !features
                        .flags
                        .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE))
            {
                return Err(Error::UnsupportedDeviceProfile(format!(
                    "unsupported attachment/sample format {format:?}"
                )));
            }
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Baozi native finite renderer"),
            required_features: wgpu::Features::empty(),
            required_limits: required,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| Error::Gpu(e.to_string()))?;
        let uncaptured = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::clone(&uncaptured);
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            if let Ok(mut errors) = errors.lock()
                && errors.len() < 16
            {
                errors.push(e.to_string());
            }
        }));
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame layout"),
            entries: &[
                buffer_layout(0, wgpu::ShaderStages::VERTEX_FRAGMENT, false, 96),
                buffer_layout(1, wgpu::ShaderStages::FRAGMENT, true, 64),
                texture_layout(2),
                sampler_layout(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
            ],
        });
        let mut entries = vec_with_capacity(21)?;
        entries.push(buffer_layout(
            0,
            wgpu::ShaderStages::FRAGMENT,
            false,
            MATERIAL_BYTES as u64,
        ));
        for slot in 0..SLOT_COUNT as u32 {
            entries.push(texture_layout(slot * 2 + 1));
            entries.push(sampler_layout(slot * 2 + 2));
        }
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ten material slots"),
            entries: &entries,
        });
        let chunk_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("source chunk layout"),
            entries: &[
                buffer_layout(0, wgpu::ShaderStages::VERTEX, false, 32),
                buffer_layout(1, wgpu::ShaderStages::VERTEX, true, 8),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Baozi material pipeline"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&material_layout),
                Some(&chunk_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Baozi material WGSL"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/material.wgsl").into()),
        });
        let mut pipelines = vec_with_capacity(8)?;
        for blend in [false, true] {
            for double_sided in [false, true] {
                for clockwise in [false, true] {
                    pipelines.push(pipeline(
                        &device,
                        &layout,
                        &shader,
                        blend,
                        double_sided,
                        clockwise,
                    ));
                }
            }
        }
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(Error::Gpu(e.to_string()));
        }
        let renderer = Self {
            device,
            queue,
            info,
            frame_layout,
            material_layout,
            chunk_layout,
            pipelines,
            uncaptured,
        };
        renderer.check_errors()?;
        Ok(renderer)
    }

    pub fn info(&self) -> &wgpu::AdapterInfo {
        &self.info
    }

    pub fn upload(
        &self,
        plan: &DevicePlan<'_>,
        camera_input: CameraInput,
        settings: RenderSettings,
        extra_lights: &[LightFrame],
        control: Control<'_>,
    ) -> Result<GpuAsset> {
        control.check()?;
        self.check_errors()?;
        if settings.width == 0
            || settings.height == 0
            || settings.timeout.is_zero()
            || settings.timeout > Duration::from_secs(60)
            || !settings.exposure.is_finite()
            || settings.exposure < 0.0
            || settings.clear.iter().any(|v| !v.is_finite() || *v < 0.0)
            || settings.clear.iter().any(|v| *v > 65504.0)
            || !(0.0..=1.0).contains(&settings.clear[3])
        {
            return Err(Error::InvalidLimit("invalid render settings"));
        }
        let device_limits = self.device.limits();
        cap(
            "viewport width",
            u64::from(settings.width),
            u64::from(device_limits.max_texture_dimension_2d),
        )?;
        cap(
            "viewport height",
            u64::from(settings.height),
            u64::from(device_limits.max_texture_dimension_2d),
        )?;
        let camera = spatial::camera(
            plan.asset(),
            plan.evaluation(),
            camera_input,
            settings.width,
            settings.height,
        )?;
        let mut lights = spatial::lights(plan.asset(), plan.evaluation(), settings.max_lights)?;
        cap(
            "light placements",
            add(lights.len() as u64, extra_lights.len() as u64)?,
            u64::from(settings.max_lights),
        )?;
        lights.try_reserve_exact(extra_lights.len())?;
        lights.extend_from_slice(extra_lights);
        let mut frame = [0u8; 96];
        for (i, &value) in camera.view_projection.iter().flatten().enumerate() {
            f32_at(&mut frame, i * 4, uniform_scalar(value, i)?);
        }
        let viewer = if camera.orthographic {
            camera.view_direction
        } else {
            camera.position
        };
        for (i, value) in viewer.into_iter().enumerate() {
            f32_at(&mut frame, 64 + i * 4, uniform_scalar(value, i)?);
        }
        f32_at(&mut frame, 76, if camera.orthographic { 1.0 } else { 0.0 });
        for (i, value) in [lights.len() as u32, settings.width, settings.height, 0]
            .into_iter()
            .enumerate()
        {
            u32_at(&mut frame, 80 + i * 4, value);
        }
        let projection_work = validate_projection(plan, &frame, control)?;
        let mut light_bytes = vec_with_capacity(lights.len().max(1) * 64)?;
        for light in &lights {
            light_bytes.extend_from_slice(&light.bytes()?);
        }
        if lights.is_empty() {
            light_bytes.resize(64, 0);
        }
        let readback = readback_bytes(settings.width, settings.height)?;
        let pixels = mul(u64::from(settings.width), u64::from(settings.height))?;
        let target_bytes = mul(pixels, 16)?;
        let host_output = add(readback, mul(pixels, 40)?)?;
        let host_metadata = add(
            65_536,
            add(
                light_bytes.capacity() as u64,
                mul(
                    plan.chunks().len() as u64,
                    std::mem::size_of::<GpuChunk>() as u64,
                )?,
            )?,
        )?;
        let host_fixed = add(mul(plan.plan_bytes(), 2)?, add(host_output, host_metadata)?)?;
        cap(
            "renderer host fixed overlap",
            host_fixed,
            settings.max_application_bytes,
        )?;
        let image_budget = (settings.max_application_bytes - host_fixed) / 2;
        let images = PreparedImages::prepare(
            plan.asset(),
            plan.materials(),
            ImageLimits {
                max_dimension: settings
                    .images
                    .max_dimension
                    .min(device_limits.max_texture_dimension_2d),
                max_resident_bytes: settings.images.max_resident_bytes.min(image_budget),
                ..settings.images
            },
            control,
        )?;
        let host_estimate = add(host_fixed, mul(images.resident_bytes, 2)?)?;
        cap(
            "renderer host overlap",
            host_estimate,
            settings.max_application_bytes,
        )?;
        let material_bytes = mul(plan.materials().len() as u64, MATERIAL_BYTES as u64)?;
        let gpu_bytes = add(
            add(
                add(plan.gpu_bytes(), images.resident_bytes)?,
                add(target_bytes, readback)?,
            )?,
            add(
                material_bytes,
                add(light_bytes.len() as u64 + 96, 32 * 32 * 2 + 12)?,
            )?,
        )?;
        cap(
            "GPU logical bytes",
            gpu_bytes,
            settings.max_gpu_bytes.min(plan.limits().max_gpu_bytes),
        )?;
        for c in plan.chunks() {
            cap(
                "vertex device buffer",
                c.vertices.len() as u64,
                device_limits.max_buffer_size,
            )?;
            cap(
                "UV device buffer",
                c.uv_bytes.len().max(8) as u64,
                device_limits.max_storage_buffer_binding_size,
            )?;
            cap(
                "index device buffer",
                mul(c.indices.len() as u64, plan.limits().index_width.bytes())?.next_multiple_of(4),
                device_limits.max_buffer_size,
            )?;
        }
        cap(
            "light storage",
            light_bytes.len() as u64,
            device_limits.max_storage_buffer_binding_size,
        )?;
        for bpp in [8, 4] {
            cap(
                "readback device buffer",
                mul(
                    u64::from(padded_row(settings.width, bpp)?),
                    u64::from(settings.height),
                )?,
                device_limits.max_buffer_size,
            )?;
        }
        control.check()?;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut variants = vec_with_capacity(images.variants.len())?;
        for variant in &images.variants {
            control.check()?;
            let base = &variant.levels[0];
            let format = if variant.transfer == ColorInterpretation::SrgbColor {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            };
            let texture = texture(
                &self.device,
                base.width,
                base.height,
                variant.levels.len() as u32,
                format,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            for (level, mip) in variant.levels.iter().enumerate() {
                write_texture(
                    &self.queue,
                    &texture,
                    level as u32,
                    mip.width,
                    mip.height,
                    4,
                    &mip.rgba,
                );
            }
            variants.push(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        }
        let dummy = texture(
            &self.device,
            1,
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        write_texture(&self.queue, &dummy, 0, 1, 1, 4, &[255, 255, 255, 255]);
        let dummy = dummy.create_view(&wgpu::TextureViewDescriptor::default());
        let default_sampler = self
            .device
            .create_sampler(&wgpu::SamplerDescriptor::default());
        let mut samplers = Vec::<(usize, wgpu::Sampler)>::new();
        for slot in plan
            .materials()
            .iter()
            .flat_map(|m| m.slots.iter().flatten())
        {
            if !samplers.iter().any(|s| s.0 == slot.sampler) {
                samplers.try_reserve_exact(1)?;
                samplers.push((
                    slot.sampler,
                    self.device.create_sampler(&sampler_descriptor(
                        plan.asset().samplers[slot.sampler].state,
                    )?),
                ));
            }
        }
        let mut materials = vec_with_capacity(plan.materials().len())?;
        for material in plan.materials() {
            let uniform = self.buffer("material", &material.bytes, wgpu::BufferUsages::UNIFORM);
            let mut entries = vec_with_capacity(21)?;
            entries.push(wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            });
            for (slot, binding) in material.slots.iter().enumerate() {
                let (view, sampler) = if let Some(b) = binding {
                    let variant = images
                        .variants
                        .iter()
                        .position(|v| v.image == b.image && v.transfer == b.transfer)
                        .ok_or(Error::InvalidAsset("image variant"))?;
                    let sampler = samplers
                        .iter()
                        .find(|s| s.0 == b.sampler)
                        .ok_or(Error::InvalidAsset("GPU sampler"))?;
                    (&variants[variant], &sampler.1)
                } else {
                    (&dummy, &default_sampler)
                };
                entries.push(wgpu::BindGroupEntry {
                    binding: slot as u32 * 2 + 1,
                    resource: wgpu::BindingResource::TextureView(view),
                });
                entries.push(wgpu::BindGroupEntry {
                    binding: slot as u32 * 2 + 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                });
            }
            materials.push(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("compiled material"),
                layout: &self.material_layout,
                entries: &entries,
            }));
        }
        let has_sheen = plan.materials().iter().any(|m| {
            (0..3).any(|i| {
                f32::from_le_bytes(m.bytes[48 + i * 4..52 + i * 4].try_into().unwrap_or([0; 4]))
                    != 0.0
            })
        });
        let (lut_width, lut_bytes, lut_work) = if has_sheen {
            let (bytes, work) = sheen_table(settings.max_lut_work, control)?;
            (32, bytes, work)
        } else {
            (1, vec![0, 0], 0)
        };
        let lut = texture(
            &self.device,
            lut_width,
            lut_width,
            1,
            wgpu::TextureFormat::R16Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        write_texture(&self.queue, &lut, 0, lut_width, lut_width, 2, &lut_bytes);
        let lut_view = lut.create_view(&wgpu::TextureViewDescriptor::default());
        let lut_sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let frame_buffer = self.buffer("camera frame", &frame, wgpu::BufferUsages::UNIFORM);
        let light_buffer = self.buffer(
            "light placements",
            &light_bytes,
            wgpu::BufferUsages::STORAGE,
        );
        let failure = self.buffer(
            "persistent fragment validation",
            &[0; 4],
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        );
        let frame = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("evaluated camera/lights"),
            layout: &self.frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&lut_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: failure.as_entire_binding(),
                },
            ],
        });
        let mut chunks = vec_with_capacity(plan.chunks().len())?;
        for chunk in plan.chunks() {
            control.check()?;
            let vertices = self.buffer(
                "world vertices",
                &chunk.vertices,
                wgpu::BufferUsages::VERTEX,
            );
            let indices = chunk.index_bytes(plan.limits().index_width)?;
            let index_bytes = indices.len() as u64;
            let indices = self.buffer(
                "triangle indices",
                &indices,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_SRC,
            );
            let uv = self.buffer(
                "all retained UV sets",
                if chunk.uv_bytes.is_empty() {
                    &[0; 8]
                } else {
                    &chunk.uv_bytes
                },
                wgpu::BufferUsages::STORAGE,
            );
            let mut params = [0u8; 32];
            for (i, value) in [
                chunk.vertex_sources.len() as u32,
                chunk.uv_sets.len() as u32,
                chunk.field_flags,
                u32::from(chunk.orientation < 0),
            ]
            .into_iter()
            .chain(chunk.active_uv_ordinals)
            .enumerate()
            {
                u32_at(&mut params, i * 4, value);
            }
            let uniform = self.buffer("chunk correspondence", &params, wgpu::BufferUsages::UNIFORM);
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("chunk streams"),
                layout: &self.chunk_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uv.as_entire_binding(),
                    },
                ],
            });
            let material = &plan.materials()[chunk.compiled_material];
            let blend = material.alpha_mode == AlphaMode::Blend;
            let source_clockwise = match plan.asset().space.front_face_winding {
                FrontFaceWinding::Clockwise => true,
                FrontFaceWinding::CounterClockwise => false,
                _ => return Err(Error::InvalidAsset("unknown raster front face")),
            };
            let clockwise = source_clockwise ^ (chunk.orientation < 0) ^ camera.reflected;
            let pipeline = usize::from(blend) * 4
                + usize::from(material.double_sided) * 2
                + usize::from(clockwise);
            chunks.push(GpuChunk {
                vertices,
                indices,
                index_bytes,
                index_count: chunk.indices.len() as u32,
                material: chunk.compiled_material,
                group,
                pipeline,
                blend,
            });
        }
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(Error::Gpu(e.to_string()));
        }
        self.check_errors()?;
        control.check()?;
        let report = serde_json::json!({ "adapter":adapter_report(&self.info),"requested_features":"empty","requested_limits":format!("{:?}",self.device.limits()),
            "inter_stage":"five user locations (world, normal, color, two full-F32 UV pairs); FrontFacing uses one additional variable budget slot",
            "viewport":[settings.width,settings.height],"camera":format!("{camera_input:?}"),"lights":format!("{lights:?}"),
            "index_width":format!("{:?}",plan.limits().index_width),"primitive_restart":false,"base_vertex":0,"skinning":"CPU full rows, evaluator world XYZ",
            "chunks":plan.chunks().len(),"plan_bytes":plan.plan_bytes(),"geometry_gpu_bytes":plan.gpu_bytes(),"gpu_logical_bytes":gpu_bytes,
            "renderer_host_overlap_admission":host_estimate,"host_exclusions":["resident ApplicationAsset","resident EvaluatedAsset","codec scratch","wgpu/driver internals"],
            "image_profile":"in-process PNG/JPEG 8-bit still components; no metadata color management",
            "image_limits":format!("{:?}",settings.images),"codec_scratch":"best effort; image::Limits is not a total heap cap","codec_within_call_cancellation":false,
            "encoded_active_bytes":images.encoded_bytes,"mip_variant_bytes":images.resident_bytes,"mip_work":images.mip_work,"planning_work":plan.work(),"projection_work":projection_work,
            "sheen_lut":"32x32 R16F raw fitted-Charlie energy, may exceed one; uniform-NoL/azimuth 32x64 midpoint quadrature", "sheen_lut_work":lut_work,
            "sheen_scaling":"d=max(1,Ev_raw,El_raw); f_fit/d; A=min(1-C*clamp(Ev_raw,0,1),1-C*clamp(El_raw,0,1)); finite consumer approximation, no exact global energy-conservation claim; cosine domain [0.001,1]",
            "point_distance":"component-scaled direction with checked bounded length; range exclusion precedes normalization; range quotients above a 2^64 divisor use common 2^-64 scaling; scale converted to meters before length; reciprocal squared with 1mm floor; failed normalization, positive-cutoff distance above 2^63 meters or nonnormal attenuation fails with bit 512",
            "view_normalization":"component-scaled world delta with checked bounded length; zero, subnormal or nonfinite delta and failed normalization fail with bit 16",
            "normal_map_frame":"world/selected transformed UV derivatives, independent base/coat; not MikkTSpace",
            "transparency":"stable submission order after opaque/mask; straight alpha, depth test, no blend depth writes",
            "brdf":"Lambert + single-scatter GGX/height-correlated Smith + Charlie albedo scaling + simple clearcoat layering",
            "output":"linear RGBA16F + R32Uint diagnostics, top-to-bottom", "preview":"exposure*c/(1+exposure*c), standard sRGB RGB, linear alpha" });
        Ok(GpuAsset {
            chunks,
            materials,
            frame,
            failure,
            settings,
            index_width: plan.limits().index_width,
            report,
        })
    }

    pub fn render(&self, asset: &GpuAsset, control: Control<'_>) -> Result<RenderedFrame> {
        control.check()?;
        self.check_errors()?;
        let s = asset.settings;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let color = texture(
            &self.device,
            s.width,
            s.height,
            1,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let diagnostics = texture(
            &self.device,
            s.width,
            s.height,
            1,
            wgpu::TextureFormat::R32Uint,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            &self.device,
            s.width,
            s.height,
            1,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let color_view = color.create_view(&Default::default());
        let diagnostic_view = diagnostics.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let rgba_buffer = self.readback_buffer(
            "RGBA16F download",
            u64::from(padded_row(s.width, 8)?) * u64::from(s.height),
        );
        let diag_buffer = self.readback_buffer(
            "R32Uint download",
            u64::from(padded_row(s.width, 4)?) * u64::from(s.height),
        );
        let failure_buffer = self.readback_buffer("persistent fragment failure download", 4);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("headless render/readback"),
            });
        encoder.clear_buffer(&asset.failure, 0, None);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("static material pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: s.clear[0],
                                g: s.clear[1],
                                b: s.clear[2],
                                a: s.clear[3],
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &diagnostic_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &asset.frame, &[]);
            for blend in [false, true] {
                for chunk in asset.chunks.iter().filter(|c| c.blend == blend) {
                    pass.set_pipeline(&self.pipelines[chunk.pipeline]);
                    pass.set_bind_group(1, &asset.materials[chunk.material], &[]);
                    pass.set_bind_group(2, &chunk.group, &[]);
                    pass.set_vertex_buffer(0, chunk.vertices.slice(..));
                    pass.set_index_buffer(
                        chunk.indices.slice(..),
                        match asset.index_width {
                            IndexWidth::U16 => wgpu::IndexFormat::Uint16,
                            IndexWidth::U32 => wgpu::IndexFormat::Uint32,
                        },
                    );
                    pass.draw_indexed(0..chunk.index_count, 0, 0..1);
                }
            }
        }
        copy_texture(&mut encoder, &color, &rgba_buffer, s.width, s.height, 8)?;
        copy_texture(
            &mut encoder,
            &diagnostics,
            &diag_buffer,
            s.width,
            s.height,
            4,
        )?;
        encoder.copy_buffer_to_buffer(&asset.failure, 0, &failure_buffer, 0, 4);
        control.check()?;
        let submission = self.queue.submit([encoder.finish()]);
        let rgba_receiver = map(&rgba_buffer);
        let diag_receiver = map(&diag_buffer);
        let failure_receiver = map(&failure_buffer);
        self.wait(submission, s.timeout, control)?;
        mapped(rgba_receiver)?;
        mapped(diag_receiver)?;
        mapped(failure_receiver)?;
        let failure = failure_buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let code = u32::from_le_bytes(
            failure[..4]
                .try_into()
                .map_err(|_| Error::InvalidAsset("fragment failure word"))?,
        );
        drop(failure);
        failure_buffer.unmap();
        if code != 0 {
            return Err(Error::RenderNumericFailure {
                pixel: usize::MAX,
                code,
            });
        }
        let rgba = rgba_buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let diag = diag_buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let pixels = usize::try_from(mul(u64::from(s.width), u64::from(s.height))?)
            .map_err(|_| Error::InvalidLimit("pixel count"))?;
        let mut linear_rgba = vec_with_capacity(pixels)?;
        let mut diagnostic_codes = vec_with_capacity(pixels)?;
        let color_row = padded_row(s.width, 8)? as usize;
        let diag_row = padded_row(s.width, 4)? as usize;
        for y in 0..s.height as usize {
            control.check()?;
            for x in 0..s.width as usize {
                let index = y * s.width as usize + x;
                let code = u32::from_le_bytes(
                    diag[y * diag_row + x * 4..y * diag_row + x * 4 + 4]
                        .try_into()
                        .map_err(|_| Error::InvalidAsset("diagnostic row"))?,
                );
                let mut pixel = [0.0; 4];
                for (lane, value) in pixel.iter_mut().enumerate() {
                    let start = y * color_row + x * 8 + lane * 2;
                    *value =
                        f16::from_bits(u16::from_le_bytes([rgba[start], rgba[start + 1]])).to_f32();
                }
                if code != 0 || pixel.iter().any(|v| !v.is_finite()) {
                    return Err(Error::RenderNumericFailure { pixel: index, code });
                }
                linear_rgba.push(pixel);
                diagnostic_codes.push(code);
            }
        }
        drop(rgba);
        drop(diag);
        rgba_buffer.unmap();
        diag_buffer.unmap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(Error::Gpu(e.to_string()));
        }
        self.check_errors()?;
        Ok(RenderedFrame {
            width: s.width,
            height: s.height,
            linear_rgba,
            diagnostics: diagnostic_codes,
            report: asset.report.clone(),
            exposure: s.exposure,
        })
    }

    /// Actual GPU round trip of the uploaded index bytes; also valid for a 65,535 index.
    pub fn read_indices(
        &self,
        asset: &GpuAsset,
        chunk: usize,
        control: Control<'_>,
    ) -> Result<Vec<u8>> {
        let chunk = asset
            .chunks
            .get(chunk)
            .ok_or(Error::InvalidAsset("GPU chunk index"))?;
        let bytes = chunk.index_bytes.next_multiple_of(4);
        let target = self.readback_buffer("index validation download", bytes);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&chunk.indices, 0, &target, 0, bytes);
        let submission = self.queue.submit([encoder.finish()]);
        let receiver = map(&target);
        self.wait(submission, asset.settings.timeout, control)?;
        mapped(receiver)?;
        let data = target
            .slice(..)
            .get_mapped_range()
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let mut out = vec_with_capacity(chunk.index_bytes as usize)?;
        out.extend_from_slice(&data[..chunk.index_bytes as usize]);
        drop(data);
        target.unmap();
        self.check_errors()?;
        Ok(out)
    }

    fn buffer(&self, label: &str, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage,
            })
    }
    fn readback_buffer(&self, label: &str, size: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }
    fn check_errors(&self) -> Result<()> {
        let errors = self
            .uncaptured
            .lock()
            .map_err(|_| Error::Gpu("uncaptured error mutex poisoned".into()))?;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Gpu(errors.join("; ")))
        }
    }
    fn wait(
        &self,
        submission: wgpu::SubmissionIndex,
        timeout: Duration,
        control: Control<'_>,
    ) -> Result<()> {
        let start = Instant::now();
        loop {
            control.check()?;
            let remaining = timeout
                .checked_sub(start.elapsed())
                .ok_or_else(|| Error::Gpu("GPU submission deadline exceeded".into()))?;
            match self.device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: Some(remaining.min(Duration::from_millis(100))),
            }) {
                Ok(_) => return self.check_errors(),
                Err(wgpu::PollError::Timeout) => continue,
                Err(e) => return Err(Error::Gpu(e.to_string())),
            }
        }
    }
}

fn validate_projection(
    plan: &DevicePlan<'_>,
    frame: &[u8; 96],
    control: Control<'_>,
) -> Result<u64> {
    fn scalar(bytes: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    }
    let matrix: [[f32; 4]; 4] =
        std::array::from_fn(|c| std::array::from_fn(|r| scalar(frame, c * 16 + r * 4)));
    let mut work = 0;
    for chunk in plan.chunks() {
        work = add(work, mul(chunk.vertex_sources.len() as u64, 16)?)?;
        cap(
            "planning and projection work",
            add(plan.work(), work)?,
            plan.limits().max_work,
        )?;
        for (vertex, bytes) in chunk.vertices.chunks_exact(64).enumerate() {
            if vertex % 1024 == 0 {
                control.check()?;
            }
            let p = [scalar(bytes, 0), scalar(bytes, 4), scalar(bytes, 8), 1.0];
            for (lane, _) in matrix[0].iter().enumerate() {
                // A conservative absolute bound also covers non-fused intermediate sums.
                let bound = (0..4)
                    .map(|c| (f64::from(matrix[c][lane]) * f64::from(p[c])).abs())
                    .sum::<f64>();
                if !bound.is_finite() || bound > f64::from(f32::MAX) {
                    return Err(Error::Scalar {
                        location: ScalarLocation {
                            instance: chunk.instance,
                            draw: chunk.draw,
                            vertex: chunk.vertex_sources[vertex] as usize,
                            stream: "clip projection",
                            lane,
                        },
                        kind: ScalarError::Overflow,
                    });
                }
            }
        }
    }
    Ok(work)
}

fn buffer_layout(
    binding: u32,
    visibility: wgpu::ShaderStages,
    storage: bool,
    size: u64,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: if storage {
                wgpu::BufferBindingType::Storage { read_only: true }
            } else {
                wgpu::BufferBindingType::Uniform
            },
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}
fn texture_layout(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn sampler_layout(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    blend: bool,
    double_sided: bool,
    clockwise: bool,
) -> wgpu::RenderPipeline {
    let attributes = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 16,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 32,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 48,
            shader_location: 3,
        },
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("MR/Unlit finite material"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: if clockwise {
                wgpu::FrontFace::Cw
            } else {
                wgpu::FrontFace::Ccw
            },
            cull_mode: if double_sided {
                None
            } else {
                Some(wgpu::Face::Back)
            },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(!blend),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: if blend {
                        Some(wgpu::BlendState::ALPHA_BLENDING)
                    } else {
                        None
                    },
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}
fn texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    levels: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bounded image/attachment"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
fn write_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    width: u32,
    height: u32,
    bpp: u32,
    bytes: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * bpp),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}
fn copy_texture(
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    buffer: &wgpu::Buffer,
    width: u32,
    height: u32,
    bpp: u32,
) -> Result<()> {
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row(width, bpp)?),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    Ok(())
}
pub fn padded_row(width: u32, bpp: u32) -> Result<u32> {
    let raw = mul(u64::from(width), u64::from(bpp))?;
    u32::try_from(add(raw, 255)? / 256 * 256)
        .map_err(|_| Error::InvalidLimit("aligned row overflow"))
}
fn readback_bytes(width: u32, height: u32) -> Result<u64> {
    mul(
        add(
            u64::from(padded_row(width, 8)?),
            u64::from(padded_row(width, 4)?),
        )?,
        u64::from(height),
    )
}
fn map(buffer: &wgpu::Buffer) -> mpsc::Receiver<std::result::Result<(), wgpu::BufferAsyncError>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    receiver
}
fn mapped(receiver: mpsc::Receiver<std::result::Result<(), wgpu::BufferAsyncError>>) -> Result<()> {
    receiver
        .try_recv()
        .map_err(|e| Error::Gpu(format!("mapping callback missing: {e}")))?
        .map_err(|e| Error::Gpu(e.to_string()))
}

/// Deliberately small, finite application work; no hidden build script or image decoder.
pub fn sheen_table(max_work: u64, control: Control<'_>) -> Result<(Vec<u8>, u64)> {
    const WORK: u64 = 32 * 32 * 32 * 64;
    cap("sheen quadrature work", WORK, max_work)?;
    let mut bytes = vec_with_capacity(32 * 32 * 2)?;
    for y in 0..32 {
        control.check()?;
        let roughness = 0.045 + 0.955 * f64::from(y) / 31.0;
        for x in 0..32 {
            let nv = 0.001 + 0.999 * f64::from(x) / 31.0;
            let energy = sheen_albedo(nv, roughness, 32, 64);
            // The published fitted visibility overshoots unit energy at grazing,
            // low-roughness angles. Retain the integral rather than clamping it.
            if !energy.is_finite() || energy < 0.0 {
                return Err(Error::InvalidAsset(
                    "nonfinite or negative sheen directional albedo",
                ));
            }
            let packed = f16::from_f64(energy);
            if !packed.is_finite() {
                return Err(Error::InvalidAsset(
                    "sheen directional albedo exceeds R16Float",
                ));
            }
            bytes.extend_from_slice(&packed.to_bits().to_le_bytes());
        }
    }
    Ok((bytes, WORK))
}

impl RenderedFrame {
    pub fn pixel(&self, x: u32, y: u32) -> Option<[f32; 4]> {
        if x >= self.width || y >= self.height {
            None
        } else {
            self.linear_rgba
                .get(y as usize * self.width as usize + x as usize)
                .copied()
        }
    }
    /// Save only a completely validated frame. Individual host file writes remain fallible.
    pub fn save(&self, prefix: &Path) -> Result<()> {
        let mut preview = vec_with_capacity(self.linear_rgba.len() * 4)?;
        let mut linear = vec_with_capacity(self.linear_rgba.len() * 16)?;
        for pixel in &self.linear_rgba {
            for (lane, &value) in pixel.iter().enumerate() {
                linear.extend_from_slice(&value.to_le_bytes());
                let value = if lane < 3 {
                    let exposed = f64::from(value).max(0.0) * self.exposure;
                    linear_to_srgb(exposed / (1.0 + exposed))
                } else {
                    f64::from(value)
                };
                if !value.is_finite() {
                    return Err(Error::InvalidLimit("preview exposure overflow"));
                }
                preview.push((value * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
        image::save_buffer_with_format(
            prefix.with_extension("png"),
            &preview,
            self.width,
            self.height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|e| Error::Image {
            image: usize::MAX,
            reason: e.to_string(),
        })?;
        std::fs::write(prefix.with_extension("rgba32f"), linear)?;
        let report =
            serde_json::to_vec_pretty(&self.report).map_err(|e| Error::Gpu(e.to_string()))?;
        std::fs::write(prefix.with_extension("json"), report)?;
        Ok(())
    }
}
