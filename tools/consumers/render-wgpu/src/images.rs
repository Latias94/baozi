//! In-process PNG/JPEG profile. Decoder scratch is explicitly best effort.

use crate::material::{CompiledMaterial, linear_to_srgb, srgb_to_linear};
use crate::{Control, Error, Result, add, cap, mul, vec_with_capacity};
use baozi::{ColorInterpretation, MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};
use baozi_asset_consumer::ApplicationAsset;
use image::{ColorType, ImageDecoder, ImageFormat, ImageReader};
use std::io::Cursor;
use turbojpeg::{Decompressor, Image, PixelFormat};

#[derive(Debug, Clone, Copy)]
pub struct ImageLimits {
    pub max_encoded_per_image: u64,
    pub max_active_encoded: u64,
    pub max_dimension: u32,
    pub max_pixels: u64,
    pub max_resident_bytes: u64,
    /// Passed to image::Limits for PNG; TurboJPEG scratch is outside this cap.
    pub best_effort_decoder_alloc: u64,
    pub max_mip_work: u64,
}
impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_encoded_per_image: 16 * 1024 * 1024,
            max_active_encoded: 128 * 1024 * 1024,
            max_dimension: 4096,
            max_pixels: 16_777_216,
            max_resident_bytes: 256 * 1024 * 1024,
            best_effort_decoder_alloc: 128 * 1024 * 1024,
            max_mip_work: 200_000_000,
        }
    }
}

#[derive(Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Debug)]
pub struct Mip {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Debug)]
pub struct ImageVariant {
    pub image: usize,
    pub transfer: ColorInterpretation,
    pub levels: Vec<Mip>,
}
#[derive(Debug)]
pub struct PreparedImages {
    pub variants: Vec<ImageVariant>,
    pub encoded_bytes: u64,
    pub resident_bytes: u64,
    pub mip_work: u64,
}

fn image_error(image: usize, reason: impl Into<String>) -> Error {
    Error::Image {
        image,
        reason: reason.into(),
    }
}

pub fn decode(
    image_index: usize,
    bytes: &[u8],
    mime: Option<&str>,
    limits: ImageLimits,
    control: Control<'_>,
) -> Result<DecodedImage> {
    control.check()?;
    cap(
        "encoded image bytes",
        bytes.len() as u64,
        limits.max_encoded_per_image,
    )?;
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_profile(image_index, bytes, limits, control)?;
        ImageFormat::Png
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        ImageFormat::Jpeg
    } else {
        return Err(image_error(image_index, "unsupported image signature"));
    };
    if mime.is_some_and(|m| {
        m != if format == ImageFormat::Png {
            "image/png"
        } else {
            "image/jpeg"
        }
    }) {
        return Err(image_error(image_index, "MIME/signature mismatch"));
    }
    let (width, height, channels, native) = if format == ImageFormat::Jpeg {
        // Require the admitted slice to end at EOI, independently of the codec's
        // error reporting. Entropy/MCU validation belongs to the mature decoder.
        if !bytes.ends_with(&[0xff, 0xd9]) {
            return Err(image_error(image_index, "missing terminal JPEG EOI"));
        }
        // RGB native output and its RGBA conversion need at least seven bytes.
        cap("decode output overlap", 7, limits.max_resident_bytes)?;
        let mut decoder =
            Decompressor::new().map_err(|e| image_error(image_index, e.to_string()))?;
        decoder
            .set_scan_limit(100)
            .map_err(|e| image_error(image_index, e.to_string()))?;
        let header = decoder
            .read_header(bytes)
            .map_err(|e| image_error(image_index, e.to_string()))?;
        let width = u32::try_from(header.width)
            .map_err(|_| image_error(image_index, "JPEG width overflow"))?;
        let height = u32::try_from(header.height)
            .map_err(|_| image_error(image_index, "JPEG height overflow"))?;
        let channels = 3;
        let total = mul(mul(u64::from(width), u64::from(height))?, channels)?;
        let mut native = native_output(width, height, channels, total, limits, control)?;
        let pitch = usize::try_from(mul(u64::from(width), channels)?)
            .map_err(|_| image_error(image_index, "JPEG pitch overflow"))?;
        control.check()?;
        // TurboJPEG borrows the input and writes into admitted caller storage.
        // Its wrapper returns Err for warnings too, including premature EOI;
        // a recovered/partial native buffer is never converted or published.
        decoder
            .decompress(
                bytes,
                Image {
                    pixels: native.as_mut_slice(),
                    width: header.width,
                    pitch,
                    height: header.height,
                    format: PixelFormat::RGB,
                },
            )
            .map_err(|e| image_error(image_index, e.to_string()))?;
        (width, height, channels, native)
    } else {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
        let mut codec_limits = image::Limits::default();
        codec_limits.max_image_width = Some(limits.max_dimension);
        codec_limits.max_image_height = Some(limits.max_dimension);
        codec_limits.max_alloc = Some(limits.best_effort_decoder_alloc);
        reader.limits(codec_limits);
        let decoder = reader
            .into_decoder()
            .map_err(|e| image_error(image_index, e.to_string()))?;
        let (width, height) = decoder.dimensions();
        let channels = match decoder.color_type() {
            ColorType::L8 => 1,
            ColorType::La8 => 2,
            ColorType::Rgb8 => 3,
            ColorType::Rgba8 => 4,
            _ => {
                return Err(image_error(
                    image_index,
                    "only decoded 8-bit components are supported",
                ));
            }
        };
        let mut native = native_output(
            width,
            height,
            channels,
            decoder.total_bytes(),
            limits,
            control,
        )?;
        control.check()?;
        decoder
            .read_image(&mut native)
            .map_err(|e| image_error(image_index, e.to_string()))?;
        (width, height, channels, native)
    };
    control.check()?;
    let rgba_bytes = mul(mul(u64::from(width), u64::from(height))?, 4)?;
    cap(
        "decode output overlap",
        add(native.capacity() as u64, rgba_bytes)?,
        limits.max_resident_bytes,
    )?;
    let mut rgba = vec_with_capacity(
        usize::try_from(rgba_bytes)
            .map_err(|_| image_error(image_index, "RGBA output overflow"))?,
    )?;
    cap(
        "decode output overlap",
        add(native.capacity() as u64, rgba.capacity() as u64)?,
        limits.max_resident_bytes,
    )?;
    for (i, pixel) in native.chunks_exact(channels as usize).enumerate() {
        if i % 1024 == 0 {
            control.check()?;
        }
        match channels {
            1 => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
            2 => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            3 => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
            4 => rgba.extend_from_slice(pixel),
            _ => return Err(image_error(image_index, "component count")),
        }
    }
    control.check()?;
    Ok(DecodedImage {
        width,
        height,
        rgba,
    })
}

fn native_output(
    width: u32,
    height: u32,
    channels: u64,
    total: u64,
    limits: ImageLimits,
    control: Control<'_>,
) -> Result<Vec<u8>> {
    admit_dimensions(width, height, limits)?;
    let pixels = mul(u64::from(width), u64::from(height))?;
    let native_bytes = mul(pixels, channels)?;
    let rgba_bytes = mul(pixels, 4)?;
    if native_bytes != total {
        return Err(Error::InvalidAsset(
            "decoder byte count disagrees with dimensions",
        ));
    }
    cap(
        "decode output overlap",
        add(native_bytes, rgba_bytes)?,
        limits.max_resident_bytes,
    )?;
    let length =
        usize::try_from(native_bytes).map_err(|_| Error::InvalidLimit("native output overflow"))?;
    let mut native = vec_with_capacity(length)?;
    cap(
        "decode output overlap",
        add(native.capacity() as u64, rgba_bytes)?,
        limits.max_resident_bytes,
    )?;
    while native.len() < length {
        control.check()?;
        native.resize(native.len().saturating_add(4096).min(length), 0);
    }
    Ok(native)
}

fn admit_dimensions(width: u32, height: u32, limits: ImageLimits) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidAsset("zero image dimension"));
    }
    cap(
        "image width",
        u64::from(width),
        u64::from(limits.max_dimension),
    )?;
    cap(
        "image height",
        u64::from(height),
        u64::from(limits.max_dimension),
    )?;
    cap(
        "image pixels",
        mul(u64::from(width), u64::from(height))?,
        limits.max_pixels,
    )
}

fn png_profile(
    index: usize,
    bytes: &[u8],
    limits: ImageLimits,
    control: Control<'_>,
) -> Result<()> {
    if bytes.len() < 33 || &bytes[12..16] != b"IHDR" || bytes[8..12] != 13u32.to_be_bytes() {
        return Err(image_error(index, "truncated or invalid PNG IHDR"));
    }
    let u32_be = |s: &[u8]| -> Result<u32> {
        Ok(u32::from_be_bytes(
            s.try_into()
                .map_err(|_| image_error(index, "truncated PNG length"))?,
        ))
    };
    admit_dimensions(u32_be(&bytes[16..20])?, u32_be(&bytes[20..24])?, limits)?;
    if bytes[24] != 8 {
        return Err(image_error(
            index,
            "PNG profile requires 8-bit source components",
        ));
    }
    let mut offset = 8usize;
    let mut end = false;
    while offset < bytes.len() {
        control.check()?;
        let header = bytes
            .get(offset..offset + 8)
            .ok_or_else(|| image_error(index, "truncated PNG chunk"))?;
        let length = u32_be(&header[..4])? as usize;
        let next = offset
            .checked_add(length)
            .and_then(|v| v.checked_add(12))
            .ok_or_else(|| image_error(index, "PNG length overflow"))?;
        if next > bytes.len() {
            return Err(image_error(index, "truncated PNG chunk payload"));
        }
        if &header[4..8] == b"acTL" {
            return Err(image_error(
                index,
                "animated PNG is outside the still-image profile",
            ));
        }
        if &header[4..8] == b"IEND" {
            end = true;
            break;
        }
        offset = next;
    }
    if !end {
        return Err(image_error(index, "missing PNG IEND"));
    }
    Ok(())
}

pub fn mip_chain(
    source: &DecodedImage,
    transfer: ColorInterpretation,
    limits: ImageLimits,
    control: Control<'_>,
    work: &mut u64,
) -> Result<Vec<Mip>> {
    control.check()?;
    admit_dimensions(source.width, source.height, limits)?;
    if source.rgba.len() as u64 != mul(mul(u64::from(source.width), u64::from(source.height))?, 4)?
    {
        return Err(Error::InvalidAsset("decoded RGBA dimensions"));
    }
    let mut width = source.width;
    let mut height = source.height;
    let mut bytes = 0u64;
    let mut count = 0;
    loop {
        bytes = add(bytes, mul(mul(u64::from(width), u64::from(height))?, 4)?)?;
        count += 1;
        if width == 1 && height == 1 {
            break;
        }
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    let source_bytes = source.rgba.capacity() as u64;
    cap(
        "image mip bytes",
        add(
            add(bytes, source_bytes)?,
            mul(count as u64, size_of::<Mip>() as u64)?,
        )?,
        limits.max_resident_bytes,
    )?;
    let mut levels = vec_with_capacity(count)?;
    let mut resident = add(
        source_bytes,
        mul(levels.capacity() as u64, size_of::<Mip>() as u64)?,
    )?;
    cap(
        "image mip bytes",
        add(resident, bytes)?,
        limits.max_resident_bytes,
    )?;
    let mut first = vec_with_capacity(source.rgba.len())?;
    resident = add(resident, first.capacity() as u64)?;
    cap("image mip bytes", resident, limits.max_resident_bytes)?;
    for chunk in source.rgba.chunks(4096) {
        control.check()?;
        first.extend_from_slice(chunk);
    }
    levels.push(Mip {
        width: source.width,
        height: source.height,
        rgba: first,
    });
    while let Some(previous) = levels.last() {
        if previous.width == 1 && previous.height == 1 {
            break;
        }
        let width = (previous.width / 2).max(1);
        let height = (previous.height / 2).max(1);
        let next_bytes = mul(mul(u64::from(width), u64::from(height))?, 4)?;
        cap(
            "image mip bytes",
            add(resident, next_bytes)?,
            limits.max_resident_bytes,
        )?;
        let next_len =
            usize::try_from(next_bytes).map_err(|_| Error::InvalidLimit("mip output overflow"))?;
        let mut rgba = vec_with_capacity(next_len)?;
        resident = add(resident, rgba.capacity() as u64)?;
        cap("image mip bytes", resident, limits.max_resident_bytes)?;
        for y in 0..height {
            control.check()?;
            let y0 = f64::from(y) * f64::from(previous.height) / f64::from(height);
            let y1 = f64::from(y + 1) * f64::from(previous.height) / f64::from(height);
            for x in 0..width {
                if x % 256 == 0 {
                    control.check()?;
                }
                let x0 = f64::from(x) * f64::from(previous.width) / f64::from(width);
                let x1 = f64::from(x + 1) * f64::from(previous.width) / f64::from(width);
                let mut sum = [0.0f64; 4];
                let mut total = 0.0;
                for sy in y0.floor() as u32..y1.ceil() as u32 {
                    for sx in x0.floor() as u32..x1.ceil() as u32 {
                        *work = add(*work, 4)?;
                        cap("mip filter work", *work, limits.max_mip_work)?;
                        let weight = (x1.min(f64::from(sx + 1)) - x0.max(f64::from(sx)))
                            * (y1.min(f64::from(sy + 1)) - y0.max(f64::from(sy)));
                        let index = (sy as usize * previous.width as usize + sx as usize) * 4;
                        for (lane, sum) in sum.iter_mut().enumerate() {
                            let encoded = f64::from(previous.rgba[index + lane]) / 255.0;
                            let value = if transfer == ColorInterpretation::SrgbColor && lane < 3 {
                                srgb_to_linear(encoded)
                            } else {
                                encoded
                            };
                            *sum += weight * value;
                        }
                        total += weight;
                    }
                }
                for (lane, sum) in sum.into_iter().enumerate() {
                    let value = sum / total;
                    let value = if transfer == ColorInterpretation::SrgbColor && lane < 3 {
                        linear_to_srgb(value)
                    } else {
                        value
                    };
                    rgba.push((value * 255.0).round().clamp(0.0, 255.0) as u8);
                }
            }
        }
        levels.push(Mip {
            width,
            height,
            rgba,
        });
    }
    control.check()?;
    Ok(levels)
}

impl PreparedImages {
    pub fn prepare(
        asset: &ApplicationAsset,
        materials: &[CompiledMaterial],
        limits: ImageLimits,
        control: Control<'_>,
    ) -> Result<Self> {
        control.check()?;
        // Compiled materials own their slots; callers may have edited either
        // those slots or the asset tables since compilation.
        for material in materials {
            control.check()?;
            for slot in material.slots.iter().flatten() {
                control.check()?;
                asset
                    .images
                    .get(slot.image)
                    .ok_or(Error::InvalidAsset("image index"))?;
                let sampler = asset
                    .samplers
                    .get(slot.sampler)
                    .ok_or(Error::InvalidAsset("sampler index"))?;
                sampler_descriptor(sampler.state)?;
            }
        }
        let mut out = Self {
            variants: Vec::new(),
            encoded_bytes: 0,
            resident_bytes: 0,
            mip_work: 0,
        };
        let transfers = [
            ColorInterpretation::SrgbColor,
            ColorInterpretation::LinearColor,
            ColorInterpretation::Data,
        ];
        for image in 0..asset.images.len() {
            control.check()?;
            let mut uses = [false; 3];
            for material in materials {
                control.check()?;
                for slot in material.slots.iter().flatten().filter(|s| s.image == image) {
                    let transfer = transfers
                        .iter()
                        .position(|&t| t == slot.transfer)
                        .ok_or(Error::InvalidAsset("image color interpretation"))?;
                    uses[transfer] = true;
                }
            }
            let variants = uses.iter().filter(|&&used| used).count();
            if variants == 0 {
                continue;
            }
            let source = &asset.images[image];
            out.encoded_bytes = add(out.encoded_bytes, source.encoded_bytes.len() as u64)?;
            cap(
                "active encoded bytes",
                out.encoded_bytes,
                limits.max_active_encoded,
            )?;
            let previous_table = mul(
                out.variants.capacity() as u64,
                size_of::<ImageVariant>() as u64,
            )?;
            let resident_without_table = out
                .resident_bytes
                .checked_sub(previous_table)
                .ok_or(Error::InvalidLimit("resident image table"))?;
            let count = add(out.variants.len() as u64, variants as u64)?
                .max(out.variants.capacity() as u64);
            let admission = if count > out.variants.capacity() as u64 {
                // A growing Vec may keep its old table live during relocation.
                add(
                    out.resident_bytes,
                    mul(count, size_of::<ImageVariant>() as u64)?,
                )?
            } else {
                out.resident_bytes
            };
            cap(
                "resident image variants",
                admission,
                limits.max_resident_bytes,
            )?;
            out.variants.try_reserve_exact(variants)?;
            out.resident_bytes = add(
                resident_without_table,
                mul(
                    out.variants.capacity() as u64,
                    size_of::<ImageVariant>() as u64,
                )?,
            )?;
            cap(
                "resident image variants",
                out.resident_bytes,
                limits.max_resident_bytes,
            )?;
            let remaining = limits
                .max_resident_bytes
                .checked_sub(out.resident_bytes)
                .ok_or(Error::InvalidLimit("resident image budget"))?;
            let local = ImageLimits {
                max_resident_bytes: remaining,
                ..limits
            };
            let decoded = decode(
                image,
                &source.encoded_bytes,
                source.media_type.as_deref(),
                local,
                control,
            )?;
            for (transfer, used) in transfers.into_iter().zip(uses) {
                if !used {
                    continue;
                }
                let remaining = limits
                    .max_resident_bytes
                    .checked_sub(out.resident_bytes)
                    .ok_or(Error::InvalidLimit("resident image budget"))?;
                let levels = mip_chain(
                    &decoded,
                    transfer,
                    ImageLimits {
                        max_resident_bytes: remaining,
                        ..limits
                    },
                    control,
                    &mut out.mip_work,
                )?;
                out.resident_bytes = add(
                    out.resident_bytes,
                    mul(levels.capacity() as u64, size_of::<Mip>() as u64)?,
                )?;
                for level in &levels {
                    out.resident_bytes = add(out.resident_bytes, level.rgba.capacity() as u64)?;
                }
                cap(
                    "resident image variants",
                    out.resident_bytes,
                    limits.max_resident_bytes,
                )?;
                out.variants.push(ImageVariant {
                    image,
                    transfer,
                    levels,
                });
            }
        }
        control.check()?;
        Ok(out)
    }
}

pub fn sampler_descriptor(state: SamplerState) -> Result<wgpu::SamplerDescriptor<'static>> {
    fn filter(f: TexelFilter) -> Result<wgpu::FilterMode> {
        match f {
            TexelFilter::Nearest => Ok(wgpu::FilterMode::Nearest),
            TexelFilter::Linear => Ok(wgpu::FilterMode::Linear),
            _ => Err(Error::UnsupportedSampler),
        }
    }
    fn address(a: TextureAddressMode) -> Result<wgpu::AddressMode> {
        match a {
            TextureAddressMode::Repeat => Ok(wgpu::AddressMode::Repeat),
            TextureAddressMode::MirroredRepeat => Ok(wgpu::AddressMode::MirrorRepeat),
            TextureAddressMode::ClampToEdge | TextureAddressMode::Decal => {
                Ok(wgpu::AddressMode::ClampToEdge)
            }
            _ => Err(Error::UnsupportedSampler),
        }
    }
    let (mipmap_filter, lod_max_clamp) = match state.mipmap() {
        MipmapFilter::None => (wgpu::MipmapFilterMode::Nearest, 0.0),
        MipmapFilter::Nearest => (wgpu::MipmapFilterMode::Nearest, 32.0),
        MipmapFilter::Linear => (wgpu::MipmapFilterMode::Linear, 32.0),
        _ => return Err(Error::UnsupportedSampler),
    };
    Ok(wgpu::SamplerDescriptor {
        label: Some("retained sampler"),
        address_mode_u: address(state.address_u())?,
        address_mode_v: address(state.address_v())?,
        address_mode_w: address(state.address_w())?,
        mag_filter: filter(state.magnification())?,
        min_filter: filter(state.minification())?,
        mipmap_filter,
        lod_min_clamp: 0.0,
        lod_max_clamp,
        compare: None,
        anisotropy_clamp: 1,
        border_color: None,
    })
}
