use super::validation::{
    StoreRuntimeError, bytes_view, canonical_record_view, outcome_evidence_record_view,
    sequence_kind_matches, sequence_view, utf8_view,
};
use super::{
    ByteBlock, CanonicalBytesRef, CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef,
    CanonicalStoreRef, CanonicalUtf8Ref, OutcomeEvidenceBytesRef, OutcomeEvidenceRecordRef,
    OutcomeEvidenceRecordView, OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef,
    OutcomeEvidenceUtf8Ref, RecordBlock, ScalarSequence, SequenceView, StoreDimensionKey,
    StoreDimensionSemanticId, StoreFallbackEnvelope, StoreKeyDomain, StorePhaseKey,
    StorePhaseSemanticId, StoredBlock, StoredScalar, Utf8Block,
};
use crate::ledger::LedgerCause;
use crate::retained::{
    PhysicalStorageAccount, PreparationFailureClass, PreparationFailureCode,
    PreparedRetainedEvidenceOnlyOutcome, PreparedRetainedEvidenceOnlySuccess,
    PreparedRetainedFallback, PreparedRetainedPublication, PreparedRetainedPublicationOutcome,
    PreparedRetainedRichFailure, PreparedRetainedSuccess, PublishedCanonicalOutput,
    PublishedFallbackOutput, PublishedLedgerEvidence, PublishedOutcomeEvidenceOutput,
    PublishedRichFailureOutput, RetainedLookupMember, TerminalEvidenceCause,
};
pub(super) type RawPreparedStore = PreparedRetainedPublication<
    StoredBlock,
    StoredBlock,
    StoreFallbackEnvelope,
    StorePhaseKey,
    StoreDimensionKey,
>;
pub(super) type RawPreparedStoreSuccess =
    PreparedRetainedSuccess<StoredBlock, StoredBlock, StorePhaseKey, StoreDimensionKey>;
type RawPreparedStoreFailure =
    PreparedRetainedRichFailure<StoredBlock, StorePhaseKey, StoreDimensionKey>;
type RawPreparedStoreFallback =
    PreparedRetainedFallback<StoreFallbackEnvelope, StorePhaseKey, StoreDimensionKey>;
type RawPreparedStoreEvidenceOnlySuccess =
    PreparedRetainedEvidenceOnlySuccess<StorePhaseKey, StoreDimensionKey>;

fn reject_lookup<T>() -> Option<T> {
    None
}

fn project_sequence_view<'a, T>(
    member: RetainedLookupMember<'a, StoredBlock>,
) -> Option<SequenceView<'a, T>>
where
    T: StoredScalar,
{
    if !matches!(member.block(), StoredBlock::Sequence(_)) {
        return reject_lookup();
    }

    if !member.schema_tag_matches() || !sequence_kind_matches::<T>(member.block()) {
        return reject_lookup();
    }

    let Some(view) = sequence_view::<T>(member.block()) else {
        return reject_lookup();
    };
    Some(view)
}

fn project_bytes_view(member: RetainedLookupMember<'_, StoredBlock>) -> Option<&[u8]> {
    if !matches!(member.block(), StoredBlock::Bytes(_)) {
        return reject_lookup();
    }

    if !member.schema_tag_matches() {
        return reject_lookup();
    }

    let Some(view) = bytes_view(member.block()) else {
        return reject_lookup();
    };
    Some(view)
}

fn project_utf8_view(member: RetainedLookupMember<'_, StoredBlock>) -> Option<&str> {
    if !matches!(member.block(), StoredBlock::Utf8(_)) {
        return reject_lookup();
    }

    if !member.schema_tag_matches() {
        return reject_lookup();
    }

    let Some(view) = utf8_view(member.block()) else {
        return reject_lookup();
    };
    Some(view)
}

fn project_canonical_record_view(
    member: RetainedLookupMember<'_, StoredBlock>,
) -> Option<CanonicalRecordView<'_>> {
    if !matches!(member.block(), StoredBlock::Records(_)) {
        return reject_lookup();
    }

    if !member.schema_tag_matches() {
        return reject_lookup();
    }

    let Some(view) = canonical_record_view(member.block()) else {
        return reject_lookup();
    };
    Some(view)
}

fn project_outcome_evidence_record_view(
    member: RetainedLookupMember<'_, StoredBlock>,
) -> Option<OutcomeEvidenceRecordView<'_>> {
    if !matches!(member.block(), StoredBlock::Records(_)) {
        return reject_lookup();
    }

    if !member.schema_tag_matches() {
        return reject_lookup();
    }

    let Some(view) = outcome_evidence_record_view(member.block()) else {
        return reject_lookup();
    };
    Some(view)
}

// Keep every canonical-source preflight on the same checked lookup rail as published readers.
// The rewrite and transcript planners must not inspect a retained member after only the first
// four taxonomy stages, otherwise their validation both bypasses the typed projection and leaves
// an active collector lookup behind.
pub(super) fn read_canonical_source_sequence<'a, T, Tag>(
    source: &'a PublishedCanonicalOutput<StoredBlock>,
    slot: CanonicalSequenceRef<T, Tag>,
) -> Option<SequenceView<'a, T>>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    project_sequence_view(source.aggregate().lookup_member(slot.raw)?)
}

pub(super) fn read_canonical_source_bytes<Tag>(
    source: &PublishedCanonicalOutput<StoredBlock>,
    slot: CanonicalBytesRef<Tag>,
) -> Option<&[u8]>
where
    Tag: Copy + Send + Sync + 'static,
{
    project_bytes_view(source.aggregate().lookup_member(slot.raw)?)
}

pub(super) fn read_canonical_source_utf8<Tag>(
    source: &PublishedCanonicalOutput<StoredBlock>,
    slot: CanonicalUtf8Ref<Tag>,
) -> Option<&str>
where
    Tag: Copy + Send + Sync + 'static,
{
    project_utf8_view(source.aggregate().lookup_member(slot.raw)?)
}

pub(super) fn read_canonical_source_records<'a, Tag>(
    source: &'a PublishedCanonicalOutput<StoredBlock>,
    slot: CanonicalRecordRef<Tag>,
) -> Option<CanonicalRecordView<'a>>
where
    Tag: Copy + Send + Sync + 'static,
{
    project_canonical_record_view(source.aggregate().lookup_member(slot.raw)?)
}

pub(super) fn classify_prepared_success<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>(
    raw: RawPreparedStore,
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
) -> PreparedStoreSuccessOutcome<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag> {
    match raw.into_outcome() {
        PreparedRetainedPublicationOutcome::Success(raw) => {
            PreparedStoreSuccessOutcome::Success(PreparedStoreSuccess {
                raw,
                canonical_root,
                evidence_root,
                domain,
            })
        }
        PreparedRetainedPublicationOutcome::RichFailure(raw) => {
            PreparedStoreSuccessOutcome::RichFailure(PreparedStoreFailure {
                raw,
                evidence_root,
                domain,
            })
        }
        PreparedRetainedPublicationOutcome::MinimalFallback(raw) => {
            PreparedStoreSuccessOutcome::MinimalFallback(PreparedStoreFallback { raw, domain })
        }
    }
}

pub(super) fn prepared_store_success_from_raw<
    CanonicalKind,
    CanonicalTag,
    EvidenceKind,
    EvidenceTag,
>(
    raw: RawPreparedStoreSuccess,
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
) -> PreparedStoreSuccess<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag> {
    PreparedStoreSuccess {
        raw,
        canonical_root,
        evidence_root,
        domain,
    }
}

pub(super) fn classify_prepared_failure<EvidenceKind, EvidenceTag>(
    raw: RawPreparedStore,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
) -> PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag> {
    match raw.into_outcome() {
        PreparedRetainedPublicationOutcome::RichFailure(raw) => {
            PreparedStoreFailureOutcome::RichFailure(PreparedStoreFailure {
                raw,
                evidence_root,
                domain,
            })
        }
        PreparedRetainedPublicationOutcome::MinimalFallback(raw) => {
            PreparedStoreFailureOutcome::MinimalFallback(PreparedStoreFallback { raw, domain })
        }
        PreparedRetainedPublicationOutcome::Success(_) => unreachable_store_state(),
    }
}

pub(super) fn classify_prepared_fallback(
    raw: RawPreparedStore,
    domain: StoreKeyDomain,
) -> PreparedStoreFallback {
    match raw.into_outcome() {
        PreparedRetainedPublicationOutcome::MinimalFallback(raw) => {
            PreparedStoreFallback { raw, domain }
        }
        PreparedRetainedPublicationOutcome::Success(_)
        | PreparedRetainedPublicationOutcome::RichFailure(_) => unreachable_store_state(),
    }
}

pub(super) fn classify_prepared_evidence_only(
    raw: PreparedRetainedEvidenceOnlyOutcome<
        StoreFallbackEnvelope,
        StorePhaseKey,
        StoreDimensionKey,
    >,
    domain: StoreKeyDomain,
) -> PreparedStoreEvidenceOnlyOutcome {
    match raw {
        PreparedRetainedEvidenceOnlyOutcome::Success(raw) => {
            PreparedStoreEvidenceOnlyOutcome::Success(PreparedStoreEvidenceOnlySuccess {
                raw,
                domain,
            })
        }
        PreparedRetainedEvidenceOnlyOutcome::MinimalFallback(raw) => {
            PreparedStoreEvidenceOnlyOutcome::MinimalFallback(PreparedStoreFallback { raw, domain })
        }
    }
}

/// Exact preparation outcome for a success candidate.
#[must_use = "every exact terminal branch owns the frozen publication"]
pub enum PreparedStoreSuccessOutcome<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag> {
    Success(PreparedStoreSuccess<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>),
    RichFailure(PreparedStoreFailure<EvidenceKind, EvidenceTag>),
    MinimalFallback(PreparedStoreFallback),
}

/// Exact preparation outcome for a rich-failure candidate.
#[must_use = "every exact terminal branch owns the frozen publication"]
pub enum PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag> {
    RichFailure(PreparedStoreFailure<EvidenceKind, EvidenceTag>),
    MinimalFallback(PreparedStoreFallback),
}

impl<EvidenceKind, EvidenceTag> PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag> {
    pub(super) fn quarantine_cause_drop_failure(self) {
        match self {
            Self::RichFailure(prepared) => prepared.raw.quarantine_cause_drop_failure(),
            Self::MinimalFallback(prepared) => prepared.quarantine_cause_drop_failure(),
        }
    }
}

/// Exact preparation outcome for a success that publishes no retained primary output.
#[must_use = "every exact terminal branch owns frozen ledger evidence"]
pub enum PreparedStoreEvidenceOnlyOutcome {
    Success(PreparedStoreEvidenceOnlySuccess),
    MinimalFallback(PreparedStoreFallback),
}

/// Exact prepared success after attribution has been frozen.
#[must_use = "prepared success must be handed off or deliberately dropped"]
pub struct PreparedStoreSuccess<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag> {
    raw: RawPreparedStoreSuccess,
    canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
}

impl<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    PreparedStoreSuccess<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
{
    /// Performs the allocation-free handoff directly into exact success owners.
    pub fn handoff(
        self,
    ) -> PublishedStoreSuccessParts<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag> {
        let raw = self.raw.handoff();
        PublishedStoreSuccessParts {
            canonical: PublishedCanonicalStore {
                raw: raw.canonical,
                root: self.canonical_root,
            },
            outcome_evidence: PublishedOutcomeEvidenceStore {
                raw: raw.outcome_evidence,
                root: self.evidence_root,
            },
            ledger_evidence: StoreLedgerEvidence::from_raw(raw.ledger_evidence, self.domain),
        }
    }
}

/// Exact prepared rich failure after attribution has been frozen.
#[must_use = "prepared rich failure must be handed off or deliberately dropped"]
pub struct PreparedStoreFailure<EvidenceKind, EvidenceTag> {
    raw: RawPreparedStoreFailure,
    evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
    domain: StoreKeyDomain,
}

impl<EvidenceKind, EvidenceTag> PreparedStoreFailure<EvidenceKind, EvidenceTag> {
    /// Performs the allocation-free handoff directly into exact rich-failure owners.
    pub fn handoff(self) -> PublishedStoreFailureParts<EvidenceKind, EvidenceTag> {
        let raw = self.raw.handoff();
        PublishedStoreFailureParts {
            outcome_evidence: PublishedRichFailureStore {
                raw: raw.outcome_evidence,
                root: self.evidence_root,
            },
            ledger_evidence: StoreLedgerEvidence::from_raw(raw.ledger_evidence, self.domain),
        }
    }
}

/// Exact prepared minimal fallback after attribution has been frozen.
#[must_use = "prepared fallback must be handed off or deliberately dropped"]
pub struct PreparedStoreFallback {
    raw: RawPreparedStoreFallback,
    domain: StoreKeyDomain,
}

impl PreparedStoreFallback {
    pub(super) fn quarantine_cause_drop_failure(self) {
        self.raw.quarantine_cause_drop_failure();
    }

    /// Performs the allocation-free handoff directly into exact fallback owners.
    pub fn handoff(self) -> PublishedStoreFallbackParts {
        let raw = self.raw.handoff();
        PublishedStoreFallbackParts {
            fallback: PublishedStoreFallbackOutput { raw: raw.fallback },
            ledger_evidence: StoreLedgerEvidence::from_raw(raw.ledger_evidence, self.domain),
        }
    }
}

/// Exact prepared success with frozen ledger evidence and no retained primary output.
#[must_use = "prepared evidence-only success must be handed off or deliberately dropped"]
pub struct PreparedStoreEvidenceOnlySuccess {
    raw: RawPreparedStoreEvidenceOnlySuccess,
    domain: StoreKeyDomain,
}

impl PreparedStoreEvidenceOnlySuccess {
    /// Performs the allocation-free handoff into frozen ledger evidence.
    pub fn handoff(self) -> PublishedStoreEvidenceOnlySuccessParts {
        let raw = self.raw.handoff();
        PublishedStoreEvidenceOnlySuccessParts {
            ledger_evidence: StoreLedgerEvidence::from_raw(raw.ledger_evidence, self.domain),
        }
    }
}

/// Exact published success owners after pure handoff.
#[must_use = "the split contains every success owner and frozen ledger evidence"]
pub struct PublishedStoreSuccessParts<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag> {
    pub canonical: PublishedCanonicalStore<CanonicalKind, CanonicalTag>,
    pub outcome_evidence: PublishedOutcomeEvidenceStore<EvidenceKind, EvidenceTag>,
    pub ledger_evidence: StoreLedgerEvidence,
}

/// Exact published rich-failure owners after pure handoff.
#[must_use = "the split contains rich-failure evidence and frozen ledger evidence"]
pub struct PublishedStoreFailureParts<EvidenceKind, EvidenceTag> {
    pub outcome_evidence: PublishedRichFailureStore<EvidenceKind, EvidenceTag>,
    pub ledger_evidence: StoreLedgerEvidence,
}

/// Exact published fallback owners after pure handoff.
#[must_use = "the split contains fallback ownership and frozen ledger evidence"]
pub struct PublishedStoreFallbackParts {
    pub fallback: PublishedStoreFallbackOutput,
    pub ledger_evidence: StoreLedgerEvidence,
}

/// Exact published success with no canonical or domain evidence owner.
#[must_use = "the split contains the frozen ledger evidence"]
pub struct PublishedStoreEvidenceOnlySuccessParts {
    pub ledger_evidence: StoreLedgerEvidence,
}

/// Canonical owning half with a typed, non-owning root reference.
pub struct PublishedCanonicalStore<Kind, Tag> {
    raw: PublishedCanonicalOutput<StoredBlock>,
    root: CanonicalStoreRef<Kind, Tag>,
}

impl<Kind, Tag> PublishedCanonicalStore<Kind, Tag> {
    pub(super) fn share_source(&self) -> &PublishedCanonicalOutput<StoredBlock> {
        &self.raw
    }

    /// Returns the typed root reference without transferring ownership.
    pub const fn root_ref(&self) -> CanonicalStoreRef<Kind, Tag> {
        self.root
    }

    /// Returns the number of retained canonical blocks.
    pub fn len(&self) -> usize {
        self.raw.aggregate().len()
    }

    /// Returns whether the canonical table is empty.
    pub fn is_empty(&self) -> bool {
        self.raw.aggregate().is_empty()
    }

    /// Returns the physical account that structurally owns this canonical output.
    pub fn account(&self) -> &PhysicalStorageAccount {
        self.raw.account()
    }

    /// Resolves one typed scalar sequence.
    pub fn sequence<T: StoredScalar, SlotTag>(
        &self,
        slot: CanonicalSequenceRef<T, SlotTag>,
    ) -> Option<SequenceView<'_, T>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        read_canonical_source_sequence(&self.raw, slot)
    }

    /// Resolves one binary block.
    pub fn bytes<SlotTag>(&self, slot: CanonicalBytesRef<SlotTag>) -> Option<&[u8]>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        read_canonical_source_bytes(&self.raw, slot)
    }

    /// Resolves one UTF-8 block.
    pub fn utf8<SlotTag>(&self, slot: CanonicalUtf8Ref<SlotTag>) -> Option<&str>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        read_canonical_source_utf8(&self.raw, slot)
    }

    /// Resolves one canonical record block.
    pub fn records<SlotTag>(
        &self,
        slot: CanonicalRecordRef<SlotTag>,
    ) -> Option<CanonicalRecordView<'_>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        read_canonical_source_records(&self.raw, slot)
    }
}

impl<T: StoredScalar, Tag> PublishedCanonicalStore<ScalarSequence<T>, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the typed scalar-sequence root.
    pub fn root(&self) -> SequenceView<'_, T> {
        self.sequence(self.root)
            .unwrap_or_else(|| published_root_invariant("canonical scalar sequence"))
    }
}

impl<Tag> PublishedCanonicalStore<ByteBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the binary root.
    pub fn root(&self) -> &[u8] {
        self.bytes(self.root)
            .unwrap_or_else(|| published_root_invariant("canonical bytes"))
    }
}

impl<Tag> PublishedCanonicalStore<Utf8Block, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the UTF-8 root.
    pub fn root(&self) -> &str {
        self.utf8(self.root)
            .unwrap_or_else(|| published_root_invariant("canonical UTF-8"))
    }
}

impl<Tag> PublishedCanonicalStore<RecordBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the record root.
    pub fn root(&self) -> CanonicalRecordView<'_> {
        self.records(self.root)
            .unwrap_or_else(|| published_root_invariant("canonical records"))
    }
}

/// Outcome-evidence owning half with a typed, non-owning root reference.
pub struct PublishedOutcomeEvidenceStore<Kind, Tag> {
    raw: PublishedOutcomeEvidenceOutput<StoredBlock>,
    root: OutcomeEvidenceStoreRef<Kind, Tag>,
}

impl<Kind, Tag> PublishedOutcomeEvidenceStore<Kind, Tag> {
    pub(super) fn share_source(&self) -> &PublishedOutcomeEvidenceOutput<StoredBlock> {
        &self.raw
    }

    pub(super) fn belongs_to_account(&self, account: &PhysicalStorageAccount) -> bool {
        self.raw.aggregate().belongs_to(account)
    }

    /// Returns the typed root reference without transferring ownership.
    pub const fn root_ref(&self) -> OutcomeEvidenceStoreRef<Kind, Tag> {
        self.root
    }

    /// Returns the number of retained outcome-evidence blocks.
    pub fn len(&self) -> usize {
        self.raw.aggregate().len()
    }

    /// Returns whether the evidence table is empty.
    pub fn is_empty(&self) -> bool {
        self.raw.aggregate().is_empty()
    }

    /// Resolves one typed scalar sequence.
    pub fn sequence<T: StoredScalar, SlotTag>(
        &self,
        slot: OutcomeEvidenceSequenceRef<T, SlotTag>,
    ) -> Option<SequenceView<'_, T>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        project_sequence_view(self.raw.aggregate().lookup_member(slot.raw)?)
    }

    /// Resolves one binary block.
    pub fn bytes<SlotTag>(&self, slot: OutcomeEvidenceBytesRef<SlotTag>) -> Option<&[u8]>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        project_bytes_view(self.raw.aggregate().lookup_member(slot.raw)?)
    }

    /// Resolves one UTF-8 block.
    pub fn utf8<SlotTag>(&self, slot: OutcomeEvidenceUtf8Ref<SlotTag>) -> Option<&str>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        project_utf8_view(self.raw.aggregate().lookup_member(slot.raw)?)
    }

    /// Resolves one outcome-evidence record block.
    pub fn records<SlotTag>(
        &self,
        slot: OutcomeEvidenceRecordRef<SlotTag>,
    ) -> Option<OutcomeEvidenceRecordView<'_>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        project_outcome_evidence_record_view(self.raw.aggregate().lookup_member(slot.raw)?)
    }
}

impl<T: StoredScalar, Tag> PublishedOutcomeEvidenceStore<ScalarSequence<T>, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the typed scalar-sequence root.
    pub fn root(&self) -> SequenceView<'_, T> {
        self.sequence(self.root)
            .unwrap_or_else(|| published_root_invariant("outcome-evidence scalar sequence"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStore<ByteBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the binary root.
    pub fn root(&self) -> &[u8] {
        self.bytes(self.root)
            .unwrap_or_else(|| published_root_invariant("outcome-evidence bytes"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStore<Utf8Block, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the UTF-8 root.
    pub fn root(&self) -> &str {
        self.utf8(self.root)
            .unwrap_or_else(|| published_root_invariant("outcome-evidence UTF-8"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStore<RecordBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the record root.
    pub fn root(&self) -> OutcomeEvidenceRecordView<'_> {
        self.records(self.root)
            .unwrap_or_else(|| published_root_invariant("outcome-evidence records"))
    }
}

/// Rich-failure evidence owner with frozen cause and physical account lineage.
pub struct PublishedRichFailureStore<Kind, Tag> {
    raw: PublishedRichFailureOutput<StoredBlock>,
    root: OutcomeEvidenceStoreRef<Kind, Tag>,
}

impl<Kind, Tag> PublishedRichFailureStore<Kind, Tag> {
    /// Returns the typed root reference.
    pub const fn root_ref(&self) -> OutcomeEvidenceStoreRef<Kind, Tag> {
        self.root
    }

    /// Returns the physical account retained by failure evidence.
    pub fn account(&self) -> &PhysicalStorageAccount {
        self.raw.account()
    }

    /// Borrows the evidence table through a typed wrapper.
    pub fn outcome_evidence(&self) -> PublishedOutcomeEvidenceStoreRef<'_, Kind, Tag> {
        PublishedOutcomeEvidenceStoreRef {
            raw: self.raw.outcome_evidence(),
            root: self.root,
        }
    }
}

/// Borrowed typed view of a rich-failure evidence owner.
pub struct PublishedOutcomeEvidenceStoreRef<'a, Kind, Tag> {
    raw: &'a PublishedOutcomeEvidenceOutput<StoredBlock>,
    root: OutcomeEvidenceStoreRef<Kind, Tag>,
}

impl<Kind, Tag> PublishedOutcomeEvidenceStoreRef<'_, Kind, Tag> {
    /// Returns the number of retained blocks in this evidence table.
    pub fn len(&self) -> usize {
        self.raw.aggregate().len()
    }

    /// Returns whether this evidence table contains no retained blocks.
    pub fn is_empty(&self) -> bool {
        self.raw.aggregate().is_empty()
    }

    /// Resolves one typed scalar sequence.
    pub fn sequence<T: StoredScalar, SlotTag>(
        &self,
        slot: OutcomeEvidenceSequenceRef<T, SlotTag>,
    ) -> Option<SequenceView<'_, T>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        project_sequence_view(self.raw.aggregate().lookup_member(slot.raw)?)
    }

    /// Resolves one binary block.
    pub fn bytes<SlotTag>(&self, slot: OutcomeEvidenceBytesRef<SlotTag>) -> Option<&[u8]>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        project_bytes_view(self.raw.aggregate().lookup_member(slot.raw)?)
    }

    /// Resolves one UTF-8 block.
    pub fn utf8<SlotTag>(&self, slot: OutcomeEvidenceUtf8Ref<SlotTag>) -> Option<&str>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        project_utf8_view(self.raw.aggregate().lookup_member(slot.raw)?)
    }

    /// Resolves one outcome-evidence record block.
    pub fn records<SlotTag>(
        &self,
        slot: OutcomeEvidenceRecordRef<SlotTag>,
    ) -> Option<OutcomeEvidenceRecordView<'_>>
    where
        SlotTag: Copy + Send + Sync + 'static,
    {
        project_outcome_evidence_record_view(self.raw.aggregate().lookup_member(slot.raw)?)
    }

    /// Returns the typed root reference.
    pub const fn root_ref(&self) -> OutcomeEvidenceStoreRef<Kind, Tag> {
        self.root
    }
}

impl<T: StoredScalar, Tag> PublishedOutcomeEvidenceStoreRef<'_, ScalarSequence<T>, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the exact rich-failure scalar-sequence root.
    pub fn root(&self) -> SequenceView<'_, T> {
        self.sequence(self.root)
            .unwrap_or_else(|| published_root_invariant("rich-failure scalar sequence"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStoreRef<'_, ByteBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the exact rich-failure binary root.
    pub fn root(&self) -> &[u8] {
        self.bytes(self.root)
            .unwrap_or_else(|| published_root_invariant("rich-failure bytes"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStoreRef<'_, Utf8Block, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the exact rich-failure UTF-8 root.
    pub fn root(&self) -> &str {
        self.utf8(self.root)
            .unwrap_or_else(|| published_root_invariant("rich-failure UTF-8"))
    }
}

impl<Tag> PublishedOutcomeEvidenceStoreRef<'_, RecordBlock, Tag>
where
    Tag: Copy + Send + Sync + 'static,
{
    /// Resolves the exact rich-failure record root.
    pub fn root(&self) -> OutcomeEvidenceRecordView<'_> {
        self.records(self.root)
            .unwrap_or_else(|| published_root_invariant("rich-failure records"))
    }
}

/// Frozen destination-aware ledger evidence.
///
/// This terminal owner retains the runtime domain anchor and the metered key-bearing mapping, so
/// semantic and ordinal queries remain valid after the producer or plugin-facing owner is gone.
pub struct StoreLedgerEvidence {
    storage: StoreLedgerEvidenceStorage,
    domain: StoreLedgerDomain,
}

enum StoreLedgerDomain {
    Ordinary(StoreKeyDomain),
    Quarantined(std::mem::ManuallyDrop<StoreKeyDomain>),
}

impl std::ops::Deref for StoreLedgerDomain {
    type Target = StoreKeyDomain;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(domain) => domain,
            Self::Quarantined(domain) => domain,
        }
    }
}

enum StoreLedgerEvidenceStorage {
    Retained(PublishedLedgerEvidence<StorePhaseKey, StoreDimensionKey>),
    Unattached {
        snapshot: crate::LedgerSnapshot<StorePhaseKey, StoreDimensionKey>,
        terminal_cause: StoreTerminalCause,
    },
}

impl StoreLedgerEvidence {
    const fn from_raw(
        raw: PublishedLedgerEvidence<StorePhaseKey, StoreDimensionKey>,
        domain: StoreKeyDomain,
    ) -> Self {
        Self {
            storage: StoreLedgerEvidenceStorage::Retained(raw),
            domain: StoreLedgerDomain::Ordinary(domain),
        }
    }

    pub(super) fn from_unattached_snapshot(
        snapshot: crate::LedgerSnapshot<StorePhaseKey, StoreDimensionKey>,
        terminal_cause: StoreTerminalCause,
        domain: StoreKeyDomain,
        contained: bool,
    ) -> Self {
        // The actual invocation residue determines the original domain's disposition.
        // Semantic failure alone does not require retaining a clean domain.
        let domain = if contained {
            StoreLedgerDomain::Quarantined(std::mem::ManuallyDrop::new(domain))
        } else {
            StoreLedgerDomain::Ordinary(domain)
        };
        Self {
            storage: StoreLedgerEvidenceStorage::Unattached {
                snapshot,
                terminal_cause,
            },
            domain,
        }
    }

    /// Returns the complete frozen terminal header.
    pub fn header(&self) -> StoreLedgerEvidenceHeader {
        match &self.storage {
            StoreLedgerEvidenceStorage::Retained(raw) => {
                let raw = raw
                    .header()
                    .unwrap_or_else(|| published_root_invariant("terminal evidence header"));
                StoreLedgerEvidenceHeader {
                    schema_revision: raw.schema_revision(),
                    terminal_cause: match raw.terminal_cause() {
                        TerminalEvidenceCause::Success => StoreTerminalCause::Success,
                        TerminalEvidenceCause::Failure(cause) => {
                            StoreTerminalCause::Failure(StorePreparationFailure::from_raw(cause))
                        }
                    },
                    last_completed_phase: raw
                        .last_completed_phase()
                        .map(StorePhaseKey::semantic_id),
                    dimension_count: raw.dimension_count(),
                    phase_count: raw.phase_count(),
                    current_live_bytes: raw.current_live_bytes(),
                    peak_live_bytes: raw.peak_live_bytes(),
                    active_reservations: raw.active_reservations(),
                    current_administrative_bytes: raw.current_administrative_bytes(),
                    peak_administrative_bytes: raw.peak_administrative_bytes(),
                }
            }
            StoreLedgerEvidenceStorage::Unattached {
                snapshot,
                terminal_cause,
            } => StoreLedgerEvidenceHeader {
                schema_revision: crate::retained::TERMINAL_LEDGER_EVIDENCE_SCHEMA_REVISION,
                terminal_cause: *terminal_cause,
                last_completed_phase: snapshot
                    .last_completed_phase()
                    .map(StorePhaseKey::semantic_id),
                dimension_count: snapshot.dimensions().len(),
                phase_count: snapshot.phases().len(),
                current_live_bytes: snapshot.current_live_bytes(),
                peak_live_bytes: snapshot.peak_live_bytes(),
                active_reservations: snapshot.active_reservations(),
                current_administrative_bytes: snapshot.current_administrative_bytes(),
                peak_administrative_bytes: snapshot.peak_administrative_bytes(),
            },
        }
    }

    /// Returns one frozen dimension snapshot after exact key-domain validation.
    pub fn dimension(
        &self,
        dimension: StoreDimensionKey,
    ) -> Result<StoreDimensionSnapshot, StoreRuntimeError> {
        if !self.domain.owns_dimension(dimension) {
            return Err(StoreRuntimeError::ForeignDimensionDomain { dimension });
        }
        let raw = match &self.storage {
            StoreLedgerEvidenceStorage::Retained(raw) => raw.dimension(dimension),
            StoreLedgerEvidenceStorage::Unattached { snapshot, .. } => {
                snapshot.dimension(dimension).copied()
            }
        }
        .ok_or(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
            dimension,
        }))?;
        Ok(StoreDimensionSnapshot::from_raw(dimension, raw))
    }

    /// Returns one frozen dimension snapshot by stable semantic identity.
    pub fn dimension_by_semantic(
        &self,
        dimension: StoreDimensionSemanticId,
    ) -> Option<StoreDimensionSnapshot> {
        let dimension = self.domain.try_admit_dimension(dimension)?;
        self.dimension(dimension).ok()
    }

    /// Enumerates one frozen dimension and its stable semantic identity by canonical ordinal.
    pub fn dimension_at(&self, index: usize) -> Option<StoreDimensionSnapshot> {
        match &self.storage {
            StoreLedgerEvidenceStorage::Retained(raw) => {
                let dimension = raw.dimension_key_at(index)?;
                self.dimension(dimension).ok()
            }
            StoreLedgerEvidenceStorage::Unattached { snapshot, .. } => {
                let (dimension, raw) = snapshot.dimensions().nth(index)?;
                self.domain
                    .owns_dimension(dimension)
                    .then(|| StoreDimensionSnapshot::from_raw(dimension, *raw))
            }
        }
    }

    /// Returns one frozen phase snapshot after exact key-domain validation.
    pub fn phase(&self, phase: StorePhaseKey) -> Result<StorePhaseSnapshot, StoreRuntimeError> {
        if !self.domain.owns_phase(phase) {
            return Err(StoreRuntimeError::ForeignPhaseDomain { phase });
        }
        let raw = match &self.storage {
            StoreLedgerEvidenceStorage::Retained(raw) => raw.phase(phase),
            StoreLedgerEvidenceStorage::Unattached { snapshot, .. } => {
                snapshot.phase(phase).copied()
            }
        }
        .ok_or(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase))?;
        Ok(StorePhaseSnapshot::from_raw(phase, raw))
    }

    /// Returns one frozen phase snapshot by stable semantic identity.
    pub fn phase_by_semantic(&self, phase: StorePhaseSemanticId) -> Option<StorePhaseSnapshot> {
        let phase = self.domain.try_admit_phase(phase)?;
        self.phase(phase).ok()
    }

    /// Enumerates one frozen phase and its stable semantic identity by canonical ordinal.
    pub fn phase_at(&self, index: usize) -> Option<StorePhaseSnapshot> {
        match &self.storage {
            StoreLedgerEvidenceStorage::Retained(raw) => {
                let phase = raw.phase_key_at(index)?;
                self.phase(phase).ok()
            }
            StoreLedgerEvidenceStorage::Unattached { snapshot, .. } => {
                let (phase, raw) = snapshot.phases().nth(index)?;
                self.domain
                    .owns_phase(phase)
                    .then(|| StorePhaseSnapshot::from_raw(phase, *raw))
            }
        }
    }
}

/// Complete copied header from one terminal retained-store operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreLedgerEvidenceHeader {
    schema_revision: u32,
    terminal_cause: StoreTerminalCause,
    last_completed_phase: Option<StorePhaseSemanticId>,
    dimension_count: usize,
    phase_count: usize,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_administrative_bytes: u64,
    peak_administrative_bytes: u64,
}

impl StoreLedgerEvidenceHeader {
    /// Returns the terminal evidence schema revision.
    pub const fn schema_revision(self) -> u32 {
        self.schema_revision
    }

    /// Returns the unique terminal cause recorded for this publication.
    pub const fn terminal_cause(self) -> StoreTerminalCause {
        self.terminal_cause
    }

    /// Returns the last semantically completed phase, when one completed.
    pub const fn last_completed_phase(self) -> Option<StorePhaseSemanticId> {
        self.last_completed_phase
    }

    /// Returns the number of frozen dimension entries.
    pub const fn dimension_count(self) -> usize {
        self.dimension_count
    }

    /// Returns the number of frozen phase entries.
    pub const fn phase_count(self) -> usize {
        self.phase_count
    }

    /// Returns operation-live bytes remaining at terminal freeze.
    pub const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    /// Returns the operation-live byte high-water mark.
    pub const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    /// Returns active reservations remaining at terminal freeze.
    pub const fn active_reservations(self) -> u64 {
        self.active_reservations
    }

    /// Returns ledger evidence bytes remaining at terminal freeze.
    pub const fn current_administrative_bytes(self) -> u64 {
        self.current_administrative_bytes
    }

    /// Returns the ledger evidence byte high-water mark.
    pub const fn peak_administrative_bytes(self) -> u64 {
        self.peak_administrative_bytes
    }
}

/// Unique terminal classification stored in the ledger evidence header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreTerminalCause {
    /// The validated canonical and outcome-evidence graph published successfully.
    Success,
    /// Preparation terminated with the contained failure.
    Failure(StorePreparationFailure),
}

/// Copied destination-aware dimension snapshot from a published operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreDimensionSnapshot {
    dimension: StoreDimensionSemanticId,
    attempted: u64,
    consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

impl StoreDimensionSnapshot {
    fn from_raw(dimension: StoreDimensionKey, raw: crate::DimensionSnapshot) -> Self {
        Self {
            dimension: dimension.semantic_id(),
            attempted: raw.attempted(),
            consumed: raw.consumed(),
            current_live_bytes: raw.current_live_bytes(),
            peak_live_bytes: raw.peak_live_bytes(),
            pending_publication: raw.pending_publication(),
            committed: raw.committed(),
            committed_canonical_output: raw.committed_canonical_output(),
            published_outcome_evidence: raw.published_outcome_evidence(),
        }
    }

    /// Returns the stable owner-defined dimension semantic identity.
    pub const fn dimension(self) -> StoreDimensionSemanticId {
        self.dimension
    }

    /// Returns attempted work.
    pub const fn attempted(self) -> u64 {
        self.attempted
    }

    /// Returns consumed work.
    pub const fn consumed(self) -> u64 {
        self.consumed
    }

    /// Returns operation-live bytes remaining at terminal freeze.
    pub const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    /// Returns peak operation-live bytes.
    pub const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    /// Returns publication units still pending at terminal freeze.
    pub const fn pending_publication(self) -> u64 {
        self.pending_publication
    }

    /// Returns total committed output in this dimension.
    pub const fn committed(self) -> u64 {
        self.committed
    }

    /// Returns committed canonical output.
    pub const fn committed_canonical_output(self) -> u64 {
        self.committed_canonical_output
    }

    /// Returns published outcome evidence.
    pub const fn published_outcome_evidence(self) -> u64 {
        self.published_outcome_evidence
    }
}

/// Copied accounting snapshot for one frozen store phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorePhaseSnapshot {
    phase: StorePhaseSemanticId,
    current_live_bytes: u64,
    peak_live_bytes: u64,
}

impl StorePhaseSnapshot {
    fn from_raw(phase: StorePhaseKey, raw: crate::PhaseSnapshot) -> Self {
        Self {
            phase: phase.semantic_id(),
            current_live_bytes: raw.current_live_bytes(),
            peak_live_bytes: raw.peak_live_bytes(),
        }
    }

    /// Returns the stable owner-defined phase semantic identity.
    pub const fn phase(self) -> StorePhaseSemanticId {
        self.phase
    }

    /// Returns operation-live bytes remaining in this phase at terminal freeze.
    pub const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    /// Returns the phase's operation-live byte high-water mark.
    pub const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }
}

/// Published allocation-free minimal fallback owner.
pub struct PublishedStoreFallbackOutput {
    raw: PublishedFallbackOutput<StoreFallbackEnvelope>,
}

impl PublishedStoreFallbackOutput {
    /// Returns the application-defined fallback envelope.
    pub fn value(&self) -> StoreFallbackEnvelope {
        *self.raw.value()
    }

    /// Returns the physical account that owns the fallback.
    pub fn account(&self) -> &PhysicalStorageAccount {
        self.raw.account()
    }
}

/// Public classification of the frozen terminal preparation cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorePreparationFailureClass {
    Requested,
    Control,
    ProductGraph,
    Ledger,
}

/// Allocation-free frozen terminal cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorePreparationFailure {
    class: StorePreparationFailureClass,
    code: u32,
}

impl StorePreparationFailure {
    pub(super) const fn from_raw(raw: PreparationFailureCode) -> Self {
        let class = match raw.class() {
            PreparationFailureClass::Requested => StorePreparationFailureClass::Requested,
            PreparationFailureClass::Control => StorePreparationFailureClass::Control,
            PreparationFailureClass::ProductGraph => StorePreparationFailureClass::ProductGraph,
            PreparationFailureClass::Ledger => StorePreparationFailureClass::Ledger,
        };
        Self {
            class,
            code: raw.code(),
        }
    }

    /// Returns the frozen cause class.
    pub const fn class(self) -> StorePreparationFailureClass {
        self.class
    }

    /// Returns the class-specific code.
    pub const fn code(self) -> u32 {
        self.code
    }
}

fn published_root_invariant(expected: &str) -> ! {
    panic!("validated published store violated its typed root invariant: expected {expected}")
}

fn unreachable_store_state() -> ! {
    panic!("concrete retained store reached an impossible private typestate")
}

#[cfg(test)]
mod unattached_domain_tests {
    use super::*;
    use crate::concrete_store::tests::{
        TestOwner, TestProgress, TestUnit, test_account, test_factory, test_operation_plan,
    };
    use crate::concrete_store::{RetainedStoreOperation, StoreInvocationRoot};
    use crate::ledger::ParserChannel;
    use crate::{FinalProgress, OperationControl};

    #[test]
    fn unattached_domain_disposition_follows_actual_residue_through_evidence_destruction() {
        // A requested failure with clean counters retires normally. Both a live token
        // and an unresolved zero-counter frame require the original domain to remain.
        for residue in 0..3 {
            let factory = test_factory();
            let account = test_account(&factory);
            let before = account.snapshot();
            let (ledger, start, keys) = test_operation_plan(u64::MAX, u64::MAX, u64::MAX, 1);
            let mut root = StoreInvocationRoot::<TestOwner, TestProgress, TestUnit>::new();
            let mut binding = RetainedStoreOperation::bind(
                &mut root,
                OperationControl::default(),
                ledger,
                &account,
                start,
            )
            .unwrap();
            let reservation = {
                let mut parts = binding
                    .raw
                    .exclusive_parts()
                    .unwrap_or_else(|_| panic!("actual prework"));
                let work = parts.work.bind([keys.work_dimension]).unwrap();
                if residue == 2 {
                    let frame = work
                        .begin_parser(&mut parts.work.work, [0], [None], ParserChannel::Child)
                        .unwrap();
                    assert_eq!(frame.remaining, [0]);
                    assert!(frame.active);
                } else {
                    parts.work.charge_observed(work, [2]).unwrap();
                }
                (residue == 1).then(|| {
                    parts
                        .ordinary
                        .storage
                        .reserve(keys.phase, keys.dimension, 7)
                        .unwrap()
                })
            };
            let pending = binding
                .defer_unattached(Some(PreparationFailureCode::domain(41)))
                .unwrap();
            let evidence = root
                .prepare_deferred_unattached(
                    pending,
                    FinalProgress::new(TestProgress::Build, TestUnit::Items, 1),
                )
                .unwrap();
            assert!(root.domain.is_none());
            assert_eq!(root.raw.must_retain_domain(), residue != 0);
            assert_eq!(
                matches!(evidence.domain, StoreLedgerDomain::Quarantined(_)),
                residue != 0
            );
            assert!(evidence.domain.owns_dimension(keys.dimension));
            if let StoreLedgerDomain::Quarantined(domain) = &evidence.domain {
                let retained: &std::mem::ManuallyDrop<StoreKeyDomain> = domain;
                assert!(retained.owns_dimension(keys.dimension));
            }
            let header = evidence.header();
            let StoreTerminalCause::Failure(cause) = header.terminal_cause() else {
                panic!("the original request cannot become success");
            };
            assert_eq!(
                cause.class(),
                if residue == 0 {
                    StorePreparationFailureClass::Requested
                } else {
                    StorePreparationFailureClass::Ledger
                }
            );
            if residue == 0 {
                assert_eq!(cause.code(), 41);
            }
            let expected_live = if residue == 1 { 7 } else { 0 };
            assert_eq!(header.current_live_bytes(), expected_live);
            assert_eq!(header.active_reservations(), u64::from(residue == 1));
            assert_eq!(
                evidence
                    .dimension(keys.dimension)
                    .unwrap()
                    .current_live_bytes(),
                expected_live
            );
            let work = evidence.dimension(keys.work_dimension).unwrap();
            let expected_work = if residue == 2 { 0 } else { 2 };
            assert_eq!(
                (work.attempted(), work.consumed()),
                (expected_work, expected_work)
            );
            drop(reservation);
            assert_eq!(evidence.header().current_live_bytes(), expected_live);
            let after = account.snapshot();
            assert_eq!(after.poisoned(), residue != 0);
            assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
            assert_eq!(
                after.live_retained_allocations(),
                before.live_retained_allocations()
            );
            if residue == 1 {
                drop(root);
                assert!(evidence.domain.owns_dimension(keys.dimension));
                assert_eq!(evidence.header().current_live_bytes(), expected_live);
                drop(evidence);
            } else {
                drop(evidence);
                drop(root);
            }
            assert_eq!(account.snapshot(), after);
        }
    }
}
