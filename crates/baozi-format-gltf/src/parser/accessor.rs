//! Shared dense, sparse and implicit-zero accessor validation before typed materialization.

use super::*;
use crate::document::mesh_encoding::{
    MeshAttributeUse, MeshEncodingClass, mesh_attribute_encoding,
};
use baozi_import::ParseServices;

pub(super) const MESH_SAMPLE_WORK_PER_LANE: u64 = 8;

// Baozi's FLOAT base-direction admission policy, not a glTF-specified epsilon.
// Quantized directions and signed morph displacements have separate contracts.
const FLOAT_BASE_DIRECTION_SQUARED_LENGTH_TOLERANCE: f64 = 1.0e-4;

pub(super) fn bind<'bytes>(
    ctx: &mut ParseServices<'_, '_, '_>,
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

pub(super) fn validate_mesh_attribute(
    ctx: &mut ParseRoot<'_, '_, '_>,
    gltf: &crate::document::Document<'_>,
    index: usize,
    owner: AccessorOwner,
    usage: MeshAttributeUse,
) -> Result<AccessorInfo> {
    let label = usage.label();
    let invalid = |reason| {
        GltfFailure::invalid_accessor(
            Some(index),
            format_args!("{reason}; {label} accessor in {owner}"),
        )
    };
    let accessor = gltf
        .tables()
        .accessors
        .get(index)
        .ok_or_else(|| invalid("mesh accessor reference is out of range"))?;
    if accessor.component_type == json::accessor::ComponentType::F32 && accessor.normalized {
        return Err(GltfFailure::invalid_accessor(
            Some(index),
            format_args!("{owner} float {label} accessor must not be normalized"),
        )
        .into());
    }
    let encoding = mesh_attribute_encoding(usage, accessor.component_type, accessor.normalized)
        .filter(|encoding| encoding.dimensions.contains(&accessor.type_))
        .ok_or_else(|| {
            GltfFailure::invalid_accessor(
                Some(index),
                format_args!("{label} accessor type, component or normalization is unsupported"),
            )
        })?;
    if encoding.class == MeshEncodingClass::Quantization && !gltf.has_mesh_quantization() {
        return Err(invalid("mesh accessor requires KHR_mesh_quantization").into());
    }
    let semantic = usage.semantic();
    validate_accessor_layout(
        &mut ctx.services()?,
        gltf,
        AccessorLayoutCheck {
            index,
            accessor,
            component: accessor.component_type,
            dimension: accessor.type_,
            contract: AccessorContract::vertex(&[], encoding.dimensions),
        },
    )?;
    let amount = 1 + component_count(accessor.type_) * 2;
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    ctx.child(&layout, [amount], |child| {
        child.run([amount], || {
            Ok::<_, ParseFailure>((|| -> Result<()> {
                let required = semantic == Semantic::Positions;
                match accessor.type_ {
                    json::accessor::Type::Vec2 => {
                        MeshAccessorBounds::<2>::new(accessor, index, required)?;
                    }
                    json::accessor::Type::Vec3 => {
                        MeshAccessorBounds::<3>::new(accessor, index, required)?;
                    }
                    json::accessor::Type::Vec4 => {
                        MeshAccessorBounds::<4>::new(accessor, index, required)?;
                    }
                    _ => return Err(invalid("mesh accessor must use a vector shape").into()),
                }
                Ok(())
            })())
        })
    })??;
    // Keep the established base-direction restriction separate from signed morph deltas.
    if matches!(
        usage,
        MeshAttributeUse::Base(Semantic::Normals | Semantic::Tangents)
    ) && accessor.buffer_view.is_none()
        && accessor
            .sparse
            .as_ref()
            .is_none_or(|sparse| sparse.count < accessor.count)
    {
        return Err(invalid("base direction accessor has implicit zero direction values").into());
    }
    let count = usize::try_from(accessor.count)
        .map_err(|_| ctx.allocation_failure("gltf-accessor-count"))?;
    Ok(AccessorInfo { count })
}

/// Bounds retain the stored component domain, including signed normalized endpoints.
pub(super) struct MeshAccessorBounds<const N: usize> {
    index: usize,
    declared_min: Option<[f32; N]>,
    declared_max: Option<[f32; N]>,
    minimum: [f32; N],
    maximum: [f32; N],
    check_data: bool,
}

impl<const N: usize> MeshAccessorBounds<N> {
    pub(super) fn new(
        accessor: &json::accessor::Accessor,
        index: usize,
        required: bool,
    ) -> Result<Self> {
        use json::accessor::ComponentType as C;
        let invalid = |reason| GltfFailure::invalid_accessor(Some(index), reason);
        if required && (accessor.min.is_none() || accessor.max.is_none()) {
            return Err(invalid("POSITION accessor requires min and max bounds").into());
        }
        let integer_range = match accessor.component_type {
            C::I8 => Some((-128.0, 127.0)),
            C::U8 => Some((0.0, 255.0)),
            C::I16 => Some((-32768.0, 32767.0)),
            C::U16 => Some((0.0, 65535.0)),
            C::F32 => None,
            C::U32 => return Err(invalid("U32 is not a mesh attribute encoding").into()),
        };
        let convert = |bounds: Option<json::accessor::Bounds>| -> Result<Option<[f32; N]>> {
            let Some(bounds) = bounds else {
                return Ok(None);
            };
            if bounds.len != N {
                return Err(
                    invalid("mesh accessor bounds lane count differs from its shape").into(),
                );
            }
            let mut result = [0.0; N];
            for (lane, output) in result.iter_mut().enumerate() {
                let value = bounds.values[lane];
                if !value.is_finite() {
                    return Err(invalid("mesh accessor bounds must be finite").into());
                }
                if let Some((minimum, maximum)) = integer_range
                    && (value.fract() != 0.0 || value < minimum || value > maximum)
                {
                    return Err(invalid(
                        "mesh accessor bounds must be integral stored component values",
                    )
                    .into());
                }
                *output = value as f32;
                if !output.is_finite() {
                    return Err(invalid("mesh accessor bounds exceed the F32 domain").into());
                }
            }
            Ok(Some(result))
        };
        let declared_min = convert(accessor.min)?;
        let declared_max = convert(accessor.max)?;
        if let Some((minimum, maximum)) = declared_min.zip(declared_max)
            && (0..N).any(|lane| minimum[lane] > maximum[lane])
        {
            return Err(invalid("mesh accessor bounds must be ordered min <= max").into());
        }
        Ok(Self {
            index,
            declared_min,
            declared_max,
            minimum: [f32::INFINITY; N],
            maximum: [f32::NEG_INFINITY; N],
            // Core permits unrelated descriptor bounds for a wholly implicit zero accessor.
            check_data: accessor.buffer_view.is_some() || accessor.sparse.is_some(),
        })
    }

    pub(super) fn observe(&mut self, sample: &crate::document::read::Sample<N>) -> Result<()> {
        if sample.decoded.iter().any(|value| !value.is_finite()) {
            return Err(GltfFailure::invalid_accessor(
                Some(self.index),
                "mesh accessor values must be finite",
            )
            .into());
        }
        self.observe_stored(sample.stored)
    }

    pub(super) fn observe_stored(&mut self, stored: [f32; N]) -> Result<()> {
        if stored.iter().any(|value| !value.is_finite()) {
            return Err(GltfFailure::invalid_accessor(
                Some(self.index),
                "mesh accessor stored values must be finite",
            )
            .into());
        }
        if self.check_data && (self.declared_min.is_some() || self.declared_max.is_some()) {
            for ((minimum, maximum), value) in
                self.minimum.iter_mut().zip(&mut self.maximum).zip(stored)
            {
                *minimum = minimum.min(value);
                *maximum = maximum.max(value);
            }
        }
        Ok(())
    }

    pub(super) fn finish(&self) -> Result<()> {
        if self.check_data
            && (self
                .declared_min
                .is_some_and(|minimum| minimum != self.minimum)
                || self
                    .declared_max
                    .is_some_and(|maximum| maximum != self.maximum))
        {
            return Err(GltfFailure::invalid_accessor(
                Some(self.index),
                "mesh accessor bounds do not match the stored values",
            )
            .into());
        }
        Ok(())
    }
}

pub(super) fn collect_mesh_attribute<'scope, const N: usize>(
    ctx: &mut ParseChildren<'_, '_, 'scope>,
    accessor: crate::document::Accessor<'_>,
    buffers: &[Option<BufferData<'_>>],
    semantic: Semantic,
    expected: usize,
    allocation: &'static str,
) -> Result<MeteredVec<'scope, f32>> {
    let sample_work = to_u64(expected)
        .checked_mul(4 * MESH_SAMPLE_WORK_PER_LANE)
        .and_then(|work| work.checked_add(9))
        .ok_or_else(|| ctx.error_view().allocation_failure("gltf-mesh-sample-work"))?;
    {
        let layout = ctx.bind_group([
            ImportWorkClass::Vertices,
            ImportWorkClass::AttributeCells,
            ImportWorkClass::AttributeStreams,
            ImportWorkClass::WorkUnits,
        ])?;
        ctx.child(
            &layout,
            [
                if semantic == Semantic::Positions {
                    to_u64(expected)
                } else {
                    0
                },
                to_u64(expected),
                1,
                sample_work,
            ],
            |child| -> Result<_> {
                let invalid =
                    |reason| GltfFailure::invalid_accessor(Some(accessor.index()), reason);
                let mut bounds = MeshAccessorBounds::<N>::new(
                    accessor.row(),
                    accessor.index(),
                    semantic == Semantic::Positions,
                )?;
                let values = bind(&mut child.services()?, accessor, buffers)?
                    .samples::<N>()
                    .ok_or_else(|| {
                        invalid("mesh accessor shape differs from its checked contract")
                    })?;
                let direction_reason = match (accessor.row().component_type, semantic) {
                    (json::accessor::ComponentType::F32, Semantic::Normals) => {
                        Some("NORMAL must be unit length")
                    }
                    (json::accessor::ComponentType::F32, Semantic::Tangents) => {
                        Some("TANGENT XYZ must be unit length")
                    }
                    _ => None,
                };
                let pad_alpha = matches!(semantic, Semantic::Colors(_)) && N == 3;
                let scalar_count = expected
                    .checked_mul(N + usize::from(pad_alpha))
                    .ok_or_else(|| child.error_view().allocation_failure(allocation))?;
                let mut output = try_vec_capacity(
                    &mut child.services()?,
                    ImportStorageClass::SceneConstruction,
                    scalar_count,
                    allocation,
                )?;
                let mut count = 0;
                for sample in values {
                    poll_parser_loop(&mut child.services()?, count)?;
                    child.run(
                        [
                            u64::from(semantic == Semantic::Positions),
                            1,
                            0,
                            4 * MESH_SAMPLE_WORK_PER_LANE,
                        ],
                        || -> Result<()> {
                            if count == expected {
                                return Err(
                                    invalid("mesh accessor exceeds its admitted count").into()
                                );
                            }
                            bounds.observe(&sample)?;
                            if semantic == Semantic::Tangents
                                && sample
                                    .decoded
                                    .get(3)
                                    .is_none_or(|&sign| sign != -1.0 && sign != 1.0)
                            {
                                return Err(invalid("TANGENT W must be -1 or +1").into());
                            }
                            if let Some(reason) = direction_reason {
                                let [x, y, z, ..] = sample.decoded.as_slice() else {
                                    return Err(invalid(
                                        "base direction sample lacks XYZ components",
                                    )
                                    .into());
                                };
                                let (x, y, z) = (f64::from(*x), f64::from(*y), f64::from(*z));
                                let squared_length = (x * x + y * y) + z * z;
                                if (squared_length - 1.0).abs()
                                    > FLOAT_BASE_DIRECTION_SQUARED_LENGTH_TOLERANCE
                                {
                                    return Err(invalid(reason).into());
                                }
                            }
                            output.try_extend_from_slice_reserved(&sample.decoded)?;
                            if pad_alpha {
                                output.try_push_reserved(1.0)?;
                            }
                            Ok(())
                        },
                    )?;
                    count += 1;
                }
                if count != expected {
                    return Err(invalid("mesh accessor differs from its admitted count").into());
                }
                bounds.finish()?;
                child.poll()?;
                child.run([0, 0, 1, 9], || Ok::<_, ContentError>(output))
            },
        )
    }
}

/// Track distinct selected vertex accessors per view without comparing accessor pairs.
pub(super) fn validate_vertex_views(
    ctx: &mut ParseRoot<'_, '_, '_>,
    document: &crate::document::Document<'_>,
    closure: &ResourceClosure<'_>,
) -> Result<()> {
    let root = document.tables();
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    let length = ctx.child(&layout, [to_u64(root.buffer_views.len())], |child| {
        child.run([to_u64(root.buffer_views.len())], || {
            Ok::<_, ParseFailure>(root.buffer_views.len())
        })
    })?;
    let mut first_accessor = try_vec_filled(
        &mut ctx.services()?,
        ImportStorageClass::FormatNormalization,
        length,
        None,
        "gltf-vertex-view-accessors",
    )?;
    let mut visit = |id: json::Index, ctx: &mut ParseRoot<'_, '_, '_>| -> Result<()> {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let index = ctx.child(&layout, [1], |child| {
            child.run([1], || Ok::<_, ParseFailure>(id.value()))
        })?;
        let accessor = &root.accessors[index];
        let Some(view) = accessor.buffer_view else {
            return Ok(());
        };
        if root.buffer_views[view.value()].byte_stride.is_some() {
            return Ok(());
        }
        let first = &mut first_accessor.as_mut_slice()[view.value()];
        if first.is_some_and(|previous| previous != index) {
            return Err(GltfFailure::invalid_accessor(
                Some(index),
                "distinct vertex accessors sharing a bufferView require byteStride",
            )
            .into());
        }
        *first = Some(index);
        Ok(())
    };
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (index, mesh) in root.meshes.iter().enumerate() {
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, ParseFailure>(closure.meshes.as_slice()[index])
            })
        })?;
        if !selected {
            continue;
        }
        for primitive in &mesh.primitives {
            let attributes = ctx.child(&work_group, [1], |child| {
                child.run([1], || Ok::<_, ParseFailure>(primitive.attributes.values()))
            })?;
            for &id in attributes {
                visit(id, ctx)?;
            }
            for target in primitive.targets.iter().flatten() {
                let accessors = ctx.child(&work_group, [1], |child| {
                    child.run([1], || {
                        Ok::<_, ParseFailure>([target.positions, target.normals, target.tangents])
                    })
                })?;
                for id in accessors.into_iter().flatten() {
                    visit(id, ctx)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_accessor(
    ctx: &mut ParseServices<'_, '_, '_>,
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
            component,
            dimension,
            contract,
        },
    )?;

    let count = usize::try_from(accessor.count)
        .map_err(|_| ctx.allocation_failure("gltf-accessor-count"))?;
    Ok(AccessorInfo { count })
}

#[derive(Clone, Copy)]
struct AccessorLayoutCheck<'a> {
    index: usize,
    accessor: &'a json::accessor::Accessor,
    component: json::accessor::ComponentType,
    dimension: json::accessor::Type,
    contract: AccessorContract,
}

fn validate_accessor_layout(
    ctx: &mut ParseServices<'_, '_, '_>,
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
        validate_base_layout(gltf, check, component_size, element_size)?;
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
    gltf: &crate::document::Document<'_>,
    check: AccessorLayoutCheck<'_>,
    component_size: u64,
    element_size: u64,
) -> Result<()> {
    let invalid = |reason| GltfFailure::invalid_accessor(Some(check.index), reason);
    let view = check.accessor.buffer_view.ok_or_else(|| {
        GltfFailure::internal("base layout validation requires an accessor bufferView")
    })?;
    let buffer_view = checked_buffer_view(gltf, check, view.value())?;
    let stride = match buffer_view.byte_stride {
        Some(_) if !check.contract.allow_stride => {
            return Err(GltfFailure::invalid_document(
                "accessor uses byteStride where it is unsupported",
            )
            .into());
        }
        Some(stride) if !(4..=252).contains(&stride) || !stride.is_multiple_of(4) => {
            return Err(
                invalid("accessor byteStride must be a multiple of four in 4..=252").into(),
            );
        }
        Some(stride) => stride as u64,
        None if check.contract.allow_stride && !component_size.is_multiple_of(4) => {
            return Err(
                invalid("short-component vertex accessors require explicit byteStride").into(),
            );
        }
        None => element_size,
    };
    if stride < element_size {
        return Err(invalid("byteStride is smaller than the accessor element size").into());
    }
    if stride > buffer_view.byte_length {
        return Err(invalid("accessor byteStride exceeds bufferView byteLength").into());
    }
    if !stride.is_multiple_of(component_size) {
        return Err(invalid("byteStride is not aligned to component size").into());
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
        return Err(invalid("byte offset is not aligned to component size").into());
    }

    let required = required_accessor_bytes(check.accessor.count, element_size, stride)
        .ok_or_else(|| invalid("accessor byte range overflow"))?;
    let accessor_end = accessor_offset
        .checked_add(required)
        .ok_or_else(|| invalid("accessor range overflow"))?;
    if accessor_end > view_length {
        return Err(invalid("bufferView byteLength does not cover accessor data").into());
    }

    Ok(())
}

/// Validate each selected sparse index stream once, before any accessor is materialized.
/// The sequential backend reader assumes ordered indices; malformed streams would otherwise
/// silently retain base values. Borrow the admitted buffers instead of collecting another table.
pub(super) fn validate_sparse_indices(
    ctx: &mut ParseRoot<'_, '_, '_>,
    gltf: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    closure: &ResourceClosure<'_>,
) -> Result<()> {
    use crate::document::IndexType;

    let sparse_work = ctx.bind_group([ImportWorkClass::DecodeWork])?;
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (ordinal, accessor) in gltf.accessors().enumerate() {
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, ParseFailure>(closure.accessors.as_slice()[ordinal])
            })
        })?;
        poll_parser_loop(&mut ctx.services()?, ordinal)?;
        if !selected {
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
        ctx.child(
            &sparse_work,
            [to_u64(sparse.count())],
            |child| -> Result<()> {
                let mut previous = None;
                for (ordinal, value) in bytes.chunks_exact(index_size).enumerate() {
                    poll_parser_loop(&mut child.services()?, ordinal)?;
                    child.run([1], || -> Result<()> {
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
                        Ok(())
                    })?;
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
