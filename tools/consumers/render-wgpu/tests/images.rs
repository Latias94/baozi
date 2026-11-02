mod common;

use baozi::{
    ColorInterpretation, MipmapFilter, SamplerState, TexelFilter, TextureAddressMode,
    TextureChannelSwizzle, TextureRole,
};
use baozi_render_wgpu::{
    Control, Error,
    images::*,
    material::{BASE, CompiledMaterial, MR},
};
use image::ImageEncoder;
use std::sync::atomic::AtomicBool;

pub fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}

fn jpeg(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100)
        .encode(rgb, width, height, image::ExtendedColorType::Rgb8)
        .unwrap();
    bytes
}

fn textured_asset() -> (baozi_asset_consumer::ApplicationAsset, CompiledMaterial) {
    let mut asset = common::asset(3);
    let texture = common::image(&mut asset, &[64, 128, 192, 255], 1, 1);
    asset.materials[0].bindings.push(common::binding(
        TextureRole::BaseColor,
        texture,
        0,
        TextureChannelSwizzle::RGBA,
        ColorInterpretation::SrgbColor,
    ));
    let compiled = CompiledMaterial::compile(&asset, Some(0)).unwrap();
    (asset, compiled)
}

#[test]
fn actual_png_jpeg_decode_and_truthful_finite_caps() {
    let rgba = [64, 128, 192, 128];
    let bytes = png(1, 1, &rgba);
    let decoded = decode(
        0,
        &bytes,
        Some("image/png"),
        ImageLimits::default(),
        Control::default(),
    )
    .unwrap();
    assert_eq!(decoded.rgba, rgba);
    let jpeg = jpeg(1, 1, &[64, 128, 192]);
    let decoded = decode(
        1,
        &jpeg,
        Some("image/jpeg"),
        ImageLimits::default(),
        Control::default(),
    )
    .unwrap();
    assert_eq!(decoded.rgba[3], 255);
    for (actual, expected) in decoded.rgba[..3].iter().zip([64i16, 128, 192]) {
        assert!((i16::from(*actual) - expected).abs() <= 3);
    }
    assert!(
        decode(
            0,
            &bytes,
            Some("image/jpeg"),
            ImageLimits::default(),
            Control::default()
        )
        .is_err()
    );
    assert!(
        decode(
            0,
            &bytes[..bytes.len() - 8],
            None,
            ImageLimits::default(),
            Control::default()
        )
        .is_err()
    );
    assert!(
        decode(
            0,
            &bytes,
            None,
            ImageLimits {
                max_encoded_per_image: 1,
                ..Default::default()
            },
            Control::default()
        )
        .is_err()
    );
    assert!(
        decode(
            0,
            &bytes,
            None,
            ImageLimits {
                max_resident_bytes: 1,
                ..Default::default()
            },
            Control::default()
        )
        .is_err()
    );
    let mut lie = bytes.clone();
    lie[16..20].copy_from_slice(&5000u32.to_be_bytes());
    assert!(decode(0, &lie, None, ImageLimits::default(), Control::default()).is_err());
    let mut deep = bytes.clone();
    deep[24] = 16;
    assert!(decode(0, &deep, None, ImageLimits::default(), Control::default()).is_err());
    let mut corrupt = bytes.clone();
    corrupt[45] ^= 0x80;
    assert!(
        decode(
            0,
            &corrupt,
            None,
            ImageLimits::default(),
            Control::default()
        )
        .is_err()
    );
}

#[test]
fn jpeg_requires_eoi_and_rejects_truncated_entropy_even_with_eoi() {
    let rgb: Vec<u8> = (0..32 * 32 * 3)
        .map(|i| ((i * 73 + i / 7) % 256) as u8)
        .collect();
    let bytes = jpeg(32, 32, &rgb);
    let valid = decode(
        7,
        &bytes,
        Some("image/jpeg"),
        ImageLimits::default(),
        Control::default(),
    )
    .unwrap();
    assert_eq!(
        (valid.width, valid.height, valid.rgba.len()),
        (32, 32, 32 * 32 * 4)
    );
    assert!(bytes.ends_with(&[0xff, 0xd9]));
    assert!(matches!(
        decode(
            7,
            &bytes[..bytes.len() - 2],
            None,
            ImageLimits::default(),
            Control::default()
        ),
        Err(Error::Image { image: 7, .. })
    ));
    let mut after_eoi = bytes.clone();
    after_eoi.push(0);
    assert!(matches!(
        decode(
            7,
            &after_eoi,
            None,
            ImageLimits::default(),
            Control::default()
        ),
        Err(Error::Image { image: 7, .. })
    ));
    // The terminal-marker check is not an exact JPEG framing parser. The mature
    // codec reads the first complete image when a tail itself ends in EOI.
    after_eoi.extend_from_slice(&[0xff, 0xd9]);
    let framed_tail = decode(
        7,
        &after_eoi,
        None,
        ImageLimits::default(),
        Control::default(),
    )
    .unwrap();
    assert_eq!(framed_tail.rgba, valid.rgba);

    // Locate the encoder's one SOS in this fixture, preserving its complete
    // headers and EOI while removing only entropy-coded bytes.
    let sos = bytes.windows(2).position(|b| b == [0xff, 0xda]).unwrap();
    let length = usize::from(u16::from_be_bytes([bytes[sos + 2], bytes[sos + 3]]));
    let entropy = sos + 2 + length;
    assert!(entropy + 8 < bytes.len() - 2);
    for cut in [
        entropy,
        entropy + 1,
        entropy + (bytes.len() - 2 - entropy) / 2,
    ] {
        let mut truncated = bytes[..cut].to_vec();
        truncated.extend_from_slice(&[0xff, 0xd9]);
        assert!(
            matches!(
                decode(
                    7,
                    &truncated,
                    None,
                    ImageLimits::default(),
                    Control::default()
                ),
                Err(Error::Image { image: 7, .. })
            ),
            "accepted truncated JPEG scan ending at byte {cut}"
        );
    }
}

#[test]
fn jpeg_tiny_resident_budget_fails_before_decoder_construction() {
    let bytes = jpeg(1, 1, &[64, 128, 192]);
    let limits = ImageLimits {
        max_resident_bytes: 1,
        ..Default::default()
    };
    // The framed but invalid neighbor would fail header decoding if construction
    // happened first. Both inputs must instead hit the known output lower bound.
    for input in [bytes.as_slice(), &[0xff, 0xd8, 0xff, 0xd9]] {
        assert!(matches!(
            decode(0, input, None, limits, Control::default()),
            Err(Error::Limit {
                dimension: "decode output overlap",
                requested: 7,
                limit: 1
            })
        ));
    }
}

#[test]
fn jpeg_admits_dimensions_pixels_and_native_rgba_overlap_before_decode() {
    let bytes = jpeg(2, 2, &[64; 2 * 2 * 3]);
    for (limits, dimension) in [
        (
            ImageLimits {
                max_dimension: 1,
                ..Default::default()
            },
            "image width",
        ),
        (
            ImageLimits {
                max_pixels: 3,
                ..Default::default()
            },
            "image pixels",
        ),
        (
            ImageLimits {
                max_resident_bytes: 27,
                ..Default::default()
            },
            "decode output overlap",
        ),
    ] {
        assert!(matches!(decode(0,&bytes,None,limits,Control::default()),
            Err(Error::Limit { dimension:actual,.. }) if actual==dimension));
    }
    let decoded = decode(0, &bytes, None, ImageLimits::default(), Control::default()).unwrap();
    assert_eq!(
        (decoded.width, decoded.height, decoded.rgba.len()),
        (2, 2, 16)
    );
}

#[test]
fn owned_asset_table_edits_and_compiled_slot_edits_are_typed_errors() {
    let (mut asset, compiled) = textured_asset();
    asset.samplers.clear();
    assert!(matches!(
        PreparedImages::prepare(
            &asset,
            &[compiled],
            ImageLimits::default(),
            Control::default()
        ),
        Err(Error::InvalidAsset("sampler index"))
    ));

    let (mut asset, compiled) = textured_asset();
    asset.images.clear();
    assert!(matches!(
        PreparedImages::prepare(
            &asset,
            &[compiled],
            ImageLimits::default(),
            Control::default()
        ),
        Err(Error::InvalidAsset("image index"))
    ));

    let (asset, mut compiled) = textured_asset();
    compiled.slots[BASE].as_mut().unwrap().sampler = usize::MAX;
    assert!(matches!(
        PreparedImages::prepare(
            &asset,
            &[compiled],
            ImageLimits::default(),
            Control::default()
        ),
        Err(Error::InvalidAsset("sampler index"))
    ));

    let (asset, mut compiled) = textured_asset();
    compiled.slots[BASE].as_mut().unwrap().image = usize::MAX;
    assert!(matches!(
        PreparedImages::prepare(
            &asset,
            &[compiled],
            ImageLimits::default(),
            Control::default()
        ),
        Err(Error::InvalidAsset("image index"))
    ));
}

#[test]
fn all_slots_are_checked_before_any_image_is_decoded() {
    for invalid_sampler in [false, true] {
        let (mut asset, mut compiled) = textured_asset();
        asset.images[0].encoded_bytes.clear();
        let mut later = compiled.slots[BASE].unwrap();
        if invalid_sampler {
            later.sampler = usize::MAX;
        } else {
            later.image = usize::MAX;
        }
        compiled.slots[MR] = Some(later);
        let expected = if invalid_sampler {
            "sampler index"
        } else {
            "image index"
        };
        assert!(
            matches!(PreparedImages::prepare(&asset,&[compiled],ImageLimits::default(),Control::default()),
            Err(Error::InvalidAsset(reason)) if reason==expected)
        );
    }
}

#[test]
fn shared_image_variants_report_all_owned_live_capacities() {
    let (asset, mut compiled) = textured_asset();
    let mut data = compiled.slots[BASE].unwrap();
    data.transfer = ColorInterpretation::Data;
    compiled.slots[MR] = Some(data);
    let prepared = PreparedImages::prepare(
        &asset,
        &[compiled],
        ImageLimits::default(),
        Control::default(),
    )
    .unwrap();
    assert_eq!(prepared.variants.len(), 2);
    assert_eq!(
        prepared.encoded_bytes,
        asset.images[0].encoded_bytes.len() as u64
    );
    let mut resident = prepared.variants.capacity() * size_of::<ImageVariant>();
    for variant in &prepared.variants {
        resident += variant.levels.capacity() * size_of::<Mip>();
        for level in &variant.levels {
            resident += level.rgba.capacity();
        }
        assert_eq!(variant.image, 0);
        assert_eq!(variant.levels[0].rgba, [64, 128, 192, 255]);
    }
    assert_eq!(prepared.resident_bytes, resident as u64);
}

#[test]
fn color_and_data_mips_differ_and_odd_edges_are_included() {
    let source = DecodedImage {
        width: 3,
        height: 1,
        rgba: vec![0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255],
    };
    let mut work = 0;
    let data = mip_chain(
        &source,
        ColorInterpretation::Data,
        ImageLimits::default(),
        Control::default(),
        &mut work,
    )
    .unwrap();
    let linear = mip_chain(
        &source,
        ColorInterpretation::LinearColor,
        ImageLimits::default(),
        Control::default(),
        &mut work,
    )
    .unwrap();
    let color = mip_chain(
        &source,
        ColorInterpretation::SrgbColor,
        ImageLimits::default(),
        Control::default(),
        &mut work,
    )
    .unwrap();
    assert_eq!(data[1].rgba, vec![85, 85, 85, 85]);
    assert_eq!(linear[1].rgba, data[1].rgba);
    assert!((i16::from(color[1].rgba[0]) - 156).abs() <= 1);
    assert_eq!(color[1].rgba[3], 85);
}

#[test]
fn mip_entry_cancellation_and_finite_work_rejection() {
    let cancellation = AtomicBool::new(true);
    let source = DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![64, 128, 192, 255],
    };
    let mut work = 17;
    assert!(matches!(
        mip_chain(
            &source,
            ColorInterpretation::Data,
            ImageLimits::default(),
            Control {
                cancellation: Some(&cancellation)
            },
            &mut work
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(work, 17);

    let source = DecodedImage {
        width: 4096,
        height: 1,
        rgba: vec![255; 4096 * 4],
    };
    let mut work = 0;
    assert!(matches!(
        mip_chain(
            &source,
            ColorInterpretation::Data,
            ImageLimits {
                max_mip_work: 4,
                ..Default::default()
            },
            Control::default(),
            &mut work
        ),
        Err(Error::Limit {
            dimension: "mip filter work",
            requested: 8,
            limit: 4
        })
    ));
}

#[test]
fn mip_admission_includes_source_capacity_and_level_table() {
    let mut rgba = Vec::with_capacity(1024);
    rgba.extend_from_slice(&[64, 128, 192, 255]);
    let source = DecodedImage {
        width: 1,
        height: 1,
        rgba,
    };
    let mut work = 0;
    assert!(matches!(
        mip_chain(
            &source,
            ColorInterpretation::Data,
            ImageLimits {
                max_resident_bytes: 1024,
                ..Default::default()
            },
            Control::default(),
            &mut work
        ),
        Err(Error::Limit {
            dimension: "image mip bytes",
            ..
        })
    ));
    let source = DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![64, 128, 192, 255],
    };
    assert!(matches!(
        mip_chain(
            &source,
            ColorInterpretation::Data,
            ImageLimits {
                max_resident_bytes: 8,
                ..Default::default()
            },
            Control::default(),
            &mut work
        ),
        Err(Error::Limit {
            dimension: "image mip bytes",
            ..
        })
    ));
}

#[test]
fn independent_sampler_fields_and_named_border_rejection() {
    let state = SamplerState::new(
        TexelFilter::Nearest,
        TexelFilter::Linear,
        MipmapFilter::None,
        TextureAddressMode::MirroredRepeat,
        TextureAddressMode::Decal,
        TextureAddressMode::Repeat,
    );
    let descriptor = sampler_descriptor(state).unwrap();
    assert_eq!(descriptor.min_filter, wgpu::FilterMode::Nearest);
    assert_eq!(descriptor.mag_filter, wgpu::FilterMode::Linear);
    assert_eq!(descriptor.lod_max_clamp, 0.0);
    assert_eq!(descriptor.address_mode_u, wgpu::AddressMode::MirrorRepeat);
    assert_eq!(descriptor.address_mode_v, wgpu::AddressMode::ClampToEdge);
    assert!(
        sampler_descriptor(SamplerState::new(
            TexelFilter::Nearest,
            TexelFilter::Nearest,
            MipmapFilter::None,
            TextureAddressMode::ClampToBorder,
            TextureAddressMode::Repeat,
            TextureAddressMode::Repeat
        ))
        .is_err()
    );
}
