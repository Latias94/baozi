use std::cell::Cell;
use std::rc::Rc;

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
fn compiled_appearance_facts_do_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let image_bytes = vec![1_u8, 2, 3];
        let images = vec![ImageAuthoring::encoded(&image_bytes, Presence::Absent)];

        validate_appearance_batch_with_control(
            AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("temporary authored storage must compile into owned appearance facts")
        .into_compiled_facts_for_test()
    };

    assert_eq!(compiled.0, 1);
    assert_eq!(compiled.1, 1);
    assert_eq!(compiled.2, 0);
    assert_eq!(compiled.3, 0);

    drop(compiled);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn compiled_appearance_storage_drops_before_its_reservation() {
    let image_bytes = [1_u8];
    let images = [ImageAuthoring::encoded(&image_bytes, Presence::Absent)];
    let stage = Rc::new(Cell::new(0));
    let mut control = TestControl {
        reservation_drop_stage: Some(Rc::clone(&stage)),
        ..TestControl::default()
    };
    let mut validated = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("appearance plan must validate before its coupled drop-order proof");

    validated.install_storage_drop_probe(Rc::clone(&stage));
    drop(validated);

    assert_eq!(stage.get(), 2);
    assert_eq!(control.live.get(), 0);
}

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
