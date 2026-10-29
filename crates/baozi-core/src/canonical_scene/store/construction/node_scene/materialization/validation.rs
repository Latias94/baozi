//! Translate private authoring failures while their original semantic fields are still present.

use crate::canonical_scene::store::api::validation::{
    SceneAnimationRateKind, SceneAnimationSamplePart, SceneAppearanceRule, SceneLightEmissionRule,
    SceneProfileOwner, SceneValidationDomain, SceneValidationError, SceneValidationLimit,
};
use crate::canonical_scene::store::production::{
    AnimationAuthoringLimit, AnimationAuthoringSemanticError, AnimationRateKind,
    AnimationSamplePart, AppearanceAuthoringLimit, AppearanceAuthoringSemanticError,
    CanonicalProfileOwner, DeformationAuthoringLimit, DeformationAuthoringSemanticError,
    GeometryAuthoringLimit, GeometryAuthoringSemanticError, NodeAuthoringLimit,
    NodeAuthoringSemanticError, ProfileAuthoringLimit, ProfileAuthoringSemanticError,
    SceneAuthoringAggregateError, SceneAuthoringAggregateLimit, SpatialAuthoringLimit,
    SpatialAuthoringSemanticError,
};

pub(super) fn scene_validation_error(
    cause: &super::NodeSceneConstructionRejection<super::StoreRuntimeError>,
) -> Option<SceneValidationError> {
    use super::NodeSceneConstructionRejection as Rejection;
    use crate::canonical_scene::store::production::{
        AnimationAuthoringFailure, AppearanceAuthoringFailure, DeformationAuthoringFailure,
        GeometryAuthoringFailure, NodeAuthoringFailure, ProfileAuthoringFailure,
        SpatialAuthoringFailure,
    };

    match cause {
        Rejection::NodeAuthoring(NodeAuthoringFailure::Semantic(cause)) => {
            node_validation_error(cause)
        }
        Rejection::GeometryAuthoring(GeometryAuthoringFailure::Semantic(cause)) => {
            geometry_validation_error(cause)
        }
        Rejection::AppearanceAuthoring(AppearanceAuthoringFailure::Semantic(cause)) => {
            appearance_validation_error(cause)
        }
        Rejection::DeformationAuthoring(DeformationAuthoringFailure::Semantic(cause)) => {
            deformation_validation_error(cause)
        }
        Rejection::AnimationAuthoring(AnimationAuthoringFailure::Semantic(cause)) => {
            animation_validation_error(cause)
        }
        Rejection::SpatialAuthoring(SpatialAuthoringFailure::Semantic(cause)) => {
            spatial_validation_error(cause)
        }
        Rejection::ProfileAuthoring(ProfileAuthoringFailure::Semantic(cause)) => {
            profile_validation_error(cause)
        }
        Rejection::Aggregate(cause) => aggregate_validation_error(cause),
        _ => None,
    }
}

fn aggregate_validation_error(
    cause: &SceneAuthoringAggregateError,
) -> Option<SceneValidationError> {
    match cause {
        SceneAuthoringAggregateError::LimitExceeded {
            limit,
            actual,
            maximum,
        } => Some(SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Scene,
            limit: match limit {
                SceneAuthoringAggregateLimit::Entities => SceneValidationLimit::Entities,
                SceneAuthoringAggregateLimit::Edges => SceneValidationLimit::Edges,
                SceneAuthoringAggregateLimit::References => SceneValidationLimit::References,
                SceneAuthoringAggregateLimit::SemanticCells => SceneValidationLimit::SemanticCells,
                SceneAuthoringAggregateLimit::Work => SceneValidationLimit::Work,
                SceneAuthoringAggregateLimit::CanonicalBytes => {
                    SceneValidationLimit::CanonicalBytes
                }
                SceneAuthoringAggregateLimit::ScratchBytes => SceneValidationLimit::ScratchBytes,
            },
            actual: *actual,
            maximum: *maximum,
        }),
        SceneAuthoringAggregateError::AccountingOverflow { .. } => None,
    }
}

fn appearance_rule(cause: &crate::appearance::AppearanceBuildError) -> Option<SceneAppearanceRule> {
    use crate::appearance::AppearanceBuildError as Source;

    Some(match cause {
        Source::MissingFactorGroup { shading, required } => {
            SceneAppearanceRule::MissingFactorGroup {
                shading: *shading,
                required: *required,
            }
        }
        Source::IncompatibleColorInterpretation {
            role,
            interpretation,
        } => SceneAppearanceRule::IncompatibleColorInterpretation {
            role: *role,
            interpretation: *interpretation,
        },
        Source::IncompatibleChannelCount {
            role,
            minimum,
            maximum,
            actual,
        } => SceneAppearanceRule::IncompatibleChannelCount {
            role: *role,
            minimum: *minimum,
            maximum: *maximum,
            actual: *actual,
        },
        Source::IncompatibleStrength { role, requirement } => {
            SceneAppearanceRule::IncompatibleStrength {
                role: *role,
                requirement: *requirement,
            }
        }
        Source::BuiltInRoleRequiresReplace { role } => {
            SceneAppearanceRule::BuiltInRoleRequiresReplace { role: *role }
        }
        Source::DuplicateBindingAuthority {
            first_index,
            duplicate_index,
        } => SceneAppearanceRule::DuplicateBindingAuthority {
            first_index: *first_index,
            duplicate_index: *duplicate_index,
        },
        Source::ConflictingBindingRoles {
            first_index,
            first_role,
            duplicate_index,
            duplicate_role,
        } => SceneAppearanceRule::ConflictingBindingRoles {
            first_index: *first_index,
            first_role: *first_role,
            duplicate_index: *duplicate_index,
            duplicate_role: *duplicate_role,
        },
        Source::LimitExceeded {
            resource,
            dimension,
            requested,
            maximum,
        } => SceneAppearanceRule::LimitExceeded {
            resource: *resource,
            dimension: *dimension,
            requested: *requested,
            maximum: *maximum,
        },
        Source::AllocationFailed { .. } => return None,
    })
}

const fn animation_sample_part(part: AnimationSamplePart) -> SceneAnimationSamplePart {
    match part {
        AnimationSamplePart::Values => SceneAnimationSamplePart::Values,
        AnimationSamplePart::InTangents => SceneAnimationSamplePart::InTangents,
        AnimationSamplePart::OutTangents => SceneAnimationSamplePart::OutTangents,
    }
}

const fn light_emission_rule(cause: crate::entity::LightEmissionError) -> SceneLightEmissionRule {
    use crate::entity::LightEmissionError as Source;
    match cause {
        Source::InvalidColor => SceneLightEmissionRule::InvalidColor,
        Source::InvalidAlpha => SceneLightEmissionRule::InvalidAlpha,
        Source::InvalidIntensity => SceneLightEmissionRule::InvalidIntensity,
        Source::NonCanonicalZero => SceneLightEmissionRule::NonCanonicalZero,
        Source::NonCanonicalTint => SceneLightEmissionRule::NonCanonicalTint,
        Source::Unrepresentable => SceneLightEmissionRule::Unrepresentable,
    }
}

const fn profile_owner(owner: CanonicalProfileOwner) -> SceneProfileOwner {
    match owner {
        CanonicalProfileOwner::Scene => SceneProfileOwner::Scene,
        CanonicalProfileOwner::SyntheticRoot => SceneProfileOwner::SyntheticRoot,
        CanonicalProfileOwner::Node(index) => SceneProfileOwner::Node(index),
        CanonicalProfileOwner::Camera(index) => SceneProfileOwner::Camera(index),
        CanonicalProfileOwner::Light(index) => SceneProfileOwner::Light(index),
        CanonicalProfileOwner::Image(index) => SceneProfileOwner::Image(index),
        CanonicalProfileOwner::Sampler(index) => SceneProfileOwner::Sampler(index),
        CanonicalProfileOwner::Texture(index) => SceneProfileOwner::Texture(index),
        CanonicalProfileOwner::TextureBinding(index) => SceneProfileOwner::TextureBinding(index),
        CanonicalProfileOwner::Material(index) => SceneProfileOwner::Material(index),
        CanonicalProfileOwner::LogicalMesh(index) => SceneProfileOwner::LogicalMesh(index),
        CanonicalProfileOwner::Primitive(index) => SceneProfileOwner::Primitive(index),
        CanonicalProfileOwner::Attribute(index) => SceneProfileOwner::Attribute(index),
        CanonicalProfileOwner::MeshInstance(index) => SceneProfileOwner::MeshInstance(index),
        CanonicalProfileOwner::Skin(index) => SceneProfileOwner::Skin(index),
        CanonicalProfileOwner::MorphDefinition(index) => SceneProfileOwner::MorphDefinition(index),
        CanonicalProfileOwner::MorphDelta(index) => SceneProfileOwner::MorphDelta(index),
        CanonicalProfileOwner::AnimationClip(index) => SceneProfileOwner::AnimationClip(index),
        CanonicalProfileOwner::AnimationChannel(index) => {
            SceneProfileOwner::AnimationChannel(index)
        }
    }
}

pub(super) fn node_validation_error(
    cause: &NodeAuthoringSemanticError,
) -> Option<SceneValidationError> {
    use NodeAuthoringSemanticError as Source;
    Some(match cause {
        Source::RootsAbsent => SceneValidationError::RootsAbsent,
        Source::NodeDomainTooLarge { authored_nodes } => SceneValidationError::NodeDomainTooLarge {
            authored_nodes: *authored_nodes,
        },
        Source::TransformDomainTooLarge { scalar_count } => {
            SceneValidationError::TransformDomainTooLarge {
                scalar_count: *scalar_count,
            }
        }
        Source::RootOutOfRange {
            root_index,
            root,
            node_count,
        } => SceneValidationError::RootOutOfRange {
            root_index: *root_index,
            root: *root,
            node_count: *node_count,
        },
        Source::DuplicateRoot {
            root,
            duplicate_index,
        } => SceneValidationError::DuplicateRoot {
            root: *root,
            duplicate_index: *duplicate_index,
        },
        Source::RootHasParent { root, parent } => SceneValidationError::RootHasParent {
            root: *root,
            parent: *parent,
        },
        Source::MissingRoot { node } => SceneValidationError::MissingRoot { node: *node },
        Source::ParentOutOfRange {
            node,
            parent,
            node_count,
        } => SceneValidationError::ParentOutOfRange {
            node: *node,
            parent: *parent,
            node_count: *node_count,
        },
        Source::SelfParent { node } => SceneValidationError::SelfParent { node: *node },
        Source::InvalidName { node, cause } => SceneValidationError::InvalidName {
            node: *node,
            cause: *cause,
        },
        Source::Cycle { node } => SceneValidationError::Cycle { node: *node },
        Source::ChildOrdinalOutOfRange {
            node,
            parent,
            ordinal,
            child_count,
        } => SceneValidationError::ChildOrdinalOutOfRange {
            node: *node,
            parent: *parent,
            ordinal: *ordinal,
            child_count: *child_count,
        },
        Source::DuplicateChildOrdinal {
            parent,
            ordinal,
            first,
            duplicate,
        } => SceneValidationError::DuplicateChildOrdinal {
            parent: *parent,
            ordinal: *ordinal,
            first: *first,
            duplicate: *duplicate,
        },
        Source::LimitExceeded {
            limit,
            actual,
            maximum,
        } => SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Node,
            limit: match limit {
                NodeAuthoringLimit::Nodes => SceneValidationLimit::Nodes,
                NodeAuthoringLimit::Edges => SceneValidationLimit::Edges,
                NodeAuthoringLimit::Depth => SceneValidationLimit::Depth,
                NodeAuthoringLimit::SemanticCells => SceneValidationLimit::SemanticCells,
                NodeAuthoringLimit::References => SceneValidationLimit::References,
                NodeAuthoringLimit::CanonicalBytes => SceneValidationLimit::CanonicalBytes,
                NodeAuthoringLimit::Work => SceneValidationLimit::Work,
                NodeAuthoringLimit::ScratchBytes => SceneValidationLimit::ScratchBytes,
            },
            actual: *actual,
            maximum: *maximum,
        },
        Source::AccountingOverflow { .. }
        | Source::ScratchCapacity { .. }
        | Source::ScratchAllocation { .. } => return None,
    })
}

pub(super) fn geometry_validation_error(
    cause: &GeometryAuthoringSemanticError,
) -> Option<SceneValidationError> {
    use GeometryAuthoringSemanticError as Source;
    Some(match cause {
        Source::InvalidLogicalMeshName {
            logical_mesh,
            source,
        } => SceneValidationError::InvalidLogicalMeshName {
            logical_mesh: *logical_mesh,
            source: *source,
        },
        Source::EmptyLogicalMesh { logical_mesh } => SceneValidationError::EmptyLogicalMesh {
            logical_mesh: *logical_mesh,
        },
        Source::LogicalMeshRangeOrder {
            logical_mesh,
            expected,
            actual,
        } => SceneValidationError::LogicalMeshRangeOrder {
            logical_mesh: *logical_mesh,
            expected: *expected,
            actual: *actual,
        },
        Source::LogicalMeshRangeOutOfBounds {
            logical_mesh,
            primitive_count,
        } => SceneValidationError::LogicalMeshRangeOutOfBounds {
            logical_mesh: *logical_mesh,
            primitive_count: *primitive_count,
        },
        Source::UnownedPrimitives { unowned } => {
            SceneValidationError::UnownedPrimitives { unowned: *unowned }
        }
        Source::PrimitiveAttributeRangeOrder {
            primitive,
            expected,
            actual,
        } => SceneValidationError::PrimitiveAttributeRangeOrder {
            primitive: *primitive,
            expected: *expected,
            actual: *actual,
        },
        Source::PrimitiveAttributeRangeOutOfBounds {
            primitive,
            attribute_count,
        } => SceneValidationError::PrimitiveAttributeRangeOutOfBounds {
            primitive: *primitive,
            attribute_count: *attribute_count,
        },
        Source::UnownedAttributes { unowned } => {
            SceneValidationError::UnownedAttributes { unowned: *unowned }
        }
        Source::MissingPosition {
            primitive,
            vertex_count,
        } => SceneValidationError::MissingPosition {
            primitive: *primitive,
            vertex_count: *vertex_count,
        },
        Source::DuplicateAttributeKey {
            primitive,
            attribute,
        } => SceneValidationError::DuplicateAttributeKey {
            primitive: *primitive,
            attribute: *attribute,
        },
        Source::AttributeOrder {
            primitive,
            attribute,
        } => SceneValidationError::AttributeOrder {
            primitive: *primitive,
            attribute: *attribute,
        },
        Source::ZeroAttributeLanes { attribute } => SceneValidationError::ZeroAttributeLanes {
            attribute: *attribute,
        },
        Source::AttributeScalarShape { attribute, lanes } => {
            SceneValidationError::AttributeScalarShape {
                attribute: *attribute,
                lanes: *lanes,
            }
        }
        Source::AttributeSchema { attribute } => SceneValidationError::AttributeSchema {
            attribute: *attribute,
        },
        Source::NonFiniteAttributeValue { attribute, scalar } => {
            SceneValidationError::NonFiniteAttributeValue {
                attribute: *attribute,
                scalar: *scalar,
            }
        }
        Source::SourceProjectionDomain { attribute } => {
            SceneValidationError::SourceProjectionDomain {
                attribute: *attribute,
            }
        }
        Source::SourceProjectionCardinality {
            attribute,
            expected,
            actual,
        } => SceneValidationError::SourceProjectionCardinality {
            attribute: *attribute,
            expected: *expected,
            actual: *actual,
        },
        Source::SourceProjectionOutOfRange {
            attribute,
            element,
            index,
            source_count,
        } => SceneValidationError::SourceProjectionOutOfRange {
            attribute: *attribute,
            element: *element,
            index: *index,
            source_count: *source_count,
        },
        Source::AttributeCardinality {
            attribute,
            domain,
            expected,
            actual,
        } => SceneValidationError::AttributeCardinality {
            attribute: *attribute,
            domain: *domain,
            expected: *expected,
            actual: *actual,
        },
        Source::SparseDefaultShape {
            attribute,
            lanes,
            actual,
        } => SceneValidationError::SparseDefaultShape {
            attribute: *attribute,
            lanes: *lanes,
            actual: *actual,
        },
        Source::SparseValueCount {
            attribute,
            index_count,
            value_elements,
        } => SceneValidationError::SparseValueCount {
            attribute: *attribute,
            index_count: *index_count,
            value_elements: *value_elements,
        },
        Source::SparseIndexOutOfRange {
            attribute,
            element,
            index,
            logical_count,
        } => SceneValidationError::SparseIndexOutOfRange {
            attribute: *attribute,
            element: *element,
            index: *index,
            logical_count: *logical_count,
        },
        Source::SparseIndexOrder { attribute, element } => SceneValidationError::SparseIndexOrder {
            attribute: *attribute,
            element: *element,
        },
        Source::VertexIndexOutOfRange {
            primitive,
            corner,
            index,
            vertex_count,
        } => SceneValidationError::VertexIndexOutOfRange {
            primitive: *primitive,
            corner: *corner,
            index: *index,
            vertex_count: *vertex_count,
        },
        Source::ListCornerCount {
            primitive,
            topology,
            corner_count,
            corners_per_face,
        } => SceneValidationError::ListCornerCount {
            primitive: *primitive,
            topology: *topology,
            corner_count: *corner_count,
            corners_per_face: *corners_per_face,
        },
        Source::PolygonFaceTooSmall {
            primitive,
            face,
            corner_count,
        } => SceneValidationError::PolygonFaceTooSmall {
            primitive: *primitive,
            face: *face,
            corner_count: *corner_count,
        },
        Source::PolygonCornerCountOverflow { primitive, face } => {
            SceneValidationError::PolygonCornerCountOverflow {
                primitive: *primitive,
                face: *face,
            }
        }
        Source::PolygonCornerCountMismatch {
            primitive,
            boundary_corners,
            corner_count,
        } => SceneValidationError::PolygonCornerCountMismatch {
            primitive: *primitive,
            boundary_corners: *boundary_corners,
            corner_count: *corner_count,
        },
        Source::MaterialOutOfRange {
            primitive,
            material,
            appearance_material_count,
        } => SceneValidationError::MaterialOutOfRange {
            primitive: *primitive,
            material: *material,
            appearance_material_count: *appearance_material_count,
        },
        Source::InstanceOwnerOutOfRange {
            instance,
            owner,
            node_count,
        } => SceneValidationError::InstanceOwnerOutOfRange {
            instance: *instance,
            owner: *owner,
            node_count: *node_count,
        },
        Source::InstanceMeshOutOfRange {
            instance,
            logical_mesh,
            logical_mesh_count,
        } => SceneValidationError::InstanceMeshOutOfRange {
            instance: *instance,
            logical_mesh: *logical_mesh,
            logical_mesh_count: *logical_mesh_count,
        },
        Source::LimitExceeded {
            limit,
            actual,
            maximum,
        } => SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Geometry,
            limit: match limit {
                GeometryAuthoringLimit::Entities => SceneValidationLimit::Entities,
                GeometryAuthoringLimit::Edges => SceneValidationLimit::Edges,
                GeometryAuthoringLimit::References => SceneValidationLimit::References,
                GeometryAuthoringLimit::SemanticCells => SceneValidationLimit::SemanticCells,
                GeometryAuthoringLimit::CanonicalBytes => SceneValidationLimit::CanonicalBytes,
                GeometryAuthoringLimit::Work => SceneValidationLimit::Work,
                GeometryAuthoringLimit::ScratchBytes => SceneValidationLimit::ScratchBytes,
            },
            actual: *actual,
            maximum: *maximum,
        },
        Source::AccountingOverflow { .. }
        | Source::ScratchCapacity { .. }
        | Source::ScratchAllocation { .. } => return None,
        Source::CountDomainTooLarge { .. } => return None,
    })
}

pub(super) fn appearance_validation_error(
    cause: &AppearanceAuthoringSemanticError,
) -> Option<SceneValidationError> {
    use AppearanceAuthoringSemanticError as Source;
    Some(match cause {
        Source::EmptyEncodedPayload { image } => {
            SceneValidationError::EmptyEncodedPayload { image: *image }
        }
        Source::EncodedPayloadTooLarge {
            image,
            actual,
            maximum,
        } => SceneValidationError::EncodedPayloadTooLarge {
            image: *image,
            actual: *actual,
            maximum: *maximum,
        },
        Source::ResourceSnapshotCardinalityMismatch {
            snapshot_count,
            resource_count,
        } => SceneValidationError::ResourceSnapshotCardinalityMismatch {
            snapshot_count: *snapshot_count,
            resource_count: *resource_count,
        },
        Source::ImageResourceOutOfRange {
            image,
            resource,
            resource_count,
        } => SceneValidationError::ImageResourceOutOfRange {
            image: *image,
            resource: *resource,
            resource_count: *resource_count,
        },
        Source::ImageResourceSnapshotAbsent { image, resource } => {
            SceneValidationError::ImageResourceSnapshotAbsent {
                image: *image,
                resource: *resource,
            }
        }
        Source::MaterialBindingRangeOrder {
            material,
            expected,
            actual,
        } => SceneValidationError::MaterialBindingRangeOrder {
            material: *material,
            expected: *expected,
            actual: *actual,
        },
        Source::MaterialBindingRangeOutOfBounds {
            material,
            binding_count,
        } => SceneValidationError::MaterialBindingRangeOutOfBounds {
            material: *material,
            binding_count: *binding_count,
        },
        Source::UnownedBindings { unowned } => {
            SceneValidationError::UnownedBindings { unowned: *unowned }
        }
        Source::TextureImageOutOfRange {
            texture,
            image,
            image_count,
        } => SceneValidationError::TextureImageOutOfRange {
            texture: *texture,
            image: *image,
            image_count: *image_count,
        },
        Source::TextureSamplerOutOfRange {
            texture,
            sampler,
            sampler_count,
        } => SceneValidationError::TextureSamplerOutOfRange {
            texture: *texture,
            sampler: *sampler,
            sampler_count: *sampler_count,
        },
        Source::BindingTextureOutOfRange {
            binding,
            texture,
            texture_count,
        } => SceneValidationError::BindingTextureOutOfRange {
            binding: *binding,
            texture: *texture,
            texture_count: *texture_count,
        },
        Source::NamespacedTextureRoleUnsupported { binding } => {
            SceneValidationError::NamespacedTextureRoleUnsupported { binding: *binding }
        }
        Source::NamespacedBindingBehaviorUnsupported { binding, behavior } => {
            SceneValidationError::NamespacedBindingBehaviorUnsupported {
                binding: *binding,
                behavior: *behavior,
            }
        }
        Source::InvalidBindingSemantics { binding, source } => {
            SceneValidationError::InvalidBindingSemantics {
                binding: *binding,
                source: appearance_rule(source)?,
            }
        }
        Source::InvalidMaterialSemantics { material, source } => {
            SceneValidationError::InvalidMaterialSemantics {
                material: *material,
                source: appearance_rule(source)?,
            }
        }
        Source::InvalidMaterialName { material, source } => {
            SceneValidationError::InvalidMaterialName {
                material: *material,
                source: *source,
            }
        }
        Source::NamespacedMaterialBehaviorUnsupported { material, behavior } => {
            SceneValidationError::NamespacedMaterialBehaviorUnsupported {
                material: *material,
                behavior: *behavior,
            }
        }
        Source::InvalidMaterialBindingSemantics { material, source } => {
            SceneValidationError::InvalidMaterialBindingSemantics {
                material: *material,
                source: appearance_rule(source)?,
            }
        }
        Source::CountDomainTooLarge { collection } => {
            SceneValidationError::CollectionDomainTooLarge {
                domain: SceneValidationDomain::Appearance,
                collection,
            }
        }
        Source::LimitExceeded {
            limit,
            actual,
            maximum,
        } => SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Appearance,
            limit: match limit {
                AppearanceAuthoringLimit::Entities => SceneValidationLimit::Entities,
                AppearanceAuthoringLimit::Edges => SceneValidationLimit::Edges,
                AppearanceAuthoringLimit::References => SceneValidationLimit::References,
                AppearanceAuthoringLimit::SemanticCells => SceneValidationLimit::SemanticCells,
                AppearanceAuthoringLimit::CanonicalBytes => SceneValidationLimit::CanonicalBytes,
                AppearanceAuthoringLimit::Work => SceneValidationLimit::Work,
                AppearanceAuthoringLimit::ScratchBytes => SceneValidationLimit::ScratchBytes,
            },
            actual: *actual,
            maximum: *maximum,
        },
        Source::AccountingOverflow { .. }
        | Source::BindingEncodingInvariant { .. }
        | Source::MaterialEncodingInvariant { .. }
        | Source::ScratchCapacity { .. }
        | Source::ScratchAllocation { .. } => return None,
    })
}

pub(super) fn deformation_validation_error(
    cause: &DeformationAuthoringSemanticError,
) -> Option<SceneValidationError> {
    use DeformationAuthoringSemanticError as Source;
    Some(match cause {
        Source::GeometryTableCardinality {
            table,
            expected,
            actual,
        } => SceneValidationError::GeometryTableCardinality {
            table,
            expected: *expected,
            actual: *actual,
        },
        Source::NonCanonicalSkinMatrixSpaces { skin } => {
            SceneValidationError::NonCanonicalSkinMatrixSpaces { skin: *skin }
        }
        Source::InverseBindCountMismatch {
            skin,
            joint_count,
            inverse_bind_count,
        } => SceneValidationError::InverseBindCountMismatch {
            skin: *skin,
            joint_count: *joint_count,
            inverse_bind_count: *inverse_bind_count,
        },
        Source::JointNodeOutOfRange {
            skin,
            joint,
            node,
            node_count,
        } => SceneValidationError::JointNodeOutOfRange {
            skin: *skin,
            joint: *joint,
            node: *node,
            node_count: *node_count,
        },
        Source::SkeletonRootOutOfRange {
            skin,
            node,
            node_count,
        } => SceneValidationError::SkeletonRootOutOfRange {
            skin: *skin,
            node: *node,
            node_count: *node_count,
        },
        Source::NonFiniteInverseBind { skin, joint } => {
            SceneValidationError::NonFiniteInverseBind {
                skin: *skin,
                joint: *joint,
            }
        }
        Source::NonAffineInverseBind { skin, joint } => {
            SceneValidationError::NonAffineInverseBind {
                skin: *skin,
                joint: *joint,
            }
        }
        Source::NonFiniteSkinCorrection { skin } => {
            SceneValidationError::NonFiniteSkinCorrection { skin: *skin }
        }
        Source::NonAffineSkinCorrection { skin } => {
            SceneValidationError::NonAffineSkinCorrection { skin: *skin }
        }
        Source::ContradictorySkinCorrectionSpaces {
            skin,
            from,
            to,
            expected_from,
            expected_to,
        } => SceneValidationError::ContradictorySkinCorrectionSpaces {
            skin: *skin,
            from: *from,
            to: *to,
            expected_from: *expected_from,
            expected_to: *expected_to,
        },
        Source::MorphDefinitionRangeOrder {
            logical_mesh,
            expected,
            actual,
        } => SceneValidationError::MorphDefinitionRangeOrder {
            logical_mesh: *logical_mesh,
            expected: *expected,
            actual: *actual,
        },
        Source::MorphDefinitionRangeOutOfRange {
            logical_mesh,
            offset,
            end,
            definition_count,
        } => SceneValidationError::MorphDefinitionRangeOutOfRange {
            logical_mesh: *logical_mesh,
            offset: *offset,
            end: *end,
            definition_count: *definition_count,
        },
        Source::UnownedMorphDefinitions {
            owned,
            definition_count,
        } => SceneValidationError::UnownedMorphDefinitions {
            owned: *owned,
            definition_count: *definition_count,
        },
        Source::LogicalMorphWidthMismatch {
            logical_mesh,
            definition_count,
            default_count,
        } => SceneValidationError::LogicalMorphWidthMismatch {
            logical_mesh: *logical_mesh,
            definition_count: *definition_count,
            default_count: *default_count,
        },
        Source::DuplicateMorphTargetIdentity {
            logical_mesh,
            target,
            identity,
        } => SceneValidationError::DuplicateMorphTargetIdentity {
            logical_mesh: *logical_mesh,
            target: *target,
            identity: *identity,
        },
        Source::MorphTargetIdentityOutOfRange {
            logical_mesh,
            target,
            identity,
            target_count,
        } => SceneValidationError::MorphTargetIdentityOutOfRange {
            logical_mesh: *logical_mesh,
            target: *target,
            identity: *identity,
            target_count: *target_count,
        },
        Source::NonFiniteDefaultMorphWeight {
            logical_mesh,
            target,
        } => SceneValidationError::NonFiniteDefaultMorphWeight {
            logical_mesh: *logical_mesh,
            target: *target,
        },
        Source::PrimitiveMorphTargetCountMismatch {
            primitive,
            expected,
            actual,
        } => SceneValidationError::PrimitiveMorphTargetCountMismatch {
            primitive: *primitive,
            expected: *expected,
            actual: *actual,
        },
        Source::InfluenceOffsetCountMismatch {
            primitive,
            expected,
            actual,
        } => SceneValidationError::InfluenceOffsetCountMismatch {
            primitive: *primitive,
            expected: *expected,
            actual: *actual,
        },
        Source::InfluenceFirstOffset { primitive, actual } => {
            SceneValidationError::InfluenceFirstOffset {
                primitive: *primitive,
                actual: *actual,
            }
        }
        Source::InfluenceOffsetOutOfRange {
            primitive,
            position,
            offset,
            influence_count,
        } => SceneValidationError::InfluenceOffsetOutOfRange {
            primitive: *primitive,
            position: *position,
            offset: *offset,
            influence_count: *influence_count,
        },
        Source::InfluenceOffsetOrder {
            primitive,
            position,
            previous,
            current,
        } => SceneValidationError::InfluenceOffsetOrder {
            primitive: *primitive,
            position: *position,
            previous: *previous,
            current: *current,
        },
        Source::InfluenceFinalOffset {
            primitive,
            expected,
            actual,
        } => SceneValidationError::InfluenceFinalOffset {
            primitive: *primitive,
            expected: *expected,
            actual: *actual,
        },
        Source::NonFiniteInfluenceWeight {
            primitive,
            influence,
        } => SceneValidationError::NonFiniteInfluenceWeight {
            primitive: *primitive,
            influence: *influence,
        },
        Source::NegativeInfluenceWeight {
            primitive,
            influence,
        } => SceneValidationError::NegativeInfluenceWeight {
            primitive: *primitive,
            influence: *influence,
        },
        Source::DuplicateInfluenceSourceOrder {
            primitive,
            vertex,
            source_order,
        } => SceneValidationError::DuplicateInfluenceSourceOrder {
            primitive: *primitive,
            vertex: *vertex,
            source_order: *source_order,
        },
        Source::InfluenceSourceOrderOutOfRange {
            primitive,
            vertex,
            influence,
            source_order,
            row_width,
        } => SceneValidationError::InfluenceSourceOrderOutOfRange {
            primitive: *primitive,
            vertex: *vertex,
            influence: *influence,
            source_order: *source_order,
            row_width: *row_width,
        },
        Source::DuplicateMorphSemantic {
            primitive,
            target,
            delta,
            semantic,
        } => SceneValidationError::DuplicateMorphSemantic {
            primitive: *primitive,
            target: *target,
            delta: *delta,
            semantic: *semantic,
        },
        Source::MorphSemanticOrder {
            primitive,
            target,
            delta,
        } => SceneValidationError::MorphSemanticOrder {
            primitive: *primitive,
            target: *target,
            delta: *delta,
        },
        Source::DenseMorphScalarCount {
            primitive,
            target,
            semantic,
            expected,
            actual,
        } => SceneValidationError::DenseMorphScalarCount {
            primitive: *primitive,
            target: *target,
            semantic: *semantic,
            expected: *expected,
            actual: *actual,
        },
        Source::SparseMorphLogicalCount {
            primitive,
            target,
            semantic,
            expected,
            actual,
        } => SceneValidationError::SparseMorphLogicalCount {
            primitive: *primitive,
            target: *target,
            semantic: *semantic,
            expected: *expected,
            actual: *actual,
        },
        Source::SparseMorphScalarCount {
            primitive,
            target,
            semantic,
            index_count,
            actual,
        } => SceneValidationError::SparseMorphScalarCount {
            primitive: *primitive,
            target: *target,
            semantic: *semantic,
            index_count: *index_count,
            actual: *actual,
        },
        Source::SparseMorphIndexOutOfRange {
            primitive,
            target,
            semantic,
            element,
            index,
            logical_count,
        } => SceneValidationError::SparseMorphIndexOutOfRange {
            primitive: *primitive,
            target: *target,
            semantic: *semantic,
            element: *element,
            index: *index,
            logical_count: *logical_count,
        },
        Source::SparseMorphIndexOrder {
            primitive,
            target,
            semantic,
            element,
        } => SceneValidationError::SparseMorphIndexOrder {
            primitive: *primitive,
            target: *target,
            semantic: *semantic,
            element: *element,
        },
        Source::NonFiniteMorphDelta {
            primitive,
            target,
            semantic,
            scalar,
        } => SceneValidationError::NonFiniteMorphDelta {
            primitive: *primitive,
            target: *target,
            semantic: *semantic,
            scalar: *scalar,
        },
        Source::InstanceSkinOutOfRange {
            instance,
            skin,
            skin_count,
        } => SceneValidationError::InstanceSkinOutOfRange {
            instance: *instance,
            skin: *skin,
            skin_count: *skin_count,
        },
        Source::InstanceMorphWidth {
            instance,
            expected,
            actual,
        } => SceneValidationError::InstanceMorphWidth {
            instance: *instance,
            expected: *expected,
            actual: *actual,
        },
        Source::NonFiniteInstanceMorphWeight { instance, target } => {
            SceneValidationError::NonFiniteInstanceMorphWeight {
                instance: *instance,
                target: *target,
            }
        }
        Source::PaletteJointOutOfRange {
            instance,
            logical_mesh,
            skin,
            joint_count,
            primitive,
            influence,
            palette_joint,
        } => SceneValidationError::PaletteJointOutOfRange {
            instance: *instance,
            logical_mesh: *logical_mesh,
            skin: *skin,
            joint_count: *joint_count,
            primitive: *primitive,
            influence: *influence,
            palette_joint: *palette_joint,
        },
        Source::CountDomainTooLarge { collection } => {
            SceneValidationError::CollectionDomainTooLarge {
                domain: SceneValidationDomain::Deformation,
                collection,
            }
        }
        Source::LimitExceeded {
            limit,
            actual,
            maximum,
        } => SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Deformation,
            limit: match limit {
                DeformationAuthoringLimit::Entities => SceneValidationLimit::Entities,
                DeformationAuthoringLimit::Edges => SceneValidationLimit::Edges,
                DeformationAuthoringLimit::References => SceneValidationLimit::References,
                DeformationAuthoringLimit::SemanticCells => SceneValidationLimit::SemanticCells,
                DeformationAuthoringLimit::CanonicalBytes => SceneValidationLimit::CanonicalBytes,
                DeformationAuthoringLimit::Work => SceneValidationLimit::Work,
                DeformationAuthoringLimit::ScratchBytes => SceneValidationLimit::ScratchBytes,
            },
            actual: *actual,
            maximum: *maximum,
        },
        Source::AccountingOverflow { .. }
        | Source::ScratchCapacity { .. }
        | Source::ScratchAllocation { .. } => return None,
    })
}

pub(super) fn animation_validation_error(
    cause: &AnimationAuthoringSemanticError,
) -> Option<SceneValidationError> {
    use AnimationAuthoringSemanticError as Source;
    Some(match cause {
        Source::ClipChannelRangeOrder {
            clip,
            expected,
            actual,
        } => SceneValidationError::ClipChannelRangeOrder {
            clip: *clip,
            expected: *expected,
            actual: *actual,
        },
        Source::ClipChannelRangeOutOfBounds {
            clip,
            channel_count,
        } => SceneValidationError::ClipChannelRangeOutOfBounds {
            clip: *clip,
            channel_count: *channel_count,
        },
        Source::ClipSourceIndexOrder {
            clip,
            previous,
            actual,
        } => SceneValidationError::ClipSourceIndexOrder {
            clip: *clip,
            previous: *previous,
            actual: *actual,
        },
        Source::ChannelSourceIndexOrder {
            channel,
            previous,
            actual,
        } => SceneValidationError::ChannelSourceIndexOrder {
            channel: *channel,
            previous: *previous,
            actual: *actual,
        },
        Source::UnownedChannels { unowned } => {
            SceneValidationError::UnownedChannels { unowned: *unowned }
        }
        Source::InvalidDuration { clip } => SceneValidationError::InvalidDuration { clip: *clip },
        Source::InvalidClipName { clip, source } => SceneValidationError::InvalidClipName {
            clip: *clip,
            source: *source,
        },
        Source::InvalidRate { clip, kind } => SceneValidationError::InvalidRate {
            clip: *clip,
            kind: match kind {
                AnimationRateKind::Frames => SceneAnimationRateKind::Frames,
                AnimationRateKind::Ticks => SceneAnimationRateKind::Ticks,
            },
        },
        Source::EmptyChannel { clip, channel } => SceneValidationError::EmptyChannel {
            clip: *clip,
            channel: *channel,
        },
        Source::NonFiniteTime { clip, channel, key } => SceneValidationError::NonFiniteTime {
            clip: *clip,
            channel: *channel,
            key: *key,
        },
        Source::TimesNotStrictlyIncreasing {
            clip,
            channel,
            previous_key,
            key,
        } => SceneValidationError::TimesNotStrictlyIncreasing {
            clip: *clip,
            channel: *channel,
            previous_key: *previous_key,
            key: *key,
        },
        Source::NodeTargetOutOfRange {
            clip,
            channel,
            node,
            node_count,
        } => SceneValidationError::NodeTargetOutOfRange {
            clip: *clip,
            channel: *channel,
            node: *node,
            node_count: *node_count,
        },
        Source::MatrixAuthoredNodeTarget {
            clip,
            channel,
            node,
        } => SceneValidationError::MatrixAuthoredNodeTarget {
            clip: *clip,
            channel: *channel,
            node: *node,
        },
        Source::MeshInstanceTargetOutOfRange {
            clip,
            channel,
            mesh_instance,
            mesh_instance_count,
        } => SceneValidationError::MeshInstanceTargetOutOfRange {
            clip: *clip,
            channel: *channel,
            mesh_instance: *mesh_instance,
            mesh_instance_count: *mesh_instance_count,
        },
        Source::MorphTargetWidthZero {
            clip,
            channel,
            mesh_instance,
        } => SceneValidationError::MorphTargetWidthZero {
            clip: *clip,
            channel: *channel,
            mesh_instance: *mesh_instance,
        },
        Source::ValueShape {
            clip,
            channel,
            expected,
            actual,
        } => SceneValidationError::ValueShape {
            clip: *clip,
            channel: *channel,
            expected: *expected,
            actual: *actual,
        },
        Source::MissingTangents {
            clip,
            channel,
            part,
        } => SceneValidationError::MissingTangents {
            clip: *clip,
            channel: *channel,
            part: animation_sample_part(*part),
        },
        Source::UnexpectedTangents {
            clip,
            channel,
            part,
        } => SceneValidationError::UnexpectedTangents {
            clip: *clip,
            channel: *channel,
            part: animation_sample_part(*part),
        },
        Source::TangentShape {
            clip,
            channel,
            part,
            expected,
            actual,
        } => SceneValidationError::TangentShape {
            clip: *clip,
            channel: *channel,
            part: animation_sample_part(*part),
            expected: *expected,
            actual: *actual,
        },
        Source::NonFiniteSample {
            clip,
            channel,
            part,
            sample,
            component,
        } => SceneValidationError::NonFiniteSample {
            clip: *clip,
            channel: *channel,
            part: animation_sample_part(*part),
            sample: *sample,
            component: *component,
        },
        Source::NonUnitQuaternion {
            clip,
            channel,
            sample,
        } => SceneValidationError::NonUnitQuaternion {
            clip: *clip,
            channel: *channel,
            sample: *sample,
        },
        Source::DuplicateTarget {
            clip,
            target,
            first_channel,
            duplicate_channel,
        } => SceneValidationError::DuplicateTarget {
            clip: *clip,
            target: *target,
            first_channel: *first_channel,
            duplicate_channel: *duplicate_channel,
        },
        Source::CountDomainTooLarge { collection } => {
            SceneValidationError::CollectionDomainTooLarge {
                domain: SceneValidationDomain::Animation,
                collection,
            }
        }
        Source::LimitExceeded {
            limit,
            actual,
            maximum,
        } => SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Animation,
            limit: match limit {
                AnimationAuthoringLimit::Entities => SceneValidationLimit::Entities,
                AnimationAuthoringLimit::Edges => SceneValidationLimit::Edges,
                AnimationAuthoringLimit::References => SceneValidationLimit::References,
                AnimationAuthoringLimit::SemanticCells => SceneValidationLimit::SemanticCells,
                AnimationAuthoringLimit::CanonicalBytes => SceneValidationLimit::CanonicalBytes,
                AnimationAuthoringLimit::Work => SceneValidationLimit::Work,
                AnimationAuthoringLimit::ScratchBytes => SceneValidationLimit::ScratchBytes,
            },
            actual: *actual,
            maximum: *maximum,
        },
        Source::AccountingOverflow { .. }
        | Source::ScratchCapacity { .. }
        | Source::ScratchAllocation { .. } => return None,
    })
}

pub(super) fn spatial_validation_error(
    cause: &SpatialAuthoringSemanticError,
) -> Option<SceneValidationError> {
    use SpatialAuthoringSemanticError as Source;
    Some(match cause {
        Source::InvalidSceneSpace(source) => SceneValidationError::InvalidSceneSpace(*source),
        Source::InvalidCameraProjection { camera } => {
            SceneValidationError::InvalidCameraProjection { camera: *camera }
        }
        Source::InvalidLightEmission { light, source } => {
            SceneValidationError::InvalidLightEmission {
                light: *light,
                source: light_emission_rule(*source),
            }
        }
        Source::CameraOutOfRange {
            node,
            camera,
            camera_count,
        } => SceneValidationError::CameraOutOfRange {
            node: *node,
            camera: *camera,
            camera_count: *camera_count,
        },
        Source::LightOutOfRange {
            node,
            light,
            light_count,
        } => SceneValidationError::LightOutOfRange {
            node: *node,
            light: *light,
            light_count: *light_count,
        },
        Source::OrphanCamera { camera } => SceneValidationError::OrphanCamera { camera: *camera },
        Source::OrphanLight { light } => SceneValidationError::OrphanLight { light: *light },
        Source::CountDomainTooLarge { collection } => {
            SceneValidationError::CollectionDomainTooLarge {
                domain: SceneValidationDomain::Spatial,
                collection,
            }
        }
        Source::LimitExceeded {
            limit,
            actual,
            maximum,
        } => SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Spatial,
            limit: match limit {
                SpatialAuthoringLimit::Entities => SceneValidationLimit::Entities,
                SpatialAuthoringLimit::Edges => SceneValidationLimit::Edges,
                SpatialAuthoringLimit::References => SceneValidationLimit::References,
                SpatialAuthoringLimit::SemanticCells => SceneValidationLimit::SemanticCells,
                SpatialAuthoringLimit::CanonicalBytes => SceneValidationLimit::CanonicalBytes,
                SpatialAuthoringLimit::Work => SceneValidationLimit::Work,
                SpatialAuthoringLimit::ScratchBytes => SceneValidationLimit::ScratchBytes,
            },
            actual: *actual,
            maximum: *maximum,
        },
        Source::AccountingOverflow { .. }
        | Source::ScratchCapacity { .. }
        | Source::ScratchAllocation { .. } => return None,
    })
}

pub(super) fn profile_validation_error(
    cause: &ProfileAuthoringSemanticError,
) -> Option<SceneValidationError> {
    use ProfileAuthoringSemanticError as Source;
    Some(match cause {
        Source::ResourceOutOfRange {
            source_index,
            resource,
            resource_count,
        } => SceneValidationError::ResourceOutOfRange {
            source_index: *source_index,
            resource: *resource,
            resource_count: *resource_count,
        },
        Source::SourceIdentityOutOfRange {
            node,
            source_index,
            source_count,
        } => SceneValidationError::SourceIdentityOutOfRange {
            node: *node,
            source_index: *source_index,
            source_count: *source_count,
        },
        Source::ParentNotBeforeChild { node, parent } => {
            SceneValidationError::ParentNotBeforeChild {
                node: *node,
                parent: *parent,
            }
        }
        Source::DuplicateParent { node, parent } => SceneValidationError::DuplicateParent {
            node: *node,
            parent: *parent,
        },
        Source::ParentOrder { node, parent } => SceneValidationError::ParentOrder {
            node: *node,
            parent: *parent,
        },
        Source::EvidenceLimitExceeded {
            node,
            actual,
            maximum,
        } => SceneValidationError::EvidenceLimitExceeded {
            node: *node,
            actual: *actual,
            maximum: *maximum,
        },
        Source::EvidenceOrder { node, evidence } => SceneValidationError::EvidenceOrder {
            node: *node,
            evidence: *evidence,
        },
        Source::OwnerOutOfRange { owner } => {
            SceneValidationError::OwnerOutOfRange { owner: *owner }
        }
        Source::DuplicateOwnerProvenance { owner } => {
            SceneValidationError::DuplicateOwnerProvenance {
                owner: profile_owner(*owner),
            }
        }
        Source::MissingOwnerProvenance { owner } => SceneValidationError::MissingOwnerProvenance {
            owner: profile_owner(*owner),
        },
        Source::EmptyOwnerProvenance { owner } => SceneValidationError::EmptyOwnerProvenance {
            owner: profile_owner(*owner),
        },
        Source::OwnerProvenanceOrder {
            owner,
            previous,
            current,
        } => SceneValidationError::OwnerProvenanceOrder {
            owner: profile_owner(*owner),
            previous: *previous,
            current: *current,
        },
        Source::ProvenanceOutOfRange {
            usage,
            provenance,
            provenance_count,
        } => SceneValidationError::ProvenanceOutOfRange {
            usage,
            provenance: *provenance,
            provenance_count: *provenance_count,
        },
        Source::MetadataNonFinite { entry } => {
            SceneValidationError::MetadataNonFinite { entry: *entry }
        }
        Source::DuplicateMetadataKey { first, duplicate } => {
            SceneValidationError::DuplicateMetadataKey {
                first: *first,
                duplicate: *duplicate,
            }
        }
        Source::MetadataKeyOrder {
            previous,
            entry,
            owner,
        } => SceneValidationError::MetadataKeyOrder {
            previous: *previous,
            entry: *entry,
            owner: profile_owner(*owner),
        },
        Source::CountDomainTooLarge { collection } => {
            SceneValidationError::CollectionDomainTooLarge {
                domain: SceneValidationDomain::Profile,
                collection,
            }
        }
        Source::LimitExceeded {
            limit,
            actual,
            maximum,
        } => SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Profile,
            limit: match limit {
                ProfileAuthoringLimit::Nodes => SceneValidationLimit::Nodes,
                ProfileAuthoringLimit::Edges => SceneValidationLimit::Edges,
                ProfileAuthoringLimit::Depth => SceneValidationLimit::Depth,
                ProfileAuthoringLimit::Entities => SceneValidationLimit::Entities,
                ProfileAuthoringLimit::References => SceneValidationLimit::References,
                ProfileAuthoringLimit::PayloadCells => SceneValidationLimit::SemanticCells,
                ProfileAuthoringLimit::MetadataEntries => SceneValidationLimit::MetadataEntries,
                ProfileAuthoringLimit::MetadataBytes => SceneValidationLimit::MetadataBytes,
                ProfileAuthoringLimit::MetadataKeyBytes => SceneValidationLimit::MetadataKeyBytes,
                ProfileAuthoringLimit::MetadataStringBytes => {
                    SceneValidationLimit::MetadataStringBytes
                }
                ProfileAuthoringLimit::CanonicalBytes => SceneValidationLimit::CanonicalBytes,
                ProfileAuthoringLimit::Work => SceneValidationLimit::Work,
                ProfileAuthoringLimit::ScratchBytes => SceneValidationLimit::ScratchBytes,
            },
            actual: *actual,
            maximum: *maximum,
        },
        Source::AccountingOverflow { .. }
        | Source::ScratchCapacity { .. }
        | Source::ScratchAllocation { .. } => return None,
    })
}
