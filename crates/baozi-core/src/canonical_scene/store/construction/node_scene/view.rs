use baozi_operation::{PhysicalStorageAccount, StoreLedgerEvidence};

#[cfg(test)]
use crate::entity::AuthoredTransform;
use crate::identity::SceneGenerationId;

use super::{NodeSceneRichFailure, NodeSceneSuccessReport, ValidatedNodeScene};

impl ValidatedNodeScene {
    pub(in crate::canonical_scene::store) const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    pub(in crate::canonical_scene::store) fn account(&self) -> &PhysicalStorageAccount {
        self.canonical.account()
    }

    pub(in crate::canonical_scene::store) fn canonical_block_count(&self) -> usize {
        self.canonical.len()
    }
}

// Compatibility probes for construction tests. Production consumers use the
// strict semantic view so an invariant failure cannot masquerade as exhaustion.
#[cfg(test)]
impl ValidatedNodeScene {
    pub(in crate::canonical_scene::store) fn source_node_count(&self) -> Option<usize> {
        self.semantic_view().ok().map(|view| view.node_count())
    }

    pub(in crate::canonical_scene::store) fn source_root_count(&self) -> Option<usize> {
        self.semantic_view().ok().map(|view| view.root_count())
    }

    pub(in crate::canonical_scene::store) fn child(
        &self,
        parent_source_index: Option<usize>,
        child_ordinal: usize,
    ) -> Option<usize> {
        let source = self.semantic_view().ok()?;
        let mut cursor = source.into_cursor().ok()?;
        if parent_source_index.is_none() {
            let mut roots = cursor.take_roots().ok()?;
            return nth_child(&mut roots, child_ordinal);
        }

        let parent_source_index = parent_source_index?;
        let _roots = cursor.take_roots().ok()?;
        loop {
            let node = cursor.next_node().ok()??;
            if node.source_index() == parent_source_index {
                let mut children = node.into_children();
                return nth_child(&mut children, child_ordinal);
            }
        }
    }

    pub(in crate::canonical_scene::store) fn parent(
        &self,
        source_index: usize,
    ) -> Option<Option<usize>> {
        let source = self.semantic_view().ok()?;
        if source_index >= source.node_count() {
            return None;
        }
        let root_count = source.root_count();
        let mut cursor = source.into_cursor().ok()?;
        let mut roots = cursor.take_roots().ok()?;
        for _ in 0..root_count {
            if roots.next().ok()?? == source_index {
                return Some(None);
            }
        }

        loop {
            let node = cursor.next_node().ok()??;
            let parent_source_index = node.source_index();
            let child_count = node.child_count();
            let mut children = node.into_children();
            for _ in 0..child_count {
                if children.next().ok()?? == source_index {
                    return Some(Some(parent_source_index));
                }
            }
        }
    }

    pub(in crate::canonical_scene::store) fn authored_transform(
        &self,
        source_index: usize,
    ) -> Option<AuthoredTransform> {
        let source = self.semantic_view().ok()?;
        let mut cursor = source.into_cursor().ok()?;
        let _roots = cursor.take_roots().ok()?;
        loop {
            let node = cursor.next_node().ok()??;
            if node.source_index() == source_index {
                return Some(node.authored_transform());
            }
        }
    }
}

#[cfg(test)]
fn nth_child(
    children: &mut super::semantic::NodeSceneChildCursor<'_>,
    ordinal: usize,
) -> Option<usize> {
    for current in 0..=ordinal {
        let child = children.next().ok()??;
        if current == ordinal {
            return Some(child);
        }
    }
    None
}

impl NodeSceneSuccessReport {
    pub(in crate::canonical_scene::store) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.len()
    }

    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }
}

impl NodeSceneRichFailure {
    pub(in crate::canonical_scene::store) fn account(&self) -> &PhysicalStorageAccount {
        self.outcome_evidence.account()
    }

    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }

    pub(in crate::canonical_scene::store) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.outcome_evidence().len()
    }
}
