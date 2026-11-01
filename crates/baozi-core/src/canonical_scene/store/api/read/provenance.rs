//! Direct borrowed relationships over the retained provenance DAG.

use super::metadata::tables;
use super::{EntityMetadataOwner, SourceIdentityRef, ValidatedScene, expect_validated};
use crate::canonical_scene::store::construction::node_scene::semantic::profile::{
    NodeSceneProfileEvidence, NodeSceneProfileOperation, NodeSceneProfileOperationKind,
    NodeSceneProfileProvenance, NodeSceneProfileProvenanceIndices, NodeSceneProfileProvenanceKind,
};
use crate::identity::SceneGenerationId;
use crate::provenance::DigestAlgorithm;

/// An opaque provenance-node handle for exactly one immutable Scene generation.
///
/// Obtain handles from [`ProvenanceRef::id`]. They are process-local capabilities, not source
/// identities or persistence keys, and must be reacquired after publishing an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProvenanceId {
    generation: SceneGenerationId,
    index: usize,
}

impl ProvenanceId {
    pub const fn generation(self) -> SceneGenerationId {
        self.generation
    }
}

/// A rejected retained-provenance lookup.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceLookupError {
    #[error("provenance token belongs to a different scene generation")]
    GenerationMismatch {
        expected: SceneGenerationId,
        actual: SceneGenerationId,
    },
    #[error("provenance handle is out of range for a collection of length {length}")]
    OutOfRange { length: usize },
}

/// One retained node, borrowing its parent relation and Source/Operation summary.
///
/// This view does not discover transitive ancestors. Each relation is finite, exact-size and
/// independently iterable; callers choose a traversal budget and any shared-ancestor deduplication.
#[derive(Clone, Copy)]
pub struct ProvenanceRef<'s> {
    scene: &'s ValidatedScene,
    node: NodeSceneProfileProvenance<'s>,
}

impl<'s> ProvenanceRef<'s> {
    fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self {
            scene,
            node: expect_validated(tables(scene).provenance_at(index)),
        }
    }

    pub fn id(self) -> ProvenanceId {
        ProvenanceId {
            generation: self.scene.generation(),
            index: self.node.index(),
        }
    }

    /// Retained longest-path depth, counting this node. Source leaves have depth one.
    pub const fn depth(self) -> usize {
        self.node.depth()
    }

    /// Borrows immediate parents in strictly increasing retained-node order.
    ///
    /// Construction is O(1); each step resolves one edge and node in O(log segment count).
    /// Source leaves and parentless generated operations return an empty iterator. Every parent
    /// precedes this node. No recursive walk, deduplication or allocation occurs.
    pub fn parents(
        self,
    ) -> impl ExactSizeIterator<Item = ProvenanceRef<'s>> + DoubleEndedIterator + Clone + 's {
        relations(self.scene, self.node.parents())
    }

    /// Reads the retained distinction without inferring it from names or numeric indices.
    /// Resolving a Source costs O(log segment count); an Operation summary costs O(1).
    pub fn kind(self) -> ProvenanceKindRef<'s> {
        match self.node.kind() {
            NodeSceneProfileProvenanceKind::Source { source } => {
                ProvenanceKindRef::Source(SourceIdentityRef::at(self.scene, source))
            }
            NodeSceneProfileProvenanceKind::Operation(operation) => {
                ProvenanceKindRef::Operation(ProvenanceOperationRef {
                    scene: self.scene,
                    operation,
                })
            }
        }
    }
}

impl std::fmt::Debug for ProvenanceRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvenanceRef")
            .field("id", &self.id())
            .field("depth", &self.depth())
            .finish_non_exhaustive()
    }
}

impl PartialEq for ProvenanceRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for ProvenanceRef<'_> {}

/// A source object or an operation retained at one provenance node.
#[derive(Debug, Clone, Copy)]
pub enum ProvenanceKindRef<'s> {
    Source(SourceIdentityRef<'s>),
    Operation(ProvenanceOperationRef<'s>),
}

/// An allocation-free operation category, including borrowed namespaced kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProvenanceOperationKindRef<'s> {
    Generated,
    Default,
    Normalize,
    Repair,
    Split,
    Merge,
    Convert,
    Transform,
    Optimize,
    Namespaced(&'s str),
}

/// A retained operation's category, exact options digest and bounded durable evidence.
/// Detailed source-scope and cross-generation remap reports remain separate from this summary.
#[derive(Clone, Copy)]
pub struct ProvenanceOperationRef<'s> {
    scene: &'s ValidatedScene,
    operation: NodeSceneProfileOperation<'s>,
}

impl<'s> ProvenanceOperationRef<'s> {
    pub fn kind(self) -> ProvenanceOperationKindRef<'s> {
        use NodeSceneProfileOperationKind as K;
        use ProvenanceOperationKindRef as R;
        match self.operation.kind() {
            K::Generated => R::Generated,
            K::Default => R::Default,
            K::Normalize => R::Normalize,
            K::Repair => R::Repair,
            K::Split => R::Split,
            K::Merge => R::Merge,
            K::Convert => R::Convert,
            K::Transform => R::Transform,
            K::Optimize => R::Optimize,
            K::Namespaced(name) => R::Namespaced(name),
        }
    }

    pub fn options_digest(self) -> (DigestAlgorithm, &'s [u8]) {
        let digest = self.operation.options_digest();
        (digest.algorithm(), digest.bytes())
    }

    pub const fn evidence_count(self) -> usize {
        self.operation.evidence_count()
    }

    /// Borrows evidence in retained code order, including an explicit empty iterator.
    /// Construction is O(1); each yielded summary costs O(log segment count).
    pub fn evidence(
        self,
    ) -> impl ExactSizeIterator<Item = ProvenanceEvidenceRef<'s>> + DoubleEndedIterator + Clone + 's
    {
        let start = self.operation.evidence_offset();
        (start..start + self.evidence_count()).map(move |index| ProvenanceEvidenceRef {
            evidence: expect_validated(tables(self.scene).evidence_at(index)),
        })
    }
}

impl std::fmt::Debug for ProvenanceOperationRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvenanceOperationRef")
            .field("kind", &self.kind())
            .field("evidence_count", &self.evidence_count())
            .finish_non_exhaustive()
    }
}

/// One durable evidence summary. Text and digest bytes borrow their original retained storage.
#[derive(Debug, Clone, Copy)]
pub struct ProvenanceEvidenceRef<'s> {
    evidence: NodeSceneProfileEvidence<'s>,
}

impl<'s> ProvenanceEvidenceRef<'s> {
    pub const fn code(self) -> &'s str {
        self.evidence.code()
    }

    pub const fn occurrence_count(self) -> u64 {
        self.evidence.occurrence_count()
    }

    pub fn digest(self) -> Option<(DigestAlgorithm, &'s [u8])> {
        self.evidence
            .digest()
            .map(|digest| (digest.algorithm(), digest.bytes()))
    }
}

fn relations<'s>(
    scene: &'s ValidatedScene,
    indices: NodeSceneProfileProvenanceIndices<'s>,
) -> impl ExactSizeIterator<Item = ProvenanceRef<'s>> + DoubleEndedIterator + Clone + 's {
    indices.iter().map(move |index| {
        ProvenanceRef::at(
            scene,
            index.expect("validated provenance reference") as usize,
        )
    })
}

impl ValidatedScene {
    /// Total retained DAG nodes, including ancestry no longer directly attached to an owner.
    pub fn provenance_node_count(&self) -> usize {
        self.as_node_scene()
            .read_profile()
            .state
            .as_ref()
            .lineage_counts
            .provenance
    }

    /// Borrows all retained nodes in topological order. Each step costs O(log segment count).
    pub fn provenance_nodes(
        &self,
    ) -> impl ExactSizeIterator<Item = ProvenanceRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.provenance_node_count()).map(|index| ProvenanceRef::at(self, index))
    }

    /// Resolves an exact-generation handle in O(log segment count), without allocation.
    pub fn provenance(&self, id: ProvenanceId) -> Result<ProvenanceRef<'_>, ProvenanceLookupError> {
        check_generation(self, id.generation)?;
        if id.index >= self.provenance_node_count() {
            return Err(ProvenanceLookupError::OutOfRange {
                length: self.provenance_node_count(),
            });
        }
        Ok(ProvenanceRef::at(self, id.index))
    }

    /// Borrows one canonical owner's direct attachments after checking its exact generation.
    ///
    /// Obtain the token from `metadata_owner()` on the current Scene or entity view; descriptive
    /// metadata need not be present. Owner lookup costs O(log owner count). Iteration is in
    /// strictly increasing retained-node order and costs O(log segment count) per attachment.
    /// Validated canonical owners have at least one attachment. No transitive walk is performed.
    pub fn owner_provenance(
        &self,
        owner: EntityMetadataOwner,
    ) -> Result<
        impl ExactSizeIterator<Item = ProvenanceRef<'_>> + DoubleEndedIterator + Clone,
        ProvenanceLookupError,
    > {
        check_generation(self, owner.generation())?;
        let attachments = expect_validated(tables(self).owner_attachments(owner.owner()));
        Ok(relations(self, attachments))
    }
}

fn check_generation(
    scene: &ValidatedScene,
    actual: SceneGenerationId,
) -> Result<(), ProvenanceLookupError> {
    if actual != scene.generation() {
        return Err(ProvenanceLookupError::GenerationMismatch {
            expected: scene.generation(),
            actual,
        });
    }
    Ok(())
}
