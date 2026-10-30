//! Bounded selected-row sanity checks; all retained payloads remain borrowed.

use super::*;

impl<'scene> NodeSceneAppearanceTables<'scene> {
    pub(in crate::canonical_scene::store) fn image_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceImage<'scene>> {
        let image = self
            .storage
            .images
            .get(row)
            .ok_or_else(|| invariant("image row is outside its proof-qualified table"))?;
        record_row_read();
        if image.payload.is_empty() || image.payload.len() > MAX_ENCODED_IMAGE_BYTES {
            return Err(invariant("validated image payload size is invalid"));
        }
        let encoded_payload = borrow_range(image.payload, &self.storage.image_bytes)?;
        let media_type = image
            .media_type
            .map(|range| -> NodeSceneSemanticResult<&'scene str> {
                let value = borrow_text(range, &self.storage.text, MAX_EXTENSION_MEDIA_TYPE_BYTES)?;
                validate_media_type(value)
                    .map_err(|_| invariant("validated image media type is invalid"))?;
                Ok(value)
            })
            .transpose()?;
        Ok(NodeSceneAppearanceImage {
            source_index: row,
            encoded_payload,
            media_type,
        })
    }

    pub(in crate::canonical_scene::store) fn sampler_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceSampler> {
        let &state = self
            .storage
            .samplers
            .get(row)
            .ok_or_else(|| invariant("sampler row is outside its proof-qualified table"))?;
        record_row_read();
        Ok(NodeSceneAppearanceSampler {
            source_index: row,
            state,
        })
    }

    pub(in crate::canonical_scene::store) fn texture_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceTexture> {
        let texture = self
            .storage
            .textures
            .get(row)
            .ok_or_else(|| invariant("texture row is outside its proof-qualified table"))?;
        record_row_read();
        let image = texture.image.0 as usize;
        let sampler = texture.sampler.0 as usize;
        if image >= self.storage.images.len() || sampler >= self.storage.samplers.len() {
            return Err(invariant(
                "validated texture target is outside image or sampler space",
            ));
        }
        Ok(NodeSceneAppearanceTexture {
            source_index: row,
            image,
            sampler,
        })
    }

    pub(in crate::canonical_scene::store) fn material_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceMaterial<'scene>> {
        let stored = self
            .storage
            .materials
            .get(row)
            .ok_or_else(|| invariant("material row is outside its proof-qualified table"))?;
        record_row_read();
        borrow_range(stored.bindings, &self.storage.bindings)?;
        stored
            .alpha
            .validate()
            .and_then(|()| stored.factors.validate())
            .map_err(|_| invariant("validated material numeric values are invalid"))?;
        validate_material_semantics(stored.semantics()).map_err(|_| {
            invariant("validated material factor groups disagree with its shading model")
        })?;
        let name = stored
            .name
            .map(|range| -> NodeSceneSemanticResult<&'scene str> {
                let value = borrow_text(range, &self.storage.text, MAX_ENTITY_NAME_BYTES)?;
                validate_entity_name_str(value)
                    .map_err(|_| invariant("validated material name is invalid"))?;
                Ok(value)
            })
            .transpose()?;
        Ok(NodeSceneAppearanceMaterial {
            source_index: row,
            stored,
            name,
            factor_offset: transcript_offset(row, MATERIAL_FACTOR_SCALARS)?,
        })
    }

    pub(in crate::canonical_scene::store) fn binding_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceBinding<'scene>> {
        let stored = self
            .storage
            .bindings
            .get(row)
            .ok_or_else(|| invariant("binding row is outside its proof-qualified table"))?;
        record_row_read();
        if stored.texture.0 as usize >= self.storage.textures.len() || !stored.channels.is_valid() {
            return Err(invariant(
                "validated binding target or channel swizzle is invalid",
            ));
        }
        stored
            .transform
            .validate()
            .map_err(|_| invariant("validated texture transform is not finite"))?;
        stored
            .strength
            .validate()
            .map_err(|_| invariant("validated texture strength is not finite"))?;
        let source_key = stored
            .source_key
            .map(|range| -> NodeSceneSemanticResult<&'scene str> {
                let value = borrow_text(
                    range,
                    &self.storage.text,
                    crate::appearance::MAX_TEXTURE_SOURCE_KEY_BYTES,
                )?;
                validate_texture_source_key(value)
                    .map_err(|_| invariant("validated texture source key is invalid"))?;
                Ok(value)
            })
            .transpose()?;
        // Source-key text is already checked above. The closed behavior domain gives it no
        // authority role, so no temporary TextureSourceKey buffer is needed for this check.
        validate_texture_binding_semantics(stored.semantics(None))
            .map_err(|_| invariant("validated texture binding semantics are inconsistent"))?;
        Ok(NodeSceneAppearanceBinding {
            source_index: row,
            stored,
            source_key,
            semantic_offset: transcript_offset(row, BINDING_SEMANTIC_SCALARS)?,
        })
    }
}
