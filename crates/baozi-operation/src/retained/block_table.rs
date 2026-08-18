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
    finish_pre_attribution_cleanup, join_pre_attribution_cleanup_cause,
    map_inherited_cleanup_error,
};
use super::{
    PendingAccountActionAdmission, PhysicalStorageAccount, RetainedBatchLimits,
    RetainedHandleSharePlan, RetainedPublicationError, RetainedShareAdmissionError,
    RetainedWorkControl, RetainedWriterError, RetainedWriterPlan,
    map_account_action_admission_error, map_batch_error, try_admit_retained_share_work,
    try_admit_retained_share_work_before_physical,
};
use crate::control::{ControlCause, discard_untrusted_panic_payload};
use crate::ledger::{
    AttributionBatchPlan, AttributionReservationBatch, CanonicalOutput, Ledger, LedgerCause,
    PublicationDestination,
};
use crate::{CapacityPlan, OperationOwner};
use std::any::TypeId;
use std::fmt;
use std::marker::PhantomData;
use std::num::{NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) type InvariantRetainedBrand<Block, Tag> = PhantomData<fn(Block, Tag) -> (Block, Tag)>;

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
    canonical_source_mapping_capacity: usize,
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
            canonical_source_mapping_capacity: 0,
        }
    }

    /// Preallocates the dense source-index map required by canonical inheritance and rewrites.
    pub(crate) const fn with_canonical_source_mapping_capacity(mut self, capacity: usize) -> Self {
        self.canonical_source_mapping_capacity = capacity;
        self
    }

    pub(in crate::retained) const fn total_blocks(self) -> usize {
        self.total_blocks
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
    pub(super) marker: InvariantRetainedBrand<Block, Tag>,
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
    pub(super) marker: InvariantRetainedBrand<Block, Tag>,
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
    tag_marker: PhantomData<fn(Tag) -> Tag>,
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
    visit_epoch: AtomicU64,
}

impl<Block> RetainedBlockMember<Block> {
    fn first_visit_in(&self, epoch: NonZeroU64) -> bool {
        self.visit_epoch.swap(epoch.get(), Ordering::AcqRel) != epoch.get()
    }

    #[cfg(feature = "_u4-activation")]
    fn completed_in(&self, epoch: NonZeroU64) -> bool {
        self.visit_epoch.load(Ordering::Acquire) == epoch.get()
    }

    #[cfg(feature = "_u4-activation")]
    fn try_complete_in(&self, epoch: NonZeroU64) -> bool {
        let previous = self.visit_epoch.load(Ordering::Acquire);
        previous != epoch.get()
            && self
                .visit_epoch
                .compare_exchange(previous, epoch.get(), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetainedBlockOrigin {
    New,
    Inherited { source: RetainedSlotIdentity },
    Rewritten { source: RetainedSlotIdentity },
}

impl RetainedBlockOrigin {
    const fn owns_new_allocation(self) -> bool {
        matches!(self, Self::New | Self::Rewritten { .. })
    }

    const fn source_identity(self) -> Option<RetainedSlotIdentity> {
        match self {
            Self::New => None,
            Self::Inherited { source } | Self::Rewritten { source } => Some(source),
        }
    }
}

struct CanonicalSourceMapping {
    entries: Vec<Option<NonZeroUsize>>,
    qualified_capacity: usize,
    source_set: Option<RetainedSetId>,
    mapped_entries: usize,
    #[cfg(test)]
    probes: AtomicU64,
}

enum CanonicalSourceBindingStateError {
    Unavailable,
    Unbound,
    Mismatch,
    Corrupt,
}

impl CanonicalSourceMapping {
    fn shape_is_exact(&self) -> bool {
        self.entries.capacity() == self.qualified_capacity
            && self.entries.len() <= self.qualified_capacity
            && self.mapped_entries <= self.entries.len()
    }

    fn lookup(&self, source_identity: RetainedSlotIdentity) -> Result<Option<usize>, ()> {
        #[cfg(test)]
        self.probes.fetch_add(1, Ordering::Relaxed);
        if self.source_set != Some(source_identity.set_id) || !self.shape_is_exact() {
            return Err(());
        }
        let Some(mapped_ordinal) = self.entries.get(source_identity.index).copied() else {
            return Err(());
        };
        mapped_ordinal
            .map(|ordinal| ordinal.get().checked_sub(1).ok_or(()))
            .transpose()
    }

    fn record(
        &mut self,
        source_identity: RetainedSlotIdentity,
        destination_index: usize,
    ) -> Result<(), ()> {
        if self.source_set != Some(source_identity.set_id) || !self.shape_is_exact() {
            return Err(());
        }
        let next_mapped_entries = self.mapped_entries.checked_add(1).ok_or(())?;
        if next_mapped_entries > self.entries.len() {
            return Err(());
        }
        let entry = self.entries.get_mut(source_identity.index).ok_or(())?;
        if entry.is_some() {
            return Err(());
        }
        let ordinal = destination_index
            .checked_add(1)
            .and_then(NonZeroUsize::new)
            .ok_or(())?;
        *entry = Some(ordinal);
        self.mapped_entries = next_mapped_entries;
        Ok(())
    }

    fn clear(&mut self, source_identity: RetainedSlotIdentity) -> Result<(), ()> {
        if self.source_set != Some(source_identity.set_id) || !self.shape_is_exact() {
            return Err(());
        }
        let next_mapped_entries = self.mapped_entries.checked_sub(1).ok_or(())?;
        let entry = self.entries.get_mut(source_identity.index).ok_or(())?;
        if entry.take().is_none() {
            return Err(());
        }
        self.mapped_entries = next_mapped_entries;
        Ok(())
    }

    #[cfg(test)]
    fn take_probe_count(&self) -> u64 {
        self.probes.swap(0, Ordering::Relaxed)
    }
}

pub(super) struct RetainedBlockTable<Block> {
    pub(super) members: Vec<RetainedBlockMember<Block>>,
    canonical_source_mapping: Option<CanonicalSourceMapping>,
    root_membership: RetainedMembership,
    total_blocks: usize,
    max_new_blocks: usize,
    // Updated only after a complete append, so any partial mutation fails exactness closed.
    committed_members: usize,
    committed_new_blocks: usize,
    committed_derived_members: usize,
    committed_new_block_bytes: u64,
    qualified_capacity: usize,
}

#[derive(Debug, Clone, Copy)]
enum RetainedBlockPopExpectation {
    New { tag: TypeId },
    Inherited,
}

impl RetainedBlockPopExpectation {
    fn matches<Block>(self, member: &RetainedBlockMember<Block>) -> bool {
        match self {
            Self::New { tag } => member.tag == tag && member.origin == RetainedBlockOrigin::New,
            Self::Inherited => matches!(member.origin, RetainedBlockOrigin::Inherited { .. }),
        }
    }
}

impl<Block> RetainedBlockTable<Block> {
    pub(super) fn member_count(&self) -> usize {
        self.committed_members
    }

    pub(super) fn new_block_count(&self) -> usize {
        self.committed_new_blocks
    }

    pub(super) fn member(
        &self,
        identity: RetainedSlotIdentity,
        set_id: RetainedSetId,
    ) -> Option<&Block> {
        if identity.set_id != set_id || identity.index >= self.committed_members {
            return None;
        }
        self.members
            .get(identity.index)
            .and_then(|member| (member.tag == identity.tag).then(|| member.allocation.as_ref()))
    }

    fn committed_summary_is_bounded(&self) -> bool {
        self.root_membership.ordinal == 0
            && self.members.capacity() == self.qualified_capacity
            && self.members.len() == self.committed_members
            && self.committed_members <= self.total_blocks
            && self.committed_members <= self.members.capacity()
            && self.committed_new_blocks <= self.max_new_blocks
            && self.committed_new_blocks <= self.committed_members
            && self.committed_derived_members <= self.committed_members
            && self
                .committed_new_blocks
                .checked_add(self.committed_derived_members)
                .is_some_and(|covered_members| covered_members >= self.committed_members)
    }

    fn try_push_committed_member(
        &mut self,
        account: &PhysicalStorageAccount,
        set_id: RetainedSetId,
        destination: RetainedMembershipDestination,
        member: RetainedBlockMember<Block>,
    ) -> Result<RetainedSlotIdentity, ()> {
        if !self.committed_summary_is_bounded()
            || self.root_membership
                != (RetainedMembership {
                    set_id,
                    destination,
                    ordinal: 0,
                })
            || (destination == RetainedMembershipDestination::Canonical
                && !canonical_source_mapping_is_exact(self, set_id))
            || (destination != RetainedMembershipDestination::Canonical
                && self.canonical_source_mapping.is_some())
        {
            return Err(());
        }

        let index = self.committed_members;
        let ordinal = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(())?;
        if member.allocation.membership()
            != Some(RetainedMembership {
                set_id,
                destination,
                ordinal,
            })
            || !member.allocation.belongs_to(account)
        {
            return Err(());
        }

        let owns_new_allocation = member.origin.owns_new_allocation();
        let source_identity = member.origin.source_identity();
        let new_block_increment = if owns_new_allocation { 1 } else { 0 };
        let derived_member_increment = if source_identity.is_some() { 1 } else { 0 };
        let next_committed_members = self.committed_members.checked_add(1).ok_or(())?;
        let next_committed_new_blocks = self
            .committed_new_blocks
            .checked_add(new_block_increment)
            .ok_or(())?;
        let next_committed_derived_members = self
            .committed_derived_members
            .checked_add(derived_member_increment)
            .ok_or(())?;
        let next_committed_new_block_bytes = if owns_new_allocation {
            self.committed_new_block_bytes
                .checked_add(member.allocation.charged_bytes())
                .ok_or(())?
        } else {
            self.committed_new_block_bytes
        };
        if next_committed_members > self.total_blocks
            || next_committed_members > self.members.capacity()
            || next_committed_new_blocks > self.max_new_blocks
        {
            return Err(());
        }

        let tag = member.tag;
        self.members.push(member);
        if destination == RetainedMembershipDestination::Canonical
            && let Some(source_identity) = source_identity
        {
            self.record_canonical_source_mapping(account, set_id, source_identity, index)?;
        }

        self.committed_members = next_committed_members;
        self.committed_new_blocks = next_committed_new_blocks;
        self.committed_derived_members = next_committed_derived_members;
        self.committed_new_block_bytes = next_committed_new_block_bytes;
        Ok(RetainedSlotIdentity { set_id, index, tag })
    }

    fn try_pop_committed_member(
        &mut self,
        expectation: RetainedBlockPopExpectation,
    ) -> Result<RetainedBlockMember<Block>, ()> {
        if !self.committed_summary_is_bounded() {
            return Err(());
        }
        let member = self.members.last().ok_or(())?;
        if !expectation.matches(member) {
            return Err(());
        }

        let owns_new_allocation = member.origin.owns_new_allocation();
        let has_source = member.origin.source_identity().is_some();
        let new_block_decrement = if owns_new_allocation { 1 } else { 0 };
        let derived_member_decrement = if has_source { 1 } else { 0 };
        let next_committed_members = self.committed_members.checked_sub(1).ok_or(())?;
        let next_committed_new_blocks = self
            .committed_new_blocks
            .checked_sub(new_block_decrement)
            .ok_or(())?;
        let next_committed_derived_members = self
            .committed_derived_members
            .checked_sub(derived_member_decrement)
            .ok_or(())?;
        let next_committed_new_block_bytes = if owns_new_allocation {
            self.committed_new_block_bytes
                .checked_sub(member.allocation.charged_bytes())
                .ok_or(())?
        } else {
            self.committed_new_block_bytes
        };

        let member = self.members.pop().ok_or(())?;
        self.committed_members = next_committed_members;
        self.committed_new_blocks = next_committed_new_blocks;
        self.committed_derived_members = next_committed_derived_members;
        self.committed_new_block_bytes = next_committed_new_block_bytes;
        Ok(member)
    }

    fn validate_canonical_source_binding(
        &self,
        source_set: RetainedSetId,
        source_len: usize,
    ) -> Result<(), CanonicalSourceBindingStateError> {
        let mapping = self
            .canonical_source_mapping
            .as_ref()
            .ok_or(CanonicalSourceBindingStateError::Unavailable)?;
        if !self.committed_summary_is_bounded()
            || self.root_membership.destination != RetainedMembershipDestination::Canonical
            || self.root_membership.set_id == source_set
            || !mapping.shape_is_exact()
            || mapping.mapped_entries != self.committed_derived_members
        {
            return Err(CanonicalSourceBindingStateError::Corrupt);
        }
        match mapping.source_set {
            None if mapping.entries.is_empty() => Err(CanonicalSourceBindingStateError::Unbound),
            None => Err(CanonicalSourceBindingStateError::Corrupt),
            Some(bound_set)
                if bound_set == source_set
                    && mapping.entries.len() == source_len
                    && source_len != 0 =>
            {
                Ok(())
            }
            Some(bound_set) if bound_set != source_set => {
                Err(CanonicalSourceBindingStateError::Mismatch)
            }
            Some(_) => Err(CanonicalSourceBindingStateError::Corrupt),
        }
    }

    fn require_canonical_source_binding<D>(
        &self,
        account: &PhysicalStorageAccount,
        source_set: RetainedSetId,
        source_len: usize,
    ) -> Result<(), RetainedPublicationError<D>> {
        match self.validate_canonical_source_binding(source_set, source_len) {
            Ok(()) => Ok(()),
            Err(CanonicalSourceBindingStateError::Unavailable) => Err(
                RetainedPublicationError::Writer(RetainedWriterError::InvalidPublicationPlan),
            ),
            Err(
                CanonicalSourceBindingStateError::Unbound
                | CanonicalSourceBindingStateError::Mismatch,
            ) => Err(RetainedPublicationError::InvalidSourceSlot),
            Err(CanonicalSourceBindingStateError::Corrupt) => {
                let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                Err(RetainedPublicationError::Writer(
                    RetainedWriterError::AccountPoisoned(poisoned),
                ))
            }
        }
    }

    fn canonical_source_mapping(
        &self,
        account: &PhysicalStorageAccount,
        destination_set: RetainedSetId,
        source_identity: RetainedSlotIdentity,
    ) -> Result<Option<(RetainedSlotIdentity, RetainedBlockOrigin)>, ()> {
        if self.root_membership
            != (RetainedMembership {
                set_id: destination_set,
                destination: RetainedMembershipDestination::Canonical,
                ordinal: 0,
            })
            || destination_set == source_identity.set_id
            || !self.committed_summary_is_bounded()
            || self
                .canonical_source_mapping
                .as_ref()
                .is_none_or(|mapping| mapping.mapped_entries != self.committed_derived_members)
        {
            return Err(());
        }
        let Some(index) = self
            .canonical_source_mapping
            .as_ref()
            .ok_or(())?
            .lookup(source_identity)?
        else {
            return Ok(None);
        };
        let member = self.members.get(index).ok_or(())?;
        let ordinal = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(())?;
        if member.tag != source_identity.tag
            || member.origin.source_identity() != Some(source_identity)
            || !member.allocation.belongs_to(account)
            || member.allocation.membership()
                != Some(RetainedMembership {
                    set_id: destination_set,
                    destination: RetainedMembershipDestination::Canonical,
                    ordinal,
                })
        {
            return Err(());
        }
        Ok(Some((
            RetainedSlotIdentity {
                set_id: destination_set,
                index,
                tag: source_identity.tag,
            },
            member.origin,
        )))
    }

    fn record_canonical_source_mapping(
        &mut self,
        account: &PhysicalStorageAccount,
        destination_set: RetainedSetId,
        source_identity: RetainedSlotIdentity,
        destination_index: usize,
    ) -> Result<(), ()> {
        if self.root_membership
            != (RetainedMembership {
                set_id: destination_set,
                destination: RetainedMembershipDestination::Canonical,
                ordinal: 0,
            })
            || self
                .canonical_source_mapping
                .as_ref()
                .is_none_or(|mapping| mapping.mapped_entries != self.committed_derived_members)
        {
            return Err(());
        }
        let member = self.members.get(destination_index).ok_or(())?;
        let ordinal = u64::try_from(destination_index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(())?;
        if member.tag != source_identity.tag
            || member.origin.source_identity() != Some(source_identity)
            || !member.allocation.belongs_to(account)
            || member.allocation.membership()
                != Some(RetainedMembership {
                    set_id: destination_set,
                    destination: RetainedMembershipDestination::Canonical,
                    ordinal,
                })
        {
            return Err(());
        }
        self.canonical_source_mapping
            .as_mut()
            .ok_or(())?
            .record(source_identity, destination_index)
    }

    fn clear_canonical_source_mapping(
        &mut self,
        source_identity: RetainedSlotIdentity,
    ) -> Result<(), ()> {
        let expected_mapped_entries = self.committed_derived_members.checked_add(1).ok_or(())?;
        if self
            .canonical_source_mapping
            .as_ref()
            .is_none_or(|mapping| mapping.mapped_entries != expected_mapped_entries)
        {
            return Err(());
        }
        self.canonical_source_mapping
            .as_mut()
            .ok_or(())?
            .clear(source_identity)
    }

    #[cfg(test)]
    pub(super) fn take_canonical_source_mapping_probe_count(&self) -> u64 {
        self.canonical_source_mapping
            .as_ref()
            .map_or(0, CanonicalSourceMapping::take_probe_count)
    }
}

/// Non-authoritative proof that one canonical batch can extend a table without structural growth.
///
/// The proof owns no table, storage, mutation, or attribution capability. Its private expected
/// state is consumed only after the ordinary single-block rail has retained every item.
pub(in crate::retained) struct QualifiedCanonicalRetainBatch<Block, Tag> {
    set_id: RetainedSetId,
    first_index: usize,
    len: NonZeroUsize,
    initial_new_blocks: usize,
    initial_derived_members: usize,
    initial_new_block_bytes: u64,
    initial_attributions: usize,
    initial_attributed_bytes: u64,
    final_members: usize,
    final_new_blocks: usize,
    final_derived_members: usize,
    final_new_block_bytes: u64,
    final_attributions: usize,
    final_attributed_bytes: u64,
    marker: InvariantRetainedBrand<Block, Tag>,
}

/// Completed structural proof consumed by the non-owning range constructor.
pub(in crate::retained) struct CompletedCanonicalRetainBatch<Block, Tag> {
    first: RetainedSlotIdentity,
    len: NonZeroUsize,
    marker: InvariantRetainedBrand<Block, Tag>,
}

impl<Block, Tag> CompletedCanonicalRetainBatch<Block, Tag> {
    pub(in crate::retained) fn into_parts(self) -> (RetainedSlotIdentity, NonZeroUsize) {
        (self.first, self.len)
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::retained) fn try_qualify_canonical_retain_batch<O, P, D, Block, Tag>(
    account: &PhysicalStorageAccount,
    table: &RetainedTableAllocation<Block>,
    batch: &AttributionReservationBatch<O, P, D, CanonicalOutput>,
    table_plan: RetainedBlockTablePlan<P, D>,
    set_id: RetainedSetId,
    count: NonZeroUsize,
    aggregate_attributed_bytes: u64,
    slot_limit_error: RetainedPublicationError<D>,
) -> Result<QualifiedCanonicalRetainBatch<Block, Tag>, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    if !retained_block_table_is_exact(
        table,
        batch,
        account,
        set_id,
        RetainedMembershipDestination::Canonical,
    ) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::OwnershipUnverified,
        ));
    }

    let table_value = table.as_ref();
    if table_plan.total_blocks != table_value.total_blocks
        || table_plan.max_new_blocks != table_value.max_new_blocks
    {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::OwnershipUnverified,
        ));
    }
    let final_members = table_value
        .committed_members
        .checked_add(count.get())
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::MembershipExhausted,
        ))?;
    let final_new_blocks = table_value
        .committed_new_blocks
        .checked_add(count.get())
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::MembershipExhausted,
        ))?;
    let final_new_block_bytes = table_value
        .committed_new_block_bytes
        .checked_add(aggregate_attributed_bytes)
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::OwnershipUnverified,
        ))?;
    if final_members > table_value.total_blocks || final_new_blocks > table_value.max_new_blocks {
        return Err(slot_limit_error);
    }
    if final_members > table_value.members.capacity()
        || final_members > table_value.qualified_capacity
    {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::BatchCapacity,
        ));
    }
    if u64::try_from(final_members).is_err() {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::MembershipExhausted,
        ));
    }

    let final_attributions = batch.attribution_len().checked_add(count.get()).ok_or(
        RetainedPublicationError::Writer(RetainedWriterError::MembershipExhausted),
    )?;
    if final_attributions > table_plan.writer.limits.attribution_slots {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AttributionBatchFull,
        ));
    }
    let final_attributed_bytes = batch
        .attributed_bytes()
        .and_then(|bytes| bytes.checked_add(aggregate_attributed_bytes))
        .ok_or(RetainedPublicationError::Writer(
            RetainedWriterError::OwnershipUnverified,
        ))?;

    Ok(QualifiedCanonicalRetainBatch {
        set_id,
        first_index: table_value.committed_members,
        len: count,
        initial_new_blocks: table_value.committed_new_blocks,
        initial_derived_members: table_value.committed_derived_members,
        initial_new_block_bytes: table_value.committed_new_block_bytes,
        initial_attributions: batch.attribution_len(),
        initial_attributed_bytes: batch.attributed_bytes().ok_or(
            RetainedPublicationError::Writer(RetainedWriterError::OwnershipUnverified),
        )?,
        final_members,
        final_new_blocks,
        final_derived_members: table_value.committed_derived_members,
        final_new_block_bytes,
        final_attributions,
        final_attributed_bytes,
        marker: PhantomData,
    })
}

impl<Block, Tag> QualifiedCanonicalRetainBatch<Block, Tag>
where
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    pub(in crate::retained) const fn len(&self) -> usize {
        self.len.get()
    }

    /// Rolls back every completed member in this batch prefix.
    ///
    /// Each member is removed newest-first, its retained storage is released, and only then is the
    /// matching attribution dropped. A verified cleanup restores the exact pre-batch table and
    /// ledger shape. Any ownership or release uncertainty is returned as containment evidence.
    pub(in crate::retained) fn try_rollback_prefix<O, P, D>(
        &self,
        account: &PhysicalStorageAccount,
        table: &mut RetainedTableAllocation<Block>,
        batch: &mut AttributionReservationBatch<O, P, D, CanonicalOutput>,
    ) -> Result<(), RetainedPublicationError<D>>
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord,
    {
        if !retained_block_table_is_exact(
            table,
            batch,
            account,
            self.set_id,
            RetainedMembershipDestination::Canonical,
        ) {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::OwnershipUnverified,
            ));
        }

        let table_value = table.as_ref();
        let completed = table_value
            .committed_members
            .checked_sub(self.first_index)
            .ok_or(RetainedPublicationError::Writer(
                RetainedWriterError::OwnershipUnverified,
            ))?;
        let expected_new_blocks = self.initial_new_blocks.checked_add(completed).ok_or(
            RetainedPublicationError::Writer(RetainedWriterError::MembershipExhausted),
        )?;
        let expected_attributions = self.initial_attributions.checked_add(completed).ok_or(
            RetainedPublicationError::Writer(RetainedWriterError::MembershipExhausted),
        )?;
        if completed > self.len.get()
            || table_value.committed_new_blocks != expected_new_blocks
            || table_value.committed_derived_members != self.initial_derived_members
            || batch.attribution_len() != expected_attributions
        {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::OwnershipUnverified,
            ));
        }

        let Some(table_value) = table.table_mut_during_construction() else {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ));
        };
        while table_value.committed_members > self.first_index {
            let member =
                match table_value.try_pop_committed_member(RetainedBlockPopExpectation::New {
                    tag: TypeId::of::<Tag>(),
                }) {
                    Ok(member) => member,
                    Err(()) => {
                        let poisoned =
                            account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                        return Err(RetainedPublicationError::Writer(
                            RetainedWriterError::AccountPoisoned(poisoned),
                        ));
                    }
                };
            if member.origin != RetainedBlockOrigin::New {
                let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                std::mem::forget(member);
                return Err(RetainedPublicationError::Writer(
                    RetainedWriterError::AccountPoisoned(poisoned),
                ));
            }
            let outcome = drop_retained_allocation_before_attribution(member.allocation, account);
            if !batch.finish_last_attribution_after_storage_rollback(
                outcome == RetainedStorageDropOutcome::Clean,
            ) {
                let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                if outcome == RetainedStorageDropOutcome::Clean {
                    return Err(RetainedPublicationError::Writer(
                        RetainedWriterError::AccountPoisoned(poisoned),
                    ));
                }
                return Err(RetainedPublicationError::Writer(
                    join_pre_attribution_cleanup_cause(
                        outcome,
                        RetainedWriterError::AccountPoisoned(poisoned),
                    ),
                ));
            }
            if outcome != RetainedStorageDropOutcome::Clean {
                return Err(RetainedPublicationError::Writer(
                    join_pre_attribution_cleanup_cause(
                        outcome,
                        RetainedWriterError::OwnershipUnverified,
                    ),
                ));
            }
        }

        if table_value.committed_members != self.first_index
            || table_value.committed_new_blocks != self.initial_new_blocks
            || table_value.committed_derived_members != self.initial_derived_members
            || table_value.committed_new_block_bytes != self.initial_new_block_bytes
            || batch.attribution_len() != self.initial_attributions
            || batch.attributed_bytes() != Some(self.initial_attributed_bytes)
        {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ));
        }
        Ok(())
    }

    pub(in crate::retained) fn try_complete<O, P, D>(
        self,
        account: &PhysicalStorageAccount,
        table: &RetainedTableAllocation<Block>,
        batch: &AttributionReservationBatch<O, P, D, CanonicalOutput>,
    ) -> Result<CompletedCanonicalRetainBatch<Block, Tag>, RetainedPublicationError<D>>
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord,
    {
        let table_value = table.as_ref();
        let exact = retained_block_table_is_exact(
            table,
            batch,
            account,
            self.set_id,
            RetainedMembershipDestination::Canonical,
        ) && table_value.committed_members == self.final_members
            && table_value.committed_new_blocks == self.final_new_blocks
            && table_value.committed_derived_members == self.final_derived_members
            && table_value.committed_new_block_bytes == self.final_new_block_bytes
            && batch.attribution_len() == self.final_attributions
            && batch.attributed_bytes() == Some(self.final_attributed_bytes);
        if !exact {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::OwnershipUnverified,
            ));
        }

        let expected_tag = TypeId::of::<Tag>();
        Ok(CompletedCanonicalRetainBatch {
            first: RetainedSlotIdentity {
                set_id: self.set_id,
                index: self.first_index,
                tag: expected_tag,
            },
            len: self.len,
            marker: PhantomData,
        })
    }
}

mod published_table_capability {
    use super::{
        AccountPoisonReason, AssertUnwindSafe, Ledger, OperationOwner, PhysicalStorageAccount,
        RetainedAllocation, RetainedBlockMember, RetainedBlockOrigin, RetainedHandleSharePlan,
        RetainedLookupMember, RetainedMembership, RetainedMembershipDestination,
        RetainedPublicationError, RetainedSetId, RetainedSlotIdentity, RetainedTableAllocation,
        RetainedWriterError, TypeId, canonical_source_mapping_is_exact, catch_unwind,
        discard_untrusted_panic_payload, map_account_action_admission_error,
    };
    use std::fmt;

    struct PublishedRetainedTable<Block> {
        table: RetainedTableAllocation<Block>,
    }

    struct PublishedLookupCapability<'a, Block> {
        table_membership: Option<RetainedMembership>,
        members: &'a [RetainedBlockMember<Block>],
    }

    struct PublishedSourceSharingCapability<'a, Block> {
        table: &'a RetainedTableAllocation<Block>,
    }

    struct ProbedPublishedMember<'a, Block> {
        block: &'a Block,
        schema_tag: TypeId,
    }

    pub(super) struct PublishedInheritanceSourceMember<'a, Block> {
        pub(super) allocation: &'a RetainedAllocation<Block>,
        pub(super) identity: RetainedSlotIdentity,
    }

    impl<'a, Block> PublishedLookupCapability<'a, Block> {
        const fn table_membership(&self) -> Option<RetainedMembership> {
            self.table_membership
        }

        fn probe_member(&self, index: usize) -> Option<ProbedPublishedMember<'a, Block>> {
            let member = self.members.get(index)?;
            Some(ProbedPublishedMember {
                block: member.allocation.as_ref(),
                schema_tag: member.tag,
            })
        }
    }

    impl<'a, Block> PublishedSourceSharingCapability<'a, Block> {
        fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
            self.table.belongs_to(account)
        }

        fn source_allocation(
            &self,
            identity: RetainedSlotIdentity,
            set_id: RetainedSetId,
        ) -> Option<&'a RetainedAllocation<Block>> {
            #[cfg(test)]
            super::super::record_test_member_lookup();
            if identity.set_id != set_id {
                return None;
            }
            self.table
                .as_ref()
                .members
                .get(identity.index)
                .and_then(|member| (member.tag == identity.tag).then_some(&member.allocation))
        }
    }

    impl<Block> PublishedRetainedTable<Block> {
        fn new(table: RetainedTableAllocation<Block>) -> Self {
            Self { table }
        }

        fn lookup_capability(&self) -> PublishedLookupCapability<'_, Block> {
            let table_membership = self.table.membership();
            let members = self.table.as_ref().members.as_slice();
            PublishedLookupCapability {
                table_membership,
                members,
            }
        }

        fn lookup_member<Tag>(
            &self,
            set_id: RetainedSetId,
            identity: RetainedSlotIdentity,
            expected_destination: RetainedMembershipDestination,
        ) -> Option<RetainedLookupMember<'_, Block>>
        where
            Tag: Copy + Send + Sync + 'static,
        {
            typed_lookup::lookup_member::<Block, Tag>(
                self.lookup_capability(),
                set_id,
                identity,
                expected_destination,
            )
        }

        #[cfg(test)]
        fn member(&self, identity: RetainedSlotIdentity, set_id: RetainedSetId) -> Option<&Block> {
            self.table.as_ref().member(identity, set_id)
        }

        fn source_sharing_capability(&self) -> PublishedSourceSharingCapability<'_, Block> {
            PublishedSourceSharingCapability { table: &self.table }
        }

        fn len(&self) -> usize {
            self.table.as_ref().member_count()
        }

        #[cfg(test)]
        fn share_call_count(&self) -> usize {
            self.table.share_call_count()
        }

        #[cfg(test)]
        fn allocation_identity(&self) -> *const () {
            self.table.allocation_identity().cast()
        }
    }

    mod typed_lookup {
        use super::{
            PublishedLookupCapability, RetainedLookupMember, RetainedMembership,
            RetainedMembershipDestination, RetainedSetId, RetainedSlotIdentity, TypeId,
        };

        pub(super) fn lookup_member<'a, Block, Tag>(
            table: PublishedLookupCapability<'a, Block>,
            set_id: RetainedSetId,
            identity: RetainedSlotIdentity,
            expected_destination: RetainedMembershipDestination,
        ) -> Option<RetainedLookupMember<'a, Block>>
        where
            Tag: Copy + Send + Sync + 'static,
        {
            if table.table_membership()
                != Some(RetainedMembership {
                    set_id,
                    destination: expected_destination,
                    ordinal: 0,
                })
            {
                return None;
            }

            if identity.set_id != set_id {
                return None;
            }

            let member = table.probe_member(identity.index)?;

            Some(RetainedLookupMember {
                block: member.block,
                schema_tag_matches: identity.tag == TypeId::of::<Tag>()
                    && member.schema_tag == identity.tag,
            })
        }
    }

    pub(in crate::retained) struct PublishedRetainedAggregate<Block> {
        table: PublishedRetainedTable<Block>,
        set_id: RetainedSetId,
    }

    impl<Block> PublishedRetainedAggregate<Block> {
        pub(in crate::retained) fn new(
            table: RetainedTableAllocation<Block>,
            set_id: RetainedSetId,
        ) -> Self {
            Self {
                table: PublishedRetainedTable::new(table),
                set_id,
            }
        }

        #[cfg(test)]
        pub(in crate::retained) fn member(&self, identity: RetainedSlotIdentity) -> Option<&Block> {
            self.table.member(identity, self.set_id)
        }

        fn source_allocation(
            &self,
            identity: RetainedSlotIdentity,
        ) -> Option<&RetainedAllocation<Block>> {
            self.table
                .source_sharing_capability()
                .source_allocation(identity, self.set_id)
        }

        fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
            self.table.source_sharing_capability().belongs_to(account)
        }

        pub(super) const fn set_id(&self) -> RetainedSetId {
            self.set_id
        }

        pub(in crate::retained) fn lookup_member<Tag>(
            &self,
            identity: RetainedSlotIdentity,
            expected_destination: RetainedMembershipDestination,
        ) -> Option<RetainedLookupMember<'_, Block>>
        where
            Tag: Copy + Send + Sync + 'static,
        {
            self.table
                .lookup_member::<Tag>(self.set_id, identity, expected_destination)
        }

        pub(super) fn rewrite_source_allocation(
            &self,
            identity: RetainedSlotIdentity,
            expected_destination: RetainedMembershipDestination,
        ) -> Option<&RetainedAllocation<Block>> {
            if self.table.lookup_capability().table_membership()
                != Some(RetainedMembership {
                    set_id: self.set_id,
                    destination: expected_destination,
                    ordinal: 0,
                })
            {
                return None;
            }
            let allocation = self.source_allocation(identity)?;
            let ordinal = u64::try_from(identity.index).ok()?.checked_add(1)?;
            (allocation.membership()
                == Some(RetainedMembership {
                    set_id: self.set_id,
                    destination: expected_destination,
                    ordinal,
                }))
            .then_some(allocation)
        }

        pub(super) fn inheritance_source_member(
            &self,
            index: usize,
            expected_destination: RetainedMembershipDestination,
        ) -> Option<PublishedInheritanceSourceMember<'_, Block>> {
            if self.table.lookup_capability().table_membership()
                != Some(RetainedMembership {
                    set_id: self.set_id,
                    destination: expected_destination,
                    ordinal: 0,
                })
            {
                return None;
            }
            let member = self.table.table.as_ref().members.get(index)?;
            let ordinal = u64::try_from(index).ok()?.checked_add(1)?;
            if member.allocation.membership()
                != Some(RetainedMembership {
                    set_id: self.set_id,
                    destination: expected_destination,
                    ordinal,
                })
            {
                return None;
            }
            Some(PublishedInheritanceSourceMember {
                allocation: &member.allocation,
                identity: RetainedSlotIdentity {
                    set_id: self.set_id,
                    index,
                    tag: member.tag,
                },
            })
        }

        pub(in crate::retained) fn len(&self) -> usize {
            self.table.len()
        }

        #[cfg(test)]
        pub(in crate::retained) fn test_source_allocation(
            &self,
            identity: RetainedSlotIdentity,
        ) -> Option<&RetainedAllocation<Block>> {
            self.source_allocation(identity)
        }

        #[cfg(test)]
        pub(in crate::retained) fn test_member_share_call_count(
            &self,
            identity: RetainedSlotIdentity,
        ) -> Option<usize> {
            self.source_allocation(identity)
                .map(RetainedAllocation::share_call_count)
        }

        #[cfg(test)]
        pub(in crate::retained) fn test_table_share_call_count(&self) -> usize {
            self.table.share_call_count()
        }

        #[cfg(test)]
        pub(in crate::retained) fn test_table_allocation_identity(&self) -> *const () {
            self.table.allocation_identity()
        }
    }

    pub(super) fn aggregate_belongs_to<Block>(
        aggregate: &PublishedRetainedAggregate<Block>,
        account: &PhysicalStorageAccount,
    ) -> bool {
        aggregate.belongs_to(account)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn share_retained_block<O, P, D, Block, Tag>(
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
        if source_identity.tag != TypeId::of::<Tag>() {
            return Err(RetainedPublicationError::InvalidSourceSlot);
        }
        share_retained_block_runtime(
            ledger,
            account,
            handle_share_work,
            table,
            set_id,
            destination,
            slot_limit_error,
            source,
            source_identity,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn share_retained_block_runtime<O, P, D, Block>(
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
        if !source.belongs_to(account) {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::ForeignAccount,
            ));
        }
        if source_identity.set_id != source.set_id {
            return Err(RetainedPublicationError::InvalidSourceSlot);
        }
        let source_allocation = source
            .rewrite_source_allocation(source_identity, destination)
            .ok_or(RetainedPublicationError::InvalidSourceSlot)?;
        if !source_allocation.belongs_to(account) {
            drop(preflight);
            let cause = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(cause),
            ));
        }
        let table_value = table.as_ref();
        let source_mapping_is_exact = destination != RetainedMembershipDestination::Canonical
            || canonical_source_mapping_is_exact(table_value, set_id);
        if table_value.root_membership
            != (RetainedMembership {
                set_id,
                destination,
                ordinal: 0,
            })
            || !table_value.committed_summary_is_bounded()
            || !source_mapping_is_exact
            || (destination != RetainedMembershipDestination::Canonical
                && table_value.canonical_source_mapping.is_some())
        {
            drop(preflight);
            let cause = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(cause),
            ));
        }
        if destination == RetainedMembershipDestination::Canonical {
            table_value.require_canonical_source_binding(account, source.set_id(), source.len())?;
            match table_value.canonical_source_mapping(account, set_id, source_identity) {
                Ok(Some((identity, RetainedBlockOrigin::Inherited { .. }))) => {
                    return Ok(identity);
                }
                Ok(Some((_, RetainedBlockOrigin::Rewritten { .. }))) => {
                    return Err(RetainedPublicationError::InvalidSourceSlot);
                }
                Ok(Some((_, RetainedBlockOrigin::New))) | Err(()) => {
                    drop(preflight);
                    let cause =
                        account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                    return Err(RetainedPublicationError::Writer(
                        RetainedWriterError::AccountPoisoned(cause),
                    ));
                }
                Ok(None) => {}
            }
        }
        if table_value.committed_members >= table_value.total_blocks
            || table_value.committed_members >= table_value.members.capacity()
        {
            return Err(slot_limit_error);
        }
        let index = table_value.committed_members;
        let ordinal = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(RetainedPublicationError::Writer(
                RetainedWriterError::MembershipExhausted,
            ))?;
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
            super::try_share_retained_block_after_admission_runtime::<O, P, D, Block>(
                ledger,
                share_admission,
                handle_share_work,
                table,
                set_id,
                destination,
                index,
                ordinal,
                source_identity,
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
}

use published_table_capability::PublishedRetainedAggregate;

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
        visit_epoch: _,
    } in members
    {
        let outcome =
            if mode == RetainedReleaseMode::BeforeAttribution && origin.owns_new_allocation() {
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
/// once within this session's nonzero epoch. Any lookup returning `None` invalidates the whole
/// session even if the callback ignores it.
pub(crate) struct RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock> {
    pub(super) canonical: &'a RetainedBlockTable<CanonicalBlock>,
    pub(super) evidence: &'a RetainedBlockTable<EvidenceBlock>,
    pub(super) set_id: RetainedSetId,
    pub(super) epoch: NonZeroU64,
    pub(super) canonical_seen: usize,
    pub(super) evidence_seen: usize,
    pub(super) valid: bool,
    pub(super) not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'a, CanonicalBlock, EvidenceBlock> RetainedClosureSession<'a, CanonicalBlock, EvidenceBlock> {
    fn canonical_member<Tag>(
        &mut self,
        slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
    ) -> Option<&'a RetainedBlockMember<CanonicalBlock>>
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
        Some(member)
    }

    fn outcome_evidence_member<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<&'a RetainedBlockMember<EvidenceBlock>>
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
        Some(member)
    }

    /// Visits one canonical slot, returning `None` and invalidating the session on mismatch.
    pub fn canonical<Tag>(
        &mut self,
        slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
    ) -> Option<&'a CanonicalBlock>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let member = self.canonical_member(slot)?;
        if member.first_visit_in(self.epoch) {
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
        let member = self.outcome_evidence_member(slot)?;
        if member.first_visit_in(self.epoch) {
            let Some(seen) = self.evidence_seen.checked_add(1) else {
                self.valid = false;
                return None;
            };
            self.evidence_seen = seen;
        }
        Some(member.allocation.as_ref())
    }

    #[cfg(feature = "_u4-activation")]
    pub(crate) fn peek_canonical<Tag>(
        &mut self,
        slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
    ) -> Option<RetainedCanonicalMemberVisit<'a, CanonicalBlock, Tag>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let member = self.canonical_member(slot)?;
        if member.completed_in(self.epoch) {
            Some(RetainedCanonicalMemberVisit::Completed(
                member.allocation.as_ref(),
            ))
        } else {
            Some(RetainedCanonicalMemberVisit::Pending(
                RetainedPendingCanonicalMember {
                    block: member.allocation.as_ref(),
                    slot,
                    epoch: self.epoch,
                },
            ))
        }
    }

    #[cfg(feature = "_u4-activation")]
    pub(crate) fn complete_canonical<Tag>(
        &mut self,
        pending: RetainedPendingCanonicalMember<'a, CanonicalBlock, Tag>,
    ) -> bool
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if pending.epoch != self.epoch {
            self.valid = false;
            return false;
        }
        let Some(member) = self.canonical_member(pending.slot) else {
            return false;
        };
        if !std::ptr::eq(member.allocation.as_ref(), pending.block)
            || !member.try_complete_in(self.epoch)
        {
            self.valid = false;
            return false;
        }
        let Some(seen) = self.canonical_seen.checked_add(1) else {
            self.valid = false;
            return false;
        };
        self.canonical_seen = seen;
        true
    }

    #[cfg(feature = "_u4-activation")]
    pub(crate) fn peek_outcome_evidence<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<RetainedOutcomeEvidenceMemberVisit<'a, EvidenceBlock, Tag>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let member = self.outcome_evidence_member(slot)?;
        if member.completed_in(self.epoch) {
            Some(RetainedOutcomeEvidenceMemberVisit::Completed(
                member.allocation.as_ref(),
            ))
        } else {
            Some(RetainedOutcomeEvidenceMemberVisit::Pending(
                RetainedPendingOutcomeEvidenceMember {
                    block: member.allocation.as_ref(),
                    slot,
                    epoch: self.epoch,
                },
            ))
        }
    }

    #[cfg(feature = "_u4-activation")]
    pub(crate) fn complete_outcome_evidence<Tag>(
        &mut self,
        pending: RetainedPendingOutcomeEvidenceMember<'a, EvidenceBlock, Tag>,
    ) -> bool
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if pending.epoch != self.epoch {
            self.valid = false;
            return false;
        }
        let Some(member) = self.outcome_evidence_member(pending.slot) else {
            return false;
        };
        if !std::ptr::eq(member.allocation.as_ref(), pending.block)
            || !member.try_complete_in(self.epoch)
        {
            self.valid = false;
            return false;
        }
        let Some(seen) = self.evidence_seen.checked_add(1) else {
            self.valid = false;
            return false;
        };
        self.evidence_seen = seen;
        true
    }

    #[cfg(feature = "_u4-activation")]
    pub(crate) fn invalidate(&mut self) {
        self.valid = false;
    }
}

#[cfg(feature = "_u4-activation")]
pub(crate) enum RetainedCanonicalMemberVisit<'a, CanonicalBlock, Tag> {
    Completed(&'a CanonicalBlock),
    Pending(RetainedPendingCanonicalMember<'a, CanonicalBlock, Tag>),
}

#[cfg(feature = "_u4-activation")]
#[must_use = "a pending canonical member must be completed or leave the closure incomplete"]
pub(crate) struct RetainedPendingCanonicalMember<'a, CanonicalBlock, Tag> {
    block: &'a CanonicalBlock,
    slot: CanonicalRetainedSlot<CanonicalBlock, Tag>,
    epoch: NonZeroU64,
}

#[cfg(feature = "_u4-activation")]
impl<'a, CanonicalBlock, Tag> RetainedPendingCanonicalMember<'a, CanonicalBlock, Tag> {
    pub(crate) const fn block(&self) -> &'a CanonicalBlock {
        self.block
    }
}

#[cfg(feature = "_u4-activation")]
pub(crate) enum RetainedOutcomeEvidenceMemberVisit<'a, EvidenceBlock, Tag> {
    Completed(&'a EvidenceBlock),
    Pending(RetainedPendingOutcomeEvidenceMember<'a, EvidenceBlock, Tag>),
}

#[cfg(feature = "_u4-activation")]
#[must_use = "a pending outcome-evidence member must be completed or leave the closure incomplete"]
pub(crate) struct RetainedPendingOutcomeEvidenceMember<'a, EvidenceBlock, Tag> {
    block: &'a EvidenceBlock,
    slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    epoch: NonZeroU64,
}

#[cfg(feature = "_u4-activation")]
impl<'a, EvidenceBlock, Tag> RetainedPendingOutcomeEvidenceMember<'a, EvidenceBlock, Tag> {
    pub(crate) const fn block(&self) -> &'a EvidenceBlock {
        self.block
    }
}

/// Read-only reachability session for an evidence-only graph.
///
/// Validation must visit every unique evidence slot. Repeated DAG edges count once within this
/// session's nonzero epoch, and a failed lookup invalidates the complete session.
pub(crate) struct RetainedEvidenceClosureSession<'a, EvidenceBlock> {
    pub(super) evidence: &'a RetainedBlockTable<EvidenceBlock>,
    pub(super) set_id: RetainedSetId,
    pub(super) epoch: NonZeroU64,
    pub(super) seen: usize,
    pub(super) valid: bool,
    pub(super) not_send_or_sync: PhantomData<Rc<()>>,
}

/// Result of opening one evidence member for controlled validation.
#[cfg(feature = "_u4-activation")]
pub(crate) enum RetainedEvidenceMemberVisit<'a, EvidenceBlock, Tag> {
    Completed(&'a EvidenceBlock),
    Pending(RetainedPendingEvidenceMember<'a, EvidenceBlock, Tag>),
}

/// Linear completion authority for one inspected evidence member.
#[cfg(feature = "_u4-activation")]
#[must_use = "a pending evidence member must be completed or leave the closure incomplete"]
pub(crate) struct RetainedPendingEvidenceMember<'a, EvidenceBlock, Tag> {
    block: &'a EvidenceBlock,
    slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    epoch: NonZeroU64,
}

#[cfg(feature = "_u4-activation")]
impl<'a, EvidenceBlock, Tag> RetainedPendingEvidenceMember<'a, EvidenceBlock, Tag> {
    pub(crate) const fn block(&self) -> &'a EvidenceBlock {
        self.block
    }
}

impl<'a, EvidenceBlock> RetainedEvidenceClosureSession<'a, EvidenceBlock> {
    fn outcome_evidence_member<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<&'a RetainedBlockMember<EvidenceBlock>>
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
        Some(member)
    }

    /// Visits one outcome-evidence slot, returning `None` and invalidating the session on mismatch.
    pub fn outcome_evidence<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<&'a EvidenceBlock>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let member = self.outcome_evidence_member(slot)?;
        if member.first_visit_in(self.epoch) {
            let Some(seen) = self.seen.checked_add(1) else {
                self.valid = false;
                return None;
            };
            self.seen = seen;
        }
        Some(member.allocation.as_ref())
    }

    /// Opens one evidence member without marking it complete in this validation epoch.
    ///
    /// A previously completed member is reported directly. A new member returns a linear pending
    /// token that must be consumed by completion after its contents are inspected. Identity,
    /// index, or tag mismatch invalidates the complete closure session.
    #[cfg(feature = "_u4-activation")]
    pub(crate) fn peek_outcome_evidence<Tag>(
        &mut self,
        slot: OutcomeEvidenceRetainedSlot<EvidenceBlock, Tag>,
    ) -> Option<RetainedEvidenceMemberVisit<'a, EvidenceBlock, Tag>>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let member = self.outcome_evidence_member(slot)?;
        if member.completed_in(self.epoch) {
            Some(RetainedEvidenceMemberVisit::Completed(
                member.allocation.as_ref(),
            ))
        } else {
            Some(RetainedEvidenceMemberVisit::Pending(
                RetainedPendingEvidenceMember {
                    block: member.allocation.as_ref(),
                    slot,
                    epoch: self.epoch,
                },
            ))
        }
    }

    /// Completes one previously opened evidence member in this validation epoch.
    ///
    /// Completion revalidates the token's epoch, complete slot identity, and backing member. A
    /// duplicate completion or competing epoch update invalidates the closure instead of
    /// increasing `seen` again.
    #[cfg(feature = "_u4-activation")]
    pub(crate) fn complete_outcome_evidence<Tag>(
        &mut self,
        pending: RetainedPendingEvidenceMember<'a, EvidenceBlock, Tag>,
    ) -> bool
    where
        Tag: Copy + Send + Sync + 'static,
    {
        if pending.epoch != self.epoch {
            self.valid = false;
            return false;
        }
        let Some(member) = self.outcome_evidence_member(pending.slot) else {
            return false;
        };
        if !std::ptr::eq(member.allocation.as_ref(), pending.block) {
            self.valid = false;
            return false;
        }
        if !member.try_complete_in(self.epoch) {
            self.valid = false;
            return false;
        }
        let Some(seen) = self.seen.checked_add(1) else {
            self.valid = false;
            return false;
        };
        self.seen = seen;
        true
    }

    /// Invalidates the complete closure after a concrete-store projection mismatch.
    #[cfg(feature = "_u4-activation")]
    pub(crate) fn invalidate(&mut self) {
        self.valid = false;
    }
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

/// Immutable canonical block table after the atomic publication handoff.
pub(crate) struct PublishedCanonicalAggregate<Block> {
    pub(super) inner: PublishedRetainedAggregate<Block>,
}

impl<Block> PublishedCanonicalAggregate<Block> {
    pub(super) fn from_table(table: RetainedTableAllocation<Block>, set_id: RetainedSetId) -> Self {
        Self {
            inner: PublishedRetainedAggregate::new(table, set_id),
        }
    }

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

    pub(crate) fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        published_table_capability::aggregate_belongs_to(&self.inner, account)
    }

    pub(super) const fn set_id(&self) -> RetainedSetId {
        self.inner.set_id()
    }

    fn rewrite_source_block(
        &self,
        identity: RetainedSlotIdentity,
        account: &PhysicalStorageAccount,
    ) -> Option<&Block> {
        if !self.belongs_to(account) {
            return None;
        }
        let allocation = self
            .inner
            .rewrite_source_allocation(identity, RetainedMembershipDestination::Canonical)?;
        allocation
            .belongs_to(account)
            .then_some(allocation.as_ref())
    }

    fn inheritance_source_member(
        &self,
        index: usize,
        account: &PhysicalStorageAccount,
    ) -> Option<published_table_capability::PublishedInheritanceSourceMember<'_, Block>> {
        if !self.belongs_to(account) {
            return None;
        }
        let member = self
            .inner
            .inheritance_source_member(index, RetainedMembershipDestination::Canonical)?;
        member.allocation.belongs_to(account).then_some(member)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub(crate) enum CanonicalSourceBindError<D> {
    Publication(RetainedPublicationError<D>),
    Control(ControlCause),
    Ledger(LedgerCause<D>),
}

pub(crate) enum CanonicalLeafInheritanceError<D> {
    Publication(RetainedPublicationError<D>),
    Control(ControlCause),
    Ledger(LedgerCause<D>),
}

impl<D> From<RetainedPublicationError<D>> for CanonicalLeafInheritanceError<D> {
    fn from(cause: RetainedPublicationError<D>) -> Self {
        Self::Publication(cause)
    }
}

pub(crate) trait CanonicalRetainedLeafPolicy: Send + Sync + 'static {
    fn is_shareable_canonical_leaf(&self) -> bool;
}

impl<D> From<RetainedPublicationError<D>> for CanonicalSourceBindError<D> {
    fn from(cause: RetainedPublicationError<D>) -> Self {
        Self::Publication(cause)
    }
}

/// One checked source-to-destination member mapping used only while rebuilding a canonical block.
pub(crate) struct CanonicalRetainedRewriteTarget<'source, 'destination, Block> {
    source_block: &'source Block,
    destination_block: &'destination Block,
    destination_identity: RetainedSlotIdentity,
}

impl<'source, 'destination, Block> CanonicalRetainedRewriteTarget<'source, 'destination, Block> {
    pub(crate) const fn source_block(&self) -> &'source Block {
        self.source_block
    }

    pub(crate) const fn destination_block(&self) -> &'destination Block {
        self.destination_block
    }

    pub(crate) const fn destination_identity(&self) -> RetainedSlotIdentity {
        self.destination_identity
    }
}

/// Allocation-free, bounded lookup from one published source member to an existing destination
/// member that was inherited or rewritten from that exact source identity.
pub(crate) struct CanonicalRetainedRewriteMap<'source, 'destination, Block> {
    source: &'source PublishedCanonicalAggregate<Block>,
    destination: &'destination RetainedBlockTable<Block>,
    account: &'source PhysicalStorageAccount,
    destination_set: RetainedSetId,
}

impl<'source, 'destination, Block> CanonicalRetainedRewriteMap<'source, 'destination, Block> {
    fn new(
        source: &'source PublishedCanonicalAggregate<Block>,
        destination: &'destination RetainedBlockTable<Block>,
        account: &'source PhysicalStorageAccount,
        destination_set: RetainedSetId,
    ) -> Self {
        Self {
            source,
            destination,
            account,
            destination_set,
        }
    }

    pub(crate) fn resolve(
        &self,
        source_identity: RetainedSlotIdentity,
    ) -> Option<CanonicalRetainedRewriteTarget<'source, 'destination, Block>> {
        if source_identity.set_id == self.destination_set {
            return None;
        }
        let source = self.source;
        let source_block = source.rewrite_source_block(source_identity, self.account)?;
        let destination = self.destination;
        let (destination_identity, _) = match destination.canonical_source_mapping(
            self.account,
            self.destination_set,
            source_identity,
        ) {
            Ok(Some(mapped)) => mapped,
            Ok(None) => return None,
            Err(()) => {
                let _ = self
                    .account
                    .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                return None;
            }
        };
        let member = destination.members.get(destination_identity.index)?;
        Some(CanonicalRetainedRewriteTarget {
            source_block,
            destination_block: member.allocation.as_ref(),
            destination_identity,
        })
    }
}

/// Immutable outcome-evidence block table after the atomic publication handoff.
pub(crate) struct PublishedOutcomeEvidenceAggregate<Block> {
    pub(super) inner: PublishedRetainedAggregate<Block>,
}

impl<Block> PublishedOutcomeEvidenceAggregate<Block> {
    pub(super) fn from_table(table: RetainedTableAllocation<Block>, set_id: RetainedSetId) -> Self {
        Self {
            inner: PublishedRetainedAggregate::new(table, set_id),
        }
    }

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

    pub(crate) fn belongs_to(&self, account: &PhysicalStorageAccount) -> bool {
        published_table_capability::aggregate_belongs_to(&self.inner, account)
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
    canonical_source_mapping_capacity: Option<CapacityPlan>,
    charge: AdmissionCharge,
    block: PhantomData<fn(Block) -> Block>,
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
    membership_destination: RetainedMembershipDestination,
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
        || (source.canonical_source_mapping_capacity != 0
            && membership_destination != RetainedMembershipDestination::Canonical)
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
    let member_bytes = member_capacity
        .envelope_delta_bytes()
        .map_err(|_| RetainedWriterError::BatchCapacity)?;
    let canonical_source_mapping_capacity = if source.canonical_source_mapping_capacity == 0 {
        None
    } else {
        Some(
            CapacityPlan::for_type::<Option<NonZeroUsize>>(
                0,
                0,
                source.canonical_source_mapping_capacity,
            )
            .map_err(|_| RetainedWriterError::BatchCapacity)?,
        )
    };
    let mapping_bytes = canonical_source_mapping_capacity
        .map(CapacityPlan::envelope_delta_bytes)
        .transpose()
        .map_err(|_| RetainedWriterError::BatchCapacity)?
        .unwrap_or(0);
    let nested_bytes = member_bytes
        .checked_add(mapping_bytes)
        .ok_or(RetainedWriterError::BatchCapacity)?;
    let nested_allocations = u64::from(member_bytes != 0)
        .checked_add(u64::from(mapping_bytes != 0))
        .ok_or(RetainedWriterError::BatchCapacity)?;
    let allocation = RetainedAllocationPlan::new(nested_bytes, nested_allocations)
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
            canonical_source_mapping_capacity,
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
        canonical_source_mapping_capacity,
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
        Err(RetainedShareAdmissionError::Physical(cause)) => {
            drop(attribution);
            return Err(RetainedWriterError::Physical(cause));
        }
        Err(RetainedShareAdmissionError::Ledger(cause)) => {
            drop(attribution);
            return Err(RetainedWriterError::Ledger(cause));
        }
    };
    let membership = RetainedMembership {
        set_id,
        destination: membership_destination,
        ordinal: 0,
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
                let canonical_source_mapping = match canonical_source_mapping_capacity {
                    Some(mapping_capacity) => {
                        let mut entries = Vec::new();
                        if entries
                            .try_reserve_exact(mapping_capacity.allocator_additional())
                            .is_err()
                        {
                            return Err(RetainedBuildFailureCode::new(1));
                        }
                        if mapping_capacity
                            .finish(entries.len(), entries.capacity())
                            .is_err()
                        {
                            return Err(RetainedBuildFailureCode::new(2));
                        }
                        Some(CanonicalSourceMapping {
                            qualified_capacity: entries.capacity(),
                            entries,
                            source_set: None,
                            mapped_entries: 0,
                            #[cfg(test)]
                            probes: AtomicU64::new(0),
                        })
                    }
                    None => None,
                };
                Ok(RetainedBlockTable {
                    qualified_capacity: members.capacity(),
                    members,
                    canonical_source_mapping,
                    root_membership: membership,
                    total_blocks,
                    max_new_blocks,
                    committed_members: 0,
                    committed_new_blocks: 0,
                    committed_derived_members: 0,
                    committed_new_block_bytes: 0,
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
    origin: RetainedBlockOrigin,
    plan: RetainedAllocationPlan,
    build: impl for<'a> FnOnce(
        BlockEmplacer<'a, Block>,
        &'a RetainedBlockTable<Block>,
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
    if !origin.owns_new_allocation() {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::InvalidPublicationPlan,
        ));
    }
    let preflight = account
        .try_begin_account_preflight()
        .map_err(RetainedWriterError::AccountPoisoned)
        .map_err(RetainedPublicationError::Writer)?;
    if !table.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount,
        ));
    }
    let index = {
        let table = table.as_ref();
        let source_mapping_is_exact = destination != RetainedMembershipDestination::Canonical
            || canonical_source_mapping_is_exact(table, set_id);
        if table.root_membership
            != (RetainedMembership {
                set_id,
                destination,
                ordinal: 0,
            })
            || !table.committed_summary_is_bounded()
            || !source_mapping_is_exact
            || (destination != RetainedMembershipDestination::Canonical
                && table.canonical_source_mapping.is_some())
        {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ));
        }
        if table.committed_members >= table.total_blocks
            || table.committed_new_blocks >= table.max_new_blocks
        {
            return Err(slot_limit_error);
        }
        if table.committed_members >= table.members.capacity() {
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::BatchCapacity,
            ));
        }
        table.committed_members
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
        Err(RetainedShareAdmissionError::Physical(cause)) => {
            drop(attribution);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Physical(cause),
            ));
        }
        Err(RetainedShareAdmissionError::Ledger(cause)) => {
            drop(attribution);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Ledger(cause),
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
            build(
                BlockEmplacer {
                    marker: PhantomData,
                },
                table,
            )
            .map(BlockPlacement::into_block)
        },
    ) {
        Ok(allocation) => allocation,
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
    let identity = table
        .try_push_committed_member(
            account,
            set_id,
            destination,
            RetainedBlockMember {
                allocation,
                tag: TypeId::of::<Tag>(),
                origin,
                visit_epoch: AtomicU64::new(0),
            },
        )
        .map_err(|()| {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(poisoned))
        })?;
    Ok(identity)
}

pub(super) fn try_bind_canonical_source_mapping<O, P, PP, U, D, Block>(
    account: &PhysicalStorageAccount,
    table: &mut RetainedTableAllocation<Block>,
    destination_set: RetainedSetId,
    source: &PublishedCanonicalAggregate<Block>,
    mut control: RetainedWorkControl<'_, O, P, PP, U, D>,
) -> Result<(), CanonicalSourceBindError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: crate::ProgressPhase,
    U: crate::ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
{
    drop(
        account
            .try_begin_account_preflight()
            .map_err(RetainedWriterError::AccountPoisoned)
            .map_err(RetainedPublicationError::Writer)?,
    );
    if !table.belongs_to(account) || !source.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(RetainedWriterError::ForeignAccount).into());
    }
    if table.as_ref().root_membership
        != (RetainedMembership {
            set_id: destination_set,
            destination: RetainedMembershipDestination::Canonical,
            ordinal: 0,
        })
    {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(CanonicalSourceBindError::Publication(
            RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(poisoned)),
        ));
    }
    let source_set = source.set_id();
    let source_len = source.len();
    if source_set == destination_set || source_len == 0 {
        return Err(RetainedPublicationError::InvalidSourceSlot.into());
    }
    match table
        .as_ref()
        .validate_canonical_source_binding(source_set, source_len)
    {
        Ok(()) => return Ok(()),
        Err(CanonicalSourceBindingStateError::Unbound) => {}
        Err(CanonicalSourceBindingStateError::Unavailable) => {
            return Err(CanonicalSourceBindError::Publication(
                RetainedPublicationError::Writer(RetainedWriterError::InvalidPublicationPlan),
            ));
        }
        Err(CanonicalSourceBindingStateError::Mismatch) => {
            return Err(RetainedPublicationError::InvalidSourceSlot.into());
        }
        Err(CanonicalSourceBindingStateError::Corrupt) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(CanonicalSourceBindError::Publication(
                RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(poisoned)),
            ));
        }
    }
    let table_value = table.table_mut_during_construction().ok_or_else(|| {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        CanonicalSourceBindError::Publication(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(poisoned),
        ))
    })?;
    let mapping = table_value.canonical_source_mapping.as_mut().ok_or({
        CanonicalSourceBindError::Publication(RetainedPublicationError::Writer(
            RetainedWriterError::InvalidPublicationPlan,
        ))
    })?;
    if !mapping.entries.is_empty()
        || !mapping.shape_is_exact()
        || mapping.mapped_entries != 0
        || source_len > mapping.qualified_capacity
    {
        return Err(RetainedPublicationError::InvalidSourceSlot.into());
    }

    let fill = catch_unwind(AssertUnwindSafe(|| {
        for _ in 0..source_len {
            control.poll().map_err(CanonicalSourceBindError::Control)?;
            control
                .charge_observed(1)
                .map_err(CanonicalSourceBindError::Ledger)?;
            mapping.entries.push(None);
        }
        Ok::<(), CanonicalSourceBindError<D>>(())
    }));
    match fill {
        Ok(Ok(()))
            if mapping.entries.len() == source_len
                && mapping.entries.capacity() == mapping.qualified_capacity =>
        {
            mapping.source_set = Some(source_set);
            Ok(())
        }
        Ok(Ok(())) => {
            mapping.entries.clear();
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            Err(CanonicalSourceBindError::Publication(
                RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(poisoned)),
            ))
        }
        Ok(Err(cause)) => {
            mapping.entries.clear();
            Err(cause)
        }
        Err(payload) => {
            mapping.entries.clear();
            let poisoned = account.record_poison(AccountPoisonReason::RetainedBuilderPanicked);
            discard_untrusted_panic_payload(payload);
            Err(CanonicalSourceBindError::Publication(
                RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(poisoned)),
            ))
        }
    }
}

fn inherited_prefix_release_error<D>(
    outcome: FinalOwnerReleaseOutcome,
) -> Option<RetainedPublicationError<D>> {
    match outcome {
        FinalOwnerReleaseOutcome::Clean => None,
        FinalOwnerReleaseOutcome::StorageReleaseUnverified(_) => Some(
            RetainedPublicationError::Writer(RetainedWriterError::StorageDropUnverified),
        ),
        FinalOwnerReleaseOutcome::ChargeReleaseUnverified(_) => Some(
            RetainedPublicationError::Writer(RetainedWriterError::ChargeReleaseUnverified),
        ),
        FinalOwnerReleaseOutcome::OwnershipUnverified(_) => Some(RetainedPublicationError::Writer(
            RetainedWriterError::OwnershipUnverified,
        )),
    }
}

fn try_rollback_inherited_prefix<D, Block>(
    account: &PhysicalStorageAccount,
    table: &mut RetainedTableAllocation<Block>,
    initial_members: usize,
) -> Result<(), RetainedPublicationError<D>>
where
    Block: Send + Sync + 'static,
{
    let Some(table) = table.table_mut_during_construction() else {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(poisoned),
        ));
    };
    if !table.committed_summary_is_bounded()
        || !canonical_source_mapping_is_exact(table, table.root_membership.set_id)
        || table.committed_members < initial_members
    {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(poisoned),
        ));
    }
    while table.committed_members > initial_members {
        let member = match table.try_pop_committed_member(RetainedBlockPopExpectation::Inherited) {
            Ok(member) => member,
            Err(()) => {
                let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(RetainedPublicationError::Writer(
                    RetainedWriterError::AccountPoisoned(poisoned),
                ));
            }
        };
        let RetainedBlockOrigin::Inherited { source } = member.origin else {
            std::mem::forget(member);
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ));
        };
        if let Some(cause) = inherited_prefix_release_error(member.allocation.release_storage()) {
            return Err(cause);
        }
        if table.clear_canonical_source_mapping(source).is_err() {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ));
        }
    }
    Ok(())
}

pub(super) fn try_inherit_all_canonical_leaves<O, P, PP, U, D, Block>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    table: &mut RetainedTableAllocation<Block>,
    destination_set: RetainedSetId,
    source: &PublishedCanonicalAggregate<Block>,
    mut control: RetainedWorkControl<'_, O, P, PP, U, D>,
) -> Result<(), CanonicalLeafInheritanceError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: crate::ProgressPhase,
    U: crate::ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: CanonicalRetainedLeafPolicy,
{
    drop(
        account
            .try_begin_account_preflight()
            .map_err(RetainedWriterError::AccountPoisoned)
            .map_err(RetainedPublicationError::Writer)?,
    );
    if !table.belongs_to(account) || !source.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(RetainedWriterError::ForeignAccount).into());
    }
    table
        .as_ref()
        .require_canonical_source_binding(account, source.set_id(), source.len())?;
    let initial_members = table.as_ref().committed_members;
    let inherit = catch_unwind(AssertUnwindSafe(|| {
        for source_index in 0..source.len() {
            control
                .poll()
                .map_err(CanonicalLeafInheritanceError::Control)?;
            control
                .charge_observed(1)
                .map_err(CanonicalLeafInheritanceError::Ledger)?;
            let Some(member) = source.inheritance_source_member(source_index, account) else {
                let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                return Err(CanonicalLeafInheritanceError::Publication(
                    RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(
                        poisoned,
                    )),
                ));
            };
            if !member.allocation.as_ref().is_shareable_canonical_leaf() {
                continue;
            }
            match table
                .as_ref()
                .canonical_source_mapping(account, destination_set, member.identity)
            {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(()) => {
                    let poisoned =
                        account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
                    return Err(CanonicalLeafInheritanceError::Publication(
                        RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(
                            poisoned,
                        )),
                    ));
                }
            }
            published_table_capability::share_retained_block_runtime(
                ledger,
                account,
                handle_share_work,
                table,
                destination_set,
                RetainedMembershipDestination::Canonical,
                RetainedPublicationError::CanonicalSlotLimit,
                &source.inner,
                member.identity,
            )
            .map(drop)
            .map_err(CanonicalLeafInheritanceError::Publication)?;
        }
        Ok(())
    }));
    let cause = match inherit {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(cause)) => cause,
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::RetainedBuilderPanicked);
            discard_untrusted_panic_payload(payload);
            CanonicalLeafInheritanceError::Publication(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ))
        }
    };
    match try_rollback_inherited_prefix(account, table, initial_members) {
        Ok(()) => Err(cause),
        Err(rollback) => Err(CanonicalLeafInheritanceError::Publication(rollback)),
    }
}

fn try_validate_canonical_rewrite_source<'source, D, Block, Tag>(
    account: &PhysicalStorageAccount,
    table: &RetainedTableAllocation<Block>,
    set_id: RetainedSetId,
    source: &'source PublishedCanonicalAggregate<Block>,
    source_identity: RetainedSlotIdentity,
) -> Result<&'source Block, RetainedPublicationError<D>>
where
    Tag: Copy + Send + Sync + 'static,
{
    if !source.belongs_to(account) || !table.belongs_to(account) {
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount,
        ));
    }
    if source_identity.tag != TypeId::of::<Tag>() || source_identity.set_id == set_id {
        return Err(RetainedPublicationError::InvalidSourceSlot);
    }
    let source_block = source
        .rewrite_source_block(source_identity, account)
        .ok_or(RetainedPublicationError::InvalidSourceSlot)?;
    table
        .as_ref()
        .require_canonical_source_binding(account, source.set_id(), source.len())?;
    match table
        .as_ref()
        .canonical_source_mapping(account, set_id, source_identity)
    {
        Ok(None) => Ok(source_block),
        Ok(Some(_)) => Err(RetainedPublicationError::InvalidSourceSlot),
        Err(()) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            Err(RetainedPublicationError::Writer(
                RetainedWriterError::AccountPoisoned(poisoned),
            ))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn try_retain_rewritten_canonical_block<O, P, D, Block, Tag>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    phase: P,
    attribution_dimension: D,
    batch: &mut AttributionReservationBatch<O, P, D, CanonicalOutput>,
    table: &mut RetainedTableAllocation<Block>,
    set_id: RetainedSetId,
    source: &PublishedCanonicalAggregate<Block>,
    source_identity: RetainedSlotIdentity,
    plan: RetainedAllocationPlan,
    build: impl for<'a> FnOnce(
        BlockEmplacer<'a, Block>,
    ) -> Result<BlockPlacement<'a, Block, Tag>, RetainedBuildFailureCode>,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    let _source_block = try_validate_canonical_rewrite_source::<D, Block, Tag>(
        account,
        table,
        set_id,
        source,
        source_identity,
    )?;
    try_retain_block(
        ledger,
        account,
        handle_share_work,
        phase,
        attribution_dimension,
        batch,
        table,
        set_id,
        RetainedMembershipDestination::Canonical,
        RetainedPublicationError::CanonicalSlotLimit,
        RetainedBlockOrigin::Rewritten {
            source: source_identity,
        },
        plan,
        |emplacer, _| build(emplacer),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn try_rewrite_canonical_block_controlled<O, P, PP, U, D, Block, Tag>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    handle_share_work: RetainedHandleSharePlan<D>,
    phase: P,
    attribution_dimension: D,
    batch: &mut AttributionReservationBatch<O, P, D, CanonicalOutput>,
    table: &mut RetainedTableAllocation<Block>,
    set_id: RetainedSetId,
    source: &PublishedCanonicalAggregate<Block>,
    source_identity: RetainedSlotIdentity,
    allocation_plan: RetainedAllocationPlan,
    control: RetainedWorkControl<'_, O, P, PP, U, D>,
    build: impl for<'placement, 'source, 'destination, 'control> FnOnce(
        BlockEmplacer<'placement, Block>,
        &'source Block,
        CanonicalRetainedRewriteMap<'source, 'destination, Block>,
        RetainedWorkControl<'control, O, P, PP, U, D>,
    ) -> Result<
        BlockPlacement<'placement, Block, Tag>,
        RetainedBuildFailureCode,
    >,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    PP: crate::ProgressPhase,
    U: crate::ProgressUnit,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    let source_block = try_validate_canonical_rewrite_source::<D, Block, Tag>(
        account,
        table,
        set_id,
        source,
        source_identity,
    )?;
    try_retain_block(
        ledger,
        account,
        handle_share_work,
        phase,
        attribution_dimension,
        batch,
        table,
        set_id,
        RetainedMembershipDestination::Canonical,
        RetainedPublicationError::CanonicalSlotLimit,
        RetainedBlockOrigin::Rewritten {
            source: source_identity,
        },
        allocation_plan,
        move |emplacer, destination| {
            let mapping = CanonicalRetainedRewriteMap::new(source, destination, account, set_id);
            build(emplacer, source_block, mapping, control)
        },
    )
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
    published_table_capability::share_retained_block::<O, P, D, Block, Tag>(
        ledger,
        account,
        handle_share_work,
        table,
        set_id,
        destination,
        slot_limit_error,
        source,
        source_identity,
    )
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
    source_identity: RetainedSlotIdentity,
    source_allocation: &RetainedAllocation<Block>,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
    Tag: Copy + Send + Sync + 'static,
{
    if source_identity.tag != TypeId::of::<Tag>() {
        return Err(RetainedPublicationError::InvalidSourceSlot);
    }
    try_share_retained_block_after_admission_runtime(
        ledger,
        share_admission,
        handle_share_work,
        table,
        set_id,
        destination,
        index,
        ordinal,
        source_identity,
        source_allocation,
    )
}

#[allow(clippy::too_many_arguments)]
fn try_share_retained_block_after_admission_runtime<O, P, D, Block>(
    ledger: &Ledger<O, P, D>,
    share_admission: PendingAccountActionAdmission<'_>,
    handle_share_work: RetainedHandleSharePlan<D>,
    table: &mut RetainedBlockTable<Block>,
    set_id: RetainedSetId,
    destination: RetainedMembershipDestination,
    index: usize,
    ordinal: u64,
    source_identity: RetainedSlotIdentity,
    source_allocation: &RetainedAllocation<Block>,
) -> Result<RetainedSlotIdentity, RetainedPublicationError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
    Block: Send + Sync + 'static,
{
    let account = share_admission.account;
    let expected_ordinal = u64::try_from(table.committed_members)
        .ok()
        .and_then(|index| index.checked_add(1));
    if index != table.committed_members || expected_ordinal != Some(ordinal) {
        let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        return Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(poisoned),
        ));
    }
    let admitted_share_work = try_admit_retained_share_work(ledger, handle_share_work)
        .map_err(RetainedPublicationError::Writer)?;
    let mut allocation = match share_admission
        .run(|_| admitted_share_work.try_share_allocation(source_allocation))
    {
        Ok(allocation) => allocation,
        Err(failure) => {
            let (allocation, cause) = failure.into_parts();
            drop(allocation);
            return Err(RetainedPublicationError::Writer(
                RetainedWriterError::Ledger(cause),
            ));
        }
    };
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
    let identity = table
        .try_push_committed_member(
            account,
            set_id,
            destination,
            RetainedBlockMember {
                allocation,
                tag: source_identity.tag,
                origin: RetainedBlockOrigin::Inherited {
                    source: source_identity,
                },
                visit_epoch: AtomicU64::new(0),
            },
        )
        .map_err(|()| {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            RetainedPublicationError::Writer(RetainedWriterError::AccountPoisoned(poisoned))
        })?;
    Ok(identity)
}

fn canonical_source_mapping_is_exact<Block>(
    table: &RetainedBlockTable<Block>,
    destination_set: RetainedSetId,
) -> bool {
    if table.root_membership
        != (RetainedMembership {
            set_id: destination_set,
            destination: RetainedMembershipDestination::Canonical,
            ordinal: 0,
        })
    {
        return false;
    }
    let Some(mapping) = table.canonical_source_mapping.as_ref() else {
        return table.committed_derived_members == 0;
    };
    if !mapping.shape_is_exact() {
        return false;
    }
    let Some(source_set) = mapping.source_set else {
        return mapping.entries.is_empty()
            && mapping.mapped_entries == 0
            && table.committed_derived_members == 0;
    };
    source_set != destination_set
        && !mapping.entries.is_empty()
        && mapping.mapped_entries == table.committed_derived_members
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
    let exact = (|| {
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
        let table_charge = table.charged_bytes();
        let table_value = table.as_ref();
        if table_value.root_membership
            != (RetainedMembership {
                set_id,
                destination,
                ordinal: 0,
            })
            || !table_value.committed_summary_is_bounded()
        {
            return false;
        }
        if destination == RetainedMembershipDestination::Canonical {
            if !canonical_source_mapping_is_exact(table_value, set_id) {
                return false;
            }
        } else if table_value.canonical_source_mapping.is_some() {
            return false;
        }
        let Some(attributed) = table_value.committed_new_blocks.checked_add(1) else {
            return false;
        };
        let Some(attributed_bytes) =
            table_charge.checked_add(table_value.committed_new_block_bytes)
        else {
            return false;
        };
        attributed == batch.attribution_len() && batch.attributed_bytes() == Some(attributed_bytes)
    })();

    #[cfg(test)]
    assert_eq!(
        exact,
        audit_retained_block_table_is_exact(table, batch, account, set_id, destination),
        "incremental retained-table proof diverged from the full structural audit"
    );

    exact
}

#[cfg(test)]
fn audit_retained_block_table_is_exact<O, P, D, Destination, Block>(
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

    let mut audited_new_blocks = 0_usize;
    let mut audited_derived_members = 0_usize;
    let mut audited_new_block_bytes = 0_u64;
    let table_charge = table.charged_bytes();
    let table_value = table.as_ref();
    if table_value.root_membership
        != (RetainedMembership {
            set_id,
            destination,
            ordinal: 0,
        })
        || table_value.members.capacity() != table_value.qualified_capacity
        || table_value.members.len() > table_value.total_blocks
        || table_value.members.len() > table_value.members.capacity()
    {
        return false;
    }

    for (index, member) in table_value.members.iter().enumerate() {
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
        if member.origin.owns_new_allocation() {
            let Some(next_new_blocks) = audited_new_blocks.checked_add(1) else {
                return false;
            };
            let Some(next_new_block_bytes) =
                audited_new_block_bytes.checked_add(member.allocation.charged_bytes())
            else {
                return false;
            };
            audited_new_blocks = next_new_blocks;
            audited_new_block_bytes = next_new_block_bytes;
        }
        if member.origin.source_identity().is_some() {
            let Some(next_derived_members) = audited_derived_members.checked_add(1) else {
                return false;
            };
            audited_derived_members = next_derived_members;
        }
    }

    if table_value.committed_members != table_value.members.len()
        || table_value.committed_new_blocks != audited_new_blocks
        || table_value.committed_derived_members != audited_derived_members
        || table_value.committed_new_block_bytes != audited_new_block_bytes
        || audited_new_blocks > table_value.max_new_blocks
    {
        return false;
    }
    if destination == RetainedMembershipDestination::Canonical {
        if !audit_canonical_source_mapping_is_exact(table_value, set_id) {
            return false;
        }
    } else if table_value.canonical_source_mapping.is_some() {
        return false;
    }

    let Some(attributed) = audited_new_blocks.checked_add(1) else {
        return false;
    };
    let Some(attributed_bytes) = table_charge.checked_add(audited_new_block_bytes) else {
        return false;
    };
    attributed == batch.attribution_len() && batch.attributed_bytes() == Some(attributed_bytes)
}

#[cfg(test)]
fn audit_canonical_source_mapping_is_exact<Block>(
    table: &RetainedBlockTable<Block>,
    destination_set: RetainedSetId,
) -> bool {
    if table.root_membership
        != (RetainedMembership {
            set_id: destination_set,
            destination: RetainedMembershipDestination::Canonical,
            ordinal: 0,
        })
    {
        return false;
    }
    let Some(mapping) = table.canonical_source_mapping.as_ref() else {
        return table
            .members
            .iter()
            .all(|member| member.origin.source_identity().is_none());
    };
    if !mapping.shape_is_exact() {
        return false;
    }
    let Some(source_set) = mapping.source_set else {
        return mapping.entries.is_empty()
            && mapping.mapped_entries == 0
            && table
                .members
                .iter()
                .all(|member| member.origin.source_identity().is_none());
    };
    if source_set == destination_set || mapping.entries.is_empty() {
        return false;
    }

    let mut audited_derived_members = 0_usize;
    for (destination_index, member) in table.members.iter().enumerate() {
        let Some(source_identity) = member.origin.source_identity() else {
            continue;
        };
        let Some(expected_ordinal) = destination_index.checked_add(1).and_then(NonZeroUsize::new)
        else {
            return false;
        };
        if source_identity.set_id != source_set
            || source_identity.index >= mapping.entries.len()
            || mapping.entries[source_identity.index] != Some(expected_ordinal)
        {
            return false;
        }
        let Some(next_derived_members) = audited_derived_members.checked_add(1) else {
            return false;
        };
        audited_derived_members = next_derived_members;
    }

    let mut audited_mapped_entries = 0_usize;
    for (source_index, destination_ordinal) in mapping.entries.iter().copied().enumerate() {
        let Some(destination_ordinal) = destination_ordinal else {
            continue;
        };
        let Some(destination_index) = destination_ordinal.get().checked_sub(1) else {
            return false;
        };
        let Some(member) = table.members.get(destination_index) else {
            return false;
        };
        let Some(source_identity) = member.origin.source_identity() else {
            return false;
        };
        if source_identity.set_id != source_set || source_identity.index != source_index {
            return false;
        }
        let Some(next_mapped_entries) = audited_mapped_entries.checked_add(1) else {
            return false;
        };
        audited_mapped_entries = next_mapped_entries;
    }
    audited_mapped_entries == audited_derived_members
        && mapping.mapped_entries == audited_mapped_entries
}
