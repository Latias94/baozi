use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileMetadataArray<'scene> {
    None,
    Bool(&'scene [u8]),
    I64(&'scene [i64]),
    U64(&'scene [u64]),
    F64(&'scene [f64]),
    F32(&'scene [f32]),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileMetadataValue<'scene> {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(&'scene str),
    Bytes {
        media_type: &'scene str,
        payload: &'scene [u8],
    },
    Vec2(&'scene [f32]),
    Vec3(&'scene [f32]),
    Vec4(&'scene [f32]),
    Color(&'scene [f32]),
    Mat4(&'scene [f32]),
    DenseArray {
        element_kind: NodeSceneProfileMetadataArrayKind,
        logical_element_count: usize,
        values: NodeSceneProfileMetadataArray<'scene>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileMetadataArrayKind {
    Bool,
    I64,
    U64,
    F64,
    Vec2,
    Vec3,
    Vec4,
    Color,
    Mat4,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileMetadataEntry<'scene> {
    owner: CanonicalProfileOwner,
    key: &'scene str,
    value: NodeSceneProfileMetadataValue<'scene>,
    provenance: usize,
}

impl<'scene> NodeSceneProfileMetadataEntry<'scene> {
    pub(super) const fn new(
        owner: CanonicalProfileOwner,
        key: &'scene str,
        value: NodeSceneProfileMetadataValue<'scene>,
        provenance: usize,
    ) -> Self {
        Self {
            owner,
            key,
            value,
            provenance,
        }
    }

    pub(in crate::canonical_scene::store) const fn owner(self) -> CanonicalProfileOwner {
        self.owner
    }

    pub(in crate::canonical_scene::store) const fn key(self) -> &'scene str {
        self.key
    }

    pub(in crate::canonical_scene::store) const fn value(
        self,
    ) -> NodeSceneProfileMetadataValue<'scene> {
        self.value
    }

    pub(in crate::canonical_scene::store) const fn provenance(self) -> usize {
        self.provenance
    }
}

pub(super) fn metadata_shape(
    value_kind: u8,
    array_kind: u8,
    logical_count: usize,
) -> NodeSceneSemanticResult<MetadataShape> {
    let scalar = |lane, scalar_count| MetadataShape {
        lane,
        scalar_count,
        auxiliary_text: false,
        bytes: false,
    };
    let shape = match value_kind {
        PROFILE_METADATA_BOOL
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::Bool, 1)
        }
        PROFILE_METADATA_I64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::I64, 1)
        }
        PROFILE_METADATA_U64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::U64, 1)
        }
        PROFILE_METADATA_F64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::F64, 1)
        }
        PROFILE_METADATA_STRING
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            MetadataShape {
                lane: MetadataLane::None,
                scalar_count: 0,
                auxiliary_text: true,
                bytes: false,
            }
        }
        PROFILE_METADATA_BYTES
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            MetadataShape {
                lane: MetadataLane::None,
                scalar_count: 0,
                auxiliary_text: true,
                bytes: true,
            }
        }
        PROFILE_METADATA_VEC2
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 2)
        }
        PROFILE_METADATA_VEC3
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 3)
        }
        PROFILE_METADATA_VEC4 | PROFILE_METADATA_COLOR
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 4)
        }
        PROFILE_METADATA_MAT4
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 16)
        }
        PROFILE_METADATA_DENSE_ARRAY => {
            let (lane, width) = match array_kind {
                PROFILE_METADATA_ARRAY_BOOL => (MetadataLane::Bool, 1),
                PROFILE_METADATA_ARRAY_I64 => (MetadataLane::I64, 1),
                PROFILE_METADATA_ARRAY_U64 => (MetadataLane::U64, 1),
                PROFILE_METADATA_ARRAY_F64 => (MetadataLane::F64, 1),
                PROFILE_METADATA_ARRAY_VEC2 => (MetadataLane::F32, 2),
                PROFILE_METADATA_ARRAY_VEC3 => (MetadataLane::F32, 3),
                PROFILE_METADATA_ARRAY_VEC4 | PROFILE_METADATA_ARRAY_COLOR => {
                    (MetadataLane::F32, 4)
                }
                PROFILE_METADATA_ARRAY_MAT4 => (MetadataLane::F32, 16),
                _ => {
                    return Err(invariant(
                        "validated profile metadata array kind is unknown",
                    ));
                }
            };
            scalar(
                lane,
                logical_count.checked_mul(width).ok_or_else(|| {
                    invariant("validated profile metadata scalar count overflows")
                })?,
            )
        }
        _ => {
            return Err(invariant(
                "validated profile metadata value shape is unknown",
            ));
        }
    };
    Ok(shape)
}

pub(super) fn require_metadata_lane_references(
    entries: CanonicalRecordView<'_>,
    row: usize,
    shape: MetadataShape,
    tables: &NodeSceneProfileTables<'_>,
    byte_count: usize,
) -> NodeSceneSemanticResult<()> {
    require_optional_utf8_reference(
        entries,
        row,
        profile_metadata_entry_field::TEXTS,
        true,
        tables.metadata_text_ref,
        "validated profile metadata text reference is inconsistent",
    )?;
    require_optional_bytes_reference(
        entries,
        row,
        profile_metadata_entry_field::BYTES,
        shape.bytes && byte_count != 0,
        tables.metadata_bytes_ref,
        "validated profile metadata byte reference is inconsistent",
    )?;
    require_optional_sequence_reference::<u8, ProfileMetadataBoolSchema>(
        entries,
        row,
        profile_metadata_entry_field::BOOL_VALUES,
        shape.lane == MetadataLane::Bool && shape.scalar_count != 0,
        tables.metadata_bools_ref,
        "validated profile metadata bool reference is inconsistent",
    )?;
    require_optional_sequence_reference::<i64, ProfileMetadataI64Schema>(
        entries,
        row,
        profile_metadata_entry_field::I64_VALUES,
        shape.lane == MetadataLane::I64 && shape.scalar_count != 0,
        tables.metadata_i64s_ref,
        "validated profile metadata i64 reference is inconsistent",
    )?;
    require_optional_sequence_reference::<u64, ProfileMetadataU64Schema>(
        entries,
        row,
        profile_metadata_entry_field::U64_VALUES,
        shape.lane == MetadataLane::U64 && shape.scalar_count != 0,
        tables.metadata_u64s_ref,
        "validated profile metadata u64 reference is inconsistent",
    )?;
    require_optional_sequence_reference::<f64, ProfileMetadataF64Schema>(
        entries,
        row,
        profile_metadata_entry_field::F64_VALUES,
        shape.lane == MetadataLane::F64 && shape.scalar_count != 0,
        tables.metadata_f64s_ref,
        "validated profile metadata f64 reference is inconsistent",
    )?;
    require_optional_sequence_reference::<f32, ProfileMetadataF32Schema>(
        entries,
        row,
        profile_metadata_entry_field::F32_VALUES,
        shape.lane == MetadataLane::F32 && shape.scalar_count != 0,
        tables.metadata_f32s_ref,
        "validated profile metadata f32 reference is inconsistent",
    )
}

pub(super) fn decode_metadata_value<'scene>(
    value_kind: u8,
    array_kind: u8,
    logical_count: usize,
    auxiliary: &'scene str,
    payload: &'scene [u8],
    lane: NodeSceneProfileMetadataArray<'scene>,
) -> NodeSceneSemanticResult<NodeSceneProfileMetadataValue<'scene>> {
    let value = match value_kind {
        PROFILE_METADATA_BOOL => NodeSceneProfileMetadataValue::Bool(require_one_bool(
            lane,
            "validated profile metadata bool value is unreadable",
        )?),
        PROFILE_METADATA_I64 => NodeSceneProfileMetadataValue::I64(require_one_i64(
            lane,
            "validated profile metadata i64 value is unreadable",
        )?),
        PROFILE_METADATA_U64 => NodeSceneProfileMetadataValue::U64(require_one_u64(
            lane,
            "validated profile metadata u64 value is unreadable",
        )?),
        PROFILE_METADATA_F64 => NodeSceneProfileMetadataValue::F64(require_one_f64(
            lane,
            "validated profile metadata f64 value is unreadable",
        )?),
        PROFILE_METADATA_STRING => {
            require_no_lane(lane)?;
            if !payload.is_empty() {
                return Err(invariant(
                    "validated profile metadata string has a byte payload",
                ));
            }
            NodeSceneProfileMetadataValue::String(auxiliary)
        }
        PROFILE_METADATA_BYTES => {
            require_no_lane(lane)?;
            if auxiliary.is_empty() {
                return Err(invariant(
                    "validated profile metadata bytes have no media type",
                ));
            }
            NodeSceneProfileMetadataValue::Bytes {
                media_type: auxiliary,
                payload,
            }
        }
        PROFILE_METADATA_VEC2 => NodeSceneProfileMetadataValue::Vec2(require_f32_width(lane, 2)?),
        PROFILE_METADATA_VEC3 => NodeSceneProfileMetadataValue::Vec3(require_f32_width(lane, 3)?),
        PROFILE_METADATA_VEC4 => NodeSceneProfileMetadataValue::Vec4(require_f32_width(lane, 4)?),
        PROFILE_METADATA_COLOR => NodeSceneProfileMetadataValue::Color(require_f32_width(lane, 4)?),
        PROFILE_METADATA_MAT4 => NodeSceneProfileMetadataValue::Mat4(require_f32_width(lane, 16)?),
        PROFILE_METADATA_DENSE_ARRAY => NodeSceneProfileMetadataValue::DenseArray {
            element_kind: decode_metadata_array_kind(array_kind)?,
            logical_element_count: logical_count,
            values: lane,
        },
        _ => {
            return Err(invariant(
                "validated profile metadata value kind is unknown",
            ));
        }
    };
    Ok(value)
}

fn require_one_bool(
    lane: NodeSceneProfileMetadataArray<'_>,
    reason: &'static str,
) -> NodeSceneSemanticResult<bool> {
    match lane {
        NodeSceneProfileMetadataArray::Bool([value]) if *value <= 1 => Ok(*value != 0),
        _ => Err(invariant(reason)),
    }
}

fn require_one_i64(
    lane: NodeSceneProfileMetadataArray<'_>,
    reason: &'static str,
) -> NodeSceneSemanticResult<i64> {
    match lane {
        NodeSceneProfileMetadataArray::I64([value]) => Ok(*value),
        _ => Err(invariant(reason)),
    }
}

fn require_one_u64(
    lane: NodeSceneProfileMetadataArray<'_>,
    reason: &'static str,
) -> NodeSceneSemanticResult<u64> {
    match lane {
        NodeSceneProfileMetadataArray::U64([value]) => Ok(*value),
        _ => Err(invariant(reason)),
    }
}

fn require_one_f64(
    lane: NodeSceneProfileMetadataArray<'_>,
    reason: &'static str,
) -> NodeSceneSemanticResult<f64> {
    match lane {
        NodeSceneProfileMetadataArray::F64([value]) => Ok(*value),
        _ => Err(invariant(reason)),
    }
}

fn require_f32_width<'scene>(
    lane: NodeSceneProfileMetadataArray<'scene>,
    width: usize,
) -> NodeSceneSemanticResult<&'scene [f32]> {
    match lane {
        NodeSceneProfileMetadataArray::F32(values) if values.len() == width => Ok(values),
        _ => Err(invariant(
            "validated profile metadata vector width is inconsistent",
        )),
    }
}

fn require_no_lane(lane: NodeSceneProfileMetadataArray<'_>) -> NodeSceneSemanticResult<()> {
    match lane {
        NodeSceneProfileMetadataArray::None => Ok(()),
        _ => Err(invariant(
            "validated profile metadata non-scalar value has a scalar lane",
        )),
    }
}

fn decode_metadata_array_kind(
    value: u8,
) -> NodeSceneSemanticResult<NodeSceneProfileMetadataArrayKind> {
    match value {
        PROFILE_METADATA_ARRAY_BOOL => Ok(NodeSceneProfileMetadataArrayKind::Bool),
        PROFILE_METADATA_ARRAY_I64 => Ok(NodeSceneProfileMetadataArrayKind::I64),
        PROFILE_METADATA_ARRAY_U64 => Ok(NodeSceneProfileMetadataArrayKind::U64),
        PROFILE_METADATA_ARRAY_F64 => Ok(NodeSceneProfileMetadataArrayKind::F64),
        PROFILE_METADATA_ARRAY_VEC2 => Ok(NodeSceneProfileMetadataArrayKind::Vec2),
        PROFILE_METADATA_ARRAY_VEC3 => Ok(NodeSceneProfileMetadataArrayKind::Vec3),
        PROFILE_METADATA_ARRAY_VEC4 => Ok(NodeSceneProfileMetadataArrayKind::Vec4),
        PROFILE_METADATA_ARRAY_COLOR => Ok(NodeSceneProfileMetadataArrayKind::Color),
        PROFILE_METADATA_ARRAY_MAT4 => Ok(NodeSceneProfileMetadataArrayKind::Mat4),
        _ => Err(invariant(
            "validated profile metadata array kind is unknown",
        )),
    }
}
