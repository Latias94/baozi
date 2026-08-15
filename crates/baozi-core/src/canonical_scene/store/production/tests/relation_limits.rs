use std::convert::Infallible;

use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::production::control::CanonicalAuthoringControl;
use crate::canonical_scene::store::production::plan::animation::{
    AnimationAuthoringFailure, AnimationAuthoringLimit, AnimationAuthoringSemanticError,
    AnimationTargetCatalog, validate_animation_batch_with_control,
};
use crate::canonical_scene::store::production::plan::appearance::{
    AppearanceAuthoringFailure, AppearanceAuthoringLimit, AppearanceAuthoringSemanticError,
    validate_appearance_batch_with_control,
};
use crate::canonical_scene::store::production::plan::deformation::{
    DeformationAuthoringFailure, DeformationAuthoringLimit, DeformationAuthoringSemanticError,
    validate_deformation_batch_with_control,
};
use crate::canonical_scene::store::production::plan::geometry::{
    GeometryAuthoringFailure, GeometryAuthoringLimit, GeometryAuthoringSemanticError,
    validate_geometry_batch_with_control,
};
use crate::canonical_scene::store::production::plan::profile::{
    ProfileAuthoringFailure, ProfileAuthoringLimit, ProfileAuthoringSemanticError,
    ProfileOwnerCatalog, validate_profile_batch_with_control,
};
use crate::canonical_scene::store::production::plan::spatial::{
    SpatialAuthoringFailure, SpatialAuthoringLimit, SpatialAuthoringSemanticError,
    validate_spatial_batch_with_control,
};
use crate::canonical_scene::store::production::plan::{
    NodeAuthoringFailure, NodeAuthoringLimit, NodeAuthoringSemanticError,
    validate_node_batch_with_control,
};
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolationAuthoring, AnimationInterpolationAuthoring, AnimationTargetAuthoring,
    AnimationTimeDomainAuthoring, AppearanceAuthoringBatch, AuthoringRange, CameraAuthoring,
    CameraLocalId, DeformationAuthoringBatch, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    ImageAuthoring, ImageLocalId, LightAuthoring, LightLocalId, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, Presence, SamplerAuthoring, SamplerLocalId, SkinAuthoring,
    SpatialAuthoringBatch, TextureAuthoring,
};
use crate::deform::SkinMatrixSpaces;
use crate::entity::{AuthoredTransform, CameraProjection, LightEmission, LightKind};
use crate::image::SamplerState;
use crate::math::{Color, Mat4};
use crate::scene_space::SceneSpace;
use crate::{SceneValidationLimits, SceneValidationWork};

use super::{TestControl, parent, profile_fixture, trs, validate};

#[derive(Default)]
struct RelationWorkControl {
    graph_edges: u64,
    references: u64,
}

impl CanonicalAuthoringControl for RelationWorkControl {
    type Error = Infallible;
    type Reservation = ();

    fn poll(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn run_unit<R>(
        &mut self,
        work: SceneValidationWork,
        action: impl FnOnce() -> R,
    ) -> Result<R, Self::Error> {
        let result = action();
        match work {
            SceneValidationWork::GraphEdge => self.graph_edges += 1,
            SceneValidationWork::Reference => self.references += 1,
            _ => {}
        }
        Ok(result)
    }

    fn reserve_scratch(&mut self, _bytes: u64) -> Result<Self::Reservation, Self::Error> {
        Ok(())
    }

    fn validate_scratch(&mut self, _reservation: &Self::Reservation) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resize_scratch(
        &mut self,
        _reservation: &mut Self::Reservation,
        _required_bytes: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn expect_node_semantic(
    result: Result<
        super::super::plan::ValidatedNodeBatch<'_, ()>,
        NodeAuthoringFailure<Infallible>,
    >,
) -> NodeAuthoringSemanticError {
    match result {
        Err(NodeAuthoringFailure::Semantic(error)) => error,
        Err(NodeAuthoringFailure::Control(error)) => match error {},
        Ok(batch) => panic!("expected node semantic failure, got {batch:?}"),
    }
}

#[test]
fn node_relation_limit_failures_preserve_the_exact_completed_prefix() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent), trs(parent(0, 0))];
    let input = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);

    let mut edge_control = RelationWorkControl::default();
    let edge_error = expect_node_semantic(validate_node_batch_with_control(
        input,
        SceneValidationLimits::default().with_max_edges(1),
        &mut edge_control,
    ));
    assert!(matches!(
        edge_error,
        NodeAuthoringSemanticError::LimitExceeded {
            limit: NodeAuthoringLimit::Edges,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(edge_control.graph_edges, 3);
    assert_eq!(edge_control.references, 1);

    let mut reference_control = RelationWorkControl::default();
    let reference_error = expect_node_semantic(validate_node_batch_with_control(
        input,
        SceneValidationLimits::default().with_max_references(1),
        &mut reference_control,
    ));
    assert!(matches!(
        reference_error,
        NodeAuthoringSemanticError::LimitExceeded {
            limit: NodeAuthoringLimit::References,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(reference_control.graph_edges, 3);
    assert_eq!(reference_control.references, 2);
}

fn expect_appearance_semantic(
    result: Result<
        super::super::plan::appearance::ValidatedAppearanceBatch<'_, ()>,
        AppearanceAuthoringFailure<Infallible>,
    >,
) -> AppearanceAuthoringSemanticError {
    match result {
        Err(AppearanceAuthoringFailure::Semantic(error)) => error,
        Err(AppearanceAuthoringFailure::Control(error)) => match error {},
        Ok(batch) => panic!("expected appearance semantic failure, got {batch:?}"),
    }
}

#[test]
fn appearance_relation_limit_failures_preserve_the_exact_completed_prefix() {
    let image_bytes = [1_u8];
    let images = [ImageAuthoring::encoded(&image_bytes, Presence::Absent)];
    let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
    let textures = [TextureAuthoring::new(
        ImageLocalId::new(0),
        SamplerLocalId::new(0),
    )];
    let input = AppearanceAuthoringBatch::new(&images, &samplers, &textures, &[], &[]);

    let mut edge_control = RelationWorkControl::default();
    let edge_error = expect_appearance_semantic(validate_appearance_batch_with_control(
        input,
        SceneValidationLimits::default().with_max_edges(1),
        &mut edge_control,
    ));
    assert!(matches!(
        edge_error,
        AppearanceAuthoringSemanticError::LimitExceeded {
            limit: AppearanceAuthoringLimit::Edges,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(edge_control.graph_edges, 2);
    assert_eq!(edge_control.references, 1);

    let mut reference_control = RelationWorkControl::default();
    let reference_error = expect_appearance_semantic(validate_appearance_batch_with_control(
        input,
        SceneValidationLimits::default().with_max_references(1),
        &mut reference_control,
    ));
    assert!(matches!(
        reference_error,
        AppearanceAuthoringSemanticError::LimitExceeded {
            limit: AppearanceAuthoringLimit::References,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(reference_control.graph_edges, 2);
    assert_eq!(reference_control.references, 2);
}

fn expect_geometry_semantic(
    result: Result<
        super::super::plan::geometry::ValidatedGeometryBatch<'_, ()>,
        GeometryAuthoringFailure<Infallible>,
    >,
) -> GeometryAuthoringSemanticError {
    match result {
        Err(GeometryAuthoringFailure::Semantic(error)) => error,
        Err(GeometryAuthoringFailure::Control(error)) => match error {},
        Ok(batch) => panic!("expected geometry semantic failure, got {batch:?}"),
    }
}

fn validate_two_geometry_relations(
    limits: SceneValidationLimits,
    control: &mut RelationWorkControl,
) -> GeometryAuthoringSemanticError {
    let positions = [0.0_f32, 0.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];

    expect_geometry_semantic(validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]),
        0,
        0,
        limits,
        control,
    ))
}

#[test]
fn geometry_relation_limit_failures_preserve_the_exact_completed_prefix() {
    let mut edge_control = RelationWorkControl::default();
    let edge_error = validate_two_geometry_relations(
        SceneValidationLimits::default().with_max_edges(1),
        &mut edge_control,
    );
    assert!(matches!(
        edge_error,
        GeometryAuthoringSemanticError::LimitExceeded {
            limit: GeometryAuthoringLimit::Edges,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(edge_control.graph_edges, 2);
    assert_eq!(edge_control.references, 1);

    let mut reference_control = RelationWorkControl::default();
    let reference_error = validate_two_geometry_relations(
        SceneValidationLimits::default().with_max_references(1),
        &mut reference_control,
    );
    assert!(matches!(
        reference_error,
        GeometryAuthoringSemanticError::LimitExceeded {
            limit: GeometryAuthoringLimit::References,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(reference_control.graph_edges, 2);
    assert_eq!(reference_control.references, 2);
}

fn expect_animation_semantic(
    result: Result<
        super::super::plan::animation::ValidatedAnimationBatch<'_, ()>,
        AnimationAuthoringFailure<Infallible>,
    >,
) -> AnimationAuthoringSemanticError {
    match result {
        Err(AnimationAuthoringFailure::Semantic(error)) => error,
        Err(AnimationAuthoringFailure::Control(error)) => match error {},
        Ok(batch) => panic!("expected animation semantic failure, got {batch:?}"),
    }
}

#[test]
fn animation_relation_limit_failures_preserve_the_exact_completed_prefix() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut setup_control = TestControl::default();
    let validated_nodes = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("the animation target node must validate");
    let validated_geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        nodes.len(),
        0,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty geometry must validate for the animation target fixture");
    let validated_deformation = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::empty(),
        &validated_nodes,
        &validated_geometry,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty deformation must validate for the animation target fixture");
    let targets = AnimationTargetCatalog::new(&validated_nodes, &validated_deformation);
    let times = [0.0_f64, 1.0];
    let values = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let channels = [AnimationChannelAuthoring::new(
        AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
        AnimationInterpolationAuthoring::Linear,
        AnimationExtrapolationAuthoring::SourceDefault,
        AnimationExtrapolationAuthoring::SourceDefault,
        &times,
        &values,
        Presence::Absent,
        Presence::Absent,
    )];
    let clips = [AnimationClipAuthoring::new(
        1.0,
        AnimationTimeDomainAuthoring::Seconds,
        AuthoringRange::new(0, 1),
    )];
    let input = AnimationAuthoringBatch::new(&clips, &channels);

    let mut edge_control = RelationWorkControl::default();
    let edge_error = expect_animation_semantic(validate_animation_batch_with_control(
        input,
        &targets,
        SceneValidationLimits::default().with_max_edges(1),
        &mut edge_control,
    ));
    assert!(matches!(
        edge_error,
        AnimationAuthoringSemanticError::LimitExceeded {
            limit: AnimationAuthoringLimit::Edges,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(edge_control.graph_edges, 2);
    assert_eq!(edge_control.references, 1);

    let mut reference_control = RelationWorkControl::default();
    let reference_error = expect_animation_semantic(validate_animation_batch_with_control(
        input,
        &targets,
        SceneValidationLimits::default().with_max_references(1),
        &mut reference_control,
    ));
    assert!(matches!(
        reference_error,
        AnimationAuthoringSemanticError::LimitExceeded {
            limit: AnimationAuthoringLimit::References,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(reference_control.graph_edges, 2);
    assert_eq!(reference_control.references, 2);
}

fn expect_deformation_semantic(
    result: Result<
        super::super::plan::deformation::ValidatedDeformationBatch<'_, ()>,
        DeformationAuthoringFailure<Infallible>,
    >,
) -> DeformationAuthoringSemanticError {
    match result {
        Err(DeformationAuthoringFailure::Semantic(error)) => error,
        Err(DeformationAuthoringFailure::Control(error)) => match error {},
        Ok(batch) => panic!("expected deformation semantic failure, got {batch:?}"),
    }
}

#[test]
fn deformation_relation_limit_failures_preserve_the_exact_completed_prefix() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent), trs(parent(0, 0))];
    let mut setup_control = TestControl::default();
    let validated_nodes = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("the two skin-joint nodes must validate");
    let validated_geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        nodes.len(),
        0,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty geometry must validate for the skin fixture");
    let joints = [NodeLocalId::new(0), NodeLocalId::new(1)];
    let inverse_bind_matrices = [Mat4::IDENTITY, Mat4::IDENTITY];
    let skins = [SkinAuthoring::new(
        Presence::Absent,
        &joints,
        &inverse_bind_matrices,
        Presence::Absent,
        SkinMatrixSpaces::canonical(),
    )];
    let input = DeformationAuthoringBatch::new(&skins, &[], &[], &[], &[]);

    let mut edge_control = RelationWorkControl::default();
    let edge_error = expect_deformation_semantic(validate_deformation_batch_with_control(
        input,
        &validated_nodes,
        &validated_geometry,
        SceneValidationLimits::default().with_max_edges(1),
        &mut edge_control,
    ));
    assert!(matches!(
        edge_error,
        DeformationAuthoringSemanticError::LimitExceeded {
            limit: DeformationAuthoringLimit::Edges,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(edge_control.graph_edges, 2);
    assert_eq!(edge_control.references, 1);

    let mut reference_control = RelationWorkControl::default();
    let reference_error = expect_deformation_semantic(validate_deformation_batch_with_control(
        input,
        &validated_nodes,
        &validated_geometry,
        SceneValidationLimits::default().with_max_references(1),
        &mut reference_control,
    ));
    assert!(matches!(
        reference_error,
        DeformationAuthoringSemanticError::LimitExceeded {
            limit: DeformationAuthoringLimit::References,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(reference_control.graph_edges, 2);
    assert_eq!(reference_control.references, 2);
}

fn expect_spatial_semantic(
    result: Result<
        super::super::plan::spatial::ValidatedSpatialBatch<'_, ()>,
        SpatialAuthoringFailure<Infallible>,
    >,
) -> SpatialAuthoringSemanticError {
    match result {
        Err(SpatialAuthoringFailure::Semantic(error)) => error,
        Err(SpatialAuthoringFailure::Control(error)) => match error {},
        Ok(batch) => panic!("expected spatial semantic failure, got {batch:?}"),
    }
}

#[test]
fn spatial_relation_limit_failures_preserve_the_exact_completed_prefix() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Present(CameraLocalId::new(0)),
        Presence::Present(LightLocalId::new(0)),
    )];
    let cameras = [CameraAuthoring::new(
        CameraProjection::perspective(1.0, Some(16.0 / 9.0), 0.1, Some(100.0))
            .expect("the test camera projection must be valid"),
    )];
    let lights = [LightAuthoring::new(
        LightKind::directional(),
        LightEmission::try_from_scaled_linear_srgb(Color::WHITE, 10.0)
            .expect("the test light emission must be valid"),
    )];
    let input = SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &cameras, &lights);
    let node_input = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);

    let mut edge_control = RelationWorkControl::default();
    let edge_error = expect_spatial_semantic(validate_spatial_batch_with_control(
        input,
        node_input,
        SceneValidationLimits::default().with_max_edges(1),
        &mut edge_control,
    ));
    assert!(matches!(
        edge_error,
        SpatialAuthoringSemanticError::LimitExceeded {
            limit: SpatialAuthoringLimit::Edges,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(edge_control.graph_edges, 2);
    assert_eq!(edge_control.references, 1);

    let mut reference_control = RelationWorkControl::default();
    let reference_error = expect_spatial_semantic(validate_spatial_batch_with_control(
        input,
        node_input,
        SceneValidationLimits::default().with_max_references(1),
        &mut reference_control,
    ));
    assert!(matches!(
        reference_error,
        SpatialAuthoringSemanticError::LimitExceeded {
            limit: SpatialAuthoringLimit::References,
            actual: 2,
            maximum: 1,
        }
    ));
    assert_eq!(reference_control.graph_edges, 2);
    assert_eq!(reference_control.references, 2);
}

fn expect_profile_semantic(
    result: Result<
        super::super::plan::profile::ValidatedProfileBatch<'_, ()>,
        ProfileAuthoringFailure<Infallible>,
    >,
) -> ProfileAuthoringSemanticError {
    match result {
        Err(ProfileAuthoringFailure::Semantic(error)) => error,
        Err(ProfileAuthoringFailure::Control(error)) => match error {},
        Ok(_) => panic!("expected profile semantic failure"),
    }
}

#[test]
fn profile_relation_limit_failures_preserve_the_exact_completed_prefix() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let spatial_input = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let node_input = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance_input = AppearanceAuthoringBatch::empty();
    let geometry_input = GeometryAuthoringBatch::empty();
    let deformation_input = DeformationAuthoringBatch::empty();
    let animation_input = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial_input,
        node_input,
        appearance_input,
        geometry_input,
        deformation_input,
        animation_input,
    );
    let mut setup_control = TestControl::default();
    let validated_spatial = validate_spatial_batch_with_control(
        spatial_input,
        node_input,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty spatial input must validate for the profile owner catalog");
    let validated_nodes = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("the profile fixture node must validate");
    let validated_appearance = validate_appearance_batch_with_control(
        appearance_input,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty appearance must validate for the profile owner catalog");
    let validated_geometry = validate_geometry_batch_with_control(
        geometry_input,
        nodes.len(),
        validated_appearance.material_count(),
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty geometry must validate for the profile owner catalog");
    let validated_deformation = validate_deformation_batch_with_control(
        deformation_input,
        &validated_nodes,
        &validated_geometry,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty deformation must validate for the profile owner catalog");
    let targets = AnimationTargetCatalog::new(&validated_nodes, &validated_deformation);
    let validated_animation = validate_animation_batch_with_control(
        animation_input,
        &targets,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty animation must validate for the profile owner catalog");
    let owners = ProfileOwnerCatalog::from_validated(
        &validated_spatial,
        &validated_nodes,
        &validated_appearance,
        &validated_geometry,
        &validated_deformation,
        &validated_animation,
    )
    .expect("the validated fixture domains must form one profile owner catalog");

    let mut edge_control = RelationWorkControl::default();
    let edge_error = expect_profile_semantic(validate_profile_batch_with_control(
        profile.batch(),
        owners,
        SceneValidationLimits::default().with_max_edges(0),
        &mut edge_control,
    ));
    assert!(matches!(
        edge_error,
        ProfileAuthoringSemanticError::LimitExceeded {
            limit: ProfileAuthoringLimit::Edges,
            actual: 1,
            maximum: 0,
        }
    ));
    assert_eq!(edge_control.graph_edges, 1);
    assert_eq!(edge_control.references, 0);

    let mut reference_control = RelationWorkControl::default();
    let reference_error = expect_profile_semantic(validate_profile_batch_with_control(
        profile.batch(),
        owners,
        SceneValidationLimits::default().with_max_references(0),
        &mut reference_control,
    ));
    assert!(matches!(
        reference_error,
        ProfileAuthoringSemanticError::LimitExceeded {
            limit: ProfileAuthoringLimit::References,
            actual: 1,
            maximum: 0,
        }
    ));
    assert_eq!(reference_control.graph_edges, 1);
    assert_eq!(reference_control.references, 1);
}
