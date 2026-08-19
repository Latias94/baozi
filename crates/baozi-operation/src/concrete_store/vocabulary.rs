use std::fmt;
use std::hash::{Hash, Hasher};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::atomic::{AtomicU64, Ordering};

use super::StorePlanError;
use crate::ledger::{DimensionPolicy, LedgerLimits};
use crate::retained::{
    RetainedBatchLimits, RetainedBlockTablePlan, RetainedFailureTerminalWorkPlan,
    RetainedHandleSharePlan, RetainedPublicationPlan, RetainedWriterPlan,
};

/// Allocation-free terminal fallback payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StoreFallbackEnvelope {
    code: u32,
}

static NEXT_STORE_KEY_DOMAIN_ID: AtomicU64 = AtomicU64::new(1);

/// Stable identity of one Baozi-managed concrete-store semantic catalog.
///
/// The fixed namespace and nonzero revision are registry data, never process-local allocation
/// order. A registry must assign namespaces canonically and increment the revision whenever a local
/// identifier changes meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreSemanticCatalogId {
    namespace: [u8; 16],
    revision: NonZeroU32,
}

impl StoreSemanticCatalogId {
    /// Declares one registry-normalized catalog identity.
    ///
    /// This inert value grants no host admission, registry authority, or BaoziVerified status.
    /// Those claims require an independent host-owned registry and evidence decision.
    pub const fn new(namespace: [u8; 16], revision: NonZeroU32) -> Self {
        Self {
            namespace,
            revision,
        }
    }

    /// Returns the fixed registry namespace.
    pub const fn namespace(self) -> [u8; 16] {
        self.namespace
    }

    /// Returns the semantic-catalog revision.
    pub const fn revision(self) -> NonZeroU32 {
        self.revision
    }

    /// Returns the canonical namespace-plus-revision encoding.
    pub const fn to_canonical_bytes(self) -> [u8; 20] {
        let revision = self.revision.get().to_be_bytes();
        let mut encoded = [0_u8; 20];
        let mut index = 0;
        while index < self.namespace.len() {
            encoded[index] = self.namespace[index];
            index += 1;
        }
        encoded[16] = revision[0];
        encoded[17] = revision[1];
        encoded[18] = revision[2];
        encoded[19] = revision[3];
        encoded
    }
}

/// Stable, host-normalized semantic identity for one accounting phase.
///
/// This inert value contains no process-local brand, pointer, callback, plugin state, or allocation
/// order and is therefore suitable for evidence transcripts and persistent identities.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StorePhaseSemanticId(StoreSemanticIdentity);

impl StorePhaseSemanticId {
    /// Declares one stable phase semantic identity.
    ///
    /// Construction alone grants no runtime admission, registry authority, or verification claim.
    pub const fn new(catalog: StoreSemanticCatalogId, local: NonZeroU64) -> Self {
        Self(StoreSemanticIdentity { catalog, local })
    }

    /// Returns the owning semantic-catalog identity.
    pub const fn catalog(self) -> StoreSemanticCatalogId {
        self.0.catalog
    }

    /// Returns the catalog-local stable identifier.
    pub const fn local(self) -> NonZeroU64 {
        self.0.local
    }

    /// Returns the canonical kind-plus-catalog-plus-local encoding.
    pub const fn to_canonical_bytes(self) -> [u8; 29] {
        self.0.to_canonical_bytes(1)
    }
}

/// Stable, host-normalized semantic identity for one accounting dimension.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreDimensionSemanticId(StoreSemanticIdentity);

impl StoreDimensionSemanticId {
    /// Declares one stable dimension semantic identity.
    ///
    /// Construction alone grants no runtime admission, registry authority, or verification claim.
    pub const fn new(catalog: StoreSemanticCatalogId, local: NonZeroU64) -> Self {
        Self(StoreSemanticIdentity { catalog, local })
    }

    /// Returns the owning semantic-catalog identity.
    pub const fn catalog(self) -> StoreSemanticCatalogId {
        self.0.catalog
    }

    /// Returns the catalog-local stable identifier.
    pub const fn local(self) -> NonZeroU64 {
        self.0.local
    }

    /// Returns the canonical kind-plus-catalog-plus-local encoding.
    pub const fn to_canonical_bytes(self) -> [u8; 29] {
        self.0.to_canonical_bytes(2)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct StoreSemanticIdentity {
    catalog: StoreSemanticCatalogId,
    local: NonZeroU64,
}

impl StoreSemanticIdentity {
    const fn to_canonical_bytes(self, kind: u8) -> [u8; 29] {
        let catalog = self.catalog.to_canonical_bytes();
        let local = self.local.get().to_be_bytes();
        let mut encoded = [0_u8; 29];
        encoded[0] = kind;
        let mut index = 0;
        while index < catalog.len() {
            encoded[index + 1] = catalog[index];
            index += 1;
        }
        index = 0;
        while index < local.len() {
            encoded[index + 21] = local[index];
            index += 1;
        }
        encoded
    }
}

/// Process-local authority for one normalized concrete-store key vocabulary.
///
/// A domain is only a runtime brand, host-admission authority, and linear lifetime anchor; it is not
/// stable wire data, digest input, user-visible semantics, or a semantic-to-ordinal mapping. The
/// complete mapping lives in the metered key-bearing ledger evidence cells. Consuming the domain
/// into [`StoreLedgerPlan`] moves the anchor through the operation into terminal evidence without
/// another mapping allocation or reference count. Plugins receive only host-admitted keys and never
/// this authority.
///
/// The authority is intentionally neither `Clone` nor `Copy`:
///
/// ```compile_fail,E0277
/// use baozi_operation::StoreKeyDomain;
///
/// fn require_copy<T: Copy>() {}
/// require_copy::<StoreKeyDomain>();
/// ```
pub struct StoreKeyDomain {
    id: NonZeroU64,
    catalog: StoreSemanticCatalogId,
}

impl StoreKeyDomain {
    /// Issues one unique process-local key authority.
    pub fn try_new(catalog: StoreSemanticCatalogId) -> Result<Self, StoreKeyDomainCreateError> {
        let id = NEXT_STORE_KEY_DOMAIN_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                (next != 0).then(|| next.wrapping_add(1))
            })
            .map_err(|_| StoreKeyDomainCreateError::Exhausted)?;
        let id = NonZeroU64::new(id).ok_or(StoreKeyDomainCreateError::Exhausted)?;
        Ok(Self { id, catalog })
    }

    /// Returns this authority's frozen semantic-catalog identity.
    pub const fn catalog(&self) -> StoreSemanticCatalogId {
        self.catalog
    }

    /// Uses host authority to admit one phase identity into this runtime domain.
    ///
    /// A matching catalog is necessary but does not by itself imply registry ownership or
    /// BaoziVerified evidence status; this call is the host's explicit runtime-admission decision.
    pub fn try_admit_phase(&self, semantic: StorePhaseSemanticId) -> Option<StorePhaseKey> {
        if semantic.catalog() == self.catalog {
            Some(StorePhaseKey {
                brand: self.id,
                semantic,
            })
        } else {
            None
        }
    }

    /// Uses host authority to admit one dimension identity into this runtime domain.
    ///
    /// A matching catalog is necessary but does not by itself imply registry ownership or
    /// BaoziVerified evidence status; this call is the host's explicit runtime-admission decision.
    pub fn try_admit_dimension(
        &self,
        semantic: StoreDimensionSemanticId,
    ) -> Option<StoreDimensionKey> {
        if semantic.catalog() == self.catalog {
            Some(StoreDimensionKey {
                brand: self.id,
                semantic,
            })
        } else {
            None
        }
    }

    pub(super) fn owns_phase(&self, key: StorePhaseKey) -> bool {
        key.brand == self.id && key.semantic.catalog() == self.catalog
    }

    pub(super) fn owns_dimension(&self, key: StoreDimensionKey) -> bool {
        key.brand == self.id && key.semantic.catalog() == self.catalog
    }
}

impl fmt::Debug for StoreKeyDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreKeyDomain")
            .finish_non_exhaustive()
    }
}

/// Failure while issuing a process-local concrete-store key authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreKeyDomainCreateError {
    Exhausted,
}

impl fmt::Display for StoreKeyDomainCreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted => formatter.write_str("concrete-store key domain space is exhausted"),
        }
    }
}

impl std::error::Error for StoreKeyDomainCreateError {}

/// Fixed, inert accounting-phase identity used by the concrete retained store.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StorePhaseKey {
    brand: NonZeroU64,
    semantic: StorePhaseSemanticId,
}

impl crate::retained::TerminalEvidenceKeySeal for StorePhaseKey {}

impl crate::retained::TerminalEvidenceKey for StorePhaseKey {}

impl StorePhaseKey {
    /// Returns stable semantic meaning without exposing the process-local brand.
    pub const fn semantic_id(self) -> StorePhaseSemanticId {
        self.semantic
    }
}

impl Hash for StorePhaseKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Semantic-only hashing permits cross-domain collection bucketing. Rust `Hash` output is
        // explicitly not a stable digest or wire encoding.
        self.semantic_id().hash(state);
    }
}

impl fmt::Debug for StorePhaseKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StorePhaseKey")
            .field(&self.semantic_id())
            .finish()
    }
}

/// Fixed, inert accounting-dimension identity used by the concrete retained store.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoreDimensionKey {
    brand: NonZeroU64,
    semantic: StoreDimensionSemanticId,
}

impl crate::retained::TerminalEvidenceKeySeal for StoreDimensionKey {}

impl crate::retained::TerminalEvidenceKey for StoreDimensionKey {}

impl StoreDimensionKey {
    /// Returns stable semantic meaning without exposing the process-local brand.
    pub const fn semantic_id(self) -> StoreDimensionSemanticId {
        self.semantic
    }
}

impl Hash for StoreDimensionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Semantic-only hashing permits cross-domain collection bucketing. Rust `Hash` output is
        // explicitly not a stable digest or wire encoding.
        self.semantic_id().hash(state);
    }
}

impl fmt::Debug for StoreDimensionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StoreDimensionKey")
            .field(&self.semantic_id())
            .finish()
    }
}

/// Failure while freezing the complete concrete-store ledger vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreLedgerPlanError {
    EmptyPhaseCatalog,
    ForeignPhaseDomain {
        phase: StorePhaseKey,
    },
    ForeignDimensionDomain {
        dimension: StoreDimensionKey,
    },
    DuplicatePhase {
        phase: StorePhaseKey,
    },
    PhaseOutOfOrder {
        previous: StorePhaseKey,
        phase: StorePhaseKey,
    },
    PhaseCatalogAllocationFailed,
}

impl fmt::Display for StoreLedgerPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPhaseCatalog => {
                formatter.write_str("retained store phase catalog must not be empty")
            }
            Self::ForeignPhaseDomain { phase } => write!(
                formatter,
                "retained store phase {phase:?} belongs to another key domain"
            ),
            Self::ForeignDimensionDomain { dimension } => write!(
                formatter,
                "retained store dimension {dimension:?} belongs to another key domain"
            ),
            Self::DuplicatePhase { phase } => {
                write!(formatter, "retained store phase catalog repeats {phase:?}")
            }
            Self::PhaseOutOfOrder { previous, phase } => write!(
                formatter,
                "retained store phase {phase:?} follows later phase {previous:?}"
            ),
            Self::PhaseCatalogAllocationFailed => {
                formatter.write_str("retained store phase catalog allocation failed")
            }
        }
    }
}

impl std::error::Error for StoreLedgerPlanError {}

/// Frozen dimension policy and complete phase vocabulary for one retained store operation.
pub struct StoreLedgerPlan {
    domain: StoreKeyDomain,
    limits: LedgerLimits<StoreDimensionKey>,
    phases: Vec<StorePhaseKey>,
}

impl fmt::Debug for StoreLedgerPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreLedgerPlan")
            .field("dimension_count", &self.dimension_count())
            .field("phases", &self.phases)
            .finish()
    }
}

impl StoreLedgerPlan {
    /// Freezes a non-empty phase vocabulary in canonical key order.
    ///
    /// This is an attempt-once authority transfer. The domain is consumed even when construction
    /// fails, so every previously admitted runtime key from that domain is burned for plan
    /// construction. A retry must create a fresh domain and re-admit the complete vocabulary; the
    /// error intentionally does not return a large authority-bearing input.
    pub fn try_new(
        domain: StoreKeyDomain,
        limits: LedgerLimits<StoreDimensionKey>,
        phases: &[StorePhaseKey],
    ) -> Result<Self, StoreLedgerPlanError> {
        if phases.is_empty() {
            return Err(StoreLedgerPlanError::EmptyPhaseCatalog);
        }
        for dimension in limits.dimensions() {
            if !domain.owns_dimension(dimension) {
                return Err(StoreLedgerPlanError::ForeignDimensionDomain { dimension });
            }
        }
        for phase in phases {
            if !domain.owns_phase(*phase) {
                return Err(StoreLedgerPlanError::ForeignPhaseDomain { phase: *phase });
            }
        }
        for pair in phases.windows(2) {
            if pair[0] == pair[1] {
                return Err(StoreLedgerPlanError::DuplicatePhase { phase: pair[1] });
            }
            if pair[0] > pair[1] {
                return Err(StoreLedgerPlanError::PhaseOutOfOrder {
                    previous: pair[0],
                    phase: pair[1],
                });
            }
        }
        let mut frozen = Vec::new();
        frozen
            .try_reserve_exact(phases.len())
            .map_err(|_| StoreLedgerPlanError::PhaseCatalogAllocationFailed)?;
        frozen.extend_from_slice(phases);
        Ok(Self {
            domain,
            limits,
            phases: frozen,
        })
    }

    /// Returns the number of frozen ledger dimensions.
    pub fn dimension_count(&self) -> usize {
        self.limits.dimension_count()
    }

    /// Returns the number of frozen ledger phases.
    pub fn phase_count(&self) -> usize {
        self.phases.len()
    }

    pub(super) fn contains_phase(&self, phase: StorePhaseKey) -> bool {
        self.owns_phase(phase) && self.phases.binary_search(&phase).is_ok()
    }

    pub(super) fn contains_dimension(&self, dimension: StoreDimensionKey) -> bool {
        self.owns_dimension(dimension) && self.limits.dimension(dimension).is_some()
    }

    pub(super) fn owns_phase(&self, phase: StorePhaseKey) -> bool {
        self.domain.owns_phase(phase)
    }

    pub(super) fn owns_dimension(&self, dimension: StoreDimensionKey) -> bool {
        self.domain.owns_dimension(dimension)
    }

    pub(super) fn failure_terminal_budget_matches(
        &self,
        plan: &RetainedStoreFailureTerminalWorkPlan,
    ) -> bool {
        if self.limits.dimension(plan.dimension()) != Some(plan.requirement().dimension_policy()) {
            return false;
        }
        let ordinary = self
            .limits
            .dimensions()
            .filter(|dimension| *dimension != plan.dimension())
            .try_fold((0_u64, 0_u64), |(attempted, consumed), dimension| {
                let policy = self.limits.dimension(dimension)?;
                Some((
                    attempted.checked_add(policy.attempted_limit())?,
                    consumed.checked_add(policy.consumed_limit())?,
                ))
            });
        matches!(
            ordinary,
            Some((attempted, consumed))
                if attempted == plan.ordinary_attempted_work_limit()
                    && consumed == plan.ordinary_consumed_work_limit()
        )
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        StoreKeyDomain,
        LedgerLimits<StoreDimensionKey>,
        Vec<StorePhaseKey>,
    ) {
        (self.domain, self.limits, self.phases)
    }
}

impl StoreFallbackEnvelope {
    /// Creates an application-defined fallback code.
    pub const fn new(code: u32) -> Self {
        Self { code }
    }

    /// Returns the application-defined fallback code.
    pub const fn code(self) -> u32 {
        self.code
    }
}

/// Fixed capacities for one concrete retained writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreBatchLimits {
    attribution_slots: usize,
    commitment_slots: usize,
}

impl RetainedStoreBatchLimits {
    /// Creates non-zero fixed capacities.
    pub fn new(attribution_slots: usize, commitment_slots: usize) -> Result<Self, StorePlanError> {
        if attribution_slots == 0 || commitment_slots == 0 {
            return Err(StorePlanError::InvalidBlockTable);
        }
        Ok(Self {
            attribution_slots,
            commitment_slots,
        })
    }

    /// Returns the attribution-slot capacity.
    pub const fn attribution_slots(self) -> usize {
        self.attribution_slots
    }

    /// Returns the commitment-slot capacity.
    pub const fn commitment_slots(self) -> usize {
        self.commitment_slots
    }

    pub(super) const fn into_raw(self) -> RetainedBatchLimits {
        RetainedBatchLimits::new(self.attribution_slots, self.commitment_slots)
    }
}

/// Ledger placement for one concrete retained writer.
///
/// The role is stable for one session vocabulary. Request-specific capacities are bound only after
/// a complete construction plan has derived its exact retained shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreWriterRole {
    phase: StorePhaseKey,
    capacity_dimension: StoreDimensionKey,
    attribution_dimension: StoreDimensionKey,
}

impl RetainedStoreWriterRole {
    /// Creates a writer role without entering the ledger or allocator.
    pub const fn new(
        phase: StorePhaseKey,
        capacity_dimension: StoreDimensionKey,
        attribution_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            phase,
            capacity_dimension,
            attribution_dimension,
        }
    }

    /// Binds one request-derived fixed-capacity envelope to this stable role.
    pub(super) const fn bind(self, limits: RetainedStoreBatchLimits) -> RetainedStoreWriterPlan {
        RetainedStoreWriterPlan {
            phase: self.phase,
            capacity_dimension: self.capacity_dimension,
            attribution_dimension: self.attribution_dimension,
            limits,
        }
    }

    #[cfg(feature = "_u4-activation")]
    pub(super) const fn keys_for_publication_validation(
        self,
    ) -> (StorePhaseKey, [StoreDimensionKey; 2]) {
        (
            self.phase,
            [self.capacity_dimension, self.attribution_dimension],
        )
    }
}

/// Ledger placement and fixed capacities for one concrete retained writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreWriterPlan {
    phase: StorePhaseKey,
    capacity_dimension: StoreDimensionKey,
    attribution_dimension: StoreDimensionKey,
    limits: RetainedStoreBatchLimits,
}

impl RetainedStoreWriterPlan {
    /// Creates a writer plan without entering the ledger or allocator.
    pub const fn new(
        phase: StorePhaseKey,
        capacity_dimension: StoreDimensionKey,
        attribution_dimension: StoreDimensionKey,
        limits: RetainedStoreBatchLimits,
    ) -> Self {
        RetainedStoreWriterRole::new(phase, capacity_dimension, attribution_dimension).bind(limits)
    }

    pub(super) const fn into_raw(self) -> RetainedWriterPlan<StorePhaseKey, StoreDimensionKey> {
        RetainedWriterPlan::new(
            self.phase,
            self.capacity_dimension,
            self.attribution_dimension,
            self.limits.into_raw(),
        )
    }

    pub(super) const fn capacity_dimension(self) -> StoreDimensionKey {
        self.capacity_dimension
    }

    pub(super) const fn keys_for_publication_validation(
        self,
    ) -> (StorePhaseKey, [StoreDimensionKey; 2]) {
        (
            self.phase,
            [self.capacity_dimension, self.attribution_dimension],
        )
    }

    pub(super) const fn transcript_parts(
        self,
    ) -> (
        StorePhaseKey,
        StoreDimensionKey,
        StoreDimensionKey,
        RetainedStoreBatchLimits,
    ) {
        (
            self.phase,
            self.capacity_dimension,
            self.attribution_dimension,
            self.limits,
        )
    }
}

/// Work dimensions charged before a same-account retained handle is shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreSharePlan {
    clone_dimension: StoreDimensionKey,
    retained_handle_dimension: StoreDimensionKey,
}

impl RetainedStoreSharePlan {
    /// Creates the work plan used by every inherited block share.
    pub const fn new(
        clone_dimension: StoreDimensionKey,
        retained_handle_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            clone_dimension,
            retained_handle_dimension,
        }
    }

    const fn into_raw(self) -> RetainedHandleSharePlan<StoreDimensionKey> {
        RetainedHandleSharePlan::new(self.clone_dimension, self.retained_handle_dimension)
    }

    fn contains(self, dimension: StoreDimensionKey) -> bool {
        self.clone_dimension == dimension || self.retained_handle_dimension == dimension
    }
}

/// Finite terminal work reserved for rich-failure evidence publication.
///
/// The dimension must be present in the operation's [`StoreLedgerPlan`] and distinct from every
/// ordinary start dimension. Startup preflights the exact upper bound for the publication account,
/// evidence table root, every admitted evidence member, and the declared construction and
/// validation envelopes without consuming the counters. Normal control, writer, and commitment
/// APIs cannot spend this dimension.
#[derive(Debug, PartialEq, Eq)]
pub struct RetainedStoreFailureTerminalWorkPlan {
    dimension: StoreDimensionKey,
    max_evidence_members: usize,
    construction_work_limit: u64,
    validation_work_limit: u64,
    requirement: RetainedStoreFailureTerminalWorkRequirement,
    ordinary_attempted_work_limit: u64,
    ordinary_consumed_work_limit: u64,
}

/// Exact counter policy required by one bounded rich-failure terminal rail.
///
/// Callers include these counters inside their public total work budget, partition ordinary work
/// with checked subtraction, and install [`Self::dimension_policy`] on the protected dimension.
/// The operation crate owns the derivation from retained publication shape.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "the protected terminal requirement must be included in the caller's hard budget"]
pub struct RetainedStoreFailureTerminalWorkRequirement {
    attempted_work_limit: u64,
    consumed_work_limit: u64,
}

impl RetainedStoreFailureTerminalWorkRequirement {
    /// Returns the attempted-work portion reserved from the caller's total hard limit.
    pub const fn attempted_work_limit(&self) -> u64 {
        self.attempted_work_limit
    }

    /// Returns the consumed-work portion reserved from the caller's total hard limit.
    pub const fn consumed_work_limit(&self) -> u64 {
        self.consumed_work_limit
    }

    /// Returns the counter-only policy for the protected terminal dimension.
    pub const fn dimension_policy(&self) -> DimensionPolicy {
        DimensionPolicy::counter(self.attempted_work_limit, self.consumed_work_limit, 0)
    }
}

impl RetainedStoreFailureTerminalWorkPlan {
    /// Derives the exact protected work requirement without requiring a runtime dimension key.
    pub fn requirement_for_evidence_members(
        max_evidence_members: usize,
    ) -> Result<RetainedStoreFailureTerminalWorkRequirement, StorePlanError> {
        Self::requirement_for_evidence_work(max_evidence_members, 0, 0)
    }

    /// Derives the exact protected work required by retained shares, evidence construction, and
    /// evidence validation without requiring a runtime dimension key.
    pub fn requirement_for_evidence_work(
        max_evidence_members: usize,
        construction_work_limit: u64,
        validation_work_limit: u64,
    ) -> Result<RetainedStoreFailureTerminalWorkRequirement, StorePlanError> {
        let raw = RetainedFailureTerminalWorkPlan::with_evidence_work(
            (),
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
        );
        let requirement = raw
            .requirement()
            .map_err(|()| StorePlanError::InvalidFailureTerminalWork)?;
        Ok(RetainedStoreFailureTerminalWorkRequirement {
            attempted_work_limit: requirement.observed_work_limit(),
            consumed_work_limit: requirement.observed_work_limit(),
        })
    }

    /// Declares the dedicated terminal dimension and maximum rich-failure evidence members.
    pub fn new(
        dimension: StoreDimensionKey,
        max_evidence_members: usize,
        total_attempted_work_limit: u64,
        total_consumed_work_limit: u64,
    ) -> Result<Self, StorePlanError> {
        Self::with_evidence_work(
            dimension,
            max_evidence_members,
            0,
            0,
            total_attempted_work_limit,
            total_consumed_work_limit,
        )
    }

    /// Declares one protected terminal dimension with independent construction and validation
    /// work envelopes. Both envelopes remain inside the caller's total hard work cap.
    pub fn with_evidence_work(
        dimension: StoreDimensionKey,
        max_evidence_members: usize,
        construction_work_limit: u64,
        validation_work_limit: u64,
        total_attempted_work_limit: u64,
        total_consumed_work_limit: u64,
    ) -> Result<Self, StorePlanError> {
        let requirement = Self::requirement_for_evidence_work(
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
        )?;
        let ordinary_attempted_work_limit = total_attempted_work_limit
            .checked_sub(requirement.attempted_work_limit())
            .ok_or(StorePlanError::InvalidFailureTerminalWork)?;
        let ordinary_consumed_work_limit = total_consumed_work_limit
            .checked_sub(requirement.consumed_work_limit())
            .ok_or(StorePlanError::InvalidFailureTerminalWork)?;
        Ok(Self {
            dimension,
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
            requirement,
            ordinary_attempted_work_limit,
            ordinary_consumed_work_limit,
        })
    }

    /// Returns the exact protected work requirement for this admitted plan.
    pub const fn requirement(&self) -> &RetainedStoreFailureTerminalWorkRequirement {
        &self.requirement
    }

    /// Returns the maximum attempted work available to every ordinary dimension combined.
    pub const fn ordinary_attempted_work_limit(&self) -> u64 {
        self.ordinary_attempted_work_limit
    }

    /// Returns the maximum consumed work available to every ordinary dimension combined.
    pub const fn ordinary_consumed_work_limit(&self) -> u64 {
        self.ordinary_consumed_work_limit
    }

    pub(super) const fn dimension(&self) -> StoreDimensionKey {
        self.dimension
    }

    const fn into_raw(self) -> RetainedFailureTerminalWorkPlan<StoreDimensionKey> {
        RetainedFailureTerminalWorkPlan::with_evidence_work(
            self.dimension,
            self.max_evidence_members,
            self.construction_work_limit,
            self.validation_work_limit,
        )
    }
}

/// Start-time policy for the operation-owned concrete retained store.
#[derive(Debug, PartialEq, Eq)]
pub struct RetainedStoreStartPlan {
    terminal_writer: RetainedStoreWriterPlan,
    share: RetainedStoreSharePlan,
    fallback: StoreFallbackEnvelope,
    fallback_dimension: StoreDimensionKey,
    fallback_amount: u64,
    snapshot_dimension: StoreDimensionKey,
    snapshot_amount: u64,
    failure_terminal_work: RetainedStoreFailureTerminalWorkPlan,
}

pub(super) struct RawRetainedStoreStartParts {
    pub(super) terminal_writer: RetainedWriterPlan<StorePhaseKey, StoreDimensionKey>,
    pub(super) share: RetainedHandleSharePlan<StoreDimensionKey>,
    pub(super) fallback: StoreFallbackEnvelope,
    pub(super) fallback_dimension: StoreDimensionKey,
    pub(super) fallback_amount: u64,
    pub(super) snapshot_dimension: StoreDimensionKey,
    pub(super) snapshot_amount: u64,
    pub(super) failure_terminal_work: RetainedFailureTerminalWorkPlan<StoreDimensionKey>,
}

impl RetainedStoreStartPlan {
    /// Creates start policy for the pre-admitted fallback and frozen ledger evidence.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        terminal_writer: RetainedStoreWriterPlan,
        share: RetainedStoreSharePlan,
        fallback: StoreFallbackEnvelope,
        fallback_dimension: StoreDimensionKey,
        fallback_amount: u64,
        snapshot_dimension: StoreDimensionKey,
        snapshot_amount: u64,
        failure_terminal_work: RetainedStoreFailureTerminalWorkPlan,
    ) -> Self {
        Self {
            terminal_writer,
            share,
            fallback,
            fallback_dimension,
            fallback_amount,
            snapshot_dimension,
            snapshot_amount,
            failure_terminal_work,
        }
    }

    pub(super) const fn keys_for_start_validation(
        &self,
    ) -> (StorePhaseKey, [StoreDimensionKey; 6], StoreDimensionKey) {
        (
            self.terminal_writer.phase,
            [
                self.terminal_writer.capacity_dimension,
                self.terminal_writer.attribution_dimension,
                self.share.clone_dimension,
                self.share.retained_handle_dimension,
                self.fallback_dimension,
                self.snapshot_dimension,
            ],
            self.failure_terminal_work.dimension(),
        )
    }

    pub(super) fn failure_terminal_work_is_separate(&self) -> bool {
        let dimension = self.failure_terminal_work.dimension();
        self.terminal_writer.capacity_dimension != dimension
            && self.terminal_writer.attribution_dimension != dimension
            && !self.share.contains(dimension)
            && self.fallback_dimension != dimension
            && self.snapshot_dimension != dimension
    }

    pub(super) fn failure_terminal_budget_matches(&self, ledger_plan: &StoreLedgerPlan) -> bool {
        ledger_plan.failure_terminal_budget_matches(&self.failure_terminal_work)
    }

    pub(super) const fn into_raw_start_parts(self) -> RawRetainedStoreStartParts {
        RawRetainedStoreStartParts {
            terminal_writer: self.terminal_writer.into_raw(),
            share: self.share.into_raw(),
            fallback: self.fallback,
            fallback_dimension: self.fallback_dimension,
            fallback_amount: self.fallback_amount,
            snapshot_dimension: self.snapshot_dimension,
            snapshot_amount: self.snapshot_amount,
            failure_terminal_work: self.failure_terminal_work.into_raw(),
        }
    }
}

/// Fixed table capacity and maximum new allocations for one publication destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStoreBlockTablePlan {
    writer: RetainedStoreWriterPlan,
    total_blocks: usize,
    max_new_blocks: usize,
}

impl RetainedStoreBlockTablePlan {
    /// Creates a non-empty table plan and rejects impossible new-block bounds.
    pub fn new(
        writer: RetainedStoreWriterPlan,
        total_blocks: usize,
        max_new_blocks: usize,
    ) -> Result<Self, StorePlanError> {
        let required_attributions = max_new_blocks
            .checked_add(1)
            .ok_or(StorePlanError::InvalidBlockTable)?;
        if total_blocks == 0
            || max_new_blocks > total_blocks
            || u64::try_from(total_blocks).is_err()
            || required_attributions > writer.limits.attribution_slots
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let writer = RetainedStoreWriterPlan::new(
            writer.phase,
            writer.capacity_dimension,
            writer.attribution_dimension,
            RetainedStoreBatchLimits::new(required_attributions, writer.limits.commitment_slots)?,
        );
        Ok(Self {
            writer,
            total_blocks,
            max_new_blocks,
        })
    }

    const fn into_raw(self) -> RetainedBlockTablePlan<StorePhaseKey, StoreDimensionKey> {
        RetainedBlockTablePlan::new(
            self.writer.into_raw(),
            self.total_blocks,
            self.max_new_blocks,
        )
    }

    pub(super) const fn keys_for_publication_validation(
        self,
    ) -> (StorePhaseKey, [StoreDimensionKey; 2]) {
        self.writer.keys_for_publication_validation()
    }

    pub(super) const fn writer(self) -> RetainedStoreWriterPlan {
        self.writer
    }
}

/// Independent canonical and outcome-evidence table plans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStorePublicationPlan {
    canonical: Option<RetainedStoreBlockTablePlan>,
    outcome_evidence: RetainedStoreBlockTablePlan,
}

impl RetainedStorePublicationPlan {
    /// Creates a success-capable dual-table publication plan.
    pub const fn success(
        canonical: RetainedStoreBlockTablePlan,
        outcome_evidence: RetainedStoreBlockTablePlan,
    ) -> Self {
        Self {
            canonical: Some(canonical),
            outcome_evidence,
        }
    }

    /// Creates an evidence-only publication plan.
    pub const fn rich_failure(outcome_evidence: RetainedStoreBlockTablePlan) -> Self {
        Self {
            canonical: None,
            outcome_evidence,
        }
    }

    pub(super) const fn is_success_capable(&self) -> bool {
        self.canonical.is_some()
    }

    pub(super) const fn tables_for_validation(&self) -> [Option<RetainedStoreBlockTablePlan>; 2] {
        [self.canonical, Some(self.outcome_evidence)]
    }

    pub(super) const fn into_raw(
        self,
    ) -> RetainedPublicationPlan<StorePhaseKey, StoreDimensionKey> {
        match self.canonical {
            Some(canonical) => RetainedPublicationPlan::success(
                canonical.into_raw(),
                self.outcome_evidence.into_raw(),
            ),
            None => RetainedPublicationPlan::rich_failure(self.outcome_evidence.into_raw()),
        }
    }
}
