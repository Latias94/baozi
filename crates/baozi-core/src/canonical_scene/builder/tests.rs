use super::*;
use crate::derived::DerivedAbsenceReason;
use crate::extension::ExtensionSchemaSnapshot;
use crate::identity::SceneEntityKind;
use crate::image::{CanonicalTextureDescriptor, Image, Sampler};
use crate::provenance::{Provenance, test_provenance_fixture};
use crate::scene_space::SceneSpace;

fn scene_builder() -> (CanonicalSceneBuilder, Provenance) {
    let (arena, provenance) = test_provenance_fixture();
    let builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("scene generation should remain available");
    (builder, provenance)
}

fn encoded_image(byte: u8, provenance: Provenance) -> Image {
    Image::try_encoded([byte], None, None, None, None, provenance)
        .expect("encoded image fixture should be valid")
}

fn assert_generation_mismatch(error: CanonicalSceneBuildError, expected_kind: SceneEntityKind) {
    let CanonicalSceneBuildError::Reference(SceneLookupError::GenerationMismatch { kind, .. }) =
        error
    else {
        panic!("expected a generation mismatch, observed {error:?}");
    };
    assert_eq!(kind, expected_kind);
}

#[test]
fn texture_construction_lowers_nonzero_qualified_references_exactly() {
    let (mut builder, provenance) = scene_builder();
    let generation = builder.generation();

    for byte in [1, 2] {
        builder
            .try_push_image(encoded_image(byte, provenance))
            .unwrap();
    }
    let image = builder
        .try_push_image(encoded_image(3, provenance))
        .unwrap();
    builder
        .try_push_sampler(Sampler::canonical_default(provenance))
        .unwrap();
    let sampler = builder
        .try_push_sampler(Sampler::canonical_default(provenance))
        .unwrap();
    let texture = builder
        .try_push_texture(CanonicalTextureDescriptor::new(image, sampler, provenance))
        .unwrap();

    assert_eq!(builder.data.images.len(), 3);
    assert_eq!(builder.data.samplers.len(), 2);
    assert_eq!(image.generation(), generation);
    assert_eq!(image.index(), 2);
    assert_eq!(sampler.generation(), generation);
    assert_eq!(sampler.index(), 1);
    assert_eq!(texture.generation(), generation);
    assert_eq!(texture.index(), 0);

    let texture_index = usize::try_from(texture.index()).expect("texture index should fit usize");
    let view = builder.data.textures[texture_index]
        .as_ref()
        .view(generation);
    assert_eq!(view.image(), image);
    assert_eq!(view.sampler(), sampler);
    assert_eq!(view.provenance(), provenance);
}

#[test]
fn texture_descriptor_rejects_foreign_generation_before_range_without_mutation() {
    let (mut first, first_provenance) = scene_builder();
    let foreign_image = first
        .try_push_image(encoded_image(1, first_provenance))
        .unwrap();
    let foreign_sampler = first
        .try_push_sampler(Sampler::canonical_default(first_provenance))
        .unwrap();

    let (mut second, second_provenance) = scene_builder();
    let local_image = second
        .try_push_image(encoded_image(2, second_provenance))
        .unwrap();
    let local_sampler = second
        .try_push_sampler(Sampler::canonical_default(second_provenance))
        .unwrap();

    for image in [
        foreign_image,
        ImageId::new(foreign_image.generation(), u32::MAX),
    ] {
        let error = second
            .try_push_texture(CanonicalTextureDescriptor::new(
                image,
                local_sampler,
                second_provenance,
            ))
            .unwrap_err();
        assert_generation_mismatch(error, SceneEntityKind::Image);
        assert!(second.data.textures.is_empty());
    }

    for sampler in [
        foreign_sampler,
        SamplerId::new(foreign_sampler.generation(), u32::MAX),
    ] {
        let error = second
            .try_push_texture(CanonicalTextureDescriptor::new(
                local_image,
                sampler,
                second_provenance,
            ))
            .unwrap_err();
        assert_generation_mismatch(error, SceneEntityKind::Sampler);
        assert!(second.data.textures.is_empty());
    }
}

#[test]
fn prepared_texture_rejects_current_generation_range_errors_and_never_publishes_early() {
    let (mut builder, provenance) = scene_builder();
    let image = builder
        .try_push_image(encoded_image(1, provenance))
        .unwrap();
    let sampler = builder
        .try_push_sampler(Sampler::canonical_default(provenance))
        .unwrap();
    let generation = builder.generation();
    let image_count = builder.data.images.len();
    let sampler_count = builder.data.samplers.len();
    let texture_count = builder.data.textures.len();
    let first_missing_image = ImageId::new(
        generation,
        u32::try_from(image_count).expect("fixture image count should fit canonical index"),
    );
    let first_missing_sampler = SamplerId::new(
        generation,
        u32::try_from(sampler_count).expect("fixture sampler count should fit canonical index"),
    );

    let image_plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::Textures)
        .unwrap();
    let image_error = builder
        .prepare_push_texture(
            CanonicalTextureDescriptor::new(first_missing_image, sampler, provenance),
            image_plan,
        )
        .err()
        .expect("an out-of-range image must fail before publication");
    assert_eq!(
        image_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Image,
            length: image_count,
        })
    );
    assert_eq!(builder.data.textures.len(), texture_count);

    let sampler_plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::Textures)
        .unwrap();
    let sampler_error = builder
        .prepare_push_texture(
            CanonicalTextureDescriptor::new(image, first_missing_sampler, provenance),
            sampler_plan,
        )
        .err()
        .expect("an out-of-range sampler must fail before publication");
    assert_eq!(
        sampler_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Sampler,
            length: sampler_count,
        })
    );
    assert_eq!(builder.data.textures.len(), texture_count);

    let plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::Textures)
        .unwrap();
    let prepared = builder
        .prepare_push_texture(
            CanonicalTextureDescriptor::new(image, sampler, provenance),
            plan,
        )
        .expect("valid texture should prepare");
    drop(prepared);
    assert!(builder.data.textures.is_empty());

    let texture = builder
        .try_push_texture(CanonicalTextureDescriptor::new(image, sampler, provenance))
        .expect("dropping a prepared texture must leave the builder reusable");
    assert_eq!(texture.generation(), generation);
    assert_eq!(texture.index(), 0);
    assert_eq!(builder.data.textures.len(), 1);
}

#[test]
fn texture_descriptor_checks_all_generations_before_any_range() {
    let (mut foreign, foreign_provenance) = scene_builder();
    let foreign_sampler = foreign
        .try_push_sampler(Sampler::canonical_default(foreign_provenance))
        .unwrap();

    let (mut builder, provenance) = scene_builder();
    builder
        .try_push_image(encoded_image(1, provenance))
        .unwrap();
    let generation = builder.generation();
    let image_count = builder.data.images.len();
    let first_missing_image = ImageId::new(
        generation,
        u32::try_from(image_count).expect("fixture image count should fit canonical index"),
    );
    let texture_count = builder.data.textures.len();

    let error = builder
        .try_push_texture(CanonicalTextureDescriptor::new(
            first_missing_image,
            foreign_sampler,
            provenance,
        ))
        .unwrap_err();

    assert_generation_mismatch(error, SceneEntityKind::Sampler);
    assert_eq!(builder.data.textures.len(), texture_count);
}
