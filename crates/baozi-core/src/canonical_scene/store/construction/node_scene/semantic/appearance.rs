//! Borrowed semantic reads over the proof-qualified typed appearance owner.
//!
//! Random reads select one row and its checked ranges. Ordered traversal closes the five tables
//! and the combined image/media/source-key/name pools without record codecs or row resets.

use crate::appearance::{
    AlphaMode, ColorInterpretation, MaterialFactors, ShadingModel, TextureBlendOperation,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength, TextureTransform,
    validate_material_semantics, validate_texture_binding_semantics, validate_texture_source_key,
};
use crate::canonical_scene::store::typed_storage::appearance::{
    AppearanceStorage, StoredBinding, StoredMaterial,
};
use crate::canonical_scene::store::typed_storage::{TableRange, TextRange};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::entity::{MAX_ENTITY_NAME_BYTES, validate_entity_name_str};
use crate::extension::{MAX_EXTENSION_MEDIA_TYPE_BYTES, validate_media_type};
use crate::image::{MAX_ENCODED_IMAGE_BYTES, SamplerState};

use super::{NodeSceneSemanticResult, invariant};

mod cursor;
mod payload;
mod read;
#[cfg(test)]
mod tests;

pub(in crate::canonical_scene::store) use cursor::NodeSceneAppearanceCursor;
pub(in crate::canonical_scene::store) use payload::{
    NodeSceneAppearanceBindingScalars, NodeSceneAppearanceMaterialFactors,
};

// These widths describe the existing snapshot transcript, not duplicate retained pools.
const MATERIAL_FACTOR_SCALARS: usize = 39;
const BINDING_SEMANTIC_SCALARS: usize = 6;

#[cfg(test)]
std::thread_local! {
    static ROWS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn take_row_read_count_for_test() -> usize {
    ROWS_READ.with(|count| count.replace(0))
}

fn record_row_read() {
    #[cfg(test)]
    ROWS_READ.with(|count| count.set(count.get() + 1));
}

/// The actual immutable owner; no proof or independently resolved payloads are retained.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceTables<'scene> {
    storage: &'scene AppearanceStorage,
}

impl<'scene> NodeSceneAppearanceTables<'scene> {
    pub(in crate::canonical_scene::store) fn try_new(
        storage: &'scene AppearanceStorage,
        proof: &NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        if [
            storage.images.len(),
            storage.samplers.len(),
            storage.textures.len(),
            storage.bindings.len(),
            storage.materials.len(),
        ] != [
            proof.image_count(),
            proof.sampler_count(),
            proof.texture_count(),
            proof.material_binding_count(),
            proof.material_count(),
        ] {
            return Err(invariant(
                "typed appearance tables disagree with central validation proof",
            ));
        }
        Ok(Self { storage })
    }

    pub(super) fn into_cursor(self) -> NodeSceneAppearanceCursor<'scene> {
        NodeSceneAppearanceCursor::new(self)
    }
}

fn borrow_range<T>(range: TableRange<T>, pool: &[T]) -> NodeSceneSemanticResult<&[T]> {
    range
        .get(pool)
        .ok_or_else(|| invariant("validated appearance payload range is unavailable"))
}

fn borrow_text(range: TextRange, text: &[u8], maximum: usize) -> NodeSceneSemanticResult<&str> {
    if range.0.len() > maximum {
        return Err(invariant(
            "validated appearance text exceeds its bounded domain",
        ));
    }
    range
        .get(text)
        .ok_or_else(|| invariant("validated appearance text range is unavailable or not UTF-8"))
}

fn transcript_offset(row: usize, width: usize) -> NodeSceneSemanticResult<usize> {
    row.checked_mul(width)
        .ok_or_else(|| invariant("appearance snapshot scalar offset overflows"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceImage<'scene> {
    source_index: usize,
    encoded_payload: &'scene [u8],
    media_type: Option<&'scene str>,
}

impl<'scene> NodeSceneAppearanceImage<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn encoded_payload(self) -> &'scene [u8] {
        self.encoded_payload
    }

    pub(in crate::canonical_scene::store) const fn media_type(self) -> Option<&'scene str> {
        self.media_type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceSampler {
    source_index: usize,
    state: SamplerState,
}

impl NodeSceneAppearanceSampler {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn state(self) -> SamplerState {
        self.state
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceTexture {
    source_index: usize,
    image: usize,
    sampler: usize,
}

impl NodeSceneAppearanceTexture {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn image(self) -> usize {
        self.image
    }

    pub(in crate::canonical_scene::store) const fn sampler(self) -> usize {
        self.sampler
    }
}

/// The row and its actual name range remain borrowed; inline factors are never cloned.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceMaterial<'scene> {
    source_index: usize,
    stored: &'scene StoredMaterial,
    name: Option<&'scene str>,
    factor_offset: usize,
}

impl<'scene> NodeSceneAppearanceMaterial<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn shading_model(&self) -> &'scene ShadingModel {
        &self.stored.shading
    }

    pub(in crate::canonical_scene::store) const fn alpha_mode(&self) -> &'scene AlphaMode {
        &self.stored.alpha
    }

    pub(in crate::canonical_scene::store) const fn double_sided(&self) -> bool {
        self.stored.double_sided
    }

    pub(in crate::canonical_scene::store) const fn factors(&self) -> MaterialFactors {
        self.stored.factors
    }

    pub(in crate::canonical_scene::store) const fn name(&self) -> Option<&'scene str> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn binding_offset(&self) -> usize {
        self.stored.bindings.start()
    }

    pub(in crate::canonical_scene::store) const fn binding_count(&self) -> usize {
        self.stored.bindings.len()
    }

    pub(in crate::canonical_scene::store) const fn factor_offset(&self) -> usize {
        self.factor_offset
    }

    /// Projects the unchanged transcript order one scalar at a time from the actual factors.
    pub(in crate::canonical_scene::store) fn factor_values(
        &self,
    ) -> NodeSceneAppearanceMaterialFactors<'scene> {
        NodeSceneAppearanceMaterialFactors::new(&self.stored.factors)
    }
}

/// One compact typed row and an optional borrowed source key, with no recreated key buffer.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceBinding<'scene> {
    source_index: usize,
    stored: &'scene StoredBinding,
    semantic_offset: usize,
    source_key: Option<&'scene str>,
}

impl<'scene> NodeSceneAppearanceBinding<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn texture(&self) -> usize {
        self.stored.texture.0 as usize
    }

    pub(in crate::canonical_scene::store) const fn role(&self) -> TextureRole {
        self.stored.role
    }

    pub(in crate::canonical_scene::store) const fn role_semantics(&self) -> &'scene TextureRole {
        &self.stored.role
    }

    pub(in crate::canonical_scene::store) const fn coordinate_source(
        &self,
    ) -> &'scene TextureCoordinateSource {
        &self.stored.coordinate_source
    }

    pub(in crate::canonical_scene::store) const fn channel_mapping(&self) -> TextureChannelSwizzle {
        self.stored.channels
    }

    pub(in crate::canonical_scene::store) const fn color_interpretation(
        &self,
    ) -> &'scene ColorInterpretation {
        &self.stored.color
    }

    pub(in crate::canonical_scene::store) const fn transform(&self) -> TextureTransform {
        self.stored.transform
    }

    pub(in crate::canonical_scene::store) const fn strength(&self) -> TextureStrength {
        self.stored.strength
    }

    pub(in crate::canonical_scene::store) const fn semantic_offset(&self) -> usize {
        self.semantic_offset
    }

    pub(in crate::canonical_scene::store) fn semantic_values(
        &self,
    ) -> NodeSceneAppearanceBindingScalars<'scene> {
        NodeSceneAppearanceBindingScalars::new(self.stored)
    }

    pub(in crate::canonical_scene::store) const fn blend(&self) -> &'scene TextureBlendOperation {
        &self.stored.blend
    }

    pub(in crate::canonical_scene::store) const fn source_key(&self) -> Option<&'scene str> {
        self.source_key
    }
}
