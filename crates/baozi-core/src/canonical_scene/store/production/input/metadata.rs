use crate::extension::ExtensionValueRef;
use crate::namespaced::NamespacedName;

use super::lineage::{ProfileOwner, SourceIdentityLocalId};

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum MetadataAuthoringOwner {
    Entity(ProfileOwner),
    SourceObject(SourceIdentityLocalId),
}

/// One descriptive metadata entry for a canonical entity or a retained source object.
#[derive(Debug, Clone, Copy)]
pub struct MetadataAuthoringEntry<'input> {
    owner: MetadataAuthoringOwner,
    key: &'input NamespacedName,
    value: ExtensionValueRef<'input>,
}

impl<'input> MetadataAuthoringEntry<'input> {
    pub const fn new(
        owner: ProfileOwner,
        key: &'input NamespacedName,
        value: ExtensionValueRef<'input>,
    ) -> Self {
        Self {
            owner: MetadataAuthoringOwner::Entity(owner),
            key,
            value,
        }
    }

    /// Attaches descriptive metadata to one retained source object, independently of entities.
    pub const fn for_source(
        source: SourceIdentityLocalId,
        key: &'input NamespacedName,
        value: ExtensionValueRef<'input>,
    ) -> Self {
        Self {
            owner: MetadataAuthoringOwner::SourceObject(source),
            key,
            value,
        }
    }

    pub(in crate::canonical_scene::store) const fn owner(self) -> MetadataAuthoringOwner {
        self.owner
    }

    pub(in crate::canonical_scene::store) const fn key(self) -> &'input NamespacedName {
        self.key
    }

    pub(in crate::canonical_scene::store) const fn value(self) -> ExtensionValueRef<'input> {
        self.value
    }
}

/// Borrowed descriptive metadata supplied alongside one lineage batch.
///
/// Entries for one owner may be interleaved with other owners, but their keys must appear in
/// strictly increasing byte order. Central validation uses that local ordering to reject duplicate
/// keys while compiling canonical owner buckets with bounded linear work.
#[derive(Debug, Clone, Copy)]
pub struct MetadataAuthoringBatch<'input> {
    entries: &'input [MetadataAuthoringEntry<'input>],
}

impl<'input> MetadataAuthoringBatch<'input> {
    pub const fn new(entries: &'input [MetadataAuthoringEntry<'input>]) -> Self {
        Self { entries }
    }

    pub const fn empty() -> Self {
        Self { entries: &[] }
    }

    pub(in crate::canonical_scene::store) const fn entries(
        self,
    ) -> &'input [MetadataAuthoringEntry<'input>] {
        self.entries
    }
}
