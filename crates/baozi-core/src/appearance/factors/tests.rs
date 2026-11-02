use super::*;
use crate::appearance::{AlphaMode, MaterialSemantics, ShadingModel, validate_material_semantics};

#[test]
fn emission_retains_authored_strength_presence_and_bits_separately_from_rgb() {
    let raw = Vec3::new(-0.0, f32::from_bits(1), f32::MAX);
    for strength in [
        None,
        Some(1.0),
        Some(0.0),
        Some(-0.0),
        Some(f32::from_bits(1)),
        Some(f32::MAX),
    ] {
        let emission = EmissionFactors::try_new(raw, strength).unwrap();
        assert_eq!(emission.factor().x.to_bits(), (-0.0_f32).to_bits());
        assert_eq!(emission.factor().y.to_bits(), 1);
        assert_eq!(emission.factor().z.to_bits(), f32::MAX.to_bits());
        assert_eq!(
            emission.strength().map(f32::to_bits),
            strength.map(f32::to_bits)
        );
        assert_eq!(
            emission.effective_strength().to_bits(),
            strength.unwrap_or(1.0).to_bits()
        );
        let factors = MaterialFactors::empty().with_emission(emission);
        assert_eq!(
            factors.emission().strength().map(f32::to_bits),
            strength.map(f32::to_bits)
        );
        factors.validate().unwrap();

        let black = EmissionFactors::try_new(Vec3::ZERO, strength).unwrap();
        assert_eq!(
            black.strength().map(f32::to_bits),
            strength.map(f32::to_bits)
        );
    }
}

#[test]
fn effective_emission_widens_both_operands_before_multiplying() {
    let raw = Vec3::new(f32::MAX, f32::from_bits(1), -0.0);
    let emission = EmissionFactors::try_new(raw, Some(f32::MAX)).unwrap();
    let actual: [f64; 3] = emission.effective_rgb();
    let expected = [
        f64::from(f32::MAX) * f64::from(f32::MAX),
        f64::from(f32::from_bits(1)) * f64::from(f32::MAX),
        -0.0_f64,
    ];
    assert!(actual[0].is_finite());
    assert!(actual[0] > f64::from(f32::MAX));
    assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));

    let absent = EmissionFactors::try_new(raw, None).unwrap();
    let explicit_one = EmissionFactors::try_new(raw, Some(1.0)).unwrap();
    assert_eq!(absent.effective_rgb(), explicit_one.effective_rgb());
    assert_eq!(absent.strength(), None);
    assert_eq!(explicit_one.strength(), Some(1.0));
    let tiny = EmissionFactors::try_new(
        Vec3::new(f32::from_bits(1), 0.0, 0.0),
        Some(f32::from_bits(1)),
    )
    .unwrap()
    .effective_rgb();
    assert_eq!(
        tiny[0],
        f64::from(f32::from_bits(1)) * f64::from(f32::from_bits(1))
    );
    assert!(tiny[0] > 0.0);
}

#[test]
fn defaults_and_named_assembly_preserve_independent_present_zero_leaves() {
    const EMPTY: MaterialFactors = MaterialFactors::empty();
    assert_eq!(MaterialFactors::default(), EMPTY);
    assert_eq!(EMPTY.emission(), EmissionFactors::default());
    assert_eq!(EMPTY.emission().factor(), Vec3::ZERO);
    assert_eq!(EMPTY.emission().strength(), None);
    assert_eq!(EMPTY.emission().effective_strength(), 1.0);
    assert_eq!(EMPTY.pbr_metallic_roughness(), None);
    assert_eq!(EMPTY.pbr_specular_glossiness(), None);
    assert_eq!(EMPTY.legacy(), None);
    assert_eq!(EMPTY.clearcoat(), None);
    assert_eq!(EMPTY.transmission(), None);
    assert_eq!(EMPTY.thickness(), None);
    assert_eq!(EMPTY.sheen(), None);

    let present = EMPTY
        .with_clearcoat(Some(ClearcoatFactors::default()))
        .with_transmission(Some(TransmissionFactors::default()))
        .with_thickness(Some(ThicknessFactors::default()))
        .with_sheen(Some(SheenFactors::default()));
    assert_eq!(present.clearcoat().unwrap().factor().to_bits(), 0);
    assert_eq!(present.clearcoat().unwrap().roughness().to_bits(), 0);
    assert_eq!(present.transmission().unwrap().factor().to_bits(), 0);
    assert_eq!(present.thickness().unwrap().thickness().to_bits(), 0);
    assert_eq!(present.sheen().unwrap().color(), Vec3::ZERO);
    assert_eq!(present.sheen().unwrap().roughness().to_bits(), 0);
    assert_ne!(present, EMPTY);
    assert_eq!(
        present.with_clearcoat(None).transmission(),
        present.transmission()
    );
    assert_eq!(
        present.with_transmission(None).thickness(),
        present.thickness()
    );
    assert_eq!(present.with_thickness(None).sheen(), present.sheen());
    assert_eq!(present.with_sheen(None).clearcoat(), present.clearcoat());
    assert_eq!(
        present
            .with_clearcoat(None)
            .with_transmission(None)
            .with_thickness(None)
            .with_sheen(None),
        EMPTY
    );
}

#[test]
fn independent_lobes_preserve_hdr_signed_zero_subnormals_and_unlit_fallback() {
    let raw = Vec3::new(f32::MAX, -0.0, f32::from_bits(1));
    let legacy = LegacyMaterialFactors::try_new(raw, raw, raw, raw, f32::MAX, -0.0).unwrap();
    let metallic = PbrMetallicRoughnessFactors::default();
    let specular = PbrSpecularGlossinessFactors::try_new(Color::WHITE, Vec3::ZERO, -0.0).unwrap();
    let factors = MaterialFactors::empty()
        .with_emission(EmissionFactors::try_new(raw, Some(-0.0)).unwrap())
        .with_pbr_metallic_roughness(Some(metallic))
        .with_pbr_specular_glossiness(Some(specular))
        .with_legacy(Some(legacy))
        .with_clearcoat(Some(
            ClearcoatFactors::try_new(-0.0, f32::from_bits(1)).unwrap(),
        ))
        .with_thickness(Some(ThicknessFactors::try_new(f32::MAX).unwrap()))
        .with_sheen(Some(SheenFactors::try_new(raw, -0.0).unwrap()));
    assert_eq!(factors.transmission(), None);
    assert_eq!(factors.thickness().unwrap().thickness(), f32::MAX);
    assert_eq!(factors.sheen().unwrap().color().x, f32::MAX);
    assert_eq!(
        factors.sheen().unwrap().color().y.to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(factors.sheen().unwrap().color().z.to_bits(), 1);
    assert_eq!(
        factors.clearcoat().unwrap().factor().to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(factors.clearcoat().unwrap().roughness().to_bits(), 1);
    let allocations = allocation_counter::measure(|| {
        factors.validate().unwrap();
        for shading in [
            ShadingModel::Unspecified,
            ShadingModel::Unlit,
            ShadingModel::PbrMetallicRoughness,
            ShadingModel::PbrSpecularGlossiness,
            ShadingModel::Phong,
        ] {
            let semantics = MaterialSemantics::new(shading, AlphaMode::Opaque, true, factors);
            validate_material_semantics(semantics).unwrap();
            assert!(semantics.factors().same_bits(factors));
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    let transmission_only = MaterialFactors::empty()
        .with_transmission(Some(TransmissionFactors::try_new(0.75).unwrap()));
    assert_eq!(transmission_only.thickness(), None);
    transmission_only.validate().unwrap();
    assert_eq!(
        factors
            .with_pbr_metallic_roughness(None)
            .pbr_metallic_roughness(),
        None
    );
    assert_eq!(
        factors
            .with_pbr_specular_glossiness(None)
            .pbr_specular_glossiness(),
        None
    );
    assert_eq!(factors.with_legacy(None).legacy(), None);
}

#[test]
fn checked_leaves_reject_invalid_values_with_the_exact_authored_field() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for (actual, field) in [
            (
                EmissionFactors::try_new(Vec3::new(0.0, value, 0.0), None).map(|_| ()),
                MaterialFactorField::Emissive,
            ),
            (
                EmissionFactors::try_new(Vec3::ZERO, Some(value)).map(|_| ()),
                MaterialFactorField::EmissiveStrength,
            ),
            (
                ClearcoatFactors::try_new(value, 0.0).map(|_| ()),
                MaterialFactorField::Clearcoat,
            ),
            (
                ClearcoatFactors::try_new(0.0, value).map(|_| ()),
                MaterialFactorField::ClearcoatRoughness,
            ),
            (
                TransmissionFactors::try_new(value).map(|_| ()),
                MaterialFactorField::Transmission,
            ),
            (
                ThicknessFactors::try_new(value).map(|_| ()),
                MaterialFactorField::Thickness,
            ),
            (
                SheenFactors::try_new(Vec3::new(0.0, 0.0, value), 0.0).map(|_| ()),
                MaterialFactorField::SheenColor,
            ),
            (
                SheenFactors::try_new(Vec3::ZERO, value).map(|_| ()),
                MaterialFactorField::SheenRoughness,
            ),
        ] {
            assert_eq!(actual, Err(MaterialFactorError::NonFinite { field }));
        }
    }
    for value in [-1.0, -f32::from_bits(1)] {
        for (actual, field) in [
            (
                EmissionFactors::try_new(Vec3::new(value, 0.0, 0.0), None).map(|_| ()),
                MaterialFactorField::Emissive,
            ),
            (
                EmissionFactors::try_new(Vec3::ZERO, Some(value)).map(|_| ()),
                MaterialFactorField::EmissiveStrength,
            ),
            (
                ThicknessFactors::try_new(value).map(|_| ()),
                MaterialFactorField::Thickness,
            ),
            (
                SheenFactors::try_new(Vec3::new(value, 0.0, 0.0), 0.0).map(|_| ()),
                MaterialFactorField::SheenColor,
            ),
        ] {
            assert_eq!(actual, Err(MaterialFactorError::Negative { field }));
        }
    }
    for value in [-f32::from_bits(1), f32::from_bits(1.0_f32.to_bits() + 1)] {
        for (actual, field) in [
            (
                ClearcoatFactors::try_new(value, 0.0).map(|_| ()),
                MaterialFactorField::Clearcoat,
            ),
            (
                ClearcoatFactors::try_new(0.0, value).map(|_| ()),
                MaterialFactorField::ClearcoatRoughness,
            ),
            (
                TransmissionFactors::try_new(value).map(|_| ()),
                MaterialFactorField::Transmission,
            ),
            (
                SheenFactors::try_new(Vec3::ZERO, value).map(|_| ()),
                MaterialFactorField::SheenRoughness,
            ),
        ] {
            assert_eq!(
                actual,
                Err(MaterialFactorError::OutsideUnitInterval { field })
            );
        }
    }
}

#[test]
fn actual_values_revalidate_every_new_field_even_with_zero_emission_and_unlit() {
    let empty = MaterialFactors::empty();
    let cases = [
        (
            empty.with_emission(EmissionFactors {
                factor: Vec3::new(f32::NAN, 0.0, 0.0),
                strength: None,
            }),
            MaterialFactorField::Emissive,
        ),
        (
            empty.with_emission(EmissionFactors {
                factor: Vec3::ZERO,
                strength: Some(f32::NAN),
            }),
            MaterialFactorField::EmissiveStrength,
        ),
        (
            empty.with_clearcoat(Some(ClearcoatFactors {
                factor: f32::NAN,
                roughness: 0.0,
            })),
            MaterialFactorField::Clearcoat,
        ),
        (
            empty.with_clearcoat(Some(ClearcoatFactors {
                factor: 0.0,
                roughness: f32::NAN,
            })),
            MaterialFactorField::ClearcoatRoughness,
        ),
        (
            empty.with_transmission(Some(TransmissionFactors { factor: f32::NAN })),
            MaterialFactorField::Transmission,
        ),
        (
            empty.with_thickness(Some(ThicknessFactors {
                thickness: f32::NAN,
            })),
            MaterialFactorField::Thickness,
        ),
        (
            empty.with_sheen(Some(SheenFactors {
                color: Vec3::new(0.0, f32::NAN, 0.0),
                roughness: 0.0,
            })),
            MaterialFactorField::SheenColor,
        ),
        (
            empty.with_sheen(Some(SheenFactors {
                color: Vec3::ZERO,
                roughness: f32::NAN,
            })),
            MaterialFactorField::SheenRoughness,
        ),
    ];
    for (factors, field) in cases {
        let expected = MaterialFactorError::NonFinite { field };
        let allocations = allocation_counter::measure(|| {
            assert_eq!(factors.validate(), Err(expected));
            let semantics =
                MaterialSemantics::new(ShadingModel::Unlit, AlphaMode::Opaque, false, factors);
            assert_eq!(semantics.factors().validate(), Err(expected));
        });
        assert_eq!(allocations.count_total, 0);
    }
}

#[test]
fn same_bits_distinguishes_new_presence_and_each_component_of_present_zero_leaves() {
    let empty = MaterialFactors::empty();
    for present in [
        empty.with_emission(EmissionFactors::try_new(Vec3::ZERO, Some(0.0)).unwrap()),
        empty.with_clearcoat(Some(ClearcoatFactors::default())),
        empty.with_transmission(Some(TransmissionFactors::default())),
        empty.with_thickness(Some(ThicknessFactors::default())),
        empty.with_sheen(Some(SheenFactors::default())),
    ] {
        assert!(!empty.same_bits(present));
        assert!(!present.same_bits(empty));
        assert!(present.same_bits(present));
    }
    let zero = empty
        .with_emission(EmissionFactors::try_new(Vec3::ZERO, Some(0.0)).unwrap())
        .with_clearcoat(Some(ClearcoatFactors::default()))
        .with_transmission(Some(TransmissionFactors::default()))
        .with_thickness(Some(ThicknessFactors::default()))
        .with_sheen(Some(SheenFactors::default()));
    for changed in [
        zero.with_emission(EmissionFactors::try_new(Vec3::ZERO, Some(-0.0)).unwrap()),
        zero.with_clearcoat(Some(ClearcoatFactors::try_new(-0.0, 0.0).unwrap())),
        zero.with_clearcoat(Some(ClearcoatFactors::try_new(0.0, -0.0).unwrap())),
        zero.with_transmission(Some(TransmissionFactors::try_new(-0.0).unwrap())),
        zero.with_thickness(Some(ThicknessFactors::try_new(-0.0).unwrap())),
        zero.with_sheen(Some(
            SheenFactors::try_new(Vec3::new(-0.0, 0.0, 0.0), 0.0).unwrap(),
        )),
        zero.with_sheen(Some(
            SheenFactors::try_new(Vec3::new(0.0, -0.0, 0.0), 0.0).unwrap(),
        )),
        zero.with_sheen(Some(
            SheenFactors::try_new(Vec3::new(0.0, 0.0, -0.0), 0.0).unwrap(),
        )),
        zero.with_sheen(Some(SheenFactors::try_new(Vec3::ZERO, -0.0).unwrap())),
    ] {
        assert_eq!(changed, zero);
        assert!(!zero.same_bits(changed));
        assert!(!changed.same_bits(zero));
    }
    let implicit =
        empty.with_emission(EmissionFactors::try_new(Vec3::new(2.0, 1.0, 0.5), None).unwrap());
    let explicit = implicit
        .with_emission(EmissionFactors::try_new(implicit.emission().factor(), Some(1.0)).unwrap());
    assert_eq!(
        implicit.emission().effective_rgb(),
        explicit.emission().effective_rgb()
    );
    assert!(!implicit.same_bits(explicit));
}
