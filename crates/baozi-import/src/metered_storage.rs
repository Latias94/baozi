use crate::error::ScopeAbortCause;
use crate::scope::ImportStorageClass;
use crate::store::{ImportStoreHost, ImportStoreReservation};
use baozi_core::{MetadataValue, OrderedMap};
use baozi_operation::{CapacityPlanError, ReplacementCapacityPlan};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::ops::{Deref, DerefMut};

/// Import-owned vector storage coupled to the reservation covering its backing allocation.
///
/// Field order is part of the safety contract: elements and backing storage are dropped before the
/// capacity reservation is released.
#[must_use = "accounted storage must remain coupled until it is dropped or terminally transferred"]
pub(crate) struct AccountedVec<T> {
    values: Vec<T>,
    capacity: Option<ImportStoreReservation>,
}

pub(crate) type AccountedReservationVec = AccountedVec<ImportStoreReservation>;

/// Reservations grouped by the canonical allocation they cover.
#[must_use = "scene value reservations must be coupled to a prepared value or dropped"]
pub(crate) struct SceneValueReservations {
    pub(crate) payload: AccountedReservationVec,
}

/// One canonical scene value coupled to every reservation covering its dynamic storage.
///
/// Field order is part of the contract: the value is dropped before its reservations.
#[must_use = "a prepared scene value must be inserted or dropped with its reservations"]
pub struct PreparedSceneValue<T> {
    pub(crate) value: Option<T>,
    pub(crate) reservations: SceneValueReservations,
}

/// Session-bound vector storage coupled to the reservation for its complete allocation.
///
/// The interface never exposes a mutable `Vec`, so capacity changes cannot bypass the meter.
/// Field order is part of the contract: elements and backing storage drop before the token.
#[must_use = "metered vector storage must remain coupled to its capacity reservation"]
pub struct MeteredVec<'scope, T> {
    pub(crate) values: Vec<T>,
    pub(crate) capacity: Option<ImportStoreReservation>,
    pub(crate) meter: StorageMeter<'scope>,
    pub(crate) class: ImportStorageClass,
    pub(crate) resource: &'static str,
}

/// Session-bound string storage coupled to the reservation for its complete allocation.
#[must_use = "metered string storage must remain coupled to its capacity reservation"]
pub struct MeteredString<'scope> {
    pub(crate) value: String,
    pub(crate) capacity: Option<ImportStoreReservation>,
    pub(crate) meter: StorageMeter<'scope>,
    pub(crate) class: ImportStorageClass,
    pub(crate) resource: &'static str,
}

/// Session-bound ordered-map storage coupled to the reservation for its complete allocation.
#[must_use = "metered map storage must remain coupled to its capacity reservation"]
pub struct MeteredOrderedMap<'scope, K, V> {
    pub(crate) value: OrderedMap<K, V>,
    pub(crate) capacity: Option<ImportStoreReservation>,
    pub(crate) meter: StorageMeter<'scope>,
    pub(crate) class: ImportStorageClass,
    pub(crate) resource: &'static str,
}

/// Decoded text whose logical bytes and allocator slack are accounted independently.
///
/// Field order keeps the backing string alive until both reservations can be released.
#[must_use = "decoded text must remain coupled to its exact and allocator-slack reservations"]
pub struct MeteredDecodedText<'scope> {
    pub(crate) value: String,
    pub(crate) decoded_len: usize,
    pub(crate) decoded: Option<ImportStoreReservation>,
    pub(crate) allocator_slack: Option<ImportStoreReservation>,
    pub(crate) meter: StorageMeter<'scope>,
    pub(crate) resource: &'static str,
}

/// A scoped conservative allowance for opaque backend allocations.
///
/// This owner is deliberately non-transferable to canonical scene storage.
#[must_use = "opaque backend work must retain its scoped allocation allowance"]
pub struct MeteredOpaqueAllowance<'scope> {
    pub(crate) reservation: Option<ImportStoreReservation>,
    pub(crate) scope: std::marker::PhantomData<&'scope ()>,
}

/// A dynamic canonical collection that owns both its outer allocation and every element payload.
///
/// Field order drops values and their outer backing allocation before releasing element payload
/// reservations.
#[must_use = "scene collection storage must be finished into scene value parts"]
pub struct MeteredSceneVec<'scope, T> {
    pub(crate) values: MeteredVec<'scope, T>,
    pub(crate) reservations: SceneValueReservations,
}

/// Canonical metadata storage coupled to each prepared entry's dynamic allocations.
///
/// The map drops every key and value before the parallel entry reservation owners.
#[must_use = "metadata storage must be finished into scene value parts"]
pub struct MeteredMetadataMap<'scope> {
    pub(crate) values: MeteredOrderedMap<'scope, String, MetadataValue>,
    pub(crate) entries: MeteredVec<'scope, SceneValueReservations>,
}

pub(crate) struct RetiredPreparedMetadataEntry {
    pub(crate) value: (String, MetadataValue),
    pub(crate) reservations: SceneValueReservations,
}

impl RetiredPreparedMetadataEntry {
    pub(crate) fn drop_in_order(self) {
        let Self {
            value,
            reservations,
        } = self;
        drop(value);
        drop(reservations);
    }
}

/// Opaque canonical construction parts that retain every storage reservation while composing.
///
/// Raw storage is available only to the terminal build closure and cannot be extracted directly.
#[must_use = "scene value parts must be finished into a prepared canonical value"]
pub struct SceneValueParts<'scope, T> {
    pub(crate) value: Option<T>,
    pub(crate) reservations: SceneValueReservations,
    pub(crate) meter: StorageMeter<'scope>,
}

/// Minimal session capability retained by owning metered containers.
#[derive(Clone, Copy)]
pub(crate) struct StorageMeter<'scope> {
    pub(crate) host: ImportStoreHost<'scope>,
    pub(crate) callback_active: &'scope Cell<bool>,
    pub(crate) terminal: &'scope RefCell<Option<ScopeAbortCause>>,
}

impl SceneValueReservations {
    /// Creates an empty canonical reservation bundle.
    pub const fn new() -> Self {
        Self {
            payload: AccountedReservationVec::new(),
        }
    }
}

impl Default for SceneValueReservations {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PreparedSceneValue<T> {
    pub(crate) fn take_value(&mut self) -> T {
        self.value
            .take()
            .expect("a prepared scene value is consumed exactly once")
    }

    pub(crate) fn take_reservations(&mut self) -> SceneValueReservations {
        std::mem::take(&mut self.reservations)
    }
}

impl<T> MeteredVec<'_, T> {
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn capacity(&self) -> usize {
        self.values.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.values
    }

    /// Provides mutable element access only for `Copy` values, which cannot own drop-managed
    /// backing storage.
    pub fn as_mut_slice(&mut self) -> &mut [T]
    where
        T: Copy,
    {
        &mut self.values
    }

    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }

    pub fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        self.values.swap_remove(index)
    }

    pub fn retain(&mut self, predicate: impl FnMut(&T) -> bool) {
        self.values.retain(predicate);
    }

    pub fn sort_unstable_by(&mut self, compare: impl FnMut(&T, &T) -> std::cmp::Ordering) {
        self.values.sort_unstable_by(compare);
    }
}

impl MeteredString<'_> {
    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn len(&self) -> usize {
        self.value.len()
    }

    pub fn capacity(&self) -> usize {
        self.value.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn truncate(&mut self, len: usize) {
        self.value.truncate(len);
    }
}

impl MeteredDecodedText<'_> {
    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn len(&self) -> usize {
        self.value.len()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn decoded_bytes(&self) -> u64 {
        self.decoded
            .as_ref()
            .map_or(0, ImportStoreReservation::bytes)
    }

    pub fn allocator_slack_bytes(&self) -> u64 {
        self.allocator_slack
            .as_ref()
            .map_or(0, ImportStoreReservation::bytes)
    }
}

impl MeteredOpaqueAllowance<'_> {
    pub fn bytes(&self) -> u64 {
        self.reservation
            .as_ref()
            .map_or(0, ImportStoreReservation::bytes)
    }
}

impl<T> MeteredSceneVec<'_, T> {
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn as_slice(&self) -> &[T] {
        self.values.as_slice()
    }
}

impl MeteredMetadataMap<'_> {
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&MetadataValue> {
        self.values.value.get(key)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&String, &MetadataValue)> {
        self.values.iter()
    }
}

impl<K, V> MeteredOrderedMap<'_, K, V> {
    pub fn len(&self) -> usize {
        self.value.len()
    }

    pub fn capacity(&self) -> usize {
        self.value.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&K, &V)> {
        self.value.iter()
    }
}

impl<K: Ord, V> MeteredOrderedMap<'_, K, V> {
    pub fn get(&self, key: &K) -> Option<&V> {
        self.value.get(key)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.value.contains_key(key)
    }
}

impl<'scope, T> SceneValueParts<'scope, T> {
    pub(crate) fn map_owned<U>(mut self, build: impl FnOnce(T) -> U) -> SceneValueParts<'scope, U> {
        let value = self
            .value
            .take()
            .expect("scene value parts are transformed exactly once");
        SceneValueParts {
            value: Some(build(value)),
            reservations: self.reservations,
            meter: self.meter,
        }
    }
}

/// A fully admitted replacement that cannot mutate the live owner before commit.
#[must_use = "commit or drop the candidate without separating its allocation from its reservation"]
pub(crate) struct AccountedVecCandidate<'owner, T> {
    owner: &'owner mut AccountedVec<T>,
    replacement: AccountedVec<T>,
}

/// Storage whose operation-owned capacity has been transferred into a terminal outcome draft.
#[must_use = "terminal storage must be moved into an import outcome or dropped"]
pub(crate) struct TerminalVec<T> {
    values: Vec<T>,
}

impl<T> AccountedVec<T> {
    pub(crate) const fn new() -> Self {
        Self {
            values: Vec::new(),
            capacity: None,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    pub(crate) fn capacity(&self) -> usize {
        self.values.capacity()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub(crate) fn has_slots(&self, additional: usize) -> bool {
        self.values
            .len()
            .checked_add(additional)
            .is_some_and(|required| required <= self.values.capacity())
    }

    pub(crate) fn capacity_bytes(&self) -> u64 {
        self.capacity
            .as_ref()
            .map_or(0, ImportStoreReservation::bytes)
    }

    pub(crate) fn prepare_replacement(
        &mut self,
        additional: usize,
        resource: &'static str,
        reserve: impl FnOnce(u64) -> Result<ImportStoreReservation, ScopeAbortCause>,
    ) -> Result<Option<AccountedVecCandidate<'_, T>>, ScopeAbortCause> {
        let required_capacity =
            self.values.len().checked_add(additional).ok_or_else(|| {
                capacity_plan_cause(resource, CapacityPlanError::CapacityOverflow)
            })?;
        self.prepare_replacement_capacity(required_capacity, resource, reserve)
    }

    pub(crate) fn prepare_replacement_capacity(
        &mut self,
        required_capacity: usize,
        resource: &'static str,
        reserve: impl FnOnce(u64) -> Result<ImportStoreReservation, ScopeAbortCause>,
    ) -> Result<Option<AccountedVecCandidate<'_, T>>, ScopeAbortCause> {
        if required_capacity <= self.values.capacity() {
            return Ok(None);
        }

        let replacement_plan = ReplacementCapacityPlan::for_type::<T>(
            self.values.len(),
            self.values.capacity(),
            required_capacity,
        )
        .map_err(|cause| capacity_plan_cause(resource, cause))?;
        let admitted = replacement_plan
            .candidate_envelope_bytes()
            .map_err(|cause| capacity_plan_cause(resource, cause))?;
        let capacity = if admitted == 0 {
            None
        } else {
            let capacity = reserve(admitted)?;
            if capacity.bytes() < admitted {
                return Err(ScopeAbortCause::DescriptorContract {
                    code: "accounted-capacity-reservation-under-covered",
                });
            }
            Some(capacity)
        };
        let mut replacement = Self {
            values: Vec::new(),
            capacity,
        };

        replacement_plan
            .verify_source(self.values.len(), self.values.capacity())
            .map_err(|cause| capacity_plan_cause(resource, cause))?;
        let candidate_plan = replacement_plan.candidate();
        candidate_plan
            .verify_preflight(replacement.values.len(), replacement.values.capacity())
            .map_err(|cause| capacity_plan_cause(resource, cause))?;
        if fail_next_allocation() {
            return Err(ScopeAbortCause::Allocation { resource });
        }
        replacement
            .values
            .try_reserve_exact(candidate_plan.allocator_additional())
            .map_err(|_| ScopeAbortCause::Allocation { resource })?;
        if fail_next_qualification() {
            return Err(capacity_plan_cause(
                resource,
                CapacityPlanError::CapacityEnvelopeViolated {
                    policy_revision: baozi_operation::CAPACITY_POLICY_REVISION,
                    desired_capacity: candidate_plan.desired_capacity(),
                    envelope_capacity: candidate_plan.envelope_capacity(),
                    actual_capacity: replacement.values.capacity(),
                },
            ));
        }
        candidate_plan
            .finish(replacement.values.len(), replacement.values.capacity())
            .map_err(|cause| capacity_plan_cause(resource, cause))?;

        Ok(Some(AccountedVecCandidate {
            owner: self,
            replacement,
        }))
    }

    pub(crate) fn try_reserve(
        &mut self,
        additional: usize,
        resource: &'static str,
        reserve: impl FnOnce(u64) -> Result<ImportStoreReservation, ScopeAbortCause>,
    ) -> Result<(), ScopeAbortCause> {
        if let Some(candidate) = self.prepare_replacement(additional, resource, reserve)? {
            candidate.commit();
        }
        Ok(())
    }

    pub(crate) fn try_reserve_capacity(
        &mut self,
        required_capacity: usize,
        resource: &'static str,
        reserve: impl FnOnce(u64) -> Result<ImportStoreReservation, ScopeAbortCause>,
    ) -> Result<(), ScopeAbortCause> {
        if let Some(candidate) =
            self.prepare_replacement_capacity(required_capacity, resource, reserve)?
        {
            candidate.commit();
        }
        Ok(())
    }

    pub(crate) fn try_push_reserved(&mut self, value: T) -> Result<(), T> {
        if !self.has_slots(1) {
            return Err(value);
        }
        self.values.push(value);
        Ok(())
    }

    pub(crate) fn push_reserved_infallible(&mut self, value: T) {
        assert!(
            self.has_slots(1),
            "a publication token may move only after its destination slot is admitted"
        );
        self.values.push(value);
    }

    pub(crate) fn try_insert_reserved(&mut self, index: usize, value: T) -> Result<(), T> {
        if index > self.values.len() || !self.has_slots(1) {
            return Err(value);
        }
        self.values.insert(index, value);
        Ok(())
    }

    pub(crate) fn try_extend_from_slice_reserved(&mut self, source: &[T]) -> Result<(), ()>
    where
        T: Copy,
    {
        if !self.has_slots(source.len()) {
            return Err(());
        }
        self.values.extend_from_slice(source);
        Ok(())
    }

    pub(crate) fn try_append_accounted(&mut self, source: &mut Self) -> Result<(), ()> {
        if !self.has_slots(source.len()) {
            return Err(());
        }
        self.values.append(&mut source.values);
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }

    pub(crate) fn retain(&mut self, predicate: impl FnMut(&T) -> bool) {
        self.values.retain(predicate);
    }

    pub(crate) fn sort(&mut self)
    where
        T: Ord,
    {
        self.values.sort();
    }

    pub(crate) fn dedup(&mut self)
    where
        T: PartialEq,
    {
        self.values.dedup();
    }

    pub(crate) fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    pub(crate) fn into_terminal(self) -> TerminalVec<T> {
        let Self { values, capacity } = self;
        let terminal = TerminalVec { values };
        drop(capacity);
        terminal
    }
}

impl<T> TerminalVec<T> {
    pub(crate) fn into_values(self) -> Vec<T> {
        self.values
    }
}

impl<T> AccountedVecCandidate<'_, T> {
    /// Atomically replaces the owner without allocator or ledger entry.
    pub(crate) fn commit(mut self) {
        assert!(
            self.replacement.has_slots(self.owner.len()),
            "an admitted replacement must hold every live element without allocation"
        );
        self.replacement.values.append(&mut self.owner.values);
        std::mem::swap(self.owner, &mut self.replacement);
    }
}

impl<T> Default for AccountedVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Deref for AccountedVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<T> DerefMut for AccountedVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}

impl<T: fmt::Debug> fmt::Debug for AccountedVec<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccountedVec")
            .field("values", &self.values)
            .field("capacity_bytes", &self.capacity_bytes())
            .finish()
    }
}

pub(crate) fn capacity_plan_cause(
    resource: &'static str,
    cause: CapacityPlanError,
) -> ScopeAbortCause {
    match cause {
        CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
            ScopeAbortCause::ResourceLimit {
                resource,
                limit: u64::MAX,
            }
        }
        CapacityPlanError::InvalidState { .. } | CapacityPlanError::StalePlan { .. } => {
            ScopeAbortCause::DescriptorContract {
                code: "capacity-plan-contract-failed",
            }
        }
        cause @ CapacityPlanError::CapacityEnvelopeViolated { .. } => {
            ScopeAbortCause::CapacityEnvelope { resource, cause }
        }
        _ => ScopeAbortCause::DescriptorContract {
            code: "capacity-plan-error-unknown",
        },
    }
}

#[cfg(test)]
std::thread_local! {
    static FAIL_NEXT_ALLOCATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NEXT_QUALIFICATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn fail_next_allocation() -> bool {
    FAIL_NEXT_ALLOCATION.with(|fault| fault.replace(false))
}

#[cfg(test)]
pub(crate) fn inject_next_accounted_allocation_failure() {
    FAIL_NEXT_ALLOCATION.with(|fault| fault.set(true));
}

#[cfg(not(test))]
const fn fail_next_allocation() -> bool {
    false
}

#[cfg(test)]
fn fail_next_qualification() -> bool {
    FAIL_NEXT_QUALIFICATION.with(|fault| fault.replace(false))
}

#[cfg(not(test))]
const fn fail_next_qualification() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{ImportDimension, ImportLedgerPhase, ImportOperationControl};
    use crate::limits::ResourceLimits;
    use crate::store::{
        ImportStoreBootstrapRequirements, ImportStoreSessionOwner, try_import_store_bootstrap,
    };
    use baozi_operation::{
        PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits,
    };
    use std::cell::RefCell;

    struct StoreHarness {
        owner: ImportStoreSessionOwner,
    }

    impl StoreHarness {
        fn new() -> Self {
            let limits = ResourceLimits::default();
            let requirements = ImportStoreBootstrapRequirements::for_node_scene(&limits)
                .expect("test retained-store requirements are valid");
            let bootstrap = try_import_store_bootstrap(&limits, requirements)
                .expect("test import store bootstrap is valid");
            let factory = PhysicalStorageAccountFactory::try_new(
                PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
                    .expect("test factory limits are valid"),
            )
            .expect("test account factory must allocate");
            let account = factory
                .try_create_account(
                    PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX)
                        .expect("test account limits are valid"),
                )
                .expect("test physical account must allocate");
            let owner = ImportStoreSessionOwner::try_start(
                ImportOperationControl::default(),
                &account,
                bootstrap,
            )
            .expect("test import store owner must start");
            drop(account);
            drop(factory);
            Self { owner }
        }

        fn host(&self) -> ImportStoreHost<'_> {
            self.owner.host()
        }

        fn reserve(
            &self,
            phase: ImportLedgerPhase,
            dimension: ImportDimension,
            bytes: u64,
        ) -> Result<ImportStoreReservation, ScopeAbortCause> {
            self.host()
                .reserve(phase, dimension, bytes)
                .map_err(|cause| cause.into_scope_abort_cause())
        }

        fn reserve_scratch(&self, bytes: u64) -> Result<ImportStoreReservation, ScopeAbortCause> {
            self.reserve(
                ImportLedgerPhase::Detection,
                ImportDimension::ScratchBytes,
                bytes,
            )
        }

        fn live_bytes(&self, dimension: ImportDimension) -> u64 {
            self.host()
                .dimension_state(dimension)
                .expect("test import store dimension must remain observable")
                .live_bytes()
        }
    }

    struct DropProbe<'a> {
        host: ImportStoreHost<'a>,
        dimension: ImportDimension,
    }

    impl Drop for DropProbe<'_> {
        fn drop(&mut self) {
            assert!(
                self.host
                    .dimension_state(self.dimension)
                    .expect("drop-order dimension state")
                    .live_bytes()
                    > 0,
                "element storage must drop before its capacity reservation"
            );
        }
    }

    std::thread_local! {
        static SCENE_COLLECTION_MERGE_OBSERVED_LIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    struct SceneCollectionMergeDropProbe;

    impl Drop for SceneCollectionMergeDropProbe {
        fn drop(&mut self) {
            SCENE_COLLECTION_MERGE_OBSERVED_LIVE.with(|observed| observed.set(true));
        }
    }

    #[test]
    fn owner_drops_elements_before_releasing_capacity() {
        let store = StoreHarness::new();
        let mut values = AccountedVec::new();
        values
            .try_reserve(1, "drop-order", |bytes| store.reserve_scratch(bytes))
            .expect("capacity admission");
        values
            .try_push_reserved(DropProbe {
                host: store.host(),
                dimension: ImportDimension::ScratchBytes,
            })
            .unwrap_or_else(|_| panic!("reserved slot must accept the probe"));

        drop(values);
        assert_eq!(store.live_bytes(ImportDimension::ScratchBytes), 0);
    }

    #[test]
    fn metered_vector_drops_elements_before_releasing_its_complete_token() {
        let store = StoreHarness::new();
        let callback_active = Cell::new(false);
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            host: store.host(),
            callback_active: &callback_active,
            terminal: &terminal,
        };
        let mut values = MeteredVec {
            values: Vec::new(),
            capacity: None,
            meter,
            class: ImportStorageClass::SceneConstruction,
            resource: "metered-drop-order",
        };
        values
            .try_push(DropProbe {
                host: store.host(),
                dimension: ImportDimension::CanonicalOutputBytes,
            })
            .expect("metered capacity admission");
        let expected = u64::try_from(
            values
                .capacity()
                .checked_mul(std::mem::size_of::<DropProbe<'_>>())
                .expect("test capacity bytes"),
        )
        .expect("test capacity fits u64");
        assert_eq!(
            values.capacity.as_ref().map(ImportStoreReservation::bytes),
            Some(expected)
        );

        drop(values);
        assert_eq!(store.live_bytes(ImportDimension::CanonicalOutputBytes), 0);
    }

    #[test]
    fn scene_collection_final_merge_failure_releases_element_and_payload_owners() {
        let store = StoreHarness::new();
        let callback_active = Cell::new(false);
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            host: store.host(),
            callback_active: &callback_active,
            terminal: &terminal,
        };
        let mut values = MeteredVec {
            values: Vec::new(),
            capacity: None,
            meter,
            class: ImportStorageClass::SceneConstruction,
            resource: "scene-collection-merge-values",
        };
        values
            .try_push(SceneCollectionMergeDropProbe)
            .expect("zero-sized scene payload admission");
        assert!(
            values.capacity.is_none(),
            "the zero-sized probe must leave the final merge as the next accounted allocation"
        );

        let mut reservations = SceneValueReservations::new();
        reservations
            .payload
            .try_reserve(1, "scene-collection-element-owner", |bytes| {
                store.reserve_scratch(bytes)
            })
            .expect("element reservation inventory capacity");
        let payload = store
            .reserve(
                ImportLedgerPhase::BackendDecode,
                ImportDimension::DecodedTextBytes,
                13,
            )
            .expect("element payload reservation");
        reservations
            .payload
            .try_push_reserved(payload)
            .unwrap_or_else(|_| panic!("element reservation inventory slot"));
        let values = MeteredSceneVec {
            values,
            reservations,
        };

        SCENE_COLLECTION_MERGE_OBSERVED_LIVE.with(|observed| observed.set(false));
        FAIL_NEXT_ALLOCATION.with(|fault| fault.set(true));

        let result = values.try_into_scene_parts();

        assert!(result.is_err(), "the final reservation merge must fail");
        assert!(matches!(
            terminal.borrow().as_ref(),
            Some(ScopeAbortCause::Allocation {
                resource: "scene-collection-final-reservations"
            })
        ));
        assert!(
            SCENE_COLLECTION_MERGE_OBSERVED_LIVE.with(std::cell::Cell::get),
            "the scene element must be dropped on final reservation merge failure"
        );
        assert_eq!(store.live_bytes(ImportDimension::DecodedTextBytes), 0);
        assert_eq!(store.live_bytes(ImportDimension::ScratchBytes), 0);
    }

    #[test]
    fn rejected_prepared_value_drops_value_before_releasing_its_reservations() {
        fn reject<T>(_prepared: PreparedSceneValue<T>) -> Result<(), ()> {
            Err(())
        }

        let store = StoreHarness::new();
        let mut reservations = SceneValueReservations::new();
        reservations
            .payload
            .try_reserve(1, "prepared-drop-order", |bytes| {
                store.reserve_scratch(bytes)
            })
            .expect("reservation inventory capacity admission");
        reservations
            .payload
            .try_push_reserved(
                store
                    .reserve_scratch(1)
                    .expect("prepared payload reservation admission"),
            )
            .unwrap_or_else(|_| panic!("reserved inventory slot must accept the payload token"));
        let prepared = PreparedSceneValue {
            value: Some(DropProbe {
                host: store.host(),
                dimension: ImportDimension::ScratchBytes,
            }),
            reservations,
        };

        assert_eq!(reject(prepared), Err(()));
        assert_eq!(store.live_bytes(ImportDimension::ScratchBytes), 0);
    }

    #[test]
    fn failed_candidates_preserve_the_live_owner_and_release_candidate_capacity() {
        let store = StoreHarness::new();
        let mut values = AccountedVec::new();
        values
            .try_reserve(1, "candidate", |bytes| store.reserve_scratch(bytes))
            .expect("initial capacity");
        values
            .try_push_reserved(7_u64)
            .unwrap_or_else(|_| panic!("initial slot"));
        let original_capacity = values.capacity();
        let original_bytes = values.capacity_bytes();
        let original_live = store.live_bytes(ImportDimension::ScratchBytes);

        FAIL_NEXT_ALLOCATION.with(|fault| fault.set(true));
        assert!(matches!(
            values.try_reserve(1, "candidate", |bytes| store.reserve_scratch(bytes)),
            Err(ScopeAbortCause::Allocation {
                resource: "candidate"
            })
        ));
        assert_eq!(&*values, &[7]);
        assert_eq!(values.capacity(), original_capacity);
        assert_eq!(values.capacity_bytes(), original_bytes);
        assert_eq!(
            store.live_bytes(ImportDimension::ScratchBytes),
            original_live
        );

        FAIL_NEXT_QUALIFICATION.with(|fault| fault.set(true));
        assert!(matches!(
            values.try_reserve(1, "candidate", |bytes| store.reserve_scratch(bytes)),
            Err(ScopeAbortCause::CapacityEnvelope {
                resource: "candidate",
                cause: CapacityPlanError::CapacityEnvelopeViolated { .. },
            })
        ));
        assert_eq!(&*values, &[7]);
        assert_eq!(values.capacity(), original_capacity);
        assert_eq!(values.capacity_bytes(), original_bytes);
        assert_eq!(
            store.live_bytes(ImportDimension::ScratchBytes),
            original_live
        );
    }

    #[test]
    fn undercovered_capacity_reservation_is_rejected_before_allocation() {
        let store = StoreHarness::new();
        let mut values = AccountedVec::<u64>::new();

        let result = values.try_reserve(1, "undercovered-capacity", |bytes| {
            assert!(bytes > 0, "a non-ZST vector must require capacity coverage");
            store.reserve_scratch(bytes - 1)
        });

        assert!(matches!(
            result,
            Err(ScopeAbortCause::DescriptorContract {
                code: "accounted-capacity-reservation-under-covered"
            })
        ));
        assert!(values.is_empty());
        assert_eq!(values.capacity(), 0);
        assert_eq!(store.live_bytes(ImportDimension::ScratchBytes), 0);
    }

    #[test]
    fn failed_reservation_preserves_the_live_owner() {
        let store = StoreHarness::new();
        let mut values = AccountedVec::new();
        values
            .try_reserve(1, "reserve-failure", |bytes| store.reserve_scratch(bytes))
            .expect("initial capacity");
        values
            .try_push_reserved(5_u64)
            .unwrap_or_else(|_| panic!("initial slot"));
        let original_capacity = values.capacity();
        let original_live = store.live_bytes(ImportDimension::ScratchBytes);

        assert!(matches!(
            values.try_reserve(1, "reserve-failure", |_| {
                Err(ScopeAbortCause::Allocation {
                    resource: "reserve-failure",
                })
            }),
            Err(ScopeAbortCause::Allocation {
                resource: "reserve-failure"
            })
        ));
        assert_eq!(&*values, &[5]);
        assert_eq!(values.capacity(), original_capacity);
        assert_eq!(
            store.live_bytes(ImportDimension::ScratchBytes),
            original_live
        );
    }

    #[test]
    fn successful_replacement_accounts_overlap_then_retires_the_old_owner() {
        let store = StoreHarness::new();
        let mut values = AccountedVec::new();
        values
            .try_reserve(1, "replacement", |bytes| store.reserve_scratch(bytes))
            .expect("initial capacity");
        values
            .try_push_reserved(1_u64)
            .unwrap_or_else(|_| panic!("initial slot"));
        let old_bytes = values.capacity_bytes();

        let candidate = values
            .prepare_replacement(1, "replacement", |bytes| store.reserve_scratch(bytes))
            .expect("candidate preparation")
            .expect("growth requires a candidate");
        assert!(store.live_bytes(ImportDimension::ScratchBytes) > old_bytes);

        candidate.commit();
        assert_eq!(&*values, &[1]);
        assert_eq!(
            store.live_bytes(ImportDimension::ScratchBytes),
            values.capacity_bytes()
        );
    }

    #[test]
    fn abandoned_replacement_preserves_the_live_owner() {
        let store = StoreHarness::new();
        let mut values = AccountedVec::new();
        values
            .try_reserve(1, "abandoned-replacement", |bytes| {
                store.reserve_scratch(bytes)
            })
            .expect("initial capacity");
        values
            .try_push_reserved(11_u64)
            .unwrap_or_else(|_| panic!("initial slot"));
        let original_capacity = values.capacity();
        let original_bytes = values.capacity_bytes();
        let original_live = store.live_bytes(ImportDimension::ScratchBytes);

        let candidate = values
            .prepare_replacement(1, "abandoned-replacement", |bytes| {
                store.reserve_scratch(bytes)
            })
            .expect("candidate preparation")
            .expect("growth requires a candidate");
        assert!(store.live_bytes(ImportDimension::ScratchBytes) > original_live);
        drop(candidate);

        assert_eq!(&*values, &[11]);
        assert_eq!(values.capacity(), original_capacity);
        assert_eq!(values.capacity_bytes(), original_bytes);
        assert_eq!(
            store.live_bytes(ImportDimension::ScratchBytes),
            original_live
        );
    }

    #[test]
    fn terminal_type_owns_values_before_capacity_is_released() {
        struct TerminalValue<'a> {
            alive: &'a std::cell::Cell<bool>,
        }

        impl Drop for TerminalValue<'_> {
            fn drop(&mut self) {
                self.alive.set(false);
            }
        }

        let store = StoreHarness::new();
        let alive = std::cell::Cell::new(true);
        let mut values = AccountedVec::new();
        values
            .try_reserve(1, "terminal-owner", |bytes| store.reserve_scratch(bytes))
            .expect("terminal capacity");
        values
            .try_push_reserved(TerminalValue { alive: &alive })
            .unwrap_or_else(|_| panic!("reserved terminal slot"));

        let terminal = values.into_terminal();
        assert_eq!(terminal.values.len(), 1);
        assert!(alive.get());
        assert_eq!(store.live_bytes(ImportDimension::ScratchBytes), 0);

        drop(terminal);
        assert!(!alive.get());
    }
}
