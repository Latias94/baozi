//! Capacity-qualified construction inside the existing retained-storage admission.

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
const IMAGE_BYTES: usize = 5;
const TEXT: usize = 6;

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
        plan.capacities()?;
        Ok(plan)
    }

    pub(in crate::canonical_scene::store) fn capacities(
        self,
    ) -> Result<[CapacityPlan; 7], StorePlanError> {
        let rows = self.row_counts();
        let payloads = self.payload_counts();
        Ok([
            capacity::<StoredImage>(rows[0])?,
            capacity::<SamplerState>(rows[1])?,
            capacity::<StoredTexture>(rows[2])?,
            capacity::<StoredBinding>(rows[3])?,
            capacity::<StoredMaterial>(rows[4])?,
            capacity::<u8>(payloads[0])?,
            capacity::<u8>(payloads[1])?,
        ])
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

    /// One action per initialized row or image byte; one string action per text byte.
    /// Numeric fields are bounded inline row values, with no scalar-copy work/pool.
    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        self.row_counts()
            .into_iter()
            .chain(self.payload_counts())
            .try_fold(0_u64, |sum, count| {
                sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
            })
    }

    pub(in crate::canonical_scene::store) fn canonical_bytes(self) -> Result<u64, StorePlanError> {
        Self::canonical_bytes_for_shape(self.row_counts(), self.payload_counts())
    }

    /// Actual target-ABI owner header plus used rows and byte pools. Retained-owner
    /// infrastructure and spare capacity are excluded; inline factor groups count once.
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
        let header = u64::try_from(std::mem::size_of::<AppearanceStorage>())
            .map_err(|_| StorePlanError::SizeOverflow)?;
        parts.into_iter().try_fold(header, |sum, (count, width)| {
            let width = u64::try_from(width).map_err(|_| StorePlanError::SizeOverflow)?;
            count
                .checked_mul(width)
                .and_then(|bytes| sum.checked_add(bytes))
                .ok_or(StorePlanError::SizeOverflow)
        })
    }

    /// Admit the owner and all seven capacities before entry. Resolved image payloads
    /// borrow the batch's same-session snapshots; no resource lookup is repeated here.
    pub(in crate::canonical_scene::store) fn build<R, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedAppearanceBatch<'_, R>,
        mut control: C,
    ) -> BuildResult<AppearanceStorage, C::Error> {
        if Self::new(batch).map_err(|_| TypedStorageBuildError::Invariant)? != self {
            return Err(TypedStorageBuildError::Invariant);
        }
        let capacities = self
            .capacities()
            .map_err(|_| TypedStorageBuildError::Invariant)?;
        let mut storage = AppearanceStorage {
            images: allocate(capacities[IMAGES])?,
            samplers: allocate(capacities[SAMPLERS])?,
            textures: allocate(capacities[TEXTURES])?,
            bindings: allocate(capacities[BINDINGS])?,
            materials: allocate(capacities[MATERIALS])?,
            image_bytes: allocate(capacities[IMAGE_BYTES])?,
            text: allocate(capacities[TEXT])?,
        };
        for image in batch.images() {
            let payload = append(
                &mut storage.image_bytes,
                capacities[IMAGE_BYTES],
                image.payload(),
                &mut control,
            )?;
            let media = match image.media_type() {
                Presence::Absent => Presence::Absent,
                Presence::Present(media) => Presence::Present(media.as_str()),
            };
            let media_type = append_name(&mut storage.text, capacities[TEXT], media, &mut control)?;
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
            let source_key = append_name(&mut storage.text, capacities[TEXT], key, &mut control)?;
            push(
                &mut storage.bindings,
                capacities[BINDINGS],
                StoredBinding {
                    texture: StoredTextureIndex(binding.texture().index()),
                    role: semantics.role(),
                    coordinate_source: semantics.coordinate_source(),
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
            let name = append_name(
                &mut storage.text,
                capacities[TEXT],
                material.name(),
                &mut control,
            )?;
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
        finish(&storage.image_bytes, capacities[IMAGE_BYTES])?;
        finish(&storage.text, capacities[TEXT])?;
        Ok(storage)
    }
}
