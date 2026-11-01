//! Direct borrowed readers for descriptive metadata and its source-object ownership.

use super::{ValidatedScene, expect_validated};
use crate::canonical_scene::store::api::validation::SceneProfileOwner;
use crate::canonical_scene::store::construction::node_scene::semantic::profile::{
    NodeSceneProfileResource, NodeSceneProfileResourceStability, NodeSceneProfileSource,
    NodeSceneProfileSourceLocator, NodeSceneProfileTables,
};
pub use crate::canonical_scene::store::production::input::lineage::SessionOpaqueReason as ProfileSessionOpaqueReason;
use crate::canonical_scene::store::production::{CanonicalMetadataOwner, CanonicalProfileOwner};
use crate::canonical_scene::store::typed_storage::TableRange;
use crate::canonical_scene::store::typed_storage::profile::{
    MetadataPayloads, StoredMetadataValue,
};
use crate::identity::SceneGenerationId;
use crate::provenance::{DigestAlgorithm, SourceObjectKind};

pub(super) fn tables(scene: &ValidatedScene) -> NodeSceneProfileTables<'_> {
    expect_validated(NodeSceneProfileTables::try_new(
        scene.as_node_scene().read_profile(),
    ))
}

/// A canonical entity owner qualified by one immutable Scene generation.
///
/// Its descriptor uses canonical coordinates, including the hidden synthetic-root offset for
/// nodes. Compare tokens obtained from entity views rather than constructing numeric indices.
/// This same token selects the entity's retained provenance via
/// [`ValidatedScene::owner_provenance`], even when it has no descriptive metadata entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityMetadataOwner {
    generation: SceneGenerationId,
    owner: CanonicalProfileOwner,
}

impl EntityMetadataOwner {
    pub(super) fn new(scene: &ValidatedScene, owner: CanonicalProfileOwner) -> Self {
        Self::from_parts(scene.generation(), owner)
    }

    pub(super) const fn from_parts(
        generation: SceneGenerationId,
        owner: CanonicalProfileOwner,
    ) -> Self {
        Self { generation, owner }
    }

    pub const fn generation(self) -> SceneGenerationId {
        self.generation
    }

    pub(super) const fn owner(self) -> CanonicalProfileOwner {
        self.owner
    }

    pub const fn descriptor(self) -> SceneProfileOwner {
        SceneProfileOwner::from_canonical(self.owner)
    }
}

/// The actual owner of one descriptive metadata entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetadataOwnerRef<'s> {
    Entity(EntityMetadataOwner),
    SourceObject(SourceIdentityRef<'s>),
}

/// One source object's retained, format-defined locator. Namespaced text borrows the Scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceObjectLocatorRef<'s> {
    Indexed {
        kind: SourceObjectKind,
        index: u64,
    },
    Namespaced {
        schema: &'s str,
        schema_version: u32,
        value: &'s str,
    },
}

/// The replay-stability evidence belonging to a retained public resource identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileResourceStabilityRef<'s> {
    Stable {
        owner: &'s str,
        schema: &'s str,
        schema_version: u32,
    },
    SessionOpaque(ProfileSessionOpaqueReason),
}

/// An observed, sanitized resource identity. This grants no resource access authority.
#[derive(Debug, Clone, Copy)]
pub struct ProfileResourceRef<'s> {
    generation: SceneGenerationId,
    resource: NodeSceneProfileResource<'s>,
}

impl<'s> ProfileResourceRef<'s> {
    pub const fn generation(self) -> SceneGenerationId {
        self.generation
    }
    pub const fn index(self) -> usize {
        self.resource.index()
    }
    pub const fn identity(self) -> &'s str {
        self.resource.identity()
    }
    pub fn stability(self) -> ProfileResourceStabilityRef<'s> {
        match self.resource.stability() {
            NodeSceneProfileResourceStability::Stable {
                owner,
                schema,
                schema_version,
            } => ProfileResourceStabilityRef::Stable {
                owner,
                schema,
                schema_version,
            },
            NodeSceneProfileResourceStability::SessionOpaque(reason) => {
                ProfileResourceStabilityRef::SessionOpaque(reason)
            }
        }
    }
}

/// A source identity retained independently of the canonical entities derived from it.
///
/// Source/resource resolution uses a segment range search in O(log segment count). Field getters
/// read the resolved row directly. Numeric positions are generation-local, not persistence keys.
/// Equality compares generation and row; correlate different Scenes using the retained identity
/// fields, with content digests and replay stability considered separately.
#[derive(Clone, Copy)]
pub struct SourceIdentityRef<'s> {
    scene: &'s ValidatedScene,
    source: NodeSceneProfileSource<'s>,
}

impl<'s> SourceIdentityRef<'s> {
    pub(super) fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self {
            scene,
            source: expect_validated(tables(scene).source_at(index)),
        }
    }
    pub fn generation(self) -> SceneGenerationId {
        self.scene.generation()
    }
    pub const fn index(self) -> usize {
        self.source.index()
    }
    pub const fn format(self) -> &'s str {
        self.source.format()
    }
    pub const fn display_name(self) -> Option<&'s str> {
        self.source.display_name()
    }
    pub fn locator(self) -> SourceObjectLocatorRef<'s> {
        match self.source.locator() {
            NodeSceneProfileSourceLocator::Indexed { kind, index } => {
                SourceObjectLocatorRef::Indexed { kind, index }
            }
            NodeSceneProfileSourceLocator::Namespaced {
                schema,
                schema_version,
                value,
            } => SourceObjectLocatorRef::Namespaced {
                schema,
                schema_version,
                value,
            },
        }
    }
    pub fn resource(self) -> ProfileResourceRef<'s> {
        ProfileResourceRef {
            generation: self.generation(),
            resource: expect_validated(tables(self.scene).resource_at(self.source.resource())),
        }
    }
    pub fn content_digest(self) -> Option<(DigestAlgorithm, &'s [u8])> {
        self.source
            .content_digest()
            .map(|digest| (digest.algorithm(), digest.bytes()))
    }

    /// Borrows only this source object's metadata in key byte order.
    ///
    /// Two binary searches cost O(log metadata count); each yielded entry costs O(1).
    /// An object without descriptive metadata returns an empty iterator. No payload is copied.
    pub fn metadata_entries(
        self,
    ) -> impl ExactSizeIterator<Item = MetadataEntryRef<'s>> + DoubleEndedIterator + Clone + 's
    {
        let entries = &self
            .scene
            .as_node_scene()
            .read_profile()
            .state
            .as_ref()
            .metadata_entries;
        let owner = CanonicalMetadataOwner::SourceObject(self.source.index() as u32);
        let start = entries.partition_point(|entry| entry.owner < owner);
        let end = entries.partition_point(|entry| entry.owner <= owner);
        (start..end).map(move |row| MetadataEntryRef {
            scene: self.scene,
            row,
        })
    }
}

impl std::fmt::Debug for SourceIdentityRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceIdentityRef")
            .field("generation", &self.generation())
            .field("index", &self.index())
            .finish_non_exhaustive()
    }
}

impl PartialEq for SourceIdentityRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.generation() == other.generation() && self.index() == other.index()
    }
}
impl Eq for SourceIdentityRef<'_> {}

/// A non-nested metadata array borrowing its original scalar pool.
/// Boolean bytes are validated zero or one; vector/matrix arrays contain packed scalar lanes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetadataArrayRef<'s> {
    Bool(&'s [u8]),
    I64(&'s [i64]),
    U64(&'s [u64]),
    F64(&'s [f64]),
    Vec2(&'s [f32]),
    Vec3(&'s [f32]),
    Vec4(&'s [f32]),
    Color(&'s [f32]),
    Mat4(&'s [f32]),
}

impl MetadataArrayRef<'_> {
    pub const fn len(self) -> usize {
        match self {
            Self::Bool(v) => v.len(),
            Self::I64(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::F64(v) => v.len(),
            Self::Vec2(v) => v.len() / 2,
            Self::Vec3(v) => v.len() / 3,
            Self::Vec4(v) | Self::Color(v) => v.len() / 4,
            Self::Mat4(v) => v.len() / 16,
        }
    }
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// A closed descriptive value. No read parses, normalizes, or copies an encoded byte payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetadataValueRef<'s> {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(&'s str),
    Bytes {
        media_type: &'s str,
        payload: &'s [u8],
    },
    Vec2(&'s [f32]),
    Vec3(&'s [f32]),
    Vec4(&'s [f32]),
    Color(&'s [f32]),
    Mat4(&'s [f32]),
    DenseArray(MetadataArrayRef<'s>),
}

/// One metadata descriptor and its payload borrowed from an immutable Scene.
#[derive(Clone, Copy)]
pub struct MetadataEntryRef<'s> {
    scene: &'s ValidatedScene,
    row: usize,
}

impl<'s> MetadataEntryRef<'s> {
    pub fn owner(self) -> MetadataOwnerRef<'s> {
        match self
            .scene
            .as_node_scene()
            .read_profile()
            .state
            .as_ref()
            .metadata_entries[self.row]
            .owner
        {
            CanonicalMetadataOwner::Entity(owner) => {
                MetadataOwnerRef::Entity(EntityMetadataOwner::new(self.scene, owner))
            }
            CanonicalMetadataOwner::SourceObject(index) => {
                MetadataOwnerRef::SourceObject(SourceIdentityRef::at(self.scene, index as usize))
            }
        }
    }
    pub fn key(self) -> &'s str {
        let profile = self.scene.as_node_scene().read_profile();
        profile.state.as_ref().metadata_entries[self.row]
            .key
            .get_str(&profile.metadata.as_ref().text)
            .expect("validated metadata key range")
    }
    pub fn value(self) -> MetadataValueRef<'s> {
        let profile = self.scene.as_node_scene().read_profile();
        read_value(
            profile.metadata.as_ref(),
            profile.state.as_ref().metadata_entries[self.row].value,
        )
    }
}

impl std::fmt::Debug for MetadataEntryRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataEntryRef")
            .field("row", &self.row)
            .finish_non_exhaustive()
    }
}

fn slice<T>(pool: &[T], range: TableRange<T>) -> &[T] {
    range.get(pool).expect("validated metadata payload range")
}

fn read_value(p: &MetadataPayloads, value: StoredMetadataValue) -> MetadataValueRef<'_> {
    use MetadataArrayRef as A;
    use MetadataValueRef as V;
    use StoredMetadataValue as S;
    match value {
        S::Bool(r) => V::Bool(slice(&p.bools, r)[0] != 0),
        S::I64(r) => V::I64(slice(&p.i64s, r)[0]),
        S::U64(r) => V::U64(slice(&p.u64s, r)[0]),
        S::F64(r) => V::F64(slice(&p.f64s, r)[0]),
        S::String(r) => V::String(r.get_str(&p.text).expect("validated metadata string range")),
        S::Bytes {
            media_type,
            payload,
        } => V::Bytes {
            media_type: media_type
                .get_str(&p.text)
                .expect("validated metadata media type range"),
            payload: slice(&p.bytes, payload),
        },
        S::Vec2(r) => V::Vec2(slice(&p.f32s, r)),
        S::Vec3(r) => V::Vec3(slice(&p.f32s, r)),
        S::Vec4(r) => V::Vec4(slice(&p.f32s, r)),
        S::Color(r) => V::Color(slice(&p.f32s, r)),
        S::Mat4(r) => V::Mat4(slice(&p.f32s, r)),
        S::BoolArray(r) => V::DenseArray(A::Bool(slice(&p.bools, r))),
        S::I64Array(r) => V::DenseArray(A::I64(slice(&p.i64s, r))),
        S::U64Array(r) => V::DenseArray(A::U64(slice(&p.u64s, r))),
        S::F64Array(r) => V::DenseArray(A::F64(slice(&p.f64s, r))),
        S::Vec2Array(r) => V::DenseArray(A::Vec2(slice(&p.f32s, r))),
        S::Vec3Array(r) => V::DenseArray(A::Vec3(slice(&p.f32s, r))),
        S::Vec4Array(r) => V::DenseArray(A::Vec4(slice(&p.f32s, r))),
        S::ColorArray(r) => V::DenseArray(A::Color(slice(&p.f32s, r))),
        S::Mat4Array(r) => V::DenseArray(A::Mat4(slice(&p.f32s, r))),
    }
}

impl ValidatedScene {
    pub fn metadata_count(&self) -> usize {
        self.as_node_scene()
            .read_profile()
            .state
            .as_ref()
            .metadata_entries
            .len()
    }

    /// Iterates in canonical entity/source owner order, then owner-local key byte order.
    /// Payload slices borrow the Scene itself and can outlive this temporary iterator.
    pub fn metadata_entries(
        &self,
    ) -> impl ExactSizeIterator<Item = MetadataEntryRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.metadata_count()).map(|row| MetadataEntryRef { scene: self, row })
    }

    /// The Scene's own canonical entity metadata token.
    pub fn metadata_owner(&self) -> EntityMetadataOwner {
        EntityMetadataOwner::new(self, CanonicalProfileOwner::Scene)
    }

    pub fn source_identity_count(&self) -> usize {
        self.as_node_scene()
            .read_profile()
            .state
            .as_ref()
            .lineage_counts
            .sources
    }

    pub fn source_identities(
        &self,
    ) -> impl ExactSizeIterator<Item = SourceIdentityRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.source_identity_count()).map(|index| SourceIdentityRef::at(self, index))
    }

    pub fn profile_resource_count(&self) -> usize {
        self.as_node_scene()
            .read_profile()
            .state
            .as_ref()
            .lineage_counts
            .resources
    }

    pub fn profile_resources(
        &self,
    ) -> impl ExactSizeIterator<Item = ProfileResourceRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.profile_resource_count()).map(|index| ProfileResourceRef {
            generation: self.generation(),
            resource: expect_validated(tables(self).resource_at(index)),
        })
    }
}
