use crate::diagnostic::DiagnosticSource;
use crate::error::{CauseOwner, ScopeAbortCause};
use crate::ledger::{ImportDimension, ImportLedgerPhase};
use crate::metered_storage::AccountedVec;
use crate::resource_graph::ManifestDigest;
use crate::resource_hash::chunked_sha256;
use crate::store::{ImportStoreHost, ImportStoreReservation};
use baozi_io::__semantic_identity::{
    RegisteredAssetResolution, RegisteredResolverProfile, RegisteredUnavailableCause,
    StableResourceIdentitySchema, VerifiedNegativeInputProof, VerifiedNegativeInputProofBinding,
};
use baozi_io::{AssetIo, AssetOpenErrorKind, AssetPath, AssetReadErrorKind, AuthorityDomain};
use baozi_operation::{CapacityPlan, CapacityPlanError, LedgerCause, ReplacementCapacityPlan};
use std::collections::hash_map::RandomState;
use std::error::Error;
use std::fmt;
use std::hash::{BuildHasher, Hash};
use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::{Arc, OnceLock};

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

    /// Returns the backend namespace that qualifies this logical path.
    pub fn authority(&self) -> &AuthorityDomain {
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

/// Owned context for one failed required dependency request.
///
/// The origin and target identities retain their backend namespaces. The normalized path is the
/// logical reference before resolver redirection; `target_identity` is the authorized open target.
/// Invalid references longer than [`AssetPath::MAX_BYTES`] retain a UTF-8 prefix and their full
/// byte length. Reading this value never allocates or performs IO.
#[derive(Debug)]
pub struct DependencyFailureContext {
    details: Vec<DependencyFailureDetails>,
}

#[derive(Debug)]
struct DependencyFailureDetails {
    origin: DependencyIdentity,
    role: DependencyRoleId,
    requested_reference: String,
    requested_reference_bytes: usize,
    normalized_path: Option<AssetPath>,
    target_identity: Option<DependencyIdentity>,
    cause: ScopeAbortCause,
}

impl DependencyFailureContext {
    const fn details(&self) -> &DependencyFailureDetails {
        &self.details.as_slice()[0]
    }

    /// The resource that declared the failing reference, rather than the primary import source.
    pub fn origin(&self) -> &DependencyIdentity {
        &self.details().origin
    }

    pub fn role(&self) -> &DependencyRoleId {
        &self.details().role
    }

    /// The caller's reference spelling, bounded to a complete UTF-8 prefix.
    pub fn requested_reference(&self) -> &str {
        &self.details().requested_reference
    }

    pub const fn requested_reference_bytes(&self) -> usize {
        self.details().requested_reference_bytes
    }

    pub fn requested_reference_is_truncated(&self) -> bool {
        self.requested_reference().len() != self.requested_reference_bytes()
    }

    /// The normalized logical reference, absent when normalization failed.
    pub fn normalized_path(&self) -> Option<&AssetPath> {
        self.details().normalized_path.as_ref()
    }

    /// The resolver-authorized resource, absent when authorization failed.
    pub fn target_identity(&self) -> Option<&DependencyIdentity> {
        self.details().target_identity.as_ref()
    }

    pub(crate) const fn cause(&self) -> &ScopeAbortCause {
        &self.details().cause
    }

    pub(crate) fn attach_cause(&mut self, cause: ScopeAbortCause) {
        self.details[0].cause = cause;
    }

    pub(crate) fn try_capture(
        host: ImportStoreHost<'_>,
        origin_authority: &AuthorityDomain,
        origin_path: &AssetPath,
        role: &DependencyRoleId,
        requested_reference: &str,
        normalized_path: Option<&AssetPath>,
        target_identity: Option<&DependencyIdentity>,
    ) -> Result<(Self, ImportStoreReservation), ScopeAbortCause> {
        const RESOURCE: &str = "required-dependency-failure-context";
        let capacity_error = |cause| ScopeAbortCause::CapacityEnvelope {
            resource: RESOURCE,
            cause,
        };
        let allocation_error = || ScopeAbortCause::Allocation { resource: RESOURCE };
        let mut end = requested_reference.len().min(AssetPath::MAX_BYTES);
        while !requested_reference.is_char_boundary(end) {
            end -= 1;
        }
        let requested_prefix = &requested_reference[..end];
        let text = [
            origin_path.as_str(),
            requested_prefix,
            normalized_path.map_or("", AssetPath::as_str),
            target_identity.map_or("", |identity| identity.path().as_str()),
        ];
        let detail_plan =
            CapacityPlan::for_type::<DependencyFailureDetails>(0, 0, 1).map_err(capacity_error)?;
        let text_plans = text.map(|text| CapacityPlan::for_type::<u8>(0, 0, text.len()));
        let mut envelope = detail_plan.envelope_delta_bytes().map_err(capacity_error)?;
        for plan in &text_plans {
            let bytes = plan
                .as_ref()
                .map_err(|cause| capacity_error(*cause))?
                .envelope_delta_bytes()
                .map_err(capacity_error)?;
            envelope = envelope
                .checked_add(bytes)
                .ok_or_else(|| capacity_error(CapacityPlanError::CapacityOverflow))?;
        }
        // One terminal-failure owner retains every backing allocation, including paths. Admission
        // precedes allocation and the lease moves with the actual terminal cause.
        let mut reservation = host
            .reserve_retained(
                ImportLedgerPhase::DependencyAcquisition,
                ImportDimension::ScratchBytes,
                envelope,
                1 + text.iter().filter(|text| !text.is_empty()).count() as u64,
            )
            .map_err(|cause| cause.into_scope_abort_cause())?;
        let mut details = Vec::new();
        details
            .try_reserve_exact(detail_plan.allocator_additional())
            .map_err(|_| allocation_error())?;
        detail_plan
            .finish(0, details.capacity())
            .map_err(capacity_error)?;
        let origin = DependencyIdentity::new(
            origin_authority.clone(),
            origin_path.try_clone().map_err(|_| allocation_error())?,
        );
        let mut requested = String::new();
        requested
            .try_reserve_exact(requested_prefix.len())
            .map_err(|_| allocation_error())?;
        requested.push_str(requested_prefix);
        let normalized = normalized_path
            .map(AssetPath::try_clone)
            .transpose()
            .map_err(|_| allocation_error())?;
        let target = target_identity
            .map(DependencyIdentity::try_clone)
            .transpose()
            .map_err(|_| allocation_error())?;
        let capacities = [
            origin.path().capacity_bytes(),
            requested.capacity(),
            normalized.as_ref().map_or(0, AssetPath::capacity_bytes),
            target
                .as_ref()
                .map_or(0, |identity| identity.path().capacity_bytes()),
        ];
        let mut actual = u64::try_from(details.capacity())
            .ok()
            .and_then(|capacity| capacity.checked_mul(size_of::<DependencyFailureDetails>() as u64))
            .ok_or_else(|| capacity_error(CapacityPlanError::CapacityOverflow))?;
        for (plan, capacity) in text_plans.into_iter().zip(capacities) {
            plan.map_err(capacity_error)?
                .finish(0, capacity)
                .map_err(capacity_error)?;
            actual = actual
                .checked_add(capacity as u64)
                .ok_or_else(|| capacity_error(CapacityPlanError::CapacityOverflow))?;
        }
        host.resize(&mut reservation, actual)
            .map_err(|cause| cause.into_scope_abort_cause())?;
        details.push(DependencyFailureDetails {
            origin,
            role: role.clone(),
            requested_reference: requested,
            requested_reference_bytes: requested_reference.len(),
            normalized_path: normalized,
            target_identity: target,
            cause: ScopeAbortCause::DescriptorContract {
                code: "dependency-failure-context-not-attached",
            },
        });
        Ok((Self { details }, reservation))
    }
}

/// A resolver decision that binds authorization to the backend used for open.
pub struct AuthorizedDependency<'a> {
    io: &'a dyn AssetIo,
    identity: DependencyIdentity,
    stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
    registered_resolver: Option<&'static RegisteredResolverProfile>,
}

impl<'a> AuthorizedDependency<'a> {
    pub fn new(io: &'a dyn AssetIo, authority: AuthorityDomain, path: AssetPath) -> Self {
        let identity = DependencyIdentity::new(authority, path);
        Self {
            io,
            identity,
            stable_identity_schema: None,
            registered_resolver: None,
        }
    }

    pub(crate) fn from_registered(resolution: RegisteredAssetResolution<'a>) -> Self {
        let registered_resolver = resolution.resolver();
        let (io, authority, stable_identity) = resolution.into_parts();
        let schema = stable_identity.schema();
        let path = stable_identity.into_value();
        Self {
            io,
            identity: DependencyIdentity::new(authority, path),
            stable_identity_schema: Some(schema),
            registered_resolver: Some(registered_resolver),
        }
    }

    pub fn identity(&self) -> &DependencyIdentity {
        &self.identity
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        &'a dyn AssetIo,
        DependencyIdentity,
        Option<&'static StableResourceIdentitySchema>,
        Option<&'static RegisteredResolverProfile>,
    ) {
        (
            self.io,
            self.identity,
            self.stable_identity_schema,
            self.registered_resolver,
        )
    }
}

/// The import session's resolver authority. Built-in profiles are the only source of admitted
/// stable semantic identity; custom implementations remain session-opaque.
#[derive(Clone, Copy)]
pub(crate) enum ResolverBinding<'a> {
    Registered(&'static RegisteredResolverProfile),
    Custom(&'a dyn DependencyResolver),
}

impl<'a> ResolverBinding<'a> {
    pub(crate) const fn registered(
        profile: &'static RegisteredResolverProfile,
    ) -> ResolverBinding<'static> {
        ResolverBinding::Registered(profile)
    }

    pub(crate) const fn custom(resolver: &'a dyn DependencyResolver) -> Self {
        Self::Custom(resolver)
    }
}

/// Authorizes a normalized logical dependency before cache lookup or backend IO.
///
/// `origin` is the immutable asset snapshot that declared `reference`. For a
/// direct dependency it is the primary asset; for a nested dependency it is
/// the already-authorized dependency snapshot supplied by the import session.
pub trait DependencyResolver: Send + Sync {
    fn authorize<'a>(
        &'a self,
        origin: &'a dyn AssetIo,
        origin_authority: &AuthorityDomain,
        origin_path: &AssetPath,
        role: &DependencyRoleId,
        reference: DependencyReference<'_>,
    ) -> Result<AuthorizedDependency<'a>, DependencyPolicyError>;
}

/// Denies every external dependency before backend IO.
#[derive(Debug, Default)]
pub struct DenyDependencies;

impl DependencyResolver for DenyDependencies {
    fn authorize<'a>(
        &'a self,
        _origin: &'a dyn AssetIo,
        _origin_authority: &AuthorityDomain,
        _origin_path: &AssetPath,
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

pub(crate) struct DependencyOriginIssuer {
    _identity: u8,
}

impl DependencyOriginIssuer {
    pub(crate) const fn new() -> Self {
        Self { _identity: 0 }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum DependencyOriginNode {
    Primary,
    Dependency(usize),
}

/// An opaque, session-issued capability naming the resource that declared a dependency.
///
/// The handle intentionally exposes no path, identity, ordering, hashing, serialization, or
/// value-revealing debug interface. It is only valid when returned to the issuing parse scope.
pub struct DependencyOrigin<'scope> {
    issuer: &'scope DependencyOriginIssuer,
    io: &'scope dyn AssetIo,
    node: DependencyOriginNode,
}

impl<'scope> DependencyOrigin<'scope> {
    pub(crate) const fn primary(
        issuer: &'scope DependencyOriginIssuer,
        io: &'scope dyn AssetIo,
    ) -> Self {
        Self {
            issuer,
            io,
            node: DependencyOriginNode::Primary,
        }
    }

    const fn dependency(
        issuer: &'scope DependencyOriginIssuer,
        io: &'scope dyn AssetIo,
        node: usize,
    ) -> Self {
        Self {
            issuer,
            io,
            node: DependencyOriginNode::Dependency(node),
        }
    }

    pub(crate) fn is_issued_by(&self, issuer: &DependencyOriginIssuer) -> bool {
        std::ptr::eq(self.issuer, issuer)
    }

    pub(crate) const fn io(&self) -> &'scope dyn AssetIo {
        self.io
    }

    pub(crate) const fn node(&self) -> DependencyOriginNode {
        self.node
    }
}

/// Immutable bytes borrowed from one import session's dependency cache.
pub struct DependencySnapshot<'scope> {
    issuer: &'scope DependencyOriginIssuer,
    origin_io: &'scope dyn AssetIo,
    origin_node: usize,
    identity: DependencySnapshotIdentity,
    bytes: Arc<SharedDependencySnapshot>,
    cache_disposition: DependencyCacheDisposition,
    scope: PhantomData<&'scope ()>,
}

pub(crate) struct DependencySnapshotIdentity {
    exact: DependencyIdentity,
    stable_schema: Option<&'static StableResourceIdentitySchema>,
    _path_reservation: ImportStoreReservation,
}

impl DependencySnapshotIdentity {
    pub(crate) const fn new(
        exact: DependencyIdentity,
        stable_schema: Option<&'static StableResourceIdentitySchema>,
        path_reservation: ImportStoreReservation,
    ) -> Self {
        Self {
            exact,
            stable_schema,
            _path_reservation: path_reservation,
        }
    }
}

impl<'scope> DependencySnapshot<'scope> {
    pub(crate) fn new(
        issuer: &'scope DependencyOriginIssuer,
        origin_io: &'scope dyn AssetIo,
        origin_node: usize,
        identity: DependencySnapshotIdentity,
        bytes: Arc<SharedDependencySnapshot>,
        cache_disposition: DependencyCacheDisposition,
    ) -> Self {
        Self {
            issuer,
            origin_io,
            origin_node,
            identity,
            bytes,
            cache_disposition,
            scope: PhantomData,
        }
    }

    pub fn identity(&self) -> &DependencyIdentity {
        &self.identity.exact
    }

    pub(crate) const fn stable_identity_schema(
        &self,
    ) -> Option<&'static StableResourceIdentitySchema> {
        self.identity.stable_schema
    }

    /// Returns the opaque origin capability for dependencies declared by this snapshot.
    pub fn origin(&self) -> DependencyOrigin<'scope> {
        DependencyOrigin::dependency(self.issuer, self.origin_io, self.origin_node)
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.bytes()
    }

    pub const fn cache_disposition(&self) -> DependencyCacheDisposition {
        self.cache_disposition
    }

    pub fn diagnostic_source(&self) -> DiagnosticSource<'_> {
        DiagnosticSource::new(
            self.identity.exact.authority().clone(),
            self.identity.exact.path(),
            self.bytes(),
        )
    }
}

pub(crate) struct SharedDependencySnapshot {
    bytes: AccountedVec<u8>,
    content_digest: OnceLock<ManifestDigest>,
    _sharing_reservation: ImportStoreReservation,
}

pub(crate) fn shared_dependency_snapshot_allocation_bytes() -> Option<usize> {
    const ARC_CONTROL_WORDS: usize = 4;
    size_of::<SharedDependencySnapshot>()
        .checked_add(size_of::<usize>().checked_mul(ARC_CONTROL_WORDS)?)
}

impl SharedDependencySnapshot {
    pub(crate) fn new(
        bytes: AccountedVec<u8>,
        sharing_reservation: ImportStoreReservation,
    ) -> Self {
        Self {
            bytes,
            content_digest: OnceLock::new(),
            _sharing_reservation: sharing_reservation,
        }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn content_digest(
        &self,
        host: ImportStoreHost<'_>,
    ) -> Result<ManifestDigest, ScopeAbortCause> {
        if let Some(digest) = self.content_digest.get() {
            return Ok(*digest);
        }
        // ResourceGraph serializes access to this session's immutable snapshots. Publish only a
        // complete digest; cancellation or rejected work admission must not seal a partial hash.
        let digest = chunked_sha256(host, self.bytes(), ImportDimension::WorkUnits)
            .map(ManifestDigest::from_bytes)?;
        Ok(*self.content_digest.get_or_init(|| digest))
    }
}

pub struct OptionalDependencyEvidence {
    identity: Option<DependencyIdentity>,
    normalized_path: AssetPath,
    owner: CauseOwner,
    cache_disposition: DependencyCacheDisposition,
    _identity_path_reservation: Option<ImportStoreReservation>,
    _normalized_path_reservation: Option<ImportStoreReservation>,
}

impl fmt::Debug for OptionalDependencyEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OptionalDependencyEvidence")
            .field("identity", &self.identity)
            .field("normalized_path", &self.normalized_path)
            .field("owner", &self.owner)
            .field("cache_disposition", &self.cache_disposition)
            .field(
                "identity_path_reserved",
                &self._identity_path_reservation.is_some(),
            )
            .field(
                "normalized_path_reserved",
                &self._normalized_path_reservation.is_some(),
            )
            .finish()
    }
}

impl OptionalDependencyEvidence {
    pub(crate) fn new(
        identity: Option<DependencyIdentity>,
        normalized_path: AssetPath,
        owner: CauseOwner,
        cache_disposition: DependencyCacheDisposition,
        identity_path_reservation: Option<ImportStoreReservation>,
        normalized_path_reservation: ImportStoreReservation,
    ) -> Self {
        Self {
            identity,
            normalized_path,
            owner,
            cache_disposition,
            _identity_path_reservation: identity_path_reservation,
            _normalized_path_reservation: Some(normalized_path_reservation),
        }
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

pub struct DependencyOutcome<'scope> {
    inner: DependencyOutcomeKind<'scope>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "linear snapshot and optional evidence retain their admitted path owners without another failure-path allocation"
)]
enum DependencyOutcomeKind<'scope> {
    Available(DependencySnapshot<'scope>),
    Unavailable(OptionalDependencyEvidence),
}

impl<'scope> DependencyOutcome<'scope> {
    pub(crate) fn available(snapshot: DependencySnapshot<'scope>) -> Self {
        Self {
            inner: DependencyOutcomeKind::Available(snapshot),
        }
    }

    pub(crate) fn unavailable(evidence: OptionalDependencyEvidence) -> Self {
        Self {
            inner: DependencyOutcomeKind::Unavailable(evidence),
        }
    }

    pub fn snapshot(&self) -> Option<&DependencySnapshot<'scope>> {
        match &self.inner {
            DependencyOutcomeKind::Available(snapshot) => Some(snapshot),
            DependencyOutcomeKind::Unavailable(_) => None,
        }
    }

    pub fn unavailable_evidence(&self) -> Option<&OptionalDependencyEvidence> {
        match &self.inner {
            DependencyOutcomeKind::Available(_) => None,
            DependencyOutcomeKind::Unavailable(evidence) => Some(evidence),
        }
    }

    /// Consumes the outcome, preserving ownership of either the admitted snapshot or its
    /// structured unavailable evidence.
    #[allow(
        clippy::result_large_err,
        reason = "dependency evidence remains inline so unavailable outcomes need no failure-path allocation"
    )]
    pub fn into_result(self) -> Result<DependencySnapshot<'scope>, OptionalDependencyEvidence> {
        match self.inner {
            DependencyOutcomeKind::Available(snapshot) => Ok(snapshot),
            DependencyOutcomeKind::Unavailable(evidence) => Err(evidence),
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
    entry_capacity_reservation: Option<ImportStoreReservation>,
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
        let entry_replacement = if self.free_entry.is_none()
            && self.entries.len() == self.entries.capacity()
        {
            let required_capacity = self.entries.len().checked_add(1).ok_or(
                DependencyCacheCapacityError::Capacity {
                    storage: DependencyCacheStorage::Entries,
                    cause: CapacityPlanError::CapacityOverflow,
                },
            )?;
            Some(
                ReplacementCapacityPlan::for_type::<DependencyCacheRecordSlot>(
                    self.entries.len(),
                    self.entries.capacity(),
                    required_capacity,
                )
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
                entry_plan
                    .verify_source(self.entries.len(), self.entries.capacity())
                    .map_err(|cause| DependencyCacheCapacityError::Capacity {
                        storage: DependencyCacheStorage::Entries,
                        cause,
                    })?;
                let candidate_plan = entry_plan.candidate();
                let mut entries = Vec::new();
                candidate_plan
                    .verify_preflight(entries.len(), entries.capacity())
                    .map_err(|cause| DependencyCacheCapacityError::Capacity {
                        storage: DependencyCacheStorage::Entries,
                        cause,
                    })?;
                entries
                    .try_reserve_exact(candidate_plan.allocator_additional())
                    .map_err(|_| DependencyCacheCapacityError::Allocation {
                        storage: DependencyCacheStorage::Entries,
                    })?;
                let actual = candidate_plan
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
    ) -> Option<ImportStoreReservation> {
        match storage {
            DependencyCacheStorage::Entries => self.entry_capacity_reservation.take(),
            DependencyCacheStorage::Index => self.index.capacity_reservation.take(),
        }
    }

    pub(crate) fn retain_capacity_reservation(
        &mut self,
        storage: DependencyCacheStorage,
        reservation: Option<ImportStoreReservation>,
    ) {
        match storage {
            DependencyCacheStorage::Entries => self.entry_capacity_reservation = reservation,
            DependencyCacheStorage::Index => self.index.capacity_reservation = reservation,
        }
    }

    pub(crate) fn mark_resolving(
        &mut self,
        identity: DependencyIdentity,
        identity_path_reservation: ImportStoreReservation,
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
        bytes: Arc<SharedDependencySnapshot>,
        stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
    ) -> Option<usize> {
        let index = self.record_index(identity)?;
        let Some(DependencyCacheRecordSlot::Occupied(record)) = self.entries.get_mut(index) else {
            return None;
        };
        let entry = &mut record.entry;
        let DependencyCacheEntry::Resolving = entry else {
            return None;
        };
        *entry = DependencyCacheEntry::Available(CachedDependencySnapshot {
            bytes,
            stable_identity_schema,
        });
        Some(index)
    }

    pub(crate) fn available_identity(&self, index: usize) -> Option<&DependencyIdentity> {
        self.available_snapshot(index).map(|(identity, _)| identity)
    }

    pub(crate) fn available_snapshot(
        &self,
        index: usize,
    ) -> Option<(&DependencyIdentity, &CachedDependencySnapshot)> {
        let DependencyCacheRecordSlot::Occupied(record) = self.entries.get(index)? else {
            return None;
        };
        match &record.entry {
            DependencyCacheEntry::Available(snapshot) => Some((&record.identity, snapshot)),
            DependencyCacheEntry::Resolving | DependencyCacheEntry::Failed(_) => None,
        }
    }

    pub(crate) fn store_failed(
        &mut self,
        identity: &DependencyIdentity,
        cause: ScopeAbortCause,
        stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
        observation: PendingNegativeObservation<'_>,
    ) -> Result<usize, ScopeAbortCause> {
        let Some(index) = self.record_index(identity) else {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "dependency-cache-transition-missing",
            });
        };
        let Some(DependencyCacheRecordSlot::Occupied(record)) = self.entries.get_mut(index) else {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "dependency-cache-transition-missing",
            });
        };
        let entry = &mut record.entry;
        let DependencyCacheEntry::Resolving = entry else {
            return Err(ScopeAbortCause::DescriptorContract {
                code: "dependency-cache-transition-missing",
            });
        };
        let failure =
            CachedDependencyFailure::try_new(identity, cause, stable_identity_schema, observation)?;
        *entry = DependencyCacheEntry::Failed(failure);
        Ok(index)
    }

    pub(crate) fn failed(&self, index: usize) -> Option<&CachedDependencyFailure> {
        match self.entries.get(index) {
            Some(DependencyCacheRecordSlot::Occupied(record)) => match &record.entry {
                DependencyCacheEntry::Failed(failure) => Some(failure),
                DependencyCacheEntry::Resolving | DependencyCacheEntry::Available(_) => None,
            },
            Some(DependencyCacheRecordSlot::Vacant { .. }) | None => None,
        }
    }

    pub(crate) fn remove_resolving(&mut self, identity: &DependencyIdentity) -> bool {
        let probe = self.probe_identity(identity);
        let (Some(record_index), Some(index_slot)) = (probe.record_index, probe.matched_slot)
        else {
            return false;
        };
        let Some(slot) = self.entries.get_mut(record_index) else {
            return false;
        };
        let DependencyCacheRecordSlot::Occupied(record) = slot else {
            return false;
        };
        if !matches!(&record.entry, DependencyCacheEntry::Resolving) {
            return false;
        }
        *slot = DependencyCacheRecordSlot::Vacant {
            next: self.free_entry,
        };
        self.free_entry = Some(record_index);
        self.entry_count -= 1;
        self.index.remove(index_slot);
        true
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

// The cache reservation covers `size_of::<DependencyCacheRecordSlot>()` for
// every slot. Keeping the bounded failure evidence inline avoids a second,
// unmetered heap allocation for occupied records.
#[allow(clippy::large_enum_variant)]
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
    _identity_path_reservation: ImportStoreReservation,
}

struct DependencyIndex {
    slots: Vec<usize>,
    tombstones: usize,
    hash_builder: RandomState,
    capacity_reservation: Option<ImportStoreReservation>,
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
    entry_replacement: Option<ReplacementCapacityPlan>,
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
        match storage {
            DependencyCacheStorage::Entries => self.entry_replacement.map_or(Ok(0), |plan| {
                plan.candidate_envelope_bytes()
                    .map_err(|cause| DependencyCacheCapacityError::Capacity { storage, cause })
            }),
            DependencyCacheStorage::Index => self.index_replacement.map_or(Ok(0), |replacement| {
                replacement
                    .allocation
                    .envelope_delta_bytes()
                    .map_err(|cause| DependencyCacheCapacityError::Capacity { storage, cause })
            }),
        }
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
    Failed(CachedDependencyFailure),
}

pub(crate) struct CachedDependencySnapshot {
    pub(crate) bytes: Arc<SharedDependencySnapshot>,
    pub(crate) stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
}

/// The first immutable unavailable result published for one physical dependency identity.
///
/// Construction accepts only failures that may be reused under the same authority-qualified cache
/// key. Trusted-backend contract failures and other session-terminal causes never enter this type.
#[derive(Debug)]
pub(crate) struct CachedDependencyFailure {
    cause: ScopeAbortCause,
    unavailable: CachedDependencyUnavailable,
    stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedDependencyUnavailable {
    EnvironmentDependent {
        code: CachedDependencyIoCode,
        observation: CachedNegativeObservation,
    },
    AcquisitionBudget,
    ProcessState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedDependencyIoCode {
    Missing,
    OpenFailed,
    ReadFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedNegativeObservation {
    None,
    Proof(VerifiedNegativeInputProof),
}

pub(crate) enum PendingNegativeObservation<'target> {
    None,
    Proof(VerifiedNegativeInputProofBinding<'target>),
}

impl CachedDependencyFailure {
    fn try_new(
        identity: &DependencyIdentity,
        cause: ScopeAbortCause,
        stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
        observation: PendingNegativeObservation<'_>,
    ) -> Result<Self, ScopeAbortCause> {
        let unavailable = match &cause {
            ScopeAbortCause::AssetOpen(error) => {
                let code = if matches!(error.kind(), AssetOpenErrorKind::NotFound) {
                    CachedDependencyIoCode::Missing
                } else {
                    CachedDependencyIoCode::OpenFailed
                };
                CachedDependencyUnavailable::EnvironmentDependent {
                    code,
                    observation: seal_negative_observation(
                        identity,
                        stable_identity_schema,
                        code,
                        observation,
                    )?,
                }
            }
            ScopeAbortCause::AssetRead(error)
                if !matches!(error.kind(), AssetReadErrorKind::ContractViolation(_)) =>
            {
                let code = CachedDependencyIoCode::ReadFailed;
                CachedDependencyUnavailable::EnvironmentDependent {
                    code,
                    observation: seal_negative_observation(
                        identity,
                        stable_identity_schema,
                        code,
                        observation,
                    )?,
                }
            }
            ScopeAbortCause::Ledger(LedgerCause::LimitExceeded { .. })
            | ScopeAbortCause::ResourceLimit { .. } => {
                if !matches!(observation, PendingNegativeObservation::None) {
                    return Err(ScopeAbortCause::DescriptorContract {
                        code: "negative-proof-non-environment-cause",
                    });
                }
                CachedDependencyUnavailable::AcquisitionBudget
            }
            ScopeAbortCause::Allocation { .. } => {
                if !matches!(observation, PendingNegativeObservation::None) {
                    return Err(ScopeAbortCause::DescriptorContract {
                        code: "negative-proof-non-environment-cause",
                    });
                }
                CachedDependencyUnavailable::ProcessState
            }
            _ => return Err(cause),
        };
        Ok(Self {
            cause,
            unavailable,
            stable_identity_schema,
        })
    }

    pub(crate) const fn cause(&self) -> &ScopeAbortCause {
        &self.cause
    }

    pub(crate) const fn unavailable(&self) -> CachedDependencyUnavailable {
        self.unavailable
    }

    pub(crate) const fn stable_identity_schema(
        &self,
    ) -> Option<&'static StableResourceIdentitySchema> {
        self.stable_identity_schema
    }

    pub(crate) fn replay_scope_cause(&self) -> ScopeAbortCause {
        match &self.cause {
            ScopeAbortCause::AssetOpen(error) => ScopeAbortCause::AssetOpen(error.clone()),
            ScopeAbortCause::AssetRead(error) => ScopeAbortCause::AssetRead(error.clone()),
            ScopeAbortCause::Ledger(cause @ LedgerCause::LimitExceeded { .. }) => {
                ScopeAbortCause::Ledger(cause.clone())
            }
            ScopeAbortCause::ResourceLimit { resource, limit } => ScopeAbortCause::ResourceLimit {
                resource,
                limit: *limit,
            },
            ScopeAbortCause::Allocation { resource } => ScopeAbortCause::Allocation { resource },
            _ => unreachable!("CachedDependencyFailure accepts only reusable dependency causes"),
        }
    }
}

fn seal_negative_observation(
    identity: &DependencyIdentity,
    stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
    code: CachedDependencyIoCode,
    observation: PendingNegativeObservation<'_>,
) -> Result<CachedNegativeObservation, ScopeAbortCause> {
    match observation {
        PendingNegativeObservation::None => Ok(CachedNegativeObservation::None),
        PendingNegativeObservation::Proof(binding) => {
            let identity_schema =
                stable_identity_schema.ok_or(ScopeAbortCause::DescriptorContract {
                    code: "registered-negative-proof-missing-stable-identity",
                })?;
            let proof = binding
                .seal_subject(
                    identity_schema,
                    identity.authority(),
                    identity.path(),
                    code.registered_cause(),
                )
                .map_err(|_| ScopeAbortCause::DescriptorContract {
                    code: "registered-negative-proof-cache-subject-mismatch",
                })?;
            Ok(CachedNegativeObservation::Proof(proof))
        }
    }
}

impl CachedDependencyIoCode {
    pub(crate) const fn registered_cause(self) -> RegisteredUnavailableCause {
        match self {
            Self::Missing => RegisteredUnavailableCause::Missing,
            Self::OpenFailed => RegisteredUnavailableCause::OpenFailed,
            Self::ReadFailed => RegisteredUnavailableCause::ReadFailed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{ImportDimension, ImportLedgerPhase, ImportOperationControl};
    use crate::limits::ResourceLimits;
    use crate::store::{
        ImportStoreBootstrapRequirements, ImportStoreHost, ImportStoreSessionOwner,
        try_import_store_bootstrap,
    };
    use baozi_io::__semantic_identity::same_authority_resolver_v1;
    use baozi_io::{
        AssetOpenError, AssetOpenStage, AssetReadContractViolation, AssetReadError,
        ImmutableMemoryAssetIo, MemoryAssetIo,
    };
    use baozi_operation::{
        PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits,
    };

    struct StoreHarness {
        owner: ImportStoreSessionOwner,
        account: PhysicalStorageAccount,
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
            drop(factory);
            Self { owner, account }
        }

        fn host(&self) -> ImportStoreHost<'_> {
            self.owner.host()
        }

        fn reserve_scratch(&self, bytes: u64) -> ImportStoreReservation {
            self.host()
                .reserve(
                    ImportLedgerPhase::DependencyAcquisition,
                    ImportDimension::ScratchBytes,
                    bytes,
                )
                .expect("test scratch reservation must be admitted")
        }

        fn live_scratch_bytes(&self) -> u64 {
            self.host()
                .dimension_state(ImportDimension::ScratchBytes)
                .expect("test scratch dimension must remain observable")
                .live_bytes()
        }

        fn assert_released(self) {
            let Self { owner, account } = self;
            drop(owner);
            let released = account.snapshot();
            assert_eq!(released.live_retained_bytes(), 0);
            assert_eq!(released.live_retained_allocations(), 0);
            assert!(!released.poisoned());
        }
    }

    fn indexed_identity(
        authority: &AuthorityDomain,
        prefix: &str,
        index: usize,
    ) -> DependencyIdentity {
        let path = AssetPath::new(format!("assets/{prefix}-{index:05}.bin"))
            .expect("generated dependency path must be valid");
        DependencyIdentity::new(authority.clone(), path)
    }

    fn missing_dependency_cause() -> ScopeAbortCause {
        ScopeAbortCause::AssetOpen(AssetOpenError::new(
            AssetOpenStage::Open,
            AssetOpenErrorKind::NotFound,
        ))
    }

    #[test]
    fn failure_context_bounds_utf8_and_keeps_all_owned_storage_admitted() {
        let store = StoreHarness::new();
        let baseline = store.live_scratch_bytes();
        let authority = AuthorityDomain::new();
        let source = AssetPath::new("models/materials/library.side").unwrap();
        let role = DependencyRoleId::new("payload.required").unwrap();
        let reference = "界".repeat(AssetPath::MAX_BYTES);
        let (context, reservation) = DependencyFailureContext::try_capture(
            store.host(),
            &authority,
            &source,
            &role,
            &reference,
            None,
            None,
        )
        .unwrap();
        assert_eq!(context.origin().path(), &source);
        assert_eq!(context.origin().authority(), &authority);
        assert_eq!(context.role(), &role);
        assert_eq!(context.requested_reference_bytes(), reference.len());
        assert!(context.requested_reference_is_truncated());
        assert_eq!(context.requested_reference().len(), 4095);
        assert!(reference.starts_with(context.requested_reference()));
        assert_eq!(store.live_scratch_bytes(), baseline + reservation.bytes());
        assert!(reservation.bytes() >= context.requested_reference().len() as u64);
        drop(context);
        drop(reservation);
        assert_eq!(store.live_scratch_bytes(), baseline);
        store.assert_released();
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
    fn shared_snapshot_allocation_counts_the_complete_arc_payload() {
        let expected = size_of::<SharedDependencySnapshot>()
            .checked_add(size_of::<usize>() * 4)
            .expect("shared snapshot fixture size");
        let actual = shared_dependency_snapshot_allocation_bytes()
            .expect("shared snapshot allocation must fit usize");

        assert_eq!(actual, expected);
        assert!(actual > size_of::<Vec<u8>>() + size_of::<usize>() * 4);
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

        let store = StoreHarness::new();
        let baseline = store.live_scratch_bytes();
        let mut entry_reservation = store.reserve_scratch(entry_envelope);
        let mut index_reservation = store.reserve_scratch(index_envelope);

        let actual = cache.apply_capacity_plan(plan).unwrap();
        let entry_bytes = actual
            .retained_bytes(DependencyCacheStorage::Entries)
            .unwrap();
        let index_bytes = actual
            .retained_bytes(DependencyCacheStorage::Index)
            .unwrap();
        store
            .host()
            .resize(&mut entry_reservation, entry_bytes)
            .unwrap();
        store
            .host()
            .resize(&mut index_reservation, index_bytes)
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

        assert_eq!(
            store.live_scratch_bytes(),
            baseline
                .checked_add(entry_bytes)
                .and_then(|bytes| bytes.checked_add(index_bytes))
                .unwrap()
        );
        drop(cache);
        assert_eq!(store.live_scratch_bytes(), baseline);
        store.assert_released();
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
    fn dependency_cache_reuses_vacant_resolving_records_without_moving_live_indices() {
        let store = StoreHarness::new();
        let authority = AuthorityDomain::new();
        let identities = (0..3)
            .map(|record_index| indexed_identity(&authority, "record", record_index))
            .collect::<Vec<_>>();
        let mut cache = DependencyCache::new();
        cache.entries = Vec::with_capacity(4);
        cache.index.slots = vec![DEPENDENCY_INDEX_EMPTY; 8];

        for identity in &identities {
            let reservation = store.reserve_scratch(1);
            assert!(
                cache
                    .mark_resolving(identity.try_clone().unwrap(), reservation)
                    .unwrap()
            );
        }
        assert_eq!(cache.get(&identities[0]).map(|(index, _)| index), Some(0));
        assert_eq!(cache.get(&identities[1]).map(|(index, _)| index), Some(1));
        assert_eq!(cache.get(&identities[2]).map(|(index, _)| index), Some(2));

        assert!(cache.remove_resolving(&identities[1]));
        assert!(cache.get(&identities[1]).is_none());
        assert_eq!(cache.get(&identities[0]).map(|(index, _)| index), Some(0));
        assert_eq!(cache.get(&identities[2]).map(|(index, _)| index), Some(2));

        let replacement = indexed_identity(&authority, "record-replacement", 0);
        let replacement_reservation = store.reserve_scratch(1);
        assert!(
            cache
                .mark_resolving(replacement.try_clone().unwrap(), replacement_reservation,)
                .unwrap()
        );
        assert_eq!(cache.get(&replacement).map(|(index, _)| index), Some(1));
        assert_eq!(cache.get(&identities[2]).map(|(index, _)| index), Some(2));
    }

    #[test]
    fn dependency_cache_never_reuses_an_available_origin_node() {
        let store = StoreHarness::new();
        let authority = AuthorityDomain::new();
        let identity = indexed_identity(&authority, "available", 0);
        let mut cache = DependencyCache::new();
        cache.entries = Vec::with_capacity(2);
        cache.index.slots = vec![DEPENDENCY_INDEX_EMPTY; 8];

        assert!(
            cache
                .mark_resolving(identity.try_clone().unwrap(), store.reserve_scratch(1),)
                .unwrap()
        );
        let bytes = Arc::new(SharedDependencySnapshot::new(
            AccountedVec::new(),
            store.reserve_scratch(1),
        ));
        assert_eq!(cache.store_available(&identity, bytes, None), Some(0));

        assert!(!cache.remove_resolving(&identity));
        assert_eq!(cache.available_identity(0), Some(&identity));
    }

    #[test]
    fn dependency_failure_cache_replays_one_immutable_negative_observation() {
        let store = StoreHarness::new();
        let authority = AuthorityDomain::new();
        let identity = indexed_identity(&authority, "missing", 0);
        let mut cache = DependencyCache::new();
        cache.entries = Vec::with_capacity(1);
        cache.index.slots = vec![DEPENDENCY_INDEX_EMPTY; 8];
        assert!(
            cache
                .mark_resolving(identity.try_clone().unwrap(), store.reserve_scratch(1),)
                .unwrap()
        );
        let cache_index = cache
            .store_failed(
                &identity,
                ScopeAbortCause::AssetOpen(AssetOpenError::new(
                    AssetOpenStage::Open,
                    AssetOpenErrorKind::NotFound,
                )),
                None,
                PendingNegativeObservation::None,
            )
            .expect("the resolving cache entry must accept its first failure");

        for _ in 0..2 {
            let cached = cache
                .failed(cache_index)
                .expect("replaying a failure must not consume the cache entry");
            assert_eq!(
                cached.unavailable(),
                CachedDependencyUnavailable::EnvironmentDependent {
                    code: CachedDependencyIoCode::Missing,
                    observation: CachedNegativeObservation::None,
                }
            );
            assert!(matches!(
                cached.replay_scope_cause(),
                ScopeAbortCause::AssetOpen(error)
                    if error.stage() == AssetOpenStage::Open
                        && error.kind() == AssetOpenErrorKind::NotFound
            ));
        }
        assert!(matches!(
            cache.get(&identity),
            Some((index, DependencyCacheEntry::Failed(_))) if index == cache_index
        ));

        let namespace = ImmutableMemoryAssetIo::try_from_entries(vec![(
            AssetPath::new("primary.bin").unwrap(),
            Arc::from(&b"primary"[..]),
        )])
        .unwrap();
        let replacement_identity = DependencyIdentity::new(
            namespace.authority_domain(),
            AssetPath::new("missing.bin").unwrap(),
        );
        let mut replacement_cache = DependencyCache::new();
        replacement_cache.entries = Vec::with_capacity(1);
        replacement_cache.index.slots = vec![DEPENDENCY_INDEX_EMPTY; 8];
        assert!(
            replacement_cache
                .mark_resolving(
                    replacement_identity.try_clone().unwrap(),
                    store.reserve_scratch(1),
                )
                .unwrap()
        );
        let replacement_index = replacement_cache
            .store_failed(
                &replacement_identity,
                missing_dependency_cause(),
                Some(same_authority_resolver_v1().identity_schema()),
                PendingNegativeObservation::None,
            )
            .unwrap();
        let replacement_binding = same_authority_resolver_v1()
            .plan_negative_proof(
                &namespace,
                replacement_identity.path(),
                RegisteredUnavailableCause::Missing,
            )
            .unwrap()
            .verify(
                replacement_identity.path(),
                RegisteredUnavailableCause::Missing,
            )
            .unwrap();
        assert!(matches!(
            replacement_cache.store_failed(
                &replacement_identity,
                missing_dependency_cause(),
                Some(same_authority_resolver_v1().identity_schema()),
                PendingNegativeObservation::Proof(replacement_binding),
            ),
            Err(ScopeAbortCause::DescriptorContract {
                code: "dependency-cache-transition-missing"
            })
        ));
        assert_eq!(
            replacement_cache
                .failed(replacement_index)
                .expect("the original none observation remains cached")
                .unavailable(),
            CachedDependencyUnavailable::EnvironmentDependent {
                code: CachedDependencyIoCode::Missing,
                observation: CachedNegativeObservation::None,
            }
        );
    }

    #[test]
    fn dependency_failure_cache_rejects_cross_subject_and_authority_negative_proof() {
        let namespace = ImmutableMemoryAssetIo::try_from_entries(vec![(
            AssetPath::new("primary.bin").unwrap(),
            Arc::from(&b"primary"[..]),
        )])
        .unwrap();
        let resolver = same_authority_resolver_v1();
        let authority = namespace.authority_domain();
        let proved_identity = DependencyIdentity::new(
            authority.clone(),
            AssetPath::new("proved-missing.bin").unwrap(),
        );
        let foreign_identity =
            DependencyIdentity::new(authority, AssetPath::new("other-missing.bin").unwrap());
        let foreign_namespace = ImmutableMemoryAssetIo::try_from_entries(vec![(
            AssetPath::new("authority-mismatch.bin").unwrap(),
            Arc::from(&b"present"[..]),
        )])
        .unwrap();
        let foreign_authority_identity = DependencyIdentity::new(
            foreign_namespace.authority_domain(),
            AssetPath::new("authority-mismatch.bin").unwrap(),
        );
        let store = StoreHarness::new();
        let mut cache = DependencyCache::new();
        cache.entries = Vec::with_capacity(3);
        cache.index.slots = vec![DEPENDENCY_INDEX_EMPTY; 8];
        for identity in [
            &proved_identity,
            &foreign_identity,
            &foreign_authority_identity,
        ] {
            assert!(
                cache
                    .mark_resolving(identity.try_clone().unwrap(), store.reserve_scratch(1))
                    .unwrap()
            );
        }
        let foreign_binding = resolver
            .plan_negative_proof(
                &namespace,
                proved_identity.path(),
                RegisteredUnavailableCause::Missing,
            )
            .unwrap()
            .verify(proved_identity.path(), RegisteredUnavailableCause::Missing)
            .unwrap();
        assert!(matches!(
            cache.store_failed(
                &foreign_identity,
                missing_dependency_cause(),
                Some(resolver.identity_schema()),
                PendingNegativeObservation::Proof(foreign_binding),
            ),
            Err(ScopeAbortCause::DescriptorContract {
                code: "registered-negative-proof-cache-subject-mismatch"
            })
        ));
        assert!(matches!(
            cache.get(&foreign_identity),
            Some((_, DependencyCacheEntry::Resolving))
        ));

        let foreign_authority_binding = resolver
            .plan_negative_proof(
                &namespace,
                foreign_authority_identity.path(),
                RegisteredUnavailableCause::Missing,
            )
            .unwrap()
            .verify(
                foreign_authority_identity.path(),
                RegisteredUnavailableCause::Missing,
            )
            .unwrap();
        assert!(matches!(
            cache.store_failed(
                &foreign_authority_identity,
                missing_dependency_cause(),
                Some(resolver.identity_schema()),
                PendingNegativeObservation::Proof(foreign_authority_binding),
            ),
            Err(ScopeAbortCause::DescriptorContract {
                code: "registered-negative-proof-cache-subject-mismatch"
            })
        ));
        assert!(matches!(
            cache.get(&foreign_authority_identity),
            Some((_, DependencyCacheEntry::Resolving))
        ));

        let wrong_cause_binding = resolver
            .plan_negative_proof(
                &namespace,
                proved_identity.path(),
                RegisteredUnavailableCause::Missing,
            )
            .unwrap()
            .verify(proved_identity.path(), RegisteredUnavailableCause::Missing)
            .unwrap();
        assert!(matches!(
            cache.store_failed(
                &proved_identity,
                ScopeAbortCause::AssetOpen(AssetOpenError::new(
                    AssetOpenStage::Open,
                    AssetOpenErrorKind::Backend,
                )),
                Some(resolver.identity_schema()),
                PendingNegativeObservation::Proof(wrong_cause_binding),
            ),
            Err(ScopeAbortCause::DescriptorContract {
                code: "registered-negative-proof-cache-subject-mismatch"
            })
        ));

        let correct_binding = resolver
            .plan_negative_proof(
                &namespace,
                proved_identity.path(),
                RegisteredUnavailableCause::Missing,
            )
            .unwrap()
            .verify(proved_identity.path(), RegisteredUnavailableCause::Missing)
            .unwrap();
        let proof_index = cache
            .store_failed(
                &proved_identity,
                missing_dependency_cause(),
                Some(resolver.identity_schema()),
                PendingNegativeObservation::Proof(correct_binding),
            )
            .expect("the correct proof must publish");
        let proof_observation = cache
            .failed(proof_index)
            .expect("the proof failure must be retained")
            .unavailable();

        assert!(matches!(
            cache.store_failed(
                &proved_identity,
                missing_dependency_cause(),
                Some(resolver.identity_schema()),
                PendingNegativeObservation::None,
            ),
            Err(ScopeAbortCause::DescriptorContract {
                code: "dependency-cache-transition-missing"
            })
        ));
        assert_eq!(
            cache
                .failed(proof_index)
                .expect("the proof failure must remain after a rejected replacement")
                .unavailable(),
            proof_observation
        );

        let replacement_binding = resolver
            .plan_negative_proof(
                &namespace,
                proved_identity.path(),
                RegisteredUnavailableCause::Missing,
            )
            .unwrap()
            .verify(proved_identity.path(), RegisteredUnavailableCause::Missing)
            .unwrap();
        assert!(matches!(
            cache.store_failed(
                &proved_identity,
                missing_dependency_cause(),
                Some(resolver.identity_schema()),
                PendingNegativeObservation::Proof(replacement_binding),
            ),
            Err(ScopeAbortCause::DescriptorContract {
                code: "dependency-cache-transition-missing"
            })
        ));
        assert_eq!(
            cache
                .failed(proof_index)
                .expect("the proof failure must remain after a second rejected replacement")
                .unavailable(),
            proof_observation
        );
    }

    #[test]
    fn trusted_reader_contract_failure_cannot_enter_dependency_cache() {
        let cause = ScopeAbortCause::AssetRead(AssetReadError::contract_violation(
            AssetReadContractViolation::CountExceedsBuffer {
                returned: 2,
                capacity: 1,
            },
        ));

        assert!(matches!(
            CachedDependencyFailure::try_new(
                &indexed_identity(&AuthorityDomain::new(), "contract", 0),
                cause,
                None,
                PendingNegativeObservation::None,
            ),
            Err(ScopeAbortCause::AssetRead(error))
                if matches!(
                    error.kind(),
                    AssetReadErrorKind::ContractViolation(
                        AssetReadContractViolation::CountExceedsBuffer {
                            returned: 2,
                            capacity: 1,
                        }
                    )
                )
        ));
    }

    #[test]
    fn dependency_origin_remains_session_bound_after_snapshot_drop() {
        let store = StoreHarness::new();
        let issuer = DependencyOriginIssuer::new();
        let io = MemoryAssetIo::new();
        let authority = io.authority_domain();
        let identity = indexed_identity(&authority, "origin", 0);
        let mut cache = DependencyCache::new();
        cache.entries = Vec::with_capacity(1);
        cache.index.slots = vec![DEPENDENCY_INDEX_EMPTY; 8];
        assert!(
            cache
                .mark_resolving(identity.try_clone().unwrap(), store.reserve_scratch(1),)
                .unwrap()
        );
        let bytes = Arc::new(SharedDependencySnapshot::new(
            AccountedVec::new(),
            store.reserve_scratch(1),
        ));
        let origin_node = cache
            .store_available(&identity, Arc::clone(&bytes), None)
            .expect("the resolving dependency must become available");
        let snapshot = DependencySnapshot::new(
            &issuer,
            &io,
            origin_node,
            DependencySnapshotIdentity::new(identity, None, store.reserve_scratch(1)),
            bytes,
            DependencyCacheDisposition::Fresh,
        );

        let origin = snapshot.origin();
        drop(snapshot);

        assert!(size_of::<DependencyOriginIssuer>() > 0);
        assert!(origin.is_issued_by(&issuer));
        assert!(std::ptr::eq(origin.io(), &io));
        assert!(matches!(
            origin.node(),
            DependencyOriginNode::Dependency(index) if index == origin_node
        ));
        assert!(cache.available_identity(origin_node).is_some());
    }
}
