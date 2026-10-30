use super::*;
use crate::appearance::{
    LegacyMaterialFactors, PbrMetallicRoughnessFactors, SupplementalPbrFactors, TextureChannel,
};
use crate::image::{MipmapFilter, TexelFilter, TextureAddressMode};
use crate::math::{Color, Vec2, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interrupted(u64);

#[derive(Default)]
struct Control {
    completed: u64,
    reject_before: Option<u64>,
    omit_before: Option<u64>,
}

impl TypedStorageValidationControl for Control {
    type Error = Interrupted;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        if self.reject_before == Some(self.completed) {
            return Err(Interrupted(self.completed));
        }
        if self.omit_before == Some(self.completed) {
            return Ok(None);
        }
        let result = action();
        self.completed += 1;
        Ok(Some(result))
    }
}

struct Fixture {
    storage: AppearanceStorage,
    counts: AppearanceMeteredCounts,
}

impl Fixture {
    fn validate(&self, control: &mut Control) -> Result<bool, Interrupted> {
        self.storage.validate(control, self.counts)
    }

    fn recount(&mut self) {
        self.counts = AppearanceMeteredCounts::for_storage(&self.storage).unwrap();
    }
}

fn empty() -> AppearanceStorage {
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

fn text(pool: &mut Vec<u8>, value: &str) -> TextRange {
    let start = pool.len();
    pool.extend_from_slice(value.as_bytes());
    TextRange(TableRange::new(start, value.len()))
}

fn fixture() -> Fixture {
    let mut storage = empty();
    let media0 = text(&mut storage.text, "image/png");
    let media1 = text(&mut storage.text, "image/jpeg");
    let key0 = text(&mut storage.text, "base.slot");
    let key2 = text(&mut storage.text, "\u{3bc}");
    let key3 = text(&mut storage.text, "rough");
    let name0 = text(&mut storage.text, "\u{3bb}mat");
    let name2 = text(&mut storage.text, "");
    storage.image_bytes = vec![0, 255, 17, 128, 42, 0, 1];
    storage.images = vec![
        StoredImage {
            payload: TableRange::new(0, 4),
            media_type: Some(media0),
        },
        StoredImage {
            payload: TableRange::new(4, 3),
            media_type: Some(media1),
        },
    ];
    storage.samplers = vec![
        SamplerState::CANONICAL_DEFAULT,
        SamplerState::new(
            TexelFilter::Nearest,
            TexelFilter::Linear,
            MipmapFilter::None,
            TextureAddressMode::ClampToBorder,
            TextureAddressMode::MirroredRepeat,
            TextureAddressMode::ClampToEdge,
        ),
    ];
    storage.textures = vec![
        StoredTexture {
            image: StoredImageIndex(0),
            sampler: StoredSamplerIndex(0),
        },
        StoredTexture {
            image: StoredImageIndex(1),
            sampler: StoredSamplerIndex(1),
        },
    ];
    let binding = |texture, role, channels, color, strength, source_key| StoredBinding {
        texture: StoredTextureIndex(texture),
        role,
        coordinate_source: TextureCoordinateSource::UvSet(0),
        channels,
        color,
        transform: TextureTransform::IDENTITY,
        strength: TextureStrength::try_new(strength).unwrap(),
        blend: TextureBlendOperation::Replace,
        source_key,
    };
    storage.bindings = vec![
        binding(
            0,
            TextureRole::BaseColor,
            TextureChannelSwizzle::try_new(&[
                TextureChannel::Blue,
                TextureChannel::Red,
                TextureChannel::Green,
                TextureChannel::Alpha,
            ])
            .unwrap(),
            ColorInterpretation::SrgbColor,
            0.75,
            Some(key0),
        ),
        binding(
            1,
            TextureRole::Normal,
            TextureChannelSwizzle::RGB,
            ColorInterpretation::Data,
            -2.0,
            None,
        ),
        binding(
            0,
            TextureRole::Metallic,
            TextureChannelSwizzle::RED,
            ColorInterpretation::Data,
            1.0,
            Some(key2),
        ),
        binding(
            0,
            TextureRole::Roughness,
            TextureChannelSwizzle::RED,
            ColorInterpretation::Data,
            1.0,
            Some(key3),
        ),
    ];
    storage.bindings[1].transform =
        TextureTransform::try_new(Vec2::new(-0.0, -2.0), 7.0, Vec2::new(-3.0, 0.0)).unwrap();
    storage.bindings[1].coordinate_source = TextureCoordinateSource::UvSet(u16::MAX);
    let factors = MaterialFactors::try_new_with_supplemental_pbr(
        Vec3::new(2.0, -0.0, 0.5),
        Some(
            PbrMetallicRoughnessFactors::try_new(
                Color::linear_rgba(0.7, -0.0, 0.5, 1.0),
                0.25,
                0.75,
            )
            .unwrap(),
        ),
        None,
        Some(
            LegacyMaterialFactors::try_new(
                Vec3::ZERO,
                Vec3::new(3.0, 2.0, 1.0),
                Vec3::ZERO,
                Vec3::ZERO,
                64.0,
                1.0,
            )
            .unwrap(),
        ),
        SupplementalPbrFactors::try_new(0.2, 0.3, 0.4, 5.0, Vec3::new(2.0, 0.0, 1.0), 0.6).unwrap(),
    )
    .unwrap();
    storage.materials = vec![
        StoredMaterial {
            bindings: TableRange::new(0, 2),
            shading: ShadingModel::PbrMetallicRoughness,
            alpha: AlphaMode::mask(-0.0).unwrap(),
            double_sided: true,
            factors,
            name: Some(name0),
        },
        StoredMaterial {
            bindings: TableRange::new(2, 2),
            shading: ShadingModel::PbrMetallicRoughness,
            alpha: AlphaMode::Opaque,
            double_sided: false,
            factors: MaterialFactors::try_new(
                Vec3::ZERO,
                Some(PbrMetallicRoughnessFactors::default()),
                None,
                None,
            )
            .unwrap(),
            name: None,
        },
        StoredMaterial {
            bindings: TableRange::new(4, 0),
            shading: ShadingModel::Unspecified,
            alpha: AlphaMode::Blend,
            double_sided: false,
            factors: MaterialFactors::empty(),
            name: Some(name2),
        },
    ];
    let counts = AppearanceMeteredCounts::for_storage(&storage).unwrap();
    Fixture { storage, counts }
}

#[test]
fn actual_rows_and_pools_validate_without_allocation_or_projection_with_exact_work() {
    let fixture = fixture();
    // 2 fixed + 2 images + 2 samplers + 2 textures + 4 bindings + 3 materials + 40 text.
    assert_eq!(fixture.counts.validation_work(), Some(55));
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            AppearanceMeteredCounts::for_storage(&fixture.storage),
            Some(fixture.counts)
        );
        assert!(fixture.validate(&mut control).unwrap());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(control.completed, 55);
    assert_eq!(fixture.storage.image_bytes, [0, 255, 17, 128, 42, 0, 1]);
    assert_eq!(
        fixture.storage.bindings[0].channels.as_slice(),
        [
            TextureChannel::Blue,
            TextureChannel::Red,
            TextureChannel::Green,
            TextureChannel::Alpha
        ]
    );
    assert_eq!(
        fixture.storage.bindings[1].strength.get().to_bits(),
        (-2.0_f32).to_bits()
    );
    assert_eq!(
        fixture.storage.bindings[1].transform.offset().x.to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        fixture.storage.materials[0]
            .alpha
            .mask_cutoff()
            .unwrap()
            .to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        fixture.storage.materials[0].factors.emissive().y.to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        fixture.storage.materials[0]
            .factors
            .legacy()
            .unwrap()
            .diffuse()
            .x,
        3.0
    );
    assert_eq!(
        fixture.storage.materials[0]
            .factors
            .supplemental_pbr()
            .thickness(),
        5.0
    );
    assert_eq!(
        fixture.storage.materials[0]
            .name
            .unwrap()
            .get(&fixture.storage.text),
        Some("\u{3bb}mat")
    );
    assert!(fixture.storage.materials[1].name.is_none());
    assert_eq!(
        fixture.storage.materials[2]
            .name
            .unwrap()
            .get(&fixture.storage.text),
        Some("")
    );
}

type Corruption = fn(&mut Fixture);

fn reject_cases(cases: &[(&str, Corruption)]) {
    for &(label, corrupt) in cases {
        let mut fixture = fixture();
        corrupt(&mut fixture);
        assert!(
            !fixture.validate(&mut Control::default()).unwrap(),
            "{label}"
        );
    }
}

#[test]
fn opaque_image_contents_require_only_bounded_range_validation() {
    for size in [1, 1024 * 1024] {
        let mut storage = empty();
        storage.image_bytes = vec![0xff; size];
        storage.images.push(StoredImage {
            payload: TableRange::new(0, size),
            media_type: None,
        });
        let counts = AppearanceMeteredCounts::for_storage(&storage).unwrap();
        assert_eq!(counts.image_byte_count, size as u64);
        assert_eq!(counts.validation_work(), Some(3));
        let mut control = Control {
            reject_before: Some(3),
            ..Control::default()
        };
        assert!(storage.validate(&mut control, counts).unwrap());
        assert_eq!(control.completed, 3);
    }
}

#[test]
fn sealed_counts_compact_ranges_and_both_pool_sections_require_complete_ownership() {
    reject_cases(&[
        ("sealed count", |f| f.counts.binding_count -= 1),
        ("payload overlap", |f| {
            f.storage.images[1].payload = TableRange::new(3, 3)
        }),
        ("empty image", |f| {
            f.storage.images[0].payload = TableRange::new(0, 0)
        }),
        ("payload overflow", |f| {
            f.storage.images[0].payload = TableRange::new(usize::MAX, 4)
        }),
        ("oversized image", |f| {
            f.storage.images[0].payload = TableRange::new(0, MAX_ENCODED_IMAGE_BYTES + 1)
        }),
        ("media gap", |f| {
            f.storage.images[1].media_type = Some(TextRange(TableRange::new(10, 9)))
        }),
        ("binding owner overlap", |f| {
            f.storage.materials[1].bindings = TableRange::new(1, 2)
        }),
        ("binding owner overflow", |f| {
            f.storage.materials[0].bindings = TableRange::new(usize::MAX, 2)
        }),
        ("unowned binding", |f| {
            f.storage.materials[1].bindings = TableRange::new(2, 1)
        }),
        ("source key overlap", |f| {
            f.storage.bindings[3].source_key = f.storage.bindings[2].source_key
        }),
        ("name overlaps media section", |f| {
            f.storage.materials[0].name = f.storage.images[0].media_type
        }),
        ("name overlaps source section", |f| {
            f.storage.materials[0].name = f.storage.bindings[0].source_key
        }),
        ("unowned image byte", |f| {
            f.storage.image_bytes.push(1);
            f.recount();
        }),
        ("unowned text suffix", |f| {
            f.storage.text.push(b'x');
            f.recount();
        }),
    ]);
}

#[test]
fn real_text_ranges_preserve_spelling_but_reject_invalid_media_keys_and_names() {
    reject_cases(&[
        ("empty media", |f| {
            f.storage.images[0].media_type = Some(TextRange(TableRange::new(0, 0)))
        }),
        ("media parameters", |f| f.storage.text[5] = b';'),
        ("uppercase media", |f| f.storage.text[0] = b'I'),
        ("non UTF8 media", |f| f.storage.text[0] = 255),
        ("oversized media", |f| {
            f.storage.images[0].media_type = Some(TextRange(TableRange::new(
                0,
                MAX_EXTENSION_MEDIA_TYPE_BYTES + 1,
            )))
        }),
        ("empty source key", |f| {
            let start = f.storage.bindings[0].source_key.unwrap().0.start();
            f.storage.bindings[0].source_key = Some(TextRange(TableRange::new(start, 0)));
        }),
        ("source key control", |f| {
            let start = f.storage.bindings[0].source_key.unwrap().0.start();
            f.storage.text[start] = b'\n';
        }),
        ("source key UTF8 split", |f| {
            let range = f.storage.bindings[2].source_key.unwrap().0;
            f.storage.bindings[2].source_key = Some(TextRange(TableRange::new(range.start(), 1)));
        }),
        ("oversized source key", |f| {
            let start = f.storage.bindings[0].source_key.unwrap().0.start();
            f.storage.bindings[0].source_key = Some(TextRange(TableRange::new(start, 129)));
        }),
        ("name control", |f| {
            let start = f.storage.materials[0].name.unwrap().0.start();
            f.storage.text[start + 2] = b'\t';
        }),
        ("name UTF8 split", |f| {
            let range = f.storage.materials[0].name.unwrap().0;
            f.storage.materials[0].name = Some(TextRange(TableRange::new(range.start(), 1)));
        }),
        ("oversized name", |f| {
            f.storage.materials[0].name =
                Some(TextRange(TableRange::new(0, MAX_ENTITY_NAME_BYTES + 1)))
        }),
    ]);
}

#[test]
fn actual_references_factors_swizzles_and_cross_binding_authority_fail_closed() {
    reject_cases(&[
        ("image reference", |f| {
            f.storage.textures[0].image = StoredImageIndex(2)
        }),
        ("sampler reference", |f| {
            f.storage.textures[0].sampler = StoredSamplerIndex(u32::MAX)
        }),
        ("texture reference", |f| {
            f.storage.bindings[0].texture = StoredTextureIndex(2)
        }),
        ("required factor group", |f| {
            f.storage.materials[1].shading = ShadingModel::Phong
        }),
        ("unselected retained numeric group", |f| {
            f.storage.materials[0].factors.corrupt_emissive_for_test()
        }),
        ("invalid swizzle zero lanes", |f| {
            f.storage.bindings[0]
                .channels
                .corrupt_lane_count_for_test(0)
        }),
        ("invalid swizzle five lanes", |f| {
            f.storage.bindings[0]
                .channels
                .corrupt_lane_count_for_test(5)
        }),
        ("role versus lanes", |f| {
            f.storage.bindings[1].channels = TextureChannelSwizzle::RGBA
        }),
        ("role versus color", |f| {
            f.storage.bindings[1].color = ColorInterpretation::SrgbColor
        }),
        ("role versus strength", |f| {
            f.storage.bindings[2].strength = TextureStrength::try_new(0.5).unwrap()
        }),
        ("role versus blend", |f| {
            f.storage.bindings[0].blend = TextureBlendOperation::Multiply
        }),
        ("duplicate role despite distinct source keys", |f| {
            f.storage.bindings[3].role = TextureRole::Metallic
        }),
        ("combined then separate authority", |f| {
            f.storage.bindings[2].role = TextureRole::MetallicRoughness;
            f.storage.bindings[2].channels =
                TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green])
                    .unwrap();
        }),
        ("separate then combined authority", |f| {
            f.storage.bindings[3].role = TextureRole::MetallicRoughness;
            f.storage.bindings[3].channels =
                TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green])
                    .unwrap();
        }),
    ]);
}

#[test]
fn duplicate_authority_resets_per_material_and_unused_sampler_rows_are_valid() {
    let mut fixture = fixture();
    fixture.storage.materials[0].bindings = TableRange::new(0, 3);
    fixture.storage.materials[1].bindings = TableRange::new(3, 1);
    fixture.storage.bindings[3].role = TextureRole::Metallic;
    fixture
        .storage
        .samplers
        .push(SamplerState::CANONICAL_DEFAULT);
    fixture.recount();
    let mut control = Control::default();
    assert!(fixture.validate(&mut control).unwrap());
    assert_eq!(control.completed, 56);
    for source in [
        TextureCoordinateSource::ObjectPosition,
        TextureCoordinateSource::ObjectNormal,
        TextureCoordinateSource::ReflectionVector,
    ] {
        fixture.storage.bindings[0].coordinate_source = source;
        assert!(fixture.validate(&mut Control::default()).unwrap());
    }
}

#[test]
fn exact_control_prefixes_include_bytes_binding_authority_and_completion() {
    let fixture = fixture();
    for prefix in [0, 1, 6, 15, 33, 39, 53, 54] {
        let mut reject = Control {
            reject_before: Some(prefix),
            ..Control::default()
        };
        assert_eq!(fixture.validate(&mut reject), Err(Interrupted(prefix)));
        assert_eq!(reject.completed, prefix);
        let mut omit = Control {
            omit_before: Some(prefix),
            ..Control::default()
        };
        assert!(!fixture.validate(&mut omit).unwrap());
        assert_eq!(omit.completed, prefix);
    }
    let mut exact = Control {
        reject_before: Some(55),
        ..Control::default()
    };
    assert!(fixture.validate(&mut exact).unwrap());
    assert_eq!(exact.completed, 55);
}

#[test]
fn empty_storage_and_explicit_empty_material_name_need_no_scratch() {
    let mut storage = empty();
    let counts = AppearanceMeteredCounts::for_storage(&storage).unwrap();
    assert_eq!(counts.validation_work(), Some(2));
    let mut control = Control::default();
    let allocation =
        allocation_counter::measure(|| assert!(storage.validate(&mut control, counts).unwrap()));
    assert_eq!(allocation.count_total, 0);
    assert_eq!(control.completed, 2);
    storage.materials.push(StoredMaterial {
        bindings: TableRange::new(0, 0),
        shading: ShadingModel::Unlit,
        alpha: AlphaMode::Opaque,
        double_sided: false,
        factors: MaterialFactors::empty(),
        name: Some(TextRange(TableRange::new(0, 0))),
    });
    let counts = AppearanceMeteredCounts::for_storage(&storage).unwrap();
    assert_eq!(counts.validation_work(), Some(3));
    let mut control = Control::default();
    assert!(storage.validate(&mut control, counts).unwrap());
    assert_eq!(control.completed, 3);
}

#[test]
fn constant_time_count_shape_rejects_overflow_without_reading_rows() {
    assert!(
        AppearanceMeteredCounts::try_new([u64::from(u32::MAX) + 1, 0, 0, 0, 0], [0, 0]).is_none()
    );
    assert!(AppearanceMeteredCounts::try_new([0; 5], [1, 0]).is_none());
    assert!(AppearanceMeteredCounts::try_new([2, 0, 0, 0, 0], [1, 0]).is_none());
    assert!(AppearanceMeteredCounts::try_new([0; 5], [0, 1]).is_none());
    let mut fixture = fixture();
    fixture.storage.materials[0].bindings = TableRange::new(usize::MAX, usize::MAX);
    assert_eq!(
        AppearanceMeteredCounts::for_storage(&fixture.storage),
        Some(fixture.counts)
    );
    assert!(!fixture.validate(&mut Control::default()).unwrap());
    #[cfg(target_pointer_width = "64")]
    {
        let counts =
            AppearanceMeteredCounts::try_new([1, 0, 0, 0, 0], [u64::MAX, u64::MAX]).unwrap();
        assert_eq!(counts.validation_work(), None);
    }
}
