use super::*;
use crate::appearance::{
    AlphaMode, CanonicalMaterialDescriptor, CanonicalTextureBindingDescriptor, ColorInterpretation,
    MaterialFactors, ShadingModel, TextureBlendOperation, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength, TextureTransform,
};
use crate::identity::TextureId;

fn push_texture(
    builder: &mut CanonicalSceneBuilder,
    byte: u8,
    provenance: Provenance,
) -> TextureId {
    let image = builder
        .try_push_image(encoded_image(byte, provenance))
        .expect("image fixture should be stored");
    let sampler = builder
        .try_push_sampler(Sampler::canonical_default(provenance))
        .expect("sampler fixture should be stored");
    builder
        .try_push_texture(CanonicalTextureDescriptor::new(image, sampler, provenance))
        .expect("texture fixture should be stored")
}

fn base_color_binding(
    texture: TextureId,
    provenance: Provenance,
) -> CanonicalTextureBindingDescriptor {
    CanonicalTextureBindingDescriptor::try_new(
        texture,
        TextureRole::BaseColor,
        TextureCoordinateSource::uv_set(0).expect("UV set zero should be valid"),
        TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA),
        ColorInterpretation::SrgbColor,
        TextureTransform::IDENTITY,
        TextureStrength::ONE,
        TextureBlendOperation::Replace,
        None,
        provenance,
        [],
    )
    .expect("base-color binding fixture should be valid")
}

fn normal_binding(texture: TextureId, provenance: Provenance) -> CanonicalTextureBindingDescriptor {
    CanonicalTextureBindingDescriptor::try_new(
        texture,
        TextureRole::Normal,
        TextureCoordinateSource::uv_set(0).expect("UV set zero should be valid"),
        TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB),
        ColorInterpretation::Data,
        TextureTransform::IDENTITY,
        TextureStrength::ONE,
        TextureBlendOperation::Replace,
        None,
        provenance,
        [],
    )
    .expect("normal binding fixture should be valid")
}

fn emissive_binding(
    texture: TextureId,
    provenance: Provenance,
) -> CanonicalTextureBindingDescriptor {
    CanonicalTextureBindingDescriptor::try_new(
        texture,
        TextureRole::Emissive,
        TextureCoordinateSource::uv_set(0).expect("UV set zero should be valid"),
        TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB),
        ColorInterpretation::SrgbColor,
        TextureTransform::IDENTITY,
        TextureStrength::ONE,
        TextureBlendOperation::Replace,
        None,
        provenance,
        [],
    )
    .expect("emissive binding fixture should be valid")
}

fn material_descriptor(
    bindings: impl IntoIterator<Item = CanonicalTextureBindingDescriptor>,
    provenance: Provenance,
) -> CanonicalMaterialDescriptor {
    CanonicalMaterialDescriptor::try_new(
        None,
        ShadingModel::Unlit,
        AlphaMode::Opaque,
        false,
        MaterialFactors::empty(),
        bindings,
        provenance,
        [],
    )
    .expect("material descriptor fixture should be valid")
}

#[test]
fn material_construction_lowers_nonzero_qualified_texture_references_exactly() {
    let (mut builder, provenance) = scene_builder();
    let generation = builder.generation();
    let first = push_texture(&mut builder, 1, provenance);
    let second = push_texture(&mut builder, 2, provenance);
    let third = push_texture(&mut builder, 3, provenance);

    let material = builder
        .try_push_material(material_descriptor(
            [
                base_color_binding(third, provenance),
                normal_binding(second, provenance),
                emissive_binding(third, provenance),
            ],
            provenance,
        ))
        .expect("qualified material should be stored");

    assert_eq!(first.index(), 0);
    assert_eq!(second.index(), 1);
    assert_eq!(third.index(), 2);
    assert_eq!(material.generation(), generation);
    assert_eq!(material.index(), 0);

    let material_index =
        usize::try_from(material.index()).expect("material index should fit usize");
    let stored = builder.data.materials[material_index].as_ref();
    let stored_indices = stored
        .texture_bindings()
        .iter()
        .map(|binding| binding.texture_index().as_u32())
        .collect::<Vec<_>>();
    assert_eq!(stored_indices, [2, 1, 2]);

    let qualified = stored
        .view(generation)
        .texture_bindings()
        .map(|binding| binding.texture())
        .collect::<Vec<_>>();
    assert_eq!(qualified, [third, second, third]);
}

#[test]
fn material_descriptor_rejects_foreign_texture_generation_without_mutation() {
    let (mut foreign, foreign_provenance) = scene_builder();
    let foreign_texture = push_texture(&mut foreign, 1, foreign_provenance);

    let (mut builder, provenance) = scene_builder();
    let local_texture = push_texture(&mut builder, 2, provenance);
    assert_eq!(foreign_texture.index(), local_texture.index());
    let original_len = builder.data.materials.len();
    let original_capacity = builder.data.materials.capacity();

    for texture in [
        foreign_texture,
        TextureId::new(foreign_texture.generation(), u32::MAX),
    ] {
        let error = builder
            .try_push_material(material_descriptor(
                [base_color_binding(texture, provenance)],
                provenance,
            ))
            .unwrap_err();

        assert_generation_mismatch_for_test(error, SceneEntityKind::Texture);
        assert_eq!(builder.data.materials.len(), original_len);
        assert_eq!(builder.data.materials.capacity(), original_capacity);
    }
}

#[test]
fn prepared_material_rejects_texture_at_len_and_never_publishes_early() {
    let (mut builder, provenance) = scene_builder();
    let texture = push_texture(&mut builder, 1, provenance);
    let generation = builder.generation();
    let texture_count = builder.data.textures.len();
    let first_missing_texture = TextureId::new(
        generation,
        u32::try_from(texture_count).expect("fixture texture count should fit canonical index"),
    );
    let original_len = builder.data.materials.len();
    let original_capacity = builder.data.materials.capacity();

    let invalid_plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::Materials)
        .expect("material insertion should have a capacity plan");
    let error = builder
        .prepare_push_material(
            material_descriptor(
                [base_color_binding(first_missing_texture, provenance)],
                provenance,
            ),
            invalid_plan,
        )
        .err()
        .expect("an out-of-range texture must fail before publication");
    assert_eq!(
        error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Texture,
            length: texture_count,
        })
    );
    assert_eq!(builder.data.materials.len(), original_len);
    assert_eq!(builder.data.materials.capacity(), original_capacity);

    let plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::Materials)
        .expect("material insertion should have a capacity plan");
    let prepared = builder
        .prepare_push_material(
            material_descriptor([base_color_binding(texture, provenance)], provenance),
            plan,
        )
        .expect("valid material should prepare");
    drop(prepared);
    assert_eq!(builder.data.materials.len(), original_len);
    assert_eq!(builder.data.materials.capacity(), original_capacity);

    let material = builder
        .try_push_material(material_descriptor(
            [base_color_binding(texture, provenance)],
            provenance,
        ))
        .expect("dropping a prepared material must leave the builder reusable");
    assert_eq!(material.generation(), generation);
    assert_eq!(material.index(), 0);
    assert_eq!(builder.data.materials.len(), 1);
    let binding = builder.data.materials[0]
        .as_ref()
        .view(generation)
        .texture_bindings()
        .next()
        .expect("stored material should retain its texture binding");
    assert_eq!(binding.texture(), texture);
}

#[test]
fn material_descriptor_checks_all_texture_generations_before_any_range() {
    let (mut foreign, foreign_provenance) = scene_builder();
    let foreign_texture = push_texture(&mut foreign, 1, foreign_provenance);

    let (mut builder, provenance) = scene_builder();
    push_texture(&mut builder, 2, provenance);
    let generation = builder.generation();
    let texture_count = builder.data.textures.len();
    let first_missing_texture = TextureId::new(
        generation,
        u32::try_from(texture_count).expect("fixture texture count should fit canonical index"),
    );
    let original_len = builder.data.materials.len();
    let original_capacity = builder.data.materials.capacity();

    let error = builder
        .try_push_material(material_descriptor(
            [
                base_color_binding(first_missing_texture, provenance),
                normal_binding(foreign_texture, provenance),
            ],
            provenance,
        ))
        .unwrap_err();

    assert_generation_mismatch_for_test(error, SceneEntityKind::Texture);
    assert_eq!(builder.data.materials.len(), original_len);
    assert_eq!(builder.data.materials.capacity(), original_capacity);
}
