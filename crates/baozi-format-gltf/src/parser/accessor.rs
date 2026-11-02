//! Shared dense, sparse and implicit-zero accessor validation before typed materialization.

use super::*;

pub(super) fn bind<'bytes>(
    ctx: &ParseScope<'_>,
    accessor: crate::document::Accessor<'_>,
    buffers: &'bytes [Option<BufferData<'_>>],
) -> Result<crate::document::read::Bound<'bytes>> {
    Ok(crate::document::read::bind(ctx, accessor, |index| {
        buffers
            .get(index)
            .and_then(Option::as_ref)
            .map(BufferData::as_slice)
    })?)
}

pub(super) fn vectors<'bytes, const N: usize>(
    ctx: &ParseScope<'_>,
    accessor: crate::document::Accessor<'_>,
    buffers: &'bytes [Option<BufferData<'_>>],
) -> Result<crate::document::read::Floats<'bytes, N>> {
    bind(ctx, accessor, buffers)?.vectors().ok_or_else(|| {
        GltfFailure::invalid_accessor(
            Some(accessor.index()),
            "accessor vector shape differs from its contract",
        )
        .into()
    })
}

pub(super) fn optional_vectors<'bytes, const N: usize>(
    ctx: &ParseScope<'_>,
    accessor: Option<crate::document::Accessor<'_>>,
    buffers: &'bytes [Option<BufferData<'_>>],
) -> Result<Option<crate::document::read::Floats<'bytes, N>>> {
    accessor
        .map(|accessor| vectors(ctx, accessor, buffers))
        .transpose()
}

#[derive(Clone, Copy)]
pub(super) struct AccessorContract {
    components: &'static [json::accessor::ComponentType],
    dimensions: &'static [json::accessor::Type],
    allow_stride: bool,
    require_normalized_integer: bool,
}

impl AccessorContract {
    pub(super) fn vertex(
        components: &'static [json::accessor::ComponentType],
        dimensions: &'static [json::accessor::Type],
    ) -> Self {
        Self {
            components,
            dimensions,
            allow_stride: true,
            require_normalized_integer: false,
        }
    }

    pub(super) fn tightly_packed(
        components: &'static [json::accessor::ComponentType],
        dimensions: &'static [json::accessor::Type],
    ) -> Self {
        Self {
            components,
            dimensions,
            allow_stride: false,
            require_normalized_integer: false,
        }
    }

    pub(super) fn require_normalized_integer(mut self) -> Self {
        self.require_normalized_integer = true;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AccessorInfo {
    pub(super) count: usize,
}

pub(super) fn validate_accessor(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    accessor_index: usize,
    owner: AccessorOwner,
    label: &'static str,
    contract: AccessorContract,
) -> Result<AccessorInfo> {
    let accessor = gltf.tables().accessors.get(accessor_index).ok_or_else(|| {
        GltfFailure::invalid_accessor(
            Some(accessor_index),
            format_args!("{owner} {label} accessor reference is out of range"),
        )
    })?;
    let component = accessor.component_type;
    let dimension = accessor.type_;
    if !contract.components.contains(&component) || !contract.dimensions.contains(&dimension) {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!("{owner} {label} accessor type is unsupported"),
        ));
    }
    if accessor.normalized && component == json::accessor::ComponentType::F32 {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!("{owner} float {label} accessor must not be normalized"),
        ));
    }
    if accessor.normalized
        && (component == json::accessor::ComponentType::U32 || !contract.require_normalized_integer)
    {
        return Err(GltfFailure::invalid_accessor(
            Some(accessor_index),
            format_args!("{owner} {label} accessor must not be normalized"),
        )
        .into());
    }
    if contract.require_normalized_integer
        && component != json::accessor::ComponentType::F32
        && !accessor.normalized
    {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!("{owner} integer {label} accessor must be normalized"),
        ));
    }
    validate_accessor_layout(
        ctx,
        gltf,
        AccessorLayoutCheck {
            index: accessor_index,
            accessor,
            owner,
            label,
            component,
            dimension,
            contract,
        },
    )?;

    // Unreplaced elements without a base view are zero, which cannot be unit directions.
    // Morph NORMAL/TANGENT deltas use a separate contract and may legitimately be zero.
    if matches!(owner, AccessorOwner::Primitive { .. })
        && matches!(label, "NORMAL" | "TANGENT")
        && accessor.buffer_view.is_none()
        && accessor
            .sparse
            .as_ref()
            .is_none_or(|sparse| sparse.count < accessor.count)
    {
        return Err(GltfFailure::invalid_accessor(
            Some(accessor_index),
            format_args!("{owner} {label} accessor has implicit zero direction values"),
        )
        .into());
    }

    let count = usize::try_from(accessor.count)
        .map_err(|_| ctx.allocation_failure("gltf-accessor-count"))?;
    Ok(AccessorInfo { count })
}

#[derive(Clone, Copy)]
struct AccessorLayoutCheck<'a> {
    index: usize,
    accessor: &'a json::accessor::Accessor,
    owner: AccessorOwner,
    label: &'static str,
    component: json::accessor::ComponentType,
    dimension: json::accessor::Type,
    contract: AccessorContract,
}

fn validate_accessor_layout(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    check: AccessorLayoutCheck<'_>,
) -> Result<()> {
    let invalid = |reason| GltfFailure::invalid_accessor(Some(check.index), reason);
    if check.accessor.count == 0 {
        return Err(invalid("accessor count must be positive").into());
    }
    let component_size = component_size(check.component);
    let element_size = element_size(check.component, check.dimension);
    // Check the complete logical range even when most elements have implicit zero values.
    // Current admitted matrix uses are F32 MAT4, so no sub-word matrix padding is needed.
    check
        .accessor
        .count
        .checked_mul(element_size)
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| ctx.allocation_failure("gltf-accessor-decoded-size"))?;

    if check.accessor.buffer_view.is_some() {
        validate_base_layout(ctx, gltf, check, component_size, element_size)?;
    } else if check.accessor.byte_offset.is_some() {
        return Err(invalid("accessor without bufferView must not define byteOffset").into());
    }

    let Some(sparse) = &check.accessor.sparse else {
        return Ok(());
    };
    if sparse.count == 0 || sparse.count > check.accessor.count {
        return Err(
            invalid("sparse count must be positive and no greater than accessor count").into(),
        );
    }
    // The private sequential sparse reader uses a u32 logical element cursor.
    ctx.enforce_resource_limit(
        "gltf-sparse-accessor-count",
        check.accessor.count,
        u64::from(u32::MAX),
    )?;
    let index_component = sparse.indices.component_type;
    let index_size = match index_component {
        json::accessor::ComponentType::U8 => 1,
        json::accessor::ComponentType::U16 => 2,
        json::accessor::ComponentType::U32 => 4,
        _ => return Err(invalid("sparse indices must use U8, U16 or U32").into()),
    };
    for (view, offset, alignment, size) in [
        (
            sparse.indices.buffer_view.value(),
            sparse.indices.byte_offset,
            index_size,
            index_size,
        ),
        (
            sparse.values.buffer_view.value(),
            sparse.values.byte_offset,
            component_size,
            element_size,
        ),
    ] {
        let buffer_view = checked_buffer_view(gltf, check, view)?;
        if buffer_view.byte_stride.is_some() || buffer_view.target.is_some() {
            return Err(invalid("sparse bufferView must not define byteStride or target").into());
        }
        let view_offset = buffer_view.byte_offset.map_or(0, |offset| offset);
        if !offset.is_multiple_of(alignment) || !view_offset.is_multiple_of(alignment) {
            return Err(invalid("sparse offsets must each align to component size").into());
        }
        let end = sparse
            .count
            .checked_mul(size)
            .and_then(|size| offset.checked_add(size))
            .ok_or_else(|| invalid("sparse accessor range overflow"))?;
        if end > buffer_view.byte_length {
            return Err(invalid("sparse bufferView byteLength does not cover data").into());
        }
    }
    Ok(())
}

fn checked_buffer_view<'a>(
    gltf: &'a crate::document::Document<'_>,
    check: AccessorLayoutCheck<'_>,
    index: usize,
) -> Result<&'a json::buffer::View> {
    let invalid = |reason| GltfFailure::invalid_accessor(Some(check.index), reason);
    let view = gltf
        .tables()
        .buffer_views
        .get(index)
        .ok_or_else(|| invalid("accessor bufferView reference is out of range"))?;
    let buffer = gltf
        .tables()
        .buffers
        .get(view.buffer.value())
        .ok_or_else(|| invalid("accessor buffer reference is out of range"))?;
    let end = view
        .byte_offset
        .map_or(0, |offset| offset)
        .checked_add(view.byte_length)
        .ok_or_else(|| invalid("accessor bufferView range overflow"))?;
    if end > buffer.byte_length {
        return Err(invalid("accessor bufferView exceeds declared buffer byteLength").into());
    }
    Ok(view)
}

fn validate_base_layout(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    check: AccessorLayoutCheck<'_>,
    component_size: u64,
    element_size: u64,
) -> Result<()> {
    let view = check.accessor.buffer_view.ok_or_else(|| {
        GltfFailure::internal("base layout validation requires an accessor bufferView")
    })?;
    let buffer_view = checked_buffer_view(gltf, check, view.value())?;
    let stride = match buffer_view.byte_stride {
        Some(_) if !check.contract.allow_stride => {
            return Err(ContentError::parse(
                ctx.source(),
                None,
                format_args!(
                    "{} {} accessor uses byteStride where it is unsupported",
                    check.owner, check.label
                ),
            ));
        }
        Some(stride) if !stride.is_multiple_of(4) => {
            return Err(GltfFailure::invalid_accessor(
                Some(check.index),
                "accessor byteStride must be a multiple of four",
            )
            .into());
        }
        Some(stride) => stride as u64,
        None => element_size,
    };
    if stride < element_size {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!(
                "{} {} byteStride is smaller than the accessor element size",
                check.owner, check.label
            ),
        ));
    }
    if !stride.is_multiple_of(component_size) {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!(
                "{} {} byteStride is not aligned to component size",
                check.owner, check.label
            ),
        ));
    }

    let view_offset = buffer_view.byte_offset.map_or(0, |offset| offset);
    let view_length = buffer_view.byte_length;
    let accessor_offset = check.accessor.byte_offset.map_or(0, |offset| offset);
    if check.contract.allow_stride && !accessor_offset.is_multiple_of(4) {
        return Err(GltfFailure::invalid_accessor(
            Some(check.index),
            "vertex accessor byteOffset must be a multiple of four",
        )
        .into());
    }
    if !accessor_offset.is_multiple_of(component_size)
        || !view_offset.is_multiple_of(component_size)
    {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!(
                "{} {} byte offset is not aligned to component size",
                check.owner, check.label
            ),
        ));
    }

    let required =
        required_accessor_bytes(check.accessor.count, element_size, stride).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format_args!("{} {} byte range overflow", check.owner, check.label),
            )
        })?;
    let accessor_end = accessor_offset.checked_add(required).ok_or_else(|| {
        gltf_parse_error(
            ctx,
            format_args!("{} {} accessor range overflow", check.owner, check.label),
        )
    })?;
    if accessor_end > view_length {
        return Err(ContentError::parse(
            ctx.source(),
            None,
            format_args!(
                "{} {} bufferView byteLength does not cover accessor data",
                check.owner, check.label
            ),
        ));
    }

    Ok(())
}

/// Validate each selected sparse index stream once, before any accessor is materialized.
/// The sequential backend reader assumes ordered indices; malformed streams would otherwise
/// silently retain base values. Borrow the admitted buffers instead of collecting another table.
pub(super) fn validate_sparse_indices(
    ctx: &ParseScope<'_>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    closure: &ResourceClosure<'_>,
) -> Result<()> {
    use crate::document::IndexType;

    for (ordinal, accessor) in gltf.accessors().enumerate() {
        ctx.charge_work(1)?;
        poll_parser_loop(ctx, ordinal)?;
        if !closure.accessors.as_slice()[ordinal] {
            continue;
        }
        let Some(sparse) = accessor.sparse() else {
            continue;
        };
        let invalid = |reason| GltfFailure::invalid_accessor(Some(accessor.index()), reason);
        let indices = sparse.indices();
        let view = indices.view();
        let buffer = buffers
            .get(view.buffer().index())
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid("sparse index buffer is unavailable"))?;
        let index_size = indices.index_type().size();
        let start = view
            .offset()
            .checked_add(indices.offset())
            .ok_or_else(|| invalid("sparse index offset overflow"))?;
        let end = sparse
            .count()
            .checked_mul(index_size)
            .and_then(|size| start.checked_add(size))
            .ok_or_else(|| invalid("sparse index range overflow"))?;
        let bytes = buffer
            .as_slice()
            .get(start..end)
            .ok_or_else(|| invalid("sparse index data is truncated"))?;
        ctx.with_work(
            ImportWorkClass::DecodeWork,
            to_u64(sparse.count()),
            |work| -> Result<()> {
                let mut previous = None;
                for chunk in bytes.chunks(PARSER_POLL_INTERVAL * index_size) {
                    work.poll()?;
                    for value in chunk.chunks_exact(index_size) {
                        let index = match (indices.index_type(), value) {
                            (IndexType::U8, &[x]) => u32::from(x),
                            (IndexType::U16, &[x, y]) => u32::from(u16::from_le_bytes([x, y])),
                            (IndexType::U32, &[x, y, z, w]) => u32::from_le_bytes([x, y, z, w]),
                            _ => return Err(invalid("sparse index encoding is invalid").into()),
                        };
                        if index as usize >= accessor.count()
                            || previous.is_some_and(|previous| index <= previous)
                        {
                            return Err(invalid(
                                "sparse indices must strictly increase within accessor count",
                            )
                            .into());
                        }
                        previous = Some(index);
                    }
                    work.complete([to_u64(chunk.len() / index_size)])?;
                }
                Ok(())
            },
        )?;
    }
    Ok(())
}

fn component_size(component: json::accessor::ComponentType) -> u64 {
    match component {
        json::accessor::ComponentType::I8 | json::accessor::ComponentType::U8 => 1,
        json::accessor::ComponentType::I16 | json::accessor::ComponentType::U16 => 2,
        json::accessor::ComponentType::U32 | json::accessor::ComponentType::F32 => 4,
    }
}

fn component_count(dimension: json::accessor::Type) -> u64 {
    match dimension {
        json::accessor::Type::Scalar => 1,
        json::accessor::Type::Vec2 => 2,
        json::accessor::Type::Vec3 => 3,
        json::accessor::Type::Vec4 | json::accessor::Type::Mat2 => 4,
        json::accessor::Type::Mat3 => 9,
        json::accessor::Type::Mat4 => 16,
    }
}

fn element_size(component: json::accessor::ComponentType, dimension: json::accessor::Type) -> u64 {
    component_size(component) * component_count(dimension)
}

fn required_accessor_bytes(count: u64, element_size: u64, stride: u64) -> Option<u64> {
    if count == 0 {
        return Some(0);
    }
    count
        .checked_sub(1)?
        .checked_mul(stride)?
        .checked_add(element_size)
}
