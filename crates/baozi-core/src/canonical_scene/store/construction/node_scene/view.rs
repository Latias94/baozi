#[cfg(test)]
use baozi_operation::{PhysicalStorageAccount, StoreLedgerEvidence};

use crate::identity::SceneGenerationId;

use super::ValidatedNodeScene;
#[cfg(test)]
use super::{NodeSceneRichFailure, NodeSceneSuccessReport};

impl ValidatedNodeScene {
    pub(in crate::canonical_scene::store) const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn account(&self) -> &PhysicalStorageAccount {
        self.canonical.account()
    }

    pub(in crate::canonical_scene::store) fn canonical_block_count(&self) -> usize {
        self.canonical.len()
    }
}

#[cfg(test)]
impl NodeSceneSuccessReport {
    pub(in crate::canonical_scene::store) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.len()
    }

    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }
}

#[cfg(test)]
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
