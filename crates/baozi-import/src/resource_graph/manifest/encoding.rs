use super::{
    InputReplayStatus, MANIFEST_DOMAIN, MANIFEST_SCHEMA_VERSION, ManifestDigest, ManifestNodeClass,
    ManifestObservedOnlyReason, ManifestReference, ManifestRelation, ManifestRelationOutcome,
    ManifestResourceIdentity, ManifestResourceIdentityStability, ManifestSessionDependentReason,
    ManifestSessionOpaqueReason, ManifestUnavailableReplay, ResourceManifestCounts,
};
use crate::dependency::{DependencyRequirement, DependencyRoleId};
use crate::metered_storage::AccountedVec;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::resource_graph) enum ManifestEncodingError {
    LengthOverflow,
    InvalidCanonicalId,
    Allocation,
}

pub(in crate::resource_graph) fn node_class_len(
    identity: &ManifestResourceIdentity,
    byte_length: u64,
    content_digest: ManifestDigest,
    outgoing_relations: &[ManifestRelation],
) -> Result<usize, ManifestEncodingError> {
    encoded_len(|output| {
        write_node_class(
            output,
            identity,
            byte_length,
            content_digest,
            outgoing_relations,
        )
    })
}

pub(in crate::resource_graph) fn encode_node_class_into_accounted(
    identity: &ManifestResourceIdentity,
    byte_length: u64,
    content_digest: ManifestDigest,
    outgoing_relations: &[ManifestRelation],
    expected: usize,
    bytes: &mut AccountedVec<u8>,
) -> Result<(), ManifestEncodingError> {
    let mut output = AccountedVecOutput::new(bytes, expected)?;
    write_node_class(
        &mut output,
        identity,
        byte_length,
        content_digest,
        outgoing_relations,
    )?;
    output.finish()
}

pub(in crate::resource_graph) fn relation_key_len(
    role: &DependencyRoleId,
    requirement: DependencyRequirement,
    reference: &ManifestReference,
    outcome: &ManifestRelationOutcome,
) -> Result<usize, ManifestEncodingError> {
    encoded_len(|output| write_relation_key(output, role, requirement, reference, outcome))
}

pub(in crate::resource_graph) fn encode_relation_key_into_accounted(
    role: &DependencyRoleId,
    requirement: DependencyRequirement,
    reference: &ManifestReference,
    outcome: &ManifestRelationOutcome,
    expected: usize,
    bytes: &mut AccountedVec<u8>,
) -> Result<(), ManifestEncodingError> {
    let mut output = AccountedVecOutput::new(bytes, expected)?;
    write_relation_key(&mut output, role, requirement, reference, outcome)?;
    output.finish()
}

pub(super) fn manifest_len(
    primary: ManifestDigest,
    node_classes: &[ManifestNodeClass],
    counts: ResourceManifestCounts,
    replay_status: &InputReplayStatus,
) -> Result<usize, ManifestEncodingError> {
    encoded_len(|output| write_manifest(output, primary, node_classes, counts, replay_status))
}

#[cfg(test)]
pub(super) fn encode_manifest(
    primary: ManifestDigest,
    node_classes: &[ManifestNodeClass],
    counts: ResourceManifestCounts,
    replay_status: &InputReplayStatus,
) -> Result<Vec<u8>, ManifestEncodingError> {
    let length = manifest_len(primary, node_classes, counts, replay_status)?;
    encode_vec(length, |output| {
        write_manifest(output, primary, node_classes, counts, replay_status)
    })
}

pub(super) fn encode_manifest_into_accounted(
    primary: ManifestDigest,
    node_classes: &[ManifestNodeClass],
    counts: ResourceManifestCounts,
    replay_status: &InputReplayStatus,
    expected: usize,
    bytes: &mut AccountedVec<u8>,
) -> Result<(), ManifestEncodingError> {
    let mut output = AccountedVecOutput::new(bytes, expected)?;
    write_manifest(&mut output, primary, node_classes, counts, replay_status)?;
    output.finish()
}

fn write_manifest<O: ManifestOutput>(
    output: &mut O,
    primary: ManifestDigest,
    node_classes: &[ManifestNodeClass],
    counts: ResourceManifestCounts,
    replay_status: &InputReplayStatus,
) -> Result<(), ManifestEncodingError> {
    write_record_value(output, "input-manifest", 7, |output| {
        write_field(output, "domain", |output| {
            write_utf8_value(output, MANIFEST_DOMAIN.as_bytes())
        })?;
        write_field(output, "schema-version", |output| {
            write_u64_value(output, MANIFEST_SCHEMA_VERSION)
        })?;
        write_field(output, "digest-algorithm", |output| {
            write_enum_value(output, "sha256")
        })?;
        write_field(output, "primary", |output| {
            write_record_value(output, "primary", 1, |output| {
                write_field(output, "node-class-digest", |output| {
                    write_digest_value(output, primary)
                })
            })
        })?;
        write_field(output, "node-classes", |output| {
            write_sequence_value(output, node_classes.len(), |output| {
                for class in node_classes {
                    write_item(output, |output| {
                        write_record_value(output, "node-entry", 2, |output| {
                            write_field(output, "node-class-digest", |output| {
                                write_digest_value(output, class.digest)
                            })?;
                            write_field(output, "node-class", |output| {
                                write_node_class(
                                    output,
                                    &class.identity,
                                    class.byte_length,
                                    class.content_digest,
                                    &class.outgoing_relations,
                                )
                            })
                        })
                    })?;
                }
                Ok(())
            })
        })?;
        write_field(output, "counts", |output| write_counts(output, counts))?;
        write_field(output, "input-replay-status", |output| {
            write_replay_status(output, replay_status)
        })
    })
}

fn write_node_class<O: ManifestOutput>(
    output: &mut O,
    identity: &ManifestResourceIdentity,
    byte_length: u64,
    content_digest: ManifestDigest,
    outgoing_relations: &[ManifestRelation],
) -> Result<(), ManifestEncodingError> {
    write_record_value(output, "node-class", 4, |output| {
        write_field(output, "resource-identity", |output| {
            write_resource_identity(output, identity)
        })?;
        write_field(output, "byte-length", |output| {
            write_u64_value(output, byte_length)
        })?;
        write_field(output, "content-digest", |output| {
            write_digest_value(output, content_digest)
        })?;
        write_field(output, "outgoing-relations", |output| {
            write_sequence_value(output, outgoing_relations.len(), |output| {
                for relation in outgoing_relations {
                    write_item(output, |output| write_relation(output, relation))?;
                }
                Ok(())
            })
        })
    })
}

fn write_relation_key<O: ManifestOutput>(
    output: &mut O,
    role: &DependencyRoleId,
    requirement: DependencyRequirement,
    reference: &ManifestReference,
    outcome: &ManifestRelationOutcome,
) -> Result<(), ManifestEncodingError> {
    write_record_value(output, "relation-key", 4, |output| {
        write_field(output, "role", |output| {
            write_enum_value(output, role.as_str())
        })?;
        write_field(output, "requirement", |output| {
            write_enum_value(output, requirement_id(requirement))
        })?;
        write_field(output, "reference", |output| {
            write_reference(output, reference)
        })?;
        write_field(output, "outcome", |output| write_outcome(output, outcome))
    })
}

fn write_relation<O: ManifestOutput>(
    output: &mut O,
    relation: &ManifestRelation,
) -> Result<(), ManifestEncodingError> {
    write_record_value(output, "relation", 2, |output| {
        write_field(output, "key", |output| {
            write_relation_key(
                output,
                &relation.role,
                relation.requirement,
                &relation.reference,
                &relation.outcome,
            )
        })?;
        write_field(output, "multiplicity", |output| {
            write_u64_value(output, relation.multiplicity)
        })
    })
}

fn write_resource_identity<O: ManifestOutput>(
    output: &mut O,
    identity: &ManifestResourceIdentity,
) -> Result<(), ManifestEncodingError> {
    write_record_value(output, "resource-identity-v1", 2, |output| {
        write_field(output, "value", |output| {
            write_utf8_value(output, identity.value.as_str().as_bytes())
        })?;
        write_field(output, "stability", |output| match &identity.stability {
            ManifestResourceIdentityStability::Stable {
                owner,
                schema,
                version,
            } => write_union_value(output, "stable-resource-identity", 3, |output| {
                write_field(output, "owner", |output| write_enum_value(output, owner))?;
                write_field(output, "schema-id", |output| {
                    write_enum_value(output, schema)
                })?;
                write_field(output, "schema-version", |output| {
                    write_u64_value(output, *version)
                })
            }),
            ManifestResourceIdentityStability::SessionOpaque(reason) => {
                write_union_value(output, "session-opaque-resource-identity", 1, |output| {
                    write_field(output, "reason", |output| {
                        write_enum_value(output, session_opaque_reason_id(*reason))
                    })
                })
            }
        })
    })
}

fn write_reference<O: ManifestOutput>(
    output: &mut O,
    reference: &ManifestReference,
) -> Result<(), ManifestEncodingError> {
    match reference {
        ManifestReference::Normalized(identity) => {
            write_union_value(output, "normalized-reference", 1, |output| {
                write_field(output, "resource-identity", |output| {
                    write_resource_identity(output, identity)
                })
            })
        }
        ManifestReference::Rejected {
            class,
            code,
            reference_digest,
        } => write_union_value(output, "rejected-reference", 3, |output| {
            write_field(output, "class", |output| {
                write_enum_value(output, class.canonical_id())
            })?;
            write_field(output, "code", |output| {
                write_enum_value(output, code.as_str())
            })?;
            write_field(output, "reference-digest", |output| {
                write_digest_value(output, *reference_digest)
            })
        }),
    }
}

fn write_outcome<O: ManifestOutput>(
    output: &mut O,
    outcome: &ManifestRelationOutcome,
) -> Result<(), ManifestEncodingError> {
    match outcome {
        ManifestRelationOutcome::Available {
            target_node_class_digest,
        } => write_union_value(output, "available", 1, |output| {
            write_field(output, "target-node-class-digest", |output| {
                write_digest_value(output, *target_node_class_digest)
            })
        }),
        ManifestRelationOutcome::Unavailable {
            class,
            code,
            identity_evidence,
            replay,
        } => write_union_value(output, "unavailable", 4, |output| {
            write_field(output, "class", |output| {
                write_enum_value(output, class.canonical_id())
            })?;
            write_field(output, "code", |output| {
                write_enum_value(output, code.as_str())
            })?;
            write_field(output, "identity-evidence", |output| {
                if let Some(identity) = identity_evidence {
                    write_optional_some(output, |output| write_resource_identity(output, identity))
                } else {
                    write_optional_none(output)
                }
            })?;
            write_field(output, "replay-classification", |output| {
                write_unavailable_replay(output, *replay)
            })
        }),
    }
}

fn write_unavailable_replay<O: ManifestOutput>(
    output: &mut O,
    replay: ManifestUnavailableReplay,
) -> Result<(), ManifestEncodingError> {
    match replay {
        ManifestUnavailableReplay::UnprovenNegativeInput => {
            write_union_value(output, "unproven-negative-input", 1, |output| {
                write_field(output, "reason", |output| {
                    write_enum_value(output, "missing-negative-proof")
                })
            })
        }
        ManifestUnavailableReplay::SessionDependent(reason) => {
            write_union_value(output, "session-dependent-unavailable", 1, |output| {
                write_field(output, "reason", |output| {
                    write_enum_value(output, session_dependent_reason_id(reason))
                })
            })
        }
    }
}

fn write_counts<O: ManifestOutput>(
    output: &mut O,
    counts: ResourceManifestCounts,
) -> Result<(), ManifestEncodingError> {
    write_record_value(output, "counts", 8, |output| {
        write_field(output, "node-class-count", |output| {
            write_u64_value(output, counts.node_class_count)
        })?;
        write_field(output, "quotient-relation-record-count", |output| {
            write_u64_value(output, counts.quotient_relation_record_count)
        })?;
        write_field(output, "relation-occurrence-count", |output| {
            write_u64_value(output, counts.relation_occurrence_count)
        })?;
        write_field(output, "available-relation-record-count", |output| {
            write_u64_value(output, counts.available_relation_record_count)
        })?;
        write_field(output, "available-relation-occurrence-count", |output| {
            write_u64_value(output, counts.available_relation_occurrence_count)
        })?;
        write_field(output, "unavailable-relation-record-count", |output| {
            write_u64_value(output, counts.unavailable_relation_record_count)
        })?;
        write_field(output, "unavailable-relation-occurrence-count", |output| {
            write_u64_value(output, counts.unavailable_relation_occurrence_count)
        })?;
        write_field(output, "maximum-depth", |output| {
            write_u64_value(output, counts.maximum_depth)
        })
    })
}

fn write_replay_status<O: ManifestOutput>(
    output: &mut O,
    replay_status: &InputReplayStatus,
) -> Result<(), ManifestEncodingError> {
    let (status, reasons) = match replay_status {
        InputReplayStatus::Replayable => ("replayable", &[][..]),
        InputReplayStatus::ObservedOnly(reasons) => ("observed-only", reasons.as_slice()),
    };
    let ordered = ordered_reasons(reasons)?;
    write_record_value(output, "input-replay-status", 2, |output| {
        write_field(output, "status", |output| write_enum_value(output, status))?;
        write_field(output, "reasons", |output| {
            write_set_value(output, ordered.len(), |output| {
                for reason in ordered.iter() {
                    write_item(output, |output| {
                        write_enum_value(output, reason.canonical_id())
                    })?;
                }
                Ok(())
            })
        })
    })
}

struct OrderedReasons {
    values: [Option<ManifestObservedOnlyReason>; 3],
    len: usize,
}

impl OrderedReasons {
    fn len(&self) -> usize {
        self.len
    }

    fn iter(&self) -> impl Iterator<Item = ManifestObservedOnlyReason> + '_ {
        self.values[..self.len]
            .iter()
            .map(|value| value.expect("the ordered reason prefix is fully initialized"))
    }
}

fn ordered_reasons(
    reasons: &[ManifestObservedOnlyReason],
) -> Result<OrderedReasons, ManifestEncodingError> {
    if reasons.len() > 3 {
        return Err(ManifestEncodingError::InvalidCanonicalId);
    }
    let mut ordered = OrderedReasons {
        values: [None; 3],
        len: 0,
    };
    for reason in reasons.iter().copied() {
        let mut position = ordered.len;
        while position > 0 {
            let previous = ordered.values[position - 1]
                .expect("the ordered reason prefix is fully initialized");
            match compare_enum_ids(reason.canonical_id(), previous.canonical_id()) {
                Ordering::Less => {
                    ordered.values[position] = Some(previous);
                    position -= 1;
                }
                Ordering::Equal => return Err(ManifestEncodingError::InvalidCanonicalId),
                Ordering::Greater => break,
            }
        }
        ordered.values[position] = Some(reason);
        ordered.len += 1;
    }
    Ok(ordered)
}

fn compare_enum_ids(left: &str, right: &str) -> Ordering {
    left.len()
        .cmp(&right.len())
        .then_with(|| left.as_bytes().cmp(right.as_bytes()))
}

fn requirement_id(requirement: DependencyRequirement) -> &'static str {
    match requirement {
        DependencyRequirement::Required => "required",
        DependencyRequirement::Optional => "optional",
    }
}

fn session_opaque_reason_id(reason: ManifestSessionOpaqueReason) -> &'static str {
    match reason {
        ManifestSessionOpaqueReason::GeneratedReportIdentity => "generated-report-identity",
        ManifestSessionOpaqueReason::UnregisteredCallerIdentity => "unregistered-caller-identity",
        ManifestSessionOpaqueReason::UnregisteredResolverIdentity => {
            "unregistered-resolver-identity"
        }
    }
}

fn session_dependent_reason_id(reason: ManifestSessionDependentReason) -> &'static str {
    match reason {
        ManifestSessionDependentReason::AcquisitionBudget => "session-dependent-acquisition-budget",
        ManifestSessionDependentReason::MutablePolicy => "session-dependent-mutable-policy",
        ManifestSessionDependentReason::HostCallback => "session-dependent-host-callback",
        ManifestSessionDependentReason::ProcessState => "session-dependent-process-state",
        ManifestSessionDependentReason::UnregisteredSemanticBasis => "unregistered-semantic-basis",
    }
}

fn validate_canonical_id(value: &str) -> Result<(), ManifestEncodingError> {
    if value.is_empty()
        || value.len() > 96
        || value.split(['.', '_', ':', '-']).any(|part| {
            part.is_empty()
                || part
                    .bytes()
                    .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit()))
        })
    {
        return Err(ManifestEncodingError::InvalidCanonicalId);
    }
    Ok(())
}

trait ManifestOutput {
    fn position(&self) -> usize;

    fn write(&mut self, bytes: &[u8]) -> Result<(), ManifestEncodingError>;

    fn overwrite(&mut self, offset: usize, bytes: &[u8]) -> Result<(), ManifestEncodingError>;
}

struct LengthOutput {
    length: usize,
}

impl ManifestOutput for LengthOutput {
    fn position(&self) -> usize {
        self.length
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), ManifestEncodingError> {
        self.length = self
            .length
            .checked_add(bytes.len())
            .ok_or(ManifestEncodingError::LengthOverflow)?;
        Ok(())
    }

    fn overwrite(&mut self, offset: usize, bytes: &[u8]) -> Result<(), ManifestEncodingError> {
        let end = offset
            .checked_add(bytes.len())
            .ok_or(ManifestEncodingError::LengthOverflow)?;
        if end > self.length {
            return Err(ManifestEncodingError::LengthOverflow);
        }
        Ok(())
    }
}

#[cfg(test)]
struct VecOutput {
    bytes: Vec<u8>,
    expected: usize,
}

struct AccountedVecOutput<'output> {
    bytes: &'output mut AccountedVec<u8>,
    expected: usize,
}

impl<'output> AccountedVecOutput<'output> {
    fn new(
        bytes: &'output mut AccountedVec<u8>,
        expected: usize,
    ) -> Result<Self, ManifestEncodingError> {
        if !bytes.is_empty() || !bytes.has_slots(expected) {
            return Err(ManifestEncodingError::Allocation);
        }
        Ok(Self { bytes, expected })
    }

    fn finish(self) -> Result<(), ManifestEncodingError> {
        if self.bytes.len() != self.expected {
            return Err(ManifestEncodingError::LengthOverflow);
        }
        Ok(())
    }
}

#[cfg(test)]
impl VecOutput {
    fn new(expected: usize) -> Result<Self, ManifestEncodingError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(expected)
            .map_err(|_| ManifestEncodingError::Allocation)?;
        Ok(Self { bytes, expected })
    }

    fn finish(self) -> Result<Vec<u8>, ManifestEncodingError> {
        if self.bytes.len() != self.expected {
            return Err(ManifestEncodingError::LengthOverflow);
        }
        Ok(self.bytes)
    }
}

#[cfg(test)]
impl ManifestOutput for VecOutput {
    fn position(&self) -> usize {
        self.bytes.len()
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), ManifestEncodingError> {
        let required = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(ManifestEncodingError::LengthOverflow)?;
        if required > self.expected || required > self.bytes.capacity() {
            return Err(ManifestEncodingError::LengthOverflow);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn overwrite(&mut self, offset: usize, bytes: &[u8]) -> Result<(), ManifestEncodingError> {
        let end = offset
            .checked_add(bytes.len())
            .ok_or(ManifestEncodingError::LengthOverflow)?;
        let destination = self
            .bytes
            .get_mut(offset..end)
            .ok_or(ManifestEncodingError::LengthOverflow)?;
        destination.copy_from_slice(bytes);
        Ok(())
    }
}

impl ManifestOutput for AccountedVecOutput<'_> {
    fn position(&self) -> usize {
        self.bytes.len()
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), ManifestEncodingError> {
        let required = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(ManifestEncodingError::LengthOverflow)?;
        if required > self.expected {
            return Err(ManifestEncodingError::LengthOverflow);
        }
        self.bytes
            .try_extend_from_slice_reserved(bytes)
            .map_err(|()| ManifestEncodingError::Allocation)
    }

    fn overwrite(&mut self, offset: usize, bytes: &[u8]) -> Result<(), ManifestEncodingError> {
        let end = offset
            .checked_add(bytes.len())
            .ok_or(ManifestEncodingError::LengthOverflow)?;
        let destination = self
            .bytes
            .get_mut(offset..end)
            .ok_or(ManifestEncodingError::LengthOverflow)?;
        destination.copy_from_slice(bytes);
        Ok(())
    }
}

fn encoded_len(
    encode: impl FnOnce(&mut LengthOutput) -> Result<(), ManifestEncodingError>,
) -> Result<usize, ManifestEncodingError> {
    let mut output = LengthOutput { length: 0 };
    encode(&mut output)?;
    Ok(output.length)
}

#[cfg(test)]
fn encode_vec(
    expected: usize,
    encode: impl FnOnce(&mut VecOutput) -> Result<(), ManifestEncodingError>,
) -> Result<Vec<u8>, ManifestEncodingError> {
    let mut output = VecOutput::new(expected)?;
    encode(&mut output)?;
    output.finish()
}

fn write_frame<O: ManifestOutput>(
    output: &mut O,
    label: &[u8],
    payload: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    output.write(&checked_len(label.len())?.to_be_bytes())?;
    output.write(label)?;
    let length_offset = output.position();
    output.write(&0_u64.to_be_bytes())?;
    let payload_start = output.position();
    payload(output)?;
    let payload_len = output
        .position()
        .checked_sub(payload_start)
        .ok_or(ManifestEncodingError::LengthOverflow)?;
    output.overwrite(length_offset, &checked_len(payload_len)?.to_be_bytes())
}

fn write_record<O: ManifestOutput>(
    output: &mut O,
    tag: &str,
    field_count: usize,
    fields: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_frame(output, b"record-tag", |output| output.write(tag.as_bytes()))?;
    output.write(&checked_len(field_count)?.to_be_bytes())?;
    fields(output)
}

fn write_value<O: ManifestOutput>(
    output: &mut O,
    kind: &str,
    payload: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_record(output, "identity-value", 2, |output| {
        write_frame(output, b"kind", |output| output.write(kind.as_bytes()))?;
        write_frame(output, b"payload", payload)
    })
}

fn write_field<O: ManifestOutput>(
    output: &mut O,
    label: &str,
    value: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_frame(output, label.as_bytes(), value)
}

fn write_record_value<O: ManifestOutput>(
    output: &mut O,
    tag: &str,
    field_count: usize,
    fields: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_value(output, "record", |output| {
        write_record(output, tag, field_count, fields)
    })
}

fn write_u64_value<O: ManifestOutput>(
    output: &mut O,
    value: u64,
) -> Result<(), ManifestEncodingError> {
    write_value(output, "u64", |output| output.write(&value.to_be_bytes()))
}

fn write_bytes_value<O: ManifestOutput>(
    output: &mut O,
    bytes: &[u8],
) -> Result<(), ManifestEncodingError> {
    write_value(output, "bytes", |output| output.write(bytes))
}

fn write_utf8_value<O: ManifestOutput>(
    output: &mut O,
    bytes: &[u8],
) -> Result<(), ManifestEncodingError> {
    write_value(output, "utf8", |output| output.write(bytes))
}

fn write_enum_value<O: ManifestOutput>(
    output: &mut O,
    value: &str,
) -> Result<(), ManifestEncodingError> {
    validate_canonical_id(value)?;
    write_value(output, "enum", |output| output.write(value.as_bytes()))
}

fn write_digest_value<O: ManifestOutput>(
    output: &mut O,
    digest: ManifestDigest,
) -> Result<(), ManifestEncodingError> {
    write_record_value(output, "digest", 2, |output| {
        write_field(output, "algorithm", |output| {
            write_enum_value(output, "sha256")
        })?;
        write_field(output, "value", |output| {
            write_bytes_value(output, &digest.0)
        })
    })
}

fn write_optional_none<O: ManifestOutput>(output: &mut O) -> Result<(), ManifestEncodingError> {
    write_value(output, "optional", |output| output.write(&[0]))
}

fn write_optional_some<O: ManifestOutput>(
    output: &mut O,
    value: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_value(output, "optional", |output| {
        output.write(&[1])?;
        write_frame(output, b"some", value)
    })
}

fn write_sequence_value<O: ManifestOutput>(
    output: &mut O,
    count: usize,
    values: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_value(output, "sequence", |output| {
        output.write(&checked_len(count)?.to_be_bytes())?;
        values(output)
    })
}

fn write_set_value<O: ManifestOutput>(
    output: &mut O,
    count: usize,
    values: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_value(output, "set", |output| {
        output.write(&checked_len(count)?.to_be_bytes())?;
        values(output)
    })
}

fn write_item<O: ManifestOutput>(
    output: &mut O,
    value: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_frame(output, b"item", value)
}

fn write_union_value<O: ManifestOutput>(
    output: &mut O,
    tag: &str,
    field_count: usize,
    fields: impl FnOnce(&mut O) -> Result<(), ManifestEncodingError>,
) -> Result<(), ManifestEncodingError> {
    write_value(output, "union", |output| {
        write_record(output, tag, field_count, fields)
    })
}

fn checked_len(value: usize) -> Result<u64, ManifestEncodingError> {
    u64::try_from(value).map_err(|_| ManifestEncodingError::LengthOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_deny_parameter_record_matches_the_adr_vector() {
        let length = encoded_len(|output| {
            write_record_value(output, "external-policy-deny-v1", 0, |_| Ok(()))
        })
        .unwrap();
        let encoded = encode_vec(length, |output| {
            write_record_value(output, "external-policy-deny-v1", 0, |_| Ok(()))
        })
        .unwrap();
        assert_eq!(encoded.len(), 154);
        assert_eq!(
            ManifestDigest::of(&encoded).as_bytes(),
            [
                0x32, 0x5d, 0x32, 0xa5, 0x71, 0xf7, 0x30, 0x88, 0x38, 0xf5, 0x88, 0x79, 0xc1, 0x7d,
                0xc5, 0xdb, 0xdb, 0x78, 0x4e, 0xb9, 0x1f, 0x23, 0x03, 0x11, 0x16, 0x02, 0xbe, 0x8b,
                0x18, 0xf5, 0x59, 0xc9,
            ]
        );
    }

    #[test]
    fn canonical_ids_reject_non_registry_grammar() {
        let error = encoded_len(|output| write_enum_value(output, "Debug Value"));
        assert_eq!(error, Err(ManifestEncodingError::InvalidCanonicalId));
    }

    #[test]
    fn set_order_matches_encoded_enum_order_without_heap_storage() {
        let reasons = [
            ManifestObservedOnlyReason::SessionDependentUnavailable,
            ManifestObservedOnlyReason::UnstableResourceIdentity,
            ManifestObservedOnlyReason::UnprovenNegativeInput,
        ];
        let ordered = ordered_reasons(&reasons).unwrap();
        assert_eq!(
            ordered.iter().collect::<Vec<_>>(),
            vec![
                ManifestObservedOnlyReason::UnprovenNegativeInput,
                ManifestObservedOnlyReason::UnstableResourceIdentity,
                ManifestObservedOnlyReason::SessionDependentUnavailable,
            ]
        );
    }
}
