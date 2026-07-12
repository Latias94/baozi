use crate::{NodeId, SceneCollection};
use baozi_operation::CapacityPlanError;
use thiserror::Error;

/// A failure while constructing an unvalidated scene draft.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SceneBuildError {
    #[error("parent node {parent:?} is out of range for {node_count} nodes")]
    ParentNodeOutOfRange { parent: NodeId, node_count: usize },

    #[error("scene {collection} collection exceeds the supported identifier range")]
    IdentifierSpaceExhausted { collection: SceneCollection },

    #[error("scene builder could not reserve {resource}")]
    AllocationFailed { resource: SceneBuildResource },

    #[error("scene builder capacity accounting overflowed for {resource}")]
    CapacityAccountingOverflow { resource: SceneBuildResource },

    #[error("scene builder capacity policy failed for {resource}: {cause}")]
    CapacityPlan {
        resource: SceneBuildResource,
        #[source]
        cause: CapacityPlanError,
    },

    #[error(
        "scene builder capacity plan for {actual} cannot be applied to expected resource {expected}"
    )]
    CapacityPlanResourceMismatch {
        expected: SceneBuildResource,
        actual: SceneBuildResource,
    },
}

/// A SceneBuilder-owned collection whose capacity growth can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneBuildResource {
    Nodes,
    NodeChildren,
    Meshes,
    Materials,
    Textures,
    Animations,
    Cameras,
    Lights,
    Skins,
}

impl std::fmt::Display for SceneBuildResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Nodes => "node capacity",
            Self::NodeChildren => "node-child capacity",
            Self::Meshes => "mesh capacity",
            Self::Materials => "material capacity",
            Self::Textures => "texture capacity",
            Self::Animations => "animation capacity",
            Self::Cameras => "camera capacity",
            Self::Lights => "light capacity",
            Self::Skins => "skin capacity",
        })
    }
}

/// A failure while proving that a scene draft satisfies every core invariant.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SceneValidationError {
    #[error("invalid scene: {issue}")]
    InvalidScene { issue: SceneValidationIssue },

    #[error("scene generation identifier space is exhausted")]
    GenerationSpaceExhausted,

    #[error("scene validation limit {limit:?} exceeded: {actual} > {maximum}")]
    LimitExceeded {
        limit: SceneValidationLimit,
        actual: u64,
        maximum: u64,
    },

    #[error("scene validation arithmetic overflow while accounting {resource}")]
    AccountingOverflow { resource: &'static str },

    #[error("scene validation allocation failed for {resource}")]
    AllocationFailed { resource: &'static str },

    #[error("scene validation capacity policy failed for {resource}: {cause}")]
    CapacityPlan {
        resource: &'static str,
        #[source]
        cause: CapacityPlanError,
    },

    #[error(
        "scene validation capacity reservation for {resource} was underestimated: {actual} > {admitted}"
    )]
    CapacityReservationUnderestimated {
        resource: &'static str,
        admitted: u64,
        actual: u64,
    },

    #[error("invalid metadata key at {owner:?} entry {entry}: {violation:?}")]
    InvalidMetadataKey {
        owner: SceneMetadataOwner,
        entry: usize,
        violation: MetadataKeyViolation,
    },

    #[error("invalid metadata value at {owner:?} entry {entry}: {violation:?}")]
    InvalidMetadataValue {
        owner: SceneMetadataOwner,
        entry: usize,
        violation: MetadataValueViolation,
    },

    #[error("invalid canonical string at {owner:?}: {violation:?}")]
    InvalidCanonicalString {
        owner: SceneStringOwner,
        violation: SceneStringViolation,
    },
}

/// A stable, allocation-free category for one failed scene invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SceneValidationCode {
    OutOfRange,
    ParentMismatch,
    NonFinite,
    Empty,
    Cycle,
    DuplicateReachability,
    Unreachable,
    LengthMismatch,
    MissingRequiredData,
    UnexpectedData,
    NotDivisible,
    MinimumNotMet,
    RequiresSkin,
    ConflictingBinding,
    OrderViolation,
    NonPositive,
    Negative,
    KindMismatch,
    Unsorted,
    InvalidSampleCount,
}

/// The semantic location of an allocation-free scene validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SceneValidationPath {
    Root,
    SceneSpaceUnitScale,
    Node {
        node: usize,
    },
    NodeReference {
        node: usize,
        reference: SceneReferenceKind,
    },
    NodeChild {
        node: usize,
        child: usize,
    },
    NodeHierarchy {
        node: usize,
    },
    Material {
        material: usize,
    },
    MaterialTextureSlot {
        material: usize,
        slot: usize,
    },
    MaterialProperty {
        material: usize,
        property: usize,
    },
    Texture {
        texture: usize,
    },
    Mesh {
        mesh: usize,
    },
    MeshReference {
        mesh: usize,
        reference: SceneReferenceKind,
    },
    MeshMorphTarget {
        mesh: usize,
        target: usize,
    },
    MeshElement {
        mesh: usize,
        element: usize,
    },
    MeshBounds {
        mesh: usize,
    },
    MeshJointChannels {
        mesh: usize,
    },
    MeshJoint {
        mesh: usize,
        vertex: usize,
        joint: usize,
    },
    MeshJointWeights {
        mesh: usize,
        vertex: usize,
    },
    MeshChannel {
        mesh: usize,
        channel: SceneMeshChannel,
    },
    MeshPolygonFace {
        mesh: usize,
        face: usize,
    },
    Skin {
        skin: usize,
    },
    SkinReference {
        skin: usize,
        reference: SceneReferenceKind,
    },
    Camera {
        camera: usize,
        parameter: SceneCameraParameter,
    },
    Light {
        light: usize,
        parameter: SceneLightParameter,
    },
    AnimationChannel {
        animation: usize,
        channel: usize,
    },
    AnimationValues {
        animation: usize,
        channel: usize,
        values: SceneAnimationValueKind,
    },
}

/// The relation targeted by an out-of-range reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SceneReferenceKind {
    Parent,
    Child,
    Mesh,
    Skin,
    Camera,
    Light,
    Material,
    Texture,
    SkeletonRoot,
    Joint,
    AnimationTarget,
}

/// A semantic mesh channel, including nested channel indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SceneMeshChannel {
    Positions,
    Normals,
    Tangents,
    TextureCoordinates(usize),
    Colors(usize),
    MorphPositions(usize),
    MorphNormals(usize),
    MorphTangents(usize),
    CustomAttribute(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SceneCameraParameter {
    Projection,
    FieldOfView,
    AspectRatio,
    NearDepth,
    FarDepth,
    Magnification,
    DepthRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SceneLightParameter {
    Value,
    Intensity,
    Range,
    InnerConeAngle,
    OuterConeAngle,
    ConeAngleOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SceneAnimationValueKind {
    Target,
    Times,
    Translations,
    Rotations,
    Scales,
    MorphWeights,
}

/// Structured numeric evidence attached to a scene validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SceneValidationData {
    #[default]
    None,
    Index {
        value: usize,
        upper_bound: usize,
    },
    Counts {
        actual: usize,
        expected: usize,
    },
    JointChannelCounts {
        indices: usize,
        weights: usize,
        positions: usize,
    },
    Divisibility {
        count: usize,
        divisor: usize,
    },
    Minimum {
        actual: usize,
        minimum: usize,
    },
}

/// Machine-readable evidence for one failed central-validation invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneValidationIssue {
    code: SceneValidationCode,
    path: SceneValidationPath,
    data: SceneValidationData,
}

impl SceneValidationIssue {
    pub const fn new(code: SceneValidationCode, path: SceneValidationPath) -> Self {
        Self {
            code,
            path,
            data: SceneValidationData::None,
        }
    }

    pub const fn with_data(mut self, data: SceneValidationData) -> Self {
        self.data = data;
        self
    }

    pub const fn code(self) -> SceneValidationCode {
        self.code
    }

    pub const fn path(self) -> SceneValidationPath {
        self.path
    }

    pub const fn data(self) -> SceneValidationData {
        self.data
    }
}

impl std::fmt::Display for SceneValidationIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?} at {:?}", self.code, self.path)?;
        if self.data != SceneValidationData::None {
            write!(formatter, " ({:?})", self.data)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneValidationLimit {
    Nodes,
    Edges,
    Depth,
    Entities,
    References,
    PayloadCells,
    Work,
    MetadataEntries,
    MetadataBytes,
    CanonicalBytes,
    ScratchBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneMetadataOwner {
    Scene,
    Node(usize),
    Mesh(usize),
    MorphTarget { mesh: usize, target: usize },
    VertexAttribute { mesh: usize, attribute: usize },
    Material(usize),
    Texture(usize),
    Animation(usize),
    AnimationChannel { animation: usize, channel: usize },
    Camera(usize),
    Light(usize),
    Skin(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataKeyViolation {
    Empty,
    TooLong,
    MissingNamespace,
    InvalidByte { offset: usize },
    InvalidSeparator { offset: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataValueViolation {
    NonFiniteF64,
    StringTooLong,
    ForbiddenControl { offset: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneStringOwner {
    NodeName(usize),
    MeshName(usize),
    MorphTargetName { mesh: usize, target: usize },
    VertexAttributeName { mesh: usize, attribute: usize },
    VertexAttributeSemantic { mesh: usize, attribute: usize },
    MaterialName(usize),
    MaterialTextureSourceKey { material: usize, slot: usize },
    MaterialPropertyString { material: usize, entry: usize },
    TextureName(usize),
    TextureExternalUri(usize),
    TextureMimeType(usize),
    AnimationName(usize),
    CameraName(usize),
    LightName(usize),
    SkinName(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneStringViolation {
    TooLong,
    ForbiddenControl { offset: usize },
}
