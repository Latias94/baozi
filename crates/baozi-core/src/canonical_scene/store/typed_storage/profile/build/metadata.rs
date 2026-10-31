//! Immutable metadata pools and their separately owned typed descriptors.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::{CanonicalProfileOwner, ValidatedProfileBatch};
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, capacity,
    count_work, finish, push,
};
use crate::extension::{ExtensionArrayRef, ExtensionValueRef};

use super::super::{
    MetadataPayloads, StoredMetadataEntry, StoredMetadataValue, TableRange, TextRange,
};
use super::{allocate_text, finish_text, text};

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct MetadataStoragePlan {
    capacities: [CapacityPlan; 7],
    entries: usize,
}

impl MetadataStoragePlan {
    pub(in crate::canonical_scene::store) fn for_batch<R>(
        batch: &ValidatedProfileBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        let shape = batch.retention_shape();
        Ok(Self {
            capacities: [
                capacity::<u8>(u64::from(shape.metadata_text_bytes()))?,
                capacity::<u8>(u64::from(shape.metadata_payload_bytes()))?,
                capacity::<u8>(u64::from(shape.metadata_bool_count()))?,
                capacity::<i64>(u64::from(shape.metadata_i64_count()))?,
                capacity::<u64>(u64::from(shape.metadata_u64_count()))?,
                capacity::<f64>(u64::from(shape.metadata_f64_count()))?,
                capacity::<f32>(u64::from(shape.metadata_f32_count()))?,
            ],
            entries: batch.metadata_entry_count() as usize,
        })
    }

    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 7] {
        self.capacities
    }

    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        count_work(self.capacities)?
            .checked_add(self.entries as u64)
            .ok_or(StorePlanError::SizeOverflow)
    }

    pub(in crate::canonical_scene::store) const fn text_bytes(self) -> usize {
        self.capacities[0].desired_capacity()
    }

    pub(in crate::canonical_scene::store) fn build<R, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedProfileBatch<'_, R>,
        control: &mut C,
    ) -> BuildResult<MetadataPayloads, C::Error> {
        let p = self.capacities;
        let mut pools = MetadataPayloads {
            text: allocate_text(p[0])?,
            bytes: allocate(p[1])?,
            bools: allocate(p[2])?,
            i64s: allocate(p[3])?,
            u64s: allocate(p[4])?,
            f64s: allocate(p[5])?,
            f32s: allocate(p[6])?,
        };
        for index in 0..self.entries {
            let entry = control
                .run_unit(|| batch.metadata_entry(index))
                .map_err(TypedStorageBuildError::Control)?
                .ok_or(TypedStorageBuildError::Invariant)?;
            text(&mut pools.text, p[0], entry.key().as_str(), control)?;
            copy_value(&mut pools, p, entry.value(), control)?;
        }
        finish_text(&pools.text, p[0])?;
        finish(&pools.bytes, p[1])?;
        finish(&pools.bools, p[2])?;
        finish(&pools.i64s, p[3])?;
        finish(&pools.u64s, p[4])?;
        finish(&pools.f64s, p[5])?;
        finish(&pools.f32s, p[6])?;
        Ok(pools)
    }
}

fn copy_value<C: TypedStorageBuildControl>(
    pools: &mut MetadataPayloads,
    p: [CapacityPlan; 7],
    value: ExtensionValueRef<'_>,
    control: &mut C,
) -> BuildResult<(), C::Error> {
    match value {
        ExtensionValueRef::Bool(value) => push(&mut pools.bools, p[2], u8::from(value), control)?,
        ExtensionValueRef::I64(value) => push(&mut pools.i64s, p[3], value, control)?,
        ExtensionValueRef::U64(value) => push(&mut pools.u64s, p[4], value, control)?,
        ExtensionValueRef::F64(value) => push(&mut pools.f64s, p[5], value, control)?,
        ExtensionValueRef::String(value) => {
            text(&mut pools.text, p[0], value, control)?;
        }
        ExtensionValueRef::Bytes {
            media_type,
            payload,
        } => {
            text(&mut pools.text, p[0], media_type.as_str(), control)?;
            append(&mut pools.bytes, p[1], payload, control)?;
        }
        ExtensionValueRef::Vec2(v) => f32_values(pools, p[6], [v.x, v.y], control)?,
        ExtensionValueRef::Vec3(v) => f32_values(pools, p[6], [v.x, v.y, v.z], control)?,
        ExtensionValueRef::Vec4(v) => f32_values(pools, p[6], [v.x, v.y, v.z, v.w], control)?,
        ExtensionValueRef::Color(v) => f32_values(pools, p[6], [v.r, v.g, v.b, v.a], control)?,
        ExtensionValueRef::Mat4(v) => {
            f32_values(pools, p[6], v.cols.into_iter().flatten(), control)?
        }
        ExtensionValueRef::DenseArray(array) => match array {
            ExtensionArrayRef::Bool(values) => {
                for &value in values {
                    push(&mut pools.bools, p[2], u8::from(value), control)?;
                }
            }
            ExtensionArrayRef::I64(values) => {
                append(&mut pools.i64s, p[3], values, control)?;
            }
            ExtensionArrayRef::U64(values) => {
                append(&mut pools.u64s, p[4], values, control)?;
            }
            ExtensionArrayRef::F64(values) => {
                append(&mut pools.f64s, p[5], values, control)?;
            }
            ExtensionArrayRef::Vec2(values) => {
                f32_values(pools, p[6], values.iter().flat_map(|v| [v.x, v.y]), control)?;
            }
            ExtensionArrayRef::Vec3(values) => {
                f32_values(
                    pools,
                    p[6],
                    values.iter().flat_map(|v| [v.x, v.y, v.z]),
                    control,
                )?;
            }
            ExtensionArrayRef::Vec4(values) => {
                f32_values(
                    pools,
                    p[6],
                    values.iter().flat_map(|v| [v.x, v.y, v.z, v.w]),
                    control,
                )?;
            }
            ExtensionArrayRef::Color(values) => {
                f32_values(
                    pools,
                    p[6],
                    values.iter().flat_map(|v| [v.r, v.g, v.b, v.a]),
                    control,
                )?;
            }
            ExtensionArrayRef::Mat4(values) => {
                f32_values(
                    pools,
                    p[6],
                    values.iter().flat_map(|v| v.cols.into_iter().flatten()),
                    control,
                )?;
            }
        },
    }
    Ok(())
}

fn f32_values<C: TypedStorageBuildControl>(
    pools: &mut MetadataPayloads,
    plan: CapacityPlan,
    values: impl IntoIterator<Item = f32>,
    control: &mut C,
) -> BuildResult<(), C::Error> {
    for value in values {
        push(&mut pools.f32s, plan, value, control)?;
    }
    Ok(())
}

#[derive(Default)]
pub(in crate::canonical_scene::store) struct MetadataDescriptorCursor {
    text: usize,
    bytes: usize,
    bools: usize,
    i64s: usize,
    u64s: usize,
    f64s: usize,
    f32s: usize,
}

impl MetadataDescriptorCursor {
    /// Computes references only. The separately retained pools are never copied here.
    pub(in crate::canonical_scene::store) fn entry(
        &mut self,
        owner: CanonicalProfileOwner,
        key: &str,
        value: ExtensionValueRef<'_>,
    ) -> Option<StoredMetadataEntry> {
        let key = TextRange(take(&mut self.text, key.len())?);
        let value = match value {
            ExtensionValueRef::Bool(_) => StoredMetadataValue::Bool(take(&mut self.bools, 1)?),
            ExtensionValueRef::I64(_) => StoredMetadataValue::I64(take(&mut self.i64s, 1)?),
            ExtensionValueRef::U64(_) => StoredMetadataValue::U64(take(&mut self.u64s, 1)?),
            ExtensionValueRef::F64(_) => StoredMetadataValue::F64(take(&mut self.f64s, 1)?),
            ExtensionValueRef::String(value) => {
                StoredMetadataValue::String(TextRange(take(&mut self.text, value.len())?))
            }
            ExtensionValueRef::Bytes {
                media_type,
                payload,
            } => StoredMetadataValue::Bytes {
                media_type: TextRange(take(&mut self.text, media_type.as_str().len())?),
                payload: take(&mut self.bytes, payload.len())?,
            },
            ExtensionValueRef::Vec2(_) => StoredMetadataValue::Vec2(take(&mut self.f32s, 2)?),
            ExtensionValueRef::Vec3(_) => StoredMetadataValue::Vec3(take(&mut self.f32s, 3)?),
            ExtensionValueRef::Vec4(_) => StoredMetadataValue::Vec4(take(&mut self.f32s, 4)?),
            ExtensionValueRef::Color(_) => StoredMetadataValue::Color(take(&mut self.f32s, 4)?),
            ExtensionValueRef::Mat4(_) => StoredMetadataValue::Mat4(take(&mut self.f32s, 16)?),
            ExtensionValueRef::DenseArray(array) => match array {
                ExtensionArrayRef::Bool(v) => {
                    StoredMetadataValue::BoolArray(take(&mut self.bools, v.len())?)
                }
                ExtensionArrayRef::I64(v) => {
                    StoredMetadataValue::I64Array(take(&mut self.i64s, v.len())?)
                }
                ExtensionArrayRef::U64(v) => {
                    StoredMetadataValue::U64Array(take(&mut self.u64s, v.len())?)
                }
                ExtensionArrayRef::F64(v) => {
                    StoredMetadataValue::F64Array(take(&mut self.f64s, v.len())?)
                }
                ExtensionArrayRef::Vec2(v) => {
                    StoredMetadataValue::Vec2Array(take(&mut self.f32s, v.len().checked_mul(2)?)?)
                }
                ExtensionArrayRef::Vec3(v) => {
                    StoredMetadataValue::Vec3Array(take(&mut self.f32s, v.len().checked_mul(3)?)?)
                }
                ExtensionArrayRef::Vec4(v) => {
                    StoredMetadataValue::Vec4Array(take(&mut self.f32s, v.len().checked_mul(4)?)?)
                }
                ExtensionArrayRef::Color(v) => {
                    StoredMetadataValue::ColorArray(take(&mut self.f32s, v.len().checked_mul(4)?)?)
                }
                ExtensionArrayRef::Mat4(v) => {
                    StoredMetadataValue::Mat4Array(take(&mut self.f32s, v.len().checked_mul(16)?)?)
                }
            },
        };
        Some(StoredMetadataEntry { owner, key, value })
    }

    pub(in crate::canonical_scene::store) fn matches(&self, pools: &MetadataPayloads) -> bool {
        self.text == pools.text.len()
            && self.bytes == pools.bytes.len()
            && self.bools == pools.bools.len()
            && self.i64s == pools.i64s.len()
            && self.u64s == pools.u64s.len()
            && self.f64s == pools.f64s.len()
            && self.f32s == pools.f32s.len()
    }
}

fn take<T>(cursor: &mut usize, length: usize) -> Option<TableRange<T>> {
    let start = *cursor;
    *cursor = start.checked_add(length)?;
    Some(TableRange::new(start, length))
}
