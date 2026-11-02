//! Fixed ten-slot material compiler. The detached material remains the authority.

use crate::device::{ScalarLocation, Tolerance, narrow};
use crate::{Error, Result, f32_at, u32_at};
use baozi::{
    AlphaMode, ColorInterpretation, ShadingModel, TextureAddressMode, TextureBlendOperation,
    TextureChannel, TextureRole, UvOrigin,
};
use baozi_asset_consumer::{ApplicationAsset, Binding, Material};

pub const MATERIAL_BYTES: usize = 560;
pub const SLOT_COUNT: usize = 10;
pub const BASE: usize = 0;
pub const MR: usize = 1;
pub const NORMAL: usize = 2;
pub const AO: usize = 3;
pub const EMISSION: usize = 4;
pub const COAT: usize = 5;
pub const COAT_ROUGHNESS: usize = 6;
pub const COAT_NORMAL: usize = 7;
pub const SHEEN: usize = 8;
pub const SHEEN_ROUGHNESS: usize = 9;

#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub texture: usize,
    pub image: usize,
    pub sampler: usize,
    pub transfer: ColorInterpretation,
    pub uv_set: u32,
    pub affine: [[f32; 3]; 2],
    pub strength: f32,
    pub channels: [u32; 4],
    pub lanes: u32,
    pub decal: [bool; 2],
}

impl Slot {
    pub fn coordinate(self, uv: [f32; 2]) -> [f32; 2] {
        self.affine.map(|r| r[0] * uv[0] + r[1] * uv[1] + r[2])
    }
}

#[derive(Debug)]
pub struct CompiledMaterial {
    pub source: Option<usize>,
    pub unlit: bool,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
    pub slots: [Option<Slot>; SLOT_COUNT],
    pub active_uv_sets: Vec<u32>,
    pub bytes: [u8; MATERIAL_BYTES],
}

impl CompiledMaterial {
    pub fn compile(asset: &ApplicationAsset, source: Option<usize>) -> Result<Self> {
        let material = source
            .map(|i| {
                asset
                    .materials
                    .get(i)
                    .ok_or(Error::InvalidAsset("material index"))
            })
            .transpose()?;
        let fallback = Material {
            name: None,
            shading_model: ShadingModel::PbrMetallicRoughness,
            alpha_mode: AlphaMode::Opaque,
            double_sided: false,
            factors: baozi::MaterialFactors::empty()
                .with_pbr_metallic_roughness(Some(Default::default())),
            bindings: Vec::new(),
        };
        let m = material.unwrap_or(&fallback);
        let unsupported = |feature| Error::UnsupportedMaterial {
            material: source,
            feature,
        };
        let unlit = m.shading_model == ShadingModel::Unlit;
        if !unlit && m.shading_model != ShadingModel::PbrMetallicRoughness {
            return Err(unsupported(
                "only Unlit and metallic-roughness workflows are supported",
            ));
        }
        if !unlit
            && (m.factors.transmission().is_some_and(|v| v.factor() != 0.0)
                || m.factors.thickness().is_some_and(|v| v.thickness() != 0.0))
        {
            return Err(unsupported("transmission or thickness"));
        }
        let pbr = m.factors.pbr_metallic_roughness().unwrap_or_default();
        let base = pbr.base_color();
        let mut out = Self {
            source,
            unlit,
            alpha_mode: m.alpha_mode,
            double_sided: m.double_sided,
            slots: [None; SLOT_COUNT],
            active_uv_sets: Vec::new(),
            bytes: [0; MATERIAL_BYTES],
        };
        let coat = if unlit { None } else { m.factors.clearcoat() };
        let sheen = if unlit { None } else { m.factors.sheen() };
        if sheen.is_some_and(|s| {
            let c = s.color();
            c.x > 1.0 || c.y > 1.0 || c.z > 1.0
        }) {
            return Err(unsupported(
                "HDR sheen exceeds the unit-albedo layering profile",
            ));
        }
        let emission = if unlit {
            [0.0; 3]
        } else {
            m.factors.emission().effective_rgb()
        };
        let color = sheen
            .map(|s| {
                let c = s.color();
                [c.x, c.y, c.z]
            })
            .unwrap_or([0.0; 3]);
        for (i, value) in [
            f64::from(base.r),
            f64::from(base.g),
            f64::from(base.b),
            f64::from(base.a),
            emission[0],
            emission[1],
            emission[2],
            f64::from(pbr.metallic()),
            f64::from(pbr.roughness()),
            f64::from(coat.map_or(0.0, |c| c.factor())),
            f64::from(coat.map_or(0.0, |c| c.roughness())),
            f64::from(m.alpha_mode.mask_cutoff().unwrap_or(0.0)),
            f64::from(color[0]),
            f64::from(color[1]),
            f64::from(color[2]),
            f64::from(sheen.map_or(0.0, |s| s.roughness())),
        ]
        .into_iter()
        .enumerate()
        {
            f32_at(&mut out.bytes, 4 * i, uniform_scalar(value, i)?);
        }
        u32_at(&mut out.bytes, 64, u32::from(unlit));
        u32_at(
            &mut out.bytes,
            68,
            match m.alpha_mode {
                AlphaMode::Opaque => 0,
                AlphaMode::Mask(_) => 1,
                AlphaMode::Blend => 2,
            },
        );
        u32_at(&mut out.bytes, 72, u32::from(m.double_sided));
        for b in &m.bindings {
            let slot = match b.role {
                TextureRole::BaseColor => BASE,
                TextureRole::Opacity => return Err(unsupported("UnsupportedOpacitySampling")),
                _ if unlit => continue,
                TextureRole::MetallicRoughness => MR,
                TextureRole::Normal => NORMAL,
                TextureRole::Occlusion => AO,
                TextureRole::Emissive if emission.iter().any(|&v| v != 0.0) => EMISSION,
                TextureRole::Emissive => continue,
                TextureRole::Clearcoat if coat.is_some_and(|v| v.factor() != 0.0) => COAT,
                TextureRole::ClearcoatRoughness if coat.is_some_and(|v| v.factor() != 0.0) => {
                    COAT_ROUGHNESS
                }
                TextureRole::ClearcoatNormal if coat.is_some_and(|v| v.factor() != 0.0) => {
                    COAT_NORMAL
                }
                TextureRole::SheenColor if color.iter().any(|&v| v != 0.0) => SHEEN,
                TextureRole::SheenRoughness if color.iter().any(|&v| v != 0.0) => SHEEN_ROUGHNESS,
                TextureRole::Clearcoat
                | TextureRole::ClearcoatRoughness
                | TextureRole::ClearcoatNormal
                | TextureRole::SheenColor
                | TextureRole::SheenRoughness => continue,
                TextureRole::Metallic | TextureRole::Roughness => {
                    return Err(unsupported("UnsupportedSeparateMrSampling"));
                }
                // Positive transmission/thickness factors were rejected above. Absent
                // and explicit-zero groups keep their binding resources inactive.
                TextureRole::Transmission | TextureRole::Thickness => continue,
                TextureRole::Displacement if b.strength.get() != 0.0 => {
                    return Err(unsupported("active unsupported texture role"));
                }
                _ => continue,
            };
            if out.slots[slot].is_some() {
                return Err(unsupported("duplicate active texture role"));
            }
            let lanes = b.channel_mapping.lane_count();
            let valid = match slot {
                BASE => lanes == 3 || lanes == 4,
                MR => lanes == 2,
                NORMAL | COAT_NORMAL => lanes == 2 || lanes == 3,
                EMISSION | SHEEN => lanes == 3,
                _ => lanes == 1,
            };
            if !valid {
                return Err(unsupported("texture channel arity"));
            }
            if b.blend != TextureBlendOperation::Replace {
                return Err(unsupported("non-Replace binding"));
            }
            if ![NORMAL, COAT_NORMAL].contains(&slot) && b.strength.get() < 0.0 {
                return Err(unsupported("negative binding strength"));
            }
            if [MR, COAT, COAT_ROUGHNESS, SHEEN_ROUGHNESS].contains(&slot)
                && b.strength.get() != 1.0
            {
                return Err(unsupported("scalar data binding strength must be one"));
            }
            if [BASE, AO].contains(&slot) && !(0.0..=1.0).contains(&b.strength.get()) {
                return Err(unsupported(
                    "base color and occlusion strength must be in [0,1]",
                ));
            }
            if [
                MR,
                NORMAL,
                AO,
                COAT,
                COAT_ROUGHNESS,
                COAT_NORMAL,
                SHEEN_ROUGHNESS,
            ]
            .contains(&slot)
                && b.color_interpretation != ColorInterpretation::Data
            {
                return Err(unsupported("data role with color transfer"));
            }
            if [BASE, EMISSION, SHEEN].contains(&slot)
                && b.color_interpretation == ColorInterpretation::Data
            {
                return Err(unsupported("color role with Data interpretation"));
            }
            let texture = asset
                .textures
                .get(b.texture)
                .ok_or(Error::InvalidAsset("texture index"))?;
            if texture.image >= asset.images.len() {
                return Err(Error::InvalidAsset("image index"));
            }
            let state = asset
                .samplers
                .get(texture.sampler)
                .ok_or(Error::InvalidAsset("sampler index"))?
                .state;
            let mut channels = [5; 4];
            for (dst, channel) in channels.iter_mut().zip(b.channel_mapping.as_slice()) {
                *dst = channel_code(*channel);
            }
            let affine = affine(b, asset.space.uv_origin)?;
            out.slots[slot] = Some(Slot {
                texture: b.texture,
                image: texture.image,
                sampler: texture.sampler,
                transfer: b.color_interpretation,
                uv_set: u32::from(b.uv_set),
                affine,
                strength: b.strength.get(),
                channels,
                lanes: u32::from(lanes),
                decal: [
                    state.address_u() == TextureAddressMode::Decal,
                    state.address_v() == TextureAddressMode::Decal,
                ],
            });
        }
        out.active_uv_sets.try_reserve_exact(SLOT_COUNT)?;
        out.active_uv_sets
            .extend(out.slots.iter().flatten().map(|s| s.uv_set));
        out.active_uv_sets.sort_unstable();
        out.active_uv_sets.dedup();
        if out.active_uv_sets.len() > 4 {
            return Err(Error::TooManyActiveUvSets);
        }
        for (i, slot) in out.slots.iter().enumerate() {
            if let Some(s) = slot {
                let offset = 80 + i * 48;
                for r in 0..2 {
                    for c in 0..3 {
                        f32_at(&mut out.bytes, offset + r * 16 + c * 4, s.affine[r][c]);
                    }
                }
                f32_at(&mut out.bytes, offset + 12, s.strength);
                u32_at(
                    &mut out.bytes,
                    offset + 32,
                    1 | (u32::from(s.decal[0]) << 1) | (u32::from(s.decal[1]) << 2),
                );
                let slot = out
                    .active_uv_sets
                    .iter()
                    .position(|&v| v == s.uv_set)
                    .ok_or(Error::InvalidAsset("UV slot"))?;
                u32_at(&mut out.bytes, offset + 36, slot as u32);
                let swizzle = s
                    .channels
                    .iter()
                    .enumerate()
                    .fold(0, |v, (i, c)| v | (c << (i * 4)));
                u32_at(&mut out.bytes, offset + 40, swizzle);
                u32_at(&mut out.bytes, offset + 44, s.lanes);
            }
        }
        Ok(out)
    }

    pub fn normal_maps(&self) -> impl Iterator<Item = Slot> + '_ {
        [self.slots[NORMAL], self.slots[COAT_NORMAL]]
            .into_iter()
            .flatten()
    }
}

pub fn uniform_scalar(value: f64, lane: usize) -> Result<f32> {
    narrow(
        value,
        Tolerance::ATTRIBUTE,
        ScalarLocation {
            instance: 0,
            draw: 0,
            vertex: 0,
            stream: "uniform",
            lane,
        },
    )
}

fn channel_code(channel: TextureChannel) -> u32 {
    match channel {
        TextureChannel::Red => 0,
        TextureChannel::Green => 1,
        TextureChannel::Blue => 2,
        TextureChannel::Alpha => 3,
        TextureChannel::Zero => 4,
        TextureChannel::One => 5,
    }
}

fn affine(binding: &Binding, origin: UvOrigin) -> Result<[[f32; 3]; 2]> {
    let t = binding.transform;
    let scale = t.scale();
    let offset = t.offset();
    let (sin, cos) = f64::from(t.rotation_radians()).sin_cos();
    let mut rows = [
        [
            cos * f64::from(scale.x),
            -sin * f64::from(scale.y),
            f64::from(offset.x),
        ],
        [
            sin * f64::from(scale.x),
            cos * f64::from(scale.y),
            f64::from(offset.y),
        ],
    ];
    match origin {
        UvOrigin::UpperLeft => {}
        UvOrigin::LowerLeft => {
            rows[1] = [-rows[1][0], -rows[1][1], 1.0 - rows[1][2]];
        }
        _ => {
            return Err(Error::UnsupportedMaterial {
                material: None,
                feature: "unknown texture UV origin",
            });
        }
    }
    let mut out = [[0.0; 3]; 2];
    for r in 0..2 {
        for c in 0..3 {
            out[r][c] = uniform_scalar(rows[r][c], r * 3 + c)?;
        }
    }
    Ok(out)
}

pub fn srgb_to_linear(x: f64) -> f64 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
pub fn linear_to_srgb(x: f64) -> f64 {
    if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// Independent CPU reference for the shader's named GGX/Smith profile.
pub fn mr_brdf(
    base: [f64; 3],
    metallic: f64,
    roughness: f64,
    nv: f64,
    nl: f64,
    nh: f64,
    vh: f64,
) -> [f64; 3] {
    let alpha = roughness.max(0.045).powi(2);
    let a2 = alpha * alpha;
    let d = a2 / (std::f64::consts::PI * (nh * nh * (a2 - 1.0) + 1.0).powi(2));
    let visibility = 0.5
        / (nl * (nv * nv * (1.0 - a2) + a2).sqrt() + nv * (nl * nl * (1.0 - a2) + a2).sqrt())
            .max(1e-30);
    base.map(|c| {
        let f0 = 0.04 * (1.0 - metallic) + c * metallic;
        let f = f0 + (1.0 - f0) * (1.0 - vh).powi(5);
        (1.0 - metallic) * (1.0 - f) * c / std::f64::consts::PI + d * visibility * f
    })
}

fn sheen_l(x: f64, alpha: f64) -> f64 {
    let t = (1.0 - alpha).powi(2);
    let a = 21.5473 + (25.3245 - 21.5473) * t;
    let b = 3.82987 + (3.32435 - 3.82987) * t;
    let c = 0.19823 + (0.16801 - 0.19823) * t;
    let d = -1.97760 + (-1.27393 + 1.97760) * t;
    let e = -4.32054 + (-4.85967 + 4.32054) * t;
    a / (1.0 + b * x.powf(c)) + d * x + e
}
fn sheen_lambda(x: f64, alpha: f64) -> f64 {
    if x < 0.5 {
        sheen_l(x, alpha).exp()
    } else {
        (2.0 * sheen_l(0.5, alpha) - sheen_l(1.0 - x, alpha)).exp()
    }
}
pub fn charlie_brdf(nv: f64, nl: f64, nh: f64, roughness: f64) -> f64 {
    let alpha = roughness.max(0.045).powi(2);
    let d = (2.0 + 1.0 / alpha) * (1.0 - nh * nh).max(0.0).powf(0.5 / alpha)
        / (2.0 * std::f64::consts::PI);
    d / ((1.0 + sheen_lambda(nv, alpha) + sheen_lambda(nl, alpha)) * 4.0 * nv * nl).max(1e-30)
}

/// Midpoint quadrature uniform in NoL and azimuth: dOmega = dNoL * dPhi.
/// NoL intervals match the independent preparation vectors. This integrates the
/// fitted visibility as published; its grazing approximation can have energy > 1.
pub fn sheen_albedo(nv: f64, roughness: f64, no_l_steps: u32, phi_steps: u32) -> f64 {
    let view = [(1.0 - nv * nv).max(0.0).sqrt(), 0.0, nv];
    let mut energy = 0.0;
    for t in 0..no_l_steps {
        let cos = (f64::from(t) + 0.5) / f64::from(no_l_steps);
        let sin = (1.0 - cos * cos).sqrt();
        for p in 0..phi_steps {
            let phi = (f64::from(p) + 0.5) * std::f64::consts::TAU / f64::from(phi_steps);
            let h = [view[0] + sin * phi.cos(), sin * phi.sin(), view[2] + cos];
            let nh = h[2] / (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
            energy += charlie_brdf(nv, cos, nh, roughness) * cos;
        }
    }
    energy * std::f64::consts::TAU / f64::from(no_l_steps) / f64::from(phi_steps)
}

/// CPU reference for the consumer's reciprocal, bounded sheen approximation.
/// Returns (Charlie multiplier, substrate multiplier). Raw energies stay unchanged;
/// the finite LUT approximation is not a proof of exact global energy conservation.
pub fn sheen_layer_weights(ev: f64, el: f64, maximum_color: f64) -> Result<(f64, f64)> {
    if !ev.is_finite()
        || !el.is_finite()
        || ev < 0.0
        || el < 0.0
        || !(0.0..=1.0).contains(&maximum_color)
    {
        return Err(Error::InvalidAsset(
            "invalid sheen normalization parameters",
        ));
    }
    let normalization = 1.0 / ev.max(el).max(1.0);
    let substrate = (1.0 - maximum_color * ev.min(1.0)).min(1.0 - maximum_color * el.min(1.0));
    Ok((normalization, substrate))
}
