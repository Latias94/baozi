//! Independent capacity-qualified construction of appearance rows and payloads.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::{Presence, ValidatedAppearanceBatch};
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, append_name,
    capacity, finish, push,
};

use super::*;

#[cfg(test)]
mod tests;

const IMAGES: usize = 0;
const SAMPLERS: usize = 1;
const TEXTURES: usize = 2;
const BINDINGS: usize = 3;
const MATERIALS: usize = 4;
const IMAGE_BYTES: usize = 0;
const TEXT: usize = 1;

/// Only initialized row/pool counts persist. Capacity envelopes, work, and typed
/// footprint are rebuilt on demand rather than carried as redundant Scene facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct AppearanceStoragePlan {
    rows: [u32; 5],
    payloads: [usize; 2],
}

impl AppearanceStoragePlan {
    pub(in crate::canonical_scene::store) fn new<R>(
        batch: &ValidatedAppearanceBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        let plan = Self::for_shape(batch.row_counts(), batch.payload_counts())?;
        if plan.canonical_bytes()? != batch.canonical_bytes() {
            return Err(StorePlanError::InvalidBlockTable);
        }
        Ok(plan)
    }

    /// Rows: images, samplers, textures, bindings, materials. Pools: image bytes,
    /// then combined UTF-8 text. Counts describe initialized elements only.
    pub(in crate::canonical_scene::store) fn for_shape(
        rows: [u64; 5],
        payloads: [u64; 2],
    ) -> Result<Self, StorePlanError> {
        let mut checked_rows = [0; 5];
        for (output, count) in checked_rows.iter_mut().zip(rows) {
            *output = u32::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
        }
        let mut checked_payloads = [0; 2];
        for (output, count) in checked_payloads.iter_mut().zip(payloads) {
            *output = usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
        }
        let plan = Self {
            rows: checked_rows,
            payloads: checked_payloads,
        };
        plan.rows_capacities()?;
        plan.payload_capacities()?;
        Ok(plan)
    }

    pub(in crate::canonical_scene::store) fn for_storage(
        rows: &AppearanceRows,
        payloads: &AppearancePayloads,
    ) -> Result<Self, StorePlanError> {
        Self::for_shape(
            [
                rows.images.len() as u64,
                rows.samplers.len() as u64,
                rows.textures.len() as u64,
                rows.bindings.len() as u64,
                rows.materials.len() as u64,
            ],
            [
                payloads.image_bytes.len() as u64,
                payloads.text.len() as u64,
            ],
        )
    }

    pub(in crate::canonical_scene::store) fn rows_capacities(
        self,
    ) -> Result<[CapacityPlan; 5], StorePlanError> {
        let rows = self.row_counts();
        Ok([
            capacity::<StoredImage>(rows[0])?,
            capacity::<SamplerState>(rows[1])?,
            capacity::<StoredTexture>(rows[2])?,
            capacity::<StoredBinding>(rows[3])?,
            capacity::<StoredMaterial>(rows[4])?,
        ])
    }

    pub(in crate::canonical_scene::store) fn payload_capacities(
        self,
    ) -> Result<[CapacityPlan; 2], StorePlanError> {
        let payloads = self.payload_counts();
        Ok([capacity::<u8>(payloads[0])?, capacity::<u8>(payloads[1])?])
    }

    pub(in crate::canonical_scene::store) fn row_counts(self) -> [u64; 5] {
        self.rows.map(u64::from)
    }

    pub(in crate::canonical_scene::store) fn payload_counts(self) -> [u64; 2] {
        self.payloads.map(|count| count as u64)
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(self) -> usize {
        self.payloads[1]
    }

    /// One action per row, including constant-time payload/text range construction.
    /// This phase copies no bytes and performs no string work.
    pub(in crate::canonical_scene::store) fn rows_work(self) -> Result<u64, StorePlanError> {
        self.row_counts().into_iter().try_fold(0_u64, |sum, count| {
            sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
        })
    }

    /// One descriptor action per image, binding, and material, one action per image
    /// byte, and one string action per text byte, including absent/empty descriptors.
    pub(in crate::canonical_scene::store) fn payload_work(self) -> Result<u64, StorePlanError> {
        self.payload_counts()
            .into_iter()
            .chain([self.rows[IMAGES], self.rows[BINDINGS], self.rows[MATERIALS]].map(u64::from))
            .try_fold(0_u64, |sum, count| {
                sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
            })
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        self.rows_work()?
            .checked_add(self.payload_work()?)
            .ok_or(StorePlanError::SizeOverflow)
    }

    pub(in crate::canonical_scene::store) fn canonical_bytes(self) -> Result<u64, StorePlanError> {
        Self::canonical_bytes_for_shape(self.row_counts(), self.payload_counts())
    }

    /// Actual target-ABI inline root and both owner headers, plus initialized rows
    /// and pools. Retained-owner infrastructure and spare capacity are excluded.
    pub(in crate::canonical_scene::store) fn canonical_bytes_for_shape(
        rows: [u64; 5],
        payloads: [u64; 2],
    ) -> Result<u64, StorePlanError> {
        let parts = [
            (rows[0], std::mem::size_of::<StoredImage>()),
            (rows[1], std::mem::size_of::<SamplerState>()),
            (rows[2], std::mem::size_of::<StoredTexture>()),
            (rows[3], std::mem::size_of::<StoredBinding>()),
            (rows[4], std::mem::size_of::<StoredMaterial>()),
            (payloads[0], std::mem::size_of::<u8>()),
            (payloads[1], std::mem::size_of::<u8>()),
        ];
        let header = [
            std::mem::size_of::<AppearanceStorage>(),
            std::mem::size_of::<AppearanceRows>(),
            std::mem::size_of::<AppearancePayloads>(),
        ]
        .into_iter()
        .try_fold(0_u64, |sum, size| {
            sum.checked_add(u64::try_from(size).map_err(|_| StorePlanError::SizeOverflow)?)
                .ok_or(StorePlanError::SizeOverflow)
        })?;
        parts.into_iter().try_fold(header, |sum, (count, width)| {
            let width = u64::try_from(width).map_err(|_| StorePlanError::SizeOverflow)?;
            count
                .checked_mul(width)
                .and_then(|bytes| sum.checked_add(bytes))
                .ok_or(StorePlanError::SizeOverflow)
        })
    }

    /// Admit the row owner and its five capacities before entry. Payload/text ranges
    /// use only borrowed descriptors and lengths; no payload owner is allocated.
    pub(in crate::canonical_scene::store) fn build_rows<R, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedAppearanceBatch<'_, R>,
        mut control: C,
    ) -> BuildResult<AppearanceRows, C::Error> {
        if Self::new(batch).map_err(|_| TypedStorageBuildError::Invariant)? != self {
            return Err(TypedStorageBuildError::Invariant);
        }
        let capacities = self
            .rows_capacities()
            .map_err(|_| TypedStorageBuildError::Invariant)?;
        let mut storage = AppearanceRows {
            images: allocate(capacities[IMAGES])?,
            samplers: allocate(capacities[SAMPLERS])?,
            textures: allocate(capacities[TEXTURES])?,
            bindings: allocate(capacities[BINDINGS])?,
            materials: allocate(capacities[MATERIALS])?,
        };
        let mut image_cursor = 0;
        let mut text_cursor = 0;
        for image in batch.images() {
            let payload = take_range(&mut image_cursor, image.payload().len(), self.payloads[0])?;
            let media = match image.media_type() {
                Presence::Absent => Presence::Absent,
                Presence::Present(media) => Presence::Present(media.as_str()),
            };
            let media_type = take_text_range(&mut text_cursor, media, self.payloads[1])?;
            push(
                &mut storage.images,
                capacities[IMAGES],
                StoredImage {
                    payload,
                    media_type,
                },
                &mut control,
            )?;
        }
        for sampler in batch.samplers() {
            push(
                &mut storage.samplers,
                capacities[SAMPLERS],
                sampler.state(),
                &mut control,
            )?;
        }
        for texture in batch.textures() {
            push(
                &mut storage.textures,
                capacities[TEXTURES],
                StoredTexture {
                    image: StoredImageIndex(texture.image().index()),
                    sampler: StoredSamplerIndex(texture.sampler().index()),
                },
                &mut control,
            )?;
        }
        for binding in batch.bindings() {
            let semantics = binding.semantics();
            let key = semantics
                .source_key()
                .map_or(Presence::Absent, |key| Presence::Present(key.as_str()));
            let source_key = take_text_range(&mut text_cursor, key, self.payloads[1])?;
            push(
                &mut storage.bindings,
                capacities[BINDINGS],
                StoredBinding {
                    texture: StoredTextureIndex(binding.texture().index()),
                    role: semantics.role(),
                    uv_set: semantics.uv_set(),
                    channels: semantics.channel_mapping(),
                    color: semantics.color_interpretation(),
                    transform: semantics.transform(),
                    strength: semantics.strength(),
                    blend: semantics.blend(),
                    source_key,
                },
                &mut control,
            )?;
        }
        for material in batch.materials() {
            let bindings = material.bindings();
            let semantics = material.semantics();
            let name = take_text_range(&mut text_cursor, material.name(), self.payloads[1])?;
            push(
                &mut storage.materials,
                capacities[MATERIALS],
                StoredMaterial {
                    bindings: TableRange::new(
                        bindings.offset() as usize,
                        bindings.count() as usize,
                    ),
                    shading: semantics.shading_model(),
                    alpha: semantics.alpha_mode(),
                    double_sided: semantics.double_sided(),
                    factors: semantics.factors(),
                    name,
                },
                &mut control,
            )?;
        }
        finish(&storage.images, capacities[IMAGES])?;
        finish(&storage.samplers, capacities[SAMPLERS])?;
        finish(&storage.textures, capacities[TEXTURES])?;
        finish(&storage.bindings, capacities[BINDINGS])?;
        finish(&storage.materials, capacities[MATERIALS])?;
        if [image_cursor, text_cursor] != self.payloads {
            return Err(TypedStorageBuildError::Invariant);
        }
        Ok(storage)
    }

    /// Admit the payload owner and its two capacities before entry. Resolved images
    /// borrow the batch's same-session snapshots; no resource lookup is repeated.
    pub(in crate::canonical_scene::store) fn build_payloads<R, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedAppearanceBatch<'_, R>,
        mut control: C,
    ) -> BuildResult<AppearancePayloads, C::Error> {
        if Self::new(batch).map_err(|_| TypedStorageBuildError::Invariant)? != self {
            return Err(TypedStorageBuildError::Invariant);
        }
        let capacities = self
            .payload_capacities()
            .map_err(|_| TypedStorageBuildError::Invariant)?;
        let mut storage = AppearancePayloads {
            image_bytes: allocate(capacities[IMAGE_BYTES])?,
            text: allocate(capacities[TEXT])?,
        };
        for image in batch.images() {
            let (payload, media) = control
                .run_unit(|| {
                    let media = match image.media_type() {
                        Presence::Absent => Presence::Absent,
                        Presence::Present(media) => Presence::Present(media.as_str()),
                    };
                    (image.payload(), media)
                })
                .map_err(TypedStorageBuildError::Control)?;
            append(
                &mut storage.image_bytes,
                capacities[IMAGE_BYTES],
                payload,
                &mut control,
            )?;
            append_name(&mut storage.text, capacities[TEXT], media, &mut control)?;
        }
        for binding in batch.bindings() {
            let key = control
                .run_unit(|| {
                    binding
                        .semantics()
                        .source_key()
                        .map_or(Presence::Absent, |key| Presence::Present(key.as_str()))
                })
                .map_err(TypedStorageBuildError::Control)?;
            append_name(&mut storage.text, capacities[TEXT], key, &mut control)?;
        }
        for material in batch.materials() {
            let name = control
                .run_unit(|| material.name())
                .map_err(TypedStorageBuildError::Control)?;
            append_name(&mut storage.text, capacities[TEXT], name, &mut control)?;
        }
        finish(&storage.image_bytes, capacities[IMAGE_BYTES])?;
        finish(&storage.text, capacities[TEXT])?;
        Ok(storage)
    }
}

fn take_range<T, E>(
    cursor: &mut usize,
    length: usize,
    limit: usize,
) -> BuildResult<TableRange<T>, E> {
    let start = *cursor;
    let end = start
        .checked_add(length)
        .ok_or(TypedStorageBuildError::Invariant)?;
    if end > limit {
        return Err(TypedStorageBuildError::Invariant);
    }
    *cursor = end;
    Ok(TableRange::new(start, length))
}

fn take_text_range<E>(
    cursor: &mut usize,
    name: Presence<&str>,
    limit: usize,
) -> BuildResult<Option<TextRange>, E> {
    match name {
        Presence::Absent => Ok(None),
        Presence::Present(name) => Ok(Some(TextRange(take_range(cursor, name.len(), limit)?))),
    }
}
