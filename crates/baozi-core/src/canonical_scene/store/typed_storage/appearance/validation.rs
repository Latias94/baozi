//! Central validation over actual appearance rows and compact byte/text pools.
//!
//! Every input-sized loop has one action per row or text byte. Numeric groups and swizzles are
//! bounded inline values; only a fixed per-material role set and scalar cursors are needed.

use crate::appearance::{
    MaterialTextureBindingSemanticsValidator, validate_material_semantics,
    validate_texture_binding_semantics, validate_texture_source_key,
};
use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
use crate::entity::{MAX_ENTITY_NAME_BYTES, validate_entity_name_str};
use crate::extension::{MAX_EXTENSION_MEDIA_TYPE_BYTES, validate_media_type};
use crate::image::MAX_ENCODED_IMAGE_BYTES;

use super::*;

#[cfg(test)]
mod tests;

/// Initialized shape in AppearanceStoragePlan row/payload order, independent of spare capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct AppearanceMeteredCounts {
    pub(in crate::canonical_scene::store) image_count: u32,
    pub(in crate::canonical_scene::store) sampler_count: u32,
    pub(in crate::canonical_scene::store) texture_count: u32,
    pub(in crate::canonical_scene::store) binding_count: u32,
    pub(in crate::canonical_scene::store) material_count: u32,
    pub(in crate::canonical_scene::store) image_byte_count: u64,
    pub(in crate::canonical_scene::store) text_byte_count: u64,
}

impl AppearanceMeteredCounts {
    /// Rows: images, samplers, textures, bindings, materials. Pools: image bytes, combined text.
    pub(in crate::canonical_scene::store) fn try_new(
        rows: [u64; 5],
        payloads: [u64; 2],
    ) -> Option<Self> {
        for count in rows.into_iter().chain(payloads) {
            usize::try_from(count).ok()?;
        }
        if (rows[0] == 0 && payloads[0] != 0)
            || (rows[0] != 0 && payloads[0] < rows[0])
            || (rows[0] == 0 && rows[3] == 0 && rows[4] == 0 && payloads[1] != 0)
        {
            return None;
        }
        Some(Self {
            image_count: u32::try_from(rows[0]).ok()?,
            sampler_count: u32::try_from(rows[1]).ok()?,
            texture_count: u32::try_from(rows[2]).ok()?,
            binding_count: u32::try_from(rows[3]).ok()?,
            material_count: u32::try_from(rows[4]).ok()?,
            image_byte_count: payloads[0],
            text_byte_count: payloads[1],
        })
    }

    /// O(1): observes only five row lengths and two pool lengths, without reading a descriptor.
    pub(in crate::canonical_scene::store) fn for_storage(
        storage: &AppearanceStorage,
    ) -> Option<Self> {
        Self::try_new(
            [
                storage.images.len() as u64,
                storage.samplers.len() as u64,
                storage.textures.len() as u64,
                storage.bindings.len() as u64,
                storage.materials.len() as u64,
            ],
            [storage.image_bytes.len() as u64, storage.text.len() as u64],
        )
    }

    /// Exact successful actions: shape, completion, every row, and every text byte.
    /// Encoded image contents are opaque: their bounded ranges are checked once per image.
    /// A binding's one action also checks its material-local role authority. No projection
    /// or scratch initialization is added to this work.
    pub(in crate::canonical_scene::store) fn validation_work(self) -> Option<u64> {
        [
            2,
            u64::from(self.image_count),
            u64::from(self.sampler_count),
            u64::from(self.texture_count),
            u64::from(self.binding_count),
            u64::from(self.material_count),
            self.text_byte_count,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }
}

impl AppearanceStorage {
    pub(in crate::canonical_scene::store) fn validate<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        counts: AppearanceMeteredCounts,
    ) -> Result<bool, C::Error> {
        if control.run_unit(|| AppearanceMeteredCounts::for_storage(self) == Some(counts))?
            != Some(true)
        {
            return Ok(false);
        }
        let mut image_cursor = 0;
        let mut text_cursor = 0;
        for image in &self.images {
            let Some(media) = run(control, || {
                (!image.payload.is_empty() && image.payload.len() <= MAX_ENCODED_IMAGE_BYTES)
                    .then_some(())?;
                consume(image.payload, &self.image_bytes, &mut image_cursor)?;
                let media = match image.media_type {
                    None => &[][..],
                    Some(range) => {
                        (range.0.len() <= MAX_EXTENSION_MEDIA_TYPE_BYTES).then_some(())?;
                        let bytes = consume(range.0, &self.text, &mut text_cursor)?;
                        validate_media_type(std::str::from_utf8(bytes).ok()?).ok()?;
                        bytes
                    }
                };
                Some(media)
            })?
            else {
                return Ok(false);
            };
            for &byte in media {
                if control.run_unit(|| byte)? != Some(byte) {
                    return Ok(false);
                }
            }
        }
        // The public sampler value is entirely closed enum state, with no numeric payload.
        for &sampler in &self.samplers {
            if control.run_unit(|| sampler)? != Some(sampler) {
                return Ok(false);
            }
        }
        for texture in &self.textures {
            if control.run_unit(|| {
                usize::try_from(texture.image.0)
                    .ok()
                    .and_then(|index| self.images.get(index))
                    .is_some()
                    && usize::try_from(texture.sampler.0)
                        .ok()
                        .and_then(|index| self.samplers.get(index))
                        .is_some()
            })? != Some(true)
            {
                return Ok(false);
            }
        }

        let mut binding_cursor = 0;
        let mut name_start = None;
        let mut name_cursor = 0;
        for material in &self.materials {
            let Some((bindings, name, mut authority)) = run(control, || {
                material.alpha.validate().ok()?;
                material.factors.validate().ok()?;
                validate_material_semantics(material.semantics()).ok()?;
                let bindings = consume(material.bindings, &self.bindings, &mut binding_cursor)?;
                let name = match material.name {
                    None => &[][..],
                    Some(range) => {
                        (range.0.len() <= MAX_ENTITY_NAME_BYTES).then_some(())?;
                        if name_start.is_none() {
                            name_start = Some(range.0.start());
                            name_cursor = range.0.start();
                        }
                        let bytes = consume(range.0, &self.text, &mut name_cursor)?;
                        validate_entity_name_str(std::str::from_utf8(bytes).ok()?).ok()?;
                        bytes
                    }
                };
                let authority = MaterialTextureBindingSemanticsValidator::new(bindings.len());
                Some((bindings, name, authority))
            })?
            else {
                return Ok(false);
            };
            for &byte in name {
                if control.run_unit(|| byte)? != Some(byte) {
                    return Ok(false);
                }
            }
            for binding in bindings {
                let Some(key) = run(control, || {
                    let texture = usize::try_from(binding.texture.0).ok()?;
                    self.textures.get(texture)?;
                    binding.transform.validate().ok()?;
                    binding.strength.validate().ok()?;
                    binding.channels.is_valid().then_some(())?;
                    let key = match binding.source_key {
                        None => &[][..],
                        Some(range) => {
                            (range.0.len() <= crate::appearance::MAX_TEXTURE_SOURCE_KEY_BYTES)
                                .then_some(())?;
                            let bytes = consume(range.0, &self.text, &mut text_cursor)?;
                            validate_texture_source_key(std::str::from_utf8(bytes).ok()?).ok()?;
                            bytes
                        }
                    };
                    // A source key carries descriptive source truth, never built-in authority.
                    let semantics = binding.semantics(None);
                    validate_texture_binding_semantics(semantics).ok()?;
                    authority.validate_next(semantics).ok()?;
                    Some(key)
                })?
                else {
                    return Ok(false);
                };
                for &byte in key {
                    if control.run_unit(|| byte)? != Some(byte) {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(control.run_unit(|| {
            image_cursor == self.image_bytes.len()
                && binding_cursor == self.bindings.len()
                && match name_start {
                    None => text_cursor == self.text.len(),
                    Some(start) => text_cursor == start && name_cursor == self.text.len(),
                }
        })? == Some(true))
    }
}

fn consume<'storage, T>(
    range: TableRange<T>,
    pool: &'storage [T],
    cursor: &mut usize,
) -> Option<&'storage [T]> {
    (range.start() == *cursor).then_some(())?;
    let values = range.get(pool)?;
    *cursor = cursor.checked_add(values.len())?;
    Some(values)
}

fn run<C: TypedStorageValidationControl, R>(
    control: &mut C,
    action: impl FnOnce() -> Option<R>,
) -> Result<Option<R>, C::Error> {
    Ok(control.run_unit(action)?.flatten())
}
