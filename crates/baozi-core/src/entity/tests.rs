use super::*;

#[test]
fn authored_transform_preserves_matrix_or_trs_without_conversion() {
    let mut transform = Mat4::IDENTITY;
    transform.cols[3][0] = -0.0;
    let matrix = AuthoredTransform::try_matrix(transform).unwrap();
    assert_eq!(matrix.kind(), AuthoredTransformKind::Matrix);
    assert_eq!(matrix.authored_matrix(), Some(transform));
    assert_eq!(matrix.local_matrix(), transform);
    assert_eq!(matrix.authored_trs(), None);

    let trs = AuthoredTransform::try_trs(
        Vec3::new(4.0, 5.0, 6.0),
        Quat::IDENTITY,
        Vec3::new(-2.0, 0.0, 3.0),
    )
    .unwrap();
    assert_eq!(trs.kind(), AuthoredTransformKind::Trs);
    assert_eq!(trs.authored_matrix(), None);
    assert_eq!(
        trs.authored_trs().unwrap().scale(),
        Vec3::new(-2.0, 0.0, 3.0)
    );
    assert_eq!(trs.local_matrix().cols[3], [4.0, 5.0, 6.0, 1.0]);

    let positive_zero_trs = AuthoredTransform::identity_trs();
    let negative_zero_trs = AuthoredTransform::try_trs(
        Vec3::new(-0.0, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .unwrap();
    assert_eq!(positive_zero_trs, negative_zero_trs);
    assert!(!positive_zero_trs.has_same_authored_bits(negative_zero_trs));

    let positive_zero_matrix = AuthoredTransform::try_matrix(Mat4::IDENTITY).unwrap();
    assert_eq!(matrix, positive_zero_matrix);
    assert!(!matrix.has_same_authored_bits(positive_zero_matrix));
}

#[test]
fn authored_transform_rejects_invalid_matrix_and_trs_values() {
    let mut non_finite = Mat4::IDENTITY;
    non_finite.cols[2][1] = f32::NAN;
    assert_eq!(
        AuthoredTransform::try_matrix(non_finite),
        Err(AuthoredTransformError::MatrixNotFinite)
    );

    let mut non_affine = Mat4::IDENTITY;
    non_affine.cols[0][3] = 0.25;
    assert_eq!(
        AuthoredTransform::try_matrix(non_affine),
        Err(AuthoredTransformError::MatrixNotAffine)
    );

    assert_eq!(
        AuthoredTransform::try_trs(
            Vec3::ZERO,
            Quat::from_xyzw(0.0, 0.0, 0.0, 2.0),
            Vec3::new(1.0, 1.0, 1.0),
        ),
        Err(AuthoredTransformError::RotationNotUnit)
    );
}

#[test]
fn camera_projection_rejects_invalid_frusta() {
    assert_eq!(
        CameraProjection::perspective(0.0, None, 0.1, None),
        Err(CameraProjectionError::VerticalFieldOfView)
    );
    assert_eq!(
        CameraProjection::perspective(1.0, Some(0.0), 0.1, None),
        Err(CameraProjectionError::AspectRatio)
    );
    assert_eq!(
        CameraProjection::perspective(1.0, None, 0.0, None),
        Err(CameraProjectionError::NearPlane)
    );
    assert_eq!(
        CameraProjection::orthographic(1.0, 1.0, 2.0, 1.0),
        Err(CameraProjectionError::FarPlane)
    );
    assert!(CameraProjection::orthographic(1.0, 1.0, 0.0, 1.0).is_ok());
    assert_eq!(
        CameraProjection::orthographic(1.0, 1.0, -f32::MIN_POSITIVE, 1.0),
        Err(CameraProjectionError::NearPlane)
    );
}

#[test]
fn light_kind_makes_spot_invariants_explicit() {
    assert_eq!(
        LightKind::spot(None, 0.5, 0.25),
        Err(LightError::InvalidSpotCone)
    );
    assert!(LightKind::spot(Some(10.0), 0.25, 0.5).is_ok());
    assert_eq!(LightKind::point(Some(0.0)), Err(LightError::InvalidRange));
}

#[test]
fn light_emission_has_one_canonical_photometric_representation() {
    let emission =
        LightEmission::try_from_scaled_linear_srgb(Color::linear_rgba(2.0, 1.0, 0.5, 1.0), 4.0)
            .expect("finite non-negative emission should canonicalize");
    let tint = emission.tint();
    let relative_luminance_numerator = rec709_luminance_numerator(tint);
    assert!(
        (relative_luminance_numerator - REC709_LUMINANCE_DENOMINATOR).abs()
            <= LIGHT_TINT_LUMINANCE_TOLERANCE
    );
    assert_eq!(emission.intensity(), 4.706_f32);
    assert_eq!(
        [
            emission.tint().x.to_bits(),
            emission.tint().y.to_bits(),
            emission.tint().z.to_bits(),
            emission.intensity().to_bits(),
        ],
        [0x3FD9_9835, 0x3F59_9835, 0x3ED9_9835, 0x4096_978D]
    );
    assert_eq!(
        validate_canonical_light_emission(tint, emission.intensity()),
        Ok(())
    );
    assert_eq!(
        LightEmission::try_from_scaled_linear_srgb(Color::linear_rgba(4.0, 2.0, 1.0, 1.0), 2.0,),
        Ok(emission)
    );

    let zero =
        LightEmission::try_from_scaled_linear_srgb(Color::linear_rgba(0.0, -0.0, 0.0, 1.0), 100.0)
            .expect("black emission should canonicalize to the unique zero form");
    assert_eq!(zero.tint(), Vec3::new(1.0, 1.0, 1.0));
    assert_eq!(zero.intensity().to_bits(), 0.0_f32.to_bits());

    let positive_subnormal =
        LightEmission::try_from_scaled_linear_srgb(Color::WHITE, f32::from_bits(1))
            .expect("positive subnormal canonical intensity remains representable");
    assert_eq!(positive_subnormal.intensity().to_bits(), 1);
}

#[test]
fn light_emission_rejects_noncanonical_alpha_and_unrepresentable_intensity() {
    assert!(matches!(
        LightEmission::try_from_scaled_linear_srgb(Color::linear_rgba(1.0, 1.0, 1.0, 0.5), 1.0,),
        Err(LightEmissionError::InvalidAlpha)
    ));
    assert!(matches!(
        LightEmission::try_from_scaled_linear_srgb(
            Color::linear_rgba(f32::MAX, f32::MAX, f32::MAX, 1.0),
            f32::MAX,
        ),
        Err(LightEmissionError::Unrepresentable)
    ));
    assert!(matches!(
        LightEmission::try_from_scaled_linear_srgb(
            Color::linear_rgba(f32::from_bits(1), 0.0, 0.0, 1.0),
            f32::from_bits(1),
        ),
        Err(LightEmissionError::Unrepresentable)
    ));
    assert!(matches!(
        LightEmission::try_from_scaled_linear_srgb(
            Color::linear_rgba(f32::from_bits(1), f32::MAX, 0.0, 1.0),
            1.0,
        ),
        Err(LightEmissionError::Unrepresentable)
    ));
    assert!(matches!(
        validate_canonical_light_emission(Vec3::new(2.0, 1.0, 0.5), 1.0,),
        Err(LightEmissionError::NonCanonicalTint)
    ));

    let pure_red =
        LightEmission::try_from_scaled_linear_srgb(Color::linear_rgba(1.0, 0.0, 0.0, 1.0), 1.0)
            .expect("pure red is representable");
    assert_eq!(pure_red.tint().x.to_bits(), 0x4096_8475);
    assert_eq!(pure_red.intensity().to_bits(), 0x3E59_B3D0);
    assert_eq!(
        validate_canonical_light_emission(
            Vec3::new(f32::from_bits(pure_red.tint().x.to_bits() + 4), 0.0, 0.0),
            pure_red.intensity(),
        ),
        Err(LightEmissionError::NonCanonicalTint)
    );
    assert_eq!(
        validate_canonical_light_emission(
            Vec3::new(pure_red.tint().x, -0.0, 0.0),
            pure_red.intensity(),
        ),
        Err(LightEmissionError::NonCanonicalTint)
    );

    let inclusive_boundary = f32::from_bits(1.0_f32.to_bits() + 1);
    assert_eq!(
        validate_canonical_light_emission(
            Vec3::new(inclusive_boundary, inclusive_boundary, inclusive_boundary),
            1.0,
        ),
        Ok(())
    );
    let outside_boundary = f32::from_bits(1.0_f32.to_bits() + 2);
    assert_eq!(
        validate_canonical_light_emission(
            Vec3::new(outside_boundary, outside_boundary, outside_boundary),
            1.0,
        ),
        Err(LightEmissionError::NonCanonicalTint)
    );
    assert_eq!(
        validate_canonical_light_emission(Vec3::new(1.0, 1.0, 0.0), 0.0),
        Err(LightEmissionError::NonCanonicalZero)
    );
    assert_eq!(
        validate_canonical_light_emission(Vec3::new(1.0, 1.0, 1.0), -0.0),
        Err(LightEmissionError::NonCanonicalZero)
    );
}

#[test]
fn entity_name_bounds_and_sanitizes_display_text() {
    assert_eq!(
        EntityName::try_new("bad\nname"),
        Err(EntityNameError::ControlCharacter { offset: 3 })
    );
    let oversized = "x".repeat(MAX_ENTITY_NAME_BYTES + 1);
    assert_eq!(
        EntityName::try_new(oversized),
        Err(EntityNameError::TooLong {
            actual: MAX_ENTITY_NAME_BYTES + 1,
            maximum: MAX_ENTITY_NAME_BYTES,
        })
    );
}
