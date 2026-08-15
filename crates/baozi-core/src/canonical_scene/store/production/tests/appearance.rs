use crate::appearance::{
    AlphaMode, AppearanceBuildError, ColorInterpretation, MaterialFactors, MaterialSemanticsRef,
    ShadingModel, TextureBindingSemanticsRef, TextureBlendOperation, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength, TextureTransform,
};
use crate::image::SamplerState;

use super::super::plan::appearance::{
    AppearanceAuthoringFailure, AppearanceAuthoringSemanticError,
    validate_appearance_batch_with_control,
};
use super::super::{
    AppearanceAuthoringBatch, AuthoringRange, ImageAuthoring, ImageLocalId, MaterialAuthoring,
    MaterialBindingAuthoring, Presence, SamplerAuthoring, SamplerLocalId, TextureAuthoring,
    TextureLocalId,
};
use super::{SceneValidationLimits, TestControl};

#[test]
fn duplicate_material_binding_stops_at_the_exact_completed_prefix() {
    let image_bytes = [1_u8];
    let images = [ImageAuthoring::encoded(&image_bytes, Presence::Absent)];
    let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
    let textures = [TextureAuthoring::new(
        ImageLocalId::new(0),
        SamplerLocalId::new(0),
    )];
    let role = TextureRole::BaseColor;
    let coordinate_source =
        TextureCoordinateSource::uv_set(0).expect("test UV set should be valid");
    let channel_mapping = TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA);
    let color_interpretation = ColorInterpretation::SrgbColor;
    let blend = TextureBlendOperation::Replace;
    let binding_semantics = TextureBindingSemanticsRef::new(
        &role,
        &coordinate_source,
        &channel_mapping,
        &color_interpretation,
        TextureTransform::IDENTITY,
        TextureStrength::ONE,
        &blend,
        None,
    );
    let bindings = [
        MaterialBindingAuthoring::new(TextureLocalId::new(0), binding_semantics),
        MaterialBindingAuthoring::new(TextureLocalId::new(0), binding_semantics),
        MaterialBindingAuthoring::new(TextureLocalId::new(0), binding_semantics),
        MaterialBindingAuthoring::new(TextureLocalId::new(0), binding_semantics),
    ];
    let shading = ShadingModel::Unlit;
    let alpha = AlphaMode::Opaque;
    let material_semantics =
        MaterialSemanticsRef::new(&shading, &alpha, false, MaterialFactors::empty());
    let materials = [MaterialAuthoring::new(
        AuthoringRange::new(0, 4),
        material_semantics,
    )];
    let input = AppearanceAuthoringBatch::new(&images, &samplers, &textures, &bindings, &materials);
    let mut control = TestControl::default();

    let result = validate_appearance_batch_with_control(
        input,
        SceneValidationLimits::default(),
        &mut control,
    );

    assert!(matches!(
        result,
        Err(AppearanceAuthoringFailure::Semantic(
            AppearanceAuthoringSemanticError::InvalidMaterialBindingSemantics {
                material: 0,
                source: AppearanceBuildError::DuplicateBindingAuthority {
                    first_index: 0,
                    duplicate_index: 1,
                },
            }
        ))
    ));
    assert_eq!(
        control.traversal_work(),
        8,
        "six structural traversals and the first two binding checks must be the exact prefix"
    );
}

#[test]
fn unowned_material_binding_is_a_completed_traversal_action() {
    let image_bytes = [1_u8];
    let images = [ImageAuthoring::encoded(&image_bytes, Presence::Absent)];
    let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
    let textures = [TextureAuthoring::new(
        ImageLocalId::new(0),
        SamplerLocalId::new(0),
    )];
    let role = TextureRole::BaseColor;
    let coordinate_source =
        TextureCoordinateSource::uv_set(0).expect("test UV set should be valid");
    let channel_mapping = TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA);
    let color_interpretation = ColorInterpretation::SrgbColor;
    let blend = TextureBlendOperation::Replace;
    let binding_semantics = TextureBindingSemanticsRef::new(
        &role,
        &coordinate_source,
        &channel_mapping,
        &color_interpretation,
        TextureTransform::IDENTITY,
        TextureStrength::ONE,
        &blend,
        None,
    );
    let bindings = [MaterialBindingAuthoring::new(
        TextureLocalId::new(0),
        binding_semantics,
    )];
    let input = AppearanceAuthoringBatch::new(&images, &samplers, &textures, &bindings, &[]);
    let mut control = TestControl::default();

    let result = validate_appearance_batch_with_control(
        input,
        SceneValidationLimits::default(),
        &mut control,
    );

    assert!(matches!(
        result,
        Err(AppearanceAuthoringFailure::Semantic(
            AppearanceAuthoringSemanticError::UnownedBindings { unowned: 1 }
        ))
    ));
    assert_eq!(
        control.last_work(),
        Some(crate::validation_control::SceneValidationWork::Traversal)
    );
}
