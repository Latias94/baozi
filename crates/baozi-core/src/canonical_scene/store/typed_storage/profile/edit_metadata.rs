//! Concrete metadata compaction when an explicitly authorized edit removes one owner.
//!
//! Owner remapping belongs to the Scene edit. This plan only removes that owner's entries and
//! keeps the surviving payload ranges canonical. A plan that removes no entries shares the
//! source payload owner instead of calling `build_payloads` or remapping descriptor ranges.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::CanonicalProfileOwner;
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, capacity,
    count_work, finish,
};

use super::build::{allocate_text, finish_text, text};
use super::validation::ProfileMeteredCounts;
use super::{MetadataPayloads, ProfileStorage, StoredMetadataEntry, StoredMetadataValue};
use super::{TableRange, TextRange};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct MetadataEditPlan {
    deleted: Option<CanonicalProfileOwner>,
    input_entries: u32,
    output_entries: u32,
    removed_entries: u32,
    counts: MetadataCounts,
    capacities: [CapacityPlan; 7],
}

impl MetadataEditPlan {
    /// Scans descriptors once, with no payload-sized allocation or payload traversal.
    /// The caller supplies the already validated source used later by `build_payloads`.
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        profile: &ProfileStorage,
        deleted: Option<CanonicalProfileOwner>,
        control: &mut C,
    ) -> BuildResult<Self, C::Error> {
        Self::for_entries(
            &profile.state.as_ref().metadata_entries,
            profile.metadata.as_ref(),
            deleted,
            control,
        )
    }

    fn for_entries<C: TypedStorageBuildControl>(
        entries: &[StoredMetadataEntry],
        payloads: &MetadataPayloads,
        deleted: Option<CanonicalProfileOwner>,
        control: &mut C,
    ) -> BuildResult<Self, C::Error> {
        let input_entries =
            u32::try_from(entries.len()).map_err(|_| TypedStorageBuildError::Invariant)?;
        let mut source = MetadataEditCursor::new();
        let mut output = MetadataEditCursor::new();
        let mut output_entries = 0_u32;
        let mut removed_entries = 0_u32;
        for entry in entries {
            control
                .run_unit(|| {
                    let entry = *entry;
                    source.remap_inner(entry, true)?;
                    validate_text_ranges(entry, &payloads.text)?;
                    if deleted == Some(entry.owner) {
                        removed_entries = removed_entries.checked_add(1)?;
                    } else {
                        output.remap(entry)?;
                        output_entries = output_entries.checked_add(1)?;
                    }
                    Some(())
                })
                .map_err(TypedStorageBuildError::Control)?
                .ok_or(TypedStorageBuildError::Invariant)?;
        }
        if !source.counts.matches(payloads) {
            return Err(TypedStorageBuildError::Invariant);
        }
        let counts = output.counts;
        let capacities = [
            capacity::<u8>(u64::from(counts.text)),
            capacity::<u8>(u64::from(counts.bytes)),
            capacity::<u8>(u64::from(counts.bools)),
            capacity::<i64>(u64::from(counts.i64s)),
            capacity::<u64>(u64::from(counts.u64s)),
            capacity::<f64>(u64::from(counts.f64s)),
            capacity::<f32>(u64::from(counts.f32s)),
        ];
        let [text, bytes, bools, i64s, u64s, f64s, f32s] = capacities.map(|result| {
            result.map_err(|error| match error {
                StorePlanError::Capacity(cause) => TypedStorageBuildError::Capacity(cause),
                _ => TypedStorageBuildError::Invariant,
            })
        });
        Ok(Self {
            deleted,
            input_entries,
            output_entries,
            removed_entries,
            counts,
            capacities: [text?, bytes?, bools?, i64s?, u64s?, f64s?, f32s?],
        })
    }

    /// Text bytes, payload bytes, bools, i64s, u64s, f64s, then f32s.
    /// These capacities need admission only when `rewrites_payloads` is true.
    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 7] {
        self.capacities
    }

    /// Exact successful `build_payloads` work: every input descriptor, including removed rows,
    /// plus each surviving text byte, payload byte, and scalar. Planning is charged separately.
    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        count_work(self.capacities)?
            .checked_add(u64::from(self.input_entries))
            .ok_or(StorePlanError::SizeOverflow)
    }

    pub(in crate::canonical_scene::store) const fn text_bytes(self) -> usize {
        self.capacities[0].desired_capacity()
    }

    pub(in crate::canonical_scene::store) const fn rewrites_payloads(self) -> bool {
        self.removed_entries != 0
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn output_entries(self) -> u32 {
        self.output_entries
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn removed_entries(self) -> u32 {
        self.removed_entries
    }

    pub(in crate::canonical_scene::store) fn output_counts(
        self,
        mut source: ProfileMeteredCounts,
    ) -> ProfileMeteredCounts {
        source.metadata_entry_count = self.output_entries;
        source.metadata_text_bytes = self.counts.text;
        source.metadata_payload_bytes = self.counts.bytes;
        source.metadata_bool_count = self.counts.bools;
        source.metadata_i64_count = self.counts.i64s;
        source.metadata_u64_count = self.counts.u64s;
        source.metadata_f64_count = self.counts.f64s;
        source.metadata_f32_count = self.counts.f32s;
        source
    }

    /// Builds only payload pools, inside the caller's central retained-storage admission.
    /// Descriptor storage is built independently using `MetadataEditCursor` in source order.
    pub(in crate::canonical_scene::store) fn build_payloads<C: TypedStorageBuildControl>(
        self,
        profile: &ProfileStorage,
        control: &mut C,
    ) -> BuildResult<MetadataPayloads, C::Error> {
        self.build_entries(
            &profile.state.as_ref().metadata_entries,
            profile.metadata.as_ref(),
            control,
        )
    }

    fn build_entries<C: TypedStorageBuildControl>(
        self,
        entries: &[StoredMetadataEntry],
        source: &MetadataPayloads,
        control: &mut C,
    ) -> BuildResult<MetadataPayloads, C::Error> {
        if u32::try_from(entries.len()).ok() != Some(self.input_entries) {
            return Err(TypedStorageBuildError::Invariant);
        }
        let p = self.capacities;
        let mut output = MetadataPayloads {
            text: allocate_text(p[0])?,
            bytes: allocate(p[1])?,
            bools: allocate(p[2])?,
            i64s: allocate(p[3])?,
            u64s: allocate(p[4])?,
            f64s: allocate(p[5])?,
            f32s: allocate(p[6])?,
        };
        let mut kept = 0_u32;
        for entry in entries {
            let entry = control
                .run_unit(|| (self.deleted != Some(entry.owner)).then_some(*entry))
                .map_err(TypedStorageBuildError::Control)?;
            let Some(entry) = entry else {
                continue;
            };
            kept = kept
                .checked_add(1)
                .ok_or(TypedStorageBuildError::Invariant)?;
            copy_text(&mut output.text, p[0], entry.key, &source.text, control)?;
            copy_value(&mut output, p, entry.value, source, control)?;
        }
        if kept != self.output_entries {
            return Err(TypedStorageBuildError::Invariant);
        }
        finish_text(&output.text, p[0])?;
        finish(&output.bytes, p[1])?;
        finish(&output.bools, p[2])?;
        finish(&output.i64s, p[3])?;
        finish(&output.u64s, p[4])?;
        finish(&output.f64s, p[5])?;
        finish(&output.f32s, p[6])?;
        Ok(output)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MetadataCounts {
    text: u32,
    bytes: u32,
    bools: u32,
    i64s: u32,
    u64s: u32,
    f64s: u32,
    f32s: u32,
}

impl MetadataCounts {
    fn matches(self, pools: &MetadataPayloads) -> bool {
        u32::try_from(pools.text.len()).ok() == Some(self.text)
            && u32::try_from(pools.bytes.len()).ok() == Some(self.bytes)
            && u32::try_from(pools.bools.len()).ok() == Some(self.bools)
            && u32::try_from(pools.i64s.len()).ok() == Some(self.i64s)
            && u32::try_from(pools.u64s.len()).ok() == Some(self.u64s)
            && u32::try_from(pools.f64s.len()).ok() == Some(self.f64s)
            && u32::try_from(pools.f32s.len()).ok() == Some(self.f32s)
    }
}

/// Compact descriptor ranges only; the Scene edit maps each returned owner separately.
#[derive(Clone, Copy, Default)]
pub(in crate::canonical_scene::store) struct MetadataEditCursor {
    counts: MetadataCounts,
}

impl MetadataEditCursor {
    pub(in crate::canonical_scene::store) fn new() -> Self {
        Self::default()
    }

    /// Call exactly once per kept entry, in source order, only for a payload-rewriting plan.
    /// Failure leaves the cursor unchanged. Empty arrays retain their original variant.
    /// The caller charges its descriptor-building action; this cursor never reads payloads.
    pub(in crate::canonical_scene::store) fn remap(
        &mut self,
        entry: StoredMetadataEntry,
    ) -> Option<StoredMetadataEntry> {
        self.remap_inner(entry, false)
    }

    fn remap_inner(
        &mut self,
        entry: StoredMetadataEntry,
        canonical_source: bool,
    ) -> Option<StoredMetadataEntry> {
        let mut c = self.counts;
        let key = TextRange(take(&mut c.text, entry.key.0, 1, true, canonical_source)?);
        use StoredMetadataValue as Value;
        let value = match entry.value {
            Value::Bool(r) => Value::Bool(take(&mut c.bools, r, 1, false, canonical_source)?),
            Value::I64(r) => Value::I64(take(&mut c.i64s, r, 1, false, canonical_source)?),
            Value::U64(r) => Value::U64(take(&mut c.u64s, r, 1, false, canonical_source)?),
            Value::F64(r) => Value::F64(take(&mut c.f64s, r, 1, false, canonical_source)?),
            Value::String(r) => Value::String(TextRange(take(
                &mut c.text,
                r.0,
                1,
                true,
                canonical_source,
            )?)),
            Value::Bytes {
                media_type,
                payload,
            } => Value::Bytes {
                media_type: TextRange(take(&mut c.text, media_type.0, 1, true, canonical_source)?),
                payload: take(&mut c.bytes, payload, 1, true, canonical_source)?,
            },
            Value::Vec2(r) => Value::Vec2(take(&mut c.f32s, r, 2, false, canonical_source)?),
            Value::Vec3(r) => Value::Vec3(take(&mut c.f32s, r, 3, false, canonical_source)?),
            Value::Vec4(r) => Value::Vec4(take(&mut c.f32s, r, 4, false, canonical_source)?),
            Value::Color(r) => Value::Color(take(&mut c.f32s, r, 4, false, canonical_source)?),
            Value::Mat4(r) => Value::Mat4(take(&mut c.f32s, r, 16, false, canonical_source)?),
            Value::BoolArray(r) => {
                Value::BoolArray(take(&mut c.bools, r, 1, true, canonical_source)?)
            }
            Value::I64Array(r) => Value::I64Array(take(&mut c.i64s, r, 1, true, canonical_source)?),
            Value::U64Array(r) => Value::U64Array(take(&mut c.u64s, r, 1, true, canonical_source)?),
            Value::F64Array(r) => Value::F64Array(take(&mut c.f64s, r, 1, true, canonical_source)?),
            Value::Vec2Array(r) => {
                Value::Vec2Array(take(&mut c.f32s, r, 2, true, canonical_source)?)
            }
            Value::Vec3Array(r) => {
                Value::Vec3Array(take(&mut c.f32s, r, 3, true, canonical_source)?)
            }
            Value::Vec4Array(r) => {
                Value::Vec4Array(take(&mut c.f32s, r, 4, true, canonical_source)?)
            }
            Value::ColorArray(r) => {
                Value::ColorArray(take(&mut c.f32s, r, 4, true, canonical_source)?)
            }
            Value::Mat4Array(r) => {
                Value::Mat4Array(take(&mut c.f32s, r, 16, true, canonical_source)?)
            }
        };
        self.counts = c;
        Some(StoredMetadataEntry {
            owner: entry.owner,
            key,
            value,
        })
    }
}

fn take<T>(
    cursor: &mut u32,
    source: TableRange<T>,
    width: usize,
    array: bool,
    canonical_source: bool,
) -> Option<TableRange<T>> {
    let valid_length = if array {
        source.len().is_multiple_of(width)
    } else {
        source.len() == width
    };
    if !valid_length {
        return None;
    }
    let source_start = u32::try_from(source.start()).ok()?;
    let length = u32::try_from(source.len()).ok()?;
    source_start.checked_add(length)?;
    if canonical_source && source_start != *cursor {
        return None;
    }
    let start = usize::try_from(*cursor).ok()?;
    *cursor = cursor.checked_add(length)?;
    Some(TableRange::new(start, source.len()))
}

fn validate_text_ranges(entry: StoredMetadataEntry, text: &str) -> Option<()> {
    entry.key.get_str(text)?;
    match entry.value {
        StoredMetadataValue::String(value) => {
            value.get_str(text)?;
        }
        StoredMetadataValue::Bytes { media_type, .. } => {
            media_type.get_str(text)?;
        }
        _ => {}
    }
    Some(())
}

fn copy_text<C: TypedStorageBuildControl>(
    output: &mut String,
    plan: CapacityPlan,
    range: TextRange,
    source: &str,
    control: &mut C,
) -> BuildResult<(), C::Error> {
    let value = range
        .get_str(source)
        .ok_or(TypedStorageBuildError::Invariant)?;
    text(output, plan, value, control)?;
    Ok(())
}

fn copy_range<T: Copy, C: TypedStorageBuildControl>(
    output: &mut Vec<T>,
    plan: CapacityPlan,
    range: TableRange<T>,
    source: &[T],
    control: &mut C,
) -> BuildResult<(), C::Error> {
    let values = range.get(source).ok_or(TypedStorageBuildError::Invariant)?;
    append(output, plan, values, control)?;
    Ok(())
}

fn copy_value<C: TypedStorageBuildControl>(
    output: &mut MetadataPayloads,
    p: [CapacityPlan; 7],
    value: StoredMetadataValue,
    source: &MetadataPayloads,
    control: &mut C,
) -> BuildResult<(), C::Error> {
    use StoredMetadataValue as Value;
    match value {
        Value::Bool(r) | Value::BoolArray(r) => {
            copy_range(&mut output.bools, p[2], r, &source.bools, control)?;
        }
        Value::I64(r) | Value::I64Array(r) => {
            copy_range(&mut output.i64s, p[3], r, &source.i64s, control)?;
        }
        Value::U64(r) | Value::U64Array(r) => {
            copy_range(&mut output.u64s, p[4], r, &source.u64s, control)?;
        }
        Value::F64(r) | Value::F64Array(r) => {
            copy_range(&mut output.f64s, p[5], r, &source.f64s, control)?;
        }
        Value::String(r) => copy_text(&mut output.text, p[0], r, &source.text, control)?,
        Value::Bytes {
            media_type,
            payload,
        } => {
            copy_text(&mut output.text, p[0], media_type, &source.text, control)?;
            copy_range(&mut output.bytes, p[1], payload, &source.bytes, control)?;
        }
        Value::Vec2(r)
        | Value::Vec3(r)
        | Value::Vec4(r)
        | Value::Color(r)
        | Value::Mat4(r)
        | Value::Vec2Array(r)
        | Value::Vec3Array(r)
        | Value::Vec4Array(r)
        | Value::ColorArray(r)
        | Value::Mat4Array(r) => {
            copy_range(&mut output.f32s, p[6], r, &source.f32s, control)?;
        }
    }
    Ok(())
}
