use std::cell::Cell;
use std::rc::Rc;

use crate::appearance::{
    AlphaMode, AppearanceBuildError, ColorInterpretation, MaterialFactors, MaterialSemanticsRef,
    ShadingModel, TextureBindingSemanticsRef, TextureBlendOperation, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength, TextureTransform,
};
use crate::extension::ExtensionMediaType;
use crate::image::SamplerState;

use super::super::plan::appearance::{
    AppearanceAuthoringFailure, AppearanceAuthoringSemanticError,
    validate_appearance_batch_with_control,
};
use super::super::{
    AppearanceAuthoringBatch, AuthoringRange, ImageAuthoring, ImageLocalId, MaterialAuthoring,
    MaterialBindingAuthoring, Presence, ResourceIdentityLocalId, ResourceSnapshotAuthoring,
    ResourceSnapshotAuthoringTable, SamplerAuthoring, SamplerLocalId, TextureAuthoring,
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
            ResourceSnapshotAuthoringTable::empty(),
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
        ResourceSnapshotAuthoringTable::empty(),
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
fn session_resource_image_resolves_from_the_aligned_snapshot_table() {
    let snapshotted_bytes = [0x89, b'P', b'N', b'G'];
    let media_type = ExtensionMediaType::new("image/png").expect("valid image media type");
    let images = [ImageAuthoring::session_resource(
        ResourceIdentityLocalId::new(0),
        Presence::Present(&media_type),
    )];
    let snapshots = [ResourceSnapshotAuthoring::snapshotted(&snapshotted_bytes)];
    let mut control = TestControl::default();

    let validated = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
        ResourceSnapshotAuthoringTable::new(1, &snapshots),
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("same-session resource snapshot must resolve");

    assert_eq!(
        validated
            .image(0)
            .expect("validated resource image")
            .payload(),
        snapshotted_bytes
    );
    assert_eq!(
        validated
            .image(0)
            .expect("validated resource image")
            .media_type(),
        Presence::Present(&media_type)
    );
    drop(validated);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn session_resource_image_rejects_missing_or_foreign_snapshots() {
    let missing_images = [ImageAuthoring::session_resource(
        ResourceIdentityLocalId::new(0),
        Presence::Absent,
    )];
    let missing_snapshots = [ResourceSnapshotAuthoring::absent()];
    let mut missing_control = TestControl::default();
    let missing = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::new(&missing_images, &[], &[], &[], &[]),
        ResourceSnapshotAuthoringTable::new(1, &missing_snapshots),
        SceneValidationLimits::default(),
        &mut missing_control,
    );
    assert!(matches!(
        missing,
        Err(AppearanceAuthoringFailure::Semantic(
            AppearanceAuthoringSemanticError::ImageResourceSnapshotAbsent {
                image: 0,
                resource: 0,
            }
        ))
    ));

    let empty_bytes = [];
    let empty_snapshots = [ResourceSnapshotAuthoring::snapshotted(&empty_bytes)];
    let mut empty_control = TestControl::default();
    let empty = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::new(&missing_images, &[], &[], &[], &[]),
        ResourceSnapshotAuthoringTable::new(1, &empty_snapshots),
        SceneValidationLimits::default(),
        &mut empty_control,
    );
    assert!(matches!(
        empty,
        Err(AppearanceAuthoringFailure::Semantic(
            AppearanceAuthoringSemanticError::EmptyEncodedPayload { image: 0 }
        ))
    ));

    let foreign_images = [ImageAuthoring::session_resource(
        ResourceIdentityLocalId::new(1),
        Presence::Absent,
    )];
    let snapshotted_bytes = [1_u8];
    let snapshots = [ResourceSnapshotAuthoring::snapshotted(&snapshotted_bytes)];
    let mut foreign_control = TestControl::default();
    let foreign = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::new(&foreign_images, &[], &[], &[], &[]),
        ResourceSnapshotAuthoringTable::new(1, &snapshots),
        SceneValidationLimits::default(),
        &mut foreign_control,
    );
    assert!(matches!(
        foreign,
        Err(AppearanceAuthoringFailure::Semantic(
            AppearanceAuthoringSemanticError::ImageResourceOutOfRange {
                image: 0,
                resource: 1,
                resource_count: 1,
            }
        ))
    ));
}

#[test]
fn resource_snapshot_table_must_align_with_profile_resources() {
    let image_bytes = [1_u8];
    let images = [ImageAuthoring::encoded(&image_bytes, Presence::Absent)];
    let snapshots = [
        ResourceSnapshotAuthoring::snapshotted(&image_bytes),
        ResourceSnapshotAuthoring::absent(),
    ];
    let mut control = TestControl::default();

    let result = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]),
        ResourceSnapshotAuthoringTable::new(1, &snapshots),
        SceneValidationLimits::default(),
        &mut control,
    );

    assert!(matches!(
        result,
        Err(AppearanceAuthoringFailure::Semantic(
            AppearanceAuthoringSemanticError::ResourceSnapshotCardinalityMismatch {
                snapshot_count: 2,
                resource_count: 1,
            }
        ))
    ));
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
        ResourceSnapshotAuthoringTable::empty(),
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
        ResourceSnapshotAuthoringTable::empty(),
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
