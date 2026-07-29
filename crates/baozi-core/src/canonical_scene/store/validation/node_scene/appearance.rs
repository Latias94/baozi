use baozi_operation::{
    CanonicalBytesRef, CanonicalRecordRef, CanonicalRecordView, CanonicalUtf8Ref,
    StoreValidationSession,
};

use crate::canonical_scene::store::production::{Presence, ValidatedAppearanceBatch};
use crate::canonical_scene::store::schema::CanonicalBlockRef;
use crate::canonical_scene::store::schema::appearance::{
    APPEARANCE_BINDING_FIELD_COUNT, APPEARANCE_DIRECTORY_FIELD_COUNT, APPEARANCE_IMAGE_FIELD_COUNT,
    APPEARANCE_IMAGE_PAYLOAD_ENCODED, APPEARANCE_MATERIAL_FIELD_COUNT, APPEARANCE_MEDIA_TYPE_UTF8,
    APPEARANCE_PRESENCE_ABSENT, APPEARANCE_PRESENCE_PRESENT, APPEARANCE_SAMPLER_FIELD_COUNT,
    APPEARANCE_SCHEMA_REVISION, APPEARANCE_TEXTURE_FIELD_COUNT, AppearanceBindingTableSchema,
    AppearanceDirectorySchema, AppearanceEncodedImagePayloadSchema, AppearanceImageTableSchema,
    AppearanceMaterialTableSchema, AppearanceMediaTypeSchema, AppearanceSamplerTableSchema,
    AppearanceTextureTableSchema, appearance_binding_field, appearance_directory_field,
    appearance_image_field, appearance_material_field, appearance_sampler_field,
    appearance_texture_field, encode_appearance_address_mode, encode_appearance_mipmap_filter,
    encode_appearance_texel_filter,
};

use super::super::require_scalar;

#[derive(Clone, Copy)]
pub(super) struct AppearanceValidatedRefs {
    pub(super) images: Option<CanonicalBlockRef<AppearanceImageTableSchema>>,
    pub(super) samplers: Option<CanonicalBlockRef<AppearanceSamplerTableSchema>>,
    pub(super) textures: Option<CanonicalBlockRef<AppearanceTextureTableSchema>>,
    pub(super) materials: Option<CanonicalBlockRef<AppearanceMaterialTableSchema>>,
    pub(super) bindings: Option<CanonicalBlockRef<AppearanceBindingTableSchema>>,
    pub(super) image_payloads: Option<CanonicalBlockRef<AppearanceEncodedImagePayloadSchema>>,
    pub(super) media_types: Option<CanonicalBlockRef<AppearanceMediaTypeSchema>>,
}

pub(super) fn validate_appearance_graph<R>(
    session: &mut StoreValidationSession<'_, '_>,
    appearance_ref: CanonicalRecordRef<AppearanceDirectorySchema>,
    expected: &ValidatedAppearanceBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<AppearanceValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(appearance_ref)?;
    if directory.rows() != 1 || directory.fields() != APPEARANCE_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, appearance_directory_field::SCHEMA_REVISION),
        APPEARANCE_SCHEMA_REVISION,
    )?;

    let images = optional_record_ref::<AppearanceImageTableSchema>(
        session,
        directory,
        appearance_directory_field::IMAGE_COUNT,
        appearance_directory_field::IMAGES,
        expected.image_count(),
        APPEARANCE_IMAGE_FIELD_COUNT,
    )?;
    let samplers = optional_record_ref::<AppearanceSamplerTableSchema>(
        session,
        directory,
        appearance_directory_field::SAMPLER_COUNT,
        appearance_directory_field::SAMPLERS,
        expected.sampler_count(),
        APPEARANCE_SAMPLER_FIELD_COUNT,
    )?;
    let textures = optional_record_ref::<AppearanceTextureTableSchema>(
        session,
        directory,
        appearance_directory_field::TEXTURE_COUNT,
        appearance_directory_field::TEXTURES,
        expected.texture_count(),
        APPEARANCE_TEXTURE_FIELD_COUNT,
    )?;
    let materials = optional_record_ref::<AppearanceMaterialTableSchema>(
        session,
        directory,
        appearance_directory_field::MATERIAL_COUNT,
        appearance_directory_field::MATERIALS,
        expected.material_count(),
        APPEARANCE_MATERIAL_FIELD_COUNT,
    )?;
    let bindings = optional_record_ref::<AppearanceBindingTableSchema>(
        session,
        directory,
        appearance_directory_field::BINDING_COUNT,
        appearance_directory_field::BINDINGS,
        expected.binding_count(),
        APPEARANCE_BINDING_FIELD_COUNT,
    )?;
    let image_payloads = optional_bytes_ref::<AppearanceEncodedImagePayloadSchema>(
        session,
        directory,
        appearance_directory_field::IMAGE_PAYLOAD_BYTE_COUNT,
        appearance_directory_field::IMAGE_PAYLOADS,
        expected.image_payload_byte_count(),
    )?;
    let media_types = optional_utf8_ref::<AppearanceMediaTypeSchema>(
        session,
        directory,
        appearance_directory_field::MEDIA_TYPE_BYTE_COUNT,
        appearance_directory_field::MEDIA_TYPES,
        expected.media_type_byte_count(),
    )?;

    validate_images(
        session,
        images,
        image_payloads,
        media_types,
        expected,
        observe_work,
    )?;
    validate_samplers(session, samplers, expected, observe_work)?;
    validate_textures(session, textures, images, samplers, expected, observe_work)?;
    validate_bindings(session, bindings, textures, expected, observe_work)?;
    validate_materials(session, materials, bindings, expected, observe_work)?;

    Some(AppearanceValidatedRefs {
        images: images.map(CanonicalBlockRef::from_store),
        samplers: samplers.map(CanonicalBlockRef::from_store),
        textures: textures.map(CanonicalBlockRef::from_store),
        materials: materials.map(CanonicalBlockRef::from_store),
        bindings: bindings.map(CanonicalBlockRef::from_store),
        image_payloads: image_payloads.map(CanonicalBlockRef::from_store),
        media_types: media_types.map(CanonicalBlockRef::from_store),
    })
}

fn optional_record_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
    expected_fields: usize,
) -> Option<Option<CanonicalRecordRef<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.record_ref::<Tag>(0, reference_field)?;
        let table = session.canonical_records(reference)?;
        if table.rows() != usize::try_from(expected_count).ok()?
            || table.fields() != expected_fields
        {
            return None;
        }
        Some(Some(reference))
    }
}

fn optional_bytes_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
) -> Option<Option<CanonicalBytesRef<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.bytes_ref::<Tag>(0, reference_field)?;
        let payload = session.canonical_bytes(reference)?;
        (payload.len() == usize::try_from(expected_count).ok()?).then_some(Some(reference))
    }
}

fn optional_utf8_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
) -> Option<Option<CanonicalUtf8Ref<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.utf8_ref::<Tag>(0, reference_field)?;
        let payload = session.canonical_utf8(reference)?;
        (payload.len() == usize::try_from(expected_count).ok()?).then_some(Some(reference))
    }
}

fn validate_images<R>(
    session: &mut StoreValidationSession<'_, '_>,
    images: Option<CanonicalRecordRef<AppearanceImageTableSchema>>,
    image_payloads: Option<CanonicalBytesRef<AppearanceEncodedImagePayloadSchema>>,
    media_types: Option<CanonicalUtf8Ref<AppearanceMediaTypeSchema>>,
    expected: &ValidatedAppearanceBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(images) = images else {
        return (expected.image_count() == 0).then_some(());
    };
    let table = session.canonical_records(images)?;
    let payload_ref = image_payloads?;
    let payload = session.canonical_bytes(payload_ref)?;
    let media = match media_types {
        Some(reference) => Some((reference, session.canonical_utf8(reference)?)),
        None => None,
    };
    for row in 0..table.rows() {
        observe_work(1)?;
        let image = expected.image(row)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::IMAGE),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_image_field::PAYLOAD_KIND),
            APPEARANCE_IMAGE_PAYLOAD_ENCODED,
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::PAYLOAD_OFFSET),
            image.payload_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::PAYLOAD_COUNT),
            image.payload_count(),
        )?;
        require_scalar(
            table.bytes_ref::<AppearanceEncodedImagePayloadSchema>(
                row,
                appearance_image_field::PAYLOADS,
            ),
            payload_ref,
        )?;
        let payload_range =
            checked_range(image.payload_offset(), image.payload_count(), payload.len())?;
        let expected_payload = image.payload();
        observe_work(u64::try_from(expected_payload.len()).ok()?)?;
        if payload.get(payload_range)? != expected_payload {
            return None;
        }

        let (presence, media_ref, expected_media) = match image.media_type() {
            Presence::Absent if !image.media_type_present() && image.media_type_count() == 0 => {
                (APPEARANCE_PRESENCE_ABSENT, None, None)
            }
            Presence::Present(expected_media) if image.media_type_present() => {
                let (reference, bytes) = media?;
                (
                    APPEARANCE_PRESENCE_PRESENT,
                    Some(reference),
                    Some((expected_media, bytes)),
                )
            }
            Presence::Absent | Presence::Present(_) => return None,
        };
        require_scalar(
            table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPE_PRESENCE),
            presence,
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPE_ENCODING),
            APPEARANCE_MEDIA_TYPE_UTF8,
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::MEDIA_TYPE_OFFSET),
            image.media_type_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::MEDIA_TYPE_COUNT),
            image.media_type_count(),
        )?;
        match (media_ref, expected_media) {
            (None, None) => require_scalar(
                table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPES),
                0_u8,
            )?,
            (Some(reference), Some((expected_media, bytes))) => {
                require_scalar(
                    table.utf8_ref::<AppearanceMediaTypeSchema>(
                        row,
                        appearance_image_field::MEDIA_TYPES,
                    ),
                    reference,
                )?;
                let range = checked_range(
                    image.media_type_offset(),
                    image.media_type_count(),
                    bytes.len(),
                )?;
                let expected_bytes = expected_media.as_str().as_bytes();
                observe_work(u64::try_from(expected_bytes.len()).ok()?)?;
                if bytes.as_bytes().get(range)? != expected_bytes {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(())
}

fn validate_samplers<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceSamplerTableSchema>>,
    expected: &ValidatedAppearanceBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.sampler_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    for (row, sampler) in expected.samplers().enumerate() {
        observe_work(1)?;
        let state = sampler.state();
        require_scalar(
            table.scalar::<u32>(row, appearance_sampler_field::SAMPLER),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_sampler_field::MINIFICATION),
            encode_appearance_texel_filter(state.minification()),
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_sampler_field::MAGNIFICATION),
            encode_appearance_texel_filter(state.magnification()),
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_sampler_field::MIPMAP),
            encode_appearance_mipmap_filter(state.mipmap()),
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_sampler_field::ADDRESS_U),
            encode_appearance_address_mode(state.address_u()),
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_sampler_field::ADDRESS_V),
            encode_appearance_address_mode(state.address_v()),
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_sampler_field::ADDRESS_W),
            encode_appearance_address_mode(state.address_w()),
        )?;
    }
    Some(())
}

fn validate_textures<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceTextureTableSchema>>,
    images: Option<CanonicalRecordRef<AppearanceImageTableSchema>>,
    samplers: Option<CanonicalRecordRef<AppearanceSamplerTableSchema>>,
    expected: &ValidatedAppearanceBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.texture_count() == 0).then_some(());
    };
    let images = images?;
    let samplers = samplers?;
    let table = session.canonical_records(reference)?;
    for (row, texture) in expected.textures().enumerate() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_texture_field::TEXTURE),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_texture_field::IMAGE),
            texture.image().index(),
        )?;
        require_scalar(
            table.record_ref::<AppearanceImageTableSchema>(row, appearance_texture_field::IMAGES),
            images,
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_texture_field::SAMPLER),
            texture.sampler().index(),
        )?;
        require_scalar(
            table.record_ref::<AppearanceSamplerTableSchema>(
                row,
                appearance_texture_field::SAMPLERS,
            ),
            samplers,
        )?;
    }
    Some(())
}

fn validate_bindings<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceBindingTableSchema>>,
    textures: Option<CanonicalRecordRef<AppearanceTextureTableSchema>>,
    expected: &ValidatedAppearanceBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.binding_count() == 0).then_some(());
    };
    let textures = textures?;
    let table = session.canonical_records(reference)?;
    for (row, binding) in expected.bindings().enumerate() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_binding_field::BINDING),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_binding_field::TEXTURE),
            binding.texture().index(),
        )?;
        require_scalar(
            table.record_ref::<AppearanceTextureTableSchema>(
                row,
                appearance_binding_field::TEXTURES,
            ),
            textures,
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_binding_field::ROLE),
            binding.role_tag(),
        )?;
    }
    Some(())
}

fn validate_materials<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    bindings: Option<CanonicalRecordRef<AppearanceBindingTableSchema>>,
    expected: &ValidatedAppearanceBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.material_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    for (row, material) in expected.materials().enumerate() {
        observe_work(1)?;
        let range = material.authored().bindings();
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::MATERIAL),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::BINDING_OFFSET),
            range.offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::BINDING_COUNT),
            range.count(),
        )?;
        if range.count() == 0 {
            require_scalar(
                table.scalar::<u8>(row, appearance_material_field::BINDINGS),
                0_u8,
            )?;
        } else {
            require_scalar(
                table.record_ref::<AppearanceBindingTableSchema>(
                    row,
                    appearance_material_field::BINDINGS,
                ),
                bindings?,
            )?;
        }
    }
    Some(())
}

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
}
