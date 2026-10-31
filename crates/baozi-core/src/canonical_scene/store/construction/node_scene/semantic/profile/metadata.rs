use crate::canonical_scene::store::production::CanonicalProfileOwner;
use crate::canonical_scene::store::typed_storage::profile::{
    MetadataPayloads, ProfileState, StoredMetadataValue,
};
use crate::canonical_scene::store::typed_storage::{TableRange, TextRange};

use super::super::{NodeSceneSemanticResult, invariant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileMetadataArray<'scene> {
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
}

impl<'scene> NodeSceneProfileMetadataEntry<'scene> {
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
}

#[derive(Default)]
pub(super) struct MetadataCursor<'scene> {
    next_entry: usize,
    text: usize,
    bytes: usize,
    bools: usize,
    i64s: usize,
    u64s: usize,
    f64s: usize,
    f32s: usize,
    previous_owner: Option<CanonicalProfileOwner>,
    previous_key: Option<&'scene str>,
}

impl<'scene> MetadataCursor<'scene> {
    pub(super) fn next(
        &mut self,
        state: &'scene ProfileState,
        pools: &'scene MetadataPayloads,
    ) -> NodeSceneSemanticResult<Option<NodeSceneProfileMetadataEntry<'scene>>> {
        if self.next_entry == state.metadata_entries.len() {
            return Ok(None);
        }
        let entry = state
            .metadata_entries
            .get(self.next_entry)
            .ok_or_else(|| invariant("profile metadata cursor exceeds its typed rows"))?;
        let key = consume_text(&pools.text, &mut self.text, entry.key)?;
        if key.is_empty() {
            return Err(invariant("validated profile metadata key is empty"));
        }
        if self
            .previous_owner
            .is_some_and(|previous| entry.owner < previous)
            || self.previous_owner == Some(entry.owner)
                && self
                    .previous_key
                    .is_some_and(|previous| previous.as_bytes() >= key.as_bytes())
        {
            return Err(invariant(
                "validated profile metadata entries are not canonically ordered",
            ));
        }
        let value = self.read_value(pools, entry.value)?;
        self.next_entry += 1;
        self.previous_owner = Some(entry.owner);
        self.previous_key = Some(key);
        Ok(Some(NodeSceneProfileMetadataEntry {
            owner: entry.owner,
            key,
            value,
        }))
    }

    pub(super) fn finish(
        &self,
        state: &ProfileState,
        pools: &MetadataPayloads,
    ) -> NodeSceneSemanticResult<()> {
        if self.next_entry != state.metadata_entries.len()
            || self.text != pools.text.len()
            || self.bytes != pools.bytes.len()
            || self.bools != pools.bools.len()
            || self.i64s != pools.i64s.len()
            || self.u64s != pools.u64s.len()
            || self.f64s != pools.f64s.len()
            || self.f32s != pools.f32s.len()
        {
            return Err(invariant(
                "profile cursor ended before all metadata rows and pools were exhausted",
            ));
        }
        Ok(())
    }

    fn read_value(
        &mut self,
        pools: &'scene MetadataPayloads,
        value: StoredMetadataValue,
    ) -> NodeSceneSemanticResult<NodeSceneProfileMetadataValue<'scene>> {
        use NodeSceneProfileMetadataArray as Array;
        use NodeSceneProfileMetadataArrayKind as Kind;
        use NodeSceneProfileMetadataValue as Value;

        Ok(match value {
            StoredMetadataValue::Bool(range) => {
                match consume(&pools.bools, &mut self.bools, range)? {
                    [value] if *value <= 1 => Value::Bool(*value != 0),
                    _ => return Err(invariant("validated profile metadata bool is unreadable")),
                }
            }
            StoredMetadataValue::I64(range) => {
                Value::I64(one(consume(&pools.i64s, &mut self.i64s, range)?)?)
            }
            StoredMetadataValue::U64(range) => {
                Value::U64(one(consume(&pools.u64s, &mut self.u64s, range)?)?)
            }
            StoredMetadataValue::F64(range) => {
                Value::F64(one(consume(&pools.f64s, &mut self.f64s, range)?)?)
            }
            StoredMetadataValue::String(range) => {
                Value::String(consume_text(&pools.text, &mut self.text, range)?)
            }
            StoredMetadataValue::Bytes {
                media_type,
                payload,
            } => {
                let media_type = consume_text(&pools.text, &mut self.text, media_type)?;
                if media_type.is_empty() {
                    return Err(invariant(
                        "validated profile metadata bytes have no media type",
                    ));
                }
                Value::Bytes {
                    media_type,
                    payload: consume(&pools.bytes, &mut self.bytes, payload)?,
                }
            }
            StoredMetadataValue::Vec2(range) => Value::Vec2(self.fixed_f32(pools, range, 2)?),
            StoredMetadataValue::Vec3(range) => Value::Vec3(self.fixed_f32(pools, range, 3)?),
            StoredMetadataValue::Vec4(range) => Value::Vec4(self.fixed_f32(pools, range, 4)?),
            StoredMetadataValue::Color(range) => Value::Color(self.fixed_f32(pools, range, 4)?),
            StoredMetadataValue::Mat4(range) => Value::Mat4(self.fixed_f32(pools, range, 16)?),
            StoredMetadataValue::BoolArray(range) => Value::DenseArray {
                element_kind: Kind::Bool,
                logical_element_count: range.len(),
                values: Array::Bool(consume(&pools.bools, &mut self.bools, range)?),
            },
            StoredMetadataValue::I64Array(range) => Value::DenseArray {
                element_kind: Kind::I64,
                logical_element_count: range.len(),
                values: Array::I64(consume(&pools.i64s, &mut self.i64s, range)?),
            },
            StoredMetadataValue::U64Array(range) => Value::DenseArray {
                element_kind: Kind::U64,
                logical_element_count: range.len(),
                values: Array::U64(consume(&pools.u64s, &mut self.u64s, range)?),
            },
            StoredMetadataValue::F64Array(range) => Value::DenseArray {
                element_kind: Kind::F64,
                logical_element_count: range.len(),
                values: Array::F64(consume(&pools.f64s, &mut self.f64s, range)?),
            },
            StoredMetadataValue::Vec2Array(range) => self.f32_array(pools, range, Kind::Vec2, 2)?,
            StoredMetadataValue::Vec3Array(range) => self.f32_array(pools, range, Kind::Vec3, 3)?,
            StoredMetadataValue::Vec4Array(range) => self.f32_array(pools, range, Kind::Vec4, 4)?,
            StoredMetadataValue::ColorArray(range) => {
                self.f32_array(pools, range, Kind::Color, 4)?
            }
            StoredMetadataValue::Mat4Array(range) => {
                self.f32_array(pools, range, Kind::Mat4, 16)?
            }
        })
    }

    fn fixed_f32(
        &mut self,
        pools: &'scene MetadataPayloads,
        range: TableRange<f32>,
        width: usize,
    ) -> NodeSceneSemanticResult<&'scene [f32]> {
        if range.len() != width {
            return Err(invariant(
                "validated profile metadata vector width is inconsistent",
            ));
        }
        consume(&pools.f32s, &mut self.f32s, range)
    }

    fn f32_array(
        &mut self,
        pools: &'scene MetadataPayloads,
        range: TableRange<f32>,
        element_kind: NodeSceneProfileMetadataArrayKind,
        width: usize,
    ) -> NodeSceneSemanticResult<NodeSceneProfileMetadataValue<'scene>> {
        if !range.len().is_multiple_of(width) {
            return Err(invariant(
                "validated profile metadata array width is inconsistent",
            ));
        }
        Ok(NodeSceneProfileMetadataValue::DenseArray {
            element_kind,
            logical_element_count: range.len() / width,
            values: NodeSceneProfileMetadataArray::F32(consume(
                &pools.f32s,
                &mut self.f32s,
                range,
            )?),
        })
    }
}

fn consume<'scene, T>(
    pool: &'scene [T],
    cursor: &mut usize,
    range: TableRange<T>,
) -> NodeSceneSemanticResult<&'scene [T]> {
    if range.start() != *cursor {
        return Err(invariant(
            "validated profile metadata pool range is not contiguous",
        ));
    }
    let range = range
        .checked(pool.len())
        .ok_or_else(|| invariant("validated profile metadata pool range is unreadable"))?;
    let end = range.end;
    let values = pool
        .get(range)
        .ok_or_else(|| invariant("validated profile metadata pool range is unreadable"))?;
    *cursor = end;
    Ok(values)
}

fn consume_text<'scene>(
    pool: &'scene str,
    cursor: &mut usize,
    range: TextRange,
) -> NodeSceneSemanticResult<&'scene str> {
    if range.0.start() != *cursor {
        return Err(invariant(
            "validated profile metadata text range is not contiguous",
        ));
    }
    let value = range
        .get_str(pool)
        .ok_or_else(|| invariant("validated profile metadata text range is unreadable"))?;
    *cursor = range
        .0
        .start()
        .checked_add(range.0.len())
        .ok_or_else(|| invariant("validated profile metadata text range overflows"))?;
    Ok(value)
}

fn one<T: Copy>(values: &[T]) -> NodeSceneSemanticResult<T> {
    match values {
        [value] => Ok(*value),
        _ => Err(invariant(
            "validated profile metadata scalar width is inconsistent",
        )),
    }
}
