use crate::appearance::{
    LegacyMaterialFactors, PbrMetallicRoughnessFactors, PbrSpecularGlossinessFactors,
    SupplementalPbrFactors, TextureChannel,
};
use crate::canonical_scene::store::typed_storage::appearance::{
    StoredImage, StoredImageIndex, StoredSamplerIndex, StoredTexture, StoredTextureIndex,
};
use crate::image::{MipmapFilter, TexelFilter, TextureAddressMode};
use crate::math::{Color, Vec2, Vec3};

use super::*;

fn empty_storage() -> AppearanceStorage {
    AppearanceStorage {
        images: Vec::new(),
        samplers: Vec::new(),
        textures: Vec::new(),
        bindings: Vec::new(),
        materials: Vec::new(),
        image_bytes: Vec::new(),
        text: Vec::new(),
    }
}

fn rich_factors() -> MaterialFactors {
    MaterialFactors::try_new_with_supplemental_pbr(
        Vec3::new(-0.0, f32::from_bits(1), 2.0),
        Some(
            PbrMetallicRoughnessFactors::try_new(
                Color::linear_rgba(0.125, -0.0, 0.25, 1.0),
                -0.0,
                0.75,
            )
            .unwrap(),
        ),
        Some(
            PbrSpecularGlossinessFactors::try_new(
                Color::linear_rgba(0.5, 0.25, -0.0, 1.0),
                Vec3::new(0.75, 0.5, 0.25),
                0.125,
            )
            .unwrap(),
        ),
        Some(
            LegacyMaterialFactors::try_new(
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(4.0, 5.0, 6.0),
                Vec3::new(7.0, 8.0, 9.0),
                Vec3::new(10.0, 11.0, 12.0),
                13.0,
                0.875,
            )
            .unwrap(),
        ),
        SupplementalPbrFactors::try_new(0.25, 0.5, 0.75, 2.5, Vec3::new(-0.0, 0.125, 0.25), 0.375)
            .unwrap(),
    )
    .unwrap()
}

fn fixture() -> AppearanceStorage {
    AppearanceStorage {
        images: vec![
            StoredImage {
                payload: TableRange::new(0, 3),
                media_type: Some(TextRange(TableRange::new(0, 9))),
            },
            StoredImage {
                payload: TableRange::new(3, 2),
                media_type: None,
            },
        ],
        samplers: vec![SamplerState::new(
            TexelFilter::Nearest,
            TexelFilter::Linear,
            MipmapFilter::None,
            TextureAddressMode::Repeat,
            TextureAddressMode::MirroredRepeat,
            TextureAddressMode::ClampToBorder,
        )],
        textures: vec![
            StoredTexture {
                image: StoredImageIndex(0),
                sampler: StoredSamplerIndex(0),
            },
            StoredTexture {
                image: StoredImageIndex(1),
                sampler: StoredSamplerIndex(0),
            },
        ],
        bindings: vec![
            StoredBinding {
                texture: StoredTextureIndex(1),
                role: TextureRole::BaseColor,
                coordinate_source: TextureCoordinateSource::UvSet(2),
                channels: TextureChannelSwizzle::RGBA,
                color: ColorInterpretation::SrgbColor,
                transform: TextureTransform::IDENTITY,
                strength: TextureStrength::try_new(0.5).unwrap(),
                blend: TextureBlendOperation::Replace,
                source_key: Some(TextRange(TableRange::new(9, 4))),
            },
            StoredBinding {
                texture: StoredTextureIndex(0),
                role: TextureRole::Normal,
                coordinate_source: TextureCoordinateSource::ObjectNormal,
                channels: TextureChannelSwizzle::RGB,
                color: ColorInterpretation::Data,
                transform: TextureTransform::try_new(
                    Vec2::new(-0.0, f32::from_bits(1)),
                    -0.25,
                    Vec2::new(-2.0, 3.0),
                )
                .unwrap(),
                strength: TextureStrength::try_new(-0.75).unwrap(),
                blend: TextureBlendOperation::Replace,
                source_key: None,
            },
            StoredBinding {
                texture: StoredTextureIndex(1),
                role: TextureRole::Occlusion,
                coordinate_source: TextureCoordinateSource::ReflectionVector,
                channels: TextureChannelSwizzle::RED,
                color: ColorInterpretation::Data,
                transform: TextureTransform::IDENTITY,
                strength: TextureStrength::try_new(-0.0).unwrap(),
                blend: TextureBlendOperation::Replace,
                source_key: Some(TextRange(TableRange::new(13, 6))),
            },
        ],
        materials: vec![
            StoredMaterial {
                bindings: TableRange::new(0, 1),
                shading: ShadingModel::Unlit,
                alpha: AlphaMode::Opaque,
                double_sided: false,
                factors: MaterialFactors::empty(),
                name: None,
            },
            StoredMaterial {
                bindings: TableRange::new(1, 1),
                shading: ShadingModel::PbrMetallicRoughness,
                alpha: AlphaMode::mask(-0.0).unwrap(),
                double_sided: true,
                factors: rich_factors(),
                name: Some(TextRange(TableRange::new(19, 0))),
            },
            StoredMaterial {
                bindings: TableRange::new(2, 1),
                shading: ShadingModel::Unspecified,
                alpha: AlphaMode::Blend,
                double_sided: false,
                factors: MaterialFactors::empty(),
                name: Some(TextRange(TableRange::new(19, 6))),
            },
        ],
        image_bytes: vec![137, 80, 78, 0, 255],
        // The model's physical order is media, source keys, material names.
        text: "image/pngbase粗糙材质".as_bytes().to_vec(),
    }
}

// Mechanical reader tests deliberately bypass proof minting. Cross-material semantics and
// corrupt numeric internals belong to central validation; these tests close borrowed projections.
fn tables(storage: &AppearanceStorage) -> NodeSceneAppearanceTables<'_> {
    NodeSceneAppearanceTables { storage }
}

fn exhaust(storage: &AppearanceStorage) -> NodeSceneSemanticResult<()> {
    let mut cursor = tables(storage).into_cursor();
    while cursor.next_image()?.is_some() {}
    while cursor.next_sampler()?.is_some() {}
    while cursor.next_texture()?.is_some() {}
    while cursor.next_material()?.is_some() {}
    while cursor.next_binding()?.is_some() {}
    cursor.finish()
}

#[test]
fn selected_views_borrow_actual_rows_bytes_and_text_without_allocating() {
    let storage = fixture();
    let allocations = allocation_counter::measure(|| {
        let view = tables(&storage);
        let image = view.image_at(0).unwrap();
        assert_eq!(image.source_index(), 0);
        assert!(std::ptr::eq(
            image.encoded_payload(),
            &storage.image_bytes[..3]
        ));
        assert_eq!(image.media_type(), Some("image/png"));
        assert!(std::ptr::eq(
            image.media_type().unwrap().as_bytes(),
            &storage.text[..9]
        ));
        let last_image = view.image_at(1).unwrap();
        assert_eq!(last_image.encoded_payload(), &[0, 255]);
        assert!(last_image.media_type().is_none());
        let sampler = view.sampler_at(0).unwrap();
        assert_eq!(sampler.source_index(), 0);
        assert_eq!(sampler.state(), storage.samplers[0]);
        let texture = view.texture_at(1).unwrap();
        assert_eq!(
            (texture.source_index(), texture.image(), texture.sampler()),
            (1, 1, 0)
        );
        assert_eq!(view.material_at(0).unwrap().name(), None);
        let material = view.material_at(1).unwrap();
        assert_eq!(material.name(), Some(""));
        assert!(std::ptr::eq(material.stored, &storage.materials[1]));
        assert!(std::ptr::eq(
            material.shading_model(),
            &storage.materials[1].shading
        ));
        assert_eq!(
            material.alpha_mode().mask_cutoff().unwrap().to_bits(),
            (-0.0_f32).to_bits()
        );
        assert!(material.double_sided());
        assert_eq!(material.factors(), storage.materials[1].factors);
        assert_eq!(
            (
                material.source_index(),
                material.binding_offset(),
                material.binding_count()
            ),
            (1, 1, 1)
        );
        assert_eq!(material.factor_offset(), 39);
        let named = view.material_at(2).unwrap();
        assert_eq!(named.name(), Some("材质"));
        assert!(std::ptr::eq(
            named.name().unwrap().as_bytes(),
            &storage.text[19..]
        ));
        let binding = view.binding_at(0).unwrap();
        assert!(std::ptr::eq(binding.stored, &storage.bindings[0]));
        assert_eq!(binding.texture(), 1);
        assert_eq!(binding.role(), TextureRole::BaseColor);
        assert!(std::ptr::eq(
            binding.role_semantics(),
            &storage.bindings[0].role
        ));
        assert_eq!(
            binding.coordinate_source(),
            &TextureCoordinateSource::UvSet(2)
        );
        assert_eq!(binding.channel_mapping(), TextureChannelSwizzle::RGBA);
        assert_eq!(
            binding.channel_mapping().as_slice(),
            &[
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Blue,
                TextureChannel::Alpha
            ]
        );
        assert_eq!(
            binding.color_interpretation(),
            &ColorInterpretation::SrgbColor
        );
        assert_eq!(binding.transform(), TextureTransform::IDENTITY);
        assert_eq!(binding.strength().get(), 0.5);
        assert_eq!(binding.blend(), &TextureBlendOperation::Replace);
        assert_eq!(binding.source_key(), Some("base"));
        assert!(std::ptr::eq(
            binding.source_key().unwrap().as_bytes(),
            &storage.text[9..13]
        ));
        assert!(view.binding_at(1).unwrap().source_key().is_none());
        let last_binding = view.binding_at(2).unwrap();
        assert_eq!(last_binding.source_key(), Some("粗糙"));
        assert_eq!(last_binding.semantic_offset(), 12);
        assert!(std::ptr::eq(
            last_binding.source_key().unwrap().as_bytes(),
            &storage.text[13..19]
        ));
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn scalar_projections_preserve_snapshot_order_bits_and_exact_exhaustion() {
    let storage = fixture();
    let expected: [f32; 39] = [
        -0.0,
        f32::from_bits(1),
        2.0,
        0.125,
        -0.0,
        0.25,
        1.0,
        -0.0,
        0.75,
        0.5,
        0.25,
        -0.0,
        1.0,
        0.75,
        0.5,
        0.25,
        0.125,
        1.0,
        2.0,
        3.0,
        4.0,
        5.0,
        6.0,
        7.0,
        8.0,
        9.0,
        10.0,
        11.0,
        12.0,
        13.0,
        0.875,
        0.25,
        0.5,
        0.75,
        2.5,
        -0.0,
        0.125,
        0.25,
        0.375,
    ];
    let allocations = allocation_counter::measure(|| {
        let material = tables(&storage).material_at(1).unwrap();
        let mut factors = material.factor_values();
        assert!(factors.clone().finish().is_err());
        assert_eq!(factors.len(), expected.len());
        for value in expected {
            assert_eq!(factors.next().unwrap().to_bits(), value.to_bits());
        }
        assert_eq!(factors.next(), None);
        factors.finish().unwrap();
        let absent = tables(&storage).material_at(0).unwrap();
        assert!(absent.factor_values().all(|value| value.to_bits() == 0));
        let binding = tables(&storage).binding_at(1).unwrap();
        let mut scalars = binding.semantic_values();
        assert!(scalars.clone().finish().is_err());
        let mut independent = scalars.clone();
        assert_eq!(
            independent.next_back().unwrap().to_bits(),
            (-0.75_f32).to_bits()
        );
        for value in [-0.0_f32, f32::from_bits(1), -0.25, -2.0, 3.0, -0.75] {
            assert_eq!(scalars.next().unwrap().to_bits(), value.to_bits());
        }
        assert_eq!(scalars.next_back(), None);
        scalars.finish().unwrap();
        assert_eq!(independent.len(), 5);
        assert_eq!(independent.next().unwrap().to_bits(), (-0.0_f32).to_bits());
        assert!(independent.finish().is_err());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}

#[test]
fn random_reads_visit_one_row_without_scanning_a_corrupt_prefix() {
    let mut storage = fixture();
    storage.images[0].payload = TableRange::new(usize::MAX, 3);
    storage.materials[0].bindings = TableRange::new(usize::MAX, 1);
    storage.bindings[0].source_key = Some(TextRange(TableRange::new(usize::MAX, 4)));
    take_row_read_count_for_test();
    assert_eq!(tables(&storage).image_at(1).unwrap().source_index(), 1);
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(tables(&storage).material_at(2).unwrap().source_index(), 2);
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(tables(&storage).binding_at(2).unwrap().source_index(), 2);
    assert_eq!(take_row_read_count_for_test(), 1);
    assert!(tables(&storage).image_at(usize::MAX).is_err());
    assert!(tables(&storage).sampler_at(1).is_err());
    assert!(tables(&storage).texture_at(2).is_err());
    assert!(tables(&storage).material_at(3).is_err());
    assert!(tables(&storage).binding_at(3).is_err());
    assert_eq!(take_row_read_count_for_test(), 0);
    assert!(tables(&storage).image_at(0).is_err());
    assert!(tables(&storage).material_at(0).is_err());
    assert!(tables(&storage).binding_at(0).is_err());
    assert_eq!(take_row_read_count_for_test(), 3);
}

#[test]
fn selected_rows_reject_broken_ranges_text_links_and_bounded_semantics() {
    let image_corruptions: [fn(&mut AppearanceStorage); 3] = [
        |s| s.images[0].payload = TableRange::new(0, 0),
        |s| s.images[0].media_type = Some(TextRange(TableRange::new(usize::MAX, 9))),
        |s| s.text[0] = b'I',
    ];
    for corrupt in image_corruptions {
        let mut storage = fixture();
        corrupt(&mut storage);
        assert!(tables(&storage).image_at(0).is_err());
    }
    let mut storage = fixture();
    storage.textures[0].image = StoredImageIndex(2);
    assert!(tables(&storage).texture_at(0).is_err());
    storage.textures[0].image = StoredImageIndex(0);
    storage.textures[0].sampler = StoredSamplerIndex(1);
    assert!(tables(&storage).texture_at(0).is_err());
    let mut storage = fixture();
    storage.text[19] = 0xff;
    assert!(tables(&storage).material_at(2).is_err());
    storage.materials[0].shading = ShadingModel::PbrMetallicRoughness;
    assert!(tables(&storage).material_at(0).is_err());
    let binding_corruptions: [fn(&mut AppearanceStorage); 3] = [
        |s| s.bindings[0].texture = StoredTextureIndex(2),
        |s| s.bindings[0].source_key = Some(TextRange(TableRange::new(9, 0))),
        |s| s.bindings[0].color = ColorInterpretation::Data,
    ];
    for corrupt in binding_corruptions {
        let mut storage = fixture();
        corrupt(&mut storage);
        assert!(tables(&storage).binding_at(0).is_err());
    }
}

#[test]
fn ordered_reads_close_both_pools_and_preserve_virtual_source_key_offsets_without_allocating() {
    let storage = fixture();
    take_row_read_count_for_test();
    let allocations = allocation_counter::measure(|| {
        let mut cursor = tables(&storage).into_cursor();
        assert_eq!(
            (
                cursor.image_count(),
                cursor.sampler_count(),
                cursor.texture_count(),
                cursor.material_count(),
                cursor.binding_count()
            ),
            (2, 1, 2, 3, 3)
        );
        for row in 0..2 {
            assert_eq!(cursor.next_image().unwrap().unwrap().source_index(), row);
        }
        assert!(cursor.next_image().unwrap().is_none());
        assert_eq!(cursor.next_sampler().unwrap().unwrap().source_index(), 0);
        assert!(cursor.next_sampler().unwrap().is_none());
        for row in 0..2 {
            assert_eq!(cursor.next_texture().unwrap().unwrap().source_index(), row);
        }
        assert!(cursor.next_texture().unwrap().is_none());
        for row in 0..3 {
            assert_eq!(cursor.next_material().unwrap().unwrap().source_index(), row);
        }
        assert!(cursor.next_material().unwrap().is_none());
        for (row, offset) in [0, 4, 4].into_iter().enumerate() {
            assert_eq!(cursor.next_binding().unwrap().unwrap().source_index(), row);
            assert_eq!(cursor.source_key_offset(), offset);
        }
        assert!(cursor.next_binding().unwrap().is_none());
        cursor.finish().unwrap();
    });
    assert_eq!(take_row_read_count_for_test(), 11);
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    let corruptions: [fn(&mut AppearanceStorage); 7] = [
        |s| s.images[1].payload = TableRange::new(2, 2),
        |s| s.materials[1].bindings = TableRange::new(0, 1),
        |s| s.materials[2].bindings = TableRange::new(2, 0),
        |s| s.materials[1].name = Some(TextRange(TableRange::new(18, 0))),
        |s| s.bindings[0].source_key = Some(TextRange(TableRange::new(10, 3))),
        |s| s.image_bytes.push(0),
        |s| s.text.push(b'x'),
    ];
    for corrupt in corruptions {
        let mut storage = fixture();
        corrupt(&mut storage);
        assert!(exhaust(&storage).is_err());
    }
}

#[test]
fn unfinished_or_backward_phases_reject_and_empty_or_binding_free_materials_finish() {
    let storage = fixture();
    assert!(tables(&storage).into_cursor().finish().is_err());
    let mut cursor = tables(&storage).into_cursor();
    assert!(cursor.next_sampler().is_err());
    while cursor.next_image().unwrap().is_some() {}
    assert!(cursor.next_sampler().unwrap().is_some());
    assert!(cursor.next_image().is_err());
    assert!(cursor.finish().is_err());
    let mut cursor = tables(&storage).into_cursor();
    while cursor.next_image().unwrap().is_some() {}
    while cursor.next_sampler().unwrap().is_some() {}
    while cursor.next_texture().unwrap().is_some() {}
    while cursor.next_material().unwrap().is_some() {}
    assert!(cursor.finish().is_err());
    exhaust(&empty_storage()).unwrap();
    let mut empty = empty_storage();
    empty.materials.push(StoredMaterial {
        bindings: TableRange::new(0, 0),
        shading: ShadingModel::Unlit,
        alpha: AlphaMode::Opaque,
        double_sided: false,
        factors: MaterialFactors::empty(),
        name: Some(TextRange(TableRange::new(0, 0))),
    });
    assert_eq!(tables(&empty).material_at(0).unwrap().name(), Some(""));
    exhaust(&empty).unwrap();
    empty.text.extend_from_slice("空".as_bytes());
    empty.materials[0].name = Some(TextRange(TableRange::new(0, 3)));
    assert_eq!(tables(&empty).material_at(0).unwrap().name(), Some("空"));
    exhaust(&empty).unwrap();
}
