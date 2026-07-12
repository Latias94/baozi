use crate::diagnostic::DiagnosticSource;
use crate::error::{CauseOwner, ScopeAbortCause};
use crate::ledger::ImportReservation;
use baozi_io::{AssetIo, AssetPath, AuthorityDomain};
use baozi_operation::{CapacityPlan, CapacityPlanError};
use std::collections::hash_map::RandomState;
use std::error::Error;
use std::fmt;
use std::hash::{BuildHasher, Hash};
use std::marker::PhantomData;
use std::sync::Arc;

pub const MAX_DEPENDENCY_ROLE_ID_BYTES: usize = 64;

/// A stable descriptor-owned dependency role identifier.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DependencyRoleId([u8; MAX_DEPENDENCY_ROLE_ID_BYTES]);

impl DependencyRoleId {
    pub fn new(value: &str) -> Result<Self, DependencyRoleIdError> {
        validate_role_id(value)?;
        let mut bytes = [0; MAX_DEPENDENCY_ROLE_ID_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self(bytes))
    }

    pub fn as_str(&self) -> &str {
        let len = self
            .0
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(MAX_DEPENDENCY_ROLE_ID_BYTES);
        std::str::from_utf8(&self.0[..len])
            .expect("DependencyRoleId construction accepts only ASCII bytes")
    }
}

impl fmt::Debug for DependencyRoleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DependencyRoleId")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for DependencyRoleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyRoleIdError {
    Empty,
    TooLong,
    InvalidGrammar,
}

impl fmt::Display for DependencyRoleIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("dependency role id is empty"),
            Self::TooLong => formatter.write_str("dependency role id exceeds its hard limit"),
            Self::InvalidGrammar => formatter
                .write_str("dependency role id must be lowercase ASCII segments separated by dots"),
        }
    }
}

impl Error for DependencyRoleIdError {}

fn validate_role_id(value: &str) -> Result<(), DependencyRoleIdError> {
    if value.is_empty() {
        return Err(DependencyRoleIdError::Empty);
    }
    if value.len() > MAX_DEPENDENCY_ROLE_ID_BYTES {
        return Err(DependencyRoleIdError::TooLong);
    }
    if value.split('.').any(|segment| {
        segment.is_empty()
            || segment.starts_with('-')
            || segment.ends_with('-')
            || segment
                .bytes()
                .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    }) {
        return Err(DependencyRoleIdError::InvalidGrammar);
    }
    Ok(())
}

/// Requiredness is frozen by the selected format descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyRequirement {
    Required,
    Optional,
}

/// A bounded logical reference retaining its caller-visible spelling and its
/// canonical path. Policy evaluates this value before cache lookup.
#[derive(Debug, Clone, Copy)]
pub struct DependencyReference<'a> {
    raw: &'a str,
    normalized: &'a AssetPath,
}

impl<'a> DependencyReference<'a> {
    pub(crate) const fn new(raw: &'a str, normalized: &'a AssetPath) -> Self {
        Self { raw, normalized }
    }

    pub const fn raw(&self) -> &'a str {
        self.raw
    }

    pub const fn normalized(&self) -> &'a AssetPath {
        self.normalized
    }
}

/// A normalized resource identity qualified by the backend authority namespace.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct DependencyIdentity {
    authority: AuthorityDomain,
    path: AssetPath,
}

impl DependencyIdentity {
    pub(crate) fn new(authority: AuthorityDomain, path: AssetPath) -> Self {
        Self { authority, path }
    }

    pub fn path(&self) -> &AssetPath {
        &self.path
    }

    pub(crate) fn authority(&self) -> &AuthorityDomain {
        &self.authority
    }

    pub(crate) fn try_clone(&self) -> Result<Self, DependencyPolicyError> {
        Ok(Self {
            authority: self.authority.clone(),
            path: self
                .path
                .try_clone()
                .map_err(|_| DependencyPolicyError::AllocationFailed)?,
        })
    }
}

/// A resolver decision that binds authorization to the backend used for open.
pub struct AuthorizedDependency<'a> {
    io: &'a dyn AssetIo,
    identity: DependencyIdentity,
}

impl<'a> AuthorizedDependency<'a> {
    pub fn new(io: &'a dyn AssetIo, authority: AuthorityDomain, path: AssetPath) -> Self {
        let identity = DependencyIdentity::new(authority, path);
        Self { io, identity }
    }

    pub fn identity(&self) -> &DependencyIdentity {
        &self.identity
    }

    pub(crate) fn into_parts(self) -> (&'a dyn AssetIo, DependencyIdentity) {
        (self.io, self.identity)
    }
}

/// Authorizes a normalized logical dependency before cache lookup or backend IO.
pub trait DependencyResolver: Send + Sync {
    fn authorize<'a>(
        &'a self,
        primary: &'a dyn AssetIo,
        primary_authority: &AuthorityDomain,
        primary_path: &AssetPath,
        role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError>;
}

/// Restricts dependencies to the primary backend's authority namespace.
#[derive(Debug, Default)]
pub struct SameAuthorityResolver;

impl DependencyResolver for SameAuthorityResolver {
    fn authorize<'a>(
        &'a self,
        primary: &'a dyn AssetIo,
        primary_authority: &AuthorityDomain,
        _primary_path: &AssetPath,
        _role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        Ok(AuthorizedDependency::new(
            primary,
            primary_authority.clone(),
            reference
                .normalized()
                .try_clone()
                .map_err(|_| DependencyPolicyError::AllocationFailed)?,
        ))
    }
}

/// Denies every external dependency before backend IO.
#[derive(Debug, Default)]
pub struct DenyDependencies;

impl DependencyResolver for DenyDependencies {
    fn authorize<'a>(
        &'a self,
        _primary: &'a dyn AssetIo,
        _primary_authority: &AuthorityDomain,
        _primary_path: &AssetPath,
        _role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError> {
        Err(DependencyPolicyError::Denied {
            path: reference
                .normalized()
                .try_clone()
                .map_err(|_| DependencyPolicyError::AllocationFailed)?,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DependencyPolicyError {
    Denied { path: AssetPath },
    AllocationFailed,
    ResolverContract { code: &'static str },
}

impl fmt::Display for DependencyPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied { path } => write!(formatter, "dependency {path} is denied by policy"),
            Self::AllocationFailed => {
                formatter.write_str("dependency policy evidence allocation failed")
            }
            Self::ResolverContract { code } => {
                write!(formatter, "dependency resolver contract failed: {code}")
            }
        }
    }
}

impl Error for DependencyPolicyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyCacheDisposition {
    Fresh,
    Reused,
}

/// Immutable bytes borrowed from one import session's dependency cache.
pub struct DependencySnapshot<'scope> {
    identity: DependencyIdentity,
    bytes: Arc<Vec<u8>>,
    cache_disposition: DependencyCacheDisposition,
    _identity_path_reservation: ImportReservation,
    scope: PhantomData<&'scope ()>,
}

impl<'scope> DependencySnapshot<'scope> {
    pub(crate) fn new(
        identity: DependencyIdentity,
        bytes: Arc<Vec<u8>>,
        cache_disposition: DependencyCacheDisposition,
        identity_path_reservation: ImportReservation,
    ) -> Self {
        Self {
            identity,
            bytes,
            cache_disposition,
            _identity_path_reservation: identity_path_reservation,
            scope: PhantomData,
        }
    }

    pub fn identity(&self) -> &DependencyIdentity {
        &self.identity
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn cache_disposition(&self) -> DependencyCacheDisposition {
        self.cache_disposition
    }

    pub fn diagnostic_source(&self) -> DiagnosticSource<'_> {
        DiagnosticSource::new(
            self.identity.authority().clone(),
            self.identity.path(),
            self.bytes(),
        )
    }
}

#[derive(Debug)]
pub struct OptionalDependencyEvidence {
    identity: Option<DependencyIdentity>,
    normalized_path: AssetPath,
    owner: CauseOwner,
    cache_disposition: DependencyCacheDisposition,
    _identity_path_reservation: Option<ImportReservation>,
    _normalized_path_reservation: Option<ImportReservation>,
    _container_capacity_reservation: Option<ImportReservation>,
}

impl OptionalDependencyEvidence {
    pub(crate) fn new(
        identity: Option<DependencyIdentity>,
        normalized_path: AssetPath,
        owner: CauseOwner,
        cache_disposition: DependencyCacheDisposition,
        identity_path_reservation: Option<ImportReservation>,
        normalized_path_reservation: ImportReservation,
    ) -> Self {
        Self {
            identity,
            normalized_path,
            owner,
            cache_disposition,
            _identity_path_reservation: identity_path_reservation,
            _normalized_path_reservation: Some(normalized_path_reservation),
            _container_capacity_reservation: None,
        }
    }

    pub(crate) fn retain_container_capacity(&mut self, reservation: ImportReservation) {
        self._container_capacity_reservation = Some(reservation);
    }

    pub(crate) fn reservation_count(&self) -> usize {
        usize::from(self._identity_path_reservation.is_some())
            + usize::from(self._normalized_path_reservation.is_some())
            + usize::from(self._container_capacity_reservation.is_some())
    }

    pub(crate) fn drain_reservations(&mut self, target: &mut Vec<ImportReservation>) {
        if let Some(reservation) = self._identity_path_reservation.take() {
            target.push(reservation);
        }
        if let Some(reservation) = self._normalized_path_reservation.take() {
            target.push(reservation);
        }
        if let Some(reservation) = self._container_capacity_reservation.take() {
            target.push(reservation);
        }
    }

    pub(crate) fn release_reservations(&mut self) {
        drop(self._identity_path_reservation.take());
        drop(self._normalized_path_reservation.take());
        drop(self._container_capacity_reservation.take());
    }

    pub fn identity(&self) -> Option<&DependencyIdentity> {
        self.identity.as_ref()
    }

    pub fn normalized_path(&self) -> &AssetPath {
        &self.normalized_path
    }

    pub fn owner(&self) -> &CauseOwner {
        &self.owner
    }

    pub const fn cache_disposition(&self) -> DependencyCacheDisposition {
        self.cache_disposition
    }
}

pub struct OptionalDependencyOutcome<'scope> {
    inner: OptionalDependencyOutcomeKind<'scope>,
}

enum OptionalDependencyOutcomeKind<'scope> {
    Available(DependencySnapshot<'scope>),
    Unavailable(OptionalDependencyEvidence),
}

impl<'scope> OptionalDependencyOutcome<'scope> {
    pub(crate) fn available(snapshot: DependencySnapshot<'scope>) -> Self {
        Self {
            inner: OptionalDependencyOutcomeKind::Available(snapshot),
        }
    }

    pub(crate) fn unavailable(evidence: OptionalDependencyEvidence) -> Self {
        Self {
            inner: OptionalDependencyOutcomeKind::Unavailable(evidence),
        }
    }

    pub fn snapshot(&self) -> Option<&DependencySnapshot<'scope>> {
        match &self.inner {
            OptionalDependencyOutcomeKind::Available(snapshot) => Some(snapshot),
            OptionalDependencyOutcomeKind::Unavailable(_) => None,
        }
    }

    pub fn unavailable_evidence(&self) -> Option<&OptionalDependencyEvidence> {
        match &self.inner {
            OptionalDependencyOutcomeKind::Available(_) => None,
            OptionalDependencyOutcomeKind::Unavailable(evidence) => Some(evidence),
        }
    }
}

const DEPENDENCY_INDEX_EMPTY: usize = usize::MAX;
const DEPENDENCY_INDEX_TOMBSTONE: usize = usize::MAX - 1;
const MIN_DEPENDENCY_INDEX_SLOTS: usize = 8;

pub(crate) struct DependencyCache {
    entries: Vec<DependencyCacheRecordSlot>,
    entry_count: usize,
    free_entry: Option<usize>,
    entry_capacity_reservation: Option<ImportReservation>,
    index: DependencyIndex,
}

impl DependencyCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
            entry_count: 0,
            free_entry: None,
            entry_capacity_reservation: None,
            index: DependencyIndex::new(),
        }
    }

    pub(crate) fn get(
        &self,
        identity: &DependencyIdentity,
    ) -> Option<(usize, &DependencyCacheEntry)> {
        let index = self.record_index(identity)?;
        let DependencyCacheRecordSlot::Occupied(record) = self.entries.get(index)? else {
            debug_assert!(false, "dependency index points at a vacant record slot");
            return None;
        };
        Some((index, &record.entry))
    }

    pub(crate) fn capacity_plan(
        &self,
    ) -> Result<DependencyCacheCapacityPlan, DependencyCacheCapacityError> {
        let entry_replacement =
            if self.free_entry.is_none() && self.entries.len() == self.entries.capacity() {
                let required_capacity = self.entries.len().checked_add(1).ok_or(
                    DependencyCacheCapacityError::Capacity {
                        storage: DependencyCacheStorage::Entries,
                        cause: CapacityPlanError::CapacityOverflow,
                    },
                )?;
                Some(
                    CapacityPlan::for_type::<DependencyCacheRecordSlot>(0, 0, required_capacity)
                        .map_err(|cause| DependencyCacheCapacityError::Capacity {
                            storage: DependencyCacheStorage::Entries,
                            cause,
                        })?,
                )
            } else {
                None
            };
        let index_slots = self.index_slots_for_next_entry()?;
        let index_replacement = index_slots
            .map(|slots| {
                CapacityPlan::for_type::<usize>(0, 0, slots)
                    .map(|allocation| DependencyIndexReplacementPlan { slots, allocation })
            })
            .transpose()
            .map_err(|cause| DependencyCacheCapacityError::Capacity {
                storage: DependencyCacheStorage::Index,
                cause,
            })?;

        Ok(DependencyCacheCapacityPlan {
            entry_len: self.entries.len(),
            entry_capacity: self.entries.capacity(),
            entry_count: self.entry_count,
            free_entry: self.free_entry,
            index_len: self.index.slots.len(),
            index_capacity: self.index.slots.capacity(),
            index_tombstones: self.index.tombstones,
            entry_replacement,
            index_replacement,
        })
    }

    pub(crate) fn apply_capacity_plan(
        &mut self,
        plan: DependencyCacheCapacityPlan,
    ) -> Result<DependencyCacheCapacityActual, DependencyCacheCapacityError> {
        plan.verify(self)?;

        // Qualify complete replacements before mutating either live container.
        let (mut replacement_index, index_retained_bytes) =
            if let Some(index_plan) = plan.index_replacement {
                let mut slots = Vec::new();
                index_plan
                    .allocation
                    .verify_preflight(slots.len(), slots.capacity())
                    .map_err(|cause| DependencyCacheCapacityError::Capacity {
                        storage: DependencyCacheStorage::Index,
                        cause,
                    })?;
                slots
                    .try_reserve_exact(index_plan.allocation.allocator_additional())
                    .map_err(|_| DependencyCacheCapacityError::Allocation {
                        storage: DependencyCacheStorage::Index,
                    })?;
                let actual = index_plan
                    .allocation
                    .finish(slots.len(), slots.capacity())
                    .map_err(|cause| DependencyCacheCapacityError::Capacity {
                        storage: DependencyCacheStorage::Index,
                        cause,
                    })?;
                slots.resize(index_plan.slots, DEPENDENCY_INDEX_EMPTY);
                (Some(slots), Some(actual))
            } else {
                (None, None)
            };

        let (mut replacement_entries, entry_retained_bytes) =
            if let Some(entry_plan) = plan.entry_replacement {
                let mut entries = Vec::new();
                entry_plan
                    .verify_preflight(entries.len(), entries.capacity())
                    .map_err(|cause| DependencyCacheCapacityError::Capacity {
                        storage: DependencyCacheStorage::Entries,
                        cause,
                    })?;
                entries
                    .try_reserve_exact(entry_plan.allocator_additional())
                    .map_err(|_| DependencyCacheCapacityError::Allocation {
                        storage: DependencyCacheStorage::Entries,
                    })?;
                let actual = entry_plan
                    .finish(entries.len(), entries.capacity())
                    .map_err(|cause| DependencyCacheCapacityError::Capacity {
                        storage: DependencyCacheStorage::Entries,
                        cause,
                    })?;
                (Some(entries), Some(actual))
            } else {
                (None, None)
            };

        if let Some(slots) = replacement_index.as_mut() {
            self.rebuild_index(slots)?;
        }

        if let Some(entries) = replacement_entries.as_mut() {
            entries.append(&mut self.entries);
            std::mem::swap(&mut self.entries, entries);
        }
        if let Some(slots) = replacement_index {
            self.index.slots = slots;
            self.index.tombstones = 0;
        }

        Ok(DependencyCacheCapacityActual {
            entry_retained_bytes,
            index_retained_bytes,
        })
    }

    pub(crate) fn take_capacity_reservation(
        &mut self,
        storage: DependencyCacheStorage,
    ) -> Option<ImportReservation> {
        match storage {
            DependencyCacheStorage::Entries => self.entry_capacity_reservation.take(),
            DependencyCacheStorage::Index => self.index.capacity_reservation.take(),
        }
    }

    pub(crate) fn retain_capacity_reservation(
        &mut self,
        storage: DependencyCacheStorage,
        reservation: Option<ImportReservation>,
    ) {
        match storage {
            DependencyCacheStorage::Entries => self.entry_capacity_reservation = reservation,
            DependencyCacheStorage::Index => self.index.capacity_reservation = reservation,
        }
    }

    pub(crate) fn mark_resolving(
        &mut self,
        identity: DependencyIdentity,
        identity_path_reservation: ImportReservation,
    ) -> Result<bool, ()> {
        let probe = self.probe_identity(&identity);
        if probe.record_index.is_some() {
            return Ok(false);
        }
        let Some(index_slot) = probe.insertion_slot else {
            return Err(());
        };
        let record = DependencyCacheRecord {
            identity,
            entry: DependencyCacheEntry::Resolving,
            _identity_path_reservation: identity_path_reservation,
        };
        let record_index = if let Some(index) = self.free_entry {
            let Some(slot) = self.entries.get_mut(index) else {
                return Err(());
            };
            let DependencyCacheRecordSlot::Vacant { next } = slot else {
                return Err(());
            };
            self.free_entry = *next;
            *slot = DependencyCacheRecordSlot::Occupied(record);
            index
        } else {
            if self.entries.len() == self.entries.capacity() {
                return Err(());
            }
            let index = self.entries.len();
            self.entries
                .push(DependencyCacheRecordSlot::Occupied(record));
            index
        };
        self.index.occupy(index_slot, record_index);
        self.entry_count += 1;
        Ok(true)
    }

    pub(crate) fn store_available(
        &mut self,
        identity: &DependencyIdentity,
        bytes: Arc<Vec<u8>>,
        reservations: Vec<ImportReservation>,
        sharing_reservation: ImportReservation,
    ) -> bool {
        let Some(entry) = self.entry_mut(identity) else {
            return false;
        };
        let DependencyCacheEntry::Resolving = entry else {
            return false;
        };
        *entry = DependencyCacheEntry::Available(CachedDependencySnapshot {
            bytes,
            _reservations: reservations,
            _sharing_reservation: sharing_reservation,
        });
        true
    }

    pub(crate) fn store_failed(
        &mut self,
        identity: &DependencyIdentity,
        cause: ScopeAbortCause,
    ) -> Result<usize, ScopeAbortCause> {
        let Some(index) = self.record_index(identity) else {
            return Err(cause);
        };
        let Some(DependencyCacheRecordSlot::Occupied(record)) = self.entries.get_mut(index) else {
            return Err(cause);
        };
        let entry = &mut record.entry;
        let DependencyCacheEntry::Resolving = entry else {
            return Err(cause);
        };
        *entry = DependencyCacheEntry::Failed(cause);
        Ok(index)
    }

    pub(crate) fn failed(&self, index: usize) -> Option<&ScopeAbortCause> {
        match self.entries.get(index) {
            Some(DependencyCacheRecordSlot::Occupied(record)) => match &record.entry {
                DependencyCacheEntry::Failed(cause) => Some(cause),
                DependencyCacheEntry::Resolving
                | DependencyCacheEntry::Available(_)
                | DependencyCacheEntry::FailureConsumed => None,
            },
            Some(DependencyCacheRecordSlot::Vacant { .. }) | None => None,
        }
    }

    pub(crate) fn take_failed(&mut self, index: usize) -> Option<ScopeAbortCause> {
        let DependencyCacheRecordSlot::Occupied(record) = self.entries.get_mut(index)? else {
            return None;
        };
        let entry = &mut record.entry;
        if !matches!(entry, DependencyCacheEntry::Failed(_)) {
            return None;
        }
        match std::mem::replace(entry, DependencyCacheEntry::FailureConsumed) {
            DependencyCacheEntry::Failed(cause) => Some(cause),
            DependencyCacheEntry::Resolving
            | DependencyCacheEntry::Available(_)
            | DependencyCacheEntry::FailureConsumed => None,
        }
    }

    pub(crate) fn remove(&mut self, identity: &DependencyIdentity) {
        let probe = self.probe_identity(identity);
        let (Some(record_index), Some(index_slot)) = (probe.record_index, probe.matched_slot)
        else {
            return;
        };
        let Some(slot) = self.entries.get_mut(record_index) else {
            return;
        };
        if !matches!(slot, DependencyCacheRecordSlot::Occupied(_)) {
            return;
        }
        *slot = DependencyCacheRecordSlot::Vacant {
            next: self.free_entry,
        };
        self.free_entry = Some(record_index);
        self.entry_count -= 1;
        self.index.remove(index_slot);
    }

    fn entry_mut(&mut self, identity: &DependencyIdentity) -> Option<&mut DependencyCacheEntry> {
        let index = self.record_index(identity)?;
        let DependencyCacheRecordSlot::Occupied(record) = self.entries.get_mut(index)? else {
            return None;
        };
        Some(&mut record.entry)
    }

    fn record_index(&self, identity: &DependencyIdentity) -> Option<usize> {
        self.probe_identity(identity).record_index
    }

    fn probe_identity(&self, identity: &DependencyIdentity) -> DependencyIndexProbe {
        self.index.probe(identity, |record_index| {
            self.entries
                .get(record_index)
                .and_then(DependencyCacheRecordSlot::record)
                .is_some_and(|record| &record.identity == identity)
        })
    }

    fn index_slots_for_next_entry(&self) -> Result<Option<usize>, DependencyCacheCapacityError> {
        let next_count =
            self.entry_count
                .checked_add(1)
                .ok_or(DependencyCacheCapacityError::Capacity {
                    storage: DependencyCacheStorage::Index,
                    cause: CapacityPlanError::CapacityOverflow,
                })?;
        let minimum_slots =
            next_count
                .checked_mul(2)
                .ok_or(DependencyCacheCapacityError::Capacity {
                    storage: DependencyCacheStorage::Index,
                    cause: CapacityPlanError::CapacityOverflow,
                })?;
        if self.index.slots.len() < minimum_slots {
            return minimum_slots
                .max(MIN_DEPENDENCY_INDEX_SLOTS)
                .checked_next_power_of_two()
                .map(Some)
                .ok_or(DependencyCacheCapacityError::Capacity {
                    storage: DependencyCacheStorage::Index,
                    cause: CapacityPlanError::CapacityOverflow,
                });
        }
        let occupied_slots = self
            .entry_count
            .checked_add(self.index.tombstones)
            .and_then(|count| count.checked_add(1))
            .and_then(|count| count.checked_mul(2))
            .ok_or(DependencyCacheCapacityError::Capacity {
                storage: DependencyCacheStorage::Index,
                cause: CapacityPlanError::CapacityOverflow,
            })?;
        Ok((occupied_slots > self.index.slots.len()).then_some(self.index.slots.len()))
    }

    fn rebuild_index(&self, slots: &mut [usize]) -> Result<(), DependencyCacheCapacityError> {
        slots.fill(DEPENDENCY_INDEX_EMPTY);
        for (record_index, slot) in self.entries.iter().enumerate() {
            let DependencyCacheRecordSlot::Occupied(record) = slot else {
                continue;
            };
            let probe = DependencyIndex::probe_slots(
                slots,
                &self.index.hash_builder,
                &record.identity,
                |_| false,
            );
            let Some(index_slot) = probe.insertion_slot else {
                return Err(DependencyCacheCapacityError::IndexContract);
            };
            slots[index_slot] = record_index;
        }
        Ok(())
    }
}

enum DependencyCacheRecordSlot {
    Occupied(DependencyCacheRecord),
    Vacant { next: Option<usize> },
}

impl DependencyCacheRecordSlot {
    fn record(&self) -> Option<&DependencyCacheRecord> {
        match self {
            Self::Occupied(record) => Some(record),
            Self::Vacant { .. } => None,
        }
    }
}

struct DependencyCacheRecord {
    identity: DependencyIdentity,
    entry: DependencyCacheEntry,
    _identity_path_reservation: ImportReservation,
}

struct DependencyIndex {
    slots: Vec<usize>,
    tombstones: usize,
    hash_builder: RandomState,
    capacity_reservation: Option<ImportReservation>,
}

impl DependencyIndex {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            tombstones: 0,
            hash_builder: RandomState::new(),
            capacity_reservation: None,
        }
    }

    fn probe(
        &self,
        identity: &DependencyIdentity,
        matches: impl FnMut(usize) -> bool,
    ) -> DependencyIndexProbe {
        Self::probe_slots(&self.slots, &self.hash_builder, identity, matches)
    }

    fn probe_slots(
        slots: &[usize],
        hash_builder: &RandomState,
        identity: &DependencyIdentity,
        mut matches: impl FnMut(usize) -> bool,
    ) -> DependencyIndexProbe {
        if slots.is_empty() {
            return DependencyIndexProbe::empty();
        }
        debug_assert!(slots.len().is_power_of_two());
        let mask = slots.len() - 1;
        let mut slot = hash_builder.hash_one(identity) as usize & mask;
        let mut first_tombstone = None;
        for probes in 1..=slots.len() {
            match slots[slot] {
                DEPENDENCY_INDEX_EMPTY => {
                    return DependencyIndexProbe {
                        record_index: None,
                        matched_slot: None,
                        insertion_slot: Some(first_tombstone.unwrap_or(slot)),
                        _probes: probes,
                    };
                }
                DEPENDENCY_INDEX_TOMBSTONE => {
                    first_tombstone.get_or_insert(slot);
                }
                record_index if matches(record_index) => {
                    return DependencyIndexProbe {
                        record_index: Some(record_index),
                        matched_slot: Some(slot),
                        insertion_slot: None,
                        _probes: probes,
                    };
                }
                _ => {}
            }
            slot = (slot + 1) & mask;
        }
        DependencyIndexProbe {
            record_index: None,
            matched_slot: None,
            insertion_slot: first_tombstone,
            _probes: slots.len(),
        }
    }

    fn occupy(&mut self, slot: usize, record_index: usize) {
        if self.slots[slot] == DEPENDENCY_INDEX_TOMBSTONE {
            self.tombstones -= 1;
        }
        self.slots[slot] = record_index;
    }

    fn remove(&mut self, slot: usize) {
        debug_assert!(self.slots[slot] < DEPENDENCY_INDEX_TOMBSTONE);
        self.slots[slot] = DEPENDENCY_INDEX_TOMBSTONE;
        self.tombstones += 1;
    }
}

#[derive(Debug, Clone, Copy)]
struct DependencyIndexProbe {
    record_index: Option<usize>,
    matched_slot: Option<usize>,
    insertion_slot: Option<usize>,
    _probes: usize,
}

impl DependencyIndexProbe {
    const fn empty() -> Self {
        Self {
            record_index: None,
            matched_slot: None,
            insertion_slot: None,
            _probes: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DependencyCacheStorage {
    Entries,
    Index,
}

#[derive(Debug)]
pub(crate) enum DependencyCacheCapacityError {
    Capacity {
        storage: DependencyCacheStorage,
        cause: CapacityPlanError,
    },
    Allocation {
        storage: DependencyCacheStorage,
    },
    Stale,
    IndexContract,
}

#[derive(Clone, Copy)]
struct DependencyIndexReplacementPlan {
    slots: usize,
    allocation: CapacityPlan,
}

#[derive(Clone, Copy)]
pub(crate) struct DependencyCacheCapacityPlan {
    entry_len: usize,
    entry_capacity: usize,
    entry_count: usize,
    free_entry: Option<usize>,
    index_len: usize,
    index_capacity: usize,
    index_tombstones: usize,
    entry_replacement: Option<CapacityPlan>,
    index_replacement: Option<DependencyIndexReplacementPlan>,
}

impl DependencyCacheCapacityPlan {
    pub(crate) const fn requires_apply(self) -> bool {
        self.entry_replacement.is_some() || self.index_replacement.is_some()
    }

    pub(crate) fn temporary_envelope_bytes(
        self,
        storage: DependencyCacheStorage,
    ) -> Result<u64, DependencyCacheCapacityError> {
        let plan = match storage {
            DependencyCacheStorage::Entries => self.entry_replacement,
            DependencyCacheStorage::Index => self
                .index_replacement
                .map(|replacement| replacement.allocation),
        };
        plan.map_or(Ok(0), |plan| {
            plan.envelope_delta_bytes()
                .map_err(|cause| DependencyCacheCapacityError::Capacity { storage, cause })
        })
    }

    fn verify(self, cache: &DependencyCache) -> Result<(), DependencyCacheCapacityError> {
        if self.entry_len != cache.entries.len()
            || self.entry_capacity != cache.entries.capacity()
            || self.entry_count != cache.entry_count
            || self.free_entry != cache.free_entry
            || self.index_len != cache.index.slots.len()
            || self.index_capacity != cache.index.slots.capacity()
            || self.index_tombstones != cache.index.tombstones
        {
            return Err(DependencyCacheCapacityError::Stale);
        }
        Ok(())
    }
}

pub(crate) struct DependencyCacheCapacityActual {
    entry_retained_bytes: Option<u64>,
    index_retained_bytes: Option<u64>,
}

impl DependencyCacheCapacityActual {
    pub(crate) const fn retained_bytes(&self, storage: DependencyCacheStorage) -> Option<u64> {
        match storage {
            DependencyCacheStorage::Entries => self.entry_retained_bytes,
            DependencyCacheStorage::Index => self.index_retained_bytes,
        }
    }
}

pub(crate) enum DependencyCacheEntry {
    Resolving,
    Available(CachedDependencySnapshot),
    Failed(ScopeAbortCause),
    FailureConsumed,
}

pub(crate) struct CachedDependencySnapshot {
    pub(crate) bytes: Arc<Vec<u8>>,
    _reservations: Vec<ImportReservation>,
    _sharing_reservation: ImportReservation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{
        ImportCoordinator, ImportDimension, ImportLedgerPhase, ImportOperationControl,
        import_ledger_limits,
    };
    use crate::limits::ResourceLimits;

    fn indexed_identity(
        authority: &AuthorityDomain,
        prefix: &str,
        index: usize,
    ) -> DependencyIdentity {
        let path = AssetPath::new(format!("assets/{prefix}-{index:05}.bin"))
            .expect("generated dependency path must be valid");
        DependencyIdentity::new(authority.clone(), path)
    }

    #[test]
    fn dependency_role_id_has_one_canonical_grammar() {
        assert_eq!(
            DependencyRoleId::new("material.library")
                .expect("valid role")
                .as_str(),
            "material.library"
        );
        for invalid in [
            "",
            "Material",
            ".material",
            "material.",
            "material..image",
            "-image",
        ] {
            assert!(
                DependencyRoleId::new(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn dependency_role_id_is_fixed_capacity_and_clone_is_allocation_free() {
        let role = DependencyRoleId::new("material.library").unwrap();
        let cloned = role.clone();

        assert_eq!(size_of::<DependencyRoleId>(), MAX_DEPENDENCY_ROLE_ID_BYTES);
        assert_eq!(cloned, role);
    }

    #[test]
    fn dependency_cache_capacity_distinguishes_entry_and_index_ledger_evidence() {
        let mut cache = DependencyCache::new();
        let plan = cache.capacity_plan().unwrap();
        let entry_envelope = plan
            .temporary_envelope_bytes(DependencyCacheStorage::Entries)
            .unwrap();
        let index_envelope = plan
            .temporary_envelope_bytes(DependencyCacheStorage::Index)
            .unwrap();
        assert!(entry_envelope > 0);
        assert!(index_envelope > 0);

        let limits = ResourceLimits::default();
        let mut coordinator = ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).unwrap(),
        );
        let mut entry_reservation = coordinator
            .checkpoint()
            .reserve(
                ImportLedgerPhase::DependencyAcquisition,
                ImportDimension::ScratchBytes,
                entry_envelope,
            )
            .unwrap();
        let mut index_reservation = coordinator
            .checkpoint()
            .reserve(
                ImportLedgerPhase::DependencyAcquisition,
                ImportDimension::ScratchBytes,
                index_envelope,
            )
            .unwrap();

        let actual = cache.apply_capacity_plan(plan).unwrap();
        let entry_bytes = actual
            .retained_bytes(DependencyCacheStorage::Entries)
            .unwrap();
        let index_bytes = actual
            .retained_bytes(DependencyCacheStorage::Index)
            .unwrap();
        coordinator
            .checkpoint()
            .resize_reservation(&mut entry_reservation, entry_bytes)
            .unwrap();
        coordinator
            .checkpoint()
            .resize_reservation(&mut index_reservation, index_bytes)
            .unwrap();
        assert_eq!(
            entry_bytes,
            u64::try_from(
                cache
                    .entries
                    .capacity()
                    .checked_mul(size_of::<DependencyCacheRecordSlot>())
                    .unwrap()
            )
            .unwrap()
        );
        assert_eq!(
            index_bytes,
            u64::try_from(
                cache
                    .index
                    .slots
                    .capacity()
                    .checked_mul(size_of::<usize>())
                    .unwrap()
            )
            .unwrap()
        );
        cache.retain_capacity_reservation(DependencyCacheStorage::Entries, Some(entry_reservation));
        cache.retain_capacity_reservation(DependencyCacheStorage::Index, Some(index_reservation));

        let evidence = coordinator
            .dimension_evidence(ImportDimension::ScratchBytes)
            .unwrap();
        assert_eq!(
            evidence.current_live_bytes(),
            entry_bytes.checked_add(index_bytes).unwrap()
        );
        drop(cache);
        assert_eq!(
            coordinator
                .dimension_evidence(ImportDimension::ScratchBytes)
                .unwrap()
                .current_live_bytes(),
            0
        );
    }

    #[test]
    fn dependency_index_high_cardinality_lookups_use_bounded_probe_work() {
        const ENTRY_COUNT: usize = 4_096;
        let authority = AuthorityDomain::new();
        let mut identities = Vec::with_capacity(ENTRY_COUNT);
        let mut index = DependencyIndex::new();
        index.slots = vec![DEPENDENCY_INDEX_EMPTY; ENTRY_COUNT * 2];
        let mut insertion_probes = 0_usize;

        for record_index in 0..ENTRY_COUNT {
            let identity = indexed_identity(&authority, "high-cardinality", record_index);
            let probe = index.probe(&identity, |candidate| {
                identities
                    .get(candidate)
                    .is_some_and(|stored| stored == &identity)
            });
            insertion_probes = insertion_probes.checked_add(probe._probes).unwrap();
            assert!(probe.record_index.is_none());
            index.occupy(probe.insertion_slot.unwrap(), record_index);
            identities.push(identity);
        }

        let mut lookup_probes = 0_usize;
        for (record_index, identity) in identities.iter().enumerate() {
            let probe = index.probe(identity, |candidate| {
                identities
                    .get(candidate)
                    .is_some_and(|stored| stored == identity)
            });
            lookup_probes = lookup_probes.checked_add(probe._probes).unwrap();
            assert_eq!(probe.record_index, Some(record_index));
        }

        let linear_scan_probes = ENTRY_COUNT.checked_mul(ENTRY_COUNT).unwrap();
        assert!(insertion_probes < ENTRY_COUNT * 16);
        assert!(lookup_probes < ENTRY_COUNT * 16);
        assert!(lookup_probes < linear_scan_probes / 128);
    }

    #[test]
    fn dependency_index_tombstones_preserve_record_indices_and_are_reused() {
        const ENTRY_COUNT: usize = 64;
        let authority = AuthorityDomain::new();
        let identities = (0..ENTRY_COUNT)
            .map(|record_index| indexed_identity(&authority, "stable", record_index))
            .collect::<Vec<_>>();
        let mut index = DependencyIndex::new();
        index.slots = vec![DEPENDENCY_INDEX_EMPTY; ENTRY_COUNT * 4];
        for (record_index, identity) in identities.iter().enumerate() {
            let probe = index.probe(identity, |candidate| {
                identities
                    .get(candidate)
                    .is_some_and(|stored| stored == identity)
            });
            index.occupy(probe.insertion_slot.unwrap(), record_index);
        }

        let removed_record = ENTRY_COUNT / 2;
        let removed_probe = index.probe(&identities[removed_record], |candidate| {
            identities
                .get(candidate)
                .is_some_and(|stored| stored == &identities[removed_record])
        });
        let removed_slot = removed_probe.matched_slot.unwrap();
        index.remove(removed_slot);
        assert_eq!(index.tombstones, 1);

        for (record_index, identity) in identities.iter().enumerate() {
            if record_index == removed_record {
                continue;
            }
            let probe = index.probe(identity, |candidate| {
                identities
                    .get(candidate)
                    .is_some_and(|stored| stored == identity)
            });
            assert_eq!(probe.record_index, Some(record_index));
        }

        let replacement = (0..100_000)
            .map(|candidate| indexed_identity(&authority, "replacement", candidate))
            .find(|identity| {
                index
                    .probe(identity, |candidate| {
                        identities
                            .get(candidate)
                            .is_some_and(|stored| stored == identity)
                    })
                    .insertion_slot
                    == Some(removed_slot)
            })
            .expect("a bounded search must find an identity whose probe reuses the tombstone");
        let replacement_probe = index.probe(&replacement, |candidate| {
            identities
                .get(candidate)
                .is_some_and(|stored| stored == &replacement)
        });
        index.occupy(replacement_probe.insertion_slot.unwrap(), ENTRY_COUNT);
        assert_eq!(index.tombstones, 0);
    }

    #[test]
    fn dependency_cache_reuses_vacant_records_without_moving_live_indices() {
        let limits = ResourceLimits::default();
        let mut coordinator = ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).unwrap(),
        );
        let authority = AuthorityDomain::new();
        let identities = (0..3)
            .map(|record_index| indexed_identity(&authority, "record", record_index))
            .collect::<Vec<_>>();
        let mut cache = DependencyCache::new();
        cache.entries = Vec::with_capacity(4);
        cache.index.slots = vec![DEPENDENCY_INDEX_EMPTY; 8];

        for identity in &identities {
            let reservation = coordinator
                .checkpoint()
                .reserve(
                    ImportLedgerPhase::DependencyAcquisition,
                    ImportDimension::ScratchBytes,
                    1,
                )
                .unwrap();
            assert!(
                cache
                    .mark_resolving(identity.try_clone().unwrap(), reservation)
                    .unwrap()
            );
        }
        assert_eq!(cache.get(&identities[0]).map(|(index, _)| index), Some(0));
        assert_eq!(cache.get(&identities[1]).map(|(index, _)| index), Some(1));
        assert_eq!(cache.get(&identities[2]).map(|(index, _)| index), Some(2));

        cache.remove(&identities[1]);
        assert!(cache.get(&identities[1]).is_none());
        assert_eq!(cache.get(&identities[0]).map(|(index, _)| index), Some(0));
        assert_eq!(cache.get(&identities[2]).map(|(index, _)| index), Some(2));

        let replacement = indexed_identity(&authority, "record-replacement", 0);
        let replacement_reservation = coordinator
            .checkpoint()
            .reserve(
                ImportLedgerPhase::DependencyAcquisition,
                ImportDimension::ScratchBytes,
                1,
            )
            .unwrap();
        assert!(
            cache
                .mark_resolving(replacement.try_clone().unwrap(), replacement_reservation,)
                .unwrap()
        );
        assert_eq!(cache.get(&replacement).map(|(index, _)| index), Some(1));
        assert_eq!(cache.get(&identities[2]).map(|(index, _)| index), Some(2));
    }
}
