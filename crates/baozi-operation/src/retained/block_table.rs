use super::account::{AccountPoisonReason, AdmissionCharge};
use super::allocation::{
    AccountPoisonEventWitness, FinalOwnerReleaseOutcome, PolicyRetainedAllocation,
    RetainedAllocation, RetainedAllocationError, RetainedAllocationPlan, RetainedBuildFailureCode,
    RetainedFinalOwnerInner, RetainedLease, RetainedMembership, RetainedMembershipDestination,
    RetainedPayload, RetainedReleaseMode, RetainedSetId, RetainedValuePolicy,
    RetainedValueReleaseOutcome, observe_retained_payload_drop_failure,
};
use super::final_owner::{FinalOwnerAllocation, unreachable_final_owner_state};
use super::ownership::{
    RetainedStorageDropOutcome, drop_retained_allocation_before_attribution,
    finish_pre_attribution_cleanup, map_inherited_cleanup_error,
};
#[cfg(test)]
use super::record_test_member_lookup;
use super::{
    PendingAccountActionAdmission, PhysicalStorageAccount, RetainedBatchLimits,
    RetainedHandleSharePlan, RetainedPublicationError, RetainedWriterError, RetainedWriterPlan,
    map_account_action_admission_error, map_batch_error, try_admit_retained_share_work,
    try_admit_retained_share_work_before_physical,
};
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{
    AttributionBatchPlan, AttributionReservationBatch, Ledger, PublicationDestination,
};
#[cfg(feature = "_u4c-collector")]
use crate::u4c_collector::{
    BlockKind, LookupStage, record_block_begin, record_block_seal, record_lookup_rejected,
    record_lookup_stage, record_payload_backing_allocation, record_slot_publication,
};
use crate::{CapacityPlan, OperationOwner};
use std::any::TypeId;
use std::fmt;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct TableRetainedPolicy;

pub(super) type RetainedTableAllocation<Block> =
    PolicyRetainedAllocation<RetainedBlockTable<Block>, TableRetainedPolicy>;

impl<Block>
    FinalOwnerAllocation<
        RetainedFinalOwnerInner<RetainedPayload<RetainedBlockTable<Block>>, TableRetainedPolicy>,
    >
{
    pub(super) fn table_mut_during_construction(
        &mut self,
    ) -> Option<&mut RetainedBlockTable<Block>> {
        let payload = self.inner_mut_if_unique()?.value.as_mut()?;
        Some(&mut payload.value)
    }
}

fn observe_retained_table_pre_forget<Block>(
    payload: &RetainedPayload<RetainedBlockTable<Block>>,
    lease: &RetainedLease,
) -> AccountPoisonEventWitness {
    let poisoned = observe_retained_payload_drop_failure(lease);
    for member in &payload.value.members {
        let _ = member
            .allocation
            .account()
            .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
    }
    poisoned
}

impl<Block> RetainedValuePolicy<RetainedPayload<RetainedBlockTable<Block>>>
    for TableRetainedPolicy
{
    fn observe_pre_forget(
        payload: &RetainedPayload<RetainedBlockTable<Block>>,
        lease: &RetainedLease,
    ) -> AccountPoisonEventWitness {
        observe_retained_table_pre_forget(payload, lease)
    }

    fn release(
        payload: RetainedPayload<RetainedBlockTable<Block>>,
        mode: RetainedReleaseMode,
    ) -> RetainedValueReleaseOutcome {
        release_retained_table_payload(payload, mode)
    }
}

impl<Block> PolicyRetainedAllocation<RetainedBlockTable<Block>, TableRetainedPolicy> {
    pub(super) fn table_mut_during_construction(
        &mut self,
    ) -> Option<&mut RetainedBlockTable<Block>> {
        self.inner.table_mut_during_construction()
    }
}

impl PhysicalStorageAccount {
    fn checked_retained_table_charge<Block>(
        &self,
        plan: RetainedAllocationPlan,
    ) -> Result<AdmissionCharge, RetainedAllocationError> {
        self.checked_retained_charge_with_policy::<RetainedBlockTable<Block>, TableRetainedPolicy>(
            plan,
        )
    }

    #[cfg(test)]
    pub(super) fn test_checked_retained_table_charge<Block>(
        &self,
        total_blocks: usize,
    ) -> Result<AdmissionCharge, RetainedAllocationError> {
        let member_capacity =
            CapacityPlan::for_type::<RetainedBlockMember<Block>>(0, 0, total_blocks)
                .map_err(|_| RetainedAllocationError::AccountingOverflow)?;
        let nested_bytes = member_capacity
            .envelope_delta_bytes()
            .map_err(|_| RetainedAllocationError::AccountingOverflow)?;
        let allocation = RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
            .map_err(|_| RetainedAllocationError::AccountingOverflow)?;
        self.checked_retained_table_charge::<Block>(allocation)
    }
}

/// One destination's fixed block capacity and ledger placement.
///
/// `total_blocks` is the exact preallocated slot capacity, not a required final length. The table
/// root consumes one attribution slot in addition to every allocated member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedBlockTablePlan<P, D> {
    pub(super) writer: RetainedWriterPlan<P, D>,
    total_blocks: usize,
    max_new_blocks: usize,
}

impl<P: Copy, D: Copy> RetainedBlockTablePlan<P, D> {
    /// Creates a fixed table plan. `total_blocks` bounds every member while `max_new_blocks` bounds
    /// only allocations attributed to this operation; inherited members consume table capacity but
    /// retain their original physical charge and receive no new allocation attribution.
    pub const fn new(
        writer: RetainedWriterPlan<P, D>,
        total_blocks: usize,
        max_new_blocks: usize,
    ) -> Self {
        Self {
            writer,
            total_blocks,
            max_new_blocks,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RetainedSlotIdentity {
    pub(super) set_id: RetainedSetId,
    pub(super) index: usize,
    pub(super) tag: TypeId,
}

impl RetainedSlotIdentity {
    pub(crate) fn matches_tag<Tag: 'static>(self) -> bool {
        self.tag == TypeId::of::<Tag>()
    }
}

/// Non-owning projection to one canonical block in a branded retained publication.
///
/// A slot is valid only for the retained set that issued it. Sharing the same physical block into a
/// later generation issues a fresh slot; the old slot is rejected by the new aggregate.
#[must_use = "a retained slot is required to reference and validate its block"]
pub(crate) struct CanonicalRetainedSlot<Block, Tag> {
    pub(super) identity: RetainedSlotIdentity,
    pub(super) marker: PhantomData<fn() -> (Block, Tag)>,
}

impl<Block, Tag> Copy for CanonicalRetainedSlot<Block, Tag> {}

impl<Block, Tag> Clone for CanonicalRetainedSlot<Block, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Block, Tag> PartialEq for CanonicalRetainedSlot<Block, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl<Block, Tag> Eq for CanonicalRetainedSlot<Block, Tag> {}

impl<Block, Tag> fmt::Debug for CanonicalRetainedSlot<Block, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalRetainedSlot(..)")
    }
}

impl<Block, Tag> CanonicalRetainedSlot<Block, Tag> {
    pub(crate) const fn from_identity(identity: RetainedSlotIdentity) -> Self {
        Self {
            identity,
            marker: PhantomData,
        }
    }

    pub(crate) const fn identity(self) -> RetainedSlotIdentity {
        self.identity
    }
}

/// Non-owning projection to one outcome-evidence block in a branded retained publication.
///
/// Like a canonical slot, this identity is set-local and cannot be retagged or persisted as a
/// cross-generation identity.
#[must_use = "a retained slot is required to reference and validate its block"]
pub(crate) struct OutcomeEvidenceRetainedSlot<Block, Tag> {
    pub(super) identity: RetainedSlotIdentity,
    pub(super) marker: PhantomData<fn() -> (Block, Tag)>,
}

impl<Block, Tag> Copy for OutcomeEvidenceRetainedSlot<Block, Tag> {}

impl<Block, Tag> Clone for OutcomeEvidenceRetainedSlot<Block, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Block, Tag> PartialEq for OutcomeEvidenceRetainedSlot<Block, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl<Block, Tag> Eq for OutcomeEvidenceRetainedSlot<Block, Tag> {}

impl<Block, Tag> fmt::Debug for OutcomeEvidenceRetainedSlot<Block, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OutcomeEvidenceRetainedSlot(..)")
    }
}

impl<Block, Tag> OutcomeEvidenceRetainedSlot<Block, Tag> {
    pub(crate) const fn from_identity(identity: RetainedSlotIdentity) -> Self {
        Self {
            identity,
            marker: PhantomData,
        }
    }

    pub(crate) const fn identity(self) -> RetainedSlotIdentity {
        self.identity
    }
}

/// Scoped constructor that couples a complete immutable block to a domain-owned tag value.
///
/// Blocks are constructed exactly once and remain immutable after placement. The table interface
/// deliberately has no replacement, retirement, or generic mutable-access operation.
#[must_use = "the emplacer must place one complete block or be returned as a builder failure"]
pub(crate) struct BlockEmplacer<'a, Block> {
    marker: PhantomData<&'a mut Block>,
}

/// Unforgeable result of placing one complete block with a value witness.
#[must_use = "placement must be returned to the retained table builder"]
pub(crate) struct BlockPlacement<'a, Block, Tag> {
    block: Block,
    block_marker: PhantomData<&'a mut Block>,
    tag_marker: PhantomData<fn() -> Tag>,
}

impl<'a, Block> BlockEmplacer<'a, Block> {
    /// Places one complete immutable block and consumes the scoped emplacer.
    #[must_use = "the placement must be returned to the retained table builder"]
    pub fn place<Tag>(self, block: Block, tag: Tag) -> BlockPlacement<'a, Block, Tag>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let _ = tag;
        BlockPlacement {
            block,
            block_marker: PhantomData,
            tag_marker: PhantomData,
        }
    }
}

impl<Block, Tag> BlockPlacement<'_, Block, Tag> {
    fn into_block(self) -> Block {
        self.block
    }
}

pub(super) struct RetainedBlockMember<Block> {
    pub(super) allocation: RetainedAllocation<Block>,
    tag: TypeId,
    origin: RetainedBlockOrigin,
    reachable: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedBlockOrigin {
    New,
    Inherited,
}

pub(super) struct RetainedBlockTable<Block> {
    pub(super) members: Vec<RetainedBlockMember<Block>>,
    total_blocks: usize,
    max_new_blocks: usize,
    new_blocks: usize,
    qualified_capacity: usize,
}

impl<Block> RetainedBlockTable<Block> {
    pub(super) fn member(
        &self,
        identity: RetainedSlotIdentity,
        set_id: RetainedSetId,
    ) -> Option<&Block> {
        if identity.set_id != set_id {
            return None;
        }
        self.members
            .get(identity.index)
            .and_then(|member| (member.tag == identity.tag).then(|| member.allocation.as_ref()))
    }

    pub(super) fn member_allocation(
        &self,
        identity: RetainedSlotIdentity,
        set_id: RetainedSetId,
    ) -> Option<&RetainedAllocation<Block>> {
        #[cfg(test)]
        record_test_member_lookup();
        if identity.set_id != set_id {
            return None;
        }
        self.members
            .get(identity.index)
            .and_then(|member| (member.tag == identity.tag).then_some(&member.allocation))
    }

    pub(super) fn reset_reachable(&self) {
        for member in &self.members {
            member.reachable.store(false, Ordering::Release);
        }
    }

    pub(super) fn all_reachable(&self) -> bool {
        self.members
            .iter()
            .all(|member| member.reachable.load(Ordering::Acquire))
    }
}

pub(super) fn record_forgotten_table_edges<Block>(table: &RetainedTableAllocation<Block>) {
    let _ = table
        .account()
        .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
    for member in &table.as_ref().members {
        let _ = member
            .allocation
            .account()
            .record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
    }
}

pub(super) fn release_retained_table_payload<Block>(
    payload: RetainedPayload<RetainedBlockTable<Block>>,
    mode: RetainedReleaseMode,
) -> RetainedValueReleaseOutcome {
    let RetainedPayload {
        value: mut table,
        charge: _,
    } = payload;
    let members = std::mem::take(&mut table.members);
    let mut nested_failure = None;
    let mut nested_charge_unverified = None;
    let mut nested_ownership_unverified = None;
    for RetainedBlockMember {
        allocation,
        tag: _,
        origin,
        reachable: _,
    } in members
    {
        let outcome = if mode == RetainedReleaseMode::BeforeAttribution
            && origin == RetainedBlockOrigin::New
        {
            match allocation.try_release_unique_before_attribution() {
                Ok(outcome) => outcome,
                Err((allocation, poisoned)) => {
                    std::mem::forget(allocation);
                    FinalOwnerReleaseOutcome::OwnershipUnverified(poisoned)
                }
            }
        } else {
            allocation.release_storage()
        };
        match outcome {
            FinalOwnerReleaseOutcome::Clean => {}
            FinalOwnerReleaseOutcome::StorageReleaseUnverified(poisoned) => {
                nested_failure.get_or_insert(poisoned);
            }
            FinalOwnerReleaseOutcome::ChargeReleaseUnverified(poisoned) => {
                nested_charge_unverified.get_or_insert(poisoned);
            }
            FinalOwnerReleaseOutcome::OwnershipUnverified(poisoned) => {
                nested_ownership_unverified.get_or_insert(poisoned);
            }
        }
    }
    drop(table);
    if let Some(poisoned) = nested_failure {
        RetainedValueReleaseOutcome::NestedStorageReleaseUnverified(poisoned)
    } else if let Some(poisoned) = nested_ownership_unverified {
        RetainedValueReleaseOutcome::NestedOwnershipUnverified(poisoned)
    } else if let Some(poisoned) = nested_charge_unverified {
        RetainedValueReleaseOutcome::NestedChargeReleaseUnverified(poisoned)
    } else {
        RetainedValueReleaseOutcome::Clean
    }
}

/// Read-only reachability session for a success-capable canonical/evidence graph.
///
/// Validation must visit every unique slot in both tables. Repeated DAG edges are valid and count
/// once. Any lookup returning `None` invalidates the whole session even if the callback ignores it.
pub(crate) struct RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock> {
    pub(super) canonical: &'a RetainedBlockTable<CanonicalBlock>,
    pub(super) evidence: &'a RetainedBlockTable<EvidenceBlock>,
    pub(super) set_id: RetainedSetId,
    pub(super) canonical_seen: usize,
    pub(super) evidence_seen: usize,
    pub(super) valid: bool,
    pub(super) not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'a, CanonicalBlock, EvidenceBlock> RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock> {
    /// Visits one canonical slot, returning `None` and invalidating the session on mismatch.
    pub fn canonical<Tag>(
        &mut self,
        slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
    ) -> Option<&'a CanonicalBlock>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.set_id != self.set_id || slot.identity.tag != TypeId::of::<Tag>() {
            self.valid = false;
            return None;
        }
        let Some(member) = self.canonical.members.get(slot.identity.index) else {
            self.valid = false;
            return None;
        };
        if member.tag != slot.identity.tag {
            self.valid = false;
            return None;
        }
        if !member.reachable.swap(true, Ordering::AcqRel) {
            let Some(seen) = self.canonical_seen.checked_add(1) else {
                self.valid = false;
                return None;
            };
            self.canonical_seen = seen;
        }
        Some(member.allocation.as_ref())
    }

    /// Visits one outcome-evidence slot, returning `None` and invalidating the session on mismatch.
    pub fn outcome_evidence<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<&'a EvidenceBlock>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.set_id != self.set_id || slot.identity.tag != TypeId::of::<Tag>() {
            self.valid = false;
            return None;
        }
        let Some(member) = self.evidence.members.get(slot.identity.index) else {
            self.valid = false;
            return None;
        };
        if member.tag != slot.identity.tag {
            self.valid = false;
            return None;
        }
        if !member.reachable.swap(true, Ordering::AcqRel) {
            let Some(seen) = self.evidence_seen.checked_add(1) else {
                self.valid = false;
                return None;
            };
            self.evidence_seen = seen;
        }
        Some(member.allocation.as_ref())
    }
}

/// Read-only reachability session for an evidence-only graph.
///
/// Validation must visit every unique evidence slot. Repeated DAG edges count once, and a failed
/// lookup invalidates the complete session.
pub(crate) struct RetainedEvidenceClosureSession<'a, EvidenceBlock> {
    pub(super) evidence: &'a RetainedBlockTable<EvidenceBlock>,
    pub(super) set_id: RetainedSetId,
    pub(super) seen: usize,
    pub(super) valid: bool,
    pub(super) not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'a, EvidenceBlock> RetainedEvidenceClosureSession<'a, EvidenceBlock> {
    /// Visits one outcome-evidence slot, returning `None` and invalidating the session on mismatch.
    pub fn outcome_evidence<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<&'a EvidenceBlock>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.set_id != self.set_id || slot.identity.tag != TypeId::of::<Tag>() {
            self.valid = false;
            return None;
        }
        let Some(member) = self.evidence.members.get(slot.identity.index) else {
            self.valid = false;
            return None;
        };
        if member.tag != slot.identity.tag {
            self.valid = false;
            return None;
        }
        if !member.reachable.swap(true, Ordering::AcqRel) {
            let Some(seen) = self.seen.checked_add(1) else {
                self.valid = false;
                return None;
            };
            self.seen = seen;
        }
        Some(member.allocation.as_ref())
    }
}

pub(super) struct PublishedRetainedAggregate<Block> {
    pub(super) table: RetainedTableAllocation<Block>,
    pub(super) set_id: RetainedSetId,
}

/// One table-validated retained member awaiting concrete-store view projection.
pub(crate) struct RetainedLookupMember<'a, Block> {
    block: &'a Block,
    schema_tag_matches: bool,
}

impl<'a, Block> RetainedLookupMember<'a, Block> {
    pub(crate) const fn block(&self) -> &'a Block {
        self.block
    }

    pub(crate) const fn schema_tag_matches(&self) -> bool {
        self.schema_tag_matches
    }
}

impl<Block> PublishedRetainedAggregate<Block> {
    /// Resolves a retained terminal root before a concrete-store projection exists.
    ///
    /// Concrete-store published views must use `lookup_member` so the U4c checked lookup
    /// taxonomy remains observable.
    pub(super) fn member(&self, identity: RetainedSlotIdentity) -> Option<&Block> {
        self.table.as_ref().member(identity, self.set_id)
    }

    fn lookup_member<Tag>(
        &self,
        identity: RetainedSlotIdentity,
        expected_destination: RetainedMembershipDestination,
    ) -> Option<RetainedLookupMember<'_, Block>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        #[cfg(feature = "_u4c-collector")]
        record_lookup_stage(LookupStage::CheckTableBrand);
        if self.table.membership()
            != Some(RetainedMembership {
                set_id: self.set_id,
                destination: expected_destination,
                ordinal: 0,
            })
        {
            #[cfg(feature = "_u4c-collector")]
            record_lookup_rejected();
            return None;
        }

        #[cfg(feature = "_u4c-collector")]
        record_lookup_stage(LookupStage::CheckSlotSet);
        if identity.set_id != self.set_id {
            #[cfg(feature = "_u4c-collector")]
            record_lookup_rejected();
            return None;
        }

        #[cfg(feature = "_u4c-collector")]
        record_lookup_stage(LookupStage::CheckRange);
        let Some(member) = self.table.as_ref().members.get(identity.index) else {
            #[cfg(feature = "_u4c-collector")]
            record_lookup_rejected();
            return None;
        };

        #[cfg(feature = "_u4c-collector")]
        record_lookup_stage(LookupStage::LoadMember);
        Some(RetainedLookupMember {
            block: member.allocation.as_ref(),
            schema_tag_matches: identity.tag == TypeId::of::<Tag>() && member.tag == identity.tag,
        })
    }

    fn len(&self) -> usize {
        self.table.as_ref().members.len()
    }
}

/// Immutable canonical block table after the atomic publication handoff.
pub(crate) struct PublishedCanonicalAggregate<Block> {
    pub(super) inner: PublishedRetainedAggregate<Block>,
}

impl<Block> PublishedCanonicalAggregate<Block> {
    #[cfg(test)]
    pub fn get<Tag>(&self, slot: CanonicalRetainedSlot<Block, Tag>) -> Option<&Block>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.tag != TypeId::of::<Tag>() {
            return None;
        }
        self.inner.member(slot.identity)
    }

    pub(crate) fn lookup_member<Tag>(
        &self,
        slot: CanonicalRetainedSlot<Block, Tag>,
    ) -> Option<RetainedLookupMember<'_, Block>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inner
            .lookup_member::<Tag>(slot.identity, RetainedMembershipDestination::Canonical)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Immutable outcome-evidence block table after the atomic publication handoff.
pub(crate) struct PublishedOutcomeEvidenceAggregate<Block> {
    pub(super) inner: PublishedRetainedAggregate<Block>,
}

impl<Block> PublishedOutcomeEvidenceAggregate<Block> {
    #[cfg(test)]
    pub fn get<Tag>(&self, slot: OutcomeEvidenceRetainedSlot<Block, Tag>) -> Option<&Block>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if slot.identity.tag != TypeId::of::<Tag>() {
            return None;
        }
        self.inner.member(slot.identity)
    }

    pub(crate) fn lookup_member<Tag>(
        &self,
        slot: OutcomeEvidenceRetainedSlot<Block, Tag>,
    ) -> Option<RetainedLookupMember<'_, Block>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.inner.lookup_member::<Tag>(
            slot.identity,
            RetainedMembershipDestination::OutcomeEvidence,
        )
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub(super) struct QualifiedRetainedBlockTableStoragePlan<P, D, Block> {
    phase: P,
    attribution_dimension: D,
    total_blocks: usize,
    max_new_blocks: usize,
    member_capacity: CapacityPlan,
    charge: AdmissionCharge,
    block: PhantomData<fn() -> Block>,
}

pub(super) struct QualifiedRetainedBlockTablePlan<O, P, D, Destination, Block>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    pub(super) source: RetainedBlockTablePlan<P, D>,
    pub(super) batch: AttributionBatchPlan<O, P, D, Destination>,
    pub(super) storage: QualifiedRetainedBlockTableStoragePlan<P, D, Block>,
}

pub(super) fn try_qualify_retained_block_table_plan<O, P, D, Destination, Block>(
    account: &PhysicalStorageAccount,
    source: RetainedBlockTablePlan<P, D>,
    terminal_limits: RetainedBatchLimits,
) -> Result<QualifiedRetainedBlockTablePlan<O, P, D, Destination, Block>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    let required_attributions = source
        .max_new_blocks
        .checked_add(1)
        .ok_or(RetainedWriterError::InvalidPublicationPlan)?;
    if source.total_blocks == 0
        || source.max_new_blocks > source.total_blocks
        || u64::try_from(source.total_blocks).is_err()
        || required_attributions > source.writer.limits.attribution_slots
        || required_attributions > terminal_limits.attribution_slots
        || source.writer.limits.attribution_slots > terminal_limits.attribution_slots
        || source.writer.limits.commitment_slots > terminal_limits.commitment_slots
    {
        return Err(RetainedWriterError::InvalidPublicationPlan);
    }
    let batch = AttributionBatchPlan::try_new(
        source.writer.phase,
        source.writer.capacity_dimension,
        source.writer.limits.attribution_slots,
        source.writer.limits.commitment_slots,
    )
    .map_err(map_batch_error)?;
    let member_capacity =
        CapacityPlan::for_type::<RetainedBlockMember<Block>>(0, 0, source.total_blocks)
            .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let nested_bytes = member_capacity
        .envelope_delta_bytes()
        .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let allocation = RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
        .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let charge = account
        .checked_retained_table_charge::<Block>(allocation)
        .map_err(RetainedWriterError::Physical)?;
    Ok(QualifiedRetainedBlockTablePlan {
        source,
        batch,
        storage: QualifiedRetainedBlockTableStoragePlan {
            phase: source.writer.phase,
            attribution_dimension: source.writer.attribution_dimension,
            total_blocks: source.total_blocks,
            max_new_blocks: source.max_new_blocks,
            member_capacity,
            charge,
            block: PhantomData,
        },
    })
}
#[allow(clippy::too_many_arguments)]
pub(super) fn try_create_retained_block_table<O, P, D, Destination, Block>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    batch: &mut AttributionReservationBatch<O, P, D, Destination>,
    plan: QualifiedRetainedBlockTableStoragePlan<P, D, Block>,
    set_id: RetainedSetId,
    membership_destination: RetainedMembershipDestination,
) -> Result<RetainedTableAllocation<Block>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Destination: PublicationDestination,
    Block: Send + Sync + 'static,
{
    let QualifiedRetainedBlockTableStoragePlan {
        phase,
        attribution_dimension,
        total_blocks,
        max_new_blocks,
        member_capacity,
        charge,
        block: _,
    } = plan;
    let preflight = account
        .try_begin_account_preflight()
        .map_err(RetainedWriterError::AccountPoisoned)?;
    if !batch.has_attribution_slot() {
        return Err(RetainedWriterError::AttributionBatchFull);
    }
    let account_admission = preflight
        .admit_action()
        .map_err(map_account_action_admission_error)?;
    let admitted_share_work =
        try_admit_retained_share_work_before_physical(ledger, account, handle_share_work)?;
    let attribution_result = catch_unwind(AssertUnwindSafe(|| {
        ledger.reserve_operation_attribution::<Destination>(
            phase,
            attribution_dimension,
            charge.bytes,
        )
    }));
    let attribution = match attribution_result {
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            return Err(RetainedWriterError::AccountPoisoned(poisoned));
        }
        Ok(Ok(attribution)) => attribution,
        Ok(Err(cause)) => return Err(RetainedWriterError::Ledger(cause)),
    };
    let mut physical_admission = match account.try_admit_retained_after_action(
        account_admission,
        charge,
        admitted_share_work,
    ) {
        Ok(admission) => Some(admission),
        Err(cause) => {
            drop(attribution);
            return Err(RetainedWriterError::Physical(cause));
        }
    };
    let mut table = match account
        .try_retain_with_admitted_charge::<RetainedBlockTable<Block>, TableRetainedPolicy>(
            match physical_admission.as_mut() {
                Some(admission) => admission,
                None => unreachable_final_owner_state(),
            },
            || {
                let mut members = Vec::new();
                if members
                    .try_reserve_exact(member_capacity.allocator_additional())
                    .is_err()
                {
                    return Err(RetainedBuildFailureCode::new(1));
                }
                if member_capacity
                    .finish(members.len(), members.capacity())
                    .is_err()
                {
                    return Err(RetainedBuildFailureCode::new(2));
                }
                Ok(RetainedBlockTable {
                    qualified_capacity: members.capacity(),
                    members,
                    total_blocks,
                    max_new_blocks,
                    new_blocks: 0,
                })
            },
        ) {
        Ok(table) => table,
        Err(cause) => {
            if cause.charge_retained() {
                std::mem::forget(attribution);
            } else {
                drop(attribution);
            }
            return Err(RetainedWriterError::Physical(cause));
        }
    };
    let membership = RetainedMembership {
        set_id,
        destination: membership_destination,
        ordinal: 0,
    };
    if table.assign_membership(membership).is_err() {
        let outcome = drop_retained_allocation_before_attribution(table, account);
        if matches!(
            outcome,
            RetainedStorageDropOutcome::Clean | RetainedStorageDropOutcome::MissingFaultAnchor
        ) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        return Err(finish_pre_attribution_cleanup(
            attribution,
            outcome,
            RetainedWriterError::OwnershipUnverified,
        ));
    }
    if let Err(attribution) = batch.try_push_attribution(attribution) {
        let outcome = drop_retained_allocation_before_attribution(table, account);
        if matches!(
            outcome,
            RetainedStorageDropOutcome::Clean | RetainedStorageDropOutcome::MissingFaultAnchor
        ) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        return Err(finish_pre_attribution_cleanup(
            attribution,
            outcome,
            RetainedWriterError::OwnershipUnverified,
        ));
    }
    Ok(table)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn try_retain_block<O, P, D, Destination, Block, Tag>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    phase: P,
    attribution_dimension: D,
    batch: &mut AttributionReservationBatch<O, P, D, Destination>,
    table: &mut RetainedTableAllocation<Block>,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
    slot_limit_error: RetainedPublicationError<D>,
    plan: RetainedAllocationPlan,
    build: impl for<'a> FnOnce(
        BlockEmplacer<'a, Block>,
    ) -> Result<BlockPlacement<'a, Block, Tag>, RetainedBuildFailureCode>,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Destination: PublicationDestination,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    let preflight = account
        .try_begin_account_preflight()
        .map_err(RetainedWriterError::AccountPoisoned)
        .map_err(RetainedPublicationError::Writer)?;
    if !table.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount,
        ));
    }
    let (index, next_new_blocks) = {
        let table = table.as_ref();
        if table.members.len() >= table.total_blocks || table.new_blocks >= table.max_new_blocks {
            return Err(slot_limit_error);
        }
        if table.members.len() >= table.members.capacity() {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::BatchCapacity,
            ));
        }
        let next_new_blocks =
            table
                .new_blocks
                .checked_add(1)
                .ok_or(RetainedPublicationError::Writer(
                    RetainedWriterError::MembershipExhausted,
                ))?;
        (table.members.len(), next_new_blocks)
    };
    let ordinal = u64::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::MembershipExhausted,
        ))?;
    if !batch.has_attribution_slot() {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AttributionBatchFull,
        ));
    }
    let charge = account
        .checked_retained_charge::<Block>(plan)
        .map_err(|cause| RetainedPublicationError::Writer(RetainedWriterError::Physical(cause)))?;
    let account_admission = preflight
        .admit_action()
        .map_err(map_account_action_admission_error)
        .map_err(RetainedPublicationError::Writer)?;
    let Some(table) = table.table_mut_during_construction() else {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(poisoned),
        ));
    };
    let admitted_share_work =
        try_admit_retained_share_work_before_physical(ledger, account, handle_share_work)
            .map_err(RetainedPublicationError::Writer)?;
    let attribution_result = catch_unwind(AssertUnwindSafe(|| {
        ledger.reserve_operation_attribution::<Destination>(
            phase,
            attribution_dimension,
            charge.bytes,
        )
    }));
    let attribution = match attribution_result {
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ));
        }
        Ok(Ok(attribution)) => attribution,
        Ok(Err(cause)) => {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Ledger(cause),
            ));
        }
    };
    let mut physical_admission = match account.try_admit_retained_after_action(
        account_admission,
        charge,
        admitted_share_work,
    ) {
        Ok(admission) => Some(admission),
        Err(cause) => {
            drop(attribution);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Physical(cause),
            ));
        }
    };
    let mut allocation = match account.try_retain_with_admitted_handle(
        match physical_admission.as_mut() {
            Some(admission) => admission,
            None => unreachable_final_owner_state(),
        },
        plan,
        || {
            #[cfg(feature = "_u4c-collector")]
            record_block_begin(BlockKind::New);
            let block = build(BlockEmplacer {
                marker: PhantomData,
            })
            .map(BlockPlacement::into_block);
            #[cfg(feature = "_u4c-collector")]
            if block.is_ok() {
                record_block_seal(BlockKind::New);
            }
            block
        },
    ) {
        Ok(allocation) => {
            #[cfg(feature = "_u4c-collector")]
            record_payload_backing_allocation();
            allocation
        }
        Err(cause) => {
            if cause.charge_retained() {
                std::mem::forget(attribution);
            } else {
                drop(attribution);
            }
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Physical(cause),
            ));
        }
    };
    let membership = RetainedMembership {
        set_id,
        destination,
        ordinal,
    };
    if allocation.assign_membership(membership).is_err() {
        let outcome = drop_retained_allocation_before_attribution(allocation, account);
        if matches!(
            outcome,
            RetainedStorageDropOutcome::Clean | RetainedStorageDropOutcome::MissingFaultAnchor
        ) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        return Err(RetainedPublicationError::Writer(
            finish_pre_attribution_cleanup(
                attribution,
                outcome,
                RetainedWriterError::OwnershipUnverified,
            ),
        ));
    }
    if let Err(attribution) = batch.try_push_attribution(attribution) {
        let outcome = drop_retained_allocation_before_attribution(allocation, account);
        if matches!(
            outcome,
            RetainedStorageDropOutcome::Clean | RetainedStorageDropOutcome::MissingFaultAnchor
        ) {
            let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        }
        return Err(RetainedPublicationError::Writer(
            finish_pre_attribution_cleanup(
                attribution,
                outcome,
                RetainedWriterError::OwnershipUnverified,
            ),
        ));
    };
    table.members.push(RetainedBlockMember {
        allocation,
        tag: TypeId::of::<Tag>(),
        origin: RetainedBlockOrigin::New,
        reachable: AtomicBool::new(false),
    });
    #[cfg(feature = "_u4c-collector")]
    record_slot_publication(BlockKind::New);
    table.new_blocks = next_new_blocks;
    Ok(RetainedSlotIdentity {
        set_id,
        index,
        tag: TypeId::of::<Tag>(),
    })
}
#[allow(clippy::too_many_arguments)]
pub(super) fn try_share_retained_block<O, P, D, Block, Tag>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    table: &mut RetainedTableAllocation<Block>,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
    slot_limit_error: RetainedPublicationError<D>,
    source: &PublishedRetainedAggregate<Block>,
    source_identity: RetainedSlotIdentity,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    let preflight = account
        .try_begin_account_preflight()
        .map_err(RetainedWriterError::AccountPoisoned)
        .map_err(RetainedPublicationError::Writer)?;
    if !table.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount,
        ));
    }
    let table_value = table.as_ref();
    if table_value.members.len() >= table_value.total_blocks
        || table_value.members.len() >= table_value.members.capacity()
    {
        return Err(slot_limit_error);
    }
    let index = table_value.members.len();
    let ordinal = u64::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::MembershipExhausted,
        ))?;
    if source_identity.tag != TypeId::of::<Tag>() || source_identity.set_id != source.set_id {
        return Err(RetainedPublicationError::InvalidSourceSlot);
    }
    if !source.table.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount,
        ));
    }
    let source_allocation = source
        .table
        .as_ref()
        .member_allocation(source_identity, source.set_id)
        .ok_or(RetainedPublicationError::InvalidSourceSlot)?;
    if !source_allocation.belongs_to(account) {
        drop(preflight);
        let cause = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(cause),
        ));
    }
    let share_admission = preflight
        .admit_action()
        .map_err(map_account_action_admission_error)
        .map_err(RetainedPublicationError::Writer)?;
    let Some(table) = table.table_mut_during_construction() else {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(poisoned),
        ));
    };
    match catch_unwind(AssertUnwindSafe(|| {
        try_share_retained_block_after_admission::<O, P, D, Block, Tag>(
            ledger,
            share_admission,
            handle_share_work,
            table,
            set_id,
            destination,
            index,
            ordinal,
            source_allocation,
        )
    })) {
        Ok(result) => result,
        Err(payload) => {
            let cause = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(cause),
            ))
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn try_share_retained_block_after_admission<O, P, D, Block, Tag>(
    ledger: &Ledger<O, P, D>,
    share_admission: PendingAccountActionAdmission<'_>,
    handle_share_work: RetainedHandleSharePlan<D>,
    table: &mut RetainedBlockTable<Block>,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
    index: usize,
    ordinal: u64,
    source_allocation: &RetainedAllocation<Block>,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    let account = share_admission.account;
    let admitted_share_work = try_admit_retained_share_work(ledger, handle_share_work)
        .map_err(RetainedPublicationError::Writer)?;
    let mut allocation =
        share_admission.run(|_| admitted_share_work.consume_allocation(source_allocation));
    let membership = RetainedMembership {
        set_id,
        destination,
        ordinal,
    };
    if allocation.assign_membership(membership).is_err() {
        let outcome = allocation.release_storage();
        return Err(RetainedPublicationError::Writer(
            map_inherited_cleanup_error(account, outcome, RetainedWriterError::OwnershipUnverified),
        ));
    }
    #[cfg(feature = "_u4c-collector")]
    {
        record_block_begin(BlockKind::Inherited);
        record_block_seal(BlockKind::Inherited);
    }
    table.members.push(RetainedBlockMember {
        allocation,
        tag: TypeId::of::<Tag>(),
        origin: RetainedBlockOrigin::Inherited,
        reachable: AtomicBool::new(false),
    });
    #[cfg(feature = "_u4c-collector")]
    record_slot_publication(BlockKind::Inherited);
    Ok(RetainedSlotIdentity {
        set_id,
        index,
        tag: TypeId::of::<Tag>(),
    })
}

pub(super) fn retained_block_table_is_exact<O, P, D, Destination, Block>(
    table: &RetainedTableAllocation<Block>,
    batch: &AttributionReservationBatch<O, P, D, Destination>,
    account: &PhysicalStorageAccount,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
) -> bool
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Destination: PublicationDestination,
{
    if !table.belongs_to(account)
        || table.membership()
            != Some(RetainedMembership {
                set_id,
                destination,
                ordinal: 0,
            })
    {
        return false;
    }
    let mut attributed_bytes = table.charged_bytes();
    let table = table.as_ref();
    if table.members.capacity() != table.qualified_capacity
        || table.members.len() > table.total_blocks
        || table.new_blocks > table.max_new_blocks
        || table.members.len() > table.members.capacity()
    {
        return false;
    }
    let mut new_blocks = 0_usize;
    for (index, member) in table.members.iter().enumerate() {
        let Some(ordinal) = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
        else {
            return false;
        };
        if !member.allocation.belongs_to(account)
            || member.allocation.membership()
                != Some(RetainedMembership {
                    set_id,
                    destination,
                    ordinal,
                })
        {
            return false;
        }
        if member.origin == RetainedBlockOrigin::New {
            let Some(next_new_blocks) = new_blocks.checked_add(1) else {
                return false;
            };
            let Some(next_bytes) = attributed_bytes.checked_add(member.allocation.charged_bytes())
            else {
                return false;
            };
            new_blocks = next_new_blocks;
            attributed_bytes = next_bytes;
        }
    }
    let Some(attributed) = new_blocks.checked_add(1) else {
        return false;
    };
    new_blocks == table.new_blocks
        && attributed == batch.attribution_len()
        && batch.attributed_bytes() == Some(attributed_bytes)
}
