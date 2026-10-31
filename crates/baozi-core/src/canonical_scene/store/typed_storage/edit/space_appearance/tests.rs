use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::appearance::{
    ColorInterpretation, LegacyMaterialFactors, PbrMetallicRoughnessFactors,
    PbrSpecularGlossinessFactors, ShadingModel, TextureBlendOperation, TextureChannelSwizzle,
    TextureTransform,
};
use crate::canonical_scene::store::typed_storage::appearance::{
    AppearancePayloads, StoredImage, StoredImageIndex, StoredSamplerIndex, StoredTexture,
    StoredTextureIndex,
};
use crate::canonical_scene::store::typed_storage::{TableRange, TextRange};
use crate::image::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};
use crate::math::{Color, Vec2, Vec3};
use crate::scene_space::{Axis, FrontFaceWinding, Handedness, SceneSpace};
use crate::space_conversion::TargetSpace;

use super::*;

// Counted independently of AppearanceStoragePlan: 2 images + 2 samplers + 2 textures
// + 5 bindings + 3 materials. Preflight visits only the 5 bindings and 3 materials.
const FIXTURE_ROW_WORK: u64 = 14;
const FIXTURE_PREFLIGHT_WORK: u64 = 8;

#[derive(Default)]
struct Control {
    completed: u64,
    attempted: u64,
    strings: u64,
    stop: Option<u64>,
    omit: Option<u64>,
    panic_before: Option<u64>,
    panic_after: Option<u64>,
}

impl Control {
    fn execute<R>(&mut self, string: bool, action: impl FnOnce() -> R) -> Result<R, u64> {
        let index = self.completed;
        self.attempted += 1;
        if self.stop == Some(index) {
            return Err(index);
        }
        if self.panic_before == Some(index) {
            // Avoid panic-hook allocations in the local-buffer release measurement.
            std::panic::resume_unwind(Box::new(index));
        }
        let result = action();
        self.completed += 1;
        self.strings += u64::from(string);
        if self.panic_after == Some(index) {
            std::panic::resume_unwind(Box::new(index));
        }
        Ok(result)
    }
}

impl TypedStorageBuildControl for Control {
    type Error = u64;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(false, action)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.execute(true, action)
    }
}

impl TypedStorageValidationControl for Control {
    type Error = u64;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        if self.omit == Some(self.completed) {
            self.attempted += 1;
            return Ok(None);
        }
        self.execute(false, action).map(Some)
    }
}

fn conversion(unit_scale: f64, reflected: bool, winding: FrontFaceWinding) -> SpaceConversion {
    let target = TargetSpace::try_new(
        if reflected {
            Handedness::Left
        } else {
            Handedness::Right
        },
        if reflected {
            Axis::PositiveZ
        } else {
            Axis::PositiveY
        },
        if reflected {
            Axis::PositiveY
        } else {
            Axis::NegativeZ
        },
        unit_scale,
        winding,
    )
    .unwrap();
    SpaceConversion::try_new(&SceneSpace::STANDARD_TARGET, target, None).unwrap()
}

fn units(unit_scale: f64) -> SpaceConversion {
    conversion(unit_scale, false, FrontFaceWinding::CounterClockwise)
}

fn factors(thickness: f32) -> MaterialFactors {
    MaterialFactors::try_new_with_supplemental_pbr(
        Vec3::new(-0.0, 2.0, f32::from_bits(1)),
        Some(
            PbrMetallicRoughnessFactors::try_new(
                Color::linear_rgba(-0.0, 0.125, 0.5, 1.0),
                -0.0,
                0.75,
            )
            .unwrap(),
        ),
        Some(
            PbrSpecularGlossinessFactors::try_new(
                Color::linear_rgba(1.0, -0.0, 0.5, 0.25),
                Vec3::new(-0.0, 1.0, 0.125),
                f32::from_bits(0x3e80_0001),
            )
            .unwrap(),
        ),
        Some(
            LegacyMaterialFactors::try_new(
                Vec3::new(-0.0, 1.0, 2.0),
                Vec3::new(0.25, -0.0, 3.0),
                Vec3::new(4.0, 0.5, -0.0),
                Vec3::new(0.0, -0.0, 7.0),
                -0.0,
                -0.0,
            )
            .unwrap(),
        ),
        SupplementalPbrFactors::try_new(
            -0.0,
            0.125,
            0.25,
            thickness,
            Vec3::new(-0.0, 1.5, f32::from_bits(1)),
            0.5,
        )
        .unwrap(),
    )
    .unwrap()
}

fn material(factors: MaterialFactors) -> StoredMaterial {
    StoredMaterial {
        bindings: TableRange::new(0, 0),
        shading: ShadingModel::Unspecified,
        alpha: AlphaMode::mask(-0.0).unwrap(),
        double_sided: true,
        factors,
        name: None,
    }
}

fn binding(role: TextureRole, strength: f32) -> StoredBinding {
    StoredBinding {
        texture: StoredTextureIndex(0),
        role,
        uv_set: 23,
        channels: if matches!(role, TextureRole::Normal | TextureRole::ClearcoatNormal) {
            TextureChannelSwizzle::RGB
        } else {
            TextureChannelSwizzle::RED
        },
        color: ColorInterpretation::Data,
        transform: TextureTransform::try_new(Vec2::new(-0.0, 0.25), -0.0, Vec2::new(1.0, -2.0))
            .unwrap(),
        strength: TextureStrength::try_new(strength).unwrap(),
        blend: TextureBlendOperation::Replace,
        source_key: None,
    }
}

fn fixture() -> AppearanceRows {
    AppearanceRows {
        images: vec![
            StoredImage {
                payload: TableRange::new(0, 3),
                media_type: Some(TextRange(TableRange::new(0, 9))),
            },
            StoredImage {
                payload: TableRange::new(3, 4),
                media_type: None,
            },
        ],
        samplers: vec![
            SamplerState::CANONICAL_DEFAULT,
            SamplerState::new(
                TexelFilter::Nearest,
                TexelFilter::Linear,
                MipmapFilter::None,
                TextureAddressMode::ClampToEdge,
                TextureAddressMode::MirroredRepeat,
                TextureAddressMode::ClampToBorder,
            ),
        ],
        textures: vec![
            StoredTexture {
                image: StoredImageIndex(1),
                sampler: StoredSamplerIndex(0),
            },
            StoredTexture {
                image: StoredImageIndex(0),
                sampler: StoredSamplerIndex(1),
            },
        ],
        bindings: vec![
            StoredBinding {
                source_key: Some(TextRange(TableRange::new(9, 11))),
                ..binding(TextureRole::Displacement, -0.75)
            },
            binding(TextureRole::Normal, -0.125),
            binding(TextureRole::Thickness, 1.0),
            StoredBinding {
                texture: StoredTextureIndex(1),
                uv_set: u16::MAX,
                ..binding(TextureRole::ClearcoatNormal, 0.25)
            },
            binding(TextureRole::Displacement, -0.0),
        ],
        materials: vec![
            StoredMaterial {
                bindings: TableRange::new(0, 4),
                shading: ShadingModel::PbrMetallicRoughness,
                name: Some(TextRange(TableRange::new(20, 7))),
                ..material(factors(1.25))
            },
            StoredMaterial {
                bindings: TableRange::new(4, 1),
                name: Some(TextRange(TableRange::new(27, 0))),
                ..material(
                    MaterialFactors::try_new_with_supplemental_pbr(
                        Vec3::ZERO,
                        None,
                        None,
                        None,
                        SupplementalPbrFactors::try_new(0.0, 0.0, 0.0, -0.0, Vec3::ZERO, 0.0)
                            .unwrap(),
                    )
                    .unwrap(),
                )
            },
            StoredMaterial {
                bindings: TableRange::new(5, 0),
                ..material(MaterialFactors::empty())
            },
        ],
    }
}

fn shape(rows: &AppearanceRows) -> AppearanceStoragePlan {
    // Raw values exercise the concrete row builder, without constructing a fake retained owner.
    AppearanceStoragePlan::for_storage(
        rows,
        &AppearancePayloads {
            image_bytes: vec![1, 2, 3, 4, 5, 6, 7],
            text: b"image/pngtest:heightsurface".to_vec(),
        },
    )
    .unwrap()
}

#[test]
fn reflected_conversion_scales_only_thickness_and_displacement_with_exact_row_work() {
    let source = fixture();
    let row_counts = [
        source.images.len(),
        source.samplers.len(),
        source.textures.len(),
        source.bindings.len(),
        source.materials.len(),
    ];
    assert_eq!(row_counts, [2, 2, 2, 5, 3]);
    assert_eq!(
        row_counts.into_iter().sum::<usize>() as u64,
        FIXTURE_ROW_WORK
    );
    assert_eq!(
        (source.bindings.len() + source.materials.len()) as u64,
        FIXTURE_PREFLIGHT_WORK
    );
    let conversion = conversion(0.5, true, FrontFaceWinding::CounterClockwise);
    assert_eq!(conversion.determinant_sign(), -1);
    let mut preflight = Control::default();
    assert!(preflight_rows(&source, conversion, &mut preflight).unwrap());
    assert_eq!(
        (preflight.completed, preflight.strings),
        (FIXTURE_PREFLIGHT_WORK, 0)
    );

    let shape = shape(&source);
    let capacities = shape.rows_capacities().unwrap();
    assert_eq!(
        capacities.map(|plan| plan.desired_capacity()),
        [2, 2, 2, 5, 3]
    );
    assert_eq!(shape.rows_work().unwrap(), FIXTURE_ROW_WORK);
    let mut build = Control::default();
    let actual = materialize_rows(&source, conversion, capacities, &mut build).unwrap();
    assert_eq!((build.completed, build.strings), (FIXTURE_ROW_WORK, 0));
    assert!(same_material(
        actual.materials[0],
        StoredMaterial {
            factors: factors(2.5),
            ..source.materials[0]
        }
    ));
    assert!(same_material(source.materials[1], actual.materials[1]));
    assert!(same_material(source.materials[2], actual.materials[2]));
    assert_eq!(
        actual.bindings[0].strength.get().to_bits(),
        (-1.5_f32).to_bits()
    );
    for index in 1..5 {
        assert!(same_binding(source.bindings[index], actual.bindings[index]));
    }
    assert_eq!(
        actual.materials[1]
            .factors
            .supplemental_pbr()
            .thickness()
            .to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        actual.bindings[4].strength.get().to_bits(),
        (-0.0_f32).to_bits()
    );
    let mut correspondence = Control::default();
    assert!(
        validate_rows_correspondence(&source, &actual, conversion, &mut correspondence).unwrap()
    );
    assert_eq!(
        (correspondence.completed, correspondence.strings),
        (FIXTURE_ROW_WORK, 0)
    );
}

#[test]
fn selectors_follow_actual_bits_for_axes_winding_rounding_and_zero_lengths() {
    let source = fixture();
    for conversion in [
        units(1.0),
        conversion(1.0, true, FrontFaceWinding::CounterClockwise),
        conversion(1.0, false, FrontFaceWinding::Clockwise),
        units(1.0 + f64::EPSILON),
    ] {
        let mut control = Control::default();
        assert!(!preflight_rows(&source, conversion, &mut control).unwrap());
        assert_eq!(control.completed, FIXTURE_PREFLIGHT_WORK);
        for index in 0..source.materials.len() {
            assert!(!material_changes(&source, index, conversion));
        }
        for index in 0..source.bindings.len() {
            assert!(!binding_changes(&source, index, conversion));
        }
    }
    assert!(material_changes(&source, 0, units(0.5)));
    assert!(!material_changes(&source, 1, units(0.5)));
    assert!(!material_changes(&source, 2, units(0.5)));
    assert!(binding_changes(&source, 0, units(0.5)));
    for index in 1..source.bindings.len() {
        assert!(!binding_changes(&source, index, units(0.5)));
    }
    assert!(!material_changes(&source, usize::MAX, units(0.5)));
    assert!(!binding_changes(&source, usize::MAX, units(0.5)));
}

#[test]
fn explicit_zero_factor_groups_retain_presence_distinct_from_absence() {
    let black = Color::linear_rgba(0.0, 0.0, 0.0, 0.0);
    for factors in [
        MaterialFactors::try_new(
            Vec3::ZERO,
            Some(PbrMetallicRoughnessFactors::try_new(black, 0.0, 0.0).unwrap()),
            None,
            None,
        )
        .unwrap(),
        MaterialFactors::try_new(
            Vec3::ZERO,
            None,
            Some(PbrSpecularGlossinessFactors::try_new(black, Vec3::ZERO, 0.0).unwrap()),
            None,
        )
        .unwrap(),
        MaterialFactors::try_new(
            Vec3::ZERO,
            None,
            None,
            Some(
                LegacyMaterialFactors::try_new(
                    Vec3::ZERO,
                    Vec3::ZERO,
                    Vec3::ZERO,
                    Vec3::ZERO,
                    0.0,
                    0.0,
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    ] {
        let source = material(factors);
        let actual = convert_material(source, units(2.0)).unwrap();
        assert!(same_material(source, actual));
        assert!(!same_factors(MaterialFactors::empty(), actual.factors));
    }
}

#[test]
fn later_numeric_failures_report_exact_material_or_displacement_row() {
    for (value, conversion, expected) in [
        (
            f32::MAX,
            units(0.5),
            NumericConversionError::NonFiniteOutput,
        ),
        (
            f32::from_bits(1),
            units(2.0),
            NumericConversionError::UnderflowToZero,
        ),
    ] {
        let mut source = fixture();
        source.bindings[4].strength = TextureStrength::try_new(-value).unwrap();
        let mut control = Control::default();
        assert!(matches!(
            preflight_rows(&source, conversion, &mut control),
            Err(AppearanceSpacePlanError::NumericDisplacement { binding: 4, source })
                if source == expected
        ));
        assert_eq!(control.completed, 5);

        let mut source = fixture();
        source.materials[2].factors = factors(value);
        let mut control = Control::default();
        assert!(matches!(
            preflight_rows(&source, conversion, &mut control),
            Err(AppearanceSpacePlanError::NumericMaterial { material: 2, source })
                if source == expected
        ));
        assert_eq!(control.completed, 8);
    }
}

#[test]
fn every_preflight_action_remains_interruptible_after_the_first_changed_row() {
    let source = fixture();
    for stop in 0..FIXTURE_PREFLIGHT_WORK {
        let mut control = Control {
            stop: Some(stop),
            ..Control::default()
        };
        assert!(matches!(
            preflight_rows(&source, units(0.5), &mut control),
            Err(AppearanceSpacePlanError::Control(actual)) if actual == stop
        ));
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        assert_eq!(control.strings, 0);
    }
    let mut control = Control {
        stop: Some(FIXTURE_PREFLIGHT_WORK),
        ..Control::default()
    };
    assert!(preflight_rows(&source, units(0.5), &mut control).unwrap());
    assert_eq!(control.completed, FIXTURE_PREFLIGHT_WORK);
    assert_eq!(control.attempted, FIXTURE_PREFLIGHT_WORK);
}

#[test]
fn correspondence_rejects_all_table_shapes_ranges_sampler_state_and_texture_references() {
    let source = fixture();
    let capacities = shape(&source).rows_capacities().unwrap();
    for table in 0..5 {
        for extra in [false, true] {
            let mut actual =
                materialize_rows(&source, units(0.5), capacities, &mut Control::default()).unwrap();
            match table {
                0 if extra => actual.images.push(source.images[0]),
                0 => {
                    actual.images.pop();
                }
                1 if extra => actual.samplers.push(source.samplers[0]),
                1 => {
                    actual.samplers.pop();
                }
                2 if extra => actual.textures.push(source.textures[0]),
                2 => {
                    actual.textures.pop();
                }
                3 if extra => actual.bindings.push(source.bindings[0]),
                3 => {
                    actual.bindings.pop();
                }
                4 if extra => actual.materials.push(source.materials[0]),
                4 => {
                    actual.materials.pop();
                }
                _ => unreachable!(),
            }
            let mut control = Control::default();
            assert!(
                !validate_rows_correspondence(&source, &actual, units(0.5), &mut control).unwrap()
            );
            assert_eq!(control.attempted, 0);
        }
    }
    for fault in 0..10 {
        let mut actual =
            materialize_rows(&source, units(0.5), capacities, &mut Control::default()).unwrap();
        match fault {
            0 => actual.images[0].payload = TableRange::new(1, 3),
            1 => actual.images[0].payload = TableRange::new(0, 2),
            2 => actual.images[0].media_type = None,
            3 => actual.images[0].media_type = Some(TextRange(TableRange::new(1, 9))),
            4 => actual.images[0].media_type = Some(TextRange(TableRange::new(0, 8))),
            5 => actual.images.swap(0, 1),
            6 => actual.samplers[0] = source.samplers[1],
            7 => actual.textures[0].image = StoredImageIndex(0),
            8 => actual.textures[0].sampler = StoredSamplerIndex(1),
            9 => actual.textures.swap(0, 1),
            _ => unreachable!(),
        }
        assert!(
            !validate_rows_correspondence(&source, &actual, units(0.5), &mut Control::default())
                .unwrap()
        );
    }
}

#[test]
fn correspondence_rejects_binding_behavior_uv_ranges_and_signed_zero_changes() {
    let source = fixture();
    let capacities = shape(&source).rows_capacities().unwrap();
    for fault in 0..17 {
        let mut actual =
            materialize_rows(&source, units(0.5), capacities, &mut Control::default()).unwrap();
        let binding = &mut actual.bindings[0];
        match fault {
            0 => binding.texture = StoredTextureIndex(1),
            1 => binding.role = TextureRole::Normal,
            2 => binding.uv_set = 0,
            3 => binding.channels = TextureChannelSwizzle::RGB,
            4 => binding.color = ColorInterpretation::LinearColor,
            5..=9 => {
                let mut offset = binding.transform.offset();
                let mut rotation = binding.transform.rotation_radians();
                let mut scale = binding.transform.scale();
                match fault {
                    5 => offset.x = 0.0,
                    6 => offset.y = 0.5,
                    7 => rotation = 0.0,
                    8 => scale.x = 2.0,
                    9 => scale.y = -3.0,
                    _ => unreachable!(),
                }
                binding.transform = TextureTransform::try_new(offset, rotation, scale).unwrap();
            }
            10 => binding.strength = TextureStrength::try_new(1.5).unwrap(),
            11 => binding.blend = TextureBlendOperation::Add,
            12 => binding.source_key = None,
            13 => binding.source_key = Some(TextRange(TableRange::new(10, 11))),
            14 => binding.source_key = Some(TextRange(TableRange::new(9, 10))),
            15 => binding.source_key = Some(TextRange(TableRange::new(9, 0))),
            16 => actual.bindings[4].strength = TextureStrength::try_new(0.0).unwrap(),
            _ => unreachable!(),
        }
        assert!(
            !validate_rows_correspondence(&source, &actual, units(0.5), &mut Control::default())
                .unwrap()
        );
    }
}

#[test]
fn correspondence_rejects_material_identity_presence_and_each_factor_groups_bits() {
    let source = fixture();
    let capacities = shape(&source).rows_capacities().unwrap();
    for fault in 0..20 {
        let mut actual =
            materialize_rows(&source, units(0.5), capacities, &mut Control::default()).unwrap();
        let material = &mut actual.materials[0];
        let factors = material.factors;
        let mut emissive = factors.emissive();
        let mut metallic = factors.pbr_metallic_roughness();
        let mut specular = factors.pbr_specular_glossiness();
        let mut legacy = factors.legacy();
        let mut supplemental = factors.supplemental_pbr();
        match fault {
            0 => material.bindings = TableRange::new(1, 4),
            1 => material.bindings = TableRange::new(0, 3),
            2 => material.shading = ShadingModel::Unlit,
            3 => material.alpha = AlphaMode::Blend,
            4 => material.alpha = AlphaMode::mask(0.0).unwrap(),
            5 => material.double_sided = false,
            6 => material.name = None,
            7 => material.name = Some(TextRange(TableRange::new(20, 0))),
            8 => material.name = Some(TextRange(TableRange::new(21, 7))),
            9 => emissive.x = 0.0,
            10 => metallic = None,
            11 => specular = None,
            12 => legacy = None,
            13 => {
                let value = metallic.unwrap();
                let mut color = value.base_color();
                color.r = 0.0;
                metallic = Some(
                    PbrMetallicRoughnessFactors::try_new(
                        color,
                        value.metallic(),
                        value.roughness(),
                    )
                    .unwrap(),
                );
            }
            14 => {
                let value = specular.unwrap();
                let mut specular_color = value.specular();
                specular_color.x = 0.0;
                specular = Some(
                    PbrSpecularGlossinessFactors::try_new(
                        value.diffuse(),
                        specular_color,
                        value.glossiness(),
                    )
                    .unwrap(),
                );
            }
            15 => {
                let value = legacy.unwrap();
                legacy = Some(
                    LegacyMaterialFactors::try_new(
                        value.ambient(),
                        value.diffuse(),
                        value.specular(),
                        value.emissive(),
                        0.0,
                        value.opacity(),
                    )
                    .unwrap(),
                );
            }
            16 => {
                supplemental = SupplementalPbrFactors::try_new(
                    0.0,
                    supplemental.clearcoat_roughness(),
                    supplemental.transmission(),
                    supplemental.thickness(),
                    supplemental.sheen_color(),
                    supplemental.sheen_roughness(),
                )
                .unwrap();
            }
            17 => {
                let mut sheen = supplemental.sheen_color();
                sheen.x = 0.0;
                supplemental = SupplementalPbrFactors::try_new(
                    supplemental.clearcoat(),
                    supplemental.clearcoat_roughness(),
                    supplemental.transmission(),
                    supplemental.thickness(),
                    sheen,
                    supplemental.sheen_roughness(),
                )
                .unwrap();
            }
            18 => {
                actual.materials[1].name = None;
            }
            19 => {
                actual.materials[1].factors = MaterialFactors::empty();
            }
            _ => unreachable!(),
        }
        if fault < 18 {
            actual.materials[0].factors = MaterialFactors::try_new_with_supplemental_pbr(
                emissive,
                metallic,
                specular,
                legacy,
                supplemental,
            )
            .unwrap();
        }
        assert!(
            !validate_rows_correspondence(&source, &actual, units(0.5), &mut Control::default())
                .unwrap()
        );
    }
}

#[test]
fn every_correspondence_action_propagates_interruption_and_rejects_omitted_work() {
    let source = fixture();
    let capacities = shape(&source).rows_capacities().unwrap();
    let actual =
        materialize_rows(&source, units(0.5), capacities, &mut Control::default()).unwrap();
    for stop in 0..FIXTURE_ROW_WORK {
        let mut control = Control {
            stop: Some(stop),
            ..Control::default()
        };
        assert_eq!(
            validate_rows_correspondence(&source, &actual, units(0.5), &mut control),
            Err(stop)
        );
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        let mut control = Control {
            omit: Some(stop),
            ..Control::default()
        };
        assert_eq!(
            validate_rows_correspondence(&source, &actual, units(0.5), &mut control),
            Ok(false)
        );
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
    }
    let mut control = Control {
        stop: Some(FIXTURE_ROW_WORK),
        ..Control::default()
    };
    assert_eq!(
        validate_rows_correspondence(&source, &actual, units(0.5), &mut control),
        Ok(true)
    );
    assert_eq!(control.completed, FIXTURE_ROW_WORK);
    assert_eq!(control.attempted, FIXTURE_ROW_WORK);
}

#[test]
fn every_materialization_action_releases_all_five_buffers_on_interruption_and_unwind() {
    let source = fixture();
    let capacities = shape(&source).rows_capacities().unwrap();
    for stop in 0..FIXTURE_ROW_WORK {
        let mut control = Control {
            stop: Some(stop),
            ..Control::default()
        };
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                materialize_rows(&source, units(0.5), capacities, &mut control),
                Err(TypedStorageBuildError::Control(actual)) if actual == stop
            ));
        });
        assert_eq!(control.completed, stop);
        assert_eq!(control.attempted, stop + 1);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
        for after in [false, true] {
            let mut control = Control {
                panic_before: (!after).then_some(stop),
                panic_after: after.then_some(stop),
                ..Control::default()
            };
            let allocations = allocation_counter::measure(|| {
                let unwind = catch_unwind(AssertUnwindSafe(|| {
                    materialize_rows(&source, units(0.5), capacities, &mut control)
                }));
                assert!(unwind.is_err());
                drop(unwind);
            });
            assert_eq!(control.completed, stop + u64::from(after));
            assert_eq!(control.attempted, stop + 1);
            assert_eq!(allocations.count_current, 0);
            assert_eq!(allocations.bytes_current, 0);
        }
    }
    let mut control = Control {
        stop: Some(FIXTURE_ROW_WORK),
        ..Control::default()
    };
    let allocations = allocation_counter::measure(|| {
        let unwind = catch_unwind(AssertUnwindSafe(|| {
            let actual = materialize_rows(&source, units(0.5), capacities, &mut control).unwrap();
            std::hint::black_box(&actual);
            std::panic::resume_unwind(Box::new(0_u64));
        }));
        assert!(unwind.is_err());
        drop(unwind);
    });
    assert_eq!(control.completed, FIXTURE_ROW_WORK);
    assert_eq!(control.attempted, FIXTURE_ROW_WORK);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}
