use baozi_operation::{PhysicalStorageAccount, StoreLedgerEvidence};

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
