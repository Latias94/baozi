use std::collections::BTreeSet;

use serde::Deserialize;

use super::capability::{BoundedPreview, FixedVector};
use super::{CanonicalSnapshotTotals, CoverageMappingSpecification, is_lowercase_sha256};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftSnapshotVectorInventorySpecification {
    entry_point: String,
    inventory_id: String,
    compatibility_state: String,
    domain: String,
    draft_schema_revision: u64,
    algorithm: String,
    vectors: Vec<DraftSnapshotVectorSpecification>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
enum DraftSnapshotVectorSource {
    ObjCapability,
    GltfCapability,
    ProfileClosure,
    SegmentPartition,
}

impl DraftSnapshotVectorSource {
    const ALL: [Self; 4] = [
        Self::ObjCapability,
        Self::GltfCapability,
        Self::ProfileClosure,
        Self::SegmentPartition,
    ];
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct DraftSnapshotVectorSpecification {
    vector_id: String,
    source: DraftSnapshotVectorSource,
    digest_hex: String,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct DraftSnapshotVectorReport {
    vector_id: String,
    digest_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SegmentPartitionInvarianceSpecification {
    entry_point: String,
    scenario_id: String,
    domain: String,
    draft_schema_revision: u64,
    algorithm: String,
    direct_profile_segment_count: u64,
    edited_profile_segment_count: u64,
    digest_hex: String,
    preview: BoundedPreview,
    transcript: FixedVector,
    semantic_totals: CanonicalSnapshotTotals,
    direct_work_units: u64,
    edited_work_units: u64,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftSnapshotVectorInventoryReport {
    pub(super) entry_point: String,
    pub(super) inventory_id: String,
    compatibility_state: String,
    domain: String,
    pub(super) draft_schema_revision: u64,
    algorithm: String,
    vectors: Vec<DraftSnapshotVectorReport>,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SegmentPartitionInvarianceReport {
    entry_point: String,
    pub(super) scenario_id: String,
    domain: String,
    draft_schema_revision: u64,
    algorithm: String,
    pub(super) direct: SegmentPartitionReceiptReport,
    pub(super) edited: SegmentPartitionReceiptReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SegmentPartitionReceiptReport {
    pub(super) segment_count: u64,
    digest_hex: String,
    preview: BoundedPreview,
    transcript: FixedVector,
    normalized_semantic_totals: CanonicalSnapshotTotals,
    pub(super) work_units: u64,
}

pub(super) fn validate_specification(
    inventory: &DraftSnapshotVectorInventorySpecification,
    partition: &SegmentPartitionInvarianceSpecification,
    obj_digest: &str,
    gltf_digest: &str,
    profile_digest: &str,
) -> Result<(), String> {
    validate_inventory_specification(inventory)?;
    validate_partition_specification(partition, inventory)?;
    validate_vectors(
        &inventory.vectors,
        obj_digest,
        gltf_digest,
        profile_digest,
        &partition.digest_hex,
        "declared",
    )
}

fn validate_partition_specification(
    partition: &SegmentPartitionInvarianceSpecification,
    inventory: &DraftSnapshotVectorInventorySpecification,
) -> Result<(), String> {
    if !is_bounded_identity(&partition.scenario_id)
        || partition.entry_point != inventory.entry_point
        || partition.domain != inventory.domain
        || partition.draft_schema_revision != inventory.draft_schema_revision
        || partition.algorithm != inventory.algorithm
        || partition.direct_profile_segment_count == 0
        || partition.edited_profile_segment_count <= partition.direct_profile_segment_count
        || !is_lowercase_sha256(&partition.digest_hex)
        || !is_bounded_identity(&partition.preview.schema_id)
        || partition.preview.bytes == 0
        || !is_lowercase_sha256(&partition.preview.sha256)
        || partition.preview.items_omitted_by_limit != 0
        || partition.preview.bytes_truncated
        || partition.preview.items_truncated
        || !is_bounded_identity(&partition.transcript.schema_id)
        || partition.transcript.observed_bytes == 0
        || !is_lowercase_sha256(&partition.transcript.sha256)
        || partition.semantic_totals.work_units != 0
        || partition.direct_work_units == 0
        || partition.edited_work_units <= partition.direct_work_units
    {
        return Err(
            "U4d segment-partition specification must freeze fewer-vs-more segment semantic equivalence with complete snapshot evidence"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_inventory_specification(
    inventory: &DraftSnapshotVectorInventorySpecification,
) -> Result<(), String> {
    if !is_bounded_identity(&inventory.entry_point)
        || !is_bounded_identity(&inventory.inventory_id)
        || inventory.compatibility_state != "draft"
        || !is_bounded_identity(&inventory.domain)
        || inventory.draft_schema_revision == 0
        || !is_bounded_identity(&inventory.algorithm)
    {
        return Err(
            "U4d draft snapshot vector inventory must declare one bounded Draft identity"
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
pub(super) fn validate_report(
    inventory: &DraftSnapshotVectorInventoryReport,
    partition: &SegmentPartitionInvarianceReport,
    expected_inventory: &DraftSnapshotVectorInventorySpecification,
    expected_partition: &SegmentPartitionInvarianceSpecification,
    obj_digest: &str,
    gltf_digest: &str,
    profile_digest: &str,
) -> Result<(), String> {
    if inventory.entry_point != expected_inventory.entry_point
        || inventory.inventory_id != expected_inventory.inventory_id
        || inventory.compatibility_state != expected_inventory.compatibility_state
        || inventory.domain != expected_inventory.domain
        || inventory.draft_schema_revision != expected_inventory.draft_schema_revision
        || inventory.algorithm != expected_inventory.algorithm
        || inventory.vectors.len() != expected_inventory.vectors.len()
        || inventory
            .vectors
            .iter()
            .zip(&expected_inventory.vectors)
            .any(|(observed, expected)| {
                observed.vector_id != expected.vector_id
                    || observed.digest_hex != expected.digest_hex
            })
    {
        return Err(
            "U4d draft snapshot vector inventory drifted from its specification".to_owned(),
        );
    }
    validate_report_vectors(
        &inventory.vectors,
        &expected_inventory.vectors,
        obj_digest,
        gltf_digest,
        profile_digest,
        &partition.direct.digest_hex,
        "observed",
    )?;

    validate_partition_report(partition, expected_partition)
}

#[cfg(feature = "repository-input-candidate")]
fn validate_partition_report(
    partition: &SegmentPartitionInvarianceReport,
    expected_partition: &SegmentPartitionInvarianceSpecification,
) -> Result<(), String> {
    if partition.entry_point != expected_partition.entry_point
        || partition.scenario_id != expected_partition.scenario_id
        || partition.domain != expected_partition.domain
        || partition.draft_schema_revision != expected_partition.draft_schema_revision
        || partition.algorithm != expected_partition.algorithm
    {
        return Err(
            "U4d segment-partition report identity drifted from its specification".to_owned(),
        );
    }

    validate_partition_receipt(
        &partition.direct,
        expected_partition.direct_profile_segment_count,
        expected_partition.direct_work_units,
        expected_partition,
        "direct",
    )?;
    validate_partition_receipt(
        &partition.edited,
        expected_partition.edited_profile_segment_count,
        expected_partition.edited_work_units,
        expected_partition,
        "edited",
    )?;

    if partition.direct.segment_count >= partition.edited.segment_count
        || partition.direct.work_units >= partition.edited.work_units
        || partition.direct.digest_hex != partition.edited.digest_hex
        || partition.direct.preview != partition.edited.preview
        || partition.direct.transcript != partition.edited.transcript
        || partition.direct.normalized_semantic_totals
            != partition.edited.normalized_semantic_totals
    {
        return Err(
            "U4d segment-partition receipts did not independently prove semantic snapshot invariance"
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_partition_receipt(
    receipt: &SegmentPartitionReceiptReport,
    expected_segment_count: u64,
    expected_work_units: u64,
    expected_partition: &SegmentPartitionInvarianceSpecification,
    label: &str,
) -> Result<(), String> {
    if receipt.segment_count != expected_segment_count
        || receipt.digest_hex != expected_partition.digest_hex
        || receipt.preview != expected_partition.preview
        || receipt.transcript != expected_partition.transcript
        || receipt.normalized_semantic_totals != expected_partition.semantic_totals
        || receipt.work_units != expected_work_units
    {
        return Err(format!(
            "U4d {label} segment-partition receipt drifted from its specification"
        ));
    }
    Ok(())
}

fn validate_vectors(
    vectors: &[DraftSnapshotVectorSpecification],
    obj_digest: &str,
    gltf_digest: &str,
    profile_digest: &str,
    partition_digest: &str,
    label: &str,
) -> Result<(), String> {
    if vectors.len() != DraftSnapshotVectorSource::ALL.len() {
        return Err(format!(
            "U4d {label} draft snapshot inventory must contain one vector for every source"
        ));
    }
    let mut ids = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for vector in vectors {
        let observed_digest = match vector.source {
            DraftSnapshotVectorSource::ObjCapability => obj_digest,
            DraftSnapshotVectorSource::GltfCapability => gltf_digest,
            DraftSnapshotVectorSource::ProfileClosure => profile_digest,
            DraftSnapshotVectorSource::SegmentPartition => partition_digest,
        };
        if !is_bounded_identity(&vector.vector_id)
            || !ids.insert(vector.vector_id.as_str())
            || !sources.insert(vector.source)
            || !is_lowercase_sha256(&vector.digest_hex)
            || vector.digest_hex != observed_digest
        {
            return Err(format!(
                "U4d {label} draft snapshot vectors drifted from their declared unique sources"
            ));
        }
    }
    if sources.len() != DraftSnapshotVectorSource::ALL.len()
        || DraftSnapshotVectorSource::ALL
            .iter()
            .any(|source| !sources.contains(source))
    {
        return Err(format!(
            "U4d {label} draft snapshot inventory omitted a required source"
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_report_vectors(
    observed: &[DraftSnapshotVectorReport],
    expected: &[DraftSnapshotVectorSpecification],
    obj_digest: &str,
    gltf_digest: &str,
    profile_digest: &str,
    partition_digest: &str,
    label: &str,
) -> Result<(), String> {
    if observed.len() != expected.len() || observed.len() != DraftSnapshotVectorSource::ALL.len() {
        return Err(format!(
            "U4d {label} draft snapshot inventory has an unexpected vector count"
        ));
    }
    let mut ids = BTreeSet::new();
    for (observed, expected) in observed.iter().zip(expected) {
        let expected_digest = match expected.source {
            DraftSnapshotVectorSource::ObjCapability => obj_digest,
            DraftSnapshotVectorSource::GltfCapability => gltf_digest,
            DraftSnapshotVectorSource::ProfileClosure => profile_digest,
            DraftSnapshotVectorSource::SegmentPartition => partition_digest,
        };
        if observed.vector_id != expected.vector_id
            || !ids.insert(observed.vector_id.as_str())
            || !is_bounded_identity(&observed.vector_id)
            || !is_lowercase_sha256(&observed.digest_hex)
            || observed.digest_hex != expected_digest
        {
            return Err(format!(
                "U4d {label} draft snapshot vector report drifted from its declared source"
            ));
        }
    }
    Ok(())
}

fn is_bounded_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

impl DraftSnapshotVectorInventorySpecification {
    pub(super) fn matches_mapping(&self, mapping: &CoverageMappingSpecification) -> bool {
        mapping.entry_point == self.entry_point && mapping.scenario_id == self.inventory_id
    }
}

#[cfg(feature = "repository-input-candidate")]
impl DraftSnapshotVectorInventoryReport {
    pub(super) fn vector_count(&self) -> usize {
        self.vectors.len()
    }
}

#[cfg(all(test, feature = "repository-input-candidate"))]
mod tests {
    use super::*;

    fn vector(
        vector_id: &str,
        source: DraftSnapshotVectorSource,
        digest_hex: &str,
    ) -> DraftSnapshotVectorSpecification {
        DraftSnapshotVectorSpecification {
            vector_id: vector_id.to_owned(),
            source,
            digest_hex: digest_hex.to_owned(),
        }
    }

    #[test]
    fn rejects_duplicate_sources_or_cross_scenario_snapshot_vectors() {
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut vectors = vec![
            vector("obj", DraftSnapshotVectorSource::ObjCapability, digest),
            vector("gltf", DraftSnapshotVectorSource::GltfCapability, digest),
            vector("profile", DraftSnapshotVectorSource::ProfileClosure, digest),
            vector(
                "partition",
                DraftSnapshotVectorSource::SegmentPartition,
                digest,
            ),
        ];
        validate_vectors(&vectors, digest, digest, digest, digest, "test")
            .expect("source-qualified vectors must validate");

        vectors[1].source = DraftSnapshotVectorSource::ObjCapability;
        assert!(validate_vectors(&vectors, digest, digest, digest, digest, "test").is_err());
        vectors[1].source = DraftSnapshotVectorSource::GltfCapability;
        vectors[3].digest_hex =
            "1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();
        assert!(validate_vectors(&vectors, digest, digest, digest, digest, "test").is_err());
    }

    #[test]
    fn rejects_report_vector_bound_to_the_wrong_observed_source() {
        let obj = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let gltf = "1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let profile = "2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let partition = "3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let expected = vec![
            vector("obj", DraftSnapshotVectorSource::ObjCapability, obj),
            vector("gltf", DraftSnapshotVectorSource::GltfCapability, gltf),
            vector(
                "profile",
                DraftSnapshotVectorSource::ProfileClosure,
                profile,
            ),
            vector(
                "partition",
                DraftSnapshotVectorSource::SegmentPartition,
                partition,
            ),
        ];
        let mut observed = expected
            .iter()
            .map(|vector| DraftSnapshotVectorReport {
                vector_id: vector.vector_id.clone(),
                digest_hex: vector.digest_hex.clone(),
            })
            .collect::<Vec<_>>();
        validate_report_vectors(&observed, &expected, obj, gltf, profile, partition, "test")
            .expect("source-bound report vectors must validate");

        observed[0].digest_hex = gltf.to_owned();
        assert!(
            validate_report_vectors(&observed, &expected, obj, gltf, profile, partition, "test",)
                .is_err()
        );
    }

    #[test]
    fn rejects_divergent_segment_partition_receipts() {
        let expected = SegmentPartitionInvarianceSpecification {
            entry_point: "canonical-node-scene-snapshot".to_owned(),
            scenario_id: "canonical-snapshot-segment-partition-invariance-v1".to_owned(),
            domain: "baozi.canonical-scene.snapshot".to_owned(),
            draft_schema_revision: 1,
            algorithm: "sha256".to_owned(),
            direct_profile_segment_count: 1,
            edited_profile_segment_count: 2,
            digest_hex: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            preview: BoundedPreview {
                schema_id: "baozi.canonical-scene.snapshot-preview.v1".to_owned(),
                bytes: 1,
                sha256: "1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_owned(),
                items_omitted_by_limit: 0,
                bytes_truncated: false,
                items_truncated: false,
            },
            transcript: FixedVector {
                schema_id: "baozi.canonical-scene.snapshot-transcript.draft-v1".to_owned(),
                observed_bytes: 1,
                sha256: "2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_owned(),
            },
            semantic_totals: zero_totals(),
            direct_work_units: 1,
            edited_work_units: 2,
        };
        let mut report = SegmentPartitionInvarianceReport {
            entry_point: expected.entry_point.clone(),
            scenario_id: expected.scenario_id.clone(),
            domain: expected.domain.clone(),
            draft_schema_revision: expected.draft_schema_revision,
            algorithm: expected.algorithm.clone(),
            direct: SegmentPartitionReceiptReport {
                segment_count: expected.direct_profile_segment_count,
                digest_hex: expected.digest_hex.clone(),
                preview: expected.preview.clone(),
                transcript: expected.transcript.clone(),
                normalized_semantic_totals: expected.semantic_totals.clone(),
                work_units: expected.direct_work_units,
            },
            edited: SegmentPartitionReceiptReport {
                segment_count: expected.edited_profile_segment_count,
                digest_hex: expected.digest_hex.clone(),
                preview: expected.preview.clone(),
                transcript: expected.transcript.clone(),
                normalized_semantic_totals: expected.semantic_totals.clone(),
                work_units: expected.edited_work_units,
            },
        };
        validate_partition_report(&report, &expected)
            .expect("independent equivalent receipts must validate");

        report.edited.transcript.sha256 =
            "3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();
        assert!(validate_partition_report(&report, &expected).is_err());
    }

    fn zero_totals() -> CanonicalSnapshotTotals {
        serde_json::from_str(
            r#"{
            "node_count": 0, "root_count": 0, "edge_count": 0,
            "camera_count": 0, "light_count": 0, "camera_attachment_count": 0,
            "light_attachment_count": 0, "image_count": 0, "sampler_count": 0,
            "texture_count": 0, "material_count": 0, "material_binding_count": 0,
            "image_payload_bytes": 0, "media_type_bytes": 0, "source_key_bytes": 0,
            "binding_semantic_scalar_count": 0, "material_factor_scalar_count": 0,
            "logical_mesh_count": 0, "primitive_count": 0, "attribute_count": 0,
            "mesh_instance_count": 0, "polygon_boundary_count": 0,
            "primitive_index_count": 0, "sparse_index_count": 0,
            "canonical_scalar_count": 0, "f32_scalar_count": 0, "u32_scalar_count": 0,
            "skin_count": 0, "logical_morph_count": 0, "morph_definition_count": 0,
            "primitive_deformation_count": 0, "morph_target_count": 0,
            "morph_delta_count": 0, "deformation_mesh_instance_count": 0,
            "skin_joint_count": 0, "inverse_bind_matrix_count": 0,
            "skin_correction_matrix_count": 0, "skin_name_bytes": 0,
            "morph_default_weight_count": 0, "morph_name_bytes": 0,
            "influence_row_count": 0, "influence_count": 0,
            "morph_value_vector_count": 0, "morph_sparse_index_count": 0,
            "instance_morph_weight_count": 0, "animation_clip_count": 0,
            "animation_channel_count": 0, "animation_time_scalar_count": 0,
            "animation_value_scalar_count": 0, "animation_in_tangent_scalar_count": 0,
            "animation_out_tangent_scalar_count": 0, "profile_resource_count": 0,
            "profile_source_count": 0, "profile_provenance_node_count": 0,
            "profile_operation_count": 0, "profile_evidence_count": 0,
            "profile_owner_count": 0, "profile_attachment_count": 0,
            "profile_metadata_entry_count": 0, "profile_parent_reference_count": 0,
            "profile_wide_value_count": 0, "profile_lineage_text_bytes": 0,
            "profile_lineage_digest_bytes": 0, "profile_metadata_text_bytes": 0,
            "profile_metadata_payload_bytes": 0, "profile_metadata_bool_scalar_count": 0,
            "profile_metadata_i64_scalar_count": 0, "profile_metadata_u64_scalar_count": 0,
            "profile_metadata_f64_scalar_count": 0, "profile_metadata_f32_scalar_count": 0,
            "transcript_bytes": 0, "work_units": 0
        }"#,
        )
        .expect("zero snapshot totals must deserialize")
    }
}
