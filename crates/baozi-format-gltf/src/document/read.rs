//! One checked dense, sparse or implicit-zero binding, independent of buffer-owner lifetimes.

use super::{
    Accessor,
    schema::accessor::{ComponentType as C, Type},
};
use crate::GltfFailure;
use baozi_import::{ParseResult, ParseServices};

#[derive(Clone, Copy)]
struct Stream<'bytes> {
    bytes: &'bytes [u8],
    stride: usize,
}

#[derive(Clone, Copy)]
struct Sparse<'bytes> {
    indices: &'bytes [u8],
    index_size: usize,
    values: &'bytes [u8],
    count: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct Bound<'bytes> {
    base: Option<Stream<'bytes>>,
    sparse: Option<Sparse<'bytes>>,
    count: usize,
    component: C,
    dimension: Type,
    normalized: bool,
}

pub(crate) fn bind<'bytes>(
    ctx: &mut ParseServices<'_, '_, '_>,
    accessor: Accessor<'_>,
    mut buffer: impl FnMut(usize) -> Option<&'bytes [u8]>,
) -> ParseResult<Bound<'bytes>> {
    let fail = |reason, ctx: &mut ParseServices<'_, '_, '_>| {
        ctx.format_failure(GltfFailure::invalid_accessor(
            Some(accessor.index()),
            reason,
        ))
    };
    let row = accessor.row();
    let component = row.component_type;
    let dimension = row.type_;
    let size = component_size(component);
    let element = size
        .checked_mul(lanes(dimension))
        .ok_or_else(|| fail("accessor element size overflow", ctx))?;
    let count =
        usize::try_from(row.count).map_err(|_| fail("accessor count exceeds host domain", ctx))?;
    if count == 0 {
        return Err(fail("accessor count is zero", ctx));
    }
    count
        .checked_mul(element)
        .ok_or_else(|| fail("accessor logical byte range overflow", ctx))?;
    if row.buffer_view.is_none() && row.byte_offset.is_some() {
        return Err(fail(
            "accessor without bufferView must not define byteOffset",
            ctx,
        ));
    }
    let view_bytes = |id: super::schema::Index,
                      buffer: &mut dyn FnMut(usize) -> Option<&'bytes [u8]>,
                      ctx: &mut ParseServices<'_, '_, '_>|
     -> ParseResult<&'bytes [u8]> {
        let view = accessor
            .doc
            .tables
            .buffer_views
            .get(id.value())
            .ok_or_else(|| fail("accessor view is missing", ctx))?;
        let start = usize::try_from(view.byte_offset.map_or(0, |offset| offset))
            .map_err(|_| fail("view offset exceeds host domain", ctx))?;
        let length = usize::try_from(view.byte_length)
            .map_err(|_| fail("view length exceeds host domain", ctx))?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| fail("view byte range overflow", ctx))?;
        buffer(view.buffer.value())
            .and_then(|bytes| bytes.get(start..end))
            .ok_or_else(|| fail("accessor buffer is unavailable or truncated", ctx))
    };
    let base = row
        .buffer_view
        .map(|id| -> ParseResult<_> {
            let view = &accessor.doc.tables.buffer_views[id.value()];
            let bytes = view_bytes(id, &mut buffer, ctx)?;
            let stride = view.byte_stride.map_or(element, |stride| stride);
            if stride < element {
                return Err(fail("accessor stride is smaller than its element", ctx));
            }
            let start = usize::try_from(row.byte_offset.map_or(0, |offset| offset))
                .map_err(|_| fail("accessor offset exceeds host domain", ctx))?;
            let length = (count - 1)
                .checked_mul(stride)
                .and_then(|size| size.checked_add(element))
                .ok_or_else(|| fail("accessor byte range overflow", ctx))?;
            let end = start
                .checked_add(length)
                .ok_or_else(|| fail("accessor byte range overflow", ctx))?;
            Ok(Stream {
                bytes: bytes
                    .get(start..end)
                    .ok_or_else(|| fail("accessor data is truncated", ctx))?,
                stride,
            })
        })
        .transpose()?;
    let sparse = row
        .sparse
        .as_ref()
        .map(|sparse| -> ParseResult<_> {
            let index_size = match sparse.indices.component_type {
                C::U8 => 1,
                C::U16 => 2,
                C::U32 => 4,
                _ => return Err(fail("invalid sparse index component", ctx)),
            };
            let count = usize::try_from(sparse.count)
                .map_err(|_| fail("sparse count exceeds host domain", ctx))?;
            if count == 0 || count > row.count as usize {
                return Err(fail("sparse count is out of range", ctx));
            }
            let slice = |bytes: &'bytes [u8],
                         offset: u64,
                         width: usize,
                         ctx: &mut ParseServices<'_, '_, '_>|
             -> ParseResult<_> {
                let start = usize::try_from(offset)
                    .map_err(|_| fail("sparse offset exceeds host domain", ctx))?;
                let end = count
                    .checked_mul(width)
                    .and_then(|size| start.checked_add(size))
                    .ok_or_else(|| fail("sparse byte range overflow", ctx))?;
                bytes
                    .get(start..end)
                    .ok_or_else(|| fail("sparse data is truncated", ctx))
            };
            Ok(Sparse {
                indices: slice(
                    view_bytes(sparse.indices.buffer_view, &mut buffer, ctx)?,
                    sparse.indices.byte_offset,
                    index_size,
                    ctx,
                )?,
                values: slice(
                    view_bytes(sparse.values.buffer_view, &mut buffer, ctx)?,
                    sparse.values.byte_offset,
                    element,
                    ctx,
                )?,
                count,
                index_size,
            })
        })
        .transpose()?;
    Ok(Bound {
        base,
        sparse,
        count,
        component,
        dimension,
        normalized: row.normalized,
    })
}

fn component_size(component: C) -> usize {
    match component {
        C::I8 | C::U8 => 1,
        C::I16 | C::U16 => 2,
        C::U32 | C::F32 => 4,
    }
}
fn lanes(dimension: Type) -> usize {
    match dimension {
        Type::Scalar => 1,
        Type::Vec2 => 2,
        Type::Vec3 => 3,
        Type::Vec4 | Type::Mat2 => 4,
        Type::Mat3 => 9,
        Type::Mat4 => 16,
    }
}
fn raw_component(bytes: &[u8], size: usize) -> Option<u32> {
    Some(match (size, bytes) {
        (1, &[a]) => u32::from(a),
        (2, &[a, b]) => u32::from(u16::from_le_bytes([a, b])),
        (4, &[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
        _ => return None,
    })
}
pub(crate) fn normalized_u8(value: u8) -> f32 {
    (f64::from(value) / 255.0) as f32
}

pub(crate) fn normalized_u16(value: u16) -> f32 {
    (f64::from(value) / 65535.0) as f32
}

fn float(component: C, normalized: bool, bits: u32) -> f32 {
    match (component, normalized) {
        (C::F32, _) => f32::from_bits(bits),
        (C::U8, true) => normalized_u8(bits as u8),
        (C::U16, true) => normalized_u16(bits as u16),
        (C::I8, true) => (f64::from((bits as u8 as i8).max(-127)) / 127.0) as f32,
        (C::I16, true) => (f64::from((bits as u16 as i16).max(-32767)) / 32767.0) as f32,
        (C::I8, false) => (bits as u8 as i8) as f32,
        (C::I16, false) => (bits as u16 as i16) as f32,
        (_, false) | (C::U32, true) => bits as f32,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Raw<'bytes, const N: usize> {
    bound: Bound<'bytes>,
    position: usize,
    sparse_position: usize,
}
impl<const N: usize> Iterator for Raw<'_, N> {
    type Item = [u32; N];
    fn next(&mut self) -> Option<Self::Item> {
        if self.position == self.bound.count {
            return None;
        }
        let size = component_size(self.bound.component);
        let width = size * N;
        let mut bytes = self.bound.base.and_then(|base| {
            base.bytes
                .get(self.position * base.stride..self.position * base.stride + width)
        });
        if let Some(sparse) = self.bound.sparse
            && self.sparse_position < sparse.count
        {
            let start = self.sparse_position * sparse.index_size;
            let index = raw_component(
                sparse.indices.get(start..start + sparse.index_size)?,
                sparse.index_size,
            )? as usize;
            if index == self.position {
                let start = self.sparse_position * width;
                bytes = Some(sparse.values.get(start..start + width)?);
                self.sparse_position += 1;
            }
        }
        let mut result = [0; N];
        if let Some(bytes) = bytes {
            for (lane, value) in result.iter_mut().enumerate() {
                *value = raw_component(bytes.get(lane * size..(lane + 1) * size)?, size)?;
            }
        }
        self.position += 1;
        Some(result)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let count = self.bound.count - self.position;
        (count, Some(count))
    }
}
impl<const N: usize> ExactSizeIterator for Raw<'_, N> {}

pub(crate) struct Floats<'bytes, const N: usize>(Raw<'bytes, N>);
impl<const N: usize> Iterator for Floats<'_, N> {
    type Item = [f32; N];
    fn next(&mut self) -> Option<Self::Item> {
        let raw = self.0.next()?;
        Some(raw.map(|value| float(self.0.bound.component, self.0.bound.normalized, value)))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
impl<const N: usize> ExactSizeIterator for Floats<'_, N> {}

pub(crate) struct Sample<const N: usize> {
    pub(crate) stored: [f32; N],
    pub(crate) decoded: [f32; N],
}

pub(crate) struct Samples<'bytes, const N: usize>(Raw<'bytes, N>);

impl<const N: usize> Iterator for Samples<'_, N> {
    type Item = Sample<N>;

    fn next(&mut self) -> Option<Self::Item> {
        let raw = self.0.next()?;
        let component = self.0.bound.component;
        Some(Sample {
            stored: raw.map(|value| float(component, false, value)),
            decoded: raw.map(|value| float(component, self.0.bound.normalized, value)),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<const N: usize> ExactSizeIterator for Samples<'_, N> {}

impl<'bytes> Bound<'bytes> {
    fn raw<const N: usize>(self) -> Option<Raw<'bytes, N>> {
        (lanes(self.dimension) == N).then_some(Raw {
            bound: self,
            position: 0,
            sparse_position: 0,
        })
    }
    pub(crate) fn vectors<const N: usize>(self) -> Option<Floats<'bytes, N>> {
        self.raw().map(Floats)
    }
    pub(crate) fn samples<const N: usize>(self) -> Option<Samples<'bytes, N>> {
        if !matches!(self.component, C::I8 | C::U8 | C::I16 | C::U16 | C::F32) {
            return None;
        }
        self.raw().map(Samples)
    }
    pub(crate) fn scalars(self) -> Option<impl ExactSizeIterator<Item = f32> + 'bytes> {
        self.vectors::<1>()
            .map(|values| values.map(|value| value[0]))
    }
    pub(crate) fn joints(self) -> Option<impl ExactSizeIterator<Item = [u16; 4]> + 'bytes> {
        matches!(self.component, C::U8 | C::U16).then_some(())?;
        Some(
            self.raw::<4>()?
                .map(|values| values.map(|value| value as u16)),
        )
    }
    pub(crate) fn weights(self) -> Option<Weights<'bytes>> {
        match self.component {
            C::U8 => Some(Weights::U8(self.raw()?)),
            C::U16 => Some(Weights::U16(self.raw()?)),
            C::F32 => Some(Weights::F32(self.vectors()?)),
            _ => None,
        }
    }
    pub(crate) fn indices(self) -> Option<Indices<'bytes>> {
        let restart = match self.component {
            C::U8 => u32::from(u8::MAX),
            C::U16 => u32::from(u16::MAX),
            C::U32 => u32::MAX,
            _ => return None,
        };
        Some(Indices {
            values: self.raw()?,
            restart,
        })
    }
    pub(crate) fn matrices(self) -> Option<impl ExactSizeIterator<Item = [[f32; 4]; 4]> + 'bytes> {
        if self.dimension != Type::Mat4 || self.component != C::F32 {
            return None;
        }
        Some(self.vectors::<16>()?.map(|value| {
            std::array::from_fn(|column| std::array::from_fn(|lane| value[column * 4 + lane]))
        }))
    }
}
pub(crate) enum Weights<'bytes> {
    U8(Raw<'bytes, 4>),
    U16(Raw<'bytes, 4>),
    F32(Floats<'bytes, 4>),
}
pub(crate) struct Indices<'bytes> {
    values: Raw<'bytes, 1>,
    pub(crate) restart: u32,
}
impl Iterator for Indices<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        self.values.next().map(|value| value[0])
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}
impl ExactSizeIterator for Indices<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_integer_witnesses_round_the_exact_rational_once() {
        // Independently computed with integer quotient/remainder, not a decoder/helper call.
        // For 3/255 = 1/85: 2^30 = 85 * 12632256 + 64, so the significand rounds up.
        for (component, raw, expected) in [
            (C::U8, 0, 0x0000_0000),
            (C::U8, 3, 0x3c40_c0c1),
            (C::U8, 255, 0x3f80_0000),
            (C::U16, 0, 0x0000_0000),
            (C::U16, 257, 0x3b80_8081),
            (C::U16, 65535, 0x3f80_0000),
            (C::I8, 0, 0x0000_0000),
            (C::I8, 9, 0x3d91_2245),
            (C::I8, (-9_i8) as u8 as u32, 0xbd91_2245),
            (C::I8, 127, 0x3f80_0000),
            (C::I8, (-127_i8) as u8 as u32, 0xbf80_0000),
            (C::I8, i8::MIN as u8 as u32, 0xbf80_0000),
            (C::I16, 0, 0x0000_0000),
            (C::I16, 513, 0x3c80_4101),
            (C::I16, (-513_i16) as u16 as u32, 0xbc80_4101),
            (C::I16, 32767, 0x3f80_0000),
            (C::I16, (-32767_i16) as u16 as u32, 0xbf80_0000),
            (C::I16, i16::MIN as u16 as u32, 0xbf80_0000),
        ] {
            assert_eq!(float(component, true, raw).to_bits(), expected);
        }
        assert_eq!(float(C::F32, false, 0x8000_0000).to_bits(), 0x8000_0000);
    }
}
