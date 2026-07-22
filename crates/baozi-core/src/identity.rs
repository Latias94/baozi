use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

/// The runtime identity of one immutable scene state.
///
/// Generation identities are process-local capabilities. They are never
/// persistence keys, and their numeric representation is intentionally hidden.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneGenerationId(u64);

static LAST_SCENE_GENERATION: AtomicU64 = AtomicU64::new(0);

impl SceneGenerationId {
    /// Reserves a generation identity that can be consumed only once.
    pub(crate) fn reserve() -> Result<ReservedSceneGeneration, SceneGenerationError> {
        reserve_scene_generation(&LAST_SCENE_GENERATION)
    }

    /// Allocates a generation identity that has not previously been returned by
    /// this process.
    pub(crate) fn allocate() -> Result<Self, SceneGenerationError> {
        Ok(Self::reserve()?.into_id())
    }

    #[cfg(test)]
    pub(crate) fn reserve_from(
        last_generation: &AtomicU64,
    ) -> Result<ReservedSceneGeneration, SceneGenerationError> {
        reserve_scene_generation(last_generation)
    }
}

/// A linear reservation for one never-reused scene generation identity.
///
/// Dropping a reservation burns its identity. Only consuming it can expose the
/// identity to crate internals.
pub(crate) struct ReservedSceneGeneration(u64);

impl ReservedSceneGeneration {
    /// Consumes the reservation without mutating the generation sequence.
    pub(crate) const fn into_id(self) -> SceneGenerationId {
        SceneGenerationId(self.0)
    }
}

fn reserve_scene_generation(
    last_generation: &AtomicU64,
) -> Result<ReservedSceneGeneration, SceneGenerationError> {
    last_generation
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |observed| {
            observed.checked_add(1)
        })
        .map(|previous| ReservedSceneGeneration(previous + 1))
        .map_err(|_| SceneGenerationError::Exhausted)
}

impl fmt::Debug for SceneGenerationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SceneGenerationId")
    }
}

/// A failure to allocate a new scene generation identity.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SceneGenerationError {
    /// The process has allocated every representable generation identity.
    #[error("scene generation identifier space is exhausted")]
    Exhausted,
}

/// The kind of entity addressed by a generation-qualified scene handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SceneEntityKind {
    Node,
    MeshInstance,
    LogicalMesh,
    Primitive,
    MorphTarget,
    Material,
    Image,
    Sampler,
    Texture,
    Skin,
    AnimationClip,
    Camera,
    Light,
}

impl fmt::Display for SceneEntityKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Node => "node",
            Self::MeshInstance => "mesh instance",
            Self::LogicalMesh => "logical mesh",
            Self::Primitive => "primitive",
            Self::MorphTarget => "morph target",
            Self::Material => "material",
            Self::Image => "image",
            Self::Sampler => "sampler",
            Self::Texture => "texture",
            Self::Skin => "skin",
            Self::AnimationClip => "animation clip",
            Self::Camera => "camera",
            Self::Light => "light",
        })
    }
}

/// A rejected lookup through a generation-qualified scene handle.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SceneLookupError {
    #[error("{kind} handle belongs to a different scene generation")]
    GenerationMismatch {
        kind: SceneEntityKind,
        expected: SceneGenerationId,
        actual: SceneGenerationId,
    },

    #[error("{kind} handle belongs to a different logical mesh")]
    OwnerMismatch {
        kind: SceneEntityKind,
        expected: LogicalMeshId,
        actual: LogicalMeshId,
    },

    #[error("{kind} handle is out of range for a collection of length {length}")]
    OutOfRange {
        kind: SceneEntityKind,
        length: usize,
    },
}

/// A failed conversion between a collection position and its canonical index.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SceneStorageIndexError {
    #[error("{kind} storage index {index} exceeds the u32 identifier range")]
    U32Overflow { kind: SceneEntityKind, index: usize },

    #[error("{kind} storage index {index} exceeds the target usize range")]
    UsizeOverflow { kind: SceneEntityKind, index: u32 },
}

pub(crate) trait SceneStorageIndex: Copy {
    const KIND: SceneEntityKind;

    fn as_u32(self) -> u32;

    fn try_to_usize(self) -> Result<usize, SceneStorageIndexError> {
        let index = self.as_u32();
        usize::try_from(index).map_err(|_| SceneStorageIndexError::UsizeOverflow {
            kind: Self::KIND,
            index,
        })
    }

    fn get<T>(self, entities: &[T]) -> Result<Option<&T>, SceneStorageIndexError> {
        let index = self.try_to_usize()?;
        Ok(entities.get(index))
    }
}

macro_rules! define_scene_wide_identity {
    ($(#[$metadata:meta])* $index_name:ident, $handle_name:ident, $kind:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(crate) struct $index_name(u32);

        impl $index_name {
            pub(crate) const fn from_u32(index: u32) -> Self {
                Self(index)
            }

            pub(crate) const fn as_u32(self) -> u32 {
                self.0
            }

            pub(crate) const fn handle(
                self,
                generation: SceneGenerationId,
            ) -> $handle_name {
                $handle_name {
                    generation,
                    index: self,
                }
            }

            pub(crate) fn from_handle(
                expected_generation: SceneGenerationId,
                handle: $handle_name,
            ) -> Result<Self, SceneLookupError> {
                let actual_generation = handle.generation();
                if actual_generation != expected_generation {
                    return Err(SceneLookupError::GenerationMismatch {
                        kind: SceneEntityKind::$kind,
                        expected: expected_generation,
                        actual: actual_generation,
                    });
                }

                Ok(handle.index)
            }
        }

        impl TryFrom<usize> for $index_name {
            type Error = SceneStorageIndexError;

            fn try_from(index: usize) -> Result<Self, Self::Error> {
                u32::try_from(index).map(Self).map_err(|_| {
                    SceneStorageIndexError::U32Overflow {
                        kind: SceneEntityKind::$kind,
                        index,
                    }
                })
            }
        }

        impl SceneStorageIndex for $index_name {
            const KIND: SceneEntityKind = SceneEntityKind::$kind;

            fn as_u32(self) -> u32 {
                self.0
            }
        }

        $(#[$metadata])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $handle_name {
            generation: SceneGenerationId,
            index: $index_name,
        }

        impl fmt::Debug for $handle_name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(stringify!($handle_name))
            }
        }

        impl $handle_name {
            pub(crate) const fn new(generation: SceneGenerationId, index: u32) -> Self {
                $index_name::from_u32(index).handle(generation)
            }

            /// Returns the immutable scene generation addressed by this handle.
            pub const fn generation(self) -> SceneGenerationId {
                self.generation
            }

            pub(crate) const fn index(self) -> u32 {
                self.index.as_u32()
            }

            pub(crate) const fn rebind(self, generation: SceneGenerationId) -> Self {
                self.index.handle(generation)
            }
        }

        impl SceneWideHandle for $handle_name {
            type StorageIndex = $index_name;

            fn storage_index(
                self,
                expected_generation: SceneGenerationId,
            ) -> Result<Self::StorageIndex, SceneLookupError> {
                $index_name::from_handle(expected_generation, self)
            }
        }
    };
}

define_scene_wide_identity!(
    /// A generation-qualified handle to a node.
    NodeIndex,
    NodeId,
    Node
);
define_scene_wide_identity!(
    /// A generation-qualified handle to a mesh instance.
    MeshInstanceIndex,
    MeshInstanceId,
    MeshInstance
);
define_scene_wide_identity!(
    /// A generation-qualified handle to a logical mesh.
    LogicalMeshIndex,
    LogicalMeshId,
    LogicalMesh
);
define_scene_wide_identity!(
    /// A generation-qualified handle to a material.
    MaterialIndex,
    MaterialId,
    Material
);
define_scene_wide_identity!(
    /// A generation-qualified handle to an image payload.
    ImageIndex,
    ImageId,
    Image
);
define_scene_wide_identity!(
    /// A generation-qualified handle to a texture sampler.
    SamplerIndex,
    SamplerId,
    Sampler
);
define_scene_wide_identity!(
    /// A generation-qualified handle to a texture binding.
    TextureIndex,
    TextureId,
    Texture
);
define_scene_wide_identity!(
    /// A generation-qualified handle to a skin.
    SkinIndex,
    SkinId,
    Skin
);
define_scene_wide_identity!(
    /// A generation-qualified handle to an animation clip.
    AnimationClipIndex,
    AnimationClipId,
    AnimationClip
);
define_scene_wide_identity!(
    /// A generation-qualified handle to a camera.
    CameraIndex,
    CameraId,
    Camera
);
define_scene_wide_identity!(
    /// A generation-qualified handle to a light.
    LightIndex,
    LightId,
    Light
);

macro_rules! define_owner_qualified_identity {
    ($(#[$metadata:meta])* $index_name:ident, $handle_name:ident, $kind:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(crate) struct $index_name(u32);

        impl $index_name {
            pub(crate) const fn from_u32(index: u32) -> Self {
                Self(index)
            }

            pub(crate) const fn as_u32(self) -> u32 {
                self.0
            }

            pub(crate) const fn handle(self, logical_mesh: LogicalMeshId) -> $handle_name {
                $handle_name {
                    logical_mesh,
                    local: self,
                }
            }

            pub(crate) fn from_handle(
                expected_generation: SceneGenerationId,
                expected_owner: LogicalMeshId,
                handle: $handle_name,
            ) -> Result<Self, SceneLookupError> {
                let actual_generation = handle.generation();
                if actual_generation != expected_generation {
                    return Err(SceneLookupError::GenerationMismatch {
                        kind: SceneEntityKind::$kind,
                        expected: expected_generation,
                        actual: actual_generation,
                    });
                }

                let actual_owner = handle.logical_mesh();
                if actual_owner != expected_owner {
                    return Err(SceneLookupError::OwnerMismatch {
                        kind: SceneEntityKind::$kind,
                        expected: expected_owner,
                        actual: actual_owner,
                    });
                }

                Ok(handle.local)
            }
        }

        impl TryFrom<usize> for $index_name {
            type Error = SceneStorageIndexError;

            fn try_from(index: usize) -> Result<Self, Self::Error> {
                u32::try_from(index).map(Self).map_err(|_| {
                    SceneStorageIndexError::U32Overflow {
                        kind: SceneEntityKind::$kind,
                        index,
                    }
                })
            }
        }

        impl SceneStorageIndex for $index_name {
            const KIND: SceneEntityKind = SceneEntityKind::$kind;

            fn as_u32(self) -> u32 {
                self.0
            }
        }

        $(#[$metadata])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $handle_name {
            logical_mesh: LogicalMeshId,
            local: $index_name,
        }

        impl fmt::Debug for $handle_name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(stringify!($handle_name))
            }
        }

        impl $handle_name {
            pub(crate) const fn new(logical_mesh: LogicalMeshId, local: u32) -> Self {
                $index_name::from_u32(local).handle(logical_mesh)
            }

            /// Returns the immutable scene generation addressed by this handle.
            pub const fn generation(self) -> SceneGenerationId {
                self.logical_mesh.generation()
            }

            /// Returns the logical mesh that owns this local entity.
            pub const fn logical_mesh(self) -> LogicalMeshId {
                self.logical_mesh
            }

            pub(crate) const fn local(self) -> u32 {
                self.local.as_u32()
            }

            pub(crate) const fn rebind(self, generation: SceneGenerationId) -> Self {
                self.local.handle(self.logical_mesh.rebind(generation))
            }
        }

        impl OwnerQualifiedHandle for $handle_name {
            type StorageIndex = $index_name;

            fn storage_index(
                self,
                expected_generation: SceneGenerationId,
                expected_owner: LogicalMeshId,
            ) -> Result<Self::StorageIndex, SceneLookupError> {
                $index_name::from_handle(expected_generation, expected_owner, self)
            }
        }
    };
}

define_owner_qualified_identity!(
    /// A generation- and logical-mesh-qualified handle to a primitive.
    PrimitiveLocalIndex,
    PrimitiveId,
    Primitive
);
define_owner_qualified_identity!(
    /// A generation- and logical-mesh-qualified handle to a morph target.
    MorphTargetLocalIndex,
    MorphTargetId,
    MorphTarget
);

pub(crate) trait SceneWideHandle: Copy {
    type StorageIndex: SceneStorageIndex;

    fn storage_index(
        self,
        expected_generation: SceneGenerationId,
    ) -> Result<Self::StorageIndex, SceneLookupError>;
}

pub(crate) trait OwnerQualifiedHandle: Copy {
    type StorageIndex: SceneStorageIndex;

    fn storage_index(
        self,
        expected_generation: SceneGenerationId,
        expected_owner: LogicalMeshId,
    ) -> Result<Self::StorageIndex, SceneLookupError>;
}

pub(crate) fn lookup_storage_index<T, I: SceneStorageIndex>(
    index: I,
    entities: &[T],
) -> Result<&T, SceneLookupError> {
    let entity = index
        .get(entities)
        .map_err(|_| out_of_range::<I>(entities.len()))?;

    entity.ok_or_else(|| out_of_range::<I>(entities.len()))
}

pub(crate) fn validate_storage_index<I: SceneStorageIndex>(
    index: I,
    length: usize,
) -> Result<I, SceneLookupError> {
    let position = index
        .try_to_usize()
        .map_err(|_| out_of_range::<I>(length))?;
    if position >= length {
        return Err(out_of_range::<I>(length));
    }
    Ok(index)
}

const fn out_of_range<I: SceneStorageIndex>(length: usize) -> SceneLookupError {
    SceneLookupError::OutOfRange {
        kind: I::KIND,
        length,
    }
}

/// Resolves a scene-wide handle to its compact storage index after validating
/// generation and collection range, in that order.
pub(crate) fn resolve_scene_wide_index<H: SceneWideHandle>(
    expected_generation: SceneGenerationId,
    handle: H,
    collection_length: usize,
) -> Result<H::StorageIndex, SceneLookupError> {
    let index = handle.storage_index(expected_generation)?;
    validate_storage_index(index, collection_length)
}

/// Looks up a scene-wide handle after validating its generation.
pub(crate) fn lookup_scene_wide<T, H: SceneWideHandle>(
    expected_generation: SceneGenerationId,
    handle: H,
    entities: &[T],
) -> Result<&T, SceneLookupError> {
    let index = handle.storage_index(expected_generation)?;
    lookup_storage_index(index, entities)
}

/// Looks up a mesh-owned handle after validating its generation and owner.
pub(crate) fn lookup_owned<T, H: OwnerQualifiedHandle>(
    expected_generation: SceneGenerationId,
    expected_owner: LogicalMeshId,
    handle: H,
    entities: &[T],
) -> Result<&T, SceneLookupError> {
    let index = handle.storage_index(expected_generation, expected_owner)?;
    lookup_storage_index(index, entities)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation() -> SceneGenerationId {
        SceneGenerationId::allocate().expect("generation space should remain available")
    }

    #[test]
    fn allocated_generations_are_unique() {
        let first = generation();
        let second = generation();

        assert_ne!(first, second);
    }

    #[test]
    fn reserved_generation_is_consumed_into_id() {
        let last_generation = AtomicU64::new(40);

        let reservation = SceneGenerationId::reserve_from(&last_generation)
            .expect("generation space should remain available");

        assert_eq!(last_generation.load(Ordering::Relaxed), 41);
        assert_eq!(reservation.into_id(), SceneGenerationId(41));
        assert_eq!(last_generation.load(Ordering::Relaxed), 41);
    }

    #[test]
    fn dropped_reservation_burns_one_invisible_gap() {
        let last_generation = AtomicU64::new(0);
        {
            let _discarded = SceneGenerationId::reserve_from(&last_generation)
                .expect("generation space should remain available");
            assert_eq!(last_generation.load(Ordering::Relaxed), 1);
        }

        let published = SceneGenerationId::reserve_from(&last_generation)
            .expect("generation space should remain available")
            .into_id();

        assert_eq!(published, SceneGenerationId(2));
        assert_eq!(last_generation.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn generation_exhaustion_preserves_nonce() {
        let last_generation = AtomicU64::new(u64::MAX);

        let result = SceneGenerationId::reserve_from(&last_generation);

        assert!(matches!(result, Err(SceneGenerationError::Exhausted)));
        assert_eq!(last_generation.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn public_identity_debug_and_lookup_errors_are_opaque() {
        const GENERATION_SENTINEL: u64 = 4_294_967_291;
        const OWNER_SENTINEL: u32 = 4_294_967_292;
        const SCENE_WIDE_SENTINEL: u32 = 4_294_967_293;
        const LOCAL_SENTINEL: u32 = 4_294_967_294;

        let actual_generation = SceneGenerationId(GENERATION_SENTINEL);
        let expected_generation = SceneGenerationId(GENERATION_SENTINEL - 1);
        let actual_owner = LogicalMeshId::new(actual_generation, OWNER_SENTINEL);
        let expected_owner = LogicalMeshId::new(expected_generation, OWNER_SENTINEL - 1);
        let node = NodeId::new(actual_generation, SCENE_WIDE_SENTINEL);
        let primitive = PrimitiveId::new(actual_owner, LOCAL_SENTINEL);
        let generation_mismatch = SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Node,
            expected: expected_generation,
            actual: actual_generation,
        };
        let owner_mismatch = SceneLookupError::OwnerMismatch {
            kind: SceneEntityKind::Primitive,
            expected: expected_owner,
            actual: actual_owner,
        };

        assert_eq!(format!("{actual_generation:?}"), "SceneGenerationId");
        assert_eq!(format!("{node:?}"), "NodeId");
        assert_eq!(format!("{actual_owner:?}"), "LogicalMeshId");
        assert_eq!(format!("{primitive:?}"), "PrimitiveId");

        for rendered in [
            generation_mismatch.to_string(),
            format!("{generation_mismatch:?}"),
            owner_mismatch.to_string(),
            format!("{owner_mismatch:?}"),
        ] {
            for sentinel in [
                GENERATION_SENTINEL.to_string(),
                OWNER_SENTINEL.to_string(),
                SCENE_WIDE_SENTINEL.to_string(),
                LOCAL_SENTINEL.to_string(),
            ] {
                assert!(!rendered.contains(&sentinel), "leaked identity: {rendered}");
            }
        }

        assert!(generation_mismatch.to_string().contains("node"));
        assert!(format!("{generation_mismatch:?}").contains("GenerationMismatch"));
        assert!(owner_mismatch.to_string().contains("primitive"));
        assert!(format!("{owner_mismatch:?}").contains("OwnerMismatch"));
    }

    #[test]
    fn storage_indices_do_not_carry_scene_generation_state() {
        assert_eq!(std::mem::size_of::<NodeIndex>(), std::mem::size_of::<u32>());
        assert_eq!(
            std::mem::size_of::<PrimitiveLocalIndex>(),
            std::mem::size_of::<u32>()
        );
    }

    #[test]
    fn one_storage_index_can_qualify_handles_for_distinct_generations() {
        let first_generation = generation();
        let second_generation = generation();
        let index = NodeIndex::try_from(7).expect("index should fit the canonical range");
        let first = index.handle(first_generation);
        let second = index.handle(second_generation);

        assert_ne!(first, second);
        assert_eq!(NodeIndex::from_handle(first_generation, first), Ok(index));
        assert_eq!(NodeIndex::from_handle(second_generation, second), Ok(index));
    }

    #[test]
    fn storage_index_performs_checked_slice_lookup() {
        let index = MaterialIndex::try_from(1).expect("index should fit the canonical range");
        let materials = ["first", "second"];

        assert_eq!(index.get(&materials), Ok(Some(&"second")));
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn usize_to_u32_overflow_returns_typed_errors() {
        let maximum = usize::try_from(u32::MAX).expect("u32 must fit a 64-bit usize");
        let overflow = maximum
            .checked_add(1)
            .expect("u32 maximum plus one must fit a 64-bit usize");

        assert_eq!(
            NodeIndex::try_from(overflow),
            Err(SceneStorageIndexError::U32Overflow {
                kind: SceneEntityKind::Node,
                index: overflow,
            })
        );
        assert_eq!(
            MorphTargetLocalIndex::try_from(overflow),
            Err(SceneStorageIndexError::U32Overflow {
                kind: SceneEntityKind::MorphTarget,
                index: overflow,
            })
        );
    }

    #[test]
    fn scene_wide_generation_mismatch_precedes_range_validation() {
        let stale_generation = generation();
        let current_generation = generation();
        let stale = NodeId::new(stale_generation, u32::MAX);

        let result = lookup_scene_wide(current_generation, stale, &["current"]);

        assert_eq!(
            result,
            Err(SceneLookupError::GenerationMismatch {
                kind: SceneEntityKind::Node,
                expected: current_generation,
                actual: stale_generation,
            })
        );
    }

    #[test]
    fn owned_generation_mismatch_precedes_owner_and_range_validation() {
        let stale_generation = generation();
        let current_generation = generation();
        let current_owner = LogicalMeshId::new(current_generation, 0);
        let stale_owner = LogicalMeshId::new(stale_generation, 1);
        let stale = PrimitiveId::new(stale_owner, u32::MAX);

        let result = lookup_owned(current_generation, current_owner, stale, &["current"]);

        assert_eq!(
            result,
            Err(SceneLookupError::GenerationMismatch {
                kind: SceneEntityKind::Primitive,
                expected: current_generation,
                actual: stale_generation,
            })
        );
    }

    #[test]
    fn owner_mismatch_precedes_local_range_validation() {
        let current_generation = generation();
        let expected_owner = LogicalMeshId::new(current_generation, 0);
        let foreign_owner = LogicalMeshId::new(current_generation, 1);
        let foreign = MorphTargetId::new(foreign_owner, u32::MAX);

        let result = lookup_owned(current_generation, expected_owner, foreign, &["current"]);

        assert_eq!(
            result,
            Err(SceneLookupError::OwnerMismatch {
                kind: SceneEntityKind::MorphTarget,
                expected: expected_owner,
                actual: foreign_owner,
            })
        );
    }

    #[test]
    fn valid_scene_wide_and_owned_handles_resolve() {
        let current_generation = generation();
        let owner = LogicalMeshIndex::try_from(3)
            .expect("index should fit the canonical range")
            .handle(current_generation);
        let node = NodeIndex::try_from(1)
            .expect("index should fit the canonical range")
            .handle(current_generation);
        let primitive = PrimitiveLocalIndex::try_from(0)
            .expect("index should fit the canonical range")
            .handle(owner);
        let nodes = ["root", "child"];
        let primitives = ["triangles"];

        assert_eq!(
            lookup_scene_wide(current_generation, node, &nodes),
            Ok(&"child")
        );
        assert_eq!(
            lookup_owned(current_generation, owner, primitive, &primitives),
            Ok(&"triangles")
        );
    }
}
