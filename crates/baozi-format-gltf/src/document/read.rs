//! One checked dense/sparse byte binding, independent of document and buffer-owner lifetimes.

use super::{
    Accessor,
    schema::accessor::{ComponentType as C, Type},
};
use crate::GltfFailure;
use baozi_import::{ParseResult, ParseScope};

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
    ctx: &ParseScope<'_>,
    accessor: Accessor<'_>,
    mut buffer: impl FnMut(usize) -> Option<&'bytes [u8]>,
) -> ParseResult<Bound<'bytes>> {
    let fail = |reason| {
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
        .ok_or_else(|| fail("accessor element size overflow"))?;
    let count =
        usize::try_from(row.count).map_err(|_| fail("accessor count exceeds host domain"))?;
    if count == 0 {
        return Err(fail("accessor count is zero"));
    }
    let view_bytes = |id: super::schema::Index,
                      buffer: &mut dyn FnMut(usize) -> Option<&'bytes [u8]>|
     -> ParseResult<&'bytes [u8]> {
        let view = accessor
            .doc
            .tables
            .buffer_views
            .get(id.value())
            .ok_or_else(|| fail("accessor view is missing"))?;
        let start = usize::try_from(view.byte_offset.map_or(0, |offset| offset))
            .map_err(|_| fail("view offset exceeds host domain"))?;
        let length = usize::try_from(view.byte_length)
            .map_err(|_| fail("view length exceeds host domain"))?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| fail("view byte range overflow"))?;
        buffer(view.buffer.value())
            .and_then(|bytes| bytes.get(start..end))
            .ok_or_else(|| fail("accessor buffer is unavailable or truncated"))
    };
    let base = row
        .buffer_view
        .map(|id| -> ParseResult<_> {
            let view = &accessor.doc.tables.buffer_views[id.value()];
            let bytes = view_bytes(id, &mut buffer)?;
            let stride = view.byte_stride.map_or(element, |stride| stride);
            if stride < element {
                return Err(fail("accessor stride is smaller than its element"));
            }
            let start = usize::try_from(row.byte_offset.map_or(0, |offset| offset))
                .map_err(|_| fail("accessor offset exceeds host domain"))?;
            let length = (count - 1)
                .checked_mul(stride)
                .and_then(|size| size.checked_add(element))
                .ok_or_else(|| fail("accessor byte range overflow"))?;
            let end = start
                .checked_add(length)
                .ok_or_else(|| fail("accessor byte range overflow"))?;
            Ok(Stream {
                bytes: bytes
                    .get(start..end)
                    .ok_or_else(|| fail("accessor data is truncated"))?,
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
                _ => return Err(fail("invalid sparse index component")),
            };
            let count = usize::try_from(sparse.count)
                .map_err(|_| fail("sparse count exceeds host domain"))?;
            if count == 0 || count > row.count as usize {
                return Err(fail("sparse count is out of range"));
            }
            let slice = |bytes: &'bytes [u8], offset: u64, width: usize| -> ParseResult<_> {
                let start = usize::try_from(offset)
                    .map_err(|_| fail("sparse offset exceeds host domain"))?;
                let end = count
                    .checked_mul(width)
                    .and_then(|size| start.checked_add(size))
                    .ok_or_else(|| fail("sparse byte range overflow"))?;
                bytes
                    .get(start..end)
                    .ok_or_else(|| fail("sparse data is truncated"))
            };
            Ok(Sparse {
                indices: slice(
                    view_bytes(sparse.indices.buffer_view, &mut buffer)?,
                    sparse.indices.byte_offset,
                    index_size,
                )?,
                values: slice(
                    view_bytes(sparse.values.buffer_view, &mut buffer)?,
                    sparse.values.byte_offset,
                    element,
                )?,
                count,
                index_size,
            })
        })
        .transpose()?;
    if base.is_none() && sparse.is_none() {
        return Err(fail("accessor has neither a base view nor sparse data"));
    }
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
fn float(component: C, normalized: bool, bits: u32) -> f32 {
    match (component, normalized) {
        (C::F32, _) => f32::from_bits(bits),
        (C::U8, true) => bits as f32 * 255.0_f32.recip(),
        (C::U16, true) => bits as f32 * 65535.0_f32.recip(),
        (C::I8, true) => ((bits as u8 as i8) as f32 * 127.0_f32.recip()).max(-1.),
        (C::I16, true) => ((bits as u16 as i16) as f32 * 32767.0_f32.recip()).max(-1.),
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
    pub(crate) fn scalars(self) -> Option<impl ExactSizeIterator<Item = f32> + 'bytes> {
        self.vectors::<1>()
            .map(|values| values.map(|value| value[0]))
    }
    pub(crate) fn rgba(self) -> Option<impl ExactSizeIterator<Item = [f32; 4]> + 'bytes> {
        match self.dimension {
            Type::Vec3 => Some(Colors::Rgb(self.vectors::<3>()?)),
            Type::Vec4 => Some(Colors::Rgba(self.vectors::<4>()?)),
            _ => None,
        }
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
enum Colors<'bytes> {
    Rgb(Floats<'bytes, 3>),
    Rgba(Floats<'bytes, 4>),
}
impl Iterator for Colors<'_> {
    type Item = [f32; 4];
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Rgb(values) => values.next().map(|[r, g, b]| [r, g, b, 1.]),
            Self::Rgba(values) => values.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Rgb(values) => values.size_hint(),
            Self::Rgba(values) => values.size_hint(),
        }
    }
}
impl ExactSizeIterator for Colors<'_> {}
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
