//! Session-owned external-resource graph and successful input manifest.
//!
//! This module owns both the authority-qualified physical dependency cache and the logical graph.
//! `ParseScope` receives only opaque origin handles and dependency results; format crates cannot
//! inspect graph IDs, physical cache keys, or manifest construction state.

mod manifest;

pub(crate) use manifest::registered_negative_proof_canonical_len;

pub use manifest::{
    InputReplayStatus, ManifestDigest, ManifestNamespaceSnapshotProof, ManifestNodeClass,
    ManifestObservedOnlyReason, ManifestObservedOnlyReasons, ManifestPerReferenceProof,
    ManifestReference, ManifestRelation, ManifestRelationOutcome, ManifestResourceIdentity,
    ManifestResourceIdentityStability, ManifestSemanticIdentity, ManifestSessionDependentReason,
    ManifestSessionOpaqueReason, ManifestUnavailableClass, ManifestUnavailableCode,
    ManifestUnavailableReplay, ResourceManifestCounts, SuccessfulInputManifest,
};

use crate::dependency::{
    CachedDependencyFailure, CachedDependencyIoCode, CachedDependencyUnavailable,
    CachedNegativeObservation, DependencyCache, DependencyCacheCapacityError, DependencyCacheEntry,
    DependencyCacheStorage, DependencyIdentity, DependencyRequirement, DependencyRoleId,
    PendingNegativeObservation, SharedDependencySnapshot,
};
use crate::error::ScopeAbortCause;
use crate::ledger::{ImportDimension, ImportLedgerPhase};
use crate::limits::ResourceLimits;
use crate::metered_storage::AccountedVec;
use crate::store::{ImportStoreHost, ImportStoreReservation};
use baozi_io::__semantic_identity::StableResourceIdentitySchema;
use baozi_io::{AssetPath, AuthorityDomain};
use baozi_operation::CapacityPlanError;
use std::sync::Arc;

#[cfg(test)]
use manifest::ManifestRegisteredUnavailableCause;
use manifest::{
    ManifestEncodingError, encode_node_class_into_accounted, encode_relation_key_into_accounted,
    node_class_len, registered_negative_proof_replay, relation_key_len,
};

#[derive(Debug)]
pub(crate) enum ResourceGraphError {
    Scope(ScopeAbortCause),
    Limit { resource: &'static str, limit: u64 },
    Allocation { resource: &'static str },
    Cycle,
    DanglingNode,
    IncompleteRelation,
    AlreadySealed,
    DigestCollision,
    MissingPrimary,
    Overflow,
    Encoding,
}

impl ResourceGraphError {
    pub(crate) fn into_scope_cause(self) -> ScopeAbortCause {
        match self {
            Self::Scope(cause) => cause,
            Self::Limit { resource, limit } => ScopeAbortCause::ResourceLimit { resource, limit },
            Self::Allocation { resource } => ScopeAbortCause::Allocation { resource },
            Self::Cycle => ScopeAbortCause::DescriptorContract {
                code: "resource-graph-cycle",
            },
            Self::DanglingNode => ScopeAbortCause::DescriptorContract {
                code: "resource-graph-dangling-node",
            },
            Self::IncompleteRelation => ScopeAbortCause::DescriptorContract {
                code: "resource-graph-incomplete-relation",
            },
            Self::AlreadySealed => ScopeAbortCause::DescriptorContract {
                code: "resource-graph-already-sealed",
            },
            Self::DigestCollision => ScopeAbortCause::DescriptorContract {
                code: "resource-manifest-digest-collision",
            },
            Self::MissingPrimary => ScopeAbortCause::DescriptorContract {
                code: "resource-graph-primary-missing",
            },
            Self::Overflow => ScopeAbortCause::ResourceLimit {
                resource: "resource-graph-counter",
                limit: u64::MAX,
            },
            Self::Encoding => ScopeAbortCause::DescriptorContract {
                code: "resource-manifest-encoding-failed",
            },
        }
    }
}

impl From<ManifestEncodingError> for ResourceGraphError {
    fn from(_cause: ManifestEncodingError) -> Self {
        Self::Encoding
    }
}

#[derive(Debug)]
struct GraphNode {
    exact_identity: DependencyIdentity,
    public_identity: ManifestResourceIdentity,
    byte_length: u64,
    content_digest: ManifestDigest,
    first_outgoing: Option<usize>,
    last_outgoing: Option<usize>,
    outgoing_count: usize,
    depth: u64,
}

#[derive(Debug)]
struct GraphRelation {
    role: DependencyRoleId,
    requirement: DependencyRequirement,
    reference: ManifestReference,
    outcome: GraphRelationOutcome,
    previous_outgoing: Option<usize>,
    next_outgoing: Option<usize>,
}

#[derive(Clone, Copy)]
enum OutgoingDirection {
    Forward,
    Reverse,
}

struct OutgoingRelations<'graph> {
    relations: &'graph [GraphRelation],
    next: Option<usize>,
    remaining: usize,
    terminal_checked: bool,
    direction: OutgoingDirection,
}

impl<'graph> Iterator for OutgoingRelations<'graph> {
    type Item = Result<&'graph GraphRelation, ResourceGraphError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            if self.terminal_checked {
                return None;
            }
            self.terminal_checked = true;
            return self
                .next
                .take()
                .map(|_| Err(ResourceGraphError::DanglingNode));
        }
        let Some(index) = self.next else {
            self.remaining = 0;
            self.terminal_checked = true;
            return Some(Err(ResourceGraphError::DanglingNode));
        };
        let Some(relation) = self.relations.get(index) else {
            self.remaining = 0;
            self.terminal_checked = true;
            return Some(Err(ResourceGraphError::DanglingNode));
        };
        self.next = match self.direction {
            OutgoingDirection::Forward => relation.next_outgoing,
            OutgoingDirection::Reverse => relation.previous_outgoing,
        };
        self.remaining -= 1;
        Some(Ok(relation))
    }
}

#[derive(Debug)]
// Graph relation storage is capacity-planned with this complete inline type;
// the bounded proof evidence must not escape into an unmetered allocation.
#[allow(clippy::large_enum_variant)]
enum GraphRelationOutcome {
    Available {
        target: usize,
    },
    Unavailable {
        class: ManifestUnavailableClass,
        code: ManifestUnavailableCode,
        identity_evidence: Option<ManifestResourceIdentity>,
        replay: ManifestUnavailableReplay,
    },
}

struct DigestedManifestRelation {
    digest: ManifestDigest,
    canonical: AccountedVec<u8>,
    relation: ManifestRelation,
}

struct DigestedManifestNode {
    digest: ManifestDigest,
    canonical: AccountedVec<u8>,
    source_index: usize,
    relations: AccountedVec<ManifestRelation>,
}

#[derive(Debug)]
enum TargetAdmission {
    Existing {
        index: usize,
        depth_updates: Vec<DepthUpdate>,
    },
    New {
        depth: u64,
    },
}

#[derive(Debug, Clone, Copy)]
struct DepthUpdate {
    index: usize,
    depth: u64,
}

pub(crate) enum ResourceAcquisitionState {
    Available {
        cache_index: usize,
        bytes: Arc<SharedDependencySnapshot>,
        stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
    },
    Failed {
        cache_index: usize,
        stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
    },
    Resolving,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResourceUnavailableDisposition {
    class: ManifestUnavailableClass,
    code: ManifestUnavailableCode,
    replay: ManifestUnavailableReplay,
    stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
}

/// One admitted logical request that must reach exactly one terminal relation.
pub(crate) struct PendingResourceRelation {
    source: usize,
    role: DependencyRoleId,
    requirement: DependencyRequirement,
    raw_reference_digest: ManifestDigest,
    target: Option<TargetAdmission>,
}

pub(crate) struct AvailableResourceIdentity<'a> {
    exact: &'a DependencyIdentity,
    normalized: &'a AssetPath,
    stable_schema: Option<&'static StableResourceIdentitySchema>,
}

impl<'a> AvailableResourceIdentity<'a> {
    pub(crate) const fn new(
        exact: &'a DependencyIdentity,
        normalized: &'a AssetPath,
        stable_schema: Option<&'static StableResourceIdentitySchema>,
    ) -> Self {
        Self {
            exact,
            normalized,
            stable_schema,
        }
    }
}

/// One session-owned graph. `DependencyCache` remains the sole physical acquisition cache.
pub(crate) struct ResourceGraph {
    cache: DependencyCache,
    limits: ResourceLimits,
    nodes: AccountedVec<GraphNode>,
    relations: AccountedVec<GraphRelation>,
    cache_to_node: AccountedVec<Option<usize>>,
    primary: Option<usize>,
    relation_count: u64,
    pending_relations: u64,
    sealed_manifest: Option<SuccessfulInputManifest>,
}

impl ResourceGraph {
    pub(crate) fn new(limits: ResourceLimits) -> Self {
        Self {
            cache: DependencyCache::new(),
            limits,
            nodes: AccountedVec::new(),
            relations: AccountedVec::new(),
            cache_to_node: AccountedVec::new(),
            primary: None,
            relation_count: 0,
            pending_relations: 0,
            sealed_manifest: None,
        }
    }

    pub(crate) fn set_limits(&mut self, limits: ResourceLimits) -> Result<(), ResourceGraphError> {
        if self.sealed_manifest.is_some() {
            return Err(ResourceGraphError::AlreadySealed);
        }
        self.limits = limits;
        self.validate_current_limits()
    }

    pub(crate) fn origin_identity(&self, cache_index: usize) -> Option<&DependencyIdentity> {
        self.cache.available_identity(cache_index)
    }

    pub(crate) fn acquisition_state(
        &self,
        identity: &DependencyIdentity,
    ) -> Option<ResourceAcquisitionState> {
        self.cache
            .get(identity)
            .map(|(cache_index, entry)| match entry {
                DependencyCacheEntry::Available(snapshot) => ResourceAcquisitionState::Available {
                    cache_index,
                    bytes: Arc::clone(&snapshot.bytes),
                    stable_identity_schema: snapshot.stable_identity_schema,
                },
                DependencyCacheEntry::Failed(failure) => ResourceAcquisitionState::Failed {
                    cache_index,
                    stable_identity_schema: failure.stable_identity_schema(),
                },
                DependencyCacheEntry::Resolving => ResourceAcquisitionState::Resolving,
            })
    }

    pub(crate) fn reserve_acquisition_capacity(
        &mut self,
        host: ImportStoreHost<'_>,
    ) -> Result<(), ResourceGraphError> {
        let plan = self.cache.capacity_plan().map_err(dependency_cache_error)?;
        if !plan.requires_apply() {
            return Ok(());
        }
        let entry_envelope = plan
            .temporary_envelope_bytes(DependencyCacheStorage::Entries)
            .map_err(dependency_cache_error)?;
        let index_envelope = plan
            .temporary_envelope_bytes(DependencyCacheStorage::Index)
            .map_err(dependency_cache_error)?;
        let mut entry_reservation = self
            .cache
            .take_capacity_reservation(DependencyCacheStorage::Entries);
        let mut index_reservation = self
            .cache
            .take_capacity_reservation(DependencyCacheStorage::Index);
        let entry_prior_bytes = entry_reservation
            .as_ref()
            .map_or(0, ImportStoreReservation::bytes);
        let index_prior_bytes = index_reservation
            .as_ref()
            .map_or(0, ImportStoreReservation::bytes);

        if let Err(cause) = grow_acquisition_reservation(
            host,
            DependencyCacheStorage::Entries,
            entry_envelope,
            &mut entry_reservation,
        ) {
            restore_acquisition_reservations(
                &mut self.cache,
                host,
                entry_reservation,
                entry_prior_bytes,
                index_reservation,
                index_prior_bytes,
            );
            return Err(ResourceGraphError::Scope(cause));
        }
        if let Err(cause) = grow_acquisition_reservation(
            host,
            DependencyCacheStorage::Index,
            index_envelope,
            &mut index_reservation,
        ) {
            restore_acquisition_reservations(
                &mut self.cache,
                host,
                entry_reservation,
                entry_prior_bytes,
                index_reservation,
                index_prior_bytes,
            );
            return Err(ResourceGraphError::Scope(cause));
        }

        let actual = match self.cache.apply_capacity_plan(plan) {
            Ok(actual) => actual,
            Err(cause) => {
                restore_acquisition_reservations(
                    &mut self.cache,
                    host,
                    entry_reservation,
                    entry_prior_bytes,
                    index_reservation,
                    index_prior_bytes,
                );
                return Err(dependency_cache_error(cause));
            }
        };

        let mut resize_failure = None;
        for (storage, reservation) in [
            (DependencyCacheStorage::Entries, &mut entry_reservation),
            (DependencyCacheStorage::Index, &mut index_reservation),
        ] {
            let Some(retained_bytes) = actual.retained_bytes(storage) else {
                continue;
            };
            let Some(reservation) = reservation.as_mut() else {
                resize_failure = Some(ScopeAbortCause::DescriptorContract {
                    code: "dependency-cache-capacity-reservation-missing",
                });
                continue;
            };
            if let Err(cause) = host.resize(reservation, retained_bytes) {
                resize_failure.get_or_insert_with(|| cause.into_scope_abort_cause());
            }
        }
        self.cache
            .retain_capacity_reservation(DependencyCacheStorage::Entries, entry_reservation);
        self.cache
            .retain_capacity_reservation(DependencyCacheStorage::Index, index_reservation);
        if let Some(cause) = resize_failure {
            return Err(ResourceGraphError::Scope(cause));
        }
        Ok(())
    }

    pub(crate) fn begin_acquisition(
        &mut self,
        identity: DependencyIdentity,
        identity_path_reservation: ImportStoreReservation,
    ) -> Result<bool, ResourceGraphError> {
        self.cache
            .mark_resolving(identity, identity_path_reservation)
            .map_err(|()| ResourceGraphError::Allocation {
                resource: "dependency-cache-entry",
            })
    }

    pub(crate) fn complete_acquisition(
        &mut self,
        identity: &DependencyIdentity,
        bytes: Arc<SharedDependencySnapshot>,
        stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
    ) -> Result<usize, ResourceGraphError> {
        self.cache
            .store_available(identity, bytes, stable_identity_schema)
            .ok_or(ResourceGraphError::DanglingNode)
    }

    pub(crate) fn abandon_acquisition(
        &mut self,
        identity: &DependencyIdentity,
    ) -> Result<(), ResourceGraphError> {
        self.cache
            .remove_resolving(identity)
            .then_some(())
            .ok_or(ResourceGraphError::DanglingNode)
    }

    pub(crate) fn complete_failed_acquisition(
        &mut self,
        identity: &DependencyIdentity,
        cause: ScopeAbortCause,
        stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
        observation: PendingNegativeObservation<'_>,
    ) -> Result<usize, ScopeAbortCause> {
        self.cache
            .store_failed(identity, cause, stable_identity_schema, observation)
    }

    pub(crate) fn cached_failure(
        &self,
        cache_index: usize,
    ) -> Result<&CachedDependencyFailure, ResourceGraphError> {
        self.cache
            .failed(cache_index)
            .ok_or(ResourceGraphError::DanglingNode)
    }

    pub(crate) fn bind_primary(
        &mut self,
        host: ImportStoreHost<'_>,
        authority: &AuthorityDomain,
        path: &AssetPath,
        bytes: &[u8],
    ) -> Result<(), ResourceGraphError> {
        if self.primary.is_some() || self.sealed_manifest.is_some() {
            return Err(ResourceGraphError::AlreadySealed);
        }
        self.require_node_capacity(1)?;
        charge_attempted(host, ImportDimension::ResourceGraphNodes, 1)?;
        self.nodes
            .try_reserve(1, "resource-graph-nodes", |bytes| {
                reserve_graph_storage(host, bytes)
            })
            .map_err(ResourceGraphError::Scope)?;
        let exact_identity = DependencyIdentity::new(
            authority.clone(),
            path.try_clone()
                .map_err(|_| ResourceGraphError::Allocation {
                    resource: "resource-graph-primary-identity",
                })?,
        );
        let public_identity = manifest_identity(
            path,
            ManifestSessionOpaqueReason::UnregisteredCallerIdentity,
            "resource-graph-primary-public-identity",
        )?;
        let node = GraphNode {
            exact_identity,
            public_identity,
            byte_length: u64::try_from(bytes.len()).map_err(|_| ResourceGraphError::Overflow)?,
            content_digest: ManifestDigest::of(bytes),
            first_outgoing: None,
            last_outgoing: None,
            outgoing_count: 0,
            depth: 0,
        };
        self.nodes
            .try_push_reserved(node)
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-graph-node-slot",
            })?;
        self.primary = Some(0);
        charge_consumed(host, ImportDimension::ResourceGraphNodes, 1)?;
        Ok(())
    }

    pub(crate) fn source_for_primary(&self) -> Result<usize, ResourceGraphError> {
        self.primary.ok_or(ResourceGraphError::MissingPrimary)
    }

    pub(crate) fn source_for_cache(&self, cache_index: usize) -> Result<usize, ResourceGraphError> {
        self.cache_to_node
            .get(cache_index)
            .and_then(|entry| *entry)
            .filter(|index| self.nodes.get(*index).is_some())
            .ok_or(ResourceGraphError::DanglingNode)
    }

    pub(crate) fn begin_relation(
        &mut self,
        host: ImportStoreHost<'_>,
        source: usize,
        role: &DependencyRoleId,
        requirement: DependencyRequirement,
        raw_reference: &str,
    ) -> Result<PendingResourceRelation, ResourceGraphError> {
        if self.sealed_manifest.is_some() {
            return Err(ResourceGraphError::AlreadySealed);
        }
        self.nodes
            .get(source)
            .ok_or(ResourceGraphError::DanglingNode)?;
        let next = self
            .relation_count
            .checked_add(self.pending_relations)
            .and_then(|value| value.checked_add(1))
            .ok_or(ResourceGraphError::Overflow)?;
        if next > u64::try_from(self.limits.max_resource_graph_relations).unwrap_or(u64::MAX) {
            return Err(ResourceGraphError::Limit {
                resource: "max_resource_graph_relations",
                limit: self.limits.max_resource_graph_relations as u64,
            });
        }
        charge_attempted(host, ImportDimension::ResourceGraphRelations, 1)?;
        charge_graph_work(host, 1)?;
        self.relations
            .try_reserve(1, "resource-graph-relations", |bytes| {
                reserve_graph_storage(host, bytes)
            })
            .map_err(ResourceGraphError::Scope)?;
        self.pending_relations = self
            .pending_relations
            .checked_add(1)
            .ok_or(ResourceGraphError::Overflow)?;
        Ok(PendingResourceRelation {
            source,
            role: role.clone(),
            requirement,
            raw_reference_digest: ManifestDigest::of(raw_reference.as_bytes()),
            target: None,
        })
    }

    pub(crate) fn prepare_target(
        &mut self,
        host: ImportStoreHost<'_>,
        pending: &mut PendingResourceRelation,
        identity: &DependencyIdentity,
    ) -> Result<(), ResourceGraphError> {
        if pending.target.is_some() {
            return Err(ResourceGraphError::IncompleteRelation);
        }
        let source_depth = self
            .nodes
            .get(pending.source)
            .map(|node| node.depth)
            .ok_or(ResourceGraphError::DanglingNode)?;
        let depth = source_depth
            .checked_add(1)
            .ok_or(ResourceGraphError::Overflow)?;
        if depth > u64::try_from(self.limits.max_resource_graph_depth).unwrap_or(u64::MAX) {
            return Err(ResourceGraphError::Limit {
                resource: "max_resource_graph_depth",
                limit: self.limits.max_resource_graph_depth as u64,
            });
        }
        if let Some(target) = self.find_node(identity) {
            let depth_updates = self.plan_existing_target(host, target, pending.source, depth)?;
            pending.target = Some(TargetAdmission::Existing {
                index: target,
                depth_updates,
            });
            return Ok(());
        }
        self.require_node_capacity(1)?;
        charge_attempted(host, ImportDimension::ResourceGraphNodes, 1)?;
        self.nodes
            .try_reserve(1, "resource-graph-nodes", |bytes| {
                reserve_graph_storage(host, bytes)
            })
            .map_err(ResourceGraphError::Scope)?;
        pending.target = Some(TargetAdmission::New { depth });
        Ok(())
    }

    pub(crate) fn complete_available(
        &mut self,
        host: ImportStoreHost<'_>,
        mut pending: PendingResourceRelation,
        cache_index: usize,
        bytes: &[u8],
        identity: AvailableResourceIdentity<'_>,
    ) -> Result<(), ResourceGraphError> {
        if identity.stable_schema.is_some() && identity.exact.path() != identity.normalized {
            return Err(ResourceGraphError::DigestCollision);
        }
        let (target, depth_updates) = match pending
            .target
            .take()
            .ok_or(ResourceGraphError::IncompleteRelation)?
        {
            TargetAdmission::Existing {
                index,
                depth_updates,
            } => {
                let node = self
                    .nodes
                    .get(index)
                    .ok_or(ResourceGraphError::DanglingNode)?;
                if &node.exact_identity != identity.exact
                    || node.content_digest != ManifestDigest::of(bytes)
                {
                    return Err(ResourceGraphError::DigestCollision);
                }
                (index, depth_updates)
            }
            TargetAdmission::New { depth } => {
                let exact_identity =
                    identity
                        .exact
                        .try_clone()
                        .map_err(|_| ResourceGraphError::Allocation {
                            resource: "resource-graph-node-identity",
                        })?;
                let public_identity = manifest_identity_for_resolution(
                    identity.exact.path(),
                    identity.stable_schema,
                    "resource-graph-node-public-identity",
                )?;
                let index = self.nodes.len();
                self.nodes
                    .try_push_reserved(GraphNode {
                        exact_identity,
                        public_identity,
                        byte_length: u64::try_from(bytes.len())
                            .map_err(|_| ResourceGraphError::Overflow)?,
                        content_digest: ManifestDigest::of(bytes),
                        first_outgoing: None,
                        last_outgoing: None,
                        outgoing_count: 0,
                        depth,
                    })
                    .map_err(|_| ResourceGraphError::Allocation {
                        resource: "resource-graph-node-slot",
                    })?;
                charge_consumed(host, ImportDimension::ResourceGraphNodes, 1)?;
                (index, Vec::new())
            }
        };
        self.bind_cache_node(host, cache_index, target)?;
        let reference = ManifestReference::Normalized(manifest_identity_for_resolution(
            identity.normalized,
            identity.stable_schema,
            "resource-graph-normalized-reference",
        )?);
        self.push_relation(
            host,
            pending,
            reference,
            GraphRelationOutcome::Available { target },
        )?;
        self.apply_depth_updates(&depth_updates)
    }

    pub(crate) fn complete_unavailable(
        &mut self,
        host: ImportStoreHost<'_>,
        pending: PendingResourceRelation,
        normalized: Option<&AssetPath>,
        identity: Option<&DependencyIdentity>,
        disposition: ResourceUnavailableDisposition,
    ) -> Result<(), ResourceGraphError> {
        let ResourceUnavailableDisposition {
            class,
            code,
            replay,
            stable_identity_schema,
        } = disposition;
        let reference = match normalized {
            Some(path) => ManifestReference::Normalized(manifest_identity_for_resolution(
                path,
                stable_identity_schema,
                "resource-graph-normalized-reference",
            )?),
            None => ManifestReference::Rejected {
                class,
                code,
                reference_digest: pending.raw_reference_digest,
            },
        };
        let identity_evidence = identity
            .map(|identity| {
                manifest_identity_for_resolution(
                    identity.path(),
                    stable_identity_schema,
                    "resource-graph-unavailable-identity",
                )
            })
            .transpose()?;
        self.push_relation(
            host,
            pending,
            reference,
            GraphRelationOutcome::Unavailable {
                class,
                code,
                identity_evidence,
                replay,
            },
        )
    }

    pub(crate) fn seal(&mut self, host: ImportStoreHost<'_>) -> Result<(), ResourceGraphError> {
        if self.sealed_manifest.is_some() {
            return Err(ResourceGraphError::AlreadySealed);
        }
        if self.pending_relations != 0 {
            return Err(ResourceGraphError::IncompleteRelation);
        }
        self.validate_relation_storage()?;
        let primary = self.primary.ok_or(ResourceGraphError::MissingPrimary)?;
        let postorder = self.postorder(host, primary)?;
        if postorder.len() != self.nodes.len() {
            return Err(ResourceGraphError::DanglingNode);
        }
        let mut digests =
            manifest_scratch_vec(host, self.nodes.len(), "resource-manifest-node-digests")?;
        for _ in 0..self.nodes.len() {
            digests.push_reserved_infallible(None);
        }
        let mut physical =
            manifest_scratch_vec(host, postorder.len(), "resource-manifest-node-classes")?;

        for &index in postorder.iter() {
            charge_manifest_work(host, 1)?;
            let node = self
                .nodes
                .get(index)
                .ok_or(ResourceGraphError::DanglingNode)?;
            let mut digested_relations = manifest_scratch_vec(
                host,
                node.outgoing_count,
                "resource-manifest-relation-order",
            )?;
            for relation in self.outgoing_relations(index)? {
                let relation = relation?;
                charge_manifest_work(host, 1)?;
                let relation = project_relation(relation, &digests)?;
                let canonical_len = relation_key_len(
                    relation.role(),
                    relation.requirement(),
                    relation.reference(),
                    relation.outcome(),
                )?;
                let mut canonical =
                    manifest_scratch_vec(host, canonical_len, "resource-manifest-relation-key")?;
                encode_relation_key_into_accounted(
                    relation.role(),
                    relation.requirement(),
                    relation.reference(),
                    relation.outcome(),
                    canonical_len,
                    &mut canonical,
                )?;
                digested_relations.push_reserved_infallible(DigestedManifestRelation {
                    digest: hash_manifest_bytes(host, &canonical)?,
                    canonical,
                    relation,
                });
            }
            let relations = fold_manifest_relations(host, digested_relations)?;
            let canonical_len = node_class_len(
                &node.public_identity,
                node.byte_length,
                node.content_digest,
                &relations,
            )?;
            let mut canonical =
                manifest_scratch_vec(host, canonical_len, "resource-manifest-node-class")?;
            encode_node_class_into_accounted(
                &node.public_identity,
                node.byte_length,
                node.content_digest,
                &relations,
                canonical_len,
                &mut canonical,
            )?;
            let digest = hash_manifest_bytes(host, &canonical)?;
            digests[index] = Some(digest);
            physical.push_reserved_infallible(DigestedManifestNode {
                digest,
                canonical,
                source_index: index,
                relations,
            });
        }
        drop(postorder);

        let classes = fold_manifest_nodes(host, physical, &self.nodes)?;
        let primary_digest = digests
            .get(primary)
            .and_then(|digest| *digest)
            .ok_or(ResourceGraphError::MissingPrimary)?;
        let (counts, replay_status) =
            manifest_counts_and_replay_status(&classes, self.maximum_depth())?;
        let mut provisional = SuccessfulInputManifest::new(
            ManifestDigest::zero(),
            primary_digest,
            classes,
            counts,
            replay_status,
        );
        let canonical_len = provisional.canonical_len()?;
        let byte_count = u64::try_from(canonical_len).map_err(|_| ResourceGraphError::Overflow)?;
        if byte_count > self.limits.max_input_manifest_bytes {
            return Err(ResourceGraphError::Limit {
                resource: "max_input_manifest_bytes",
                limit: self.limits.max_input_manifest_bytes,
            });
        }
        charge_attempted(host, ImportDimension::InputManifestBytes, byte_count)?;
        let mut canonical = AccountedVec::new();
        canonical
            .try_reserve_capacity(
                canonical_len,
                "resource-manifest-canonical-bytes",
                |bytes| reserve_manifest_storage(host, bytes),
            )
            .map_err(ResourceGraphError::Scope)?;
        provisional.write_canonical_bytes(canonical_len, &mut canonical)?;
        let digest = hash_manifest_bytes(host, &canonical)?;
        charge_consumed(host, ImportDimension::InputManifestBytes, byte_count)?;
        drop(canonical);
        provisional.release_live_reservations();
        self.sealed_manifest = Some(provisional.with_digest(digest));
        Ok(())
    }

    pub(crate) fn take_manifest(&mut self) -> Result<SuccessfulInputManifest, ResourceGraphError> {
        self.sealed_manifest
            .take()
            .ok_or(ResourceGraphError::IncompleteRelation)
    }

    fn push_relation(
        &mut self,
        host: ImportStoreHost<'_>,
        pending: PendingResourceRelation,
        reference: ManifestReference,
        outcome: GraphRelationOutcome,
    ) -> Result<(), ResourceGraphError> {
        let (first_outgoing, last_outgoing, outgoing_count) = self
            .nodes
            .get(pending.source)
            .map(|node| (node.first_outgoing, node.last_outgoing, node.outgoing_count))
            .ok_or(ResourceGraphError::DanglingNode)?;
        if !self.relations.has_slots(1) {
            return Err(ResourceGraphError::Allocation {
                resource: "resource-graph-relation-slot",
            });
        }
        if outgoing_count == 0 {
            if first_outgoing.is_some() || last_outgoing.is_some() {
                return Err(ResourceGraphError::DanglingNode);
            }
        } else {
            first_outgoing.ok_or(ResourceGraphError::DanglingNode)?;
            let last = last_outgoing.ok_or(ResourceGraphError::DanglingNode)?;
            if self
                .relations
                .get(last)
                .is_none_or(|relation| relation.next_outgoing.is_some())
            {
                return Err(ResourceGraphError::DanglingNode);
            }
            let first = first_outgoing.ok_or(ResourceGraphError::DanglingNode)?;
            if self
                .relations
                .get(first)
                .is_none_or(|relation| relation.previous_outgoing.is_some())
            {
                return Err(ResourceGraphError::DanglingNode);
            }
        }
        let next_outgoing_count = outgoing_count
            .checked_add(1)
            .ok_or(ResourceGraphError::Overflow)?;
        let next_pending_relations = self
            .pending_relations
            .checked_sub(1)
            .ok_or(ResourceGraphError::IncompleteRelation)?;
        let next_relation_count = self
            .relation_count
            .checked_add(1)
            .ok_or(ResourceGraphError::Overflow)?;

        let relation_index = self.relations.len();
        self.relations
            .try_push_reserved(GraphRelation {
                role: pending.role,
                requirement: pending.requirement,
                reference,
                outcome,
                previous_outgoing: last_outgoing,
                next_outgoing: None,
            })
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-graph-relation-slot",
            })?;
        if let Some(last) = last_outgoing {
            self.relations
                .get_mut(last)
                .ok_or(ResourceGraphError::DanglingNode)?
                .next_outgoing = Some(relation_index);
        }
        let node = self
            .nodes
            .get_mut(pending.source)
            .ok_or(ResourceGraphError::DanglingNode)?;
        node.first_outgoing.get_or_insert(relation_index);
        node.last_outgoing = Some(relation_index);
        node.outgoing_count = next_outgoing_count;
        self.pending_relations = next_pending_relations;
        self.relation_count = next_relation_count;
        charge_consumed(host, ImportDimension::ResourceGraphRelations, 1)
    }

    fn bind_cache_node(
        &mut self,
        host: ImportStoreHost<'_>,
        cache_index: usize,
        node_index: usize,
    ) -> Result<(), ResourceGraphError> {
        if self.nodes.get(node_index).is_none() {
            return Err(ResourceGraphError::DanglingNode);
        }
        if cache_index >= self.cache_to_node.len() {
            self.cache_to_node
                .try_reserve(
                    cache_index + 1 - self.cache_to_node.len(),
                    "resource-graph-cache-index",
                    |bytes| reserve_graph_storage(host, bytes),
                )
                .map_err(ResourceGraphError::Scope)?;
            while self.cache_to_node.len() <= cache_index {
                self.cache_to_node.try_push_reserved(None).map_err(|_| {
                    ResourceGraphError::Allocation {
                        resource: "resource-graph-cache-index-slot",
                    }
                })?;
            }
        }
        match self.cache_to_node[cache_index] {
            Some(existing) if existing != node_index => Err(ResourceGraphError::DigestCollision),
            _ => {
                self.cache_to_node[cache_index] = Some(node_index);
                Ok(())
            }
        }
    }

    fn find_node(&self, identity: &DependencyIdentity) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .find_map(|(index, node)| (&node.exact_identity == identity).then_some(index))
    }

    fn outgoing_relations(
        &self,
        node_index: usize,
    ) -> Result<OutgoingRelations<'_>, ResourceGraphError> {
        let node = self
            .nodes
            .get(node_index)
            .ok_or(ResourceGraphError::DanglingNode)?;
        Ok(OutgoingRelations {
            relations: &self.relations,
            next: node.first_outgoing,
            remaining: node.outgoing_count,
            terminal_checked: false,
            direction: OutgoingDirection::Forward,
        })
    }

    fn outgoing_relations_rev(
        &self,
        node_index: usize,
    ) -> Result<OutgoingRelations<'_>, ResourceGraphError> {
        let node = self
            .nodes
            .get(node_index)
            .ok_or(ResourceGraphError::DanglingNode)?;
        Ok(OutgoingRelations {
            relations: &self.relations,
            next: node.last_outgoing,
            remaining: node.outgoing_count,
            terminal_checked: false,
            direction: OutgoingDirection::Reverse,
        })
    }

    fn plan_existing_target(
        &self,
        host: ImportStoreHost<'_>,
        target: usize,
        source: usize,
        target_depth: u64,
    ) -> Result<Vec<DepthUpdate>, ResourceGraphError> {
        let mut states = Vec::new();
        states
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-graph-depth-visit-state",
            })?;
        states.resize(self.nodes.len(), 0_u8);
        let mut stack = Vec::new();
        stack
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-graph-depth-stack",
            })?;
        stack.push((target, false));
        let mut postorder = Vec::new();
        postorder.try_reserve_exact(self.nodes.len()).map_err(|_| {
            ResourceGraphError::Allocation {
                resource: "resource-graph-depth-postorder",
            }
        })?;

        while let Some((index, expanded)) = stack.pop() {
            charge_graph_work(host, 1)?;
            if index == source {
                return Err(ResourceGraphError::Cycle);
            }
            self.nodes
                .get(index)
                .ok_or(ResourceGraphError::DanglingNode)?;
            if expanded {
                states[index] = 2;
                postorder.push(index);
                continue;
            }
            match states[index] {
                2 => continue,
                1 => return Err(ResourceGraphError::Cycle),
                _ => {}
            }
            states[index] = 1;
            stack.push((index, true));
            for relation in self.outgoing_relations_rev(index)? {
                let relation = relation?;
                charge_graph_work(host, 1)?;
                if let GraphRelationOutcome::Available { target } = &relation.outcome {
                    match states.get(*target).copied() {
                        Some(0) => stack.push((*target, false)),
                        Some(1) => return Err(ResourceGraphError::Cycle),
                        Some(2) => {}
                        _ => return Err(ResourceGraphError::DanglingNode),
                    }
                }
            }
        }

        let mut proposed = Vec::new();
        proposed.try_reserve_exact(self.nodes.len()).map_err(|_| {
            ResourceGraphError::Allocation {
                resource: "resource-graph-depth-projection",
            }
        })?;
        proposed.resize(self.nodes.len(), None);
        proposed[target] = Some(target_depth);
        let mut updates = Vec::new();
        updates
            .try_reserve_exact(postorder.len())
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-graph-depth-updates",
            })?;
        let maximum_depth = u64::try_from(self.limits.max_resource_graph_depth).unwrap_or(u64::MAX);
        for index in postorder.into_iter().rev() {
            charge_graph_work(host, 1)?;
            let depth = proposed[index].ok_or(ResourceGraphError::DanglingNode)?;
            if depth > maximum_depth {
                return Err(ResourceGraphError::Limit {
                    resource: "max_resource_graph_depth",
                    limit: maximum_depth,
                });
            }
            let node = self
                .nodes
                .get(index)
                .ok_or(ResourceGraphError::DanglingNode)?;
            if depth > node.depth {
                updates.push(DepthUpdate { index, depth });
            }
            let child_depth = depth.checked_add(1).ok_or(ResourceGraphError::Overflow)?;
            for relation in self.outgoing_relations(index)? {
                let relation = relation?;
                charge_graph_work(host, 1)?;
                if let GraphRelationOutcome::Available { target } = relation.outcome {
                    let projected = proposed
                        .get_mut(target)
                        .ok_or(ResourceGraphError::DanglingNode)?;
                    if projected.is_none_or(|current| child_depth > current) {
                        *projected = Some(child_depth);
                    }
                }
            }
        }
        Ok(updates)
    }

    fn apply_depth_updates(&mut self, updates: &[DepthUpdate]) -> Result<(), ResourceGraphError> {
        for update in updates {
            let node = self
                .nodes
                .get_mut(update.index)
                .ok_or(ResourceGraphError::DanglingNode)?;
            node.depth = node.depth.max(update.depth);
        }
        Ok(())
    }

    fn postorder(
        &self,
        host: ImportStoreHost<'_>,
        primary: usize,
    ) -> Result<AccountedVec<usize>, ResourceGraphError> {
        let mut states = AccountedVec::new();
        states
            .try_reserve_capacity(self.nodes.len(), "resource-manifest-visit-state", |bytes| {
                reserve_manifest_scratch(host, bytes)
            })
            .map_err(ResourceGraphError::Scope)?;
        for _ in 0..self.nodes.len() {
            states.push_reserved_infallible(0_u8);
        }
        let stack_capacity = self
            .nodes
            .len()
            .checked_add(self.relations.len())
            .and_then(|capacity| capacity.checked_add(1))
            .ok_or(ResourceGraphError::Overflow)?;
        let mut stack = AccountedVec::new();
        stack
            .try_reserve_capacity(
                stack_capacity,
                "resource-manifest-postorder-stack",
                |bytes| reserve_manifest_scratch(host, bytes),
            )
            .map_err(ResourceGraphError::Scope)?;
        stack.push_reserved_infallible((primary, false));
        let mut order = AccountedVec::new();
        order
            .try_reserve_capacity(self.nodes.len(), "resource-manifest-postorder", |bytes| {
                reserve_manifest_scratch(host, bytes)
            })
            .map_err(ResourceGraphError::Scope)?;
        while let Some((index, expanded)) = stack.pop() {
            charge_manifest_work(host, 1)?;
            self.nodes
                .get(index)
                .ok_or(ResourceGraphError::DanglingNode)?;
            if expanded {
                states[index] = 2;
                order.push_reserved_infallible(index);
                continue;
            }
            match states[index] {
                2 => continue,
                1 => return Err(ResourceGraphError::Cycle),
                _ => {}
            }
            states[index] = 1;
            stack.push_reserved_infallible((index, true));
            for relation in self.outgoing_relations_rev(index)? {
                let relation = relation?;
                charge_manifest_work(host, 1)?;
                if let GraphRelationOutcome::Available { target } = &relation.outcome {
                    match states.get(*target).copied() {
                        Some(0) => stack.push_reserved_infallible((*target, false)),
                        Some(1) => return Err(ResourceGraphError::Cycle),
                        Some(2) => {}
                        _ => return Err(ResourceGraphError::DanglingNode),
                    }
                }
            }
        }
        Ok(order)
    }

    fn require_node_capacity(&self, additional: usize) -> Result<(), ResourceGraphError> {
        let count = self
            .nodes
            .len()
            .checked_add(additional)
            .ok_or(ResourceGraphError::Overflow)?;
        if count > self.limits.max_resource_graph_nodes {
            return Err(ResourceGraphError::Limit {
                resource: "max_resource_graph_nodes",
                limit: self.limits.max_resource_graph_nodes as u64,
            });
        }
        Ok(())
    }

    fn validate_current_limits(&self) -> Result<(), ResourceGraphError> {
        self.require_node_capacity(0)?;
        if self.relation_count
            > u64::try_from(self.limits.max_resource_graph_relations).unwrap_or(u64::MAX)
        {
            return Err(ResourceGraphError::Limit {
                resource: "max_resource_graph_relations",
                limit: self.limits.max_resource_graph_relations as u64,
            });
        }
        if self.maximum_depth()
            > u64::try_from(self.limits.max_resource_graph_depth).unwrap_or(u64::MAX)
        {
            return Err(ResourceGraphError::Limit {
                resource: "max_resource_graph_depth",
                limit: self.limits.max_resource_graph_depth as u64,
            });
        }
        Ok(())
    }

    fn validate_relation_storage(&self) -> Result<(), ResourceGraphError> {
        let relation_count =
            usize::try_from(self.relation_count).map_err(|_| ResourceGraphError::Overflow)?;
        if relation_count != self.relations.len() {
            return Err(ResourceGraphError::IncompleteRelation);
        }
        let declared = self.nodes.iter().try_fold(0usize, |total, node| {
            total
                .checked_add(node.outgoing_count)
                .ok_or(ResourceGraphError::Overflow)
        })?;
        if declared != self.relations.len() {
            return Err(ResourceGraphError::DanglingNode);
        }
        Ok(())
    }

    fn maximum_depth(&self) -> u64 {
        self.nodes.iter().map(|node| node.depth).max().unwrap_or(0)
    }
}

fn fold_manifest_relations(
    host: ImportStoreHost<'_>,
    mut digested: AccountedVec<DigestedManifestRelation>,
) -> Result<AccountedVec<ManifestRelation>, ResourceGraphError> {
    sort_by_manifest_digest(host, &mut digested, |entry| entry.digest)?;
    let mut relations = manifest_scratch_vec(host, digested.len(), "resource-manifest-relations")?;
    let mut entries = digested.into_iter().peekable();
    while let Some(first) = entries.next() {
        let mut multiplicity = first.relation.multiplicity();
        while next_digest_matches(host, entries.peek(), first.digest, |entry| entry.digest)? {
            let next = entries
                .next()
                .expect("a peeked manifest relation remains available");
            if !manifest_bytes_equal(host, &first.canonical, &next.canonical)? {
                return Err(ResourceGraphError::DigestCollision);
            }
            multiplicity = multiplicity
                .checked_add(next.relation.multiplicity())
                .ok_or(ResourceGraphError::Overflow)?;
        }
        relations.push_reserved_infallible(first.relation.with_multiplicity(multiplicity));
    }
    Ok(relations)
}

fn fold_manifest_nodes(
    host: ImportStoreHost<'_>,
    mut digested: AccountedVec<DigestedManifestNode>,
    graph_nodes: &[GraphNode],
) -> Result<AccountedVec<ManifestNodeClass>, ResourceGraphError> {
    sort_by_manifest_digest(host, &mut digested, |entry| entry.digest)?;
    let mut classes = manifest_scratch_vec(
        host,
        digested.len(),
        "resource-manifest-folded-node-classes",
    )?;
    let mut entries = digested.into_iter().peekable();
    while let Some(first) = entries.next() {
        let source = graph_nodes
            .get(first.source_index)
            .ok_or(ResourceGraphError::DanglingNode)?;
        while next_digest_matches(host, entries.peek(), first.digest, |entry| entry.digest)? {
            let next = entries
                .next()
                .expect("a peeked manifest node remains available");
            if !manifest_bytes_equal(host, &first.canonical, &next.canonical)? {
                return Err(ResourceGraphError::DigestCollision);
            }
        }
        classes.push_reserved_infallible(ManifestNodeClass::new(
            first.digest,
            source.public_identity.clone(),
            source.byte_length,
            source.content_digest,
            first.relations,
        ));
    }
    Ok(classes)
}

fn sort_by_manifest_digest<T>(
    host: ImportStoreHost<'_>,
    values: &mut [T],
    digest: impl Fn(&T) -> ManifestDigest + Copy,
) -> Result<(), ResourceGraphError> {
    if values.len() < 2 {
        return Ok(());
    }
    for root in (0..values.len() / 2).rev() {
        sift_manifest_digest_heap(host, values, root, values.len(), digest)?;
    }
    for end in (1..values.len()).rev() {
        values.swap(0, end);
        sift_manifest_digest_heap(host, values, 0, end, digest)?;
    }
    Ok(())
}

fn sift_manifest_digest_heap<T>(
    host: ImportStoreHost<'_>,
    values: &mut [T],
    mut root: usize,
    end: usize,
    digest: impl Fn(&T) -> ManifestDigest + Copy,
) -> Result<(), ResourceGraphError> {
    while root < end / 2 {
        let child = root * 2 + 1;
        let mut larger = child;
        if child + 1 < end
            && manifest_digest_less(host, &values[child], &values[child + 1], digest)?
        {
            larger = child + 1;
        }
        if !manifest_digest_less(host, &values[root], &values[larger], digest)? {
            break;
        }
        values.swap(root, larger);
        root = larger;
    }
    Ok(())
}

fn manifest_digest_less<T>(
    host: ImportStoreHost<'_>,
    left: &T,
    right: &T,
    digest: impl Fn(&T) -> ManifestDigest,
) -> Result<bool, ResourceGraphError> {
    charge_manifest_work(host, 1)?;
    Ok(digest(left) < digest(right))
}

fn next_digest_matches<T>(
    host: ImportStoreHost<'_>,
    next: Option<&T>,
    expected: ManifestDigest,
    digest: impl FnOnce(&T) -> ManifestDigest,
) -> Result<bool, ResourceGraphError> {
    let Some(next) = next else {
        return Ok(false);
    };
    charge_manifest_work(host, 1)?;
    Ok(digest(next) == expected)
}

fn hash_manifest_bytes(
    host: ImportStoreHost<'_>,
    bytes: &[u8],
) -> Result<ManifestDigest, ResourceGraphError> {
    let work = u64::try_from(bytes.len()).map_err(|_| ResourceGraphError::Overflow)?;
    charge_attempted(host, ImportDimension::InputManifestWork, work)?;
    let digest = ManifestDigest::of(bytes);
    charge_consumed(host, ImportDimension::InputManifestWork, work)?;
    Ok(digest)
}

fn manifest_bytes_equal(
    host: ImportStoreHost<'_>,
    left: &[u8],
    right: &[u8],
) -> Result<bool, ResourceGraphError> {
    charge_manifest_work(host, 1)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.chunks(256).zip(right.chunks(256)) {
        let work = u64::try_from(left.len()).map_err(|_| ResourceGraphError::Overflow)?;
        charge_attempted(host, ImportDimension::InputManifestWork, work)?;
        let equal = left == right;
        charge_consumed(host, ImportDimension::InputManifestWork, work)?;
        if !equal {
            return Ok(false);
        }
    }
    Ok(true)
}

fn project_relation(
    relation: &GraphRelation,
    digests: &[Option<ManifestDigest>],
) -> Result<ManifestRelation, ResourceGraphError> {
    let outcome = match &relation.outcome {
        GraphRelationOutcome::Available { target } => ManifestRelationOutcome::Available {
            target_node_class_digest: digests
                .get(*target)
                .and_then(|digest| *digest)
                .ok_or(ResourceGraphError::DanglingNode)?,
        },
        GraphRelationOutcome::Unavailable {
            class,
            code,
            identity_evidence,
            replay,
        } => ManifestRelationOutcome::Unavailable {
            class: *class,
            code: *code,
            identity_evidence: identity_evidence.clone(),
            replay: *replay,
        },
    };
    Ok(ManifestRelation::new(
        relation.role.clone(),
        relation.requirement,
        relation.reference.clone(),
        outcome,
        1,
    ))
}

fn manifest_counts_and_replay_status(
    classes: &[ManifestNodeClass],
    maximum_depth: u64,
) -> Result<(ResourceManifestCounts, InputReplayStatus), ResourceGraphError> {
    let mut counts = ResourceManifestCounts {
        node_class_count: u64::try_from(classes.len()).map_err(|_| ResourceGraphError::Overflow)?,
        maximum_depth,
        ..ResourceManifestCounts::default()
    };
    let mut reasons = ManifestObservedOnlyReasons::new();
    for class in classes {
        note_unstable_identity(&mut reasons, class.identity());
        for relation in class.outgoing_relations() {
            if let ManifestReference::Normalized(identity) = relation.reference() {
                note_unstable_identity(&mut reasons, identity);
            }
            counts.quotient_relation_record_count = counts
                .quotient_relation_record_count
                .checked_add(1)
                .ok_or(ResourceGraphError::Overflow)?;
            counts.relation_occurrence_count = counts
                .relation_occurrence_count
                .checked_add(relation.multiplicity())
                .ok_or(ResourceGraphError::Overflow)?;
            match relation.outcome() {
                ManifestRelationOutcome::Available { .. } => {
                    counts.available_relation_record_count = counts
                        .available_relation_record_count
                        .checked_add(1)
                        .ok_or(ResourceGraphError::Overflow)?;
                    counts.available_relation_occurrence_count = counts
                        .available_relation_occurrence_count
                        .checked_add(relation.multiplicity())
                        .ok_or(ResourceGraphError::Overflow)?;
                }
                ManifestRelationOutcome::Unavailable {
                    identity_evidence,
                    replay,
                    ..
                } => {
                    counts.unavailable_relation_record_count = counts
                        .unavailable_relation_record_count
                        .checked_add(1)
                        .ok_or(ResourceGraphError::Overflow)?;
                    counts.unavailable_relation_occurrence_count = counts
                        .unavailable_relation_occurrence_count
                        .checked_add(relation.multiplicity())
                        .ok_or(ResourceGraphError::Overflow)?;
                    if let Some(identity) = identity_evidence {
                        note_unstable_identity(&mut reasons, identity);
                    }
                    match replay {
                        ManifestUnavailableReplay::NamespaceSnapshotProof(_)
                        | ManifestUnavailableReplay::PerReferenceProof(_) => {}
                        ManifestUnavailableReplay::UnprovenNegativeInput => {
                            reasons.insert(ManifestObservedOnlyReason::UnprovenNegativeInput);
                        }
                        ManifestUnavailableReplay::SessionDependent(_) => {
                            reasons.insert(ManifestObservedOnlyReason::SessionDependentUnavailable);
                        }
                    }
                }
            }
        }
    }
    let replay_status = if reasons.is_empty() {
        InputReplayStatus::Replayable
    } else {
        InputReplayStatus::ObservedOnly(reasons)
    };
    Ok((counts, replay_status))
}

fn note_unstable_identity(
    reasons: &mut ManifestObservedOnlyReasons,
    identity: &ManifestResourceIdentity,
) {
    if matches!(
        identity.stability(),
        ManifestResourceIdentityStability::SessionOpaque(_)
    ) {
        reasons.insert(ManifestObservedOnlyReason::UnstableResourceIdentity);
    }
}

fn manifest_identity(
    path: &AssetPath,
    reason: ManifestSessionOpaqueReason,
    resource: &'static str,
) -> Result<ManifestResourceIdentity, ResourceGraphError> {
    Ok(ManifestResourceIdentity::new(
        path.try_clone()
            .map_err(|_| ResourceGraphError::Allocation { resource })?,
        ManifestResourceIdentityStability::SessionOpaque(reason),
    ))
}

fn manifest_identity_for_resolution(
    path: &AssetPath,
    stable_identity_schema: Option<&'static StableResourceIdentitySchema>,
    resource: &'static str,
) -> Result<ManifestResourceIdentity, ResourceGraphError> {
    let Some(schema) = stable_identity_schema else {
        return manifest_identity(
            path,
            ManifestSessionOpaqueReason::UnregisteredResolverIdentity,
            resource,
        );
    };
    Ok(ManifestResourceIdentity::new(
        path.try_clone()
            .map_err(|_| ResourceGraphError::Allocation { resource })?,
        ManifestResourceIdentityStability::Stable {
            owner: schema.owner(),
            schema: schema.schema_id(),
            version: u64::from(schema.schema_version().get()),
        },
    ))
}

pub(crate) fn unavailable_disposition(cause: &ScopeAbortCause) -> ResourceUnavailableDisposition {
    let (class, code, replay) = match cause {
        ScopeAbortCause::Path(_) => (
            ManifestUnavailableClass::Reference,
            ManifestUnavailableCode::new("path-rejected"),
            ManifestUnavailableReplay::SessionDependent(
                ManifestSessionDependentReason::UnregisteredSemanticBasis,
            ),
        ),
        ScopeAbortCause::DependencyPolicy(_) => (
            ManifestUnavailableClass::Policy,
            ManifestUnavailableCode::new("policy-rejected"),
            ManifestUnavailableReplay::SessionDependent(
                ManifestSessionDependentReason::UnregisteredSemanticBasis,
            ),
        ),
        ScopeAbortCause::AssetOpen(_) => (
            ManifestUnavailableClass::Io,
            ManifestUnavailableCode::new("asset-open-failed"),
            ManifestUnavailableReplay::UnprovenNegativeInput,
        ),
        ScopeAbortCause::AssetRead(_) => (
            ManifestUnavailableClass::Io,
            ManifestUnavailableCode::new("asset-read-failed"),
            ManifestUnavailableReplay::UnprovenNegativeInput,
        ),
        ScopeAbortCause::Ledger(_) | ScopeAbortCause::ResourceLimit { .. } => (
            ManifestUnavailableClass::Resource,
            ManifestUnavailableCode::new("resource-limit"),
            ManifestUnavailableReplay::SessionDependent(
                ManifestSessionDependentReason::AcquisitionBudget,
            ),
        ),
        ScopeAbortCause::Control(_) => (
            ManifestUnavailableClass::Control,
            ManifestUnavailableCode::new("operation-control"),
            ManifestUnavailableReplay::SessionDependent(
                ManifestSessionDependentReason::HostCallback,
            ),
        ),
        ScopeAbortCause::TrustedExtension { .. } => (
            ManifestUnavailableClass::TrustedExtension,
            ManifestUnavailableCode::new("trusted-extension-failed"),
            ManifestUnavailableReplay::SessionDependent(
                ManifestSessionDependentReason::HostCallback,
            ),
        ),
        ScopeAbortCause::Diagnostic(_)
        | ScopeAbortCause::FormatRecoveryRejectedByConformance { .. }
        | ScopeAbortCause::FormatRecoveryRejectedByLossPolicy { .. }
        | ScopeAbortCause::FormatFailureDetail(_)
        | ScopeAbortCause::Allocation { .. }
        | ScopeAbortCause::CapacityEnvelope { .. }
        | ScopeAbortCause::OptionTypeMismatch
        | ScopeAbortCause::DescriptorContract { .. } => (
            ManifestUnavailableClass::Contract,
            ManifestUnavailableCode::new("import-contract-failed"),
            ManifestUnavailableReplay::SessionDependent(
                ManifestSessionDependentReason::ProcessState,
            ),
        ),
    };
    ResourceUnavailableDisposition {
        class,
        code,
        replay,
        stable_identity_schema: None,
    }
}

pub(crate) fn cached_unavailable_disposition(
    failure: &CachedDependencyFailure,
) -> Result<ResourceUnavailableDisposition, ScopeAbortCause> {
    let (class, code, replay) = match failure.unavailable() {
        CachedDependencyUnavailable::EnvironmentDependent { code, observation } => {
            let registered_cause = code.registered_cause();
            let code = match code {
                CachedDependencyIoCode::Missing => ManifestUnavailableCode::new("missing"),
                CachedDependencyIoCode::OpenFailed => ManifestUnavailableCode::new("open-failed"),
                CachedDependencyIoCode::ReadFailed => ManifestUnavailableCode::new("read-failed"),
            };
            let replay = match observation {
                CachedNegativeObservation::None => ManifestUnavailableReplay::UnprovenNegativeInput,
                CachedNegativeObservation::Proof(proof) => {
                    let identity_schema = failure.stable_identity_schema().ok_or(
                        ScopeAbortCause::DescriptorContract {
                            code: "registered-negative-proof-manifest-missing-stable-identity",
                        },
                    )?;
                    registered_negative_proof_replay(proof, identity_schema, registered_cause)
                        .map_err(|_| ScopeAbortCause::DescriptorContract {
                            code: "registered-negative-proof-manifest-mismatch",
                        })?
                }
            };
            (ManifestUnavailableClass::Io, code, replay)
        }
        CachedDependencyUnavailable::AcquisitionBudget => (
            ManifestUnavailableClass::Resource,
            ManifestUnavailableCode::new("resource-limit"),
            ManifestUnavailableReplay::SessionDependent(
                ManifestSessionDependentReason::AcquisitionBudget,
            ),
        ),
        CachedDependencyUnavailable::ProcessState => (
            ManifestUnavailableClass::Contract,
            ManifestUnavailableCode::new("import-contract-failed"),
            ManifestUnavailableReplay::SessionDependent(
                ManifestSessionDependentReason::ProcessState,
            ),
        ),
    };
    Ok(ResourceUnavailableDisposition {
        class,
        code,
        replay,
        stable_identity_schema: failure.stable_identity_schema(),
    })
}

fn charge_attempted(
    host: ImportStoreHost<'_>,
    dimension: ImportDimension,
    amount: u64,
) -> Result<(), ResourceGraphError> {
    host.charge_attempted(dimension, amount)
        .map_err(|cause| ResourceGraphError::Scope(cause.into_scope_abort_cause()))
}

fn charge_consumed(
    host: ImportStoreHost<'_>,
    dimension: ImportDimension,
    amount: u64,
) -> Result<(), ResourceGraphError> {
    host.charge_consumed(dimension, amount)
        .map_err(|cause| ResourceGraphError::Scope(cause.into_scope_abort_cause()))
}

fn charge_graph_work(host: ImportStoreHost<'_>, amount: u64) -> Result<(), ResourceGraphError> {
    charge_attempted(host, ImportDimension::ResourceGraphWork, amount)?;
    charge_consumed(host, ImportDimension::ResourceGraphWork, amount)
}

fn charge_manifest_work(host: ImportStoreHost<'_>, amount: u64) -> Result<(), ResourceGraphError> {
    charge_attempted(host, ImportDimension::InputManifestWork, amount)?;
    charge_consumed(host, ImportDimension::InputManifestWork, amount)
}

fn reserve_graph_storage(
    host: ImportStoreHost<'_>,
    bytes: u64,
) -> Result<ImportStoreReservation, ScopeAbortCause> {
    host.reserve(
        ImportLedgerPhase::DependencyAcquisition,
        ImportDimension::ScratchBytes,
        bytes,
    )
    .map_err(|cause| cause.into_scope_abort_cause())
}

fn reserve_manifest_storage(
    host: ImportStoreHost<'_>,
    bytes: u64,
) -> Result<ImportStoreReservation, ScopeAbortCause> {
    host.reserve(
        ImportLedgerPhase::OutcomeFinalization,
        ImportDimension::InputManifestBytes,
        bytes,
    )
    .map_err(|cause| cause.into_scope_abort_cause())
}

fn reserve_manifest_scratch(
    host: ImportStoreHost<'_>,
    bytes: u64,
) -> Result<ImportStoreReservation, ScopeAbortCause> {
    host.reserve(
        ImportLedgerPhase::OutcomeFinalization,
        ImportDimension::ScratchBytes,
        bytes,
    )
    .map_err(|cause| cause.into_scope_abort_cause())
}

fn manifest_scratch_vec<T>(
    host: ImportStoreHost<'_>,
    capacity: usize,
    resource: &'static str,
) -> Result<AccountedVec<T>, ResourceGraphError> {
    let mut values = AccountedVec::new();
    values
        .try_reserve_capacity(capacity, resource, |bytes| {
            reserve_manifest_scratch(host, bytes)
        })
        .map_err(ResourceGraphError::Scope)?;
    Ok(values)
}

fn dependency_cache_error(cause: DependencyCacheCapacityError) -> ResourceGraphError {
    let cause = match cause {
        DependencyCacheCapacityError::Capacity { storage, cause } => {
            dependency_cache_capacity_plan_cause(dependency_cache_storage_resource(storage), cause)
        }
        DependencyCacheCapacityError::Allocation { storage } => ScopeAbortCause::Allocation {
            resource: dependency_cache_storage_resource(storage),
        },
        DependencyCacheCapacityError::Stale => ScopeAbortCause::DescriptorContract {
            code: "dependency-cache-capacity-plan-stale",
        },
        DependencyCacheCapacityError::IndexContract => ScopeAbortCause::DescriptorContract {
            code: "dependency-cache-index-contract-failed",
        },
    };
    ResourceGraphError::Scope(cause)
}

const fn dependency_cache_storage_resource(storage: DependencyCacheStorage) -> &'static str {
    match storage {
        DependencyCacheStorage::Entries => "dependency-cache-entry-capacity",
        DependencyCacheStorage::Index => "dependency-cache-index-capacity",
    }
}

fn grow_acquisition_reservation(
    host: ImportStoreHost<'_>,
    storage: DependencyCacheStorage,
    temporary_envelope: u64,
    reservation: &mut Option<ImportStoreReservation>,
) -> Result<(), ScopeAbortCause> {
    if temporary_envelope == 0 {
        return Ok(());
    }
    let prior_bytes = reservation
        .as_ref()
        .map_or(0, ImportStoreReservation::bytes);
    let target = prior_bytes.checked_add(temporary_envelope).ok_or_else(|| {
        dependency_cache_capacity_plan_cause(
            dependency_cache_storage_resource(storage),
            CapacityPlanError::CapacityOverflow,
        )
    })?;
    if let Some(reservation) = reservation.as_mut() {
        host.resize(reservation, target)
            .map_err(|cause| cause.into_scope_abort_cause())
    } else {
        *reservation = Some(
            host.reserve(
                ImportLedgerPhase::DependencyAcquisition,
                ImportDimension::ScratchBytes,
                target,
            )
            .map_err(|cause| cause.into_scope_abort_cause())?,
        );
        Ok(())
    }
}

fn dependency_cache_capacity_plan_cause(
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
            code: "capacity-plan-contract-failed",
        },
    }
}

fn restore_acquisition_reservations(
    cache: &mut DependencyCache,
    host: ImportStoreHost<'_>,
    mut entry_reservation: Option<ImportStoreReservation>,
    entry_prior_bytes: u64,
    mut index_reservation: Option<ImportStoreReservation>,
    index_prior_bytes: u64,
) {
    for (reservation, prior_bytes) in [
        (&mut entry_reservation, entry_prior_bytes),
        (&mut index_reservation, index_prior_bytes),
    ] {
        if prior_bytes == 0 {
            drop(reservation.take());
        } else if let Some(reservation) = reservation.as_mut() {
            let _ = host.resize(reservation, prior_bytes);
        }
    }
    cache.retain_capacity_reservation(DependencyCacheStorage::Entries, entry_reservation);
    cache.retain_capacity_reservation(DependencyCacheStorage::Index, index_reservation);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::ImportOperationControl;
    use crate::store::{
        ImportStoreBootstrapRequirements, ImportStoreSessionOwner, try_import_store_bootstrap,
    };
    use baozi_operation::{
        DimensionPolicy, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits,
    };

    fn store_owner(limits: &ResourceLimits) -> ImportStoreSessionOwner {
        let requirements = ImportStoreBootstrapRequirements::for_node_scene(limits).unwrap();
        let bootstrap = try_import_store_bootstrap(limits, requirements).unwrap();
        let factory = PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
        )
        .unwrap();
        let account = factory
            .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX).unwrap())
            .unwrap();
        ImportStoreSessionOwner::try_start(ImportOperationControl::default(), &account, bootstrap)
            .unwrap()
    }

    fn dependency_identity(authority: &AuthorityDomain, path: &str) -> DependencyIdentity {
        DependencyIdentity::new(authority.clone(), AssetPath::new(path).unwrap())
    }

    fn manifest_identity_for_test(path: &str) -> ManifestResourceIdentity {
        ManifestResourceIdentity::new(
            AssetPath::new(path).unwrap(),
            ManifestResourceIdentityStability::SessionOpaque(
                ManifestSessionOpaqueReason::UnregisteredResolverIdentity,
            ),
        )
    }

    fn stable_manifest_identity_for_test(path: &str) -> ManifestResourceIdentity {
        ManifestResourceIdentity::new(
            AssetPath::new(path).unwrap(),
            ManifestResourceIdentityStability::Stable {
                owner: "dev.baozi.test",
                schema: "resource-identity",
                version: 1,
            },
        )
    }

    fn manifest_relation_for_test(reference: &str) -> ManifestRelation {
        ManifestRelation::new(
            DependencyRoleId::new("test.resource").unwrap(),
            DependencyRequirement::Required,
            ManifestReference::Normalized(manifest_identity_for_test(reference)),
            ManifestRelationOutcome::Available {
                target_node_class_digest: ManifestDigest::of(b"target"),
            },
            1,
        )
    }

    fn graph_node_for_test(
        authority: &AuthorityDomain,
        exact_path: &str,
        public_path: &str,
    ) -> GraphNode {
        GraphNode {
            exact_identity: dependency_identity(authority, exact_path),
            public_identity: manifest_identity_for_test(public_path),
            byte_length: 4,
            content_digest: ManifestDigest::of(b"same"),
            first_outgoing: None,
            last_outgoing: None,
            outgoing_count: 0,
            depth: 0,
        }
    }

    fn replay_status_for_relation(
        host: ImportStoreHost<'_>,
        relation: ManifestRelation,
    ) -> InputReplayStatus {
        let class = ManifestNodeClass::new(
            ManifestDigest::of(b"class"),
            stable_manifest_identity_for_test("primary.bin"),
            7,
            ManifestDigest::of(b"primary"),
            accounted_manifest_values(host, vec![relation]),
        );
        manifest_counts_and_replay_status(&[class], 0).unwrap().1
    }

    fn accounted_manifest_values<T>(host: ImportStoreHost<'_>, values: Vec<T>) -> AccountedVec<T> {
        let mut accounted =
            manifest_scratch_vec(host, values.len(), "resource-manifest-test-values").unwrap();
        for value in values {
            accounted.push_reserved_infallible(value);
        }
        accounted
    }

    fn digested_manifest_relation_for_test(
        host: ImportStoreHost<'_>,
        digest: ManifestDigest,
        reference: &str,
    ) -> DigestedManifestRelation {
        let relation = manifest_relation_for_test(reference);
        let canonical_len = relation_key_len(
            relation.role(),
            relation.requirement(),
            relation.reference(),
            relation.outcome(),
        )
        .unwrap();
        let mut canonical =
            manifest_scratch_vec(host, canonical_len, "resource-manifest-test-relation-key")
                .unwrap();
        encode_relation_key_into_accounted(
            relation.role(),
            relation.requirement(),
            relation.reference(),
            relation.outcome(),
            canonical_len,
            &mut canonical,
        )
        .unwrap();
        DigestedManifestRelation {
            digest,
            canonical,
            relation,
        }
    }

    fn digested_manifest_node_for_test(
        host: ImportStoreHost<'_>,
        digest: ManifestDigest,
        source_index: usize,
        source: &GraphNode,
    ) -> DigestedManifestNode {
        let relations =
            manifest_scratch_vec(host, 0, "resource-manifest-test-node-relations").unwrap();
        let canonical_len = node_class_len(
            &source.public_identity,
            source.byte_length,
            source.content_digest,
            &relations,
        )
        .unwrap();
        let mut canonical =
            manifest_scratch_vec(host, canonical_len, "resource-manifest-test-node-class").unwrap();
        encode_node_class_into_accounted(
            &source.public_identity,
            source.byte_length,
            source.content_digest,
            &relations,
            canonical_len,
            &mut canonical,
        )
        .unwrap();
        DigestedManifestNode {
            digest,
            canonical,
            source_index,
            relations,
        }
    }

    fn manifest_comparison_work_for_test(left: &[u8], right: &[u8]) -> u64 {
        let mut work = 1_u64;
        if left.len() != right.len() {
            return work;
        }
        for (left, right) in left.chunks(256).zip(right.chunks(256)) {
            work += u64::try_from(left.len()).unwrap();
            if left != right {
                break;
            }
        }
        work
    }

    fn add_available(
        graph: &mut ResourceGraph,
        host: ImportStoreHost<'_>,
        source: usize,
        role: &DependencyRoleId,
        cache_index: usize,
        identity: &DependencyIdentity,
        bytes: &[u8],
    ) -> usize {
        let mut pending = graph
            .begin_relation(
                host,
                source,
                role,
                DependencyRequirement::Required,
                identity.path().as_str(),
            )
            .unwrap();
        graph.prepare_target(host, &mut pending, identity).unwrap();
        graph
            .complete_available(
                host,
                pending,
                cache_index,
                bytes,
                AvailableResourceIdentity::new(identity, identity.path(), None),
            )
            .unwrap();
        graph.source_for_cache(cache_index).unwrap()
    }

    fn shared_depth_prefix(
        host: ImportStoreHost<'_>,
        maximum_depth: usize,
    ) -> (ResourceGraph, DependencyRoleId, DependencyIdentity, usize) {
        let mut limits = ResourceLimits {
            max_resource_graph_depth: maximum_depth,
            ..ResourceLimits::default()
        };
        limits.max_resource_graph_nodes = 4;
        let authority = AuthorityDomain::default();
        let primary_path = AssetPath::new("main.asset").unwrap();
        let role = DependencyRoleId::new("test.resource").unwrap();
        let mut graph = ResourceGraph::new(limits);
        graph
            .bind_primary(host, &authority, &primary_path, b"primary")
            .unwrap();
        let primary = graph.source_for_primary().unwrap();
        let identity_a = dependency_identity(&authority, "a.asset");
        let node_a = add_available(&mut graph, host, primary, &role, 0, &identity_a, b"a");
        let identity_b = dependency_identity(&authority, "b.asset");
        add_available(&mut graph, host, node_a, &role, 1, &identity_b, b"b");
        let identity_c = dependency_identity(&authority, "c.asset");
        let node_c = add_available(&mut graph, host, primary, &role, 2, &identity_c, b"c");
        (graph, role, identity_a, node_c)
    }

    #[test]
    fn shared_existing_target_rejects_descendant_depth_overflow() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let (mut graph, role, identity_a, node_c) = shared_depth_prefix(host, 2);
        let mut pending = graph
            .begin_relation(
                host,
                node_c,
                &role,
                DependencyRequirement::Required,
                identity_a.path().as_str(),
            )
            .unwrap();

        assert!(matches!(
            graph.prepare_target(host, &mut pending, &identity_a),
            Err(ResourceGraphError::Limit {
                resource: "max_resource_graph_depth",
                limit: 2,
            })
        ));
    }

    #[test]
    fn shared_existing_target_updates_manifest_maximum_depth() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let (mut graph, role, identity_a, node_c) = shared_depth_prefix(host, 3);
        let mut pending = graph
            .begin_relation(
                host,
                node_c,
                &role,
                DependencyRequirement::Required,
                identity_a.path().as_str(),
            )
            .unwrap();
        graph
            .prepare_target(host, &mut pending, &identity_a)
            .unwrap();
        graph
            .complete_available(
                host,
                pending,
                0,
                b"a",
                AvailableResourceIdentity::new(&identity_a, identity_a.path(), None),
            )
            .unwrap();
        graph.seal(host).unwrap();

        assert_eq!(graph.take_manifest().unwrap().counts().maximum_depth(), 3);
    }

    #[test]
    fn graph_arenas_hold_one_accounted_owner_per_storage_class() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let authority = AuthorityDomain::default();
        let primary_path = AssetPath::new("main.asset").unwrap();
        let role = DependencyRoleId::new("test.resource").unwrap();
        let identity = dependency_identity(&authority, "dependency.asset");
        let mut graph = ResourceGraph::new(limits);

        graph
            .bind_primary(host, &authority, &primary_path, b"primary")
            .unwrap();
        let primary = graph.source_for_primary().unwrap();
        add_available(
            &mut graph,
            host,
            primary,
            &role,
            0,
            &identity,
            b"dependency",
        );

        let accounted = graph
            .nodes
            .capacity_bytes()
            .checked_add(graph.relations.capacity_bytes())
            .and_then(|bytes| bytes.checked_add(graph.cache_to_node.capacity_bytes()))
            .unwrap();
        assert!(graph.nodes.capacity_bytes() > 0);
        assert!(graph.relations.capacity_bytes() > 0);
        assert!(graph.cache_to_node.capacity_bytes() > 0);
        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes(),
            accounted
        );

        drop(graph);
        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes(),
            0
        );
    }

    #[test]
    fn manifest_postorder_scratch_is_released_after_seal() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let authority = AuthorityDomain::default();
        let primary_path = AssetPath::new("main.asset").unwrap();
        let mut graph = ResourceGraph::new(limits);
        graph
            .bind_primary(host, &authority, &primary_path, b"primary")
            .unwrap();
        let graph_storage = host
            .dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes();

        graph.seal(host).unwrap();

        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes(),
            graph_storage
        );
    }

    #[test]
    fn manifest_postorder_scratch_is_admitted_before_manifest_bytes() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let authority = AuthorityDomain::default();
        let primary_path = AssetPath::new("main.asset").unwrap();
        let mut graph = ResourceGraph::new(limits);
        graph
            .bind_primary(host, &authority, &primary_path, b"primary")
            .unwrap();
        let graph_storage = host
            .dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes();
        host.tighten_dimension_policy(
            ImportDimension::ScratchBytes,
            DimensionPolicy::reservable_bytes(0, 0, 0, graph_storage),
        )
        .unwrap();

        assert!(graph.seal(host).is_err());
        let manifest_bytes = host
            .dimension_state(ImportDimension::InputManifestBytes)
            .unwrap();
        assert_eq!(manifest_bytes.attempted(), 0);
        assert_eq!(manifest_bytes.consumed(), 0);
        assert_eq!(manifest_bytes.live_bytes(), 0);
        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes(),
            graph_storage
        );
        assert!(graph.sealed_manifest.is_none());
    }

    #[test]
    fn graph_node_storage_is_admitted_before_primary_mutation() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let existing_live = host
            .dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes();
        host.tighten_dimension_policy(
            ImportDimension::ScratchBytes,
            DimensionPolicy::reservable_bytes(0, 0, 0, existing_live + 1),
        )
        .unwrap();
        let authority = AuthorityDomain::default();
        let primary_path = AssetPath::new("main.asset").unwrap();
        let mut graph = ResourceGraph::new(limits);

        assert!(
            graph
                .bind_primary(host, &authority, &primary_path, b"primary")
                .is_err()
        );
        assert!(graph.nodes.is_empty());
        assert_eq!(graph.nodes.capacity_bytes(), 0);
        assert!(graph.primary.is_none());
    }

    #[test]
    fn dependency_cache_preallocation_failure_is_bookkeeping_not_structural_graph_limit() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        host.tighten_dimension_policy(
            ImportDimension::ScratchBytes,
            DimensionPolicy::reservable_bytes(0, 0, 0, 0),
        )
        .unwrap();
        let mut graph = ResourceGraph::new(limits);

        let failure = graph
            .reserve_acquisition_capacity(host)
            .expect_err("dependency-cache capacity must be admitted before cache mutation");

        match failure {
            ResourceGraphError::Scope(ScopeAbortCause::Ledger(
                baozi_operation::LedgerCause::LimitExceeded {
                    dimension: ImportDimension::ScratchBytes,
                    ..
                },
            )) => {}
            ResourceGraphError::Limit {
                resource:
                    "max_resource_graph_nodes"
                    | "max_resource_graph_relations"
                    | "max_resource_graph_depth",
                ..
            } => panic!("cache preallocation failure was relabeled as a structural graph limit"),
            other => panic!("unexpected dependency-cache preallocation failure: {other:?}"),
        }
        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes(),
            0
        );
        assert!(
            graph.cache.capacity_plan().unwrap().requires_apply(),
            "failed preallocation must leave cache growth unapplied"
        );
    }

    #[test]
    fn equal_digest_relations_fold_only_when_canonical_bytes_match() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let forced_digest = ManifestDigest::of(b"forced-relation-digest");
        let first = digested_manifest_relation_for_test(host, forced_digest, "shared.asset");
        let second = digested_manifest_relation_for_test(host, forced_digest, "shared.asset");
        let equal_work = 2 + manifest_comparison_work_for_test(&first.canonical, &second.canonical);
        let folded =
            fold_manifest_relations(host, accounted_manifest_values(host, vec![first, second]))
                .unwrap();
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].multiplicity(), 2);
        let work = host
            .dimension_state(ImportDimension::InputManifestWork)
            .unwrap();
        assert_eq!(work.attempted(), equal_work);
        assert_eq!(work.consumed(), equal_work);

        let first = digested_manifest_relation_for_test(host, forced_digest, "first.asset");
        let second = digested_manifest_relation_for_test(host, forced_digest, "second.asset");
        let collision_work =
            2 + manifest_comparison_work_for_test(&first.canonical, &second.canonical);

        assert!(matches!(
            fold_manifest_relations(host, accounted_manifest_values(host, vec![first, second]),),
            Err(ResourceGraphError::DigestCollision)
        ));
        let work = host
            .dimension_state(ImportDimension::InputManifestWork)
            .unwrap();
        assert_eq!(work.attempted(), equal_work + collision_work);
        assert_eq!(work.consumed(), equal_work + collision_work);
    }

    #[test]
    fn equal_digest_nodes_fold_only_when_canonical_bytes_match() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let first_authority = AuthorityDomain::new();
        let second_authority = AuthorityDomain::new();
        let forced_digest = ManifestDigest::of(b"forced-node-digest");
        let graph_nodes = vec![
            graph_node_for_test(&first_authority, "first.asset", "shared.asset"),
            graph_node_for_test(&second_authority, "second.asset", "shared.asset"),
        ];
        let first = digested_manifest_node_for_test(host, forced_digest, 0, &graph_nodes[0]);
        let second = digested_manifest_node_for_test(host, forced_digest, 1, &graph_nodes[1]);
        let equal_work = 2 + manifest_comparison_work_for_test(&first.canonical, &second.canonical);
        let folded = fold_manifest_nodes(
            host,
            accounted_manifest_values(host, vec![first, second]),
            &graph_nodes,
        )
        .unwrap();
        assert_eq!(folded.len(), 1);
        let work = host
            .dimension_state(ImportDimension::InputManifestWork)
            .unwrap();
        assert_eq!(work.attempted(), equal_work);
        assert_eq!(work.consumed(), equal_work);

        let graph_nodes = vec![
            graph_node_for_test(&first_authority, "first.asset", "first.asset"),
            graph_node_for_test(&second_authority, "second.asset", "second.asset"),
        ];
        let first = digested_manifest_node_for_test(host, forced_digest, 0, &graph_nodes[0]);
        let second = digested_manifest_node_for_test(host, forced_digest, 1, &graph_nodes[1]);
        let collision_work =
            2 + manifest_comparison_work_for_test(&first.canonical, &second.canonical);
        assert!(matches!(
            fold_manifest_nodes(
                host,
                accounted_manifest_values(host, vec![first, second]),
                &graph_nodes,
            ),
            Err(ResourceGraphError::DigestCollision)
        ));
        let work = host
            .dimension_state(ImportDimension::InputManifestWork)
            .unwrap();
        assert_eq!(work.attempted(), equal_work + collision_work);
        assert_eq!(work.consumed(), equal_work + collision_work);
    }

    #[test]
    fn replay_status_checks_reference_and_unavailable_identity_stability() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let normalized_unstable = ManifestRelation::new(
            DependencyRoleId::new("test.resource").unwrap(),
            DependencyRequirement::Required,
            ManifestReference::Normalized(manifest_identity_for_test("dependency.bin")),
            ManifestRelationOutcome::Available {
                target_node_class_digest: ManifestDigest::of(b"target"),
            },
            1,
        );
        assert!(matches!(
            replay_status_for_relation(host, normalized_unstable),
            InputReplayStatus::ObservedOnly(reasons)
                if reasons.as_slice() == [ManifestObservedOnlyReason::UnstableResourceIdentity]
        ));

        let proof = ManifestNamespaceSnapshotProof::new(
            "dev.baozi.test.absence",
            1,
            "dev.baozi.test.membership-v1",
            ManifestSemanticIdentity::new(
                "dev.baozi.test",
                "resolver",
                1,
                ManifestDigest::of(b"resolver"),
            ),
            ManifestDigest::of(b"namespace"),
            ManifestRegisteredUnavailableCause::new("baozi.io", "missing"),
            ManifestDigest::of(b"proof"),
        );
        let unavailable_identity_unstable = ManifestRelation::new(
            DependencyRoleId::new("test.resource").unwrap(),
            DependencyRequirement::Required,
            ManifestReference::Normalized(stable_manifest_identity_for_test("dependency.bin")),
            ManifestRelationOutcome::Unavailable {
                class: ManifestUnavailableClass::Io,
                code: ManifestUnavailableCode::new("missing"),
                identity_evidence: Some(manifest_identity_for_test("dependency.bin")),
                replay: ManifestUnavailableReplay::NamespaceSnapshotProof(proof),
            },
            1,
        );
        assert!(matches!(
            replay_status_for_relation(host, unavailable_identity_unstable),
            InputReplayStatus::ObservedOnly(reasons)
                if reasons.as_slice() == [ManifestObservedOnlyReason::UnstableResourceIdentity]
        ));
    }

    #[test]
    fn digest_sort_records_only_the_first_rejected_comparison_attempt() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let scratch_baseline = host
            .dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes();
        let forced_digest = ManifestDigest::of(b"forced-relation-digest");
        let digested = accounted_manifest_values(
            host,
            vec![
                digested_manifest_relation_for_test(host, forced_digest, "first.asset"),
                digested_manifest_relation_for_test(host, forced_digest, "second.asset"),
            ],
        );
        host.tighten_dimension_policy(
            ImportDimension::InputManifestWork,
            DimensionPolicy::counter(0, 0, 0),
        )
        .unwrap();

        assert!(fold_manifest_relations(host, digested).is_err());
        let work = host
            .dimension_state(ImportDimension::InputManifestWork)
            .unwrap();
        assert_eq!(work.attempted(), 1);
        assert_eq!(work.consumed(), 0);
        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes(),
            scratch_baseline
        );
    }

    #[test]
    fn primary_only_manifest_is_complete_and_observed_only() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let authority = AuthorityDomain::default();
        let path = AssetPath::new("models/main.obj").unwrap();
        let mut graph = ResourceGraph::new(limits);
        graph
            .bind_primary(host, &authority, &path, b"v 0 0 0")
            .unwrap();
        let node = graph.nodes.first().unwrap();
        let node_hash_work = u64::try_from(
            node_class_len(
                &node.public_identity,
                node.byte_length,
                node.content_digest,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        graph.seal(host).unwrap();
        let manifest = graph.take_manifest().unwrap();
        let canonical = manifest.canonical_bytes().unwrap();
        let expected_digest = [
            0x69, 0x67, 0x8e, 0x43, 0xf1, 0xde, 0x95, 0xd9, 0x01, 0x3c, 0xf1, 0x6d, 0x22, 0x62,
            0x0b, 0x14, 0x7a, 0xcd, 0x0b, 0xd7, 0x22, 0xa8, 0x44, 0xfc, 0xcf, 0x32, 0x8e, 0xd5,
            0x3b, 0x5b, 0xcd, 0x68,
        ];
        assert_eq!(canonical.len(), 5_327);
        assert_eq!(manifest.digest().as_bytes(), expected_digest);
        assert_eq!(ManifestDigest::of(&canonical).as_bytes(), expected_digest);
        let manifest_bytes = host
            .dimension_state(ImportDimension::InputManifestBytes)
            .unwrap();
        assert_eq!(manifest_bytes.attempted(), 5_327);
        assert_eq!(manifest_bytes.consumed(), 5_327);
        assert_eq!(manifest_bytes.live_bytes(), 0);
        let manifest_work = host
            .dimension_state(ImportDimension::InputManifestWork)
            .unwrap();
        let expected_work = 3 + node_hash_work + 5_327;
        assert_eq!(manifest_work.attempted(), expected_work);
        assert_eq!(manifest_work.consumed(), expected_work);
        assert_eq!(manifest.schema_version(), 1);
        assert_eq!(manifest.counts().node_class_count(), 1);
        assert_eq!(manifest.counts().quotient_relation_record_count(), 0);
        assert!(matches!(
            manifest.replay_status(),
            InputReplayStatus::ObservedOnly(reasons)
                if reasons.as_slice()
                    == [ManifestObservedOnlyReason::UnstableResourceIdentity]
        ));
    }

    #[test]
    fn node_hash_work_is_admitted_before_manifest_bytes() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let authority = AuthorityDomain::default();
        let path = AssetPath::new("models/main.obj").unwrap();
        let mut graph = ResourceGraph::new(limits);
        graph
            .bind_primary(host, &authority, &path, b"v 0 0 0")
            .unwrap();
        let node = graph.nodes.first().unwrap();
        let node_hash_work = u64::try_from(
            node_class_len(
                &node.public_identity,
                node.byte_length,
                node.content_digest,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        let graph_storage = host
            .dimension_state(ImportDimension::ScratchBytes)
            .unwrap()
            .live_bytes();
        host.tighten_dimension_policy(
            ImportDimension::InputManifestWork,
            DimensionPolicy::counter(3, 3, 0),
        )
        .unwrap();

        assert!(graph.seal(host).is_err());
        let work = host
            .dimension_state(ImportDimension::InputManifestWork)
            .unwrap();
        assert_eq!(work.attempted(), 3 + node_hash_work);
        assert_eq!(work.consumed(), 3);
        assert_eq!(
            host.dimension_state(ImportDimension::InputManifestBytes)
                .unwrap()
                .attempted(),
            0
        );
        assert_eq!(
            host.dimension_state(ImportDimension::ScratchBytes)
                .unwrap()
                .live_bytes(),
            graph_storage
        );
        assert!(graph.sealed_manifest.is_none());
    }

    #[test]
    fn canonical_manifest_storage_is_admitted_before_encoding() {
        let limits = ResourceLimits::default();
        let owner = store_owner(&limits);
        let host = owner.host();
        let authority = AuthorityDomain::default();
        let path = AssetPath::new("models/main.obj").unwrap();
        let mut graph = ResourceGraph::new(limits);
        graph
            .bind_primary(host, &authority, &path, b"v 0 0 0")
            .unwrap();
        host.tighten_dimension_policy(
            ImportDimension::InputManifestBytes,
            DimensionPolicy::reservable_bytes(5_327, 5_327, 0, 5_326),
        )
        .unwrap();

        assert!(graph.seal(host).is_err());
        let manifest_bytes = host
            .dimension_state(ImportDimension::InputManifestBytes)
            .unwrap();
        assert_eq!(manifest_bytes.attempted(), 5_327);
        assert_eq!(manifest_bytes.consumed(), 0);
        assert_eq!(manifest_bytes.live_bytes(), 0);
        assert!(graph.sealed_manifest.is_none());
    }
}
