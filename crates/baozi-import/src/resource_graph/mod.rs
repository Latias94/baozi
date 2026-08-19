//! Session-owned external-resource graph and successful input manifest.
//!
//! This module owns both the authority-qualified physical dependency cache and the logical graph.
//! `ParseScope` receives only opaque origin handles and dependency results; format crates cannot
//! inspect graph IDs, physical cache keys, or manifest construction state.

mod manifest;

pub use manifest::{
    InputReplayStatus, ManifestDigest, ManifestNodeClass, ManifestObservedOnlyReason,
    ManifestReference, ManifestRelation, ManifestRelationOutcome, ManifestResourceIdentity,
    ManifestResourceIdentityStability, ManifestSessionDependentReason, ManifestSessionOpaqueReason,
    ManifestUnavailableClass, ManifestUnavailableCode, ManifestUnavailableReplay,
    ResourceManifestCounts, SuccessfulInputManifest,
};

use crate::dependency::{
    DependencyCache, DependencyCacheCapacityError, DependencyCacheEntry, DependencyCacheStorage,
    DependencyIdentity, DependencyRequirement, DependencyRoleId, SharedDependencySnapshot,
};
use crate::error::ScopeAbortCause;
use crate::ledger::{ImportDimension, ImportLedgerPhase};
use crate::limits::ResourceLimits;
use crate::store::{ImportStoreHost, ImportStoreReservation};
use baozi_io::{AssetPath, AuthorityDomain};
use baozi_operation::CapacityPlanError;
use std::sync::Arc;

use manifest::{ManifestEncodingError, encode_node_class, encode_relation_key};

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
    outgoing: Vec<GraphRelation>,
    depth: u64,
}

#[derive(Debug)]
struct GraphRelation {
    role: DependencyRoleId,
    requirement: DependencyRequirement,
    reference: ManifestReference,
    outcome: GraphRelationOutcome,
}

#[derive(Debug)]
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
    },
    Failed {
        cache_index: usize,
    },
    Resolving,
    FailureConsumed,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResourceUnavailableDisposition {
    class: ManifestUnavailableClass,
    code: ManifestUnavailableCode,
    replay: ManifestUnavailableReplay,
}

/// One admitted logical request that must reach exactly one terminal relation.
pub(crate) struct PendingResourceRelation {
    source: usize,
    role: DependencyRoleId,
    requirement: DependencyRequirement,
    raw_reference_digest: ManifestDigest,
    target: Option<TargetAdmission>,
}

/// One session-owned graph. `DependencyCache` remains the sole physical acquisition cache.
pub(crate) struct ResourceGraph {
    cache: DependencyCache,
    limits: ResourceLimits,
    nodes: Vec<Option<GraphNode>>,
    cache_to_node: Vec<Option<usize>>,
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
            nodes: Vec::new(),
            cache_to_node: Vec::new(),
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
                },
                DependencyCacheEntry::Failed(_) => ResourceAcquisitionState::Failed { cache_index },
                DependencyCacheEntry::Resolving => ResourceAcquisitionState::Resolving,
                DependencyCacheEntry::FailureConsumed => ResourceAcquisitionState::FailureConsumed,
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
    ) -> Result<usize, ResourceGraphError> {
        self.cache
            .store_available(identity, bytes)
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
    ) -> Result<usize, ScopeAbortCause> {
        self.cache.store_failed(identity, cause)
    }

    pub(crate) fn cached_failure(
        &self,
        cache_index: usize,
    ) -> Result<&ScopeAbortCause, ResourceGraphError> {
        self.cache
            .failed(cache_index)
            .ok_or(ResourceGraphError::DanglingNode)
    }

    pub(crate) fn take_cached_failure(
        &mut self,
        cache_index: usize,
    ) -> Result<ScopeAbortCause, ResourceGraphError> {
        self.cache
            .take_failed(cache_index)
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
            outgoing: Vec::new(),
            depth: 0,
        };
        self.nodes
            .try_reserve_exact(1)
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-graph-nodes",
            })?;
        self.nodes.push(Some(node));
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
            .filter(|index| self.nodes.get(*index).and_then(Option::as_ref).is_some())
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
        let source_node = self
            .nodes
            .get_mut(source)
            .and_then(Option::as_mut)
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
        source_node
            .outgoing
            .try_reserve(1)
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-graph-relations",
            })?;
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
            .and_then(Option::as_ref)
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
            .try_reserve(1)
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-graph-nodes",
            })?;
        pending.target = Some(TargetAdmission::New { depth });
        Ok(())
    }

    pub(crate) fn complete_available(
        &mut self,
        host: ImportStoreHost<'_>,
        mut pending: PendingResourceRelation,
        cache_index: usize,
        identity: &DependencyIdentity,
        bytes: &[u8],
        normalized: &AssetPath,
    ) -> Result<(), ResourceGraphError> {
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
                    .and_then(Option::as_ref)
                    .ok_or(ResourceGraphError::DanglingNode)?;
                if &node.exact_identity != identity
                    || node.content_digest != ManifestDigest::of(bytes)
                {
                    return Err(ResourceGraphError::DigestCollision);
                }
                (index, depth_updates)
            }
            TargetAdmission::New { depth } => {
                let exact_identity =
                    identity
                        .try_clone()
                        .map_err(|_| ResourceGraphError::Allocation {
                            resource: "resource-graph-node-identity",
                        })?;
                let public_identity = manifest_identity(
                    identity.path(),
                    ManifestSessionOpaqueReason::UnregisteredResolverIdentity,
                    "resource-graph-node-public-identity",
                )?;
                let index = self.nodes.len();
                self.nodes.push(Some(GraphNode {
                    exact_identity,
                    public_identity,
                    byte_length: u64::try_from(bytes.len())
                        .map_err(|_| ResourceGraphError::Overflow)?,
                    content_digest: ManifestDigest::of(bytes),
                    outgoing: Vec::new(),
                    depth,
                }));
                charge_consumed(host, ImportDimension::ResourceGraphNodes, 1)?;
                (index, Vec::new())
            }
        };
        self.bind_cache_node(cache_index, target)?;
        let reference = ManifestReference::Normalized(manifest_identity(
            normalized,
            ManifestSessionOpaqueReason::UnregisteredResolverIdentity,
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
        } = disposition;
        let reference = match normalized {
            Some(path) => ManifestReference::Normalized(manifest_identity(
                path,
                ManifestSessionOpaqueReason::UnregisteredResolverIdentity,
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
                manifest_identity(
                    identity.path(),
                    ManifestSessionOpaqueReason::UnregisteredResolverIdentity,
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
        let primary = self.primary.ok_or(ResourceGraphError::MissingPrimary)?;
        let postorder = self.postorder(host, primary)?;
        if postorder.len() != self.nodes.iter().filter(|node| node.is_some()).count() {
            return Err(ResourceGraphError::DanglingNode);
        }
        let mut digests = vec![None; self.nodes.len()];
        let mut physical = Vec::new();
        physical.try_reserve_exact(postorder.len()).map_err(|_| {
            ResourceGraphError::Allocation {
                resource: "resource-manifest-node-classes",
            }
        })?;

        for index in postorder {
            charge_manifest_work(host, 1)?;
            let node = self
                .nodes
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(ResourceGraphError::DanglingNode)?;
            let mut keyed = Vec::new();
            keyed.try_reserve_exact(node.outgoing.len()).map_err(|_| {
                ResourceGraphError::Allocation {
                    resource: "resource-manifest-relation-order",
                }
            })?;
            for relation in &node.outgoing {
                charge_manifest_work(host, 1)?;
                let relation = project_relation(relation, &digests)?;
                let key_bytes = encode_relation_key(
                    relation.role(),
                    relation.requirement(),
                    relation.reference(),
                    relation.outcome(),
                )?;
                keyed.push((ManifestDigest::of(&key_bytes), key_bytes, relation));
            }
            keyed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
            let mut relations: Vec<ManifestRelation> = Vec::new();
            relations.try_reserve_exact(keyed.len()).map_err(|_| {
                ResourceGraphError::Allocation {
                    resource: "resource-manifest-relations",
                }
            })?;
            for (key_digest, key_bytes, relation) in keyed {
                if let Some(previous) = relations.last_mut() {
                    let previous_bytes = encode_relation_key(
                        previous.role(),
                        previous.requirement(),
                        previous.reference(),
                        previous.outcome(),
                    )?;
                    let previous_digest = ManifestDigest::of(&previous_bytes);
                    if previous_digest == key_digest {
                        if previous_bytes != key_bytes {
                            return Err(ResourceGraphError::DigestCollision);
                        }
                        let multiplicity = previous
                            .multiplicity()
                            .checked_add(1)
                            .ok_or(ResourceGraphError::Overflow)?;
                        *previous = ManifestRelation::new(
                            previous.role().clone(),
                            previous.requirement(),
                            previous.reference().clone(),
                            previous.outcome().clone(),
                            multiplicity,
                        );
                        continue;
                    }
                }
                relations.push(relation);
            }
            let node_bytes = encode_node_class(
                &node.public_identity,
                node.byte_length,
                node.content_digest,
                &relations,
            )?;
            let digest = ManifestDigest::of(&node_bytes);
            digests[index] = Some(digest);
            physical.push((digest, node_bytes, index, relations));
        }

        physical.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        let mut classes: Vec<ManifestNodeClass> = Vec::new();
        classes
            .try_reserve_exact(physical.len())
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-manifest-node-classes",
            })?;
        for (digest, bytes, index, relations) in physical {
            if let Some(previous) = classes.last()
                && previous.digest() == digest
            {
                let previous_bytes = encode_node_class(
                    previous.identity(),
                    previous.byte_length(),
                    previous.content_digest(),
                    previous.outgoing_relations(),
                )?;
                if previous_bytes != bytes {
                    return Err(ResourceGraphError::DigestCollision);
                }
                continue;
            }
            let node = self
                .nodes
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(ResourceGraphError::DanglingNode)?;
            classes.push(ManifestNodeClass::new(
                digest,
                node.public_identity.clone(),
                node.byte_length,
                node.content_digest,
                relations,
            ));
        }
        let primary_digest = digests
            .get(primary)
            .and_then(|digest| *digest)
            .ok_or(ResourceGraphError::MissingPrimary)?;
        let counts = manifest_counts(&classes, self.maximum_depth())?;
        let replay_status = replay_status(&classes);
        let provisional = SuccessfulInputManifest::new(
            ManifestDigest::zero(),
            primary_digest,
            classes,
            counts,
            replay_status,
        );
        let canonical = provisional.canonical_bytes()?;
        let byte_count =
            u64::try_from(canonical.len()).map_err(|_| ResourceGraphError::Overflow)?;
        if byte_count > self.limits.max_input_manifest_bytes {
            return Err(ResourceGraphError::Limit {
                resource: "max_input_manifest_bytes",
                limit: self.limits.max_input_manifest_bytes,
            });
        }
        charge_attempted(host, ImportDimension::InputManifestBytes, byte_count)?;
        charge_consumed(host, ImportDimension::InputManifestBytes, byte_count)?;
        let digest = ManifestDigest::of(&canonical);
        let manifest = SuccessfulInputManifest::new(
            digest,
            primary_digest,
            provisional.node_classes().to_vec(),
            counts,
            provisional.replay_status().clone(),
        );
        self.sealed_manifest = Some(manifest);
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
        let node = self
            .nodes
            .get_mut(pending.source)
            .and_then(Option::as_mut)
            .ok_or(ResourceGraphError::DanglingNode)?;
        if node.outgoing.len() == node.outgoing.capacity() {
            return Err(ResourceGraphError::Allocation {
                resource: "resource-graph-relation-slot",
            });
        }
        node.outgoing.push(GraphRelation {
            role: pending.role,
            requirement: pending.requirement,
            reference,
            outcome,
        });
        self.pending_relations = self
            .pending_relations
            .checked_sub(1)
            .ok_or(ResourceGraphError::IncompleteRelation)?;
        self.relation_count = self
            .relation_count
            .checked_add(1)
            .ok_or(ResourceGraphError::Overflow)?;
        charge_consumed(host, ImportDimension::ResourceGraphRelations, 1)
    }

    fn bind_cache_node(
        &mut self,
        cache_index: usize,
        node_index: usize,
    ) -> Result<(), ResourceGraphError> {
        if cache_index >= self.cache_to_node.len() {
            self.cache_to_node
                .try_reserve(cache_index + 1 - self.cache_to_node.len())
                .map_err(|_| ResourceGraphError::Allocation {
                    resource: "resource-graph-cache-index",
                })?;
            self.cache_to_node.resize(cache_index + 1, None);
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
        self.nodes.iter().enumerate().find_map(|(index, node)| {
            node.as_ref()
                .filter(|node| &node.exact_identity == identity)
                .map(|_| index)
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
            let node = self
                .nodes
                .get(index)
                .and_then(Option::as_ref)
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
            for relation in node.outgoing.iter().rev() {
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
                .and_then(Option::as_ref)
                .ok_or(ResourceGraphError::DanglingNode)?;
            if depth > node.depth {
                updates.push(DepthUpdate { index, depth });
            }
            let child_depth = depth.checked_add(1).ok_or(ResourceGraphError::Overflow)?;
            for relation in &node.outgoing {
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
                .and_then(Option::as_mut)
                .ok_or(ResourceGraphError::DanglingNode)?;
            node.depth = node.depth.max(update.depth);
        }
        Ok(())
    }

    fn postorder(
        &self,
        host: ImportStoreHost<'_>,
        primary: usize,
    ) -> Result<Vec<usize>, ResourceGraphError> {
        let mut states = Vec::new();
        states
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-manifest-visit-state",
            })?;
        states.resize(self.nodes.len(), 0_u8);
        let mut stack = vec![(primary, false)];
        let mut order = Vec::new();
        order
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| ResourceGraphError::Allocation {
                resource: "resource-manifest-postorder",
            })?;
        while let Some((index, expanded)) = stack.pop() {
            charge_manifest_work(host, 1)?;
            let node = self
                .nodes
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(ResourceGraphError::DanglingNode)?;
            if expanded {
                states[index] = 2;
                order.push(index);
                continue;
            }
            match states[index] {
                2 => continue,
                1 => return Err(ResourceGraphError::Cycle),
                _ => {}
            }
            states[index] = 1;
            stack.push((index, true));
            for relation in node.outgoing.iter().rev() {
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
        Ok(order)
    }

    fn require_node_capacity(&self, additional: usize) -> Result<(), ResourceGraphError> {
        let count = self
            .nodes
            .iter()
            .filter(|node| node.is_some())
            .count()
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

    fn maximum_depth(&self) -> u64 {
        self.nodes
            .iter()
            .filter_map(Option::as_ref)
            .map(|node| node.depth)
            .max()
            .unwrap_or(0)
    }
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

fn manifest_counts(
    classes: &[ManifestNodeClass],
    maximum_depth: u64,
) -> Result<ResourceManifestCounts, ResourceGraphError> {
    let mut counts = ResourceManifestCounts {
        node_class_count: u64::try_from(classes.len()).map_err(|_| ResourceGraphError::Overflow)?,
        maximum_depth,
        ..ResourceManifestCounts::default()
    };
    for class in classes {
        for relation in class.outgoing_relations() {
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
                ManifestRelationOutcome::Unavailable { .. } => {
                    counts.unavailable_relation_record_count = counts
                        .unavailable_relation_record_count
                        .checked_add(1)
                        .ok_or(ResourceGraphError::Overflow)?;
                    counts.unavailable_relation_occurrence_count = counts
                        .unavailable_relation_occurrence_count
                        .checked_add(relation.multiplicity())
                        .ok_or(ResourceGraphError::Overflow)?;
                }
            }
        }
    }
    Ok(counts)
}

fn replay_status(classes: &[ManifestNodeClass]) -> InputReplayStatus {
    let mut reasons = Vec::new();
    if classes.iter().any(|class| {
        matches!(
            class.identity().stability(),
            ManifestResourceIdentityStability::SessionOpaque(_)
        )
    }) {
        reasons.push(ManifestObservedOnlyReason::UnstableResourceIdentity);
    }
    for class in classes {
        for relation in class.outgoing_relations() {
            if let ManifestRelationOutcome::Unavailable { replay, .. } = relation.outcome() {
                reasons.push(match replay {
                    ManifestUnavailableReplay::UnprovenNegativeInput => {
                        ManifestObservedOnlyReason::UnprovenNegativeInput
                    }
                    ManifestUnavailableReplay::SessionDependent(_) => {
                        ManifestObservedOnlyReason::SessionDependentUnavailable
                    }
                });
            }
        }
    }
    reasons.sort();
    reasons.dedup();
    if reasons.is_empty() {
        InputReplayStatus::Replayable
    } else {
        InputReplayStatus::ObservedOnly(reasons)
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
    }
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
        PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
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
            .complete_available(host, pending, cache_index, identity, bytes, identity.path())
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
            .complete_available(host, pending, 0, &identity_a, b"a", identity_a.path())
            .unwrap();
        graph.seal(host).unwrap();

        assert_eq!(graph.take_manifest().unwrap().counts().maximum_depth(), 3);
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
        assert_eq!(manifest.schema_version(), 1);
        assert_eq!(manifest.counts().node_class_count(), 1);
        assert_eq!(manifest.counts().quotient_relation_record_count(), 0);
        assert!(matches!(
            manifest.replay_status(),
            InputReplayStatus::ObservedOnly(reasons)
                if reasons == &[ManifestObservedOnlyReason::UnstableResourceIdentity]
        ));
    }
}
