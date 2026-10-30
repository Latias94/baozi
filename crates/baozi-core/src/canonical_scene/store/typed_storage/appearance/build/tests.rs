use std::convert::Infallible;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::appearance::{
    LegacyMaterialFactors, PbrMetallicRoughnessFactors, PbrSpecularGlossinessFactors,
    SupplementalPbrFactors, TextureChannel, validate_material_semantics,
    validate_texture_binding_semantics,
};
use crate::canonical_scene::store::production::{
    AppearanceAuthoringBatch, AuthoringRange, CanonicalAuthoringControl, ImageAuthoring,
    ImageLocalId, MaterialAuthoring, MaterialBindingAuthoring, ResourceIdentityLocalId,
    ResourceSnapshotAuthoring, ResourceSnapshotAuthoringTable, SamplerAuthoring, SamplerLocalId,
    TextureAuthoring, TextureLocalId, validate_appearance_batch_with_control,
};
use crate::extension::ExtensionMediaType;
use crate::image::{MipmapFilter, TexelFilter, TextureAddressMode};
use crate::math::{Color, Vec2, Vec3};
use crate::{SceneValidationLimits, SceneValidationWork};

use super::*;

struct AuthoringControl;

impl CanonicalAuthoringControl for AuthoringControl {
    type Error = Infallible;
    type Reservation = u64;

    fn poll(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn run_unit<R>(
        &mut self,
        _work: SceneValidationWork,
        mut action: impl FnMut() -> R,
    ) -> Result<R, Self::Error> {
        Ok(action())
    }

    fn reserve_scratch(&mut self, bytes: u64) -> Result<Self::Reservation, Self::Error> {
        Ok(bytes)
    }

    fn validate_scratch(&mut self, _reservation: &Self::Reservation) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut Self::Reservation,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        *reservation = required_bytes;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interrupted(usize);

#[derive(Default)]
struct Control {
    completed: usize,
    string_completed: usize,
    reject_before: Option<usize>,
    panic_before: Option<usize>,
}

impl Control {
    fn execute<R>(&mut self, string: bool, action: impl FnOnce() -> R) -> Result<R, Interrupted> {
        if self.reject_before == Some(self.completed) {
            return Err(Interrupted(self.completed));
        }
        if self.panic_before == Some(self.completed) {
            // Avoid panic-hook/output allocations in the partial-buffer allocation probe.
            std::panic::resume_unwind(Box::new(Interrupted(self.completed)));
        }
        let result = action();
        self.completed += 1;
        self.string_completed += usize::from(string);
        Ok(result)
    }
}

impl TypedStorageBuildControl for Control {
    type Error = Interrupted;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(false, action)
    }
    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(true, action)
    }
}

fn full_factors() -> MaterialFactors {
    let tiny = f32::from_bits(1);
    let metallic =
        PbrMetallicRoughnessFactors::try_new(Color::linear_rgba(-0.0, tiny, 0.5, 1.0), -0.0, 0.25)
            .unwrap();
    let specular = PbrSpecularGlossinessFactors::try_new(
        Color::linear_rgba(0.1, 0.2, 0.3, -0.0),
        Vec3::new(tiny, 0.0, -0.0),
        0.75,
    )
    .unwrap();
    let legacy = LegacyMaterialFactors::try_new(
        Vec3::new(-0.0, 0.5, 2.0),
        Vec3::new(3.0, 4.0, 5.0),
        Vec3::new(6.0, 7.0, 8.0),
        Vec3::new(9.0, 10.0, 11.0),
        128.25,
        -0.0,
    )
    .unwrap();
    let supplemental =
        SupplementalPbrFactors::try_new(-0.0, 0.125, 0.75, 3.5, Vec3::new(5.0, -0.0, tiny), 0.5)
            .unwrap();
    MaterialFactors::try_new_with_supplemental_pbr(
        Vec3::new(0.0, tiny, -0.0),
        Some(metallic),
        Some(specular),
        Some(legacy),
        supplemental,
    )
    .unwrap()
}

fn with_fixture<T>(action: impl FnOnce(&ValidatedAppearanceBatch<'_, u64>) -> T) -> T {
    let image_a = vec![0_u8, 255, 7];
    let resource_bytes = vec![42_u8, 128, 0, 99];
    let image_c = vec![3_u8, 1];
    let png = ExtensionMediaType::new("image/png").unwrap();
    let jpeg = ExtensionMediaType::new("image/jpeg").unwrap();
    let images = [
        ImageAuthoring::encoded(&image_a, Presence::Present(&png)),
        ImageAuthoring::session_resource(ResourceIdentityLocalId::new(0), Presence::Absent),
        ImageAuthoring::encoded(&image_c, Presence::Present(&jpeg)),
    ];
    let snapshots = [ResourceSnapshotAuthoring::snapshotted(&resource_bytes)];
    let sampler = SamplerState::new(
        TexelFilter::Nearest,
        TexelFilter::Linear,
        MipmapFilter::Nearest,
        TextureAddressMode::ClampToBorder,
        TextureAddressMode::ClampToEdge,
        TextureAddressMode::MirroredRepeat,
    );
    let samplers = [
        SamplerAuthoring::new(sampler),
        SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT),
    ];
    let textures = [
        TextureAuthoring::new(ImageLocalId::new(2), SamplerLocalId::new(1)),
        TextureAuthoring::new(ImageLocalId::new(0), SamplerLocalId::new(0)),
        TextureAuthoring::new(ImageLocalId::new(1), SamplerLocalId::new(0)),
    ];
    let roles = [
        TextureRole::BaseColor,
        TextureRole::Normal,
        TextureRole::Specular,
        TextureRole::Displacement,
    ];
    let coordinates = [
        TextureCoordinateSource::UvSet(u16::MAX),
        TextureCoordinateSource::ObjectNormal,
        TextureCoordinateSource::ReflectionVector,
        TextureCoordinateSource::ObjectPosition,
    ];
    let channels = [
        TextureChannelSwizzle::try_new(&[
            TextureChannel::One,
            TextureChannel::Blue,
            TextureChannel::Red,
            TextureChannel::Alpha,
        ])
        .unwrap(),
        TextureChannelSwizzle::RGB,
        TextureChannelSwizzle::RGB,
        TextureChannelSwizzle::try_new(&[TextureChannel::Zero]).unwrap(),
    ];
    let colors = [
        ColorInterpretation::SrgbColor,
        ColorInterpretation::Data,
        ColorInterpretation::LinearColor,
        ColorInterpretation::Data,
    ];
    let transform = TextureTransform::try_new(
        Vec2::new(-0.0, f32::from_bits(1)),
        -0.0,
        Vec2::new(-2.0, 0.0),
    )
    .unwrap();
    let base_key = TextureSourceKey::try_new("base:α").unwrap();
    let spec_key = TextureSourceKey::try_new("spec").unwrap();
    let keys = [Some(&base_key), None, Some(&spec_key), Some(&base_key)];
    let strengths = [0.75, -2.0, -0.0, -0.0];
    let bindings: [_; 4] = std::array::from_fn(|index| {
        MaterialBindingAuthoring::new(
            TextureLocalId::new([0, 1, 2, 0][index]),
            TextureBindingSemantics::new(
                roles[index],
                coordinates[index],
                channels[index],
                colors[index],
                if index == 0 {
                    transform
                } else {
                    TextureTransform::IDENTITY
                },
                TextureStrength::try_new(strengths[index]).unwrap(),
                TextureBlendOperation::Replace,
                keys[index],
            ),
        )
    });
    let full = full_factors();
    let legacy_only = MaterialFactors::try_new(Vec3::ZERO, None, None, full.legacy()).unwrap();
    let specular_only =
        MaterialFactors::try_new(Vec3::ZERO, None, full.pbr_specular_glossiness(), None).unwrap();
    let shadings = [
        ShadingModel::PbrMetallicRoughness,
        ShadingModel::Unlit,
        ShadingModel::Lambert,
        ShadingModel::PbrSpecularGlossiness,
        ShadingModel::Unspecified,
    ];
    let alphas = [
        AlphaMode::mask(-0.0).unwrap(),
        AlphaMode::Blend,
        AlphaMode::Opaque,
        AlphaMode::Opaque,
        AlphaMode::Opaque,
    ];
    let factors = [
        full,
        MaterialFactors::empty(),
        legacy_only,
        specular_only,
        MaterialFactors::empty(),
    ];
    let material_name = String::from("材質");
    let other_name = String::from("α");
    let names = [
        Some(material_name.as_str()),
        Some(""),
        None,
        Some(other_name.as_str()),
        None,
    ];
    let materials: [_; 5] = std::array::from_fn(|index| {
        let mut material = MaterialAuthoring::new(
            [
                AuthoringRange::new(0, 2),
                AuthoringRange::new(2, 0),
                AuthoringRange::new(2, 2),
                AuthoringRange::new(4, 0),
                AuthoringRange::new(4, 0),
            ][index],
            MaterialSemantics::new(shadings[index], alphas[index], index == 0, factors[index]),
        );
        if let Some(name) = names[index] {
            material = material.with_name(name);
        }
        material
    });
    let mut control = AuthoringControl;
    let batch = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::new(&images, &samplers, &textures, &bindings, &materials),
        ResourceSnapshotAuthoringTable::new(1, &snapshots),
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap();
    action(&batch)
}

fn vec3_bits(value: Vec3) -> [u32; 3] {
    [value.x, value.y, value.z].map(f32::to_bits)
}
fn color_bits(value: Color) -> [u32; 4] {
    [value.r, value.g, value.b, value.a].map(f32::to_bits)
}

fn assert_factor_bits(actual: MaterialFactors, expected: MaterialFactors) {
    assert_eq!(vec3_bits(actual.emissive()), vec3_bits(expected.emissive()));
    match (
        actual.pbr_metallic_roughness(),
        expected.pbr_metallic_roughness(),
    ) {
        (Some(a), Some(e)) => {
            assert_eq!(color_bits(a.base_color()), color_bits(e.base_color()));
            assert_eq!(a.metallic().to_bits(), e.metallic().to_bits());
            assert_eq!(a.roughness().to_bits(), e.roughness().to_bits());
        }
        (None, None) => {}
        _ => panic!("metallic factor presence changed"),
    }
    match (
        actual.pbr_specular_glossiness(),
        expected.pbr_specular_glossiness(),
    ) {
        (Some(a), Some(e)) => {
            assert_eq!(color_bits(a.diffuse()), color_bits(e.diffuse()));
            assert_eq!(vec3_bits(a.specular()), vec3_bits(e.specular()));
            assert_eq!(a.glossiness().to_bits(), e.glossiness().to_bits());
        }
        (None, None) => {}
        _ => panic!("specular factor presence changed"),
    }
    match (actual.legacy(), expected.legacy()) {
        (Some(a), Some(e)) => {
            assert_eq!(vec3_bits(a.ambient()), vec3_bits(e.ambient()));
            assert_eq!(vec3_bits(a.diffuse()), vec3_bits(e.diffuse()));
            assert_eq!(vec3_bits(a.specular()), vec3_bits(e.specular()));
            assert_eq!(vec3_bits(a.emissive()), vec3_bits(e.emissive()));
            assert_eq!(a.shininess().to_bits(), e.shininess().to_bits());
            assert_eq!(a.opacity().to_bits(), e.opacity().to_bits());
        }
        (None, None) => {}
        _ => panic!("legacy factor presence changed"),
    }
    let a = actual.supplemental_pbr();
    let e = expected.supplemental_pbr();
    assert_eq!(vec3_bits(a.sheen_color()), vec3_bits(e.sheen_color()));
    assert_eq!(
        [
            a.clearcoat(),
            a.clearcoat_roughness(),
            a.transmission(),
            a.thickness(),
            a.sheen_roughness()
        ]
        .map(f32::to_bits),
        [
            e.clearcoat(),
            e.clearcoat_roughness(),
            e.transmission(),
            e.thickness(),
            e.sheen_roughness()
        ]
        .map(f32::to_bits)
    );
}

#[test]
fn build_owns_resolved_images_and_compact_text_in_source_order_with_exact_work() {
    let (storage, plan, control) = with_fixture(|batch| {
        let plan = AppearanceStoragePlan::new(batch).unwrap();
        assert_eq!(plan.row_counts(), [3, 2, 3, 4, 5]);
        assert_eq!(plan.payload_counts(), [9, 45]);
        assert_eq!(plan.work().unwrap(), 71);
        let mut control = Control::default();
        let storage = plan.build(batch, &mut control).unwrap();
        (storage, plan, control)
    });
    // Every authoring buffer, session-resource snapshot, and bounded text value
    // has left scope. The following slices borrow only the new retained value.
    assert_eq!(control.completed, 71);
    assert_eq!(control.string_completed, plan.name_bytes());
    assert_eq!(storage.image_bytes, [0, 255, 7, 42, 128, 0, 99, 3, 1]);
    assert_eq!(storage.images[0].payload, TableRange::new(0, 3));
    assert_eq!(storage.images[1].payload, TableRange::new(3, 4));
    assert_eq!(storage.images[2].payload, TableRange::new(7, 2));
    assert_eq!(
        storage.images[1].payload.get(&storage.image_bytes).unwrap(),
        [42, 128, 0, 99]
    );
    assert_eq!(
        storage.text,
        "image/pngimage/jpegbase:αspecbase:α材質α".as_bytes()
    );
    assert_eq!(
        storage.images[0].media_type.unwrap().get(&storage.text),
        Some("image/png")
    );
    assert!(storage.images[1].media_type.is_none());
    assert_eq!(
        storage.images[2].media_type.unwrap().get(&storage.text),
        Some("image/jpeg")
    );
    assert_eq!(
        storage.bindings[0].source_key,
        Some(TextRange(TableRange::new(19, 7)))
    );
    assert!(storage.bindings[1].source_key.is_none());
    assert_eq!(
        storage.bindings[2].source_key,
        Some(TextRange(TableRange::new(26, 4)))
    );
    assert_eq!(
        storage.bindings[3].source_key,
        Some(TextRange(TableRange::new(30, 7)))
    );
    assert_eq!(
        storage.materials[0].name.unwrap().get(&storage.text),
        Some("材質")
    );
    assert_eq!(
        storage.materials[1].name,
        Some(TextRange(TableRange::new(43, 0)))
    );
    assert!(storage.materials[2].name.is_none());
    assert_eq!(
        storage.materials[3].name.unwrap().get(&storage.text),
        Some("α")
    );
    assert!(storage.materials[4].name.is_none());
    assert_eq!(
        storage.samplers[0],
        SamplerState::new(
            TexelFilter::Nearest,
            TexelFilter::Linear,
            MipmapFilter::Nearest,
            TextureAddressMode::ClampToBorder,
            TextureAddressMode::ClampToEdge,
            TextureAddressMode::MirroredRepeat,
        )
    );
    assert_eq!(storage.samplers[1], SamplerState::CANONICAL_DEFAULT);
    for (row, (image, sampler)) in storage.textures.iter().zip([(2, 1), (0, 0), (1, 0)]) {
        assert_eq!(row.image, StoredImageIndex(image));
        assert_eq!(row.sampler, StoredSamplerIndex(sampler));
    }
    for (row, (texture, role, coordinate, strength, color)) in storage.bindings.iter().zip([
        (
            0,
            TextureRole::BaseColor,
            TextureCoordinateSource::UvSet(u16::MAX),
            0.75_f32,
            ColorInterpretation::SrgbColor,
        ),
        (
            1,
            TextureRole::Normal,
            TextureCoordinateSource::ObjectNormal,
            -2.0,
            ColorInterpretation::Data,
        ),
        (
            2,
            TextureRole::Specular,
            TextureCoordinateSource::ReflectionVector,
            -0.0,
            ColorInterpretation::LinearColor,
        ),
        (
            0,
            TextureRole::Displacement,
            TextureCoordinateSource::ObjectPosition,
            -0.0,
            ColorInterpretation::Data,
        ),
    ]) {
        assert_eq!(row.texture, StoredTextureIndex(texture));
        assert_eq!(row.role, role);
        assert_eq!(row.coordinate_source, coordinate);
        assert_eq!(row.strength.get().to_bits(), strength.to_bits());
        assert_eq!(row.color, color);
        assert_eq!(row.blend, TextureBlendOperation::Replace);
    }
    assert_eq!(
        storage.bindings[0].channels.as_slice(),
        [
            TextureChannel::One,
            TextureChannel::Blue,
            TextureChannel::Red,
            TextureChannel::Alpha
        ]
    );
    assert_eq!(
        storage.bindings[3].channels.as_slice(),
        [TextureChannel::Zero]
    );
    let transform = storage.bindings[0].transform;
    assert_eq!(
        [
            transform.offset().x,
            transform.offset().y,
            transform.rotation_radians(),
            transform.scale().x,
            transform.scale().y
        ]
        .map(f32::to_bits),
        [
            (-0.0_f32).to_bits(),
            1,
            (-0.0_f32).to_bits(),
            (-2.0_f32).to_bits(),
            0
        ]
    );
    for (row, (range, shading)) in storage.materials.iter().zip([
        (TableRange::new(0, 2), ShadingModel::PbrMetallicRoughness),
        (TableRange::new(2, 0), ShadingModel::Unlit),
        (TableRange::new(2, 2), ShadingModel::Lambert),
        (TableRange::new(4, 0), ShadingModel::PbrSpecularGlossiness),
        (TableRange::new(4, 0), ShadingModel::Unspecified),
    ]) {
        assert_eq!(row.bindings, range);
        assert_eq!(row.shading, shading);
        assert!(validate_material_semantics(row.semantics()).is_ok());
    }
    assert!(storage.materials[0].double_sided);
    assert!(!storage.materials[1].double_sided);
    assert_eq!(
        storage.materials[0].alpha.mask_cutoff().unwrap().to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(storage.materials[1].alpha, AlphaMode::Blend);
    assert_eq!(storage.materials[2].alpha, AlphaMode::Opaque);
    assert_factor_bits(storage.materials[0].factors, full_factors());
    assert_factor_bits(storage.materials[1].factors, MaterialFactors::empty());
    assert_factor_bits(
        storage.materials[2].factors,
        MaterialFactors::try_new(Vec3::ZERO, None, None, full_factors().legacy()).unwrap(),
    );
    assert_factor_bits(
        storage.materials[3].factors,
        MaterialFactors::try_new(
            Vec3::ZERO,
            None,
            full_factors().pbr_specular_glossiness(),
            None,
        )
        .unwrap(),
    );
    for row in &storage.bindings {
        let key = row
            .source_key
            .map(|range| TextureSourceKey::try_new(range.get(&storage.text).unwrap()).unwrap());
        assert!(validate_texture_binding_semantics(row.semantics(key.as_ref())).is_ok());
    }
}

#[test]
fn each_row_byte_and_string_action_releases_partial_buffers_on_error_and_unwind() {
    with_fixture(|batch| {
        let plan = AppearanceStoragePlan::new(batch).unwrap();
        for before in 0..usize::try_from(plan.work().unwrap()).unwrap() {
            let mut control = Control {
                reject_before: Some(before),
                ..Control::default()
            };
            let allocations = allocation_counter::measure(|| {
                assert!(matches!(plan.build(batch, &mut control),
                    Err(TypedStorageBuildError::Control(Interrupted(completed))) if completed == before));
            });
            assert_eq!(control.completed, before);
            assert_eq!(allocations.count_current, 0);
            assert_eq!(allocations.bytes_current, 0);
            let mut control = Control {
                panic_before: Some(before),
                ..Control::default()
            };
            let allocations = allocation_counter::measure(|| {
                let result = catch_unwind(AssertUnwindSafe(|| plan.build(batch, &mut control)));
                assert!(result.is_err());
                drop(result);
            });
            assert_eq!(control.completed, before);
            assert_eq!(allocations.count_current, 0);
            assert_eq!(allocations.bytes_current, 0);
        }
    });
}

#[test]
fn initialized_footprint_counts_inline_fields_once_and_excludes_spare_capacity() {
    with_fixture(|batch| {
        let plan = AppearanceStoragePlan::new(batch).unwrap();
        let expected = (std::mem::size_of::<AppearanceStorage>()
            + 3 * std::mem::size_of::<StoredImage>()
            + 2 * std::mem::size_of::<SamplerState>()
            + 3 * std::mem::size_of::<StoredTexture>()
            + 4 * std::mem::size_of::<StoredBinding>()
            + 5 * std::mem::size_of::<StoredMaterial>()
            + 9
            + 45) as u64;
        assert_eq!(plan.canonical_bytes().unwrap(), expected);
        let mut storage = plan.build(batch, Control::default()).unwrap();
        storage.images.reserve(100);
        storage.materials.reserve(20);
        storage.image_bytes.reserve(200);
        storage.text.reserve(400);
        assert_eq!(
            AppearanceStoragePlan::canonical_bytes_for_shape(
                [
                    storage.images.len() as u64,
                    storage.samplers.len() as u64,
                    storage.textures.len() as u64,
                    storage.bindings.len() as u64,
                    storage.materials.len() as u64
                ],
                [storage.image_bytes.len() as u64, storage.text.len() as u64],
            )
            .unwrap(),
            expected
        );
    });
}

#[test]
fn empty_owner_has_seven_empty_capacities_and_zero_materialization_allocations_or_work() {
    let mut authoring = AuthoringControl;
    let batch = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::empty(),
        ResourceSnapshotAuthoringTable::empty(),
        SceneValidationLimits::default(),
        &mut authoring,
    )
    .unwrap();
    let plan = AppearanceStoragePlan::new(&batch).unwrap();
    assert_eq!(plan.row_counts(), [0; 5]);
    assert_eq!(plan.payload_counts(), [0; 2]);
    assert_eq!(
        plan.capacities()
            .unwrap()
            .map(|capacity| capacity.desired_capacity()),
        [0; 7]
    );
    assert_eq!(plan.name_bytes(), 0);
    assert_eq!(plan.work().unwrap(), 0);
    assert_eq!(
        plan.canonical_bytes().unwrap(),
        std::mem::size_of::<AppearanceStorage>() as u64
    );
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        let storage = plan.build(&batch, &mut control).unwrap();
        assert!(
            storage.images.is_empty() && storage.samplers.is_empty() && storage.textures.is_empty()
        );
        assert!(storage.bindings.is_empty() && storage.materials.is_empty());
        assert!(storage.image_bytes.is_empty() && storage.text.is_empty());
    });
    assert_eq!(control.completed, 0);
    assert_eq!(allocations.count_total, 0);
}

#[test]
fn checked_shape_is_lean_and_stale_plan_fails_before_allocation() {
    assert!(
        std::mem::size_of::<AppearanceStoragePlan>() < std::mem::size_of::<[CapacityPlan; 7]>()
    );
    assert!(matches!(
        AppearanceStoragePlan::for_shape([u64::from(u32::MAX) + 1, 0, 0, 0, 0], [0; 2]),
        Err(StorePlanError::SizeOverflow)
    ));
    assert!(AppearanceStoragePlan::for_shape([0; 5], [u64::MAX; 2]).is_err());
    assert!(matches!(
        AppearanceStoragePlan::canonical_bytes_for_shape([u64::MAX; 5], [u64::MAX; 2]),
        Err(StorePlanError::SizeOverflow)
    ));
    with_fixture(|batch| {
        let original = AppearanceStoragePlan::new(batch).unwrap();
        let incorrect =
            AppearanceStoragePlan::for_shape([3, 2, 3, 3, 5], original.payload_counts()).unwrap();
        let mut control = Control::default();
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                incorrect.build(batch, &mut control),
                Err(TypedStorageBuildError::Invariant)
            ));
        });
        assert_eq!(control.completed, 0);
        assert_eq!(allocations.count_total, 0);
    });
}
