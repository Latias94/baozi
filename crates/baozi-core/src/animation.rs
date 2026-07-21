use crate::extension::{
    ExtensionEntry, ExtensionEntryValidationError, ExtensionSchemaId,
    validate_unique_extension_entry_keys,
};
use crate::identity::{MeshInstanceIndex, NodeIndex};
use crate::math::{Quat, Vec3, Vec4};
use crate::provenance::Provenance;
use baozi_operation::{CapacityPlanError, GrowableCapacityPlan};
use std::collections::{HashMap, TryReserveError};
use std::fmt;
use std::iter::FusedIterator;
use thiserror::Error;

pub const MAX_ANIMATION_KEYS_PER_CHANNEL: usize = 1_048_576;
pub const MAX_ANIMATION_CHANNELS_PER_CLIP: usize = 65_536;
pub const MAX_ANIMATION_COLLECTION_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ANIMATION_CLIP_RETAINED_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ANIMATION_SAMPLE_CELLS_PER_CHANNEL: usize = 16_777_216;
pub const MAX_ANIMATION_SAMPLE_BYTES_PER_CHANNEL: usize = 64 * 1024 * 1024;
pub const MAX_ANIMATION_TIMING_METADATA_ENTRIES: usize = 32;
pub const MAX_ANIMATION_TIMING_METADATA_CELLS: usize = 4_096;
pub const MAX_ANIMATION_TIMING_METADATA_DYNAMIC_BYTES: usize = 16 * 1024;

/// Caller-selected animation limits bounded by the canonical IR maxima.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationLimits {
    max_keys_per_channel: usize,
    max_channels_per_clip: usize,
    max_collection_bytes: usize,
    max_clip_retained_bytes: usize,
    max_sample_cells_per_channel: usize,
    max_sample_bytes_per_channel: usize,
    max_timing_metadata_entries: usize,
    max_timing_metadata_cells: usize,
    max_timing_metadata_dynamic_bytes: usize,
}

impl AnimationLimits {
    pub const CANONICAL: Self = Self {
        max_keys_per_channel: MAX_ANIMATION_KEYS_PER_CHANNEL,
        max_channels_per_clip: MAX_ANIMATION_CHANNELS_PER_CLIP,
        max_collection_bytes: MAX_ANIMATION_COLLECTION_BYTES,
        max_clip_retained_bytes: MAX_ANIMATION_CLIP_RETAINED_BYTES,
        max_sample_cells_per_channel: MAX_ANIMATION_SAMPLE_CELLS_PER_CHANNEL,
        max_sample_bytes_per_channel: MAX_ANIMATION_SAMPLE_BYTES_PER_CHANNEL,
        max_timing_metadata_entries: MAX_ANIMATION_TIMING_METADATA_ENTRIES,
        max_timing_metadata_cells: MAX_ANIMATION_TIMING_METADATA_CELLS,
        max_timing_metadata_dynamic_bytes: MAX_ANIMATION_TIMING_METADATA_DYNAMIC_BYTES,
    };

    pub const fn canonical() -> Self {
        Self::CANONICAL
    }

    pub fn try_with_max_keys_per_channel(
        mut self,
        maximum: usize,
    ) -> Result<Self, AnimationLimitConfigurationError> {
        validate_configured_limit(
            AnimationLimitField::KeysPerChannel,
            maximum,
            MAX_ANIMATION_KEYS_PER_CHANNEL,
        )?;
        self.max_keys_per_channel = maximum;
        Ok(self)
    }

    pub fn try_with_max_channels_per_clip(
        mut self,
        maximum: usize,
    ) -> Result<Self, AnimationLimitConfigurationError> {
        validate_configured_limit(
            AnimationLimitField::ChannelsPerClip,
            maximum,
            MAX_ANIMATION_CHANNELS_PER_CLIP,
        )?;
        self.max_channels_per_clip = maximum;
        Ok(self)
    }

    pub fn try_with_max_collection_bytes(
        mut self,
        maximum: usize,
    ) -> Result<Self, AnimationLimitConfigurationError> {
        validate_configured_limit(
            AnimationLimitField::CollectionBytes,
            maximum,
            MAX_ANIMATION_COLLECTION_BYTES,
        )?;
        self.max_collection_bytes = maximum;
        Ok(self)
    }

    pub fn try_with_max_clip_retained_bytes(
        mut self,
        maximum: usize,
    ) -> Result<Self, AnimationLimitConfigurationError> {
        validate_configured_limit(
            AnimationLimitField::ClipRetainedBytes,
            maximum,
            MAX_ANIMATION_CLIP_RETAINED_BYTES,
        )?;
        self.max_clip_retained_bytes = maximum;
        Ok(self)
    }

    pub fn try_with_max_sample_payload(
        mut self,
        maximum_cells: usize,
        maximum_bytes: usize,
    ) -> Result<Self, AnimationLimitConfigurationError> {
        validate_configured_limit(
            AnimationLimitField::SampleCellsPerChannel,
            maximum_cells,
            MAX_ANIMATION_SAMPLE_CELLS_PER_CHANNEL,
        )?;
        validate_configured_limit(
            AnimationLimitField::SampleBytesPerChannel,
            maximum_bytes,
            MAX_ANIMATION_SAMPLE_BYTES_PER_CHANNEL,
        )?;
        self.max_sample_cells_per_channel = maximum_cells;
        self.max_sample_bytes_per_channel = maximum_bytes;
        Ok(self)
    }

    pub fn try_with_timing_metadata_limits(
        mut self,
        maximum_entries: usize,
        maximum_cells: usize,
        maximum_dynamic_bytes: usize,
    ) -> Result<Self, AnimationLimitConfigurationError> {
        validate_configured_limit(
            AnimationLimitField::TimingMetadataEntries,
            maximum_entries,
            MAX_ANIMATION_TIMING_METADATA_ENTRIES,
        )?;
        validate_configured_limit(
            AnimationLimitField::TimingMetadataCells,
            maximum_cells,
            MAX_ANIMATION_TIMING_METADATA_CELLS,
        )?;
        validate_configured_limit(
            AnimationLimitField::TimingMetadataDynamicBytes,
            maximum_dynamic_bytes,
            MAX_ANIMATION_TIMING_METADATA_DYNAMIC_BYTES,
        )?;
        self.max_timing_metadata_entries = maximum_entries;
        self.max_timing_metadata_cells = maximum_cells;
        self.max_timing_metadata_dynamic_bytes = maximum_dynamic_bytes;
        Ok(self)
    }

    pub const fn max_keys_per_channel(self) -> usize {
        self.max_keys_per_channel
    }

    pub const fn max_channels_per_clip(self) -> usize {
        self.max_channels_per_clip
    }

    pub const fn max_sample_cells_per_channel(self) -> usize {
        self.max_sample_cells_per_channel
    }

    pub const fn max_sample_bytes_per_channel(self) -> usize {
        self.max_sample_bytes_per_channel
    }

    pub const fn max_clip_retained_bytes(self) -> usize {
        self.max_clip_retained_bytes
    }

    pub const fn authored_times(self) -> AnimationCollectionLimit {
        AnimationCollectionLimit::new(self.max_keys_per_channel, self.max_collection_bytes)
    }

    pub const fn sample_array(self) -> AnimationCollectionLimit {
        AnimationCollectionLimit::new(self.max_keys_per_channel, self.max_collection_bytes)
    }

    pub const fn channels(self) -> AnimationCollectionLimit {
        AnimationCollectionLimit::new(self.max_channels_per_clip, self.max_collection_bytes)
    }

    pub const fn timing_metadata(self) -> AnimationCollectionLimit {
        AnimationCollectionLimit::new(self.max_timing_metadata_entries, self.max_collection_bytes)
    }
}

impl Default for AnimationLimits {
    fn default() -> Self {
        Self::CANONICAL
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationCollectionLimit {
    max_count: usize,
    max_bytes: usize,
}

impl AnimationCollectionLimit {
    const fn new(max_count: usize, max_bytes: usize) -> Self {
        Self {
            max_count,
            max_bytes,
        }
    }

    pub const fn max_count(self) -> usize {
        self.max_count
    }

    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }

    /// Checks logical element count and storage bytes without allocating.
    pub fn preflight<T>(
        self,
        resource: AnimationBuildResource,
        count: usize,
    ) -> Result<AnimationCollectionPreflight, AnimationBuildError> {
        if count > self.max_count {
            return Err(AnimationBuildError::CollectionLimitExceeded {
                resource,
                dimension: AnimationLimitDimension::Count,
                requested: count,
                maximum: self.max_count,
            });
        }
        let bytes = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(AnimationBuildError::CollectionSizeOverflow { resource, count })?;
        if bytes > self.max_bytes {
            return Err(AnimationBuildError::CollectionLimitExceeded {
                resource,
                dimension: AnimationLimitDimension::Bytes,
                requested: bytes,
                maximum: self.max_bytes,
            });
        }
        Ok(AnimationCollectionPreflight { count, bytes })
    }

    fn preflight_capacity<T>(
        self,
        resource: AnimationBuildResource,
        capacity: usize,
    ) -> Result<(), AnimationBuildError> {
        let bytes = capacity.checked_mul(std::mem::size_of::<T>()).ok_or(
            AnimationBuildError::CollectionSizeOverflow {
                resource,
                count: capacity,
            },
        )?;
        if bytes > self.max_bytes {
            return Err(AnimationBuildError::CollectionLimitExceeded {
                resource,
                dimension: AnimationLimitDimension::Bytes,
                requested: bytes,
                maximum: self.max_bytes,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationCollectionPreflight {
    count: usize,
    bytes: usize,
}

impl AnimationCollectionPreflight {
    pub const fn count(self) -> usize {
        self.count
    }

    pub const fn bytes(self) -> usize {
        self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationLimitField {
    KeysPerChannel,
    ChannelsPerClip,
    CollectionBytes,
    ClipRetainedBytes,
    SampleCellsPerChannel,
    SampleBytesPerChannel,
    TimingMetadataEntries,
    TimingMetadataCells,
    TimingMetadataDynamicBytes,
}

impl fmt::Display for AnimationLimitField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::KeysPerChannel => "keys per channel",
            Self::ChannelsPerClip => "channels per clip",
            Self::CollectionBytes => "collection bytes",
            Self::ClipRetainedBytes => "clip retained bytes",
            Self::SampleCellsPerChannel => "sample cells per channel",
            Self::SampleBytesPerChannel => "sample bytes per channel",
            Self::TimingMetadataEntries => "timing metadata entries",
            Self::TimingMetadataCells => "timing metadata cells",
            Self::TimingMetadataDynamicBytes => "timing metadata dynamic bytes",
        })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AnimationLimitConfigurationError {
    #[error("animation {field} limit {requested} exceeds canonical maximum {maximum}")]
    ExceedsCanonicalMaximum {
        field: AnimationLimitField,
        requested: usize,
        maximum: usize,
    },
}

/// The authored unit in which clip durations and channel key times are stored.
#[derive(Debug, Clone, PartialEq)]
pub enum AnimationTimeDomain {
    Seconds,
    Frames { rate: f64 },
    Ticks { rate: Option<f64> },
    Namespaced { schema_id: ExtensionSchemaId },
}

impl AnimationTimeDomain {
    pub fn frames(rate: f64) -> Result<Self, AnimationBuildError> {
        validate_rate(AnimationRateKind::Frames, rate)?;
        Ok(Self::Frames { rate })
    }

    pub fn ticks(rate: Option<f64>) -> Result<Self, AnimationBuildError> {
        if let Some(rate) = rate {
            validate_rate(AnimationRateKind::Ticks, rate)?;
        }
        Ok(Self::Ticks { rate })
    }

    /// Creates a source time domain only when its declarative schema was admitted.
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AnimationBuildError> {
        let schema_id = schema_id.ok_or(AnimationBuildError::BehaviorSchemaRequired {
            behavior: AnimationBehaviorKind::TimeDomain,
        })?;
        Ok(Self::Namespaced { schema_id })
    }

    /// Returns the declarative schema for a namespaced source time domain.
    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::Seconds | Self::Frames { .. } | Self::Ticks { .. } => None,
        }
    }

    fn validate(&self) -> Result<(), AnimationBuildError> {
        match self {
            Self::Frames { rate } => validate_rate(AnimationRateKind::Frames, *rate),
            Self::Ticks { rate: Some(rate) } => validate_rate(AnimationRateKind::Ticks, *rate),
            Self::Seconds | Self::Ticks { rate: None } | Self::Namespaced { .. } => Ok(()),
        }
    }

    fn seconds_conversion(&self) -> Result<SecondsConversion, AnimationSecondsViewError> {
        match self {
            Self::Seconds => Ok(SecondsConversion::Identity),
            Self::Frames { rate } | Self::Ticks { rate: Some(rate) } => {
                Ok(SecondsConversion::DivideBy(*rate))
            }
            Self::Ticks { rate: None } => Err(AnimationSecondsViewError::UnknownTickRate),
            Self::Namespaced { .. } => Err(AnimationSecondsViewError::NamespacedDomain),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationInterpolation {
    Step,
    Linear,
    CubicSpline,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AnimationExtrapolation {
    SourceDefault,
    Constant,
    Linear,
    Repeat,
    Namespaced { schema_id: ExtensionSchemaId },
}

impl AnimationExtrapolation {
    /// Creates a source behavior only when its declarative schema was admitted.
    pub fn try_namespaced(
        schema_id: Option<ExtensionSchemaId>,
    ) -> Result<Self, AnimationBuildError> {
        let schema_id = schema_id.ok_or(AnimationBuildError::BehaviorSchemaRequired {
            behavior: AnimationBehaviorKind::Extrapolation,
        })?;
        Ok(Self::Namespaced { schema_id })
    }

    pub const fn schema_id(&self) -> Option<&ExtensionSchemaId> {
        match self {
            Self::Namespaced { schema_id } => Some(schema_id),
            Self::SourceDefault | Self::Constant | Self::Linear | Self::Repeat => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum AnimationTarget {
    NodeTranslation(NodeIndex),
    NodeRotation(NodeIndex),
    NodeScale(NodeIndex),
    MeshInstanceMorphWeights(MeshInstanceIndex),
}

impl AnimationTarget {
    pub(crate) const fn kind(self) -> AnimationTargetKind {
        match self {
            Self::NodeTranslation(_) => AnimationTargetKind::NodeTranslation,
            Self::NodeRotation(_) => AnimationTargetKind::NodeRotation,
            Self::NodeScale(_) => AnimationTargetKind::NodeScale,
            Self::MeshInstanceMorphWeights(_) => AnimationTargetKind::MeshInstanceMorphWeights,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationTargetKind {
    NodeTranslation,
    NodeRotation,
    NodeScale,
    MeshInstanceMorphWeights,
}

impl fmt::Display for AnimationTargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NodeTranslation => "node translation",
            Self::NodeRotation => "node rotation",
            Self::NodeScale => "node scale",
            Self::MeshInstanceMorphWeights => "mesh-instance morph weights",
        })
    }
}

/// Private owned storage for one animation sample layout.
#[derive(Debug, PartialEq)]
enum AnimationSampleStorage<Value, Tangent> {
    Values(Vec<Value>),
    CubicSpline {
        in_tangents: Vec<Tangent>,
        values: Vec<Value>,
        out_tangents: Vec<Tangent>,
    },
}

/// Samples with either no tangent payload or explicit cubic in/value/out arrays.
///
/// Owned vectors stay private so callers cannot inject allocator capacity that
/// bypasses the canonical resource contract. Construction always copies
/// through fallible bounded collectors; inspection borrows through [`AnimationSamplesView`].
#[derive(Debug, PartialEq)]
pub struct AnimationSamples<Value, Tangent> {
    storage: AnimationSampleStorage<Value, Tangent>,
}

impl<Value, Tangent> AnimationSamples<Value, Tangent> {
    fn from_values(values: Vec<Value>) -> Self {
        Self {
            storage: AnimationSampleStorage::Values(values),
        }
    }

    fn from_cubic_spline(
        in_tangents: Vec<Tangent>,
        values: Vec<Value>,
        out_tangents: Vec<Tangent>,
    ) -> Self {
        Self {
            storage: AnimationSampleStorage::CubicSpline {
                in_tangents,
                values,
                out_tangents,
            },
        }
    }

    fn try_values(
        values: impl IntoIterator<Item = Value>,
        cells_per_value: usize,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        let (values, _) = try_collect_fixed_sample_part(
            values,
            AnimationBuildResource::SampleValues,
            cells_per_value,
            limits.sample_array(),
            SamplePayloadSize::default(),
            limits,
        )?;
        Ok(Self {
            storage: AnimationSampleStorage::Values(values),
        })
    }

    fn try_cubic_spline(
        in_tangents: impl IntoIterator<Item = Tangent>,
        values: impl IntoIterator<Item = Value>,
        out_tangents: impl IntoIterator<Item = Tangent>,
        value_cells: usize,
        tangent_cells: usize,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        let (in_tangents, footprint) = try_collect_fixed_sample_part(
            in_tangents,
            AnimationBuildResource::SampleInTangents,
            tangent_cells,
            limits.sample_array(),
            SamplePayloadSize::default(),
            limits,
        )?;
        let (values, footprint) = try_collect_fixed_sample_part(
            values,
            AnimationBuildResource::SampleValues,
            value_cells,
            limits.sample_array(),
            footprint,
            limits,
        )?;
        let (out_tangents, _) = try_collect_fixed_sample_part(
            out_tangents,
            AnimationBuildResource::SampleOutTangents,
            tangent_cells,
            limits.sample_array(),
            footprint,
            limits,
        )?;
        Ok(Self {
            storage: AnimationSampleStorage::CubicSpline {
                in_tangents,
                values,
                out_tangents,
            },
        })
    }

    pub fn view(&self) -> AnimationSamplesView<'_, Value, Tangent> {
        match &self.storage {
            AnimationSampleStorage::Values(values) => AnimationSamplesView::Values(values),
            AnimationSampleStorage::CubicSpline {
                in_tangents,
                values,
                out_tangents,
            } => AnimationSamplesView::CubicSpline {
                in_tangents,
                values,
                out_tangents,
            },
        }
    }

    pub fn values(&self) -> &[Value] {
        self.view().values()
    }

    pub fn in_tangents(&self) -> Option<&[Tangent]> {
        self.view().in_tangents()
    }

    pub fn out_tangents(&self) -> Option<&[Tangent]> {
        self.view().out_tangents()
    }

    pub fn len(&self) -> usize {
        self.values().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Allocation-free borrowed view of one animation sample layout.
#[derive(Debug)]
pub enum AnimationSamplesView<'a, Value, Tangent> {
    Values(&'a [Value]),
    CubicSpline {
        in_tangents: &'a [Tangent],
        values: &'a [Value],
        out_tangents: &'a [Tangent],
    },
}

impl<'a, Value, Tangent> AnimationSamplesView<'a, Value, Tangent> {
    pub const fn values(self) -> &'a [Value] {
        match self {
            Self::Values(values) | Self::CubicSpline { values, .. } => values,
        }
    }

    pub const fn in_tangents(self) -> Option<&'a [Tangent]> {
        match self {
            Self::Values(_) => None,
            Self::CubicSpline { in_tangents, .. } => Some(in_tangents),
        }
    }

    pub const fn out_tangents(self) -> Option<&'a [Tangent]> {
        match self {
            Self::Values(_) => None,
            Self::CubicSpline { out_tangents, .. } => Some(out_tangents),
        }
    }

    pub const fn is_cubic_spline(&self) -> bool {
        matches!(self, Self::CubicSpline { .. })
    }
}

/// Private target-typed animation payload storage.
#[derive(Debug, PartialEq)]
enum AnimationValueStorage {
    Translations(AnimationSamples<Vec3, Vec3>),
    Rotations(AnimationSamples<Quat, Vec4>),
    Scales(AnimationSamples<Vec3, Vec3>),
    MorphWeights {
        width: usize,
        samples: AnimationSamples<Vec<f32>, Vec<f32>>,
    },
}

/// Closed, target-typed animation payloads.
#[derive(Debug, PartialEq)]
pub struct AnimationValues {
    storage: AnimationValueStorage,
    retained_cell_capacity: usize,
    retained_allocation_bytes: usize,
}

impl AnimationValues {
    pub fn try_translations(
        values: impl IntoIterator<Item = Vec3>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        Self::try_from_storage(
            AnimationValueStorage::Translations(AnimationSamples::try_values(values, 3, limits)?),
            limits,
        )
    }

    pub fn try_cubic_translations(
        in_tangents: impl IntoIterator<Item = Vec3>,
        values: impl IntoIterator<Item = Vec3>,
        out_tangents: impl IntoIterator<Item = Vec3>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        Self::try_from_storage(
            AnimationValueStorage::Translations(AnimationSamples::try_cubic_spline(
                in_tangents,
                values,
                out_tangents,
                3,
                3,
                limits,
            )?),
            limits,
        )
    }

    pub fn try_rotations(
        values: impl IntoIterator<Item = Quat>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        Self::try_from_storage(
            AnimationValueStorage::Rotations(AnimationSamples::try_values(values, 4, limits)?),
            limits,
        )
    }

    pub fn try_cubic_rotations(
        in_tangents: impl IntoIterator<Item = Vec4>,
        values: impl IntoIterator<Item = Quat>,
        out_tangents: impl IntoIterator<Item = Vec4>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        Self::try_from_storage(
            AnimationValueStorage::Rotations(AnimationSamples::try_cubic_spline(
                in_tangents,
                values,
                out_tangents,
                4,
                4,
                limits,
            )?),
            limits,
        )
    }

    pub fn try_scales(
        values: impl IntoIterator<Item = Vec3>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        Self::try_from_storage(
            AnimationValueStorage::Scales(AnimationSamples::try_values(values, 3, limits)?),
            limits,
        )
    }

    pub fn try_cubic_scales(
        in_tangents: impl IntoIterator<Item = Vec3>,
        values: impl IntoIterator<Item = Vec3>,
        out_tangents: impl IntoIterator<Item = Vec3>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        Self::try_from_storage(
            AnimationValueStorage::Scales(AnimationSamples::try_cubic_spline(
                in_tangents,
                values,
                out_tangents,
                3,
                3,
                limits,
            )?),
            limits,
        )
    }

    pub fn try_morph_weights<Row>(
        width: usize,
        values: impl IntoIterator<Item = Row>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError>
    where
        Row: IntoIterator<Item = f32>,
    {
        validate_morph_width(width)?;
        let (values, _) = try_collect_morph_rows(
            values,
            AnimationBuildResource::SampleValues,
            SamplePayloadSize::default(),
            limits,
        )?;
        Self::try_from_storage(
            AnimationValueStorage::MorphWeights {
                width,
                samples: AnimationSamples::from_values(values),
            },
            limits,
        )
    }

    pub fn try_cubic_morph_weights<InRow, ValueRow, OutRow>(
        width: usize,
        in_tangents: impl IntoIterator<Item = InRow>,
        values: impl IntoIterator<Item = ValueRow>,
        out_tangents: impl IntoIterator<Item = OutRow>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError>
    where
        InRow: IntoIterator<Item = f32>,
        ValueRow: IntoIterator<Item = f32>,
        OutRow: IntoIterator<Item = f32>,
    {
        validate_morph_width(width)?;
        let (in_tangents, footprint) = try_collect_morph_rows(
            in_tangents,
            AnimationBuildResource::SampleInTangents,
            SamplePayloadSize::default(),
            limits,
        )?;
        let (values, footprint) = try_collect_morph_rows(
            values,
            AnimationBuildResource::SampleValues,
            footprint,
            limits,
        )?;
        let (out_tangents, _) = try_collect_morph_rows(
            out_tangents,
            AnimationBuildResource::SampleOutTangents,
            footprint,
            limits,
        )?;
        Self::try_from_storage(
            AnimationValueStorage::MorphWeights {
                width,
                samples: AnimationSamples::from_cubic_spline(in_tangents, values, out_tangents),
            },
            limits,
        )
    }

    fn try_from_storage(
        storage: AnimationValueStorage,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        let size = SamplePayloadSize::measure_storage(&storage)?;
        validate_sample_payload_size(size, limits)?;
        Ok(Self {
            storage,
            retained_cell_capacity: size.cells,
            retained_allocation_bytes: size.bytes,
        })
    }

    pub const fn kind(&self) -> AnimationValueKind {
        match &self.storage {
            AnimationValueStorage::Translations(_) => AnimationValueKind::Translations,
            AnimationValueStorage::Rotations(_) => AnimationValueKind::Rotations,
            AnimationValueStorage::Scales(_) => AnimationValueKind::Scales,
            AnimationValueStorage::MorphWeights { .. } => AnimationValueKind::MorphWeights,
        }
    }

    pub fn key_count(&self) -> usize {
        match &self.storage {
            AnimationValueStorage::Translations(samples) => samples.len(),
            AnimationValueStorage::Rotations(samples) => samples.len(),
            AnimationValueStorage::Scales(samples) => samples.len(),
            AnimationValueStorage::MorphWeights { samples, .. } => samples.len(),
        }
    }

    pub const fn retained_cell_capacity(&self) -> usize {
        self.retained_cell_capacity
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }

    pub fn view(&self) -> AnimationValuesView<'_> {
        match &self.storage {
            AnimationValueStorage::Translations(samples) => {
                AnimationValuesView::Translations(samples.view())
            }
            AnimationValueStorage::Rotations(samples) => {
                AnimationValuesView::Rotations(samples.view())
            }
            AnimationValueStorage::Scales(samples) => AnimationValuesView::Scales(samples.view()),
            AnimationValueStorage::MorphWeights { width, samples } => {
                AnimationValuesView::MorphWeights {
                    width: *width,
                    samples: samples.view(),
                }
            }
        }
    }
}

/// Allocation-free borrowed view of a target-typed animation payload.
#[derive(Debug)]
pub enum AnimationValuesView<'a> {
    Translations(AnimationSamplesView<'a, Vec3, Vec3>),
    Rotations(AnimationSamplesView<'a, Quat, Vec4>),
    Scales(AnimationSamplesView<'a, Vec3, Vec3>),
    MorphWeights {
        width: usize,
        samples: AnimationSamplesView<'a, Vec<f32>, Vec<f32>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationValueKind {
    Translations,
    Rotations,
    Scales,
    MorphWeights,
}

impl fmt::Display for AnimationValueKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Translations => "translations",
            Self::Rotations => "rotations",
            Self::Scales => "scales",
            Self::MorphWeights => "morph weights",
        })
    }
}

/// Bounded, non-behavioral source timing facts attached to one channel.
///
/// Values reuse the closed extension value shape. Behavior-bearing timing
/// data must instead use a descriptor backed by an `ExtensionSchemaId`.
#[derive(Debug, PartialEq)]
pub struct AnimationTimingMetadata {
    entries: Vec<ExtensionEntry>,
    cell_count: usize,
    dynamic_payload_bytes: usize,
    retained_allocation_bytes: usize,
}

impl AnimationTimingMetadata {
    pub const fn empty() -> Self {
        Self {
            entries: Vec::new(),
            cell_count: 0,
            dynamic_payload_bytes: 0,
            retained_allocation_bytes: 0,
        }
    }

    pub fn try_new(
        entries: impl IntoIterator<Item = ExtensionEntry>,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        let entries = try_collect_timing_metadata(entries, limits)?;
        validate_unique_extension_entry_keys(&entries)
            .map_err(|source| AnimationBuildError::InvalidTimingMetadata { source })?;

        let mut cell_count = 0_usize;
        let mut dynamic_payload_bytes = 0_usize;
        let mut retained_payload_bytes = 0_usize;
        for entry in &entries {
            cell_count = cell_count.checked_add(entry.value().cell_count()).ok_or(
                AnimationBuildError::TimingMetadataSizeOverflow {
                    dimension: AnimationLimitDimension::Cells,
                },
            )?;
            dynamic_payload_bytes = dynamic_payload_bytes
                .checked_add(entry.value().dynamic_payload_bytes())
                .ok_or(AnimationBuildError::TimingMetadataSizeOverflow {
                    dimension: AnimationLimitDimension::Bytes,
                })?;
            retained_payload_bytes = retained_payload_bytes
                .checked_add(entry.value().retained_allocation_bytes())
                .ok_or(AnimationBuildError::TimingMetadataSizeOverflow {
                    dimension: AnimationLimitDimension::Bytes,
                })?;
        }

        let retained_allocation_bytes = retained_capacity_bytes::<ExtensionEntry>(
            entries.capacity(),
            AnimationBuildResource::TimingMetadata,
        )?
        .checked_add(retained_payload_bytes)
        .ok_or(AnimationBuildError::RetainedSizeOverflow {
            resource: AnimationBuildResource::TimingMetadata,
        })?;

        let metadata = Self {
            entries,
            cell_count,
            dynamic_payload_bytes,
            retained_allocation_bytes,
        };
        metadata.validate_limits(limits)?;
        Ok(metadata)
    }

    pub fn entries(&self) -> &[ExtensionEntry] {
        &self.entries
    }

    pub const fn cell_count(&self) -> usize {
        self.cell_count
    }

    pub const fn dynamic_payload_bytes(&self) -> usize {
        self.dynamic_payload_bytes
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }

    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn validate_limits(&self, limits: &AnimationLimits) -> Result<(), AnimationBuildError> {
        limits.timing_metadata().preflight::<ExtensionEntry>(
            AnimationBuildResource::TimingMetadata,
            self.entries.len(),
        )?;
        if self.cell_count > limits.max_timing_metadata_cells {
            return Err(AnimationBuildError::TimingMetadataLimitExceeded {
                dimension: AnimationLimitDimension::Cells,
                requested: self.cell_count,
                maximum: limits.max_timing_metadata_cells,
            });
        }
        if self.dynamic_payload_bytes > limits.max_timing_metadata_dynamic_bytes {
            return Err(AnimationBuildError::TimingMetadataLimitExceeded {
                dimension: AnimationLimitDimension::Bytes,
                requested: self.dynamic_payload_bytes,
                maximum: limits.max_timing_metadata_dynamic_bytes,
            });
        }
        let maximum_retained_bytes = limits
            .max_collection_bytes
            .checked_add(limits.max_timing_metadata_dynamic_bytes)
            .ok_or(AnimationBuildError::TimingMetadataSizeOverflow {
                dimension: AnimationLimitDimension::Bytes,
            })?;
        if self.retained_allocation_bytes > maximum_retained_bytes {
            return Err(AnimationBuildError::TimingMetadataLimitExceeded {
                dimension: AnimationLimitDimension::Bytes,
                requested: self.retained_allocation_bytes,
                maximum: maximum_retained_bytes,
            });
        }
        Ok(())
    }
}

impl Default for AnimationTimingMetadata {
    fn default() -> Self {
        Self::empty()
    }
}

/// One validated channel in authored time-domain order.
#[derive(Debug, PartialEq)]
pub struct AnimationChannel {
    target: AnimationTarget,
    interpolation: AnimationInterpolation,
    authored_times: Vec<f64>,
    pre_extrapolation: AnimationExtrapolation,
    post_extrapolation: AnimationExtrapolation,
    values: AnimationValues,
    timing_metadata: AnimationTimingMetadata,
    provenance: Provenance,
    retained_allocation_bytes: usize,
}

impl AnimationChannel {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        target: AnimationTarget,
        interpolation: AnimationInterpolation,
        authored_times: impl IntoIterator<Item = f64>,
        pre_extrapolation: AnimationExtrapolation,
        post_extrapolation: AnimationExtrapolation,
        values: AnimationValues,
        timing_metadata: AnimationTimingMetadata,
        provenance: Provenance,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        validate_target_value_kind(target.kind(), values.kind())?;

        let authored_times = try_collect(
            authored_times,
            AnimationBuildResource::AuthoredTimes,
            limits.authored_times(),
        )?;
        validate_authored_times(&authored_times)?;
        validate_animation_values(interpolation, authored_times.len(), &values)?;
        validate_sample_payload_limits(&values, limits)?;
        timing_metadata.validate_limits(limits)?;
        let retained_allocation_bytes = retained_capacity_bytes::<f64>(
            authored_times.capacity(),
            AnimationBuildResource::AuthoredTimes,
        )?
        .checked_add(values.retained_allocation_bytes())
        .and_then(|bytes| bytes.checked_add(timing_metadata.retained_allocation_bytes()))
        .ok_or(AnimationBuildError::RetainedSizeOverflow {
            resource: AnimationBuildResource::ChannelPayload,
        })?;

        Ok(Self {
            target,
            interpolation,
            authored_times,
            pre_extrapolation,
            post_extrapolation,
            values,
            timing_metadata,
            provenance,
            retained_allocation_bytes,
        })
    }

    pub(crate) const fn target(&self) -> AnimationTarget {
        self.target
    }

    pub const fn interpolation(&self) -> AnimationInterpolation {
        self.interpolation
    }

    pub fn authored_times(&self) -> &[f64] {
        &self.authored_times
    }

    pub const fn pre_extrapolation(&self) -> &AnimationExtrapolation {
        &self.pre_extrapolation
    }

    pub const fn post_extrapolation(&self) -> &AnimationExtrapolation {
        &self.post_extrapolation
    }

    pub const fn values(&self) -> &AnimationValues {
        &self.values
    }

    pub const fn timing_metadata(&self) -> &AnimationTimingMetadata {
        &self.timing_metadata
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }

    fn validate_limits(&self, limits: &AnimationLimits) -> Result<(), AnimationBuildError> {
        limits.authored_times().preflight::<f64>(
            AnimationBuildResource::AuthoredTimes,
            self.authored_times.len(),
        )?;
        validate_sample_payload_limits(&self.values, limits)?;
        self.timing_metadata.validate_limits(limits)
    }
}

/// A source-domain animation clip.
#[derive(Debug, PartialEq)]
pub struct AnimationClip {
    duration: f64,
    time_domain: AnimationTimeDomain,
    channels: Vec<AnimationChannel>,
    provenance: Provenance,
    retained_allocation_bytes: usize,
}

impl AnimationClip {
    pub fn try_new(
        duration: f64,
        time_domain: AnimationTimeDomain,
        channels: impl IntoIterator<Item = AnimationChannel>,
        provenance: Provenance,
        limits: &AnimationLimits,
    ) -> Result<Self, AnimationBuildError> {
        if !duration.is_finite() || duration < 0.0 {
            return Err(AnimationBuildError::InvalidDuration);
        }
        time_domain.validate()?;

        let (channels, retained_allocation_bytes) = try_collect_channels(channels, limits)?;
        validate_unique_channel_targets(&channels)?;

        Ok(Self {
            duration,
            time_domain,
            channels,
            provenance,
            retained_allocation_bytes,
        })
    }

    pub const fn duration(&self) -> f64 {
        self.duration
    }

    pub const fn time_domain(&self) -> &AnimationTimeDomain {
        &self.time_domain
    }

    pub fn channels(&self) -> &[AnimationChannel] {
        &self.channels
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }

    /// Creates a zero-allocation seconds view when this source domain defines one.
    pub fn seconds_view(&self) -> Result<AnimationClipSecondsView<'_>, AnimationSecondsViewError> {
        AnimationClipSecondsView::try_new(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AnimationClipSecondsView<'a> {
    clip: &'a AnimationClip,
    conversion: SecondsConversion,
}

impl<'a> AnimationClipSecondsView<'a> {
    fn try_new(clip: &'a AnimationClip) -> Result<Self, AnimationSecondsViewError> {
        let conversion = clip.time_domain.seconds_conversion()?;
        if !conversion.convert(clip.duration).is_finite() {
            return Err(AnimationSecondsViewError::NonFiniteConvertedDuration);
        }
        for (channel_index, channel) in clip.channels.iter().enumerate() {
            for (key_index, time) in channel.authored_times.iter().enumerate() {
                if !conversion.convert(*time).is_finite() {
                    return Err(AnimationSecondsViewError::NonFiniteConvertedTime {
                        channel_index,
                        key_index,
                    });
                }
            }
        }
        Ok(Self { clip, conversion })
    }

    pub fn duration_seconds(self) -> f64 {
        self.conversion.convert(self.clip.duration)
    }

    pub fn channel(self, index: usize) -> Option<AnimationChannelSecondsView<'a>> {
        self.clip
            .channels
            .get(index)
            .map(|channel| AnimationChannelSecondsView {
                channel,
                conversion: self.conversion,
            })
    }

    pub fn channel_count(self) -> usize {
        self.clip.channels.len()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AnimationChannelSecondsView<'a> {
    channel: &'a AnimationChannel,
    conversion: SecondsConversion,
}

impl<'a> AnimationChannelSecondsView<'a> {
    pub const fn source(self) -> &'a AnimationChannel {
        self.channel
    }

    pub fn authored_times_seconds(self) -> AnimationSecondsIter<'a> {
        AnimationSecondsIter {
            inner: self.channel.authored_times.iter(),
            conversion: self.conversion,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnimationSecondsIter<'a> {
    inner: std::slice::Iter<'a, f64>,
    conversion: SecondsConversion,
}

impl Iterator for AnimationSecondsIter<'_> {
    type Item = f64;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|time| self.conversion.convert(*time))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for AnimationSecondsIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner
            .next_back()
            .map(|time| self.conversion.convert(*time))
    }
}

impl ExactSizeIterator for AnimationSecondsIter<'_> {}
impl FusedIterator for AnimationSecondsIter<'_> {}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AnimationSecondsViewError {
    #[error("tick-domain animation has no ticks-per-second rate")]
    UnknownTickRate,

    #[error("namespaced animation time domains do not define a seconds conversion")]
    NamespacedDomain,

    #[error("animation duration is not finite after conversion to seconds")]
    NonFiniteConvertedDuration,

    #[error(
        "animation channel {channel_index} key {key_index} is not finite after conversion to seconds"
    )]
    NonFiniteConvertedTime {
        channel_index: usize,
        key_index: usize,
    },
}

#[derive(Debug, Clone, Copy)]
enum SecondsConversion {
    Identity,
    DivideBy(f64),
}

impl SecondsConversion {
    fn convert(self, authored: f64) -> f64 {
        match self {
            Self::Identity => authored,
            Self::DivideBy(rate) => authored / rate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationRateKind {
    Frames,
    Ticks,
}

impl fmt::Display for AnimationRateKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Frames => "frame",
            Self::Ticks => "tick",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationSamplePart {
    Values,
    InTangents,
    OutTangents,
}

impl fmt::Display for AnimationSamplePart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Values => "values",
            Self::InTangents => "in tangents",
            Self::OutTangents => "out tangents",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationBehaviorKind {
    TimeDomain,
    Interpolation,
    Extrapolation,
}

impl fmt::Display for AnimationBehaviorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TimeDomain => "time domain",
            Self::Interpolation => "interpolation",
            Self::Extrapolation => "extrapolation",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationLimitDimension {
    Count,
    Cells,
    Bytes,
}

impl fmt::Display for AnimationLimitDimension {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Count => "count",
            Self::Cells => "cells",
            Self::Bytes => "bytes",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationBuildResource {
    AuthoredTimes,
    SampleValues,
    SampleInTangents,
    SampleOutTangents,
    Channels,
    ChannelTargets,
    TimingMetadata,
    ChannelPayload,
    ClipPayload,
}

impl fmt::Display for AnimationBuildResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AuthoredTimes => "animation authored times",
            Self::SampleValues => "animation sample values",
            Self::SampleInTangents => "animation sample in tangents",
            Self::SampleOutTangents => "animation sample out tangents",
            Self::Channels => "animation channels",
            Self::ChannelTargets => "animation channel target validation",
            Self::TimingMetadata => "animation timing metadata",
            Self::ChannelPayload => "animation channel retained payload",
            Self::ClipPayload => "animation clip retained payload",
        })
    }
}

#[derive(Debug, Error)]
pub enum AnimationBuildError {
    #[error("animation clip duration must be finite and non-negative")]
    InvalidDuration,

    #[error("animation {kind} rate must be finite and positive")]
    InvalidRate { kind: AnimationRateKind },

    #[error("namespaced animation {behavior} requires an admitted extension schema")]
    BehaviorSchemaRequired { behavior: AnimationBehaviorKind },

    #[error("animation channel must contain at least one authored key time")]
    EmptyChannel,

    #[error("animation key time at index {index} is not finite")]
    NonFiniteTime { index: usize },

    #[error(
        "animation key times must be strictly increasing; indices {previous_index} and {index} are out of order"
    )]
    TimesNotStrictlyIncreasing { previous_index: usize, index: usize },

    #[error("animation target {target} cannot use {values} values")]
    TargetValueKindMismatch {
        target: AnimationTargetKind,
        values: AnimationValueKind,
    },

    #[error("{interpolation:?} interpolation is incompatible with the supplied tangent layout")]
    InterpolationPayloadMismatch {
        interpolation: AnimationInterpolation,
    },

    #[error("animation {part} count does not match key count: expected {expected}, found {actual}")]
    SampleCountMismatch {
        part: AnimationSamplePart,
        expected: usize,
        actual: usize,
    },

    #[error("morph-weight animation width must be greater than zero")]
    InvalidMorphWidth,

    #[error("morph-weight {part} sample {sample_index} has width {actual}, expected {expected}")]
    MorphWidthMismatch {
        part: AnimationSamplePart,
        sample_index: usize,
        expected: usize,
        actual: usize,
    },

    #[error(
        "animation {values} {part} sample {sample_index}, component {component_index} is not finite"
    )]
    NonFiniteSample {
        values: AnimationValueKind,
        part: AnimationSamplePart,
        sample_index: usize,
        component_index: usize,
    },

    #[error("rotation sample {sample_index} is not a unit quaternion")]
    NonUnitQuaternion { sample_index: usize },

    #[error(
        "animation channel {duplicate_channel_index} duplicates channel {first_channel_index} for {target}"
    )]
    DuplicateTarget {
        target: AnimationTargetKind,
        first_channel_index: usize,
        duplicate_channel_index: usize,
    },

    #[error("{resource} size overflows for {count} elements")]
    CollectionSizeOverflow {
        resource: AnimationBuildResource,
        count: usize,
    },

    #[error("{resource} {dimension} limit exceeded: requested {requested}, maximum {maximum}")]
    CollectionLimitExceeded {
        resource: AnimationBuildResource,
        dimension: AnimationLimitDimension,
        requested: usize,
        maximum: usize,
    },

    #[error("animation sample payload size overflows")]
    SamplePayloadSizeOverflow,

    #[error("{resource} retained size overflows")]
    RetainedSizeOverflow { resource: AnimationBuildResource },

    #[error(
        "animation sample payload {dimension} limit exceeded: requested {requested}, maximum {maximum}"
    )]
    SamplePayloadLimitExceeded {
        dimension: AnimationLimitDimension,
        requested: usize,
        maximum: usize,
    },

    #[error("animation timing metadata {dimension} size overflows")]
    TimingMetadataSizeOverflow { dimension: AnimationLimitDimension },

    #[error(
        "animation timing metadata {dimension} limit exceeded: requested {requested}, maximum {maximum}"
    )]
    TimingMetadataLimitExceeded {
        dimension: AnimationLimitDimension,
        requested: usize,
        maximum: usize,
    },

    #[error("animation timing metadata is invalid")]
    InvalidTimingMetadata {
        #[source]
        source: ExtensionEntryValidationError,
    },

    #[error("failed to reserve {resource}")]
    Allocation {
        resource: AnimationBuildResource,
        #[source]
        source: TryReserveError,
    },

    #[error("failed to plan {resource} capacity")]
    CapacityPlan {
        resource: AnimationBuildResource,
        #[source]
        source: CapacityPlanError,
    },
}

fn validate_configured_limit(
    field: AnimationLimitField,
    requested: usize,
    maximum: usize,
) -> Result<(), AnimationLimitConfigurationError> {
    if requested <= maximum {
        Ok(())
    } else {
        Err(AnimationLimitConfigurationError::ExceedsCanonicalMaximum {
            field,
            requested,
            maximum,
        })
    }
}

fn validate_rate(kind: AnimationRateKind, rate: f64) -> Result<(), AnimationBuildError> {
    if rate.is_finite() && rate > 0.0 {
        Ok(())
    } else {
        Err(AnimationBuildError::InvalidRate { kind })
    }
}

fn validate_target_value_kind(
    target: AnimationTargetKind,
    values: AnimationValueKind,
) -> Result<(), AnimationBuildError> {
    let matches = matches!(
        (target, values),
        (
            AnimationTargetKind::NodeTranslation,
            AnimationValueKind::Translations
        ) | (
            AnimationTargetKind::NodeRotation,
            AnimationValueKind::Rotations
        ) | (AnimationTargetKind::NodeScale, AnimationValueKind::Scales)
            | (
                AnimationTargetKind::MeshInstanceMorphWeights,
                AnimationValueKind::MorphWeights
            )
    );
    if matches {
        Ok(())
    } else {
        Err(AnimationBuildError::TargetValueKindMismatch { target, values })
    }
}

fn try_collect_channels(
    channels: impl IntoIterator<Item = AnimationChannel>,
    limits: &AnimationLimits,
) -> Result<(Vec<AnimationChannel>, usize), AnimationBuildError> {
    let mut iterator = channels.into_iter();
    let mut collected = Vec::new();
    let mut nested_retained_bytes = 0_usize;
    let (minimum, _) = iterator.size_hint();
    if minimum != 0 {
        try_reserve_clip_channels(&mut collected, minimum, nested_retained_bytes, limits)?;
    }

    for channel in &mut iterator {
        channel.validate_limits(limits)?;
        let next_nested_retained_bytes = nested_retained_bytes
            .checked_add(channel.retained_allocation_bytes())
            .ok_or(AnimationBuildError::RetainedSizeOverflow {
                resource: AnimationBuildResource::ClipPayload,
            })?;
        let desired =
            collected
                .len()
                .checked_add(1)
                .ok_or(AnimationBuildError::CollectionSizeOverflow {
                    resource: AnimationBuildResource::Channels,
                    count: usize::MAX,
                })?;
        limits
            .channels()
            .preflight::<AnimationChannel>(AnimationBuildResource::Channels, desired)?;
        if collected.len() == collected.capacity() {
            try_reserve_clip_channels(&mut collected, desired, next_nested_retained_bytes, limits)?;
        } else {
            validate_clip_retained_budget(
                collected.capacity(),
                next_nested_retained_bytes,
                limits,
            )?;
        }
        collected.push(channel);
        nested_retained_bytes = next_nested_retained_bytes;
    }

    let retained_allocation_bytes =
        validate_clip_retained_budget(collected.capacity(), nested_retained_bytes, limits)?;
    Ok((collected, retained_allocation_bytes))
}

fn try_reserve_clip_channels(
    channels: &mut Vec<AnimationChannel>,
    desired: usize,
    nested_retained_bytes: usize,
    limits: &AnimationLimits,
) -> Result<(), AnimationBuildError> {
    let resource = AnimationBuildResource::Channels;
    let collection_limit = limits.channels();
    collection_limit.preflight::<AnimationChannel>(resource, desired)?;
    if desired <= channels.capacity() {
        return Ok(());
    }

    let plan = GrowableCapacityPlan::for_type::<AnimationChannel>(
        channels.len(),
        channels.capacity(),
        desired,
    )
    .map_err(|source| AnimationBuildError::CapacityPlan { resource, source })?;
    collection_limit.preflight_capacity::<AnimationChannel>(resource, plan.envelope_capacity())?;
    validate_clip_retained_budget(plan.envelope_capacity(), nested_retained_bytes, limits)?;

    channels
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|source| AnimationBuildError::Allocation { resource, source })?;
    plan.finish(channels.len(), channels.capacity())
        .map_err(|source| AnimationBuildError::CapacityPlan { resource, source })?;
    validate_clip_retained_budget(channels.capacity(), nested_retained_bytes, limits)?;
    Ok(())
}

fn validate_clip_retained_budget(
    channel_capacity: usize,
    nested_retained_bytes: usize,
    limits: &AnimationLimits,
) -> Result<usize, AnimationBuildError> {
    let retained_allocation_bytes = retained_capacity_bytes::<AnimationChannel>(
        channel_capacity,
        AnimationBuildResource::Channels,
    )?
    .checked_add(nested_retained_bytes)
    .ok_or(AnimationBuildError::RetainedSizeOverflow {
        resource: AnimationBuildResource::ClipPayload,
    })?;
    if retained_allocation_bytes > limits.max_clip_retained_bytes {
        return Err(AnimationBuildError::CollectionLimitExceeded {
            resource: AnimationBuildResource::ClipPayload,
            dimension: AnimationLimitDimension::Bytes,
            requested: retained_allocation_bytes,
            maximum: limits.max_clip_retained_bytes,
        });
    }
    Ok(retained_allocation_bytes)
}

fn validate_authored_times(times: &[f64]) -> Result<(), AnimationBuildError> {
    if times.is_empty() {
        return Err(AnimationBuildError::EmptyChannel);
    }
    for (index, time) in times.iter().enumerate() {
        if !time.is_finite() {
            return Err(AnimationBuildError::NonFiniteTime { index });
        }
    }
    for (previous_index, times) in times.windows(2).enumerate() {
        if times[0] >= times[1] {
            return Err(AnimationBuildError::TimesNotStrictlyIncreasing {
                previous_index,
                index: previous_index + 1,
            });
        }
    }
    Ok(())
}

fn validate_unique_channel_targets(
    channels: &[AnimationChannel],
) -> Result<(), AnimationBuildError> {
    let resource = AnimationBuildResource::ChannelTargets;
    let mut first_indices = HashMap::new();
    first_indices
        .try_reserve(channels.len())
        .map_err(|source| AnimationBuildError::Allocation { resource, source })?;

    for (duplicate_channel_index, channel) in channels.iter().enumerate() {
        let target = channel.target();
        if let Some(&first_channel_index) = first_indices.get(&target) {
            return Err(AnimationBuildError::DuplicateTarget {
                target: target.kind(),
                first_channel_index,
                duplicate_channel_index,
            });
        }
        first_indices.insert(target, duplicate_channel_index);
    }
    Ok(())
}

fn validate_animation_values(
    interpolation: AnimationInterpolation,
    key_count: usize,
    values: &AnimationValues,
) -> Result<(), AnimationBuildError> {
    match &values.storage {
        AnimationValueStorage::Translations(samples) => {
            validate_sample_layout(interpolation, key_count, samples)?;
            validate_vec3_samples(AnimationValueKind::Translations, samples)
        }
        AnimationValueStorage::Rotations(samples) => {
            validate_sample_layout(interpolation, key_count, samples)?;
            validate_rotation_samples(samples)
        }
        AnimationValueStorage::Scales(samples) => {
            validate_sample_layout(interpolation, key_count, samples)?;
            validate_vec3_samples(AnimationValueKind::Scales, samples)
        }
        AnimationValueStorage::MorphWeights { width, samples } => {
            validate_sample_layout(interpolation, key_count, samples)?;
            validate_morph_samples(*width, samples)
        }
    }
}

fn validate_sample_payload_limits(
    values: &AnimationValues,
    limits: &AnimationLimits,
) -> Result<(), AnimationBuildError> {
    validate_sample_payload_size(
        SamplePayloadSize {
            cells: values.retained_cell_capacity,
            bytes: values.retained_allocation_bytes,
        },
        limits,
    )
}

fn validate_sample_payload_size(
    size: SamplePayloadSize,
    limits: &AnimationLimits,
) -> Result<(), AnimationBuildError> {
    if size.cells > limits.max_sample_cells_per_channel {
        return Err(AnimationBuildError::SamplePayloadLimitExceeded {
            dimension: AnimationLimitDimension::Cells,
            requested: size.cells,
            maximum: limits.max_sample_cells_per_channel,
        });
    }
    if size.bytes > limits.max_sample_bytes_per_channel {
        return Err(AnimationBuildError::SamplePayloadLimitExceeded {
            dimension: AnimationLimitDimension::Bytes,
            requested: size.bytes,
            maximum: limits.max_sample_bytes_per_channel,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct SamplePayloadSize {
    cells: usize,
    bytes: usize,
}

impl SamplePayloadSize {
    fn try_with_fixed_part<T>(
        mut self,
        capacity: usize,
        cells_per_value: usize,
    ) -> Result<Self, AnimationBuildError> {
        self.add_fixed_part::<T>(capacity, cells_per_value)?;
        Ok(self)
    }

    fn measure_storage(values: &AnimationValueStorage) -> Result<Self, AnimationBuildError> {
        let mut size = Self::default();
        match values {
            AnimationValueStorage::Translations(samples)
            | AnimationValueStorage::Scales(samples) => {
                size.add_fixed_samples(samples, 3)?;
            }
            AnimationValueStorage::Rotations(samples) => match &samples.storage {
                AnimationSampleStorage::Values(values) => {
                    size.add_fixed_part::<Quat>(values.capacity(), 4)?;
                }
                AnimationSampleStorage::CubicSpline {
                    in_tangents,
                    values,
                    out_tangents,
                } => {
                    size.add_fixed_part::<Vec4>(in_tangents.capacity(), 4)?;
                    size.add_fixed_part::<Quat>(values.capacity(), 4)?;
                    size.add_fixed_part::<Vec4>(out_tangents.capacity(), 4)?;
                }
            },
            AnimationValueStorage::MorphWeights { samples, .. } => match &samples.storage {
                AnimationSampleStorage::Values(values) => {
                    size.add_morph_part(values, values.capacity())?;
                }
                AnimationSampleStorage::CubicSpline {
                    in_tangents,
                    values,
                    out_tangents,
                } => {
                    size.add_morph_part(in_tangents, in_tangents.capacity())?;
                    size.add_morph_part(values, values.capacity())?;
                    size.add_morph_part(out_tangents, out_tangents.capacity())?;
                }
            },
        }
        Ok(size)
    }

    fn add_fixed_samples(
        &mut self,
        samples: &AnimationSamples<Vec3, Vec3>,
        cells_per_value: usize,
    ) -> Result<(), AnimationBuildError> {
        match &samples.storage {
            AnimationSampleStorage::Values(values) => {
                self.add_fixed_part::<Vec3>(values.capacity(), cells_per_value)?;
            }
            AnimationSampleStorage::CubicSpline {
                in_tangents,
                values,
                out_tangents,
            } => {
                self.add_fixed_part::<Vec3>(in_tangents.capacity(), cells_per_value)?;
                self.add_fixed_part::<Vec3>(values.capacity(), cells_per_value)?;
                self.add_fixed_part::<Vec3>(out_tangents.capacity(), cells_per_value)?;
            }
        }
        Ok(())
    }

    fn add_fixed_part<T>(
        &mut self,
        count: usize,
        cells_per_value: usize,
    ) -> Result<(), AnimationBuildError> {
        let cells = count
            .checked_mul(cells_per_value)
            .ok_or(AnimationBuildError::SamplePayloadSizeOverflow)?;
        let bytes = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(AnimationBuildError::SamplePayloadSizeOverflow)?;
        self.add(cells, bytes)
    }

    fn add_morph_part(
        &mut self,
        samples: &[Vec<f32>],
        outer_capacity: usize,
    ) -> Result<(), AnimationBuildError> {
        let headers = outer_capacity
            .checked_mul(std::mem::size_of::<Vec<f32>>())
            .ok_or(AnimationBuildError::SamplePayloadSizeOverflow)?;
        self.add(0, headers)?;
        for sample in samples {
            let bytes = sample
                .capacity()
                .checked_mul(std::mem::size_of::<f32>())
                .ok_or(AnimationBuildError::SamplePayloadSizeOverflow)?;
            self.add(sample.capacity(), bytes)?;
        }
        Ok(())
    }

    fn add(&mut self, cells: usize, bytes: usize) -> Result<(), AnimationBuildError> {
        self.cells = self
            .cells
            .checked_add(cells)
            .ok_or(AnimationBuildError::SamplePayloadSizeOverflow)?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(AnimationBuildError::SamplePayloadSizeOverflow)?;
        Ok(())
    }
}

fn validate_sample_layout<Value, Tangent>(
    interpolation: AnimationInterpolation,
    key_count: usize,
    samples: &AnimationSamples<Value, Tangent>,
) -> Result<(), AnimationBuildError> {
    match (interpolation, &samples.storage) {
        (
            AnimationInterpolation::Step | AnimationInterpolation::Linear,
            AnimationSampleStorage::Values(values),
        ) => validate_sample_count(AnimationSamplePart::Values, key_count, values.len()),
        (
            AnimationInterpolation::CubicSpline,
            AnimationSampleStorage::CubicSpline {
                in_tangents,
                values,
                out_tangents,
            },
        ) => {
            validate_sample_count(
                AnimationSamplePart::InTangents,
                key_count,
                in_tangents.len(),
            )?;
            validate_sample_count(AnimationSamplePart::Values, key_count, values.len())?;
            validate_sample_count(
                AnimationSamplePart::OutTangents,
                key_count,
                out_tangents.len(),
            )
        }
        _ => Err(AnimationBuildError::InterpolationPayloadMismatch { interpolation }),
    }
}

fn validate_sample_count(
    part: AnimationSamplePart,
    expected: usize,
    actual: usize,
) -> Result<(), AnimationBuildError> {
    if actual == expected {
        Ok(())
    } else {
        Err(AnimationBuildError::SampleCountMismatch {
            part,
            expected,
            actual,
        })
    }
}

fn validate_vec3_samples(
    kind: AnimationValueKind,
    samples: &AnimationSamples<Vec3, Vec3>,
) -> Result<(), AnimationBuildError> {
    validate_vec3_array(kind, AnimationSamplePart::Values, samples.values())?;
    if let Some(in_tangents) = samples.in_tangents() {
        validate_vec3_array(kind, AnimationSamplePart::InTangents, in_tangents)?;
    }
    if let Some(out_tangents) = samples.out_tangents() {
        validate_vec3_array(kind, AnimationSamplePart::OutTangents, out_tangents)?;
    }
    Ok(())
}

fn validate_vec3_array(
    kind: AnimationValueKind,
    part: AnimationSamplePart,
    samples: &[Vec3],
) -> Result<(), AnimationBuildError> {
    for (sample_index, sample) in samples.iter().enumerate() {
        let components = [sample.x, sample.y, sample.z];
        if let Some(component_index) = components
            .iter()
            .position(|component| !component.is_finite())
        {
            return Err(AnimationBuildError::NonFiniteSample {
                values: kind,
                part,
                sample_index,
                component_index,
            });
        }
    }
    Ok(())
}

fn validate_rotation_samples(
    samples: &AnimationSamples<Quat, Vec4>,
) -> Result<(), AnimationBuildError> {
    for (sample_index, sample) in samples.values().iter().enumerate() {
        let components = [sample.x, sample.y, sample.z, sample.w];
        if let Some(component_index) = components
            .iter()
            .position(|component| !component.is_finite())
        {
            return Err(AnimationBuildError::NonFiniteSample {
                values: AnimationValueKind::Rotations,
                part: AnimationSamplePart::Values,
                sample_index,
                component_index,
            });
        }
        if !sample.is_unit() {
            return Err(AnimationBuildError::NonUnitQuaternion { sample_index });
        }
    }
    if let Some(in_tangents) = samples.in_tangents() {
        validate_vec4_array(AnimationSamplePart::InTangents, in_tangents)?;
    }
    if let Some(out_tangents) = samples.out_tangents() {
        validate_vec4_array(AnimationSamplePart::OutTangents, out_tangents)?;
    }
    Ok(())
}

fn validate_vec4_array(
    part: AnimationSamplePart,
    samples: &[Vec4],
) -> Result<(), AnimationBuildError> {
    for (sample_index, sample) in samples.iter().enumerate() {
        let components = [sample.x, sample.y, sample.z, sample.w];
        if let Some(component_index) = components
            .iter()
            .position(|component| !component.is_finite())
        {
            return Err(AnimationBuildError::NonFiniteSample {
                values: AnimationValueKind::Rotations,
                part,
                sample_index,
                component_index,
            });
        }
    }
    Ok(())
}

fn validate_morph_samples(
    width: usize,
    samples: &AnimationSamples<Vec<f32>, Vec<f32>>,
) -> Result<(), AnimationBuildError> {
    validate_morph_width(width)?;
    validate_morph_array(width, AnimationSamplePart::Values, samples.values())?;
    if let Some(in_tangents) = samples.in_tangents() {
        validate_morph_array(width, AnimationSamplePart::InTangents, in_tangents)?;
    }
    if let Some(out_tangents) = samples.out_tangents() {
        validate_morph_array(width, AnimationSamplePart::OutTangents, out_tangents)?;
    }
    Ok(())
}

fn validate_morph_width(width: usize) -> Result<(), AnimationBuildError> {
    if width == 0 {
        Err(AnimationBuildError::InvalidMorphWidth)
    } else {
        Ok(())
    }
}

fn validate_morph_array(
    width: usize,
    part: AnimationSamplePart,
    samples: &[Vec<f32>],
) -> Result<(), AnimationBuildError> {
    for (sample_index, sample) in samples.iter().enumerate() {
        if sample.len() != width {
            return Err(AnimationBuildError::MorphWidthMismatch {
                part,
                sample_index,
                expected: width,
                actual: sample.len(),
            });
        }
        if let Some(component_index) = sample.iter().position(|component| !component.is_finite()) {
            return Err(AnimationBuildError::NonFiniteSample {
                values: AnimationValueKind::MorphWeights,
                part,
                sample_index,
                component_index,
            });
        }
    }
    Ok(())
}

fn retained_capacity_bytes<T>(
    capacity: usize,
    resource: AnimationBuildResource,
) -> Result<usize, AnimationBuildError> {
    capacity
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(AnimationBuildError::RetainedSizeOverflow { resource })
}

fn try_collect<T>(
    values: impl IntoIterator<Item = T>,
    resource: AnimationBuildResource,
    limit: AnimationCollectionLimit,
) -> Result<Vec<T>, AnimationBuildError> {
    let mut iterator = values.into_iter();
    let mut collected = Vec::new();
    let (minimum, _) = iterator.size_hint();
    if minimum != 0 {
        try_reserve_collection(&mut collected, minimum, resource, limit)?;
    }
    for value in &mut iterator {
        let desired =
            collected
                .len()
                .checked_add(1)
                .ok_or(AnimationBuildError::CollectionSizeOverflow {
                    resource,
                    count: usize::MAX,
                })?;
        if collected.len() == collected.capacity() {
            try_reserve_collection(&mut collected, desired, resource, limit)?;
        } else {
            limit.preflight::<T>(resource, desired)?;
        }
        collected.push(value);
    }
    Ok(collected)
}

fn try_reserve_collection<T>(
    values: &mut Vec<T>,
    desired: usize,
    resource: AnimationBuildResource,
    limit: AnimationCollectionLimit,
) -> Result<(), AnimationBuildError> {
    limit.preflight::<T>(resource, desired)?;
    if desired <= values.capacity() {
        return Ok(());
    }

    let plan = GrowableCapacityPlan::for_type::<T>(values.len(), values.capacity(), desired)
        .map_err(|source| AnimationBuildError::CapacityPlan { resource, source })?;
    limit.preflight_capacity::<T>(resource, plan.envelope_capacity())?;
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|source| AnimationBuildError::Allocation { resource, source })?;
    plan.finish(values.len(), values.capacity())
        .map_err(|source| AnimationBuildError::CapacityPlan { resource, source })?;
    limit.preflight_capacity::<T>(resource, values.capacity())?;
    Ok(())
}

fn try_collect_fixed_sample_part<T>(
    values: impl IntoIterator<Item = T>,
    resource: AnimationBuildResource,
    cells_per_value: usize,
    collection_limit: AnimationCollectionLimit,
    base: SamplePayloadSize,
    limits: &AnimationLimits,
) -> Result<(Vec<T>, SamplePayloadSize), AnimationBuildError> {
    let mut iterator = values.into_iter();
    let mut collected = Vec::new();
    let (minimum, _) = iterator.size_hint();
    if minimum != 0 {
        try_reserve_fixed_sample_part(
            &mut collected,
            minimum,
            resource,
            cells_per_value,
            collection_limit,
            base,
            limits,
        )?;
    }

    for value in &mut iterator {
        let desired =
            collected
                .len()
                .checked_add(1)
                .ok_or(AnimationBuildError::CollectionSizeOverflow {
                    resource,
                    count: usize::MAX,
                })?;
        if collected.len() == collected.capacity() {
            try_reserve_fixed_sample_part(
                &mut collected,
                desired,
                resource,
                cells_per_value,
                collection_limit,
                base,
                limits,
            )?;
        } else {
            collection_limit.preflight::<T>(resource, desired)?;
        }
        collected.push(value);
    }

    let footprint = base.try_with_fixed_part::<T>(collected.capacity(), cells_per_value)?;
    validate_sample_payload_size(footprint, limits)?;
    Ok((collected, footprint))
}

fn try_reserve_fixed_sample_part<T>(
    values: &mut Vec<T>,
    desired: usize,
    resource: AnimationBuildResource,
    cells_per_value: usize,
    collection_limit: AnimationCollectionLimit,
    base: SamplePayloadSize,
    limits: &AnimationLimits,
) -> Result<(), AnimationBuildError> {
    collection_limit.preflight::<T>(resource, desired)?;
    if desired <= values.capacity() {
        return Ok(());
    }

    let plan = GrowableCapacityPlan::for_type::<T>(values.len(), values.capacity(), desired)
        .map_err(|source| AnimationBuildError::CapacityPlan { resource, source })?;
    collection_limit.preflight_capacity::<T>(resource, plan.envelope_capacity())?;
    let projected = base.try_with_fixed_part::<T>(plan.envelope_capacity(), cells_per_value)?;
    validate_sample_payload_size(projected, limits)?;

    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|source| AnimationBuildError::Allocation { resource, source })?;
    plan.finish(values.len(), values.capacity())
        .map_err(|source| AnimationBuildError::CapacityPlan { resource, source })?;
    let actual = base.try_with_fixed_part::<T>(values.capacity(), cells_per_value)?;
    validate_sample_payload_size(actual, limits)
}

fn try_collect_morph_rows<Row>(
    rows: impl IntoIterator<Item = Row>,
    resource: AnimationBuildResource,
    base: SamplePayloadSize,
    limits: &AnimationLimits,
) -> Result<(Vec<Vec<f32>>, SamplePayloadSize), AnimationBuildError>
where
    Row: IntoIterator<Item = f32>,
{
    let mut iterator = rows.into_iter();
    let mut collected = Vec::new();
    let outer_limit = limits.sample_array();
    let mut retained_inner = SamplePayloadSize::default();
    let (minimum, _) = iterator.size_hint();
    if minimum != 0 {
        try_reserve_morph_outer(
            &mut collected,
            minimum,
            resource,
            base,
            retained_inner,
            limits,
        )?;
    }

    for row in &mut iterator {
        let desired =
            collected
                .len()
                .checked_add(1)
                .ok_or(AnimationBuildError::CollectionSizeOverflow {
                    resource,
                    count: usize::MAX,
                })?;
        outer_limit.preflight::<Vec<f32>>(resource, desired)?;
        if collected.len() == collected.capacity() {
            try_reserve_morph_outer(
                &mut collected,
                desired,
                resource,
                base,
                retained_inner,
                limits,
            )?;
        }

        let row_base = validate_morph_retained_budget(
            base,
            retained_inner,
            collected.capacity(),
            resource,
            limits,
        )?;
        let remaining_cells = limits
            .max_sample_cells_per_channel
            .checked_sub(row_base.cells)
            .ok_or(AnimationBuildError::SamplePayloadLimitExceeded {
                dimension: AnimationLimitDimension::Cells,
                requested: row_base.cells,
                maximum: limits.max_sample_cells_per_channel,
            })?;
        let remaining_bytes = limits
            .max_sample_bytes_per_channel
            .checked_sub(row_base.bytes)
            .ok_or(AnimationBuildError::SamplePayloadLimitExceeded {
                dimension: AnimationLimitDimension::Bytes,
                requested: row_base.bytes,
                maximum: limits.max_sample_bytes_per_channel,
            })?;
        let row_limit = AnimationCollectionLimit::new(remaining_cells, remaining_bytes);
        let (row, _) = try_collect_fixed_sample_part(row, resource, 1, row_limit, row_base, limits)
            .map_err(|error| {
                map_morph_row_limit_error(error, row_base.cells, row_base.bytes, limits)
            })?;
        let row_cells = row.capacity();
        let row_bytes = row_cells
            .checked_mul(std::mem::size_of::<f32>())
            .ok_or(AnimationBuildError::SamplePayloadSizeOverflow)?;
        retained_inner.add(row_cells, row_bytes)?;
        collected.push(row);
    }

    let footprint = validate_morph_retained_budget(
        base,
        retained_inner,
        collected.capacity(),
        resource,
        limits,
    )?;
    Ok((collected, footprint))
}

fn try_reserve_morph_outer(
    values: &mut Vec<Vec<f32>>,
    desired: usize,
    resource: AnimationBuildResource,
    base: SamplePayloadSize,
    retained_inner: SamplePayloadSize,
    limits: &AnimationLimits,
) -> Result<(), AnimationBuildError> {
    let collection_limit = limits.sample_array();
    collection_limit.preflight::<Vec<f32>>(resource, desired)?;
    if desired <= values.capacity() {
        return Ok(());
    }

    let plan = GrowableCapacityPlan::for_type::<Vec<f32>>(values.len(), values.capacity(), desired)
        .map_err(|source| AnimationBuildError::CapacityPlan { resource, source })?;
    collection_limit.preflight_capacity::<Vec<f32>>(resource, plan.envelope_capacity())?;
    validate_morph_retained_budget(
        base,
        retained_inner,
        plan.envelope_capacity(),
        resource,
        limits,
    )?;

    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|source| AnimationBuildError::Allocation { resource, source })?;
    plan.finish(values.len(), values.capacity())
        .map_err(|source| AnimationBuildError::CapacityPlan { resource, source })?;
    validate_morph_retained_budget(base, retained_inner, values.capacity(), resource, limits)?;
    Ok(())
}

fn validate_morph_retained_budget(
    base: SamplePayloadSize,
    retained_inner: SamplePayloadSize,
    outer_capacity: usize,
    resource: AnimationBuildResource,
    limits: &AnimationLimits,
) -> Result<SamplePayloadSize, AnimationBuildError> {
    let outer_bytes = retained_capacity_bytes::<Vec<f32>>(outer_capacity, resource)?;
    let retained_bytes = outer_bytes
        .checked_add(retained_inner.bytes)
        .ok_or(AnimationBuildError::SamplePayloadSizeOverflow)?;
    let mut footprint = base;
    footprint.add(retained_inner.cells, retained_bytes)?;
    validate_sample_payload_size(footprint, limits)?;
    Ok(footprint)
}

fn map_morph_row_limit_error(
    error: AnimationBuildError,
    base_cells: usize,
    base_bytes: usize,
    limits: &AnimationLimits,
) -> AnimationBuildError {
    match error {
        AnimationBuildError::CollectionLimitExceeded {
            dimension: AnimationLimitDimension::Count,
            requested,
            ..
        } => AnimationBuildError::SamplePayloadLimitExceeded {
            dimension: AnimationLimitDimension::Cells,
            requested: base_cells.saturating_add(requested),
            maximum: limits.max_sample_cells_per_channel,
        },
        AnimationBuildError::CollectionLimitExceeded {
            dimension: AnimationLimitDimension::Bytes,
            requested,
            ..
        } => AnimationBuildError::SamplePayloadLimitExceeded {
            dimension: AnimationLimitDimension::Bytes,
            requested: base_bytes.saturating_add(requested),
            maximum: limits.max_sample_bytes_per_channel,
        },
        AnimationBuildError::CollectionSizeOverflow { .. } => {
            AnimationBuildError::SamplePayloadSizeOverflow
        }
        other => other,
    }
}

fn try_collect_timing_metadata(
    entries: impl IntoIterator<Item = ExtensionEntry>,
    limits: &AnimationLimits,
) -> Result<Vec<ExtensionEntry>, AnimationBuildError> {
    try_collect(
        entries,
        AnimationBuildResource::TimingMetadata,
        limits.timing_metadata(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{ExtensionEntry, ExtensionSchemaId, ExtensionValue};
    use crate::namespaced::NamespacedName;
    use crate::provenance::test_provenance_fixture;
    use std::cell::Cell;
    use std::marker::PhantomData;
    use std::rc::Rc;

    struct NeverPolled<T> {
        remaining: usize,
        marker: PhantomData<T>,
    }

    impl<T> NeverPolled<T> {
        fn one() -> Self {
            Self {
                remaining: 1,
                marker: PhantomData,
            }
        }
    }

    impl<T> Iterator for NeverPolled<T> {
        type Item = T;

        fn next(&mut self) -> Option<Self::Item> {
            panic!("iterator must not be polled after preflight rejection")
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (self.remaining, Some(self.remaining))
        }
    }

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn marker() {}
                }

                impl<T: ?Sized> AmbiguousIfClone<()> for T {}

                struct ImplementsClone;
                impl<T: ?Sized + Clone> AmbiguousIfClone<ImplementsClone> for T {}

                let _ = <$type as AmbiguousIfClone<_>>::marker;
            };
        };
    }

    assert_not_clone!(AnimationSamples<Vec3, Vec3>);
    assert_not_clone!(AnimationValues);

    fn node(index: usize) -> NodeIndex {
        NodeIndex::try_from(index).expect("test node index should fit")
    }

    fn mesh_instance(index: usize) -> MeshInstanceIndex {
        MeshInstanceIndex::try_from(index).expect("test mesh-instance index should fit")
    }

    fn translation_values(count: usize) -> AnimationValues {
        AnimationValues::try_translations(vec![Vec3::ZERO; count], &AnimationLimits::canonical())
            .expect("test translation values should fit canonical limits")
    }

    #[allow(clippy::too_many_arguments)]
    fn channel_with_limits(
        target: AnimationTarget,
        interpolation: AnimationInterpolation,
        authored_times: impl IntoIterator<Item = f64>,
        pre_extrapolation: AnimationExtrapolation,
        post_extrapolation: AnimationExtrapolation,
        values: AnimationValues,
        timing_metadata: AnimationTimingMetadata,
        provenance: Provenance,
        limits: &AnimationLimits,
    ) -> Result<AnimationChannel, AnimationBuildError> {
        AnimationChannel::try_new(
            target,
            interpolation,
            authored_times,
            pre_extrapolation,
            post_extrapolation,
            values,
            timing_metadata,
            provenance,
            limits,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn channel(
        target: AnimationTarget,
        interpolation: AnimationInterpolation,
        authored_times: impl IntoIterator<Item = f64>,
        pre_extrapolation: AnimationExtrapolation,
        post_extrapolation: AnimationExtrapolation,
        values: AnimationValues,
        provenance: Provenance,
    ) -> Result<AnimationChannel, AnimationBuildError> {
        channel_with_limits(
            target,
            interpolation,
            authored_times,
            pre_extrapolation,
            post_extrapolation,
            values,
            AnimationTimingMetadata::empty(),
            provenance,
            &AnimationLimits::canonical(),
        )
    }

    fn clip_with_limits(
        duration: f64,
        time_domain: AnimationTimeDomain,
        channels: impl IntoIterator<Item = AnimationChannel>,
        provenance: Provenance,
        limits: &AnimationLimits,
    ) -> Result<AnimationClip, AnimationBuildError> {
        AnimationClip::try_new(duration, time_domain, channels, provenance, limits)
    }

    fn clip(
        duration: f64,
        time_domain: AnimationTimeDomain,
        channels: impl IntoIterator<Item = AnimationChannel>,
        provenance: Provenance,
    ) -> Result<AnimationClip, AnimationBuildError> {
        clip_with_limits(
            duration,
            time_domain,
            channels,
            provenance,
            &AnimationLimits::canonical(),
        )
    }

    #[test]
    fn preserves_adjacent_f64_times_at_long_durations() {
        let (_arena, provenance) = test_provenance_fixture();
        let base = 2_f64.powi(40);
        let adjacent = f64::from_bits(base.to_bits() + 1);
        let channel = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Linear,
            [base, adjacent],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            translation_values(2),
            provenance,
        )
        .expect("adjacent f64 times should remain distinct");
        let clip = clip(
            adjacent,
            AnimationTimeDomain::Seconds,
            [channel],
            provenance,
        )
        .expect("long finite durations should be accepted");

        assert_eq!(clip.duration(), adjacent);
        assert_eq!(clip.channels()[0].authored_times(), &[base, adjacent]);
        assert!(clip.channels()[0].authored_times()[0] < clip.channels()[0].authored_times()[1]);
    }

    #[test]
    fn rejects_duplicate_authored_times() {
        let (_arena, provenance) = test_provenance_fixture();
        let result = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Step,
            [3.0, 3.0],
            AnimationExtrapolation::SourceDefault,
            AnimationExtrapolation::SourceDefault,
            translation_values(2),
            provenance,
        );

        assert!(matches!(
            result,
            Err(AnimationBuildError::TimesNotStrictlyIncreasing {
                previous_index: 0,
                index: 1
            })
        ));
    }

    #[test]
    fn stores_target_indices_and_validates_morph_width() {
        let (_arena, provenance) = test_provenance_fixture();
        let node_target = AnimationTarget::NodeTranslation(node(7));
        assert!(matches!(
            node_target,
            AnimationTarget::NodeTranslation(index) if index.as_u32() == 7
        ));

        let target = AnimationTarget::MeshInstanceMorphWeights(mesh_instance(3));
        let values = AnimationValues::try_morph_weights(
            2,
            [vec![0.0, 1.0], vec![0.25, 0.75]],
            &AnimationLimits::canonical(),
        )
        .expect("test morph values should fit canonical limits");
        let valid_channel = channel(
            target,
            AnimationInterpolation::Linear,
            [0.0, 1.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Repeat,
            values,
            provenance,
        )
        .expect("matching morph samples should be accepted");
        assert_eq!(valid_channel.target(), target);

        let wrong_width =
            AnimationValues::try_morph_weights(2, [vec![0.0]], &AnimationLimits::canonical())
                .expect("a bounded malformed width should reach channel validation");
        let result = channel(
            target,
            AnimationInterpolation::Step,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            wrong_width,
            provenance,
        );
        assert!(matches!(
            result,
            Err(AnimationBuildError::MorphWidthMismatch {
                part: AnimationSamplePart::Values,
                sample_index: 0,
                expected: 2,
                actual: 1,
            })
        ));

        let wrong_target = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Step,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            AnimationValues::try_morph_weights(1, [vec![0.0]], &AnimationLimits::canonical())
                .expect("test morph values should fit canonical limits"),
            provenance,
        );
        assert!(matches!(
            wrong_target,
            Err(AnimationBuildError::TargetValueKindMismatch { .. })
        ));
    }

    #[test]
    fn cubic_rotation_tangents_are_finite_vectors_not_unit_quaternions() {
        let (_arena, provenance) = test_provenance_fixture();
        let values = AnimationValues::try_cubic_rotations(
            [Vec4::new(20.0, -4.0, 3.0, 0.5)],
            [Quat::IDENTITY],
            [Vec4::new(-8.0, 2.0, 1.0, 7.0)],
            &AnimationLimits::canonical(),
        )
        .expect("test cubic rotations should fit canonical limits");
        let result = channel(
            AnimationTarget::NodeRotation(node(0)),
            AnimationInterpolation::CubicSpline,
            [0.0],
            AnimationExtrapolation::Linear,
            AnimationExtrapolation::Linear,
            values,
            provenance,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn rejects_non_unit_rotation_samples() {
        let (_arena, provenance) = test_provenance_fixture();
        let result = channel(
            AnimationTarget::NodeRotation(node(0)),
            AnimationInterpolation::Linear,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            AnimationValues::try_rotations(
                [Quat {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 0.5,
                }],
                &AnimationLimits::canonical(),
            )
            .expect("bounded rotation storage should be admitted before semantic validation"),
            provenance,
        );

        assert!(matches!(
            result,
            Err(AnimationBuildError::NonUnitQuaternion { sample_index: 0 })
        ));
    }

    #[test]
    fn validates_rates_and_requires_schema_for_namespaced_behavior() {
        let (_arena, provenance) = test_provenance_fixture();
        assert!(matches!(
            AnimationTimeDomain::frames(0.0),
            Err(AnimationBuildError::InvalidRate {
                kind: AnimationRateKind::Frames
            })
        ));
        assert!(matches!(
            AnimationTimeDomain::ticks(Some(f64::INFINITY)),
            Err(AnimationBuildError::InvalidRate {
                kind: AnimationRateKind::Ticks
            })
        ));
        assert_eq!(
            AnimationTimeDomain::ticks(None).expect("an unknown tick rate is valid"),
            AnimationTimeDomain::Ticks { rate: None }
        );
        assert!(matches!(
            AnimationTimeDomain::try_namespaced(None),
            Err(AnimationBuildError::BehaviorSchemaRequired {
                behavior: AnimationBehaviorKind::TimeDomain
            })
        ));

        assert!(matches!(
            AnimationExtrapolation::try_namespaced(None),
            Err(AnimationBuildError::BehaviorSchemaRequired {
                behavior: AnimationBehaviorKind::Extrapolation
            })
        ));
        let schema = ExtensionSchemaId::new("assimp:cycle-offset", 1)
            .expect("test behavior schema identity should be valid");
        let extrapolation = AnimationExtrapolation::try_namespaced(Some(schema.clone()))
            .expect("schema-bound behavior should be accepted");
        assert_eq!(extrapolation.schema_id(), Some(&schema));

        let channel = channel(
            AnimationTarget::NodeScale(node(0)),
            AnimationInterpolation::Step,
            [0.0],
            extrapolation.clone(),
            AnimationExtrapolation::Repeat,
            AnimationValues::try_scales([Vec3::new(1.0, 1.0, 1.0)], &AnimationLimits::canonical())
                .expect("test scale values should fit canonical limits"),
            provenance,
        )
        .expect("valid extrapolation should be retained");
        let time_schema = ExtensionSchemaId::new("assimp:source-ticks", 4)
            .expect("test time-domain schema identity should be valid");
        let domain = AnimationTimeDomain::try_namespaced(Some(time_schema.clone()))
            .expect("schema-bound namespaced time domain should be accepted");
        assert_eq!(domain.schema_id(), Some(&time_schema));
        let clip = clip(0.0, domain.clone(), [channel], provenance)
            .expect("namespaced source time should be accepted");

        assert_eq!(clip.time_domain(), &domain);
        assert_eq!(clip.channels()[0].pre_extrapolation(), &extrapolation);
        assert_eq!(
            clip.channels()[0].post_extrapolation(),
            &AnimationExtrapolation::Repeat
        );
    }

    #[test]
    fn seconds_view_is_typed_lazy_and_only_available_for_defined_domains() {
        let (_arena, provenance) = test_provenance_fixture();
        let channel = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Linear,
            [0.0, 24.0, 48.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            translation_values(3),
            provenance,
        )
        .expect("frame-domain channel should be valid");
        let frame_clip = clip(
            48.0,
            AnimationTimeDomain::frames(24.0).expect("frame rate should be valid"),
            [channel],
            provenance,
        )
        .expect("frame-domain clip should be valid");
        let view = frame_clip
            .seconds_view()
            .expect("known frame rate defines seconds");
        assert_eq!(view.duration_seconds(), 2.0);
        assert_eq!(view.channel_count(), 1);
        assert_eq!(
            view.channel(0)
                .expect("channel should exist")
                .authored_times_seconds()
                .collect::<Vec<_>>(),
            vec![0.0, 1.0, 2.0]
        );

        let unknown_ticks = clip(
            1.0,
            AnimationTimeDomain::ticks(None).expect("unknown tick rate is preservable"),
            [],
            provenance,
        )
        .expect("unknown tick rate is valid source data");
        assert!(matches!(
            unknown_ticks.seconds_view(),
            Err(AnimationSecondsViewError::UnknownTickRate)
        ));

        let time_schema = ExtensionSchemaId::new("source:beat", 1)
            .expect("test time-domain schema identity should be valid");
        let namespaced = clip(
            1.0,
            AnimationTimeDomain::try_namespaced(Some(time_schema.clone()))
                .expect("schema-bound namespaced time domain should be valid"),
            [],
            provenance,
        )
        .expect("namespaced source time is preservable");
        assert_eq!(namespaced.time_domain().schema_id(), Some(&time_schema));
        assert!(matches!(
            namespaced.seconds_view(),
            Err(AnimationSecondsViewError::NamespacedDomain)
        ));
    }

    #[test]
    fn timing_metadata_has_canonical_aggregate_limits() {
        let (_arena, provenance) = test_provenance_fixture();
        let limits = AnimationLimits::canonical()
            .try_with_timing_metadata_limits(1, 1, 4)
            .expect("lower test limits should be valid");
        let oversized = ExtensionEntry::source_extension(
            NamespacedName::new("gltf:input-accessor").expect("metadata key should be canonical"),
            ExtensionValue::try_string("12345").expect("value is below extension-wide limit"),
            provenance,
        );
        assert!(matches!(
            AnimationTimingMetadata::try_new([oversized], &limits),
            Err(AnimationBuildError::TimingMetadataLimitExceeded {
                dimension: AnimationLimitDimension::Bytes,
                requested: 5,
                maximum: 4,
            })
        ));

        let first = ExtensionEntry::descriptive_metadata(
            NamespacedName::new("gltf:time-set").expect("metadata key should be canonical"),
            ExtensionValue::from_u64(0),
            provenance,
        );
        let second = ExtensionEntry::descriptive_metadata(
            NamespacedName::new("gltf:time-offset").expect("metadata key should be canonical"),
            ExtensionValue::from_u64(1),
            provenance,
        );
        assert!(matches!(
            AnimationTimingMetadata::try_new([first, second], &limits),
            Err(AnimationBuildError::CollectionLimitExceeded {
                resource: AnimationBuildResource::TimingMetadata,
                dimension: AnimationLimitDimension::Count,
                requested: 2,
                maximum: 1,
            })
        ));
    }

    #[test]
    fn collection_preflight_enforces_count_and_bytes_before_growth() {
        let (_arena, provenance) = test_provenance_fixture();
        let count_limits = AnimationLimits::canonical()
            .try_with_max_keys_per_channel(1)
            .expect("lower key limit should be valid");
        let count_result = channel_with_limits(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Linear,
            [0.0, 1.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            translation_values(2),
            AnimationTimingMetadata::empty(),
            provenance,
            &count_limits,
        );
        assert!(matches!(
            count_result,
            Err(AnimationBuildError::CollectionLimitExceeded {
                resource: AnimationBuildResource::AuthoredTimes,
                dimension: AnimationLimitDimension::Count,
                requested: 2,
                maximum: 1,
            })
        ));

        let wide_channel = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Linear,
            [0.0, 1.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            translation_values(2),
            provenance,
        )
        .expect("canonical limits should admit two keys");
        let nested_result = clip_with_limits(
            1.0,
            AnimationTimeDomain::Seconds,
            [wide_channel],
            provenance,
            &count_limits,
        );
        assert!(matches!(
            nested_result,
            Err(AnimationBuildError::CollectionLimitExceeded {
                resource: AnimationBuildResource::AuthoredTimes,
                dimension: AnimationLimitDimension::Count,
                requested: 2,
                maximum: 1,
            })
        ));

        let byte_limits = AnimationLimits::canonical()
            .try_with_max_collection_bytes(4)
            .expect("lower byte limit should be valid");
        assert!(matches!(
            byte_limits
                .authored_times()
                .preflight::<f64>(AnimationBuildResource::AuthoredTimes, 1),
            Err(AnimationBuildError::CollectionLimitExceeded {
                resource: AnimationBuildResource::AuthoredTimes,
                dimension: AnimationLimitDimension::Bytes,
                requested: 8,
                maximum: 4,
            })
        ));
        let byte_result = channel_with_limits(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Step,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            translation_values(1),
            AnimationTimingMetadata::empty(),
            provenance,
            &byte_limits,
        );
        assert!(matches!(
            byte_result,
            Err(AnimationBuildError::CollectionLimitExceeded {
                resource: AnimationBuildResource::AuthoredTimes,
                dimension: AnimationLimitDimension::Bytes,
                requested: 8,
                maximum: 4,
            })
        ));
    }

    #[test]
    fn interpolation_layout_and_cubic_cardinality_are_exact() {
        let (_arena, provenance) = test_provenance_fixture();
        let tangent_payload = AnimationValues::try_cubic_translations(
            [Vec3::ZERO],
            [Vec3::ZERO],
            [Vec3::ZERO],
            &AnimationLimits::canonical(),
        )
        .expect("test cubic translations should fit canonical limits");
        let step_result = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Step,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            tangent_payload,
            provenance,
        );
        assert!(matches!(
            step_result,
            Err(AnimationBuildError::InterpolationPayloadMismatch {
                interpolation: AnimationInterpolation::Step
            })
        ));

        let missing_out_tangent = AnimationValues::try_cubic_translations(
            [Vec3::ZERO],
            [Vec3::ZERO],
            [],
            &AnimationLimits::canonical(),
        )
        .expect("bounded cardinality mismatch should reach channel validation");
        let cubic_result = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::CubicSpline,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            missing_out_tangent,
            provenance,
        );
        assert!(matches!(
            cubic_result,
            Err(AnimationBuildError::SampleCountMismatch {
                part: AnimationSamplePart::OutTangents,
                expected: 1,
                actual: 0,
            })
        ));
    }

    #[test]
    fn sample_construction_discards_hostile_capacity_and_reports_retained_storage() {
        let limits = AnimationLimits::canonical();
        let mut translations = Vec::with_capacity(1_024);
        translations.push(Vec3::ZERO);
        let source_translation_bytes = translations.capacity() * std::mem::size_of::<Vec3>();

        let translations = AnimationValues::try_translations(translations, &limits)
            .expect("one translation sample should be admitted");
        assert_eq!(translations.key_count(), 1);
        assert_eq!(translations.retained_cell_capacity(), 3);
        assert!(translations.retained_allocation_bytes() < source_translation_bytes);

        let mut morph_row = Vec::with_capacity(2_048);
        morph_row.extend([0.25, 0.75]);
        let source_row_bytes = morph_row.capacity() * std::mem::size_of::<f32>();
        let morph = AnimationValues::try_morph_weights(2, [morph_row], &limits)
            .expect("one bounded morph row should be admitted");

        assert_eq!(morph.key_count(), 1);
        assert_eq!(morph.retained_cell_capacity(), 2);
        assert!(morph.retained_allocation_bytes() < source_row_bytes);
        assert!(matches!(
            morph.view(),
            AnimationValuesView::MorphWeights { width: 2, .. }
        ));
        assert!(
            morph.retained_allocation_bytes()
                >= std::mem::size_of::<Vec<f32>>() + 2 * std::mem::size_of::<f32>()
        );

        let (_arena, provenance) = test_provenance_fixture();
        let mut authored_times = Vec::with_capacity(1_024);
        authored_times.push(0.0);
        let channel = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Linear,
            authored_times,
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            AnimationValues::try_translations([Vec3::ZERO], &limits)
                .expect("one translation sample should be admitted"),
            provenance,
        )
        .expect("one bounded channel should be admitted");
        assert!(channel.retained_allocation_bytes() >= std::mem::size_of::<f64>());

        let mut channels = Vec::with_capacity(1_024);
        channels.push(channel);
        let source_channel_headers = channels.capacity() * std::mem::size_of::<AnimationChannel>();
        let clip = clip(0.0, AnimationTimeDomain::Seconds, channels, provenance)
            .expect("one bounded clip channel should be admitted");
        assert!(clip.retained_allocation_bytes() < source_channel_headers);
        assert!(clip.retained_allocation_bytes() >= std::mem::size_of::<AnimationChannel>());
    }

    #[test]
    fn morph_sample_limits_charge_outer_headers_and_inner_capacity() {
        let limits = AnimationLimits::canonical()
            .try_with_max_sample_payload(2, 31)
            .expect("lower sample limits should be valid");
        assert!(matches!(
            AnimationValues::try_morph_weights(2, [[0.25, 0.75]], &limits),
            Err(AnimationBuildError::SamplePayloadLimitExceeded {
                dimension: AnimationLimitDimension::Bytes,
                requested,
                maximum: 31,
            }) if requested >= std::mem::size_of::<Vec<f32>>() + 8
        ));
    }

    #[test]
    fn clip_rejects_the_first_duplicate_target_in_source_order() {
        let (_arena, provenance) = test_provenance_fixture();
        let channels = [
            channel(
                AnimationTarget::NodeTranslation(node(5)),
                AnimationInterpolation::Linear,
                [0.0],
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Constant,
                translation_values(1),
                provenance,
            )
            .expect("first target should be valid"),
            channel(
                AnimationTarget::NodeTranslation(node(6)),
                AnimationInterpolation::Linear,
                [0.0],
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Constant,
                translation_values(1),
                provenance,
            )
            .expect("second target should be valid"),
            channel(
                AnimationTarget::NodeTranslation(node(6)),
                AnimationInterpolation::Linear,
                [0.0],
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Constant,
                translation_values(1),
                provenance,
            )
            .expect("first duplicate should be structurally valid"),
            channel(
                AnimationTarget::NodeTranslation(node(5)),
                AnimationInterpolation::Linear,
                [0.0],
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Constant,
                translation_values(1),
                provenance,
            )
            .expect("later duplicate should be structurally valid"),
        ];

        assert!(matches!(
            clip(1.0, AnimationTimeDomain::Seconds, channels, provenance),
            Err(AnimationBuildError::DuplicateTarget {
                target: AnimationTargetKind::NodeTranslation,
                first_channel_index: 1,
                duplicate_channel_index: 2,
            })
        ));
    }

    #[test]
    fn clip_uniqueness_distinguishes_owner_and_semantic() {
        let (_arena, provenance) = test_provenance_fixture();
        let channels = [
            channel(
                AnimationTarget::NodeTranslation(node(0)),
                AnimationInterpolation::Linear,
                [0.0],
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Constant,
                translation_values(1),
                provenance,
            )
            .expect("translation channel should be valid"),
            channel(
                AnimationTarget::NodeRotation(node(0)),
                AnimationInterpolation::Linear,
                [0.0],
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Constant,
                AnimationValues::try_rotations([Quat::IDENTITY], &AnimationLimits::canonical())
                    .expect("rotation values should be valid"),
                provenance,
            )
            .expect("rotation channel should be valid"),
            channel(
                AnimationTarget::NodeTranslation(node(1)),
                AnimationInterpolation::Linear,
                [0.0],
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Constant,
                translation_values(1),
                provenance,
            )
            .expect("different-owner channel should be valid"),
        ];

        let clip = clip(1.0, AnimationTimeDomain::Seconds, channels, provenance)
            .expect("different owner-semantic pairs must remain distinct");
        assert_eq!(clip.channels().len(), 3);
    }

    #[test]
    fn clip_retained_budget_is_independent_and_exact_boundary_is_inclusive() {
        let (_arena, provenance) = test_provenance_fixture();
        let admitted = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Linear,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            translation_values(1),
            provenance,
        )
        .expect("test channel should be valid");
        let exact = std::mem::size_of::<AnimationChannel>()
            .checked_add(admitted.retained_allocation_bytes())
            .expect("test retained size should fit");
        let exact_limits = AnimationLimits::canonical()
            .try_with_max_clip_retained_bytes(exact)
            .expect("exact clip limit should be canonical");
        let admitted = clip_with_limits(
            1.0,
            AnimationTimeDomain::Seconds,
            [admitted],
            provenance,
            &exact_limits,
        )
        .expect("the exact retained boundary should be inclusive");
        assert_eq!(admitted.retained_allocation_bytes(), exact);

        let rejected = channel(
            AnimationTarget::NodeTranslation(node(0)),
            AnimationInterpolation::Linear,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            translation_values(1),
            provenance,
        )
        .expect("test channel should be valid");
        let lower_limits = AnimationLimits::canonical()
            .try_with_max_clip_retained_bytes(exact - 1)
            .expect("one-below test limit should be canonical");
        assert!(matches!(
            clip_with_limits(
                1.0,
                AnimationTimeDomain::Seconds,
                [rejected],
                provenance,
                &lower_limits,
            ),
            Err(AnimationBuildError::CollectionLimitExceeded {
                resource: AnimationBuildResource::ClipPayload,
                dimension: AnimationLimitDimension::Bytes,
                requested,
                maximum,
            }) if requested == exact && maximum == exact - 1
        ));
    }

    #[test]
    fn clip_retained_budget_stops_before_polling_the_tail() {
        let (_arena, provenance) = test_provenance_fixture();
        let channel_header_bytes = std::mem::size_of::<AnimationChannel>();
        let keys_per_channel = channel_header_bytes
            .checked_div(std::mem::size_of::<Vec3>())
            .and_then(|whole_values| whole_values.checked_add(1))
            .expect("test channel payload size should fit");
        let keys_per_channel_u32 =
            u32::try_from(keys_per_channel).expect("test key count should fit u32");
        let channels = (0..3)
            .map(|index| {
                channel(
                    AnimationTarget::NodeTranslation(node(index)),
                    AnimationInterpolation::Linear,
                    (0..keys_per_channel_u32).map(f64::from),
                    AnimationExtrapolation::Constant,
                    AnimationExtrapolation::Constant,
                    translation_values(keys_per_channel),
                    provenance,
                )
                .expect("test channel should be valid")
            })
            .collect::<Vec<_>>();
        let one_channel_bytes = channels[0].retained_allocation_bytes();
        assert!(one_channel_bytes > channel_header_bytes);
        let planned_header_bytes = 4 * channel_header_bytes;
        let maximum = planned_header_bytes + one_channel_bytes;
        let limits = AnimationLimits::canonical()
            .try_with_max_clip_retained_bytes(maximum)
            .expect("test clip limit should be canonical");
        let polled = Rc::new(Cell::new(0));
        let observed = Rc::clone(&polled);
        let channels = channels.into_iter().inspect(move |_| {
            observed.set(observed.get() + 1);
        });

        assert!(matches!(
            clip_with_limits(
                1.0,
                AnimationTimeDomain::Seconds,
                channels,
                provenance,
                &limits,
            ),
            Err(AnimationBuildError::CollectionLimitExceeded {
                resource: AnimationBuildResource::ClipPayload,
                dimension: AnimationLimitDimension::Bytes,
                ..
            })
        ));
        assert_eq!(polled.get(), 2);
    }

    #[test]
    fn fixed_cubic_parts_share_one_preallocated_payload_budget() {
        let two_part_limits = AnimationLimits::canonical()
            .try_with_max_sample_payload(6, 2 * std::mem::size_of::<Vec3>())
            .expect("two-part sample limit should be canonical");
        assert!(matches!(
            AnimationValues::try_cubic_translations(
                [Vec3::ZERO],
                [Vec3::ZERO],
                NeverPolled::<Vec3>::one(),
                &two_part_limits,
            ),
            Err(AnimationBuildError::SamplePayloadLimitExceeded {
                dimension: AnimationLimitDimension::Cells,
                requested: 9,
                maximum: 6,
            })
        ));

        let exact_limits = AnimationLimits::canonical()
            .try_with_max_sample_payload(9, 3 * std::mem::size_of::<Vec3>())
            .expect("exact cubic sample limit should be canonical");
        let values = AnimationValues::try_cubic_translations(
            [Vec3::ZERO],
            [Vec3::ZERO],
            [Vec3::ZERO],
            &exact_limits,
        )
        .expect("the exact cubic payload boundary should be inclusive");
        assert_eq!(values.retained_cell_capacity(), 9);
        assert_eq!(
            values.retained_allocation_bytes(),
            3 * std::mem::size_of::<Vec3>()
        );
    }

    #[test]
    fn cubic_morph_parts_share_headers_and_rows_in_one_payload_budget() {
        let part_bytes = std::mem::size_of::<Vec<f32>>() + 2 * std::mem::size_of::<f32>();
        let two_part_limits = AnimationLimits::canonical()
            .try_with_max_sample_payload(4, 2 * part_bytes)
            .expect("two-part morph limit should be canonical");
        assert!(matches!(
            AnimationValues::try_cubic_morph_weights(
                2,
                [[0.0, 0.0]],
                [[0.5, 0.5]],
                NeverPolled::<[f32; 2]>::one(),
                &two_part_limits,
            ),
            Err(AnimationBuildError::SamplePayloadLimitExceeded {
                dimension: AnimationLimitDimension::Bytes,
                requested,
                maximum,
            }) if requested == 2 * part_bytes + std::mem::size_of::<Vec<f32>>()
                && maximum == 2 * part_bytes
        ));

        let exact_limits = AnimationLimits::canonical()
            .try_with_max_sample_payload(6, 3 * part_bytes)
            .expect("exact cubic morph limit should be canonical");
        let values = AnimationValues::try_cubic_morph_weights(
            2,
            [[0.0, 0.0]],
            [[0.5, 0.5]],
            [[0.0, 0.0]],
            &exact_limits,
        )
        .expect("the exact cubic morph boundary should be inclusive");
        assert_eq!(values.retained_cell_capacity(), 6);
        assert_eq!(values.retained_allocation_bytes(), 3 * part_bytes);
    }

    #[test]
    fn cubic_morph_discards_hostile_capacity_from_every_part() {
        let mut in_tangent = Vec::with_capacity(1_024);
        let mut value = Vec::with_capacity(2_048);
        let mut out_tangent = Vec::with_capacity(4_096);
        in_tangent.extend([0.0, 0.0]);
        value.extend([0.5, 0.5]);
        out_tangent.extend([0.0, 0.0]);
        let source_bytes = (in_tangent.capacity() + value.capacity() + out_tangent.capacity())
            * std::mem::size_of::<f32>();

        let values = AnimationValues::try_cubic_morph_weights(
            2,
            [in_tangent],
            [value],
            [out_tangent],
            &AnimationLimits::canonical(),
        )
        .expect("bounded cubic rows should discard source spare capacity");
        let expected = 3 * (std::mem::size_of::<Vec<f32>>() + 2 * std::mem::size_of::<f32>());
        assert_eq!(values.retained_allocation_bytes(), expected);
        assert!(values.retained_allocation_bytes() < source_bytes);
    }

    #[test]
    fn zero_width_cubic_morph_rejects_before_polling_any_part() {
        assert!(matches!(
            AnimationValues::try_cubic_morph_weights(
                0,
                NeverPolled::<[f32; 0]>::one(),
                NeverPolled::<[f32; 0]>::one(),
                NeverPolled::<[f32; 0]>::one(),
                &AnimationLimits::canonical(),
            ),
            Err(AnimationBuildError::InvalidMorphWidth)
        ));
    }
}
