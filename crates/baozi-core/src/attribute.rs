pub use crate::extension::PackedByteOrder;
use crate::extension::{
    ExtensionSchemaId, ExtensionSchemaOwner, ExtensionSchemaSnapshot,
    MAX_PACKED_ATTRIBUTE_ELEMENTS, MAX_PACKED_ATTRIBUTE_PAYLOAD_BYTES, PackedAttributeDecodedKind,
    PackedAttributeEncodingSchema, PackedAttributeValidationError,
};
use crate::math::{Vec2, Vec3, Vec4};
use crate::namespaced::{NamespacedName, NamespacedNameError};
use crate::provenance::Provenance;
use baozi_operation::CapacityPlan;
use std::collections::HashMap;
use thiserror::Error;

/// Maximum logical elements retained by one canonical attribute buffer.
pub const MAX_ATTRIBUTE_ELEMENTS: usize = MAX_PACKED_ATTRIBUTE_ELEMENTS;

/// Maximum retained allocation bytes owned by one canonical attribute buffer.
pub const MAX_ATTRIBUTE_BUFFER_BYTES: usize = MAX_PACKED_ATTRIBUTE_PAYLOAD_BYTES;

/// The owner-controlled cardinality domain of an attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeDomain {
    Vertex,
    Corner,
    Face,
    Instance,
}

/// A validated namespaced custom attribute semantic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomAttributeSemantic {
    name: NamespacedName,
}

impl CustomAttributeSemantic {
    pub fn new(value: impl AsRef<str>) -> Result<Self, NamespacedNameError> {
        Ok(Self {
            name: NamespacedName::new(value)?,
        })
    }

    pub fn from_name(name: NamespacedName) -> Self {
        Self { name }
    }

    pub fn name(&self) -> &NamespacedName {
        &self.name
    }

    pub fn as_str(&self) -> &str {
        self.name.as_str()
    }
}

/// Canonical built-in and extension attribute semantics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeSemantic {
    Position,
    Normal,
    Tangent,
    Texcoord(u32),
    Color(u32),
    SmoothingGroup,
    Custom(CustomAttributeSemantic),
}

impl AttributeSemantic {
    pub fn kind(&self) -> AttributeSemanticKind {
        match self {
            Self::Position => AttributeSemanticKind::Position,
            Self::Normal => AttributeSemanticKind::Normal,
            Self::Tangent => AttributeSemanticKind::Tangent,
            Self::Texcoord(_) => AttributeSemanticKind::Texcoord,
            Self::Color(_) => AttributeSemanticKind::Color,
            Self::SmoothingGroup => AttributeSemanticKind::SmoothingGroup,
            Self::Custom(_) => AttributeSemanticKind::Custom,
        }
    }
}

/// A text-free semantic category suitable for diagnostics and errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeSemanticKind {
    Position,
    Normal,
    Tangent,
    Texcoord,
    Color,
    SmoothingGroup,
    Custom,
}

/// The scalar component type interpreted by an attribute format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComponentType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl ComponentType {
    pub fn is_integer(self) -> bool {
        !self.is_float()
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    pub fn is_signed_integer(self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32)
    }

    pub fn is_unsigned_integer(self) -> bool {
        matches!(self, Self::U8 | Self::U16 | Self::U32)
    }

    pub fn byte_width(self) -> u8 {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }
}

/// Integer signedness admitted by a registered named normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntegerSignedness {
    Signed,
    Unsigned,
}

/// A named normalization descriptor admitted by an encoding registry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegisteredNormalization {
    schema_id: ExtensionSchemaId,
    signedness: IntegerSignedness,
}

impl RegisteredNormalization {
    /// Registration boundary for a normalization admitted by a declarative schema.
    pub(crate) fn try_new(
        schema_id: Option<ExtensionSchemaId>,
        signedness: IntegerSignedness,
    ) -> Result<Self, RegisteredNormalizationError> {
        let schema_id = schema_id.ok_or(RegisteredNormalizationError::SchemaRequired)?;
        Ok(Self {
            schema_id,
            signedness,
        })
    }

    pub fn name(&self) -> &NamespacedName {
        self.schema_id.name()
    }

    /// Returns the declarative schema that defines this normalization behavior.
    pub const fn schema_id(&self) -> &ExtensionSchemaId {
        &self.schema_id
    }

    pub fn signedness(&self) -> IntegerSignedness {
        self.signedness
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RegisteredNormalizationError {
    #[error("registered normalization requires an admitted extension schema")]
    SchemaRequired,
}

/// Canonical scalar normalization behavior.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Normalization {
    None,
    UNorm,
    SNorm,
    Named(RegisteredNormalization),
}

impl Normalization {
    pub fn kind(&self) -> NormalizationKind {
        match self {
            Self::None => NormalizationKind::None,
            Self::UNorm => NormalizationKind::UNorm,
            Self::SNorm => NormalizationKind::SNorm,
            Self::Named(_) => NormalizationKind::Named,
        }
    }
}

/// A text-free normalization category suitable for diagnostics and errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NormalizationKind {
    None,
    UNorm,
    SNorm,
    Named,
}

/// The fixed contract of a registered named packed encoding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegisteredPackedEncoding {
    schema_id: ExtensionSchemaId,
    schema: PackedAttributeEncodingSchema,
    normalization: Normalization,
}

impl RegisteredPackedEncoding {
    /// Derives the complete executable contract from a scene-owned snapshot.
    pub(crate) fn try_from_schema(
        schemas: &ExtensionSchemaSnapshot,
        schema_id: ExtensionSchemaId,
    ) -> Result<Self, RegisteredPackedEncodingError> {
        let schema = schemas
            .get(&schema_id)
            .ok_or(RegisteredPackedEncodingError::MissingSchema)?;
        if schema.owner() != ExtensionSchemaOwner::Attribute {
            return Err(RegisteredPackedEncodingError::WrongSchemaOwner);
        }
        let packed_schema = schema
            .packed_attribute_encoding()
            .ok_or(RegisteredPackedEncodingError::PackedBehaviorRequired)?;
        let normalization = match packed_schema.decoded_kind() {
            PackedAttributeDecodedKind::F32Vec3 => Normalization::None,
        };
        validate_base_format(
            packed_component_type(packed_schema),
            packed_schema.logical_lanes(),
            &normalization,
        )?;
        Ok(Self {
            schema_id,
            schema: packed_schema,
            normalization,
        })
    }

    /// Rechecks that this descriptor has the exact body retained by a scene.
    pub(crate) fn validate_against_snapshot(
        &self,
        schemas: &ExtensionSchemaSnapshot,
    ) -> Result<(), RegisteredPackedEncodingError> {
        let schema = schemas
            .get(&self.schema_id)
            .ok_or(RegisteredPackedEncodingError::MissingSchema)?;
        if schema.owner() != ExtensionSchemaOwner::Attribute {
            return Err(RegisteredPackedEncodingError::WrongSchemaOwner);
        }
        if schema.packed_attribute_encoding() != Some(self.schema) {
            return Err(RegisteredPackedEncodingError::SchemaBodyMismatch);
        }
        Ok(())
    }

    pub fn name(&self) -> &NamespacedName {
        self.schema_id.name()
    }

    /// Returns the declarative schema that defines this packed encoding.
    pub const fn schema_id(&self) -> &ExtensionSchemaId {
        &self.schema_id
    }

    pub fn component_type(&self) -> ComponentType {
        packed_component_type(self.schema)
    }

    pub fn lanes(&self) -> u8 {
        self.schema.logical_lanes()
    }

    pub fn normalization(&self) -> &Normalization {
        &self.normalization
    }

    pub fn element_width(&self) -> u16 {
        self.schema.element_width()
    }

    pub fn byte_order(&self) -> PackedByteOrder {
        self.schema.byte_order()
    }

    pub const fn schema(&self) -> PackedAttributeEncodingSchema {
        self.schema
    }
}

const fn packed_component_type(schema: PackedAttributeEncodingSchema) -> ComponentType {
    match schema.decoded_kind() {
        PackedAttributeDecodedKind::F32Vec3 => ComponentType::F32,
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RegisteredPackedEncodingError {
    #[error("registered packed encoding schema is missing from the scene snapshot")]
    MissingSchema,
    #[error("registered packed encoding schema is not attribute-owned")]
    WrongSchemaOwner,
    #[error("registered packed encoding schema has no executable packed behavior")]
    PackedBehaviorRequired,
    #[error("registered packed encoding descriptor differs from the scene schema body")]
    SchemaBodyMismatch,
    #[error(transparent)]
    Format(#[from] AttributeFormatError),
}

/// A semantic attribute format independent of public memory layout.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttributeFormat {
    component_type: ComponentType,
    lanes: u8,
    normalization: Normalization,
    packed_encoding: Option<RegisteredPackedEncoding>,
}

impl AttributeFormat {
    pub fn new(
        component_type: ComponentType,
        lanes: u8,
        normalization: Normalization,
        packed_encoding: Option<RegisteredPackedEncoding>,
    ) -> Result<Self, AttributeFormatError> {
        validate_base_format(component_type, lanes, &normalization)?;
        if let Some(encoding) = &packed_encoding {
            if encoding.component_type() != component_type {
                return Err(AttributeFormatError::PackedComponentTypeMismatch);
            }
            if encoding.lanes() != lanes {
                return Err(AttributeFormatError::PackedLaneCountMismatch);
            }
            if encoding.normalization != normalization {
                return Err(AttributeFormatError::PackedNormalizationMismatch);
            }
        }
        Ok(Self {
            component_type,
            lanes,
            normalization,
            packed_encoding,
        })
    }

    fn plain_f32(lanes: u8) -> Self {
        Self {
            component_type: ComponentType::F32,
            lanes,
            normalization: Normalization::None,
            packed_encoding: None,
        }
    }

    pub fn component_type(&self) -> ComponentType {
        self.component_type
    }

    pub fn lanes(&self) -> u8 {
        self.lanes
    }

    pub fn normalization(&self) -> &Normalization {
        &self.normalization
    }

    pub fn packed_encoding(&self) -> Option<&RegisteredPackedEncoding> {
        self.packed_encoding.as_ref()
    }

    pub fn is_plain_f32(&self, lanes: u8) -> bool {
        self.component_type == ComponentType::F32
            && self.lanes == lanes
            && self.normalization == Normalization::None
            && self.packed_encoding.is_none()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AttributeFormatError {
    #[error("attribute lane count {lanes} is outside 1..=16")]
    InvalidLaneCount { lanes: u8 },
    #[error("floating-point attributes do not admit normalization")]
    FloatingPointNormalization,
    #[error("UNorm requires an unsigned integer component type")]
    UnsignedNormalizationOnSignedInteger,
    #[error("SNorm requires a signed integer component type")]
    SignedNormalizationOnUnsignedInteger,
    #[error("named normalization signedness does not match the integer component type")]
    NamedNormalizationSignednessMismatch,
    #[error("packed encoding component type differs from the attribute format")]
    PackedComponentTypeMismatch,
    #[error("packed encoding lane count differs from the attribute format")]
    PackedLaneCountMismatch,
    #[error("packed encoding normalization differs from the attribute format")]
    PackedNormalizationMismatch,
}

fn validate_base_format(
    component_type: ComponentType,
    lanes: u8,
    normalization: &Normalization,
) -> Result<(), AttributeFormatError> {
    if !(1..=16).contains(&lanes) {
        return Err(AttributeFormatError::InvalidLaneCount { lanes });
    }
    if component_type.is_float() {
        return if normalization == &Normalization::None {
            Ok(())
        } else {
            Err(AttributeFormatError::FloatingPointNormalization)
        };
    }

    match normalization {
        Normalization::None => Ok(()),
        Normalization::UNorm if component_type.is_unsigned_integer() => Ok(()),
        Normalization::UNorm => Err(AttributeFormatError::UnsignedNormalizationOnSignedInteger),
        Normalization::SNorm if component_type.is_signed_integer() => Ok(()),
        Normalization::SNorm => Err(AttributeFormatError::SignedNormalizationOnUnsignedInteger),
        Normalization::Named(named)
            if matches!(
                (component_type.is_signed_integer(), named.signedness),
                (true, IntegerSignedness::Signed) | (false, IntegerSignedness::Unsigned)
            ) =>
        {
            Ok(())
        }
        Normalization::Named(_) => Err(AttributeFormatError::NamedNormalizationSignednessMismatch),
    }
}

/// The private storage family behind an owned attribute buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeStorageKind {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
    F32Vec2,
    F32Vec3,
    F32Vec4,
    Packed,
}

#[derive(Debug, PartialEq)]
enum AttributeStorage {
    I8(Vec<i8>),
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    F32Vec2(Vec<Vec2>),
    F32Vec3(Vec<Vec3>),
    F32Vec4(Vec<Vec4>),
    Packed(Vec<u8>),
}

impl AttributeStorage {
    fn try_compact(self, resource: AttributeAllocation) -> Result<Self, AttributeBufferError> {
        Ok(match self {
            Self::I8(values) => Self::I8(try_compact_values(values, resource)?),
            Self::U8(values) => Self::U8(try_compact_values(values, resource)?),
            Self::I16(values) => Self::I16(try_compact_values(values, resource)?),
            Self::U16(values) => Self::U16(try_compact_values(values, resource)?),
            Self::I32(values) => Self::I32(try_compact_values(values, resource)?),
            Self::U32(values) => Self::U32(try_compact_values(values, resource)?),
            Self::F32(values) => Self::F32(try_compact_values(values, resource)?),
            Self::F64(values) => Self::F64(try_compact_values(values, resource)?),
            Self::F32Vec2(values) => Self::F32Vec2(try_compact_values(values, resource)?),
            Self::F32Vec3(values) => Self::F32Vec3(try_compact_values(values, resource)?),
            Self::F32Vec4(values) => Self::F32Vec4(try_compact_values(values, resource)?),
            Self::Packed(values) => Self::Packed(try_compact_values(values, resource)?),
        })
    }

    fn retained_allocation_bytes(&self) -> usize {
        match self {
            Self::I8(values) => values.retained_bytes(),
            Self::U8(values) | Self::Packed(values) => values.retained_bytes(),
            Self::I16(values) => values.retained_bytes(),
            Self::U16(values) => values.retained_bytes(),
            Self::I32(values) => values.retained_bytes(),
            Self::U32(values) => values.retained_bytes(),
            Self::F32(values) => values.retained_bytes(),
            Self::F64(values) => values.retained_bytes(),
            Self::F32Vec2(values) => values.retained_bytes(),
            Self::F32Vec3(values) => values.retained_bytes(),
            Self::F32Vec4(values) => values.retained_bytes(),
        }
    }

    fn dynamic_payload_bytes(&self) -> usize {
        match self {
            Self::I8(values) => logical_vec_bytes(values),
            Self::U8(values) | Self::Packed(values) => logical_vec_bytes(values),
            Self::I16(values) => logical_vec_bytes(values),
            Self::U16(values) => logical_vec_bytes(values),
            Self::I32(values) => logical_vec_bytes(values),
            Self::U32(values) => logical_vec_bytes(values),
            Self::F32(values) => logical_vec_bytes(values),
            Self::F64(values) => logical_vec_bytes(values),
            Self::F32Vec2(values) => logical_vec_bytes(values),
            Self::F32Vec3(values) => logical_vec_bytes(values),
            Self::F32Vec4(values) => logical_vec_bytes(values),
        }
    }

    fn validate_finite(&self) -> Result<(), AttributeBufferError> {
        let invalid_index = match self {
            Self::F32(values) => values.iter().position(|value| !value.is_finite()),
            Self::F64(values) => values.iter().position(|value| !value.is_finite()),
            Self::F32Vec2(values) => values.iter().position(|value| !value.is_finite()),
            Self::F32Vec3(values) => values.iter().position(|value| !value.is_finite()),
            Self::F32Vec4(values) => values.iter().position(|value| !value.is_finite()),
            Self::I8(_)
            | Self::U8(_)
            | Self::I16(_)
            | Self::U16(_)
            | Self::I32(_)
            | Self::U32(_)
            | Self::Packed(_) => None,
        };
        if let Some(element_index) = invalid_index {
            Err(AttributeBufferError::NonFiniteValue { element_index })
        } else {
            Ok(())
        }
    }
}

trait RetainedVecBytes {
    fn retained_bytes(&self) -> usize;
}

impl<T> RetainedVecBytes for Vec<T> {
    fn retained_bytes(&self) -> usize {
        self.capacity().saturating_mul(std::mem::size_of::<T>())
    }
}

fn logical_vec_bytes<T>(values: &[T]) -> usize {
    values.len().saturating_mul(std::mem::size_of::<T>())
}

fn try_compact_values<T: Copy>(
    values: Vec<T>,
    resource: AttributeAllocation,
) -> Result<Vec<T>, AttributeBufferError> {
    let retained_bytes = values
        .capacity()
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(AttributeBufferError::RetainedCapacityTooLarge {
            retained_bytes: usize::MAX,
            maximum: MAX_ATTRIBUTE_BUFFER_BYTES,
        })?;
    if retained_bytes > MAX_ATTRIBUTE_BUFFER_BYTES {
        return Err(AttributeBufferError::RetainedCapacityTooLarge {
            retained_bytes,
            maximum: MAX_ATTRIBUTE_BUFFER_BYTES,
        });
    }

    let plan = CapacityPlan::for_type::<T>(0, 0, values.len())
        .map_err(|_| AttributeBufferError::AllocationFailed { resource })?;
    if values.capacity() <= plan.envelope_capacity() {
        return Ok(values);
    }

    let mut compact = Vec::new();
    if !values.is_empty() {
        compact
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| AttributeBufferError::AllocationFailed { resource })?;
        plan.finish(compact.len(), compact.capacity())
            .map_err(|_| AttributeBufferError::AllocationFailed { resource })?;
        compact.extend_from_slice(&values);
    }
    Ok(compact)
}

#[derive(Debug, Clone, Copy)]
enum F32VectorViewSource<'a, T> {
    Flat(&'a [f32]),
    Typed(&'a [T]),
}

macro_rules! define_f32_vector_view {
    ($view:ident, $iter:ident, $vector:ty, $lanes:expr, [$($index:expr),+]) => {
        /// A non-allocating typed view over flat or specialized F32 vector storage.
        #[derive(Debug, Clone, Copy)]
        pub struct $view<'a> {
            source: F32VectorViewSource<'a, $vector>,
        }

        impl<'a> $view<'a> {
            fn from_flat(values: &'a [f32]) -> Self {
                Self {
                    source: F32VectorViewSource::Flat(values),
                }
            }

            fn from_typed(values: &'a [$vector]) -> Self {
                Self {
                    source: F32VectorViewSource::Typed(values),
                }
            }

            pub fn len(self) -> usize {
                match self.source {
                    F32VectorViewSource::Flat(values) => values.len() / $lanes,
                    F32VectorViewSource::Typed(values) => values.len(),
                }
            }

            pub fn is_empty(self) -> bool {
                self.len() == 0
            }

            pub fn get(self, index: usize) -> Option<$vector> {
                match self.source {
                    F32VectorViewSource::Flat(values) => {
                        let offset = index.checked_mul($lanes)?;
                        Some(<$vector>::new(
                            $(*values.get(offset.checked_add($index)?)?),+
                        ))
                    }
                    F32VectorViewSource::Typed(values) => values.get(index).copied(),
                }
            }

            pub fn iter(self) -> $iter<'a> {
                $iter {
                    view: self,
                    front: 0,
                    back: self.len(),
                }
            }

            pub fn as_typed_slice(self) -> Option<&'a [$vector]> {
                match self.source {
                    F32VectorViewSource::Flat(_) => None,
                    F32VectorViewSource::Typed(values) => Some(values),
                }
            }

            pub fn as_flat_scalars(self) -> Option<&'a [f32]> {
                match self.source {
                    F32VectorViewSource::Flat(values) => Some(values),
                    F32VectorViewSource::Typed(_) => None,
                }
            }
        }

        /// An exact-size iterator over one typed F32 vector view.
        #[derive(Debug, Clone, Copy)]
        pub struct $iter<'a> {
            view: $view<'a>,
            front: usize,
            back: usize,
        }

        impl Iterator for $iter<'_> {
            type Item = $vector;

            fn next(&mut self) -> Option<Self::Item> {
                if self.front >= self.back {
                    return None;
                }
                let index = self.front;
                self.front += 1;
                self.view.get(index)
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                let remaining = self.back - self.front;
                (remaining, Some(remaining))
            }
        }

        impl DoubleEndedIterator for $iter<'_> {
            fn next_back(&mut self) -> Option<Self::Item> {
                if self.front >= self.back {
                    return None;
                }
                self.back -= 1;
                self.view.get(self.back)
            }
        }

        impl ExactSizeIterator for $iter<'_> {}
        impl std::iter::FusedIterator for $iter<'_> {}

        impl<'a> IntoIterator for $view<'a> {
            type Item = $vector;
            type IntoIter = $iter<'a>;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }
    };
}

define_f32_vector_view!(F32Vec2View, F32Vec2Iter, Vec2, 2, [0, 1]);
define_f32_vector_view!(F32Vec3View, F32Vec3Iter, Vec3, 3, [0, 1, 2]);
define_f32_vector_view!(F32Vec4View, F32Vec4Iter, Vec4, 4, [0, 1, 2, 3]);

/// A checked non-allocating typed view over packed F32x3 logical values.
#[derive(Debug, Clone, Copy)]
pub struct PackedF32Vec3View<'a> {
    bytes: &'a [u8],
    schema: PackedAttributeEncodingSchema,
    element_count: usize,
}

impl<'a> PackedF32Vec3View<'a> {
    fn from_validated(
        bytes: &'a [u8],
        schema: PackedAttributeEncodingSchema,
        element_count: usize,
    ) -> Self {
        Self {
            bytes,
            schema,
            element_count,
        }
    }

    pub const fn len(self) -> usize {
        self.element_count
    }

    pub const fn is_empty(self) -> bool {
        self.element_count == 0
    }

    pub fn get(self, index: usize) -> Result<Option<Vec3>, PackedAttributeValidationError> {
        if index >= self.element_count {
            return Ok(None);
        }
        let width = usize::from(self.schema.element_width());
        let Some(start) = index.checked_mul(width) else {
            return Ok(None);
        };
        let Some(end) = start.checked_add(width) else {
            return Ok(None);
        };
        let Some(element) = self.bytes.get(start..end) else {
            return Ok(None);
        };
        self.schema.decode_validated_vec3_element(element).map(Some)
    }

    pub fn iter(self) -> PackedF32Vec3Iter<'a> {
        PackedF32Vec3Iter {
            view: self,
            front: 0,
            back: self.element_count,
        }
    }
}

/// Exact-size checked iterator over a packed F32x3 typed view.
#[derive(Debug, Clone, Copy)]
pub struct PackedF32Vec3Iter<'a> {
    view: PackedF32Vec3View<'a>,
    front: usize,
    back: usize,
}

impl Iterator for PackedF32Vec3Iter<'_> {
    type Item = Result<Vec3, PackedAttributeValidationError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        let index = self.front;
        self.front += 1;
        match self.view.get(index) {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => {
                self.front = self.back;
                None
            }
            Err(error) => Some(Err(error)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}

impl DoubleEndedIterator for PackedF32Vec3Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        match self.view.get(self.back) {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => {
                self.front = self.back;
                None
            }
            Err(error) => Some(Err(error)),
        }
    }
}

impl ExactSizeIterator for PackedF32Vec3Iter<'_> {}
impl std::iter::FusedIterator for PackedF32Vec3Iter<'_> {}

impl<'a> IntoIterator for PackedF32Vec3View<'a> {
    type Item = Result<Vec3, PackedAttributeValidationError>;
    type IntoIter = PackedF32Vec3Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// One typed value from a one-, two-, or three-lane F32 texcoord stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum F32TexcoordValue {
    Scalar(f32),
    Vec2(Vec2),
    Vec3(Vec3),
}

/// A non-allocating typed view over a dense F32 texcoord stream.
#[derive(Debug, Clone, Copy)]
pub enum F32TexcoordView<'a> {
    Scalar(&'a [f32]),
    Vec2(F32Vec2View<'a>),
    Vec3(F32Vec3View<'a>),
}

impl<'a> F32TexcoordView<'a> {
    pub fn lanes(self) -> u8 {
        match self {
            Self::Scalar(_) => 1,
            Self::Vec2(_) => 2,
            Self::Vec3(_) => 3,
        }
    }

    pub fn len(self) -> usize {
        match self {
            Self::Scalar(values) => values.len(),
            Self::Vec2(values) => values.len(),
            Self::Vec3(values) => values.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn get(self, index: usize) -> Option<F32TexcoordValue> {
        match self {
            Self::Scalar(values) => values.get(index).copied().map(F32TexcoordValue::Scalar),
            Self::Vec2(values) => values.get(index).map(F32TexcoordValue::Vec2),
            Self::Vec3(values) => values.get(index).map(F32TexcoordValue::Vec3),
        }
    }

    pub fn iter(self) -> F32TexcoordIter<'a> {
        F32TexcoordIter {
            view: self,
            front: 0,
            back: self.len(),
        }
    }
}

/// An exact-size iterator over one typed F32 texcoord view.
#[derive(Debug, Clone, Copy)]
pub struct F32TexcoordIter<'a> {
    view: F32TexcoordView<'a>,
    front: usize,
    back: usize,
}

impl Iterator for F32TexcoordIter<'_> {
    type Item = F32TexcoordValue;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        let index = self.front;
        self.front += 1;
        self.view.get(index)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}

impl DoubleEndedIterator for F32TexcoordIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        self.view.get(self.back)
    }
}

impl ExactSizeIterator for F32TexcoordIter<'_> {}
impl std::iter::FusedIterator for F32TexcoordIter<'_> {}

impl<'a> IntoIterator for F32TexcoordView<'a> {
    type Item = F32TexcoordValue;
    type IntoIter = F32TexcoordIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Owned, format-bearing typed attribute storage.
#[derive(Debug, PartialEq)]
pub struct AttributeBuffer {
    format: AttributeFormat,
    element_count: u32,
    storage: AttributeStorage,
}

impl AttributeBuffer {
    pub fn from_i8(format: AttributeFormat, values: Vec<i8>) -> Result<Self, AttributeBufferError> {
        Self::from_flat(
            format,
            values.len(),
            ComponentType::I8,
            AttributeStorage::I8(values),
        )
    }

    pub fn from_u8(format: AttributeFormat, values: Vec<u8>) -> Result<Self, AttributeBufferError> {
        Self::from_flat(
            format,
            values.len(),
            ComponentType::U8,
            AttributeStorage::U8(values),
        )
    }

    pub fn from_i16(
        format: AttributeFormat,
        values: Vec<i16>,
    ) -> Result<Self, AttributeBufferError> {
        Self::from_flat(
            format,
            values.len(),
            ComponentType::I16,
            AttributeStorage::I16(values),
        )
    }

    pub fn from_u16(
        format: AttributeFormat,
        values: Vec<u16>,
    ) -> Result<Self, AttributeBufferError> {
        Self::from_flat(
            format,
            values.len(),
            ComponentType::U16,
            AttributeStorage::U16(values),
        )
    }

    pub fn from_i32(
        format: AttributeFormat,
        values: Vec<i32>,
    ) -> Result<Self, AttributeBufferError> {
        Self::from_flat(
            format,
            values.len(),
            ComponentType::I32,
            AttributeStorage::I32(values),
        )
    }

    pub fn from_u32(
        format: AttributeFormat,
        values: Vec<u32>,
    ) -> Result<Self, AttributeBufferError> {
        Self::from_flat(
            format,
            values.len(),
            ComponentType::U32,
            AttributeStorage::U32(values),
        )
    }

    pub fn from_f32(
        format: AttributeFormat,
        values: Vec<f32>,
    ) -> Result<Self, AttributeBufferError> {
        Self::from_flat(
            format,
            values.len(),
            ComponentType::F32,
            AttributeStorage::F32(values),
        )
    }

    pub fn from_f64(
        format: AttributeFormat,
        values: Vec<f64>,
    ) -> Result<Self, AttributeBufferError> {
        Self::from_flat(
            format,
            values.len(),
            ComponentType::F64,
            AttributeStorage::F64(values),
        )
    }

    pub fn from_f32_vec2(values: Vec<Vec2>) -> Result<Self, AttributeBufferError> {
        Self::from_specialized(
            AttributeFormat::plain_f32(2),
            values.len(),
            AttributeStorage::F32Vec2(values),
        )
    }

    pub fn from_f32_vec3(values: Vec<Vec3>) -> Result<Self, AttributeBufferError> {
        Self::from_specialized(
            AttributeFormat::plain_f32(3),
            values.len(),
            AttributeStorage::F32Vec3(values),
        )
    }

    #[cfg(test)]
    pub(crate) fn from_f32_unchecked_for_test(format: AttributeFormat, values: Vec<f32>) -> Self {
        assert!(format.packed_encoding.is_none());
        assert_eq!(format.component_type, ComponentType::F32);
        let lanes = usize::from(format.lanes);
        assert!(values.len().is_multiple_of(lanes));
        Self {
            format,
            element_count: u32::try_from(values.len() / lanes)
                .expect("test attribute element count should fit"),
            storage: AttributeStorage::F32(values),
        }
    }

    pub fn from_f32_vec4(values: Vec<Vec4>) -> Result<Self, AttributeBufferError> {
        Self::from_specialized(
            AttributeFormat::plain_f32(4),
            values.len(),
            AttributeStorage::F32Vec4(values),
        )
    }

    pub fn from_packed(
        format: AttributeFormat,
        bytes: Vec<u8>,
    ) -> Result<Self, AttributeBufferError> {
        let Some(encoding) = format.packed_encoding() else {
            return Err(AttributeBufferError::PackedEncodingRequired);
        };
        let element_count = encoding.schema().validate_payload(&bytes)?;
        let storage =
            AttributeStorage::Packed(bytes).try_compact(AttributeAllocation::PackedPayload)?;
        Ok(Self {
            format,
            element_count,
            storage,
        })
    }

    fn from_flat(
        format: AttributeFormat,
        scalar_count: usize,
        actual_component_type: ComponentType,
        storage: AttributeStorage,
    ) -> Result<Self, AttributeBufferError> {
        if format.packed_encoding.is_some() {
            return Err(AttributeBufferError::TypedStorageForPackedEncoding);
        }
        if format.component_type != actual_component_type {
            return Err(AttributeBufferError::ComponentTypeMismatch {
                format: format.component_type,
                storage: actual_component_type,
            });
        }
        let lanes = usize::from(format.lanes);
        if !scalar_count.is_multiple_of(lanes) {
            return Err(AttributeBufferError::ScalarCountNotDivisible {
                scalar_count,
                lanes: format.lanes,
            });
        }
        let element_count = checked_element_count(scalar_count / lanes)?;
        let storage = storage.try_compact(AttributeAllocation::TypedValues)?;
        let buffer = Self {
            format,
            element_count,
            storage,
        };
        buffer.validate_intrinsic()?;
        Ok(buffer)
    }

    fn from_specialized(
        format: AttributeFormat,
        element_count: usize,
        storage: AttributeStorage,
    ) -> Result<Self, AttributeBufferError> {
        let storage = storage.try_compact(AttributeAllocation::TypedValues)?;
        let buffer = Self {
            format,
            element_count: checked_element_count(element_count)?,
            storage,
        };
        buffer.validate_intrinsic()?;
        Ok(buffer)
    }

    fn validate_intrinsic(&self) -> Result<(), AttributeBufferError> {
        let retained_bytes = self.storage.retained_allocation_bytes();
        if retained_bytes > MAX_ATTRIBUTE_BUFFER_BYTES {
            return Err(AttributeBufferError::RetainedCapacityTooLarge {
                retained_bytes,
                maximum: MAX_ATTRIBUTE_BUFFER_BYTES,
            });
        }
        self.storage.validate_finite()?;

        let actual_element_count = match &self.storage {
            AttributeStorage::Packed(bytes) => {
                let encoding = self
                    .format
                    .packed_encoding()
                    .ok_or(AttributeBufferError::PackedEncodingRequired)?;
                encoding.schema().validate_payload(bytes)?
            }
            AttributeStorage::F32Vec2(values) => {
                if !self.format.is_plain_f32(2) {
                    return Err(AttributeBufferError::SpecializedFormatMismatch);
                }
                checked_element_count(values.len())?
            }
            AttributeStorage::F32Vec3(values) => {
                if !self.format.is_plain_f32(3) {
                    return Err(AttributeBufferError::SpecializedFormatMismatch);
                }
                checked_element_count(values.len())?
            }
            AttributeStorage::F32Vec4(values) => {
                if !self.format.is_plain_f32(4) {
                    return Err(AttributeBufferError::SpecializedFormatMismatch);
                }
                checked_element_count(values.len())?
            }
            storage => {
                if self.format.packed_encoding().is_some() {
                    return Err(AttributeBufferError::TypedStorageForPackedEncoding);
                }
                let (storage_component, scalar_count) = match storage {
                    AttributeStorage::I8(values) => (ComponentType::I8, values.len()),
                    AttributeStorage::U8(values) => (ComponentType::U8, values.len()),
                    AttributeStorage::I16(values) => (ComponentType::I16, values.len()),
                    AttributeStorage::U16(values) => (ComponentType::U16, values.len()),
                    AttributeStorage::I32(values) => (ComponentType::I32, values.len()),
                    AttributeStorage::U32(values) => (ComponentType::U32, values.len()),
                    AttributeStorage::F32(values) => (ComponentType::F32, values.len()),
                    AttributeStorage::F64(values) => (ComponentType::F64, values.len()),
                    AttributeStorage::F32Vec2(_)
                    | AttributeStorage::F32Vec3(_)
                    | AttributeStorage::F32Vec4(_)
                    | AttributeStorage::Packed(_) => {
                        return Err(AttributeBufferError::SpecializedFormatMismatch);
                    }
                };
                if storage_component != self.format.component_type() {
                    return Err(AttributeBufferError::ComponentTypeMismatch {
                        format: self.format.component_type(),
                        storage: storage_component,
                    });
                }
                let lanes = usize::from(self.format.lanes());
                if !scalar_count.is_multiple_of(lanes) {
                    return Err(AttributeBufferError::ScalarCountNotDivisible {
                        scalar_count,
                        lanes: self.format.lanes(),
                    });
                }
                checked_element_count(scalar_count / lanes)?
            }
        };
        if actual_element_count != self.element_count {
            return Err(AttributeBufferError::StoredElementCountMismatch {
                stored: self.element_count,
                actual: actual_element_count,
            });
        }
        Ok(())
    }

    pub fn format(&self) -> &AttributeFormat {
        &self.format
    }

    pub fn element_count(&self) -> u32 {
        self.element_count
    }

    pub fn is_empty(&self) -> bool {
        self.element_count == 0
    }

    /// Returns logical bytes occupied by stored values.
    pub fn dynamic_payload_bytes(&self) -> usize {
        self.storage.dynamic_payload_bytes()
    }

    /// Returns retained heap capacity charged to canonical accounting.
    pub fn retained_allocation_bytes(&self) -> usize {
        self.storage.retained_allocation_bytes()
    }

    pub fn storage_kind(&self) -> AttributeStorageKind {
        match &self.storage {
            AttributeStorage::I8(_) => AttributeStorageKind::I8,
            AttributeStorage::U8(_) => AttributeStorageKind::U8,
            AttributeStorage::I16(_) => AttributeStorageKind::I16,
            AttributeStorage::U16(_) => AttributeStorageKind::U16,
            AttributeStorage::I32(_) => AttributeStorageKind::I32,
            AttributeStorage::U32(_) => AttributeStorageKind::U32,
            AttributeStorage::F32(_) => AttributeStorageKind::F32,
            AttributeStorage::F64(_) => AttributeStorageKind::F64,
            AttributeStorage::F32Vec2(_) => AttributeStorageKind::F32Vec2,
            AttributeStorage::F32Vec3(_) => AttributeStorageKind::F32Vec3,
            AttributeStorage::F32Vec4(_) => AttributeStorageKind::F32Vec4,
            AttributeStorage::Packed(_) => AttributeStorageKind::Packed,
        }
    }

    pub fn as_i8(&self) -> Option<&[i8]> {
        match &self.storage {
            AttributeStorage::I8(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_u8(&self) -> Option<&[u8]> {
        match &self.storage {
            AttributeStorage::U8(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_i16(&self) -> Option<&[i16]> {
        match &self.storage {
            AttributeStorage::I16(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_u16(&self) -> Option<&[u16]> {
        match &self.storage {
            AttributeStorage::U16(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<&[i32]> {
        match &self.storage {
            AttributeStorage::I32(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<&[u32]> {
        match &self.storage {
            AttributeStorage::U32(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<&[f32]> {
        match &self.storage {
            AttributeStorage::F32(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<&[f64]> {
        match &self.storage {
            AttributeStorage::F64(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_f32_vec2(&self) -> Option<&[Vec2]> {
        match &self.storage {
            AttributeStorage::F32Vec2(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_f32_vec3(&self) -> Option<&[Vec3]> {
        match &self.storage {
            AttributeStorage::F32Vec3(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_f32_vec4(&self) -> Option<&[Vec4]> {
        match &self.storage {
            AttributeStorage::F32Vec4(values) => Some(values),
            _ => None,
        }
    }

    /// Returns a typed two-lane view without converting or allocating.
    pub fn f32_vec2_view(&self) -> Option<F32Vec2View<'_>> {
        if !self.format.is_plain_f32(2) {
            return None;
        }
        match &self.storage {
            AttributeStorage::F32(values) => Some(F32Vec2View::from_flat(values)),
            AttributeStorage::F32Vec2(values) => Some(F32Vec2View::from_typed(values)),
            _ => None,
        }
    }

    /// Returns a typed three-lane view without converting or allocating.
    pub fn f32_vec3_view(&self) -> Option<F32Vec3View<'_>> {
        if !self.format.is_plain_f32(3) {
            return None;
        }
        match &self.storage {
            AttributeStorage::F32(values) => Some(F32Vec3View::from_flat(values)),
            AttributeStorage::F32Vec3(values) => Some(F32Vec3View::from_typed(values)),
            _ => None,
        }
    }

    /// Returns a typed four-lane view without converting or allocating.
    pub fn f32_vec4_view(&self) -> Option<F32Vec4View<'_>> {
        if !self.format.is_plain_f32(4) {
            return None;
        }
        match &self.storage {
            AttributeStorage::F32(values) => Some(F32Vec4View::from_flat(values)),
            AttributeStorage::F32Vec4(values) => Some(F32Vec4View::from_typed(values)),
            _ => None,
        }
    }

    pub fn as_packed_bytes(&self) -> Option<&[u8]> {
        match &self.storage {
            AttributeStorage::Packed(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Executes the schema-bound typed-view decoder without allocating.
    pub fn packed_f32_vec3_view(
        &self,
    ) -> Result<Option<PackedF32Vec3View<'_>>, AttributeBufferError> {
        let AttributeStorage::Packed(bytes) = &self.storage else {
            return Ok(None);
        };
        let encoding = self
            .format
            .packed_encoding()
            .ok_or(AttributeBufferError::PackedEncodingRequired)?;
        if encoding.schema().decoded_kind() != PackedAttributeDecodedKind::F32Vec3 {
            return Ok(None);
        }
        let element_count = usize::try_from(self.element_count)
            .map_err(|_| AttributeBufferError::ElementCountTooLarge { count: usize::MAX })?;
        Ok(Some(PackedF32Vec3View::from_validated(
            bytes,
            encoding.schema(),
            element_count,
        )))
    }
}

fn checked_element_count(count: usize) -> Result<u32, AttributeBufferError> {
    if count > MAX_ATTRIBUTE_ELEMENTS {
        return Err(AttributeBufferError::ElementCountLimitExceeded {
            count,
            maximum: MAX_ATTRIBUTE_ELEMENTS,
        });
    }
    u32::try_from(count).map_err(|_| AttributeBufferError::ElementCountTooLarge { count })
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AttributeBufferError {
    #[error("typed attribute storage cannot represent a packed encoding")]
    TypedStorageForPackedEncoding,
    #[error("packed attribute storage requires a packed encoding")]
    PackedEncodingRequired,
    #[error("specialized attribute storage differs from the declared format")]
    SpecializedFormatMismatch,
    #[error("attribute component type differs between format and typed storage")]
    ComponentTypeMismatch {
        format: ComponentType,
        storage: ComponentType,
    },
    #[error("flat scalar count {scalar_count} is not divisible by lane count {lanes}")]
    ScalarCountNotDivisible { scalar_count: usize, lanes: u8 },
    #[error("attribute value at element {element_index} is not finite")]
    NonFiniteValue { element_index: usize },
    #[error("attribute buffer element count {count} exceeds the maximum {maximum}")]
    ElementCountLimitExceeded { count: usize, maximum: usize },
    #[error("attribute element count {count} exceeds the u32 domain limit")]
    ElementCountTooLarge { count: usize },
    #[error("attribute buffer stores {stored} elements but contains {actual}")]
    StoredElementCountMismatch { stored: u32, actual: u32 },
    #[error("attribute buffer retains {retained_bytes} bytes; the maximum is {maximum}")]
    RetainedCapacityTooLarge {
        retained_bytes: usize,
        maximum: usize,
    },
    #[error("attribute {resource:?} allocation failed")]
    AllocationFailed { resource: AttributeAllocation },
    #[error(transparent)]
    PackedValidation(#[from] PackedAttributeValidationError),
}

/// Heap-backed attribute resource whose canonicalization allocation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttributeAllocation {
    TypedValues,
    PackedPayload,
    SparseIndices,
}

/// Dense or sparse canonical attribute values.
#[derive(Debug, PartialEq)]
// Keeping the one-element sparse default inline avoids an unmetered allocation
// whose only purpose would be equalizing enum variant sizes.
#[allow(clippy::large_enum_variant)]
pub enum AttributePayload {
    Dense(AttributeBuffer),
    Sparse {
        logical_count: u32,
        indices: Vec<u32>,
        values: AttributeBuffer,
        default: AttributeBuffer,
    },
}

impl AttributePayload {
    pub fn dense(values: AttributeBuffer) -> Self {
        Self::Dense(values)
    }

    pub fn sparse(
        logical_count: u32,
        indices: Vec<u32>,
        values: AttributeBuffer,
        default: AttributeBuffer,
    ) -> Result<Self, AttributePayloadError> {
        let indices = try_compact_values(indices, AttributeAllocation::SparseIndices)?;
        let payload = Self::Sparse {
            logical_count,
            indices,
            values,
            default,
        };
        payload.validate_intrinsic(None)?;
        Ok(payload)
    }

    pub fn format(&self) -> Result<&AttributeFormat, AttributePayloadError> {
        self.validate_intrinsic(None)?;
        match self {
            Self::Dense(values) => Ok(values.format()),
            Self::Sparse { values, .. } => Ok(values.format()),
        }
    }

    pub fn logical_count(&self) -> u32 {
        match self {
            Self::Dense(values) => values.element_count(),
            Self::Sparse { logical_count, .. } => *logical_count,
        }
    }

    pub fn explicit_values(&self) -> &AttributeBuffer {
        match self {
            Self::Dense(values) => values,
            Self::Sparse { values, .. } => values,
        }
    }

    pub fn sparse_indices(&self) -> Option<&[u32]> {
        match self {
            Self::Dense(_) => None,
            Self::Sparse { indices, .. } => Some(indices),
        }
    }

    pub fn default_value(&self) -> Option<&AttributeBuffer> {
        match self {
            Self::Dense(_) => None,
            Self::Sparse { default, .. } => Some(default),
        }
    }

    pub fn is_sparse(&self) -> bool {
        matches!(self, Self::Sparse { .. })
    }

    pub fn dynamic_payload_bytes(&self) -> usize {
        match self {
            Self::Dense(values) => values.dynamic_payload_bytes(),
            Self::Sparse {
                indices,
                values,
                default,
                ..
            } => logical_vec_bytes(indices)
                .saturating_add(values.dynamic_payload_bytes())
                .saturating_add(default.dynamic_payload_bytes()),
        }
    }

    pub fn retained_allocation_bytes(&self) -> usize {
        match self {
            Self::Dense(values) => values.retained_allocation_bytes(),
            Self::Sparse {
                indices,
                values,
                default,
                ..
            } => indices
                .retained_bytes()
                .saturating_add(values.retained_allocation_bytes())
                .saturating_add(default.retained_allocation_bytes()),
        }
    }

    pub fn validate_shape(
        &self,
        expected_format: &AttributeFormat,
    ) -> Result<(), AttributePayloadError> {
        self.validate_intrinsic(Some(expected_format))
    }

    pub fn validate_cardinality(&self, expected: u32) -> Result<(), AttributeCardinalityError> {
        let actual = self.logical_count();
        if actual == expected {
            Ok(())
        } else {
            Err(AttributeCardinalityError { expected, actual })
        }
    }

    fn validate_intrinsic(
        &self,
        expected_format: Option<&AttributeFormat>,
    ) -> Result<(), AttributePayloadError> {
        match self {
            Self::Dense(values) => {
                values.validate_intrinsic()?;
                if expected_format.is_some_and(|format| values.format() != format) {
                    return Err(AttributePayloadError::FormatMismatch);
                }
                Ok(())
            }
            Self::Sparse {
                logical_count,
                indices,
                values,
                default,
            } => {
                values.validate_intrinsic()?;
                default.validate_intrinsic()?;
                if values.format() != default.format()
                    || expected_format.is_some_and(|format| values.format() != format)
                {
                    return Err(AttributePayloadError::FormatMismatch);
                }
                if default.element_count() != 1 {
                    return Err(AttributePayloadError::DefaultElementCount {
                        actual: default.element_count(),
                    });
                }
                let index_count = u32::try_from(indices.len()).map_err(|_| {
                    AttributePayloadError::IndexCountTooLarge {
                        count: indices.len(),
                    }
                })?;
                if values.element_count() != index_count {
                    return Err(AttributePayloadError::SparseValueCount {
                        indices: index_count,
                        values: values.element_count(),
                    });
                }

                let mut previous = None;
                for (entry, &index) in indices.iter().enumerate() {
                    if index >= *logical_count {
                        return Err(AttributePayloadError::SparseIndexOutOfRange {
                            entry,
                            index,
                            logical_count: *logical_count,
                        });
                    }
                    if let Some(previous_index) = previous {
                        if index == previous_index {
                            return Err(AttributePayloadError::DuplicateSparseIndex {
                                entry,
                                index,
                            });
                        }
                        if index < previous_index {
                            return Err(AttributePayloadError::SparseIndicesNotIncreasing {
                                entry,
                                previous: previous_index,
                                current: index,
                            });
                        }
                    }
                    previous = Some(index);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AttributePayloadError {
    #[error(transparent)]
    Buffer(#[from] AttributeBufferError),
    #[error("attribute payload buffers do not match the declared format")]
    FormatMismatch,
    #[error("sparse default contains {actual} elements instead of exactly one")]
    DefaultElementCount { actual: u32 },
    #[error("sparse index count {count} exceeds the u32 domain limit")]
    IndexCountTooLarge { count: usize },
    #[error("sparse values contain {values} elements for {indices} indices")]
    SparseValueCount { indices: u32, values: u32 },
    #[error("sparse index at entry {entry} is outside logical count {logical_count}")]
    SparseIndexOutOfRange {
        entry: usize,
        index: u32,
        logical_count: u32,
    },
    #[error("sparse index at entry {entry} duplicates an earlier index")]
    DuplicateSparseIndex { entry: usize, index: u32 },
    #[error("sparse indices are not strictly increasing at entry {entry}")]
    SparseIndicesNotIncreasing {
        entry: usize,
        previous: u32,
        current: u32,
    },
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("attribute cardinality is {actual}; expected {expected}")]
pub struct AttributeCardinalityError {
    pub expected: u32,
    pub actual: u32,
}

/// A normalized owner-local attribute identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttributeKey {
    semantic: AttributeSemantic,
    domain: AttributeDomain,
}

impl AttributeKey {
    pub fn new(semantic: AttributeSemantic, domain: AttributeDomain) -> Self {
        Self { semantic, domain }
    }

    pub fn semantic(&self) -> &AttributeSemantic {
        &self.semantic
    }

    pub fn domain(&self) -> AttributeDomain {
        self.domain
    }
}

/// A canonical, schema-checked attribute with opaque provenance.
pub struct Attribute {
    semantic: AttributeSemantic,
    domain: AttributeDomain,
    format: AttributeFormat,
    payload: AttributePayload,
    provenance: Provenance,
}

impl Attribute {
    pub fn new(
        semantic: AttributeSemantic,
        domain: AttributeDomain,
        format: AttributeFormat,
        payload: AttributePayload,
        provenance: Provenance,
    ) -> Result<Self, AttributeError> {
        payload.validate_shape(&format)?;
        validate_builtin_schema(&semantic, domain, &format)?;
        Ok(Self {
            semantic,
            domain,
            format,
            payload,
            provenance,
        })
    }

    pub fn semantic(&self) -> &AttributeSemantic {
        &self.semantic
    }

    pub fn domain(&self) -> AttributeDomain {
        self.domain
    }

    pub fn format(&self) -> &AttributeFormat {
        &self.format
    }

    pub fn payload(&self) -> &AttributePayload {
        &self.payload
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    pub fn key(&self) -> AttributeKey {
        AttributeKey::new(self.semantic.clone(), self.domain)
    }

    pub fn validate_cardinality(&self, expected: u32) -> Result<(), AttributeCardinalityError> {
        self.payload.validate_cardinality(expected)
    }

    /// Returns a non-allocating typed view over dense position values.
    pub fn positions(&self) -> Option<F32Vec3View<'_>> {
        if self.semantic != AttributeSemantic::Position {
            return None;
        }
        match &self.payload {
            AttributePayload::Dense(values) => values.f32_vec3_view(),
            AttributePayload::Sparse { .. } => None,
        }
    }

    /// Returns a non-allocating typed view over dense normal values.
    pub fn normals(&self) -> Option<F32Vec3View<'_>> {
        if self.semantic != AttributeSemantic::Normal {
            return None;
        }
        match &self.payload {
            AttributePayload::Dense(values) => values.f32_vec3_view(),
            AttributePayload::Sparse { .. } => None,
        }
    }

    /// Returns a non-allocating typed view over dense tangent values.
    pub fn tangents(&self) -> Option<F32Vec4View<'_>> {
        if self.semantic != AttributeSemantic::Tangent {
            return None;
        }
        match &self.payload {
            AttributePayload::Dense(values) => values.f32_vec4_view(),
            AttributePayload::Sparse { .. } => None,
        }
    }

    /// Returns a non-allocating typed view over dense F32 texcoord values.
    pub fn f32_texcoords(&self) -> Option<F32TexcoordView<'_>> {
        if !matches!(self.semantic, AttributeSemantic::Texcoord(_)) {
            return None;
        }
        let AttributePayload::Dense(values) = &self.payload else {
            return None;
        };
        match self.format.lanes {
            1 if self.format.is_plain_f32(1) => values.as_f32().map(F32TexcoordView::Scalar),
            2 => values.f32_vec2_view().map(F32TexcoordView::Vec2),
            3 => values.f32_vec3_view().map(F32TexcoordView::Vec3),
            _ => None,
        }
    }

    /// Returns a zero-copy dense position view when vector storage was selected.
    pub fn as_dense_positions(&self) -> Option<&[Vec3]> {
        if self.semantic == AttributeSemantic::Position {
            match &self.payload {
                AttributePayload::Dense(values) => values.as_f32_vec3(),
                AttributePayload::Sparse { .. } => None,
            }
        } else {
            None
        }
    }

    /// Returns a zero-copy dense normal view when vector storage was selected.
    pub fn as_dense_normals(&self) -> Option<&[Vec3]> {
        if self.semantic == AttributeSemantic::Normal {
            match &self.payload {
                AttributePayload::Dense(values) => values.as_f32_vec3(),
                AttributePayload::Sparse { .. } => None,
            }
        } else {
            None
        }
    }

    /// Returns a zero-copy dense tangent view when vector storage was selected.
    pub fn as_dense_tangents(&self) -> Option<&[Vec4]> {
        if self.semantic == AttributeSemantic::Tangent {
            match &self.payload {
                AttributePayload::Dense(values) => values.as_f32_vec4(),
                AttributePayload::Sparse { .. } => None,
            }
        } else {
            None
        }
    }

    pub fn into_parts(
        self,
    ) -> (
        AttributeSemantic,
        AttributeDomain,
        AttributeFormat,
        AttributePayload,
        Provenance,
    ) {
        (
            self.semantic,
            self.domain,
            self.format,
            self.payload,
            self.provenance,
        )
    }
}

impl std::fmt::Debug for Attribute {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Attribute")
            .field("semantic", &self.semantic)
            .field("domain", &self.domain)
            .field("format", &self.format)
            .field("payload", &self.payload)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AttributeError {
    #[error(transparent)]
    Payload(#[from] AttributePayloadError),
    #[error(transparent)]
    Schema(#[from] AttributeSchemaError),
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AttributeSchemaError {
    #[error("attribute semantic {semantic:?} is not valid in domain {domain:?}")]
    InvalidDomain {
        semantic: AttributeSemanticKind,
        domain: AttributeDomain,
    },
    #[error("attribute semantic {semantic:?} does not admit the declared format")]
    InvalidFormat { semantic: AttributeSemanticKind },
}

fn validate_builtin_schema(
    semantic: &AttributeSemantic,
    domain: AttributeDomain,
    format: &AttributeFormat,
) -> Result<(), AttributeSchemaError> {
    let domain_valid = match semantic {
        AttributeSemantic::Position => domain == AttributeDomain::Vertex,
        AttributeSemantic::Normal | AttributeSemantic::Tangent | AttributeSemantic::Texcoord(_) => {
            matches!(domain, AttributeDomain::Vertex | AttributeDomain::Corner)
        }
        AttributeSemantic::Color(_) | AttributeSemantic::Custom(_) => true,
        AttributeSemantic::SmoothingGroup => domain == AttributeDomain::Face,
    };
    if !domain_valid {
        return Err(AttributeSchemaError::InvalidDomain {
            semantic: semantic.kind(),
            domain,
        });
    }

    let format_valid = match semantic {
        AttributeSemantic::Position | AttributeSemantic::Normal => format.is_plain_f32(3),
        AttributeSemantic::Tangent => format.is_plain_f32(4),
        AttributeSemantic::Texcoord(_) => {
            (1..=3).contains(&format.lanes)
                && format.packed_encoding.is_none()
                && matches!(
                    (format.component_type, &format.normalization),
                    (ComponentType::F32, Normalization::None)
                        | (ComponentType::U8 | ComponentType::U16, Normalization::UNorm)
                )
        }
        AttributeSemantic::Color(_) => {
            matches!(format.lanes, 3 | 4)
                && format.packed_encoding.is_none()
                && matches!(
                    (format.component_type, &format.normalization),
                    (ComponentType::F32, Normalization::None)
                        | (ComponentType::U8 | ComponentType::U16, Normalization::UNorm)
                )
        }
        AttributeSemantic::SmoothingGroup => {
            format.component_type == ComponentType::U32
                && format.lanes == 1
                && format.normalization == Normalization::None
                && format.packed_encoding.is_none()
        }
        AttributeSemantic::Custom(_) => true,
    };
    if format_valid {
        Ok(())
    } else {
        Err(AttributeSchemaError::InvalidFormat {
            semantic: semantic.kind(),
        })
    }
}

/// Validates owner-local `(semantic, domain)` uniqueness in deterministic order.
pub fn validate_unique_attribute_keys(
    attributes: &[Attribute],
) -> Result<(), AttributeKeyValidationError> {
    checked_attribute_key_count(attributes.len())?;
    let mut first_indices = HashMap::new();
    try_reserve_attribute_keys(&mut first_indices, attributes.len())?;
    for (duplicate_index, attribute) in attributes.iter().enumerate() {
        let key = attribute.key();
        if let Some(&first_index) = first_indices.get(&key) {
            return Err(DuplicateAttributeKeyError {
                first_index,
                duplicate_index,
                semantic: attribute.semantic.kind(),
                domain: attribute.domain,
            }
            .into());
        }
        first_indices.insert(key, duplicate_index);
    }
    Ok(())
}

fn checked_attribute_key_count(count: usize) -> Result<u32, AttributeKeyValidationError> {
    u32::try_from(count).map_err(|_| AttributeKeyValidationError::AttributeCountTooLarge { count })
}

fn try_reserve_attribute_keys(
    first_indices: &mut HashMap<AttributeKey, usize>,
    requested: usize,
) -> Result<(), AttributeKeyValidationError> {
    first_indices
        .try_reserve(requested)
        .map_err(|_| AttributeKeyValidationError::AllocationFailed { requested })
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AttributeKeyValidationError {
    #[error("attribute count {count} exceeds the u32 owner-domain limit")]
    AttributeCountTooLarge { count: usize },
    #[error("failed to reserve duplicate-key validation storage for {requested} attributes")]
    AllocationFailed { requested: usize },
    #[error(transparent)]
    Duplicate(#[from] DuplicateAttributeKeyError),
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error(
    "attribute key {semantic:?}/{domain:?} at index {duplicate_index} duplicates index {first_index}"
)]
pub struct DuplicateAttributeKeyError {
    pub first_index: usize,
    pub duplicate_index: usize,
    pub semantic: AttributeSemanticKind,
    pub domain: AttributeDomain,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{
        ExtensionCardinality, ExtensionInvalidationRule, ExtensionReferenceRule,
        ExtensionRemapRule, ExtensionSchema, ExtensionSchemaBehavior, ExtensionSchemaSnapshot,
        ExtensionValueSchema,
    };
    use crate::provenance::test_provenance_fixture;

    fn plain_format(component_type: ComponentType, lanes: u8) -> AttributeFormat {
        AttributeFormat::new(component_type, lanes, Normalization::None, None).unwrap()
    }

    fn f32_flat(lanes: u8, element_count: usize) -> AttributeBuffer {
        let scalar_count = usize::from(lanes) * element_count;
        AttributeBuffer::from_f32(
            plain_format(ComponentType::F32, lanes),
            vec![0.0; scalar_count],
        )
        .unwrap()
    }

    fn sparse_f32_buffer(element_count: usize) -> AttributeBuffer {
        AttributeBuffer::from_f32(
            plain_format(ComponentType::F32, 1),
            vec![0.0; element_count],
        )
        .unwrap()
    }

    fn packed_schema_snapshot(
        schema_id: ExtensionSchemaId,
        byte_order: PackedByteOrder,
    ) -> ExtensionSchemaSnapshot {
        let encoding = PackedAttributeEncodingSchema::octahedral_unit_vector_snorm16(byte_order)
            .expect("test packed contract should be core-understood");
        let schema = ExtensionSchema::new_with_behavior(
            schema_id,
            ExtensionSchemaOwner::Attribute,
            ExtensionValueSchema::Vec3,
            ExtensionSchemaBehavior::PackedAttribute(encoding),
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::Never,
        )
        .expect("test packed schema should be consistent");
        ExtensionSchemaSnapshot::try_from_schemas([schema])
            .expect("test packed snapshot should be unique")
    }

    #[test]
    fn accepts_one_two_and_three_lane_texcoords() {
        let (_arena, provenance) = test_provenance_fixture();
        for lanes in 1..=3 {
            let format = plain_format(ComponentType::F32, lanes);
            let payload = AttributePayload::dense(f32_flat(lanes, 2));
            let attribute = Attribute::new(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Corner,
                format,
                payload,
                provenance,
            )
            .unwrap();
            assert_eq!(attribute.payload().logical_count(), 2);
        }
    }

    #[test]
    fn admits_corner_normals_and_face_smoothing_groups() {
        let (_arena, provenance) = test_provenance_fixture();
        let normals = Attribute::new(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            plain_format(ComponentType::F32, 3),
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(vec![Vec3::new(0.0, 0.0, 1.0)]).unwrap(),
            ),
            provenance,
        );
        assert!(normals.is_ok());

        let smoothing_format = plain_format(ComponentType::U32, 1);
        let smoothing = Attribute::new(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            smoothing_format.clone(),
            AttributePayload::dense(
                AttributeBuffer::from_u32(smoothing_format, vec![0, 3]).unwrap(),
            ),
            provenance,
        );
        assert!(smoothing.is_ok());
    }

    #[test]
    fn rejects_position_in_the_wrong_domain_or_format() {
        let (_arena, provenance) = test_provenance_fixture();
        let format = plain_format(ComponentType::F32, 3);
        let wrong_domain = Attribute::new(
            AttributeSemantic::Position,
            AttributeDomain::Corner,
            format.clone(),
            AttributePayload::dense(f32_flat(3, 1)),
            provenance,
        );
        assert_eq!(
            wrong_domain.unwrap_err(),
            AttributeError::Schema(AttributeSchemaError::InvalidDomain {
                semantic: AttributeSemanticKind::Position,
                domain: AttributeDomain::Corner,
            })
        );

        let wrong_format = plain_format(ComponentType::F32, 2);
        let wrong_format = Attribute::new(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            wrong_format.clone(),
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec2(vec![Vec2::new(0.0, 0.0)]).unwrap(),
            ),
            provenance,
        );
        assert_eq!(
            wrong_format.unwrap_err(),
            AttributeError::Schema(AttributeSchemaError::InvalidFormat {
                semantic: AttributeSemanticKind::Position,
            })
        );
    }

    #[test]
    fn rejects_float_normalization_and_signedness_mismatches() {
        assert_eq!(
            AttributeFormat::new(ComponentType::F32, 3, Normalization::UNorm, None).unwrap_err(),
            AttributeFormatError::FloatingPointNormalization
        );
        assert_eq!(
            AttributeFormat::new(ComponentType::I16, 1, Normalization::UNorm, None).unwrap_err(),
            AttributeFormatError::UnsignedNormalizationOnSignedInteger
        );
        assert_eq!(
            AttributeFormat::new(ComponentType::U16, 1, Normalization::SNorm, None).unwrap_err(),
            AttributeFormatError::SignedNormalizationOnUnsignedInteger
        );
    }

    #[test]
    fn rejects_non_finite_plain_float_storage() {
        let f32_format = plain_format(ComponentType::F32, 1);
        assert!(AttributeBuffer::from_f32(f32_format, vec![f32::NAN]).is_err());

        let f64_format = plain_format(ComponentType::F64, 1);
        assert!(AttributeBuffer::from_f64(f64_format, vec![f64::INFINITY]).is_err());

        assert!(AttributeBuffer::from_f32_vec2(vec![Vec2::new(f32::NAN, 0.0)]).is_err());
        assert!(AttributeBuffer::from_f32_vec3(vec![Vec3::new(0.0, f32::INFINITY, 0.0)]).is_err());
        assert!(
            AttributeBuffer::from_f32_vec4(vec![Vec4::new(0.0, 0.0, 0.0, f32::NEG_INFINITY,)])
                .is_err()
        );

        let invalid_sparse_value = AttributeBuffer {
            format: plain_format(ComponentType::F32, 1),
            element_count: 1,
            storage: AttributeStorage::F32(vec![f32::NAN]),
        };
        let valid_default = sparse_f32_buffer(1);
        assert!(AttributePayload::sparse(1, vec![0], invalid_sparse_value, valid_default).is_err());

        let valid_value = sparse_f32_buffer(1);
        let invalid_default = AttributeBuffer {
            format: plain_format(ComponentType::F32, 1),
            element_count: 1,
            storage: AttributeStorage::F32(vec![f32::INFINITY]),
        };
        assert!(AttributePayload::sparse(1, vec![0], valid_value, invalid_default).is_err());
    }

    #[test]
    fn registered_attribute_behaviors_require_and_preserve_schema_ids() {
        assert_eq!(
            RegisteredNormalization::try_new(None, IntegerSignedness::Unsigned),
            Err(RegisteredNormalizationError::SchemaRequired)
        );

        let normalization_schema = ExtensionSchemaId::new("example:octahedral-normalization", 2)
            .expect("test normalization schema should be canonical");
        let normalization = RegisteredNormalization::try_new(
            Some(normalization_schema.clone()),
            IntegerSignedness::Unsigned,
        )
        .expect("schema-bound normalization should be accepted");
        assert_eq!(normalization.schema_id(), &normalization_schema);
        assert_eq!(normalization.name(), normalization_schema.name());
        assert_eq!(normalization.signedness(), IntegerSignedness::Unsigned);

        let packed_schema = ExtensionSchemaId::new("example:packed-octahedral-normal", 3)
            .expect("test packed schema should be canonical");
        assert_eq!(
            RegisteredPackedEncoding::try_from_schema(
                &ExtensionSchemaSnapshot::empty(),
                packed_schema.clone(),
            ),
            Err(RegisteredPackedEncodingError::MissingSchema)
        );

        let snapshot = packed_schema_snapshot(packed_schema.clone(), PackedByteOrder::LittleEndian);
        let packed = RegisteredPackedEncoding::try_from_schema(&snapshot, packed_schema.clone())
            .expect("schema-bound packed encoding should be accepted");
        assert_eq!(packed.schema_id(), &packed_schema);
        assert_eq!(packed.name(), packed_schema.name());
        assert_eq!(packed.component_type(), ComponentType::F32);
        assert_eq!(packed.lanes(), 3);
        assert_eq!(packed.normalization(), &Normalization::None);
        assert_eq!(packed.element_width(), 4);
        assert_eq!(packed.byte_order(), PackedByteOrder::LittleEndian);
        assert_eq!(packed.validate_against_snapshot(&snapshot), Ok(()));

        let mismatched_snapshot = packed_schema_snapshot(packed_schema, PackedByteOrder::BigEndian);
        assert_eq!(
            packed.validate_against_snapshot(&mismatched_snapshot),
            Err(RegisteredPackedEncodingError::SchemaBodyMismatch)
        );
    }

    #[test]
    fn packed_attributes_validate_bytes_and_decode_typed_views() {
        let schema_id = ExtensionSchemaId::new("example:packed-unit-normal", 1).unwrap();
        let snapshot = packed_schema_snapshot(schema_id.clone(), PackedByteOrder::LittleEndian);
        let encoding = RegisteredPackedEncoding::try_from_schema(&snapshot, schema_id).unwrap();
        let format =
            AttributeFormat::new(ComponentType::F32, 3, Normalization::None, Some(encoding))
                .unwrap();

        assert!(matches!(
            AttributeBuffer::from_packed(format.clone(), vec![0; 3]),
            Err(AttributeBufferError::PackedValidation(
                PackedAttributeValidationError::ByteCountNotDivisible { .. }
            ))
        ));
        assert!(matches!(
            AttributeBuffer::from_packed(format.clone(), vec![0x00, 0x80, 0x00, 0x00]),
            Err(AttributeBufferError::PackedValidation(
                PackedAttributeValidationError::ReservedBitPattern { .. }
            ))
        ));

        let buffer = AttributeBuffer::from_packed(
            format,
            vec![0x00, 0x00, 0x00, 0x00, 0xff, 0x7f, 0x00, 0x00],
        )
        .unwrap();
        let view = buffer.packed_f32_vec3_view().unwrap().unwrap();
        assert_eq!(view.len(), 2);
        assert_eq!(view.get(0).unwrap(), Some(Vec3::new(0.0, 0.0, 1.0)));
        let second = view.get(1).unwrap().unwrap();
        assert!((second.x - 1.0).abs() <= f32::EPSILON);
        assert_eq!(second.y, 0.0);
        assert_eq!(second.z, 0.0);
        assert_eq!(view.iter().count(), 2);
    }

    #[test]
    fn built_in_normal_rejects_registered_packing_but_custom_semantic_accepts_it() {
        let (_arena, provenance) = test_provenance_fixture();
        let schema_id = ExtensionSchemaId::new("example:packed-unit-normal", 1).unwrap();
        let snapshot = packed_schema_snapshot(schema_id.clone(), PackedByteOrder::LittleEndian);
        let encoding = RegisteredPackedEncoding::try_from_schema(&snapshot, schema_id.clone())
            .expect("test packed schema should be registered");
        let format =
            AttributeFormat::new(ComponentType::F32, 3, Normalization::None, Some(encoding))
                .unwrap();
        let packed_bytes = vec![0x00, 0x00, 0x00, 0x00];

        let normal = Attribute::new(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            format.clone(),
            AttributePayload::dense(
                AttributeBuffer::from_packed(format.clone(), packed_bytes.clone()).unwrap(),
            ),
            provenance,
        );
        assert_eq!(
            normal.unwrap_err(),
            AttributeError::Schema(AttributeSchemaError::InvalidFormat {
                semantic: AttributeSemanticKind::Normal,
            })
        );

        let custom = Attribute::new(
            AttributeSemantic::Custom(CustomAttributeSemantic::from_name(schema_id.name().clone())),
            AttributeDomain::Vertex,
            format.clone(),
            AttributePayload::dense(AttributeBuffer::from_packed(format, packed_bytes).unwrap()),
            provenance,
        )
        .expect("a namespaced custom semantic should admit registered packing");
        assert_eq!(
            custom
                .format()
                .packed_encoding()
                .expect("custom packed format should retain its encoding")
                .schema_id(),
            &schema_id
        );
    }

    #[test]
    fn canonicalizes_large_capacity_small_length_attribute_storage() {
        let mut values = Vec::with_capacity(1_024);
        values.push(1.0_f32);
        let buffer = AttributeBuffer::from_f32(plain_format(ComponentType::F32, 1), values)
            .expect("bounded source capacity should be compacted fallibly");
        assert_eq!(buffer.dynamic_payload_bytes(), std::mem::size_of::<f32>());
        assert!(buffer.retained_allocation_bytes() <= std::mem::size_of::<f32>());

        let mut indices = Vec::with_capacity(1_024);
        indices.push(0);
        let payload =
            AttributePayload::sparse(1, indices, sparse_f32_buffer(1), sparse_f32_buffer(1))
                .unwrap();
        assert!(payload.retained_allocation_bytes() <= 3 * std::mem::size_of::<u32>());
        assert_eq!(
            checked_element_count(MAX_ATTRIBUTE_ELEMENTS + 1),
            Err(AttributeBufferError::ElementCountLimitExceeded {
                count: MAX_ATTRIBUTE_ELEMENTS + 1,
                maximum: MAX_ATTRIBUTE_ELEMENTS,
            })
        );
    }

    #[test]
    fn validates_dense_cardinality_exactly() {
        let payload = AttributePayload::dense(f32_flat(3, 2));
        assert_eq!(payload.validate_cardinality(2), Ok(()));
        assert_eq!(
            payload.validate_cardinality(1),
            Err(AttributeCardinalityError {
                expected: 1,
                actual: 2,
            })
        );
        assert_eq!(
            payload.validate_cardinality(3),
            Err(AttributeCardinalityError {
                expected: 3,
                actual: 2,
            })
        );
    }

    #[test]
    fn validates_sparse_order_uniqueness_range_values_and_default() {
        let valid =
            AttributePayload::sparse(4, vec![0, 3], sparse_f32_buffer(2), sparse_f32_buffer(1));
        assert!(valid.is_ok());

        assert!(matches!(
            AttributePayload::sparse(4, vec![2, 1], sparse_f32_buffer(2), sparse_f32_buffer(1),),
            Err(AttributePayloadError::SparseIndicesNotIncreasing { .. })
        ));
        assert!(matches!(
            AttributePayload::sparse(4, vec![2, 2], sparse_f32_buffer(2), sparse_f32_buffer(1),),
            Err(AttributePayloadError::DuplicateSparseIndex { .. })
        ));
        assert!(matches!(
            AttributePayload::sparse(4, vec![4], sparse_f32_buffer(1), sparse_f32_buffer(1),),
            Err(AttributePayloadError::SparseIndexOutOfRange { .. })
        ));
        assert!(matches!(
            AttributePayload::sparse(4, vec![1, 2], sparse_f32_buffer(1), sparse_f32_buffer(1),),
            Err(AttributePayloadError::SparseValueCount { .. })
        ));
        assert_eq!(
            AttributePayload::sparse(4, Vec::new(), sparse_f32_buffer(0), sparse_f32_buffer(0),)
                .unwrap_err(),
            AttributePayloadError::DefaultElementCount { actual: 0 }
        );
    }

    #[test]
    fn dense_positions_borrow_the_original_specialized_storage() {
        let (_arena, provenance) = test_provenance_fixture();
        let values = vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)];
        let pointer = values.as_ptr();
        let attribute = Attribute::new(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            plain_format(ComponentType::F32, 3),
            AttributePayload::dense(AttributeBuffer::from_f32_vec3(values).unwrap()),
            provenance,
        )
        .unwrap();

        let positions = attribute.as_dense_positions().unwrap();
        assert_eq!(positions.as_ptr(), pointer);
        assert_eq!(positions[1], Vec3::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn flat_f32_storage_has_non_allocating_typed_views() {
        let vec2_scalars = vec![1.0, 2.0, 3.0, 4.0];
        let vec2_pointer = vec2_scalars.as_ptr();
        let vec2_buffer =
            AttributeBuffer::from_f32(plain_format(ComponentType::F32, 2), vec2_scalars).unwrap();
        let vec2_view = vec2_buffer.f32_vec2_view().unwrap();
        assert_eq!(vec2_view.as_flat_scalars().unwrap().as_ptr(), vec2_pointer);
        assert!(vec2_view.as_typed_slice().is_none());
        assert_eq!(vec2_view.get(1), Some(Vec2::new(3.0, 4.0)));
        assert_eq!(vec2_view.iter().next_back(), Some(Vec2::new(3.0, 4.0)));

        let vec3_buffer = AttributeBuffer::from_f32(
            plain_format(ComponentType::F32, 3),
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        )
        .unwrap();
        let mut vec3_iter = vec3_buffer.f32_vec3_view().unwrap().iter();
        assert_eq!(vec3_iter.len(), 2);
        assert_eq!(vec3_iter.next(), Some(Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!(vec3_iter.next_back(), Some(Vec3::new(4.0, 5.0, 6.0)));

        let vec4_buffer = AttributeBuffer::from_f32(
            plain_format(ComponentType::F32, 4),
            vec![1.0, 2.0, 3.0, -1.0],
        )
        .unwrap();
        assert_eq!(
            vec4_buffer.f32_vec4_view().unwrap().get(0),
            Some(Vec4::new(1.0, 2.0, 3.0, -1.0))
        );
    }

    #[test]
    fn common_attribute_views_accept_flat_f32_storage() {
        let (_arena, provenance) = test_provenance_fixture();
        let position_format = plain_format(ComponentType::F32, 3);
        let positions = Attribute::new(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            position_format.clone(),
            AttributePayload::dense(
                AttributeBuffer::from_f32(position_format, vec![1.0, 2.0, 3.0]).unwrap(),
            ),
            provenance,
        )
        .unwrap();
        assert_eq!(
            positions.positions().unwrap().get(0),
            Some(Vec3::new(1.0, 2.0, 3.0))
        );

        let normal_format = plain_format(ComponentType::F32, 3);
        let normals = Attribute::new(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            normal_format.clone(),
            AttributePayload::dense(
                AttributeBuffer::from_f32(normal_format, vec![0.0, 0.0, 1.0]).unwrap(),
            ),
            provenance,
        )
        .unwrap();
        assert_eq!(
            normals.normals().unwrap().get(0),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );

        let tangent_format = plain_format(ComponentType::F32, 4);
        let tangents = Attribute::new(
            AttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            tangent_format.clone(),
            AttributePayload::dense(
                AttributeBuffer::from_f32(tangent_format, vec![1.0, 0.0, 0.0, -1.0]).unwrap(),
            ),
            provenance,
        )
        .unwrap();
        assert_eq!(
            tangents.tangents().unwrap().get(0),
            Some(Vec4::new(1.0, 0.0, 0.0, -1.0))
        );

        let texcoord_format = plain_format(ComponentType::F32, 2);
        let texcoords = Attribute::new(
            AttributeSemantic::Texcoord(1),
            AttributeDomain::Corner,
            texcoord_format.clone(),
            AttributePayload::dense(
                AttributeBuffer::from_f32(texcoord_format, vec![0.25, 0.75]).unwrap(),
            ),
            provenance,
        )
        .unwrap();
        let texcoord_view = texcoords.f32_texcoords().unwrap();
        assert_eq!(texcoord_view.lanes(), 2);
        assert_eq!(
            texcoord_view.get(0),
            Some(F32TexcoordValue::Vec2(Vec2::new(0.25, 0.75)))
        );
    }

    #[test]
    fn sparse_default_remains_inline_and_exactly_one_element() {
        let payload =
            AttributePayload::sparse(2, vec![1], sparse_f32_buffer(1), sparse_f32_buffer(1))
                .unwrap();
        let AttributePayload::Sparse { default, .. } = payload else {
            panic!("expected sparse payload");
        };
        let _: AttributeBuffer = default;
    }

    #[test]
    fn hostile_custom_names_are_not_echoed_by_errors() {
        let hostile = "vendor:\nprivate-secret";
        let error = CustomAttributeSemantic::new(hostile).unwrap_err();
        let rendered = format!("{error:?}: {error}");
        assert!(!rendered.contains("private-secret"));

        let (_arena, provenance) = test_provenance_fixture();
        let format = plain_format(ComponentType::F32, 1);
        let first = Attribute::new(
            AttributeSemantic::Custom(
                CustomAttributeSemantic::new("vendor:private_secret").unwrap(),
            ),
            AttributeDomain::Vertex,
            format.clone(),
            AttributePayload::dense(AttributeBuffer::from_f32(format.clone(), vec![1.0]).unwrap()),
            provenance,
        )
        .unwrap();
        let duplicate = Attribute::new(
            AttributeSemantic::Custom(
                CustomAttributeSemantic::new("vendor:private_secret").unwrap(),
            ),
            AttributeDomain::Vertex,
            format.clone(),
            AttributePayload::dense(AttributeBuffer::from_f32(format, vec![2.0]).unwrap()),
            provenance,
        )
        .unwrap();
        let error = validate_unique_attribute_keys(&[first, duplicate]).unwrap_err();
        let rendered = format!("{error:?}: {error}");
        assert!(!rendered.contains("private_secret"));
    }

    #[test]
    fn key_validation_capacity_failures_are_typed() {
        let mut first_indices = HashMap::<AttributeKey, usize>::new();
        assert_eq!(
            try_reserve_attribute_keys(&mut first_indices, usize::MAX),
            Err(AttributeKeyValidationError::AllocationFailed {
                requested: usize::MAX,
            })
        );

        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            checked_attribute_key_count((u32::MAX as usize) + 1),
            Err(AttributeKeyValidationError::AttributeCountTooLarge {
                count: (u32::MAX as usize) + 1,
            })
        );
    }

    #[test]
    fn rejects_duplicate_owner_local_attribute_keys() {
        let (_arena, provenance) = test_provenance_fixture();
        let format = plain_format(ComponentType::F32, 3);
        let first = Attribute::new(
            AttributeSemantic::Color(2),
            AttributeDomain::Vertex,
            format.clone(),
            AttributePayload::dense(f32_flat(3, 1)),
            provenance,
        )
        .unwrap();
        let duplicate = Attribute::new(
            AttributeSemantic::Color(2),
            AttributeDomain::Vertex,
            format,
            AttributePayload::dense(f32_flat(3, 1)),
            provenance,
        )
        .unwrap();

        assert_eq!(
            validate_unique_attribute_keys(&[first, duplicate]),
            Err(AttributeKeyValidationError::Duplicate(
                DuplicateAttributeKeyError {
                    first_index: 0,
                    duplicate_index: 1,
                    semantic: AttributeSemanticKind::Color,
                    domain: AttributeDomain::Vertex,
                }
            ))
        );
    }
}
