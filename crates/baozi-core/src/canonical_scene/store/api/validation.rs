//! Bounded producer-facing validation errors, independent of Scene activation and storage.

use thiserror::Error;

pub use super::super::production::input::animation::AnimationTargetAuthoring as SceneAnimationTarget;
pub use super::super::production::input::lineage::ProfileOwner as SceneAuthoredProfileOwner;
pub use crate::appearance::{
    AppearanceBehaviorKind, AppearanceBuildResource, AppearanceLimitDimension,
    ColorInterpretationKind, MaterialFactorGroup, ShadingModelKind, TextureRoleKind,
    TextureStrengthRequirement,
};
pub use crate::attribute::AttributeDomain;
pub use crate::deform::{MorphDeltaSemantic, SkinMatrixSpace};
pub use crate::entity::EntityNameError;
pub use crate::geometry::TopologyKind;
pub use crate::scene_space::SceneSpaceValidationError;

/// The semantic domain in which producer input was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneValidationDomain {
    Scene,
    Node,
    Geometry,
    Appearance,
    Deformation,
    Animation,
    Spatial,
    Profile,
}

/// A bounded validation limit, independent of physical store keys and terminal codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneValidationLimit {
    Nodes,
    Entities,
    Edges,
    Depth,
    SemanticCells,
    References,
    CanonicalBytes,
    Work,
    ScratchBytes,
    MetadataEntries,
    MetadataBytes,
    MetadataKeyBytes,
    MetadataStringBytes,
}

/// A producer collection whose element count cannot be represented by the Scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneInputCollection {
    Nodes,
    Roots,
    Cameras,
    Lights,
    Images,
    Samplers,
    Textures,
    MaterialBindings,
    Materials,
    LogicalMeshes,
    Primitives,
    Attributes,
    MeshInstances,
    Skins,
    MorphTargetDefinitions,
    LogicalMorphs,
    PrimitiveDeformations,
    MeshInstanceDeformations,
    AnimationClips,
    AnimationChannels,
    Resources,
    SourceIdentities,
    ProvenanceNodes,
    OwnerProvenance,
    MetadataEntries,
}

/// The existing appearance semantic rule and its bounded diagnostic values.
/// Binding authority indices are local to the owning material's binding range.
/// Allocation failures remain separate from producer input rejection.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneAppearanceRule {
    #[error("{shading:?} material requires factor group {required:?}")]
    MissingFactorGroup {
        shading: ShadingModelKind,
        required: MaterialFactorGroup,
    },
    #[error("texture role {role:?} does not admit color interpretation {interpretation:?}")]
    IncompatibleColorInterpretation {
        role: TextureRoleKind,
        interpretation: ColorInterpretationKind,
    },
    #[error("texture role {role:?} requires {minimum}..={maximum} mapped lanes, found {actual}")]
    IncompatibleChannelCount {
        role: TextureRoleKind,
        minimum: u8,
        maximum: u8,
        actual: u8,
    },
    #[error("texture role {role:?} violates its {requirement:?} strength requirement")]
    IncompatibleStrength {
        role: TextureRoleKind,
        requirement: TextureStrengthRequirement,
    },
    #[error("built-in texture role {role:?} requires Replace blend semantics")]
    BuiltInRoleRequiresReplace { role: TextureRoleKind },
    #[error("texture binding at index {duplicate_index} duplicates authority at {first_index}")]
    DuplicateBindingAuthority {
        first_index: usize,
        duplicate_index: usize,
    },
    #[error(
        "texture role {duplicate_role:?} at index {duplicate_index} conflicts with {first_role:?} authority at index {first_index}"
    )]
    ConflictingBindingRoles {
        first_index: usize,
        first_role: TextureRoleKind,
        duplicate_index: usize,
        duplicate_role: TextureRoleKind,
    },
    #[error(
        "appearance resource {resource:?} exceeds {dimension:?} limit: requested {requested}, maximum {maximum}"
    )]
    LimitExceeded {
        resource: AppearanceBuildResource,
        dimension: AppearanceLimitDimension,
        requested: usize,
        maximum: usize,
    },
}

/// The authored unit whose rate was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneAnimationRateKind {
    Frames,
    Ticks,
}

/// The stream containing an invalid animation sample or tangent shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneAnimationSamplePart {
    Values,
    InTangents,
    OutTangents,
}

/// Why retained light emission violates canonical semantics.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneLightEmissionRule {
    #[error("light tint components must be finite and non-negative")]
    InvalidColor,
    #[error("light tint alpha must be exactly one")]
    InvalidAlpha,
    #[error("light intensity must be finite and non-negative")]
    InvalidIntensity,
    #[error("zero light emission must use exact white tint and positive zero intensity")]
    NonCanonicalZero,
    #[error("positive light tint must be unit-luminance and use positive zero channels")]
    NonCanonicalTint,
    #[error("source light emission is not representable in canonical f32 storage")]
    Unrepresentable,
}

/// A profile owner in the validated canonical domain, not a physical storage key.
/// Node indices include canonical reordering and the synthetic root; rejected authored owners
/// instead retain their original indices in `SceneAuthoredProfileOwner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneProfileOwner {
    Scene,
    SyntheticRoot,
    Node(u32),
    Camera(u32),
    Light(u32),
    Image(u32),
    Sampler(u32),
    Texture(u32),
    TextureBinding(u32),
    Material(u32),
    LogicalMesh(u32),
    Primitive(u32),
    Attribute(u32),
    MeshInstance(u32),
    Skin(u32),
    MorphDefinition(u32),
    MorphDelta(u32),
    AnimationClip(u32),
    AnimationChannel(u32),
}

/// An allocation-free explanation of a rejected producer submission.
///
/// Top-level object indices follow producer input slice order. Animation channel indices are
/// local to their clip, and nested binding, joint, morph, and sample indices are owner-local.
/// Explicitly typed canonical profile owners retain their validated canonical indices.
/// Collection lengths and scalar positions retain their original integer domains without
/// narrowing casts.
/// This value contains neither activation authority nor physical storage identifiers. Control,
/// allocation, and internal materialization failures are reported separately.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneValidationError {
    #[error("{domain:?} {limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        domain: SceneValidationDomain,
        limit: SceneValidationLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("input collection {collection:?} count {actual} exceeds maximum {maximum}")]
    InputCountDomainExceeded {
        collection: SceneInputCollection,
        actual: u64,
        maximum: u64,
    },
    #[error("the authored root list is absent")]
    RootsAbsent,
    #[error(
        "{authored_nodes} authored nodes plus the synthetic root exceed \
         the u32 canonical node domain"
    )]
    NodeDomainTooLarge { authored_nodes: usize },
    #[error(
        "authored transform scalar count {scalar_count} exceeds the u32 canonical sequence domain"
    )]
    TransformDomainTooLarge { scalar_count: u64 },
    #[error(
        "root reference {root} at roots[{root_index}] is outside authored node count {node_count}"
    )]
    RootOutOfRange {
        root_index: usize,
        root: u32,
        node_count: usize,
    },
    #[error("root reference {root} repeats at roots[{duplicate_index}]")]
    DuplicateRoot { root: u32, duplicate_index: usize },
    #[error("declared root node {root} has parent {parent}")]
    RootHasParent { root: usize, parent: u32 },
    #[error("parentless node {node} is absent from the explicit root list")]
    MissingRoot { node: usize },
    #[error("parent reference {parent} on node {node} is outside authored node count {node_count}")]
    ParentOutOfRange {
        node: usize,
        parent: u32,
        node_count: usize,
    },
    #[error("node {node} references itself as parent")]
    SelfParent { node: usize },
    #[error("node {node} has an invalid display name: {cause}")]
    InvalidName { node: usize, cause: EntityNameError },
    #[error("node parent graph contains a cycle through node {node}")]
    Cycle { node: usize },
    #[error(
        "child ordinal {ordinal} on node {node} exceeds parent {parent} child count {child_count}"
    )]
    ChildOrdinalOutOfRange {
        node: u32,
        parent: u32,
        ordinal: u32,
        child_count: u32,
    },
    #[error(
        "child ordinal {ordinal} on parent {parent} is assigned to both node {first} and node {duplicate}"
    )]
    DuplicateChildOrdinal {
        parent: u32,
        ordinal: u32,
        first: u32,
        duplicate: u32,
    },
    #[error("logical mesh {logical_mesh} has an invalid entity name")]
    InvalidLogicalMeshName {
        logical_mesh: usize,
        source: EntityNameError,
    },
    #[error("logical mesh {logical_mesh} owns an empty primitive range")]
    EmptyLogicalMesh { logical_mesh: usize },
    #[error(
        "logical mesh {logical_mesh} primitive range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    LogicalMeshRangeOrder {
        logical_mesh: usize,
        expected: u32,
        actual: u32,
    },
    #[error(
        "logical mesh {logical_mesh} primitive range exceeds primitive count {primitive_count}"
    )]
    LogicalMeshRangeOutOfBounds {
        logical_mesh: usize,
        primitive_count: usize,
    },
    #[error("flat primitive table has {unowned} rows without exactly one logical mesh owner")]
    UnownedPrimitives { unowned: usize },
    #[error(
        "primitive {primitive} attribute range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    PrimitiveAttributeRangeOrder {
        primitive: usize,
        expected: u32,
        actual: u32,
    },
    #[error("primitive {primitive} attribute range exceeds attribute count {attribute_count}")]
    PrimitiveAttributeRangeOutOfBounds {
        primitive: usize,
        attribute_count: usize,
    },
    #[error("flat attribute table has {unowned} rows without exactly one primitive owner")]
    UnownedAttributes { unowned: usize },
    #[error("primitive {primitive} has no POSITION attribute for {vertex_count} vertices")]
    MissingPosition { primitive: usize, vertex_count: u32 },
    #[error("primitive {primitive} attribute {attribute} repeats an earlier canonical key")]
    DuplicateAttributeKey { primitive: usize, attribute: usize },
    #[error("primitive {primitive} attributes are not in canonical key order at {attribute}")]
    AttributeOrder { primitive: usize, attribute: usize },
    #[error("attribute {attribute} has zero lanes")]
    ZeroAttributeLanes { attribute: usize },
    #[error("attribute {attribute} scalar count is not divisible by lane count {lanes}")]
    AttributeScalarShape { attribute: usize, lanes: u8 },
    #[error("attribute {attribute} uses unsupported built-in domain or scalar format")]
    AttributeSchema { attribute: usize },
    #[error("attribute {attribute} has non-finite floating-point source value at scalar {scalar}")]
    NonFiniteAttributeValue { attribute: usize, scalar: usize },
    #[error("attribute {attribute} source projection is valid only for the Corner domain")]
    SourceProjectionDomain { attribute: usize },
    #[error(
        "attribute {attribute} source projection has {actual} indices; expected domain cardinality {expected}"
    )]
    SourceProjectionCardinality {
        attribute: usize,
        expected: u32,
        actual: usize,
    },
    #[error(
        "attribute {attribute} source projection index {index} at element {element} exceeds source count {source_count}"
    )]
    SourceProjectionOutOfRange {
        attribute: usize,
        element: usize,
        index: u32,
        source_count: u32,
    },
    #[error(
        "attribute {attribute} has canonical element count {actual}; {domain:?} requires {expected}"
    )]
    AttributeCardinality {
        attribute: usize,
        domain: AttributeDomain,
        expected: u32,
        actual: u32,
    },
    #[error("sparse attribute {attribute} default has {actual} scalars; expected {lanes}")]
    SparseDefaultShape {
        attribute: usize,
        lanes: u8,
        actual: usize,
    },
    #[error(
        "sparse attribute {attribute} has {value_elements} explicit values for {index_count} indices"
    )]
    SparseValueCount {
        attribute: usize,
        index_count: usize,
        value_elements: usize,
    },
    #[error(
        "sparse attribute {attribute} index {index} at element {element} is outside logical count {logical_count}"
    )]
    SparseIndexOutOfRange {
        attribute: usize,
        element: usize,
        index: u32,
        logical_count: u32,
    },
    #[error(
        "sparse attribute {attribute} indices are not strictly increasing at element {element}"
    )]
    SparseIndexOrder { attribute: usize, element: usize },
    #[error(
        "primitive {primitive} index {index} at corner {corner} exceeds vertex count {vertex_count}"
    )]
    VertexIndexOutOfRange {
        primitive: usize,
        corner: usize,
        index: u32,
        vertex_count: u32,
    },
    #[error(
        "primitive {primitive} {topology:?} corner count {corner_count} is not divisible by {corners_per_face}"
    )]
    ListCornerCount {
        primitive: usize,
        topology: TopologyKind,
        corner_count: u32,
        corners_per_face: u32,
    },
    #[error("primitive {primitive} polygon face {face} has {corner_count} corners")]
    PolygonFaceTooSmall {
        primitive: usize,
        face: usize,
        corner_count: u32,
    },
    #[error("primitive {primitive} polygon corner count overflowed at face {face}")]
    PolygonCornerCountOverflow { primitive: usize, face: usize },
    #[error(
        "primitive {primitive} polygon boundaries contain {boundary_corners} corners; index stream contains {corner_count}"
    )]
    PolygonCornerCountMismatch {
        primitive: usize,
        boundary_corners: u32,
        corner_count: u32,
    },
    #[error(
        "primitive {primitive} material {material} exceeds appearance material count {appearance_material_count}"
    )]
    MaterialOutOfRange {
        primitive: usize,
        material: u32,
        appearance_material_count: u32,
    },
    #[error("mesh instance {instance} owner node {owner} exceeds authored node count {node_count}")]
    InstanceOwnerOutOfRange {
        instance: usize,
        owner: u32,
        node_count: usize,
    },
    #[error(
        "mesh instance {instance} logical mesh {logical_mesh} exceeds logical mesh count {logical_mesh_count}"
    )]
    InstanceMeshOutOfRange {
        instance: usize,
        logical_mesh: u32,
        logical_mesh_count: usize,
    },
    #[error("{domain:?} collection {collection} exceeds the u32 canonical count domain")]
    CollectionDomainTooLarge {
        domain: SceneValidationDomain,
        collection: &'static str,
    },
    #[error("image {image} has an empty encoded payload")]
    EmptyEncodedPayload { image: usize },
    #[error(
        "image {image} encoded payload has {actual} bytes; the canonical per-image maximum is {maximum}"
    )]
    EncodedPayloadTooLarge {
        image: usize,
        actual: usize,
        maximum: usize,
    },
    #[error(
        "session resource snapshot table has {snapshot_count} rows; the profile declares {resource_count} resources"
    )]
    ResourceSnapshotCardinalityMismatch {
        snapshot_count: usize,
        resource_count: usize,
    },
    #[error(
        "image {image} references session resource {resource}; the profile declares {resource_count} resources"
    )]
    ImageResourceOutOfRange {
        image: usize,
        resource: u32,
        resource_count: usize,
    },
    #[error("image {image} session resource {resource} has no snapshotted bytes")]
    ImageResourceSnapshotAbsent { image: usize, resource: u32 },
    #[error(
        "material {material} binding range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    MaterialBindingRangeOrder {
        material: usize,
        expected: u32,
        actual: u32,
    },
    #[error("material {material} binding range exceeds binding count {binding_count}")]
    MaterialBindingRangeOutOfBounds {
        material: usize,
        binding_count: usize,
    },
    #[error("flat binding table has {unowned} rows without exactly one material owner")]
    UnownedBindings { unowned: usize },
    #[error("texture {texture} image {image} exceeds image count {image_count}")]
    TextureImageOutOfRange {
        texture: usize,
        image: u32,
        image_count: u32,
    },
    #[error("texture {texture} sampler {sampler} exceeds sampler count {sampler_count}")]
    TextureSamplerOutOfRange {
        texture: usize,
        sampler: u32,
        sampler_count: u32,
    },
    #[error("binding {binding} texture {texture} exceeds texture count {texture_count}")]
    BindingTextureOutOfRange {
        binding: usize,
        texture: u32,
        texture_count: u32,
    },
    #[error(
        "binding {binding} uses a namespaced texture role before extension authority is retained by the production appearance rail"
    )]
    NamespacedTextureRoleUnsupported { binding: usize },
    #[error(
        "binding {binding} uses namespaced {behavior:?} before extension authority is retained by the production appearance rail"
    )]
    NamespacedBindingBehaviorUnsupported {
        binding: usize,
        behavior: AppearanceBehaviorKind,
    },
    #[error("binding {binding} has invalid canonical usage semantics: {source}")]
    InvalidBindingSemantics {
        binding: usize,
        #[source]
        source: SceneAppearanceRule,
    },
    #[error("material {material} has invalid canonical material semantics: {source}")]
    InvalidMaterialSemantics {
        material: usize,
        #[source]
        source: SceneAppearanceRule,
    },
    #[error("material {material} has an invalid entity name: {source}")]
    InvalidMaterialName {
        material: usize,
        #[source]
        source: crate::entity::EntityNameError,
    },
    #[error(
        "material {material} uses namespaced {behavior:?} before extension authority is retained by the production appearance rail"
    )]
    NamespacedMaterialBehaviorUnsupported {
        material: usize,
        behavior: AppearanceBehaviorKind,
    },
    #[error("material {material} has conflicting canonical binding authority: {source}")]
    InvalidMaterialBindingSemantics {
        material: usize,
        #[source]
        source: SceneAppearanceRule,
    },
    #[error("deformation table {table} has {actual} rows; validated geometry requires {expected}")]
    GeometryTableCardinality {
        table: &'static str,
        expected: u32,
        actual: usize,
    },
    #[error("skin {skin} has an invalid name: {source}")]
    InvalidSkinName {
        skin: usize,
        #[source]
        source: EntityNameError,
    },
    #[error("logical mesh {logical_mesh} morph target {target} has an invalid name: {source}")]
    InvalidMorphTargetName {
        logical_mesh: usize,
        target: usize,
        #[source]
        source: EntityNameError,
    },
    #[error("skin {skin} uses a non-canonical matrix-space contract")]
    NonCanonicalSkinMatrixSpaces { skin: usize },
    #[error("skin {skin} has {joint_count} joints but {inverse_bind_count} inverse-bind matrices")]
    InverseBindCountMismatch {
        skin: usize,
        joint_count: usize,
        inverse_bind_count: usize,
    },
    #[error(
        "skin {skin} joint {joint} references node {node} beyond authored node count {node_count}"
    )]
    JointNodeOutOfRange {
        skin: usize,
        joint: usize,
        node: u32,
        node_count: usize,
    },
    #[error(
        "skin {skin} skeleton root references node {node} beyond authored node count {node_count}"
    )]
    SkeletonRootOutOfRange {
        skin: usize,
        node: u32,
        node_count: usize,
    },
    #[error("skin {skin} inverse-bind matrix {joint} contains a non-finite scalar")]
    NonFiniteInverseBind { skin: usize, joint: usize },
    #[error("skin {skin} inverse-bind matrix {joint} is not exactly affine")]
    NonAffineInverseBind { skin: usize, joint: usize },
    #[error("skin {skin} correction matrix contains a non-finite scalar")]
    NonFiniteSkinCorrection { skin: usize },
    #[error("skin {skin} correction matrix is not exactly affine")]
    NonAffineSkinCorrection { skin: usize },
    #[error(
        "skin {skin} correction maps {from:?} to {to:?}; canonical composition requires {expected_from:?} to {expected_to:?}"
    )]
    ContradictorySkinCorrectionSpaces {
        skin: usize,
        from: SkinMatrixSpace,
        to: SkinMatrixSpace,
        expected_from: SkinMatrixSpace,
        expected_to: SkinMatrixSpace,
    },
    #[error(
        "logical mesh {logical_mesh} definition range starts at {actual}; expected contiguous offset {expected}"
    )]
    MorphDefinitionRangeOrder {
        logical_mesh: usize,
        expected: u32,
        actual: u32,
    },
    #[error(
        "logical mesh {logical_mesh} definition range [{offset}, {end}) exceeds definition count {definition_count}"
    )]
    MorphDefinitionRangeOutOfRange {
        logical_mesh: usize,
        offset: u32,
        end: u64,
        definition_count: u32,
    },
    #[error("logical morph ranges own {owned} definitions; the batch contains {definition_count}")]
    UnownedMorphDefinitions { owned: u32, definition_count: u32 },
    #[error(
        "logical mesh {logical_mesh} owns {definition_count} morph definitions but has {default_count} default weights"
    )]
    LogicalMorphWidthMismatch {
        logical_mesh: usize,
        definition_count: u32,
        default_count: usize,
    },
    #[error(
        "logical mesh {logical_mesh} repeats morph target identity {identity} at target {target}"
    )]
    DuplicateMorphTargetIdentity {
        logical_mesh: usize,
        target: usize,
        identity: u32,
    },
    #[error(
        "logical mesh {logical_mesh} morph target {target} identity {identity} exceeds target width {target_count}"
    )]
    MorphTargetIdentityOutOfRange {
        logical_mesh: usize,
        target: usize,
        identity: u32,
        target_count: u32,
    },
    #[error("logical mesh {logical_mesh} default morph weight {target} is non-finite")]
    NonFiniteDefaultMorphWeight { logical_mesh: usize, target: usize },
    #[error(
        "primitive {primitive} exposes {actual} morph targets; logical mesh requires {expected}"
    )]
    PrimitiveMorphTargetCountMismatch {
        primitive: usize,
        expected: u32,
        actual: usize,
    },
    #[error(
        "primitive {primitive} influence offsets contain {actual} entries; vertex count requires {expected}"
    )]
    InfluenceOffsetCountMismatch {
        primitive: usize,
        expected: u64,
        actual: usize,
    },
    #[error("primitive {primitive} first influence offset is {actual}; expected zero")]
    InfluenceFirstOffset { primitive: usize, actual: u32 },
    #[error(
        "primitive {primitive} influence offset {offset} at position {position} exceeds influence count {influence_count}"
    )]
    InfluenceOffsetOutOfRange {
        primitive: usize,
        position: usize,
        offset: u32,
        influence_count: u32,
    },
    #[error(
        "primitive {primitive} influence offsets decrease at position {position}: {previous} to {current}"
    )]
    InfluenceOffsetOrder {
        primitive: usize,
        position: usize,
        previous: u32,
        current: u32,
    },
    #[error("primitive {primitive} final influence offset is {actual}; expected {expected}")]
    InfluenceFinalOffset {
        primitive: usize,
        expected: u32,
        actual: u32,
    },
    #[error("primitive {primitive} influence {influence} has a non-finite weight")]
    NonFiniteInfluenceWeight { primitive: usize, influence: usize },
    #[error("primitive {primitive} influence {influence} has a negative weight")]
    NegativeInfluenceWeight { primitive: usize, influence: usize },
    #[error("primitive {primitive} vertex {vertex} repeats source order {source_order}")]
    DuplicateInfluenceSourceOrder {
        primitive: usize,
        vertex: u32,
        source_order: u32,
    },
    #[error(
        "primitive {primitive} vertex {vertex} influence {influence} has source order {source_order}; row width is {row_width}"
    )]
    InfluenceSourceOrderOutOfRange {
        primitive: usize,
        vertex: u32,
        influence: usize,
        source_order: u32,
        row_width: u32,
    },
    #[error("primitive {primitive} morph target {target} repeats {semantic:?} at delta {delta}")]
    DuplicateMorphSemantic {
        primitive: usize,
        target: usize,
        delta: usize,
        semantic: MorphDeltaSemantic,
    },
    #[error(
        "primitive {primitive} morph target {target} delta {delta} is not in Position, Normal, Tangent order"
    )]
    MorphSemanticOrder {
        primitive: usize,
        target: usize,
        delta: usize,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} dense delta has {actual} scalars; expected {expected}"
    )]
    DenseMorphScalarCount {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        expected: u64,
        actual: usize,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} sparse logical count {actual} differs from vertex count {expected}"
    )]
    SparseMorphLogicalCount {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        expected: u32,
        actual: u32,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} sparse delta has {actual} scalars for {index_count} indices"
    )]
    SparseMorphScalarCount {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        index_count: usize,
        actual: usize,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} sparse index {index} at element {element} exceeds logical count {logical_count}"
    )]
    SparseMorphIndexOutOfRange {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        element: usize,
        index: u32,
        logical_count: u32,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} sparse indices are not strictly increasing at element {element}"
    )]
    SparseMorphIndexOrder {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        element: usize,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} delta scalar {scalar} is non-finite"
    )]
    NonFiniteMorphDelta {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        scalar: usize,
    },
    #[error("mesh instance {instance} references skin {skin} beyond skin count {skin_count}")]
    InstanceSkinOutOfRange {
        instance: usize,
        skin: u32,
        skin_count: u32,
    },
    #[error(
        "mesh instance {instance} morph override has width {actual}; logical mesh requires {expected}"
    )]
    InstanceMorphWidth {
        instance: usize,
        expected: u32,
        actual: usize,
    },
    #[error("mesh instance {instance} morph override weight {target} is non-finite")]
    NonFiniteInstanceMorphWeight { instance: usize, target: usize },
    #[error(
        "mesh instance {instance} selects skin {skin} with {joint_count} joints, but logical mesh {logical_mesh} primitive {primitive} influence {influence} uses palette joint {palette_joint}"
    )]
    PaletteJointOutOfRange {
        instance: usize,
        logical_mesh: u32,
        skin: u32,
        joint_count: u32,
        primitive: u32,
        influence: u32,
        palette_joint: u32,
    },
    #[error(
        "animation clip {clip} channel range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    ClipChannelRangeOrder {
        clip: usize,
        expected: u32,
        actual: u32,
    },
    #[error("animation clip {clip} channel range exceeds channel count {channel_count}")]
    ClipChannelRangeOutOfBounds { clip: usize, channel_count: usize },
    #[error(
        "animation clip {clip} source index {actual} is not greater than previous source index {previous}"
    )]
    ClipSourceIndexOrder {
        clip: usize,
        previous: u32,
        actual: u32,
    },
    #[error(
        "animation channel {channel} source index {actual} is not greater than previous source index {previous}"
    )]
    ChannelSourceIndexOrder {
        channel: usize,
        previous: u32,
        actual: u32,
    },
    #[error("flat animation channel table has {unowned} rows without exactly one clip owner")]
    UnownedChannels { unowned: usize },
    #[error("animation clip {clip} duration must be finite and non-negative")]
    InvalidDuration { clip: usize },
    #[error("animation clip {clip} has an invalid name: {source}")]
    InvalidClipName {
        clip: usize,
        #[source]
        source: crate::entity::EntityNameError,
    },
    #[error("animation clip {clip} {kind:?} rate must be finite and positive")]
    InvalidRate {
        clip: usize,
        kind: SceneAnimationRateKind,
    },
    #[error("animation clip {clip} channel {channel} has no authored key time")]
    EmptyChannel { clip: usize, channel: usize },
    #[error("animation clip {clip} channel {channel} key time {key} is not finite")]
    NonFiniteTime {
        clip: usize,
        channel: usize,
        key: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} key times {previous_key} and {key} are not strictly increasing"
    )]
    TimesNotStrictlyIncreasing {
        clip: usize,
        channel: usize,
        previous_key: usize,
        key: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} targets node {node}, but only {node_count} authored nodes exist"
    )]
    NodeTargetOutOfRange {
        clip: usize,
        channel: usize,
        node: u32,
        node_count: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} targets matrix-authored node {node} through a TRS channel"
    )]
    MatrixAuthoredNodeTarget {
        clip: usize,
        channel: usize,
        node: u32,
    },
    #[error(
        "animation clip {clip} channel {channel} targets mesh instance {mesh_instance}, but only {mesh_instance_count} instances exist"
    )]
    MeshInstanceTargetOutOfRange {
        clip: usize,
        channel: usize,
        mesh_instance: u32,
        mesh_instance_count: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} targets mesh instance {mesh_instance} without logical morph targets"
    )]
    MorphTargetWidthZero {
        clip: usize,
        channel: usize,
        mesh_instance: u32,
    },
    #[error(
        "animation clip {clip} channel {channel} has {actual} value scalars; expected {expected}"
    )]
    ValueShape {
        clip: usize,
        channel: usize,
        expected: u64,
        actual: usize,
    },
    #[error("animation clip {clip} channel {channel} requires {part:?} for cubic interpolation")]
    MissingTangents {
        clip: usize,
        channel: usize,
        part: SceneAnimationSamplePart,
    },
    #[error(
        "animation clip {clip} channel {channel} supplies {part:?} for non-cubic interpolation"
    )]
    UnexpectedTangents {
        clip: usize,
        channel: usize,
        part: SceneAnimationSamplePart,
    },
    #[error(
        "animation clip {clip} channel {channel} has {actual} {part:?} scalars; expected {expected}"
    )]
    TangentShape {
        clip: usize,
        channel: usize,
        part: SceneAnimationSamplePart,
        expected: u64,
        actual: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} {part:?} sample {sample}, component {component} is not finite"
    )]
    NonFiniteSample {
        clip: usize,
        channel: usize,
        part: SceneAnimationSamplePart,
        sample: usize,
        component: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} rotation sample {sample} is not a unit quaternion"
    )]
    NonUnitQuaternion {
        clip: usize,
        channel: usize,
        sample: usize,
    },
    #[error(
        "animation clip {clip} channel {duplicate_channel} duplicates channel {first_channel} target {target:?}"
    )]
    DuplicateTarget {
        clip: usize,
        target: SceneAnimationTarget,
        first_channel: usize,
        duplicate_channel: usize,
    },
    #[error("SceneSpace is not canonical: {0}")]
    InvalidSceneSpace(#[source] SceneSpaceValidationError),
    #[error("camera {camera} has invalid retained projection state")]
    InvalidCameraProjection { camera: usize },
    #[error("light {light} has invalid retained emission: {source}")]
    InvalidLightEmission {
        light: usize,
        #[source]
        source: SceneLightEmissionRule,
    },
    #[error("node {node} camera {camera} exceeds camera count {camera_count}")]
    CameraOutOfRange {
        node: usize,
        camera: u32,
        camera_count: u32,
    },
    #[error("node {node} light {light} exceeds light count {light_count}")]
    LightOutOfRange {
        node: usize,
        light: u32,
        light_count: u32,
    },
    #[error("camera {camera} has no node placement")]
    OrphanCamera { camera: usize },
    #[error("light {light} has no node placement")]
    OrphanLight { light: usize },
    #[error(
        "source identity {source_index} references resource {resource} outside resource count {resource_count}"
    )]
    ResourceOutOfRange {
        source_index: usize,
        resource: u32,
        resource_count: u32,
    },
    #[error(
        "source provenance node {node} references source identity {source_index} outside source count {source_count}"
    )]
    SourceIdentityOutOfRange {
        node: u32,
        source_index: u32,
        source_count: u32,
    },
    #[error("provenance node {node} references parent {parent}, which is not before the child")]
    ParentNotBeforeChild { node: u32, parent: u32 },
    #[error("provenance node {node} repeats parent {parent}")]
    DuplicateParent { node: u32, parent: u32 },
    #[error("provenance node {node} parent {parent} is not in strictly increasing order")]
    ParentOrder { node: u32, parent: u32 },
    #[error("provenance operation {node} has {actual} evidence rows; maximum is {maximum}")]
    EvidenceLimitExceeded {
        node: u32,
        actual: usize,
        maximum: usize,
    },
    #[error("provenance operation {node} evidence is not strictly ordered at index {evidence}")]
    EvidenceOrder { node: u32, evidence: usize },
    #[error("profile owner {owner:?} is outside its validated domain")]
    OwnerOutOfRange { owner: SceneAuthoredProfileOwner },
    #[error("profile owner {owner:?} has more than one provenance row")]
    DuplicateOwnerProvenance { owner: SceneProfileOwner },
    #[error("profile owner {owner:?} has no provenance row")]
    MissingOwnerProvenance { owner: SceneProfileOwner },
    #[error("profile owner {owner:?} has an empty provenance row")]
    EmptyOwnerProvenance { owner: SceneProfileOwner },
    #[error(
        "profile owner {owner:?} provenance attachment {current} must follow {previous} in strictly increasing order"
    )]
    OwnerProvenanceOrder {
        owner: SceneProfileOwner,
        previous: u32,
        current: u32,
    },
    #[error("{usage} references provenance {provenance} outside node count {provenance_count}")]
    ProvenanceOutOfRange {
        usage: &'static str,
        provenance: u32,
        provenance_count: u32,
    },
    #[error("metadata entry {entry} contains a non-finite value")]
    MetadataNonFinite { entry: usize },
    #[error("metadata entries {first} and {duplicate} repeat one owner-local key")]
    DuplicateMetadataKey { first: usize, duplicate: usize },
    #[error(
        "metadata entry {entry} key must follow entry {previous} for canonical owner {owner:?}"
    )]
    MetadataKeyOrder {
        previous: usize,
        entry: usize,
        owner: SceneProfileOwner,
    },
}

impl SceneValidationError {
    /// Returns the domain responsible for this semantic rule.
    pub const fn domain(&self) -> SceneValidationDomain {
        match self {
            Self::LimitExceeded { domain, .. } => *domain,
            Self::InputCountDomainExceeded { .. } => SceneValidationDomain::Scene,
            Self::CollectionDomainTooLarge { domain, .. } => *domain,
            Self::EmptyEncodedPayload { .. }
            | Self::EncodedPayloadTooLarge { .. }
            | Self::ResourceSnapshotCardinalityMismatch { .. }
            | Self::ImageResourceOutOfRange { .. }
            | Self::ImageResourceSnapshotAbsent { .. }
            | Self::MaterialBindingRangeOrder { .. }
            | Self::MaterialBindingRangeOutOfBounds { .. }
            | Self::UnownedBindings { .. }
            | Self::TextureImageOutOfRange { .. }
            | Self::TextureSamplerOutOfRange { .. }
            | Self::BindingTextureOutOfRange { .. }
            | Self::NamespacedTextureRoleUnsupported { .. }
            | Self::NamespacedBindingBehaviorUnsupported { .. }
            | Self::InvalidBindingSemantics { .. }
            | Self::InvalidMaterialSemantics { .. }
            | Self::InvalidMaterialName { .. }
            | Self::NamespacedMaterialBehaviorUnsupported { .. }
            | Self::InvalidMaterialBindingSemantics { .. } => SceneValidationDomain::Appearance,
            Self::GeometryTableCardinality { .. }
            | Self::InvalidSkinName { .. }
            | Self::InvalidMorphTargetName { .. }
            | Self::NonCanonicalSkinMatrixSpaces { .. }
            | Self::InverseBindCountMismatch { .. }
            | Self::JointNodeOutOfRange { .. }
            | Self::SkeletonRootOutOfRange { .. }
            | Self::NonFiniteInverseBind { .. }
            | Self::NonAffineInverseBind { .. }
            | Self::NonFiniteSkinCorrection { .. }
            | Self::NonAffineSkinCorrection { .. }
            | Self::ContradictorySkinCorrectionSpaces { .. }
            | Self::MorphDefinitionRangeOrder { .. }
            | Self::MorphDefinitionRangeOutOfRange { .. }
            | Self::UnownedMorphDefinitions { .. }
            | Self::LogicalMorphWidthMismatch { .. }
            | Self::DuplicateMorphTargetIdentity { .. }
            | Self::MorphTargetIdentityOutOfRange { .. }
            | Self::NonFiniteDefaultMorphWeight { .. }
            | Self::PrimitiveMorphTargetCountMismatch { .. }
            | Self::InfluenceOffsetCountMismatch { .. }
            | Self::InfluenceFirstOffset { .. }
            | Self::InfluenceOffsetOutOfRange { .. }
            | Self::InfluenceOffsetOrder { .. }
            | Self::InfluenceFinalOffset { .. }
            | Self::NonFiniteInfluenceWeight { .. }
            | Self::NegativeInfluenceWeight { .. }
            | Self::DuplicateInfluenceSourceOrder { .. }
            | Self::InfluenceSourceOrderOutOfRange { .. }
            | Self::DuplicateMorphSemantic { .. }
            | Self::MorphSemanticOrder { .. }
            | Self::DenseMorphScalarCount { .. }
            | Self::SparseMorphLogicalCount { .. }
            | Self::SparseMorphScalarCount { .. }
            | Self::SparseMorphIndexOutOfRange { .. }
            | Self::SparseMorphIndexOrder { .. }
            | Self::NonFiniteMorphDelta { .. }
            | Self::InstanceSkinOutOfRange { .. }
            | Self::InstanceMorphWidth { .. }
            | Self::NonFiniteInstanceMorphWeight { .. }
            | Self::PaletteJointOutOfRange { .. } => SceneValidationDomain::Deformation,
            Self::ClipChannelRangeOrder { .. }
            | Self::ClipChannelRangeOutOfBounds { .. }
            | Self::ClipSourceIndexOrder { .. }
            | Self::ChannelSourceIndexOrder { .. }
            | Self::UnownedChannels { .. }
            | Self::InvalidDuration { .. }
            | Self::InvalidClipName { .. }
            | Self::InvalidRate { .. }
            | Self::EmptyChannel { .. }
            | Self::NonFiniteTime { .. }
            | Self::TimesNotStrictlyIncreasing { .. }
            | Self::NodeTargetOutOfRange { .. }
            | Self::MatrixAuthoredNodeTarget { .. }
            | Self::MeshInstanceTargetOutOfRange { .. }
            | Self::MorphTargetWidthZero { .. }
            | Self::ValueShape { .. }
            | Self::MissingTangents { .. }
            | Self::UnexpectedTangents { .. }
            | Self::TangentShape { .. }
            | Self::NonFiniteSample { .. }
            | Self::NonUnitQuaternion { .. }
            | Self::DuplicateTarget { .. } => SceneValidationDomain::Animation,
            Self::InvalidSceneSpace(..)
            | Self::InvalidCameraProjection { .. }
            | Self::InvalidLightEmission { .. }
            | Self::CameraOutOfRange { .. }
            | Self::LightOutOfRange { .. }
            | Self::OrphanCamera { .. }
            | Self::OrphanLight { .. } => SceneValidationDomain::Spatial,
            Self::ResourceOutOfRange { .. }
            | Self::SourceIdentityOutOfRange { .. }
            | Self::ParentNotBeforeChild { .. }
            | Self::DuplicateParent { .. }
            | Self::ParentOrder { .. }
            | Self::EvidenceLimitExceeded { .. }
            | Self::EvidenceOrder { .. }
            | Self::OwnerOutOfRange { .. }
            | Self::DuplicateOwnerProvenance { .. }
            | Self::MissingOwnerProvenance { .. }
            | Self::EmptyOwnerProvenance { .. }
            | Self::OwnerProvenanceOrder { .. }
            | Self::ProvenanceOutOfRange { .. }
            | Self::MetadataNonFinite { .. }
            | Self::DuplicateMetadataKey { .. }
            | Self::MetadataKeyOrder { .. } => SceneValidationDomain::Profile,
            Self::RootsAbsent
            | Self::NodeDomainTooLarge { .. }
            | Self::TransformDomainTooLarge { .. }
            | Self::RootOutOfRange { .. }
            | Self::DuplicateRoot { .. }
            | Self::RootHasParent { .. }
            | Self::MissingRoot { .. }
            | Self::ParentOutOfRange { .. }
            | Self::SelfParent { .. }
            | Self::InvalidName { .. }
            | Self::Cycle { .. }
            | Self::ChildOrdinalOutOfRange { .. }
            | Self::DuplicateChildOrdinal { .. } => SceneValidationDomain::Node,
            Self::InvalidLogicalMeshName { .. }
            | Self::EmptyLogicalMesh { .. }
            | Self::LogicalMeshRangeOrder { .. }
            | Self::LogicalMeshRangeOutOfBounds { .. }
            | Self::UnownedPrimitives { .. }
            | Self::PrimitiveAttributeRangeOrder { .. }
            | Self::PrimitiveAttributeRangeOutOfBounds { .. }
            | Self::UnownedAttributes { .. }
            | Self::MissingPosition { .. }
            | Self::DuplicateAttributeKey { .. }
            | Self::AttributeOrder { .. }
            | Self::ZeroAttributeLanes { .. }
            | Self::AttributeScalarShape { .. }
            | Self::AttributeSchema { .. }
            | Self::NonFiniteAttributeValue { .. }
            | Self::SourceProjectionDomain { .. }
            | Self::SourceProjectionCardinality { .. }
            | Self::SourceProjectionOutOfRange { .. }
            | Self::AttributeCardinality { .. }
            | Self::SparseDefaultShape { .. }
            | Self::SparseValueCount { .. }
            | Self::SparseIndexOutOfRange { .. }
            | Self::SparseIndexOrder { .. }
            | Self::VertexIndexOutOfRange { .. }
            | Self::ListCornerCount { .. }
            | Self::PolygonFaceTooSmall { .. }
            | Self::PolygonCornerCountOverflow { .. }
            | Self::PolygonCornerCountMismatch { .. }
            | Self::MaterialOutOfRange { .. }
            | Self::InstanceOwnerOutOfRange { .. }
            | Self::InstanceMeshOutOfRange { .. } => SceneValidationDomain::Geometry,
        }
    }

    /// Returns a stable semantic rule identifier; it is not an operation terminal code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::LimitExceeded { .. } => "scene.limit-exceeded",
            Self::CollectionDomainTooLarge { .. } => "scene.collection-domain-too-large",
            Self::EmptyEncodedPayload { .. } => "appearance.empty-encoded-payload",
            Self::EncodedPayloadTooLarge { .. } => "appearance.encoded-payload-too-large",
            Self::ResourceSnapshotCardinalityMismatch { .. } => {
                "appearance.resource-snapshot-cardinality-mismatch"
            }
            Self::ImageResourceOutOfRange { .. } => "appearance.image-resource-out-of-range",
            Self::ImageResourceSnapshotAbsent { .. } => "appearance.image-resource-snapshot-absent",
            Self::MaterialBindingRangeOrder { .. } => "appearance.material-binding-range-order",
            Self::MaterialBindingRangeOutOfBounds { .. } => {
                "appearance.material-binding-range-out-of-bounds"
            }
            Self::UnownedBindings { .. } => "appearance.unowned-bindings",
            Self::TextureImageOutOfRange { .. } => "appearance.texture-image-out-of-range",
            Self::TextureSamplerOutOfRange { .. } => "appearance.texture-sampler-out-of-range",
            Self::BindingTextureOutOfRange { .. } => "appearance.binding-texture-out-of-range",
            Self::NamespacedTextureRoleUnsupported { .. } => {
                "appearance.namespaced-texture-role-unsupported"
            }
            Self::NamespacedBindingBehaviorUnsupported { .. } => {
                "appearance.namespaced-binding-behavior-unsupported"
            }
            Self::InvalidBindingSemantics { .. } => "appearance.invalid-binding-semantics",
            Self::InvalidMaterialSemantics { .. } => "appearance.invalid-material-semantics",
            Self::InvalidMaterialName { .. } => "appearance.invalid-material-name",
            Self::NamespacedMaterialBehaviorUnsupported { .. } => {
                "appearance.namespaced-material-behavior-unsupported"
            }
            Self::InvalidMaterialBindingSemantics { .. } => {
                "appearance.invalid-material-binding-semantics"
            }
            Self::GeometryTableCardinality { .. } => "deformation.geometry-table-cardinality",
            Self::InvalidSkinName { .. } => "deformation.invalid-skin-name",
            Self::InvalidMorphTargetName { .. } => "deformation.invalid-morph-target-name",
            Self::NonCanonicalSkinMatrixSpaces { .. } => {
                "deformation.non-canonical-skin-matrix-spaces"
            }
            Self::InverseBindCountMismatch { .. } => "deformation.inverse-bind-count-mismatch",
            Self::JointNodeOutOfRange { .. } => "deformation.joint-node-out-of-range",
            Self::SkeletonRootOutOfRange { .. } => "deformation.skeleton-root-out-of-range",
            Self::NonFiniteInverseBind { .. } => "deformation.non-finite-inverse-bind",
            Self::NonAffineInverseBind { .. } => "deformation.non-affine-inverse-bind",
            Self::NonFiniteSkinCorrection { .. } => "deformation.non-finite-skin-correction",
            Self::NonAffineSkinCorrection { .. } => "deformation.non-affine-skin-correction",
            Self::ContradictorySkinCorrectionSpaces { .. } => {
                "deformation.contradictory-skin-correction-spaces"
            }
            Self::MorphDefinitionRangeOrder { .. } => "deformation.morph-definition-range-order",
            Self::MorphDefinitionRangeOutOfRange { .. } => {
                "deformation.morph-definition-range-out-of-range"
            }
            Self::UnownedMorphDefinitions { .. } => "deformation.unowned-morph-definitions",
            Self::LogicalMorphWidthMismatch { .. } => "deformation.logical-morph-width-mismatch",
            Self::DuplicateMorphTargetIdentity { .. } => {
                "deformation.duplicate-morph-target-identity"
            }
            Self::MorphTargetIdentityOutOfRange { .. } => {
                "deformation.morph-target-identity-out-of-range"
            }
            Self::NonFiniteDefaultMorphWeight { .. } => {
                "deformation.non-finite-default-morph-weight"
            }
            Self::PrimitiveMorphTargetCountMismatch { .. } => {
                "deformation.primitive-morph-target-count-mismatch"
            }
            Self::InfluenceOffsetCountMismatch { .. } => {
                "deformation.influence-offset-count-mismatch"
            }
            Self::InfluenceFirstOffset { .. } => "deformation.influence-first-offset",
            Self::InfluenceOffsetOutOfRange { .. } => "deformation.influence-offset-out-of-range",
            Self::InfluenceOffsetOrder { .. } => "deformation.influence-offset-order",
            Self::InfluenceFinalOffset { .. } => "deformation.influence-final-offset",
            Self::NonFiniteInfluenceWeight { .. } => "deformation.non-finite-influence-weight",
            Self::NegativeInfluenceWeight { .. } => "deformation.negative-influence-weight",
            Self::DuplicateInfluenceSourceOrder { .. } => {
                "deformation.duplicate-influence-source-order"
            }
            Self::InfluenceSourceOrderOutOfRange { .. } => {
                "deformation.influence-source-order-out-of-range"
            }
            Self::DuplicateMorphSemantic { .. } => "deformation.duplicate-morph-semantic",
            Self::MorphSemanticOrder { .. } => "deformation.morph-semantic-order",
            Self::DenseMorphScalarCount { .. } => "deformation.dense-morph-scalar-count",
            Self::SparseMorphLogicalCount { .. } => "deformation.sparse-morph-logical-count",
            Self::SparseMorphScalarCount { .. } => "deformation.sparse-morph-scalar-count",
            Self::SparseMorphIndexOutOfRange { .. } => {
                "deformation.sparse-morph-index-out-of-range"
            }
            Self::SparseMorphIndexOrder { .. } => "deformation.sparse-morph-index-order",
            Self::NonFiniteMorphDelta { .. } => "deformation.non-finite-morph-delta",
            Self::InstanceSkinOutOfRange { .. } => "deformation.instance-skin-out-of-range",
            Self::InstanceMorphWidth { .. } => "deformation.instance-morph-width",
            Self::NonFiniteInstanceMorphWeight { .. } => {
                "deformation.non-finite-instance-morph-weight"
            }
            Self::PaletteJointOutOfRange { .. } => "deformation.palette-joint-out-of-range",
            Self::ClipChannelRangeOrder { .. } => "animation.clip-channel-range-order",
            Self::ClipChannelRangeOutOfBounds { .. } => {
                "animation.clip-channel-range-out-of-bounds"
            }
            Self::ClipSourceIndexOrder { .. } => "animation.clip-source-index-order",
            Self::ChannelSourceIndexOrder { .. } => "animation.channel-source-index-order",
            Self::UnownedChannels { .. } => "animation.unowned-channels",
            Self::InvalidDuration { .. } => "animation.invalid-duration",
            Self::InvalidClipName { .. } => "animation.invalid-clip-name",
            Self::InvalidRate { .. } => "animation.invalid-rate",
            Self::EmptyChannel { .. } => "animation.empty-channel",
            Self::NonFiniteTime { .. } => "animation.non-finite-time",
            Self::TimesNotStrictlyIncreasing { .. } => "animation.times-not-strictly-increasing",
            Self::NodeTargetOutOfRange { .. } => "animation.node-target-out-of-range",
            Self::MatrixAuthoredNodeTarget { .. } => "animation.matrix-authored-node-target",
            Self::MeshInstanceTargetOutOfRange { .. } => {
                "animation.mesh-instance-target-out-of-range"
            }
            Self::MorphTargetWidthZero { .. } => "animation.morph-target-width-zero",
            Self::ValueShape { .. } => "animation.value-shape",
            Self::MissingTangents { .. } => "animation.missing-tangents",
            Self::UnexpectedTangents { .. } => "animation.unexpected-tangents",
            Self::TangentShape { .. } => "animation.tangent-shape",
            Self::NonFiniteSample { .. } => "animation.non-finite-sample",
            Self::NonUnitQuaternion { .. } => "animation.non-unit-quaternion",
            Self::DuplicateTarget { .. } => "animation.duplicate-target",
            Self::InvalidSceneSpace(..) => "spatial.invalid-scene-space",
            Self::InvalidCameraProjection { .. } => "spatial.invalid-camera-projection",
            Self::InvalidLightEmission { .. } => "spatial.invalid-light-emission",
            Self::CameraOutOfRange { .. } => "spatial.camera-out-of-range",
            Self::LightOutOfRange { .. } => "spatial.light-out-of-range",
            Self::OrphanCamera { .. } => "spatial.orphan-camera",
            Self::OrphanLight { .. } => "spatial.orphan-light",
            Self::ResourceOutOfRange { .. } => "profile.resource-out-of-range",
            Self::SourceIdentityOutOfRange { .. } => "profile.source-identity-out-of-range",
            Self::ParentNotBeforeChild { .. } => "profile.parent-not-before-child",
            Self::DuplicateParent { .. } => "profile.duplicate-parent",
            Self::ParentOrder { .. } => "profile.parent-order",
            Self::EvidenceLimitExceeded { .. } => "profile.evidence-limit-exceeded",
            Self::EvidenceOrder { .. } => "profile.evidence-order",
            Self::OwnerOutOfRange { .. } => "profile.owner-out-of-range",
            Self::DuplicateOwnerProvenance { .. } => "profile.duplicate-owner-provenance",
            Self::MissingOwnerProvenance { .. } => "profile.missing-owner-provenance",
            Self::EmptyOwnerProvenance { .. } => "profile.empty-owner-provenance",
            Self::OwnerProvenanceOrder { .. } => "profile.owner-provenance-order",
            Self::ProvenanceOutOfRange { .. } => "profile.provenance-out-of-range",
            Self::MetadataNonFinite { .. } => "profile.metadata-non-finite",
            Self::DuplicateMetadataKey { .. } => "profile.duplicate-metadata-key",
            Self::MetadataKeyOrder { .. } => "profile.metadata-key-order",
            Self::InputCountDomainExceeded { .. } => "scene.input-count-domain-exceeded",
            Self::RootsAbsent => "node.roots-absent",
            Self::NodeDomainTooLarge { .. } => "node.node-domain-too-large",
            Self::TransformDomainTooLarge { .. } => "node.transform-domain-too-large",
            Self::RootOutOfRange { .. } => "node.root-out-of-range",
            Self::DuplicateRoot { .. } => "node.duplicate-root",
            Self::RootHasParent { .. } => "node.root-has-parent",
            Self::MissingRoot { .. } => "node.missing-root",
            Self::ParentOutOfRange { .. } => "node.parent-out-of-range",
            Self::SelfParent { .. } => "node.self-parent",
            Self::InvalidName { .. } => "node.invalid-name",
            Self::Cycle { .. } => "node.cycle",
            Self::ChildOrdinalOutOfRange { .. } => "node.child-ordinal-out-of-range",
            Self::DuplicateChildOrdinal { .. } => "node.duplicate-child-ordinal",
            Self::InvalidLogicalMeshName { .. } => "geometry.invalid-logical-mesh-name",
            Self::EmptyLogicalMesh { .. } => "geometry.empty-logical-mesh",
            Self::LogicalMeshRangeOrder { .. } => "geometry.logical-mesh-range-order",
            Self::LogicalMeshRangeOutOfBounds { .. } => "geometry.logical-mesh-range-out-of-bounds",
            Self::UnownedPrimitives { .. } => "geometry.unowned-primitives",
            Self::PrimitiveAttributeRangeOrder { .. } => "geometry.primitive-attribute-range-order",
            Self::PrimitiveAttributeRangeOutOfBounds { .. } => {
                "geometry.primitive-attribute-range-out-of-bounds"
            }
            Self::UnownedAttributes { .. } => "geometry.unowned-attributes",
            Self::MissingPosition { .. } => "geometry.missing-position",
            Self::DuplicateAttributeKey { .. } => "geometry.duplicate-attribute-key",
            Self::AttributeOrder { .. } => "geometry.attribute-order",
            Self::ZeroAttributeLanes { .. } => "geometry.zero-attribute-lanes",
            Self::AttributeScalarShape { .. } => "geometry.attribute-scalar-shape",
            Self::AttributeSchema { .. } => "geometry.attribute-schema",
            Self::NonFiniteAttributeValue { .. } => "geometry.non-finite-attribute-value",
            Self::SourceProjectionDomain { .. } => "geometry.source-projection-domain",
            Self::SourceProjectionCardinality { .. } => "geometry.source-projection-cardinality",
            Self::SourceProjectionOutOfRange { .. } => "geometry.source-projection-out-of-range",
            Self::AttributeCardinality { .. } => "geometry.attribute-cardinality",
            Self::SparseDefaultShape { .. } => "geometry.sparse-default-shape",
            Self::SparseValueCount { .. } => "geometry.sparse-value-count",
            Self::SparseIndexOutOfRange { .. } => "geometry.sparse-index-out-of-range",
            Self::SparseIndexOrder { .. } => "geometry.sparse-index-order",
            Self::VertexIndexOutOfRange { .. } => "geometry.vertex-index-out-of-range",
            Self::ListCornerCount { .. } => "geometry.list-corner-count",
            Self::PolygonFaceTooSmall { .. } => "geometry.polygon-face-too-small",
            Self::PolygonCornerCountOverflow { .. } => "geometry.polygon-corner-count-overflow",
            Self::PolygonCornerCountMismatch { .. } => "geometry.polygon-corner-count-mismatch",
            Self::MaterialOutOfRange { .. } => "geometry.material-out-of-range",
            Self::InstanceOwnerOutOfRange { .. } => "geometry.instance-owner-out-of-range",
            Self::InstanceMeshOutOfRange { .. } => "geometry.instance-mesh-out-of-range",
        }
    }
}

use super::super::production::SceneInputEnvelopeError;

pub(super) fn input_validation_error(
    cause: &SceneInputEnvelopeError,
) -> Option<SceneValidationError> {
    Some(match cause {
        SceneInputEnvelopeError::CountDomain {
            collection,
            count,
            maximum,
        } => {
            let collection = match *collection {
                "authored nodes" => SceneInputCollection::Nodes,
                "authored roots" => SceneInputCollection::Roots,
                "cameras" => SceneInputCollection::Cameras,
                "lights" => SceneInputCollection::Lights,
                "images" => SceneInputCollection::Images,
                "samplers" => SceneInputCollection::Samplers,
                "textures" => SceneInputCollection::Textures,
                "material bindings" => SceneInputCollection::MaterialBindings,
                "materials" => SceneInputCollection::Materials,
                "logical meshes" => SceneInputCollection::LogicalMeshes,
                "primitives" => SceneInputCollection::Primitives,
                "attributes" => SceneInputCollection::Attributes,
                "mesh instances" => SceneInputCollection::MeshInstances,
                "skins" => SceneInputCollection::Skins,
                "morph target definitions" => SceneInputCollection::MorphTargetDefinitions,
                "logical morphs" => SceneInputCollection::LogicalMorphs,
                "primitive deformation" => SceneInputCollection::PrimitiveDeformations,
                "mesh instance deformation" => SceneInputCollection::MeshInstanceDeformations,
                "animation clips" => SceneInputCollection::AnimationClips,
                "animation channels" => SceneInputCollection::AnimationChannels,
                "observed resources" => SceneInputCollection::Resources,
                "source identities" => SceneInputCollection::SourceIdentities,
                "provenance nodes" => SceneInputCollection::ProvenanceNodes,
                "owner provenance rows" => SceneInputCollection::OwnerProvenance,
                "metadata entries" => SceneInputCollection::MetadataEntries,
                _ => return None,
            };
            SceneValidationError::InputCountDomainExceeded {
                collection,
                actual: *count,
                maximum: *maximum,
            }
        }
        SceneInputEnvelopeError::LimitExceeded {
            limit,
            actual,
            maximum,
        } => SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Scene,
            limit: match *limit {
                "nodes" => SceneValidationLimit::Nodes,
                "entity lower bound" => SceneValidationLimit::Entities,
                "metadata entries" => SceneValidationLimit::MetadataEntries,
                _ => return None,
            },
            actual: *actual,
            maximum: *maximum,
        },
        SceneInputEnvelopeError::AccountingOverflow { .. } => return None,
    })
}
