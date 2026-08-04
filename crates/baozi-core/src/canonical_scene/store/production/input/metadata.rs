use crate::extension::ExtensionValueRef;
use crate::namespaced::NamespacedName;

use super::lineage::ProfileOwner;

/// One descriptive metadata entry for a canonical owner.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct MetadataAuthoringEntry<'input> {
    owner: ProfileOwner,
    key: &'input NamespacedName,
    value: ExtensionValueRef<'input>,
}

impl<'input> MetadataAuthoringEntry<'input> {
    pub(in crate::canonical_scene::store) const fn new(
        owner: ProfileOwner,
        key: &'input NamespacedName,
        value: ExtensionValueRef<'input>,
    ) -> Self {
        Self { owner, key, value }
    }

    pub(in crate::canonical_scene::store) const fn owner(self) -> ProfileOwner {
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
pub(in crate::canonical_scene::store) struct MetadataAuthoringBatch<'input> {
    entries: &'input [MetadataAuthoringEntry<'input>],
}

impl<'input> MetadataAuthoringBatch<'input> {
    pub(in crate::canonical_scene::store) const fn new(
        entries: &'input [MetadataAuthoringEntry<'input>],
    ) -> Self {
        Self { entries }
    }

    pub(in crate::canonical_scene::store) const fn empty() -> Self {
        Self { entries: &[] }
    }

    pub(in crate::canonical_scene::store) const fn entries(
        self,
    ) -> &'input [MetadataAuthoringEntry<'input>] {
        self.entries
    }
}
