use std::sync::atomic::{AtomicU64, Ordering};

use allocation_counter::AllocationInfo;
use baozi_operation::{__u4c::MeasurementSession, StoreTerminalCause};

use crate::derived::DerivedAbsenceReason;
use crate::entity::AuthoredTransform;
use crate::identity::{NodeId, SceneGenerationId};
use crate::math::{Quat, Vec3};
use crate::provenance::OptionsDigest;
use crate::remap::RemapChange;

use super::super::super::construction::activation::SceneActivationTransition;
use super::super::super::construction::node_scene::edit::{
    CanonicalRemapSet, NodeSceneEditTerminal, NodeTransformEditStart, try_begin_node_transform_edit,
};
use super::super::super::construction::node_scene::snapshot::{
    NodeSceneSnapshotAlgorithm, NodeSceneSnapshotLimits,
    collect_segment_partition_snapshot_observation_for_probe,
};
use super::super::super::construction::node_scene::{
    FACADE_OBSERVATION_SCHEMA_ID, FacadeObservationOutcomeV1, GLTF_SOURCE_VECTOR_SCHEMA_ID,
    GltfCapabilityCanonicalShapeObservation, GltfCapabilitySourceShapeObservation,
    NodeSceneConstructionTerminal, OBJ_SOURCE_VECTOR_SCHEMA_ID,
    ObjCapabilityCanonicalShapeObservation, ObjCapabilitySourceShapeObservation,
    ProfileClosureCanonicalShapeObservation, ProfileClosureNegativeCaseObservation,
    ProfileClosurePreflightStorageObservation, build_gltf_capability_scene_for_probe,
    build_minimal_fallback_terminal_for_probe, build_obj_capability_scene_for_probe,
    build_profile_closure_scene_for_probe, build_protected_terminal_activation_for_probe,
    capture_gltf_capability_snapshots_for_probe, capture_obj_capability_snapshots_for_probe,
    capture_profile_closure_snapshots_for_probe, collect_gltf_capability_vectors_for_probe,
    collect_obj_capability_vectors_for_probe, collect_profile_closure_digest_vectors_for_probe,
    collect_profile_closure_negative_cases_for_probe, inspect_gltf_capability_semantics_for_probe,
    inspect_gltf_capability_semantics_with_first_transform_for_probe,
    inspect_obj_capability_semantics_for_probe, inspect_profile_closure_semantics_for_probe,
    project_gltf_facade_observation_v1_for_probe,
};
use super::super::super::construction::{
    COMPLEX_SEMANTIC_TRANSCRIPT_SCHEMA_ID, ComplexFixtureFault, ComplexFixtureTerminal,
    FAILURE_TERMINAL_WORK_DIMENSION, STORE_DIMENSION, SceneConstructionOperation,
    SceneConstructionTerminal,
};
use super::super::{
    authored_fixture, test_account, test_account_with_allocation_limit, test_factory, test_limits,
};
use super::report::{
    CanonicalSnapshotTotalsReport, CapabilityBoundedPreviewReport, CapabilityMutationReport,
    CapabilityPhysicalReport, FacadeEquivalentInspectionReport, FacadeObservationSuccessReport,
    FacadeObservationTypedErrorReport, FaultPhysicalReport, FixedVectorReport,
    GltfCanonicalShapeReport, GltfCapabilityReport, GltfChangedSceneEditPhysicalReport,
    GltfChangedSceneEditReport, GltfSourceShapeReport, GltfUnchangedViewReport,
    HotLeafObservationReport, InheritedHotLeafReport, MinimalFallbackReport,
    ObjCanonicalShapeReport, ObjCapabilityReport, ObjSourceShapeReport,
    PendingSceneActivationPhysicalReport, PendingSceneActivationReport,
    ProfileClosureCanonicalShapeReport, ProfileClosureMutationReport,
    ProfileClosureNegativeCaseReport, ProfileClosureReport, ProtectedTerminalActivationReport,
    ReadOnlyPhysicalReport, ScalePhysicalReport, ScaleScenarioReport,
    SegmentPartitionInvarianceReport, SegmentPartitionReceiptReport, SemanticTranscriptReport,
    ShareGenerationReport, SharePhysicalReport, ValidationFaultReport, evidence_report,
    handoff_report, hex_encode, phase_report, release_report, sha256_hex,
};

const COMPLEX_PREVIEW_LIMIT_BYTES: usize = 192;

pub(super) fn collect_segment_partition_invariance() -> SegmentPartitionInvarianceReport {
    let observation = collect_segment_partition_snapshot_observation_for_probe();
    let algorithm = match observation.algorithm {
        NodeSceneSnapshotAlgorithm::Sha256 => "sha256",
    };
    let direct = observation.direct;
    let edited = observation.edited;

    SegmentPartitionInvarianceReport {
        entry_point: "canonical-node-scene-snapshot",
        scenario_id: "canonical-snapshot-segment-partition-invariance-v1",
        domain: observation.domain,
        draft_schema_revision: observation.draft_schema_revision,
        algorithm,
        direct: SegmentPartitionReceiptReport {
            segment_count: direct.segment_count,
            digest_hex: hex_encode(&direct.digest),
            preview: CapabilityBoundedPreviewReport {
                schema_id: "baozi.canonical-scene.snapshot-preview.v1",
                bytes: direct.preview_text.len(),
                sha256: sha256_hex(direct.preview_text.as_bytes()),
                items_omitted_by_limit: direct.preview_items_omitted_by_limit,
                bytes_truncated: direct.preview_bytes_truncated,
                items_truncated: direct.preview_items_truncated,
            },
            transcript: FixedVectorReport {
                schema_id: "baozi.canonical-scene.snapshot-transcript.draft-v1",
                observed_bytes: direct.transcript.len(),
                sha256: sha256_hex(&direct.transcript),
            },
            normalized_semantic_totals: CanonicalSnapshotTotalsReport::from(
                direct.normalized_semantic_totals,
            ),
            work_units: direct.work_units,
        },
        edited: SegmentPartitionReceiptReport {
            segment_count: edited.segment_count,
            digest_hex: hex_encode(&edited.digest),
            preview: CapabilityBoundedPreviewReport {
                schema_id: "baozi.canonical-scene.snapshot-preview.v1",
                bytes: edited.preview_text.len(),
                sha256: sha256_hex(edited.preview_text.as_bytes()),
                items_omitted_by_limit: edited.preview_items_omitted_by_limit,
                bytes_truncated: edited.preview_bytes_truncated,
                items_truncated: edited.preview_items_truncated,
            },
            transcript: FixedVectorReport {
                schema_id: "baozi.canonical-scene.snapshot-transcript.draft-v1",
                observed_bytes: edited.transcript.len(),
                sha256: sha256_hex(&edited.transcript),
            },
            normalized_semantic_totals: CanonicalSnapshotTotalsReport::from(
                edited.normalized_semantic_totals,
            ),
            work_units: edited.work_units,
        },
    }
}

#[derive(Clone, Copy)]
struct ScaleDescriptor {
    scenario_id: &'static str,
    construction_scenario_id: &'static str,
    inspection_scenario_id: &'static str,
    target_release_scenario_id: &'static str,
    foreign_release_scenario_id: &'static str,
    image_count: usize,
}

const SCALE_DESCRIPTORS: [ScaleDescriptor; 3] = [
    ScaleDescriptor {
        scenario_id: "complex-members-n-v1",
        construction_scenario_id: "complex-members-n-construction-v1",
        inspection_scenario_id: "complex-members-n-typed-inspection-v1",
        target_release_scenario_id: "complex-members-n-target-release-v1",
        foreign_release_scenario_id: "complex-members-n-foreign-release-v1",
        image_count: 1,
    },
    ScaleDescriptor {
        scenario_id: "complex-members-2n-v1",
        construction_scenario_id: "complex-members-2n-construction-v1",
        inspection_scenario_id: "complex-members-2n-typed-inspection-v1",
        target_release_scenario_id: "complex-members-2n-target-release-v1",
        foreign_release_scenario_id: "complex-members-2n-foreign-release-v1",
        image_count: 14,
    },
    ScaleDescriptor {
        scenario_id: "complex-members-4n-v1",
        construction_scenario_id: "complex-members-4n-construction-v1",
        inspection_scenario_id: "complex-members-4n-typed-inspection-v1",
        target_release_scenario_id: "complex-members-4n-target-release-v1",
        foreign_release_scenario_id: "complex-members-4n-foreign-release-v1",
        image_count: 40,
    },
];

#[derive(Clone, Copy)]
struct ValidationFaultDescriptor {
    scenario_id: &'static str,
    release_scenario_id: &'static str,
    field_class: &'static str,
    fault: ComplexFixtureFault,
}

const VALIDATION_FAULT_DESCRIPTORS: [ValidationFaultDescriptor; 6] = [
    ValidationFaultDescriptor {
        scenario_id: "complex.scalar.texture-image-index.v1",
        release_scenario_id: "complex.scalar.texture-image-index.release.v1",
        field_class: "scalar",
        fault: ComplexFixtureFault::CrossTableIndex,
    },
    ValidationFaultDescriptor {
        scenario_id: "complex.reference.texture-images.v1",
        release_scenario_id: "complex.reference.texture-images.release.v1",
        field_class: "reference",
        fault: ComplexFixtureFault::ReferenceTarget,
    },
    ValidationFaultDescriptor {
        scenario_id: "complex.sequence.inverse-bind.v1",
        release_scenario_id: "complex.sequence.inverse-bind.release.v1",
        field_class: "sequence",
        fault: ComplexFixtureFault::SequenceValue,
    },
    ValidationFaultDescriptor {
        scenario_id: "complex.utf8.image-mime.v1",
        release_scenario_id: "complex.utf8.image-mime.release.v1",
        field_class: "utf8",
        fault: ComplexFixtureFault::Utf8Value,
    },
    ValidationFaultDescriptor {
        scenario_id: "complex.bytes.image-payload.v1",
        release_scenario_id: "complex.bytes.image-payload.release.v1",
        field_class: "bytes",
        fault: ComplexFixtureFault::BytesValue,
    },
    ValidationFaultDescriptor {
        scenario_id: "complex.orphan.canonical-block.v1",
        release_scenario_id: "complex.orphan.canonical-block.release.v1",
        field_class: "orphan",
        fault: ComplexFixtureFault::OrphanCanonicalBlock,
    },
];

fn measure_release<T>(value: T) -> AllocationInfo {
    allocation_counter::measure(move || drop(value))
}

fn assert_zero_allocation_handoff(label: &str, observation: &AllocationInfo) {
    assert_eq!(
        observation.count_total, 0,
        "{label} handoff must not allocate"
    );
    assert_eq!(
        observation.count_current, 0,
        "{label} handoff must not change the live allocation count"
    );
    assert_eq!(
        observation.count_max, 0,
        "{label} handoff must not allocate"
    );
    assert_eq!(
        observation.bytes_total, 0,
        "{label} handoff must not allocate"
    );
    assert_eq!(
        observation.bytes_current, 0,
        "{label} handoff must not change live allocation bytes"
    );
    assert_eq!(
        observation.bytes_max, 0,
        "{label} handoff must not allocate"
    );
}

fn assert_allocator_lifecycle_balanced(label: &str, observations: &[&AllocationInfo]) {
    let count_current = observations
        .iter()
        .try_fold(0_i64, |total, observation| {
            total.checked_add(observation.count_current)
        })
        .expect("allocator lifecycle count delta must fit i64");
    let bytes_current = observations
        .iter()
        .try_fold(0_i64, |total, observation| {
            total.checked_add(observation.bytes_current)
        })
        .expect("allocator lifecycle byte delta must fit i64");
    assert_eq!(
        count_current, 0,
        "{label} must release every allocation observed during construction"
    );
    assert_eq!(
        bytes_current, 0,
        "{label} must release every byte observed during construction"
    );
}

pub(super) fn collect_obj_capability() -> ObjCapabilityReport {
    const SCENARIO_ID: &str = "canonical-obj-corner-smoothing-v1";
    const CONSTRUCTION_ID: &str = "canonical-obj-corner-smoothing-construction-v1";
    const INSPECTION_ID: &str = "canonical-obj-corner-smoothing-inspection-v1";
    const SNAPSHOT_ID: &str = "canonical-obj-corner-smoothing-snapshot-v1";
    const RELEASE_ID: &str = "canonical-obj-corner-smoothing-release-v1";

    let vector_observations = collect_obj_capability_vectors_for_probe();
    let factory = test_factory(1);
    let account = test_account(&factory);
    let generation_source = AtomicU64::new(0);
    let physical_baseline = account.snapshot();

    let construction_session =
        MeasurementSession::begin().expect("OBJ construction measurement must start");
    let mut parts = None;
    let construction_allocations = allocation_counter::measure(|| {
        parts = Some(build_obj_capability_scene_for_probe(
            &account,
            &generation_source,
        ));
    });
    let construction_measurement = construction_session
        .finish()
        .expect("OBJ construction measurement must finish");
    let parts = parts.expect("OBJ construction closure must publish a Scene");
    let physical_retained = account.snapshot();
    let canonical_block_count = parts.scene.canonical_block_count();
    let outcome_evidence_block_count = parts.report.outcome_evidence_block_count();
    let evidence = evidence_report(outcome_evidence_block_count, parts.report.ledger_evidence());

    let inspection_session =
        MeasurementSession::begin().expect("OBJ typed inspection measurement must start");
    let mut canonical_shape = None;
    let inspection_allocations = allocation_counter::measure(|| {
        canonical_shape = Some(inspect_obj_capability_semantics_for_probe(&parts.scene));
    });
    let inspection_measurement = inspection_session
        .finish()
        .expect("OBJ typed inspection measurement must finish");
    let physical_after_inspection = account.snapshot();
    let canonical_shape = canonical_shape.expect("OBJ typed inspection must return its shape");

    let snapshot_session =
        MeasurementSession::begin().expect("OBJ snapshot measurement must start");
    let mut snapshots = None;
    let snapshot_allocations = allocation_counter::measure(|| {
        snapshots = Some(capture_obj_capability_snapshots_for_probe(&parts));
    });
    let snapshot_measurement = snapshot_session
        .finish()
        .expect("OBJ snapshot measurement must finish");
    let (full_snapshot, bounded_snapshot) =
        snapshots.expect("OBJ snapshot closure must return both snapshots");
    let physical_after_snapshot = account.snapshot();

    assert_eq!(bounded_snapshot.digest, full_snapshot.digest);
    assert_eq!(
        vector_observations.baseline_snapshot_digest,
        full_snapshot.digest,
    );
    assert_eq!(
        vector_observations.equivalent_snapshot_digest,
        full_snapshot.digest,
    );
    assert_ne!(
        vector_observations.equivalent_source_vector,
        vector_observations.source_vector,
    );
    for (index, mutation) in vector_observations.mutations.iter().enumerate() {
        assert_ne!(mutation.snapshot_digest, full_snapshot.digest);
        for earlier in &vector_observations.mutations[..index] {
            assert_ne!(mutation.snapshot_digest, earlier.snapshot_digest);
        }
    }
    assert_eq!(bounded_snapshot.totals, full_snapshot.totals);
    assert!(!full_snapshot.preview.bytes_truncated);
    assert!(!full_snapshot.preview.items_truncated);
    assert!(bounded_snapshot.preview.text.len() <= 96);
    assert!(bounded_snapshot.preview.items_truncated);
    assert!(bounded_snapshot.preview.items_omitted_by_limit > 0);

    let full_snapshot_digest_hex = hex_encode(&full_snapshot.digest);
    let bounded_snapshot_digest_hex = hex_encode(&bounded_snapshot.digest);
    let snapshot_totals = CanonicalSnapshotTotalsReport::from(full_snapshot.totals);
    let bounded_preview = CapabilityBoundedPreviewReport {
        schema_id: "baozi.canonical-scene.bounded-preview.v1",
        bytes: bounded_snapshot.preview.text.len(),
        sha256: sha256_hex(bounded_snapshot.preview.text.as_bytes()),
        items_omitted_by_limit: bounded_snapshot.preview.items_omitted_by_limit,
        bytes_truncated: bounded_snapshot.preview.bytes_truncated,
        items_truncated: bounded_snapshot.preview.items_truncated,
    };
    let source_vector = FixedVectorReport {
        schema_id: OBJ_SOURCE_VECTOR_SCHEMA_ID,
        observed_bytes: vector_observations.source_vector.len(),
        sha256: sha256_hex(&vector_observations.source_vector),
    };
    let equivalent_source_vector = FixedVectorReport {
        schema_id: OBJ_SOURCE_VECTOR_SCHEMA_ID,
        observed_bytes: vector_observations.equivalent_source_vector.len(),
        sha256: sha256_hex(&vector_observations.equivalent_source_vector),
    };
    let equivalent_snapshot_digest_hex =
        hex_encode(&vector_observations.equivalent_snapshot_digest);
    let mutations = vector_observations
        .mutations
        .into_iter()
        .map(|mutation| CapabilityMutationReport {
            field_class: mutation.field_class,
            source_vector: FixedVectorReport {
                schema_id: OBJ_SOURCE_VECTOR_SCHEMA_ID,
                observed_bytes: mutation.source_vector.len(),
                sha256: sha256_hex(&mutation.source_vector),
            },
            snapshot_digest_hex: hex_encode(&mutation.snapshot_digest),
        })
        .collect();

    let release_allocations = allocation_counter::measure(move || {
        drop(full_snapshot);
        drop(bounded_snapshot);
        drop(parts);
    });
    assert_allocator_lifecycle_balanced(
        "canonical OBJ capability",
        &[
            &construction_allocations,
            &inspection_allocations,
            &snapshot_allocations,
            &release_allocations,
        ],
    );
    let physical_after_release = account.snapshot();
    drop(account);
    let factory_after_account_release = factory.snapshot();

    ObjCapabilityReport {
        entry_point: "canonical-obj-corner-smoothing",
        scenario_id: SCENARIO_ID,
        source_shape: source_shape_report(vector_observations.source_shape),
        canonical_shape: canonical_shape_report(canonical_shape),
        source_vector,
        equivalent_source_vector,
        equivalent_snapshot_digest_hex,
        mutations,
        generation_reservations: generation_source.load(Ordering::Relaxed),
        canonical_block_count,
        outcome_evidence_block_count,
        full_snapshot_digest_hex,
        bounded_snapshot_digest_hex,
        snapshot_totals,
        bounded_preview,
        evidence,
        construction: phase_report(
            CONSTRUCTION_ID,
            construction_measurement,
            construction_allocations,
        ),
        inspection: phase_report(
            INSPECTION_ID,
            inspection_measurement,
            inspection_allocations,
        ),
        snapshot: phase_report(SNAPSHOT_ID, snapshot_measurement, snapshot_allocations),
        release: release_report(RELEASE_ID, release_allocations),
        physical_account: CapabilityPhysicalReport {
            baseline: physical_baseline.into(),
            retained: physical_retained.into(),
            after_inspection: physical_after_inspection.into(),
            after_snapshot: physical_after_snapshot.into(),
            after_release: physical_after_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}

fn source_shape_report(shape: ObjCapabilitySourceShapeObservation) -> ObjSourceShapeReport {
    ObjSourceShapeReport {
        position_count: shape.position_count,
        texcoord_count: shape.texcoord_count,
        normal_count: shape.normal_count,
        face_count: shape.face_count,
        corner_count: shape.corner_count,
    }
}

fn canonical_shape_report(
    shape: ObjCapabilityCanonicalShapeObservation,
) -> ObjCanonicalShapeReport {
    ObjCanonicalShapeReport {
        topology: shape.topology,
        vertex_count: shape.vertex_count,
        corner_count: shape.corner_count,
        face_count: shape.face_count,
        vertex_position_elements: shape.vertex_position_elements,
        corner_normal_elements: shape.corner_normal_elements,
        corner_texcoord_elements: shape.corner_texcoord_elements,
        face_smoothing_group_elements: shape.face_smoothing_group_elements,
    }
}

pub(super) fn collect_profile_closure() -> ProfileClosureReport {
    const SCENARIO_ID: &str = "canonical-profile-closure-v1";
    const CONSTRUCTION_ID: &str = "canonical-profile-closure-construction-v1";
    const INSPECTION_ID: &str = "canonical-profile-closure-inspection-v1";
    const SNAPSHOT_ID: &str = "canonical-profile-closure-snapshot-v1";
    const RELEASE_ID: &str = "canonical-profile-closure-release-v1";

    let digest_vectors = collect_profile_closure_digest_vectors_for_probe();
    let negative_cases = collect_profile_closure_negative_cases_for_probe()
        .into_iter()
        .map(profile_closure_negative_case_report)
        .collect();
    let factory = test_factory(1);
    let account = test_account_with_allocation_limit(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let physical_baseline = account.snapshot();

    let construction_session =
        MeasurementSession::begin().expect("profile-closure construction measurement must start");
    let mut parts = None;
    let construction_allocations = allocation_counter::measure(|| {
        parts = Some(build_profile_closure_scene_for_probe(
            &account,
            &generation_source,
        ));
    });
    let construction_measurement = construction_session
        .finish()
        .expect("profile-closure construction measurement must finish");
    let parts = parts.expect("profile-closure construction must publish a Scene");
    let physical_retained = account.snapshot();
    let canonical_block_count = parts.scene.canonical_block_count();
    let outcome_evidence_block_count = parts.report.outcome_evidence_block_count();
    let evidence = evidence_report(outcome_evidence_block_count, parts.report.ledger_evidence());

    let inspection_session =
        MeasurementSession::begin().expect("profile-closure inspection measurement must start");
    let mut canonical_shape = None;
    let inspection_allocations = allocation_counter::measure(|| {
        canonical_shape = Some(inspect_profile_closure_semantics_for_probe(&parts.scene));
    });
    let inspection_measurement = inspection_session
        .finish()
        .expect("profile-closure inspection measurement must finish");
    let physical_after_inspection = account.snapshot();
    let canonical_shape =
        canonical_shape.expect("profile-closure inspection must return its shape");

    let snapshot_session =
        MeasurementSession::begin().expect("profile-closure snapshot measurement must start");
    let mut snapshots = None;
    let snapshot_allocations = allocation_counter::measure(|| {
        snapshots = Some(capture_profile_closure_snapshots_for_probe(&parts));
    });
    let snapshot_measurement = snapshot_session
        .finish()
        .expect("profile-closure snapshot measurement must finish");
    let (full_snapshot, bounded_snapshot) =
        snapshots.expect("profile-closure snapshot must return both snapshots");
    let physical_after_snapshot = account.snapshot();

    assert_eq!(bounded_snapshot.digest, full_snapshot.digest);
    assert_eq!(digest_vectors.baseline, full_snapshot.digest);
    assert_ne!(digest_vectors.metadata_u64_mutation, full_snapshot.digest);
    assert_eq!(bounded_snapshot.totals, full_snapshot.totals);
    assert!(!full_snapshot.preview.bytes_truncated);
    assert!(!full_snapshot.preview.items_truncated);
    assert!(bounded_snapshot.preview.text.len() <= 96);
    assert!(bounded_snapshot.preview.items_truncated || bounded_snapshot.preview.bytes_truncated);

    let full_snapshot_digest_hex = hex_encode(&full_snapshot.digest);
    let bounded_snapshot_digest_hex = hex_encode(&bounded_snapshot.digest);
    let snapshot_totals = CanonicalSnapshotTotalsReport::from(full_snapshot.totals);
    let bounded_preview = CapabilityBoundedPreviewReport {
        schema_id: "baozi.canonical-scene.bounded-preview.v1",
        bytes: bounded_snapshot.preview.text.len(),
        sha256: sha256_hex(bounded_snapshot.preview.text.as_bytes()),
        items_omitted_by_limit: bounded_snapshot.preview.items_omitted_by_limit,
        bytes_truncated: bounded_snapshot.preview.bytes_truncated,
        items_truncated: bounded_snapshot.preview.items_truncated,
    };
    let mutations = vec![ProfileClosureMutationReport {
        field_class: "metadata_u64_value",
        snapshot_digest_hex: hex_encode(&digest_vectors.metadata_u64_mutation),
    }];

    let release_allocations = allocation_counter::measure(move || {
        drop(full_snapshot);
        drop(bounded_snapshot);
        drop(parts);
    });
    assert_allocator_lifecycle_balanced(
        "canonical profile closure",
        &[
            &construction_allocations,
            &inspection_allocations,
            &snapshot_allocations,
            &release_allocations,
        ],
    );
    let physical_after_release = account.snapshot();
    drop(account);
    let factory_after_account_release = factory.snapshot();

    ProfileClosureReport {
        entry_point: "canonical-profile-closure",
        scenario_id: SCENARIO_ID,
        canonical_shape: profile_closure_shape_report(canonical_shape),
        mutations,
        negative_cases,
        generation_reservations: generation_source.load(Ordering::Relaxed),
        canonical_block_count,
        outcome_evidence_block_count,
        full_snapshot_digest_hex,
        bounded_snapshot_digest_hex,
        snapshot_totals,
        bounded_preview,
        evidence,
        construction: phase_report(
            CONSTRUCTION_ID,
            construction_measurement,
            construction_allocations,
        ),
        inspection: phase_report(
            INSPECTION_ID,
            inspection_measurement,
            inspection_allocations,
        ),
        snapshot: phase_report(SNAPSHOT_ID, snapshot_measurement, snapshot_allocations),
        release: release_report(RELEASE_ID, release_allocations),
        physical_account: CapabilityPhysicalReport {
            baseline: physical_baseline.into(),
            retained: physical_retained.into(),
            after_inspection: physical_after_inspection.into(),
            after_snapshot: physical_after_snapshot.into(),
            after_release: physical_after_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}

fn profile_closure_negative_case_report(
    observation: ProfileClosureNegativeCaseObservation,
) -> ProfileClosureNegativeCaseReport {
    let preflight_storage = observation.preflight_storage;
    ProfileClosureNegativeCaseReport {
        scenario_id: observation.scenario_id,
        preflight_disposition: observation.preflight_disposition,
        activation_eligibility_reached: observation.activation_eligibility_reached,
        failure_class: observation.failure_class,
        failure_kind: observation.failure_kind,
        failure_code: observation.failure_code,
        peak_retained_bytes_delta: preflight_storage.peak_retained_bytes_delta,
        peak_retained_allocations_delta: preflight_storage.peak_retained_allocations_delta,
        total_admitted_retained_bytes_delta: preflight_storage.total_admitted_retained_bytes_delta,
        total_admitted_retained_allocations_delta: preflight_storage
            .total_admitted_retained_allocations_delta,
        live_retained_bytes_delta: preflight_storage.live_retained_bytes_delta,
        live_retained_allocations_delta: preflight_storage.live_retained_allocations_delta,
        final_live_retained_bytes: preflight_storage.final_live_retained_bytes,
        final_live_retained_allocations: preflight_storage.final_live_retained_allocations,
    }
}

#[test]
fn profile_negative_report_preserves_nonzero_preflight_storage_deltas() {
    let report = profile_closure_negative_case_report(ProfileClosureNegativeCaseObservation {
        scenario_id: "profile-negative-report-history-canary-v1",
        preflight_disposition: "rejected",
        activation_eligibility_reached: false,
        failure_class: "profile-authoring",
        failure_kind: "history-canary",
        failure_code: 0x5343_1108,
        preflight_storage: ProfileClosurePreflightStorageObservation {
            peak_retained_bytes_delta: 11,
            peak_retained_allocations_delta: 2,
            total_admitted_retained_bytes_delta: 17,
            total_admitted_retained_allocations_delta: 3,
            live_retained_bytes_delta: 5,
            live_retained_allocations_delta: 1,
            final_live_retained_bytes: 5,
            final_live_retained_allocations: 1,
        },
    });

    assert_eq!(report.peak_retained_bytes_delta, 11);
    assert_eq!(report.peak_retained_allocations_delta, 2);
    assert_eq!(report.total_admitted_retained_bytes_delta, 17);
    assert_eq!(report.total_admitted_retained_allocations_delta, 3);
    assert_eq!(report.live_retained_bytes_delta, 5);
    assert_eq!(report.live_retained_allocations_delta, 1);
    assert_eq!(report.final_live_retained_bytes, 5);
    assert_eq!(report.final_live_retained_allocations, 1);
}

fn profile_closure_shape_report(
    shape: ProfileClosureCanonicalShapeObservation,
) -> ProfileClosureCanonicalShapeReport {
    ProfileClosureCanonicalShapeReport {
        node_count: shape.node_count,
        root_count: shape.root_count,
        camera_count: shape.camera_count,
        light_count: shape.light_count,
        camera_attachment_count: shape.camera_attachment_count,
        light_attachment_count: shape.light_attachment_count,
        unit_scale_to_meters: shape.unit_scale_to_meters,
        profile_resource_count: shape.profile_resource_count,
        profile_source_count: shape.profile_source_count,
        profile_provenance_count: shape.profile_provenance_count,
        profile_owner_count: shape.profile_owner_count,
        profile_attachment_count: shape.profile_attachment_count,
        profile_metadata_entry_count: shape.profile_metadata_entry_count,
    }
}

pub(super) fn collect_gltf_capability() -> GltfCapabilityReport {
    const SCENARIO_ID: &str = "backend-free-gltf-shape-v1";
    const CONSTRUCTION_ID: &str = "canonical-gltf-five-domain-construction-v1";
    const INSPECTION_ID: &str = "canonical-gltf-five-domain-inspection-v1";
    const FACADE_EQUIVALENT_ID: &str = "canonical-gltf-facade-observation-v1";
    const FACADE_TYPED_ERROR_ID: &str = "canonical-gltf-facade-typed-error-v1";
    const SNAPSHOT_ID: &str = "canonical-gltf-five-domain-snapshot-v1";
    const UNCHANGED_VIEW_ID: &str = "canonical-gltf-five-domain-unchanged-view-v1";
    const CHANGED_SCENE_EDIT_ID: &str = "canonical-gltf-node-transform-edit-v1";
    const EDIT_SNAPSHOT_ID: &str = "canonical-gltf-node-transform-edit-snapshot-v1";
    const EDIT_INPUT_RELEASE_ID: &str = "canonical-gltf-edit-input-release-v1";
    const EDIT_REPORT_RELEASE_ID: &str = "canonical-gltf-edit-report-release-v1";
    const EDIT_OUTPUT_RELEASE_ID: &str = "canonical-gltf-edit-output-release-v1";
    const RELEASE_ID: &str = "canonical-gltf-five-domain-release-v1";

    let vector_observations = collect_gltf_capability_vectors_for_probe();
    let factory = test_factory(1);
    let account = test_account_with_allocation_limit(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let physical_baseline = account.snapshot();

    let construction_session =
        MeasurementSession::begin().expect("glTF construction measurement must start");
    let mut parts = None;
    let construction_allocations = allocation_counter::measure(|| {
        parts = Some(build_gltf_capability_scene_for_probe(
            &account,
            &generation_source,
        ));
    });
    let construction_measurement = construction_session
        .finish()
        .expect("glTF construction measurement must finish");
    let mut parts = parts.expect("glTF construction closure must publish a Scene");
    let physical_retained = account.snapshot();
    let canonical_block_count = parts.scene.canonical_block_count();
    let outcome_evidence_block_count = parts.report.outcome_evidence_block_count();
    let evidence = evidence_report(outcome_evidence_block_count, parts.report.ledger_evidence());

    let inspection_session =
        MeasurementSession::begin().expect("glTF typed inspection measurement must start");
    let mut canonical_shape = None;
    let inspection_allocations = allocation_counter::measure(|| {
        canonical_shape = Some(inspect_gltf_capability_semantics_for_probe(&parts.scene));
    });
    let inspection_measurement = inspection_session
        .finish()
        .expect("glTF typed inspection measurement must finish");
    let physical_after_inspection = account.snapshot();
    let canonical_shape = canonical_shape.expect("glTF typed inspection must return its shape");

    let snapshot_session =
        MeasurementSession::begin().expect("glTF snapshot measurement must start");
    let mut snapshots = None;
    let snapshot_allocations = allocation_counter::measure(|| {
        snapshots = Some(capture_gltf_capability_snapshots_for_probe(&parts));
    });
    let snapshot_measurement = snapshot_session
        .finish()
        .expect("glTF snapshot measurement must finish");
    let (full_snapshot, bounded_snapshot) =
        snapshots.expect("glTF snapshot closure must return both snapshots");
    let physical_after_snapshot = account.snapshot();

    let facade_success_generation_reservations_before = generation_source.load(Ordering::Relaxed);
    let facade_success_canonical_block_count_before = parts.scene.canonical_block_count();
    let facade_success_physical_before = account.snapshot();
    let facade_success_session =
        MeasurementSession::begin().expect("facade-equivalent success measurement must start");
    let mut facade_success_outcome: Option<FacadeObservationOutcomeV1> = None;
    let facade_success_allocations = allocation_counter::measure(|| {
        facade_success_outcome = Some(project_gltf_facade_observation_v1_for_probe(&parts.scene));
    });
    let facade_success_measurement = facade_success_session
        .finish()
        .expect("facade-equivalent success measurement must finish");
    let facade_success = match facade_success_outcome
        .expect("facade-equivalent success projection must produce an outcome")
    {
        Ok(observation) => observation,
        Err(error) => panic!(
            "validated glTF facade observation must project success: {} / {}",
            error.error_class, error.reason
        ),
    };
    let facade_success_physical_after = account.snapshot();
    let facade_success_generation_reservations_after = generation_source.load(Ordering::Relaxed);
    let facade_success_canonical_block_count_after = parts.scene.canonical_block_count();
    assert_eq!(facade_success.schema_id, FACADE_OBSERVATION_SCHEMA_ID);
    assert_eq!(facade_success.totals, canonical_shape);
    assert_ne!(facade_success.transcript_digest, full_snapshot.digest);
    assert!(facade_success.transcript_bytes > 0);
    assert!(facade_success.reference_closure_verified);
    assert!(facade_success.cursor_exhausted);
    assert_eq!(
        facade_success_generation_reservations_after,
        facade_success_generation_reservations_before,
    );
    assert_eq!(
        facade_success_canonical_block_count_after,
        facade_success_canonical_block_count_before,
    );
    assert_eq!(
        facade_success_physical_after,
        facade_success_physical_before
    );

    let unchanged_generation = parts.scene.generation();
    let unchanged_generation_reservations_before = generation_source.load(Ordering::Relaxed);
    let unchanged_canonical_block_count_before = parts.scene.canonical_block_count();
    let unchanged_physical_before = account.snapshot();
    let unchanged_view_session =
        MeasurementSession::begin().expect("glTF unchanged view measurement must start");
    let mut unchanged_observations = None;
    let unchanged_view_allocations = allocation_counter::measure(|| {
        let first = inspect_gltf_capability_semantics_for_probe(&parts.scene);
        let second = inspect_gltf_capability_semantics_for_probe(&parts.scene);
        unchanged_observations = Some((first, second));
    });
    let unchanged_view_measurement = unchanged_view_session
        .finish()
        .expect("glTF unchanged view measurement must finish");
    let unchanged_physical_after = account.snapshot();
    let (unchanged_observation, repeated_unchanged_observation) =
        unchanged_observations.expect("glTF unchanged view must return both observations");
    let unchanged_generation_preserved = parts.scene.generation() == unchanged_generation;
    let unchanged_generation_reservations_after = generation_source.load(Ordering::Relaxed);
    let unchanged_canonical_block_count_after = parts.scene.canonical_block_count();

    assert_eq!(bounded_snapshot.digest, full_snapshot.digest);
    assert_eq!(bounded_snapshot.totals, full_snapshot.totals);
    assert_eq!(
        vector_observations.baseline_snapshot_digest,
        full_snapshot.digest,
    );
    assert!(!full_snapshot.preview.bytes_truncated);
    assert!(!full_snapshot.preview.items_truncated);
    assert!(bounded_snapshot.preview.text.len() <= 96);
    assert!(bounded_snapshot.preview.bytes_truncated || bounded_snapshot.preview.items_truncated);
    assert!(bounded_snapshot.preview.items_omitted_by_limit > 0);
    assert_eq!(vector_observations.mutations.len(), 9);
    assert_eq!(unchanged_observation, canonical_shape);
    assert_eq!(repeated_unchanged_observation, canonical_shape);
    assert!(unchanged_generation_preserved);
    assert_eq!(
        unchanged_generation_reservations_after,
        unchanged_generation_reservations_before,
    );
    assert_eq!(
        unchanged_canonical_block_count_after,
        unchanged_canonical_block_count_before,
    );
    assert_eq!(unchanged_physical_after, unchanged_physical_before);
    for (index, mutation) in vector_observations.mutations.iter().enumerate() {
        assert_ne!(mutation.source_vector, vector_observations.source_vector);
        assert_ne!(mutation.snapshot_digest, full_snapshot.digest);
        for earlier in &vector_observations.mutations[..index] {
            assert_ne!(mutation.source_vector, earlier.source_vector);
            assert_ne!(mutation.snapshot_digest, earlier.snapshot_digest);
        }
    }
    assert_eq!(
        (
            vector_observations.source_shape.node_count,
            vector_observations.source_shape.image_count,
            vector_observations.source_shape.primitive_count,
            vector_observations.source_shape.skin_count,
            vector_observations.source_shape.animation_channel_count,
        ),
        (
            canonical_shape.node_count,
            canonical_shape.image_count,
            canonical_shape.primitive_count,
            canonical_shape.skin_count,
            canonical_shape.animation_channel_count,
        ),
    );

    let full_snapshot_digest_hex = hex_encode(&full_snapshot.digest);
    let bounded_snapshot_digest_hex = hex_encode(&bounded_snapshot.digest);
    let snapshot_totals = CanonicalSnapshotTotalsReport::from(full_snapshot.totals);
    let bounded_preview = CapabilityBoundedPreviewReport {
        schema_id: "baozi.canonical-scene.bounded-preview.v1",
        bytes: bounded_snapshot.preview.text.len(),
        sha256: sha256_hex(bounded_snapshot.preview.text.as_bytes()),
        items_omitted_by_limit: bounded_snapshot.preview.items_omitted_by_limit,
        bytes_truncated: bounded_snapshot.preview.bytes_truncated,
        items_truncated: bounded_snapshot.preview.items_truncated,
    };
    let source_vector = FixedVectorReport {
        schema_id: GLTF_SOURCE_VECTOR_SCHEMA_ID,
        observed_bytes: vector_observations.source_vector.len(),
        sha256: sha256_hex(&vector_observations.source_vector),
    };
    let mutations = vector_observations
        .mutations
        .into_iter()
        .map(|mutation| CapabilityMutationReport {
            field_class: mutation.field_class,
            source_vector: FixedVectorReport {
                schema_id: GLTF_SOURCE_VECTOR_SCHEMA_ID,
                observed_bytes: mutation.source_vector.len(),
                sha256: sha256_hex(&mutation.source_vector),
            },
            snapshot_digest_hex: hex_encode(&mutation.snapshot_digest),
        })
        .collect();

    let edit_replacement = AuthoredTransform::try_trs(
        Vec3::new(7.0, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .expect("production edit replacement must be a finite authored TRS");
    let edit_input_generation = parts.scene.generation();
    let edit_node = NodeId::new(edit_input_generation, 0);
    let edit_generation_reservations_before = generation_source.load(Ordering::Relaxed);
    let edit_canonical_block_count_before = parts.scene.canonical_block_count();
    let edit_physical_before = account.snapshot();
    let edit_session =
        MeasurementSession::begin().expect("glTF changed edit measurement must start");
    let mut edit_terminal = None;
    let mut edit_physical_before_handoff = None;
    let edit_allocations = allocation_counter::measure(|| {
        let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
            &parts.scene,
            edit_node,
            edit_replacement,
            OptionsDigest::sha256([0; 32]),
        )
        .expect("changed glTF authored bits must enter edit planning") else {
            panic!("changed glTF authored bits cannot classify as unchanged");
        };
        let prepared = pending
            .prepare_with_generation_source(&parts.scene, test_limits(), || {
                SceneGenerationId::reserve_from(&generation_source)
            })
            .expect("exact-source glTF node transform edit must prepare");
        edit_physical_before_handoff = Some(account.snapshot());
        edit_terminal = Some(prepared.handoff());
    });
    let edit_measurement = edit_session
        .finish()
        .expect("glTF changed edit measurement must finish");
    let NodeSceneEditTerminal::Success(output) =
        edit_terminal.expect("glTF changed edit closure must publish a terminal")
    else {
        panic!("valid exact-source glTF node transform edit must publish success");
    };
    let edit_physical_before_handoff = edit_physical_before_handoff
        .expect("changed glTF edit must snapshot its prepared physical account");
    let edit_physical_with_output = account.snapshot();
    let handoff_physical_account_preserved =
        edit_physical_before_handoff == edit_physical_with_output;
    let edit_output_generation = output.scene.generation();
    let edit_generation_reservations_after = generation_source.load(Ordering::Relaxed);
    let edit_canonical_block_count_after = output.scene.canonical_block_count();
    let edit_canonical_block_count_delta = edit_canonical_block_count_after
        .checked_sub(edit_canonical_block_count_before)
        .expect("changed glTF edit cannot remove canonical blocks");
    let edit_output_shape = inspect_gltf_capability_semantics_with_first_transform_for_probe(
        &output.scene,
        edit_replacement,
    );
    let edit_input_shape = inspect_gltf_capability_semantics_for_probe(&parts.scene);
    let replacement_authored_bits_preserved = output
        .scene
        .authored_transform(0)
        .expect("edited glTF authored transform")
        .has_same_authored_bits(edit_replacement);
    let input_authored_bits_preserved = parts
        .scene
        .authored_transform(0)
        .expect("input glTF authored transform after edit")
        .has_same_authored_bits(AuthoredTransform::identity_trs());
    let exact_physical_account_preserved = output
        .scene
        .account()
        .is_same_account(parts.scene.account());
    let generation_changed = edit_output_generation != edit_input_generation;
    let all_subjects_explicitly_unchanged = all_remaps_bind_exact_unchanged(
        output.report.remaps(),
        edit_input_generation,
        edit_output_generation,
    );
    let target_rebind_matches = output
        .report
        .remaps()
        .nodes()
        .try_rebind_unchanged(edit_node)
        .expect("changed edit node remap must remain valid")
        == Some(NodeId::new(edit_output_generation, 0));
    let remap_storage_bytes = output.report.remaps().retained_allocation_bytes();
    let scene_bounds_invalidated =
        output.report.invalidations().scene_bounds() == Some(DerivedAbsenceReason::Invalidated);
    let edit_outcome_evidence_block_count = output.report.outcome_evidence_block_count();
    let edit_evidence = evidence_report(
        edit_outcome_evidence_block_count,
        output.report.ledger_evidence(),
    );
    let input_readable_after_publication = edit_input_shape == canonical_shape;
    let output_shape_matches_input = edit_output_shape == canonical_shape;

    let edit_snapshot_limits = NodeSceneSnapshotLimits::try_new(64 * 1024, 128, 1_000_000)
        .expect("changed glTF edit snapshot limits must be valid");
    let edit_snapshot_session =
        MeasurementSession::begin().expect("glTF changed edit snapshot measurement must start");
    let mut edit_snapshots = None;
    let edit_snapshot_allocations = allocation_counter::measure(|| {
        edit_snapshots = Some((
            parts
                .scene
                .capture_snapshot(edit_snapshot_limits, None)
                .expect("input glTF scene snapshot after edit"),
            output
                .scene
                .capture_snapshot(edit_snapshot_limits, None)
                .expect("edited glTF scene snapshot"),
        ));
    });
    let edit_snapshot_measurement = edit_snapshot_session
        .finish()
        .expect("glTF changed edit snapshot measurement must finish");
    let (input_after_edit_snapshot, output_edit_snapshot) =
        edit_snapshots.expect("changed glTF edit snapshots must be captured");
    let input_after_edit_snapshot_digest_hex = hex_encode(&input_after_edit_snapshot.digest);
    let output_snapshot_digest_hex = hex_encode(&output_edit_snapshot.digest);

    let facade_typed_error_generation_reservations_before =
        generation_source.load(Ordering::Relaxed);
    let facade_typed_error_canonical_block_count_before = parts.scene.canonical_block_count();
    let facade_typed_error_physical_before = account.snapshot();
    parts.scene.install_child_reference_read_fault_for_test(0);
    let facade_typed_error_session =
        MeasurementSession::begin().expect("facade-equivalent typed-error measurement must start");
    let mut facade_typed_error_outcome: Option<FacadeObservationOutcomeV1> = None;
    let facade_typed_error_allocations = allocation_counter::measure(|| {
        facade_typed_error_outcome =
            Some(project_gltf_facade_observation_v1_for_probe(&parts.scene));
    });
    let facade_typed_error_measurement = facade_typed_error_session
        .finish()
        .expect("facade-equivalent typed-error measurement must finish");
    let Err(facade_typed_error) = facade_typed_error_outcome
        .expect("facade-equivalent typed-error projection must produce an outcome")
    else {
        panic!("injected typed facade read failure cannot project success");
    };
    let facade_typed_error_physical_after = account.snapshot();
    let facade_typed_error_generation_reservations_after =
        generation_source.load(Ordering::Relaxed);
    let facade_typed_error_canonical_block_count_after = parts.scene.canonical_block_count();
    assert_eq!(facade_typed_error.schema_id, FACADE_OBSERVATION_SCHEMA_ID);
    assert_eq!(facade_typed_error.error_class, "canonical-typed-view");
    assert_eq!(
        facade_typed_error.reason,
        "injected child-reference read failure",
    );
    assert!(facade_typed_error.reason.len() <= 128);
    assert_eq!(
        facade_typed_error_generation_reservations_after,
        facade_typed_error_generation_reservations_before,
    );
    assert_eq!(
        facade_typed_error_canonical_block_count_after,
        facade_typed_error_canonical_block_count_before,
    );
    assert_eq!(
        facade_typed_error_physical_after,
        facade_typed_error_physical_before,
    );

    let output_scene = output.scene;
    let output_report = output.report;
    let mut output_after_input_release_shape = None;
    let mut output_report_readable_after_input_release = false;
    let input_release_allocations = allocation_counter::measure(|| {
        drop(parts);
        output_report_readable_after_input_release = matches!(
            output_report.ledger_evidence().header().terminal_cause(),
            StoreTerminalCause::Success
        );
        output_after_input_release_shape = Some(
            inspect_gltf_capability_semantics_with_first_transform_for_probe(
                &output_scene,
                edit_replacement,
            ),
        );
    });
    let edit_physical_after_input_release = account.snapshot();
    let output_readable_after_input_release =
        output_after_input_release_shape == Some(canonical_shape);

    let mut output_after_report_release_shape = None;
    let output_report_release_allocations = allocation_counter::measure(|| {
        drop(output_report);
        output_after_report_release_shape = Some(
            inspect_gltf_capability_semantics_with_first_transform_for_probe(
                &output_scene,
                edit_replacement,
            ),
        );
    });
    let edit_physical_after_output_report_release = account.snapshot();
    let output_readable_after_report_release =
        output_after_report_release_shape == Some(canonical_shape);

    let output_scene_release_allocations = allocation_counter::measure(|| drop(output_scene));
    let edit_physical_after_output_scene_release = account.snapshot();

    assert!(replacement_authored_bits_preserved);
    assert!(input_authored_bits_preserved);
    assert!(input_readable_after_publication);
    assert!(output_shape_matches_input);
    assert!(output_report_readable_after_input_release);
    assert_eq!(input_after_edit_snapshot.digest, full_snapshot.digest);
    assert_ne!(output_edit_snapshot.digest, full_snapshot.digest);
    assert!(output_readable_after_input_release);
    assert!(output_readable_after_report_release);
    assert!(exact_physical_account_preserved);
    assert!(handoff_physical_account_preserved);
    assert!(generation_changed);
    assert_eq!(edit_generation_reservations_before, 1);
    assert_eq!(edit_generation_reservations_after, 2);
    assert!(edit_canonical_block_count_delta > 0);
    assert!(all_subjects_explicitly_unchanged);
    assert!(target_rebind_matches);
    assert_eq!(remap_storage_bytes, 0);
    assert!(scene_bounds_invalidated);
    assert!(edit_outcome_evidence_block_count > 0);
    assert!(edit_measurement.new_block_begins() > 0);
    assert!(edit_measurement.inherited_block_begins() > 0);
    assert_eq!(
        edit_measurement.new_block_begins(),
        edit_measurement.payload_backing_allocations(),
    );
    assert!(
        edit_physical_with_output.live_retained_allocations()
            > edit_physical_before.live_retained_allocations(),
    );
    assert!(
        edit_physical_with_output.live_retained_bytes()
            > edit_physical_before.live_retained_bytes(),
    );
    assert!(edit_physical_after_input_release.live_retained_allocations() > 0);
    assert!(edit_physical_after_input_release.live_retained_bytes() > 0);
    assert!(
        edit_physical_after_input_release.live_retained_allocations()
            < edit_physical_with_output.live_retained_allocations(),
    );
    assert!(
        edit_physical_after_input_release.live_retained_bytes()
            < edit_physical_with_output.live_retained_bytes(),
    );
    assert!(edit_physical_after_output_report_release.live_retained_allocations() > 0);
    assert!(edit_physical_after_output_report_release.live_retained_bytes() > 0);
    assert!(
        edit_physical_after_output_report_release.live_retained_allocations()
            < edit_physical_after_input_release.live_retained_allocations(),
    );
    assert!(
        edit_physical_after_output_report_release.live_retained_bytes()
            < edit_physical_after_input_release.live_retained_bytes(),
    );
    assert_eq!(
        edit_physical_after_output_scene_release.live_retained_allocations(),
        0,
    );
    assert_eq!(
        edit_physical_after_output_scene_release.live_retained_bytes(),
        0,
    );

    let release_allocations = allocation_counter::measure(move || {
        drop(full_snapshot);
        drop(bounded_snapshot);
        drop(input_after_edit_snapshot);
        drop(output_edit_snapshot);
    });
    assert_allocator_lifecycle_balanced(
        "canonical glTF capability",
        &[
            &construction_allocations,
            &inspection_allocations,
            &facade_success_allocations,
            &snapshot_allocations,
            &unchanged_view_allocations,
            &edit_allocations,
            &edit_snapshot_allocations,
            &facade_typed_error_allocations,
            &input_release_allocations,
            &output_report_release_allocations,
            &output_scene_release_allocations,
            &release_allocations,
        ],
    );
    let physical_after_release = account.snapshot();
    drop(account);
    let factory_after_account_release = factory.snapshot();

    GltfCapabilityReport {
        entry_point: "backend-free-gltf-shape",
        scenario_id: SCENARIO_ID,
        source_shape: gltf_source_shape_report(vector_observations.source_shape),
        canonical_shape: gltf_canonical_shape_report(canonical_shape),
        source_vector,
        mutations,
        source_generation_reservations: unchanged_generation_reservations_after,
        canonical_block_count,
        outcome_evidence_block_count,
        full_snapshot_digest_hex,
        bounded_snapshot_digest_hex,
        snapshot_totals,
        bounded_preview,
        evidence,
        construction: phase_report(
            CONSTRUCTION_ID,
            construction_measurement,
            construction_allocations,
        ),
        inspection: phase_report(
            INSPECTION_ID,
            inspection_measurement,
            inspection_allocations,
        ),
        facade_equivalent: FacadeEquivalentInspectionReport {
            entry_point: "facade-equivalent-inspection",
            scenario_id: FACADE_EQUIVALENT_ID,
            success: FacadeObservationSuccessReport {
                outcome: "success",
                schema_id: facade_success.schema_id,
                transcript_bytes: facade_success.transcript_bytes,
                transcript_digest_hex: hex_encode(&facade_success.transcript_digest),
                totals: gltf_canonical_shape_report(facade_success.totals),
                reference_closure_verified: facade_success.reference_closure_verified,
                cursor_exhausted: facade_success.cursor_exhausted,
            },
            typed_error: FacadeObservationTypedErrorReport {
                outcome: "typed-error",
                schema_id: facade_typed_error.schema_id,
                error_class: facade_typed_error.error_class,
                reason: facade_typed_error.reason,
                reason_bytes: facade_typed_error.reason.len(),
            },
            success_generation_reservations_before: facade_success_generation_reservations_before,
            success_generation_reservations_after: facade_success_generation_reservations_after,
            success_canonical_block_count_before: facade_success_canonical_block_count_before,
            success_canonical_block_count_after: facade_success_canonical_block_count_after,
            typed_error_generation_reservations_before:
                facade_typed_error_generation_reservations_before,
            typed_error_generation_reservations_after:
                facade_typed_error_generation_reservations_after,
            typed_error_canonical_block_count_before:
                facade_typed_error_canonical_block_count_before,
            typed_error_canonical_block_count_after: facade_typed_error_canonical_block_count_after,
            success_phase: phase_report(
                FACADE_EQUIVALENT_ID,
                facade_success_measurement,
                facade_success_allocations,
            ),
            typed_error_phase: phase_report(
                FACADE_TYPED_ERROR_ID,
                facade_typed_error_measurement,
                facade_typed_error_allocations,
            ),
            success_physical_account: ReadOnlyPhysicalReport {
                before: facade_success_physical_before.into(),
                after: facade_success_physical_after.into(),
            },
            typed_error_physical_account: ReadOnlyPhysicalReport {
                before: facade_typed_error_physical_before.into(),
                after: facade_typed_error_physical_after.into(),
            },
        },
        snapshot: phase_report(SNAPSHOT_ID, snapshot_measurement, snapshot_allocations),
        unchanged_view: GltfUnchangedViewReport {
            observation: gltf_canonical_shape_report(unchanged_observation),
            repeated_observation_matches: unchanged_observation == repeated_unchanged_observation,
            generation_preserved: unchanged_generation_preserved,
            generation_reservations_before: unchanged_generation_reservations_before,
            generation_reservations_after: unchanged_generation_reservations_after,
            canonical_block_count_before: unchanged_canonical_block_count_before,
            canonical_block_count_after: unchanged_canonical_block_count_after,
            phase: phase_report(
                UNCHANGED_VIEW_ID,
                unchanged_view_measurement,
                unchanged_view_allocations,
            ),
            physical_account: ReadOnlyPhysicalReport {
                before: unchanged_physical_before.into(),
                after: unchanged_physical_after.into(),
            },
        },
        changed_scene_edit: GltfChangedSceneEditReport {
            entry_point: "changed-scene-edit",
            scenario_id: CHANGED_SCENE_EDIT_ID,
            output_shape: gltf_canonical_shape_report(edit_output_shape),
            replacement_authored_bits_preserved,
            input_authored_bits_preserved,
            input_readable_after_publication,
            output_report_readable_after_input_release,
            output_readable_after_input_release,
            output_readable_after_report_release,
            exact_physical_account_preserved,
            handoff_physical_account_preserved,
            generation_changed,
            generation_reservations_before: edit_generation_reservations_before,
            generation_reservations_after: edit_generation_reservations_after,
            canonical_block_count_before: edit_canonical_block_count_before,
            canonical_block_count_after: edit_canonical_block_count_after,
            all_subjects_explicitly_unchanged,
            target_rebind_matches,
            remap_storage_bytes,
            scene_bounds_invalidated,
            outcome_evidence_block_count: edit_outcome_evidence_block_count,
            evidence: edit_evidence,
            phase: phase_report(CHANGED_SCENE_EDIT_ID, edit_measurement, edit_allocations),
            input_snapshot_digest_hex: input_after_edit_snapshot_digest_hex,
            output_snapshot_digest_hex,
            snapshot: phase_report(
                EDIT_SNAPSHOT_ID,
                edit_snapshot_measurement,
                edit_snapshot_allocations,
            ),
            input_release: release_report(EDIT_INPUT_RELEASE_ID, input_release_allocations),
            output_report_release: release_report(
                EDIT_REPORT_RELEASE_ID,
                output_report_release_allocations,
            ),
            output_scene_release: release_report(
                EDIT_OUTPUT_RELEASE_ID,
                output_scene_release_allocations,
            ),
            physical_account: GltfChangedSceneEditPhysicalReport {
                before: edit_physical_before.into(),
                before_handoff: edit_physical_before_handoff.into(),
                with_output: edit_physical_with_output.into(),
                after_input_release: edit_physical_after_input_release.into(),
                after_output_report_release: edit_physical_after_output_report_release.into(),
                after_output_scene_release: edit_physical_after_output_scene_release.into(),
            },
        },
        release: release_report(RELEASE_ID, release_allocations),
        physical_account: CapabilityPhysicalReport {
            baseline: physical_baseline.into(),
            retained: physical_retained.into(),
            after_inspection: physical_after_inspection.into(),
            after_snapshot: physical_after_snapshot.into(),
            after_release: physical_after_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}

fn all_remaps_bind_exact_unchanged(
    remaps: &CanonicalRemapSet,
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
) -> bool {
    macro_rules! exact_unchanged {
        ($remap:expr) => {{
            let remap = $remap;
            remap.input_generation() == input_generation
                && remap.output_generation() == output_generation
                && remap.change() == RemapChange::ExplicitlyUnchanged
        }};
    }

    exact_unchanged!(remaps.nodes())
        && exact_unchanged!(remaps.mesh_instances())
        && exact_unchanged!(remaps.logical_meshes())
        && exact_unchanged!(remaps.primitives())
        && exact_unchanged!(remaps.morph_targets())
        && exact_unchanged!(remaps.materials())
        && exact_unchanged!(remaps.images())
        && exact_unchanged!(remaps.samplers())
        && exact_unchanged!(remaps.textures())
        && exact_unchanged!(remaps.skins())
        && exact_unchanged!(remaps.animation_clips())
        && exact_unchanged!(remaps.cameras())
        && exact_unchanged!(remaps.lights())
        && exact_unchanged!(remaps.vertices())
        && exact_unchanged!(remaps.corners())
        && exact_unchanged!(remaps.faces())
        && exact_unchanged!(remaps.instances())
}

fn gltf_source_shape_report(shape: GltfCapabilitySourceShapeObservation) -> GltfSourceShapeReport {
    GltfSourceShapeReport {
        node_count: shape.node_count,
        root_count: shape.root_count,
        image_count: shape.image_count,
        image_payload_bytes: shape.image_payload_bytes,
        media_type_bytes: shape.media_type_bytes,
        sampler_count: shape.sampler_count,
        texture_count: shape.texture_count,
        material_count: shape.material_count,
        material_binding_count: shape.material_binding_count,
        logical_mesh_count: shape.logical_mesh_count,
        primitive_count: shape.primitive_count,
        attribute_count: shape.attribute_count,
        mesh_instance_count: shape.mesh_instance_count,
        skin_count: shape.skin_count,
        skin_joint_count: shape.skin_joint_count,
        morph_definition_count: shape.morph_definition_count,
        morph_target_count: shape.morph_target_count,
        morph_delta_count: shape.morph_delta_count,
        influence_row_count: shape.influence_row_count,
        influence_count: shape.influence_count,
        animation_clip_count: shape.animation_clip_count,
        animation_channel_count: shape.animation_channel_count,
        animation_key_count: shape.animation_key_count,
    }
}

fn gltf_canonical_shape_report(
    shape: GltfCapabilityCanonicalShapeObservation,
) -> GltfCanonicalShapeReport {
    GltfCanonicalShapeReport {
        node_count: shape.node_count,
        root_count: shape.root_count,
        edge_count: shape.edge_count,
        image_count: shape.image_count,
        sampler_count: shape.sampler_count,
        texture_count: shape.texture_count,
        material_count: shape.material_count,
        material_binding_count: shape.material_binding_count,
        logical_mesh_count: shape.logical_mesh_count,
        primitive_count: shape.primitive_count,
        attribute_count: shape.attribute_count,
        mesh_instance_count: shape.mesh_instance_count,
        skin_count: shape.skin_count,
        skin_joint_count: shape.skin_joint_count,
        logical_morph_count: shape.logical_morph_count,
        morph_definition_count: shape.morph_definition_count,
        primitive_deformation_count: shape.primitive_deformation_count,
        morph_target_count: shape.morph_target_count,
        morph_delta_count: shape.morph_delta_count,
        influence_row_count: shape.influence_row_count,
        influence_count: shape.influence_count,
        animation_clip_count: shape.animation_clip_count,
        animation_channel_count: shape.animation_channel_count,
        animation_key_count: shape.animation_key_count,
    }
}

pub(super) fn collect_scale_scenarios() -> Vec<ScaleScenarioReport> {
    SCALE_DESCRIPTORS
        .into_iter()
        .map(collect_scale_scenario)
        .collect()
}

fn collect_scale_scenario(descriptor: ScaleDescriptor) -> ScaleScenarioReport {
    const SCALE_ACCOUNT_MAX_LIVE_ALLOCATIONS: u64 = 512;

    let factory = test_factory(1);
    let account = test_account_with_allocation_limit(&factory, SCALE_ACCOUNT_MAX_LIVE_ALLOCATIONS);
    let foreign_nonce = AtomicU64::new(0);
    let target_nonce = AtomicU64::new(0);

    let foreign_prepared = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("foreign complex fixture operation must start")
        .prepare_complex_fixture_for_test(None, || SceneGenerationId::reserve_from(&foreign_nonce))
        .expect("foreign complex fixture must prepare");
    let foreign = match foreign_prepared.handoff() {
        ComplexFixtureTerminal::Success(parts) => parts,
        ComplexFixtureTerminal::RichFailure(_) => {
            panic!("foreign complex fixture must validate")
        }
        ComplexFixtureTerminal::MinimalFallback(_) => {
            panic!("foreign complex fixture must not fall back")
        }
    };
    let foreign_canonical_block_count = foreign.fixture.canonical_block_count();
    let physical_foreign_baseline = account.snapshot();

    let construction_session =
        MeasurementSession::begin().expect("complex construction measurement must start");
    let mut target_terminal = None;
    let construction_allocations = allocation_counter::measure(|| {
        let prepared = SceneConstructionOperation::with_account_for_scaled_complex_fixture(
            &account,
            test_limits(),
            descriptor.image_count,
        )
        .expect("scaled complex operation must start")
        .prepare(None, || SceneGenerationId::reserve_from(&target_nonce))
        .expect("scaled complex fixture must prepare");
        target_terminal = Some(prepared.handoff());
    });
    let construction_measurement = construction_session
        .finish()
        .expect("complex construction measurement must finish");
    let target = match target_terminal.expect("complex construction closure must set a terminal") {
        ComplexFixtureTerminal::Success(parts) => parts,
        ComplexFixtureTerminal::RichFailure(_) => {
            panic!("scaled complex fixture must validate")
        }
        ComplexFixtureTerminal::MinimalFallback(_) => {
            panic!("scaled complex fixture must not fall back")
        }
    };
    let physical_target_retained = account.snapshot();

    let canonical_block_count = target.fixture.canonical_block_count();

    let transcript = target
        .fixture
        .semantic_transcript()
        .expect("complex typed views must yield a semantic transcript");
    assert!(transcript.is_ascii());
    let preview_bytes = transcript.len().min(COMPLEX_PREVIEW_LIMIT_BYTES);
    let semantic_transcript = SemanticTranscriptReport {
        schema_id: COMPLEX_SEMANTIC_TRANSCRIPT_SCHEMA_ID,
        observed_bytes: transcript.len(),
        sha256: sha256_hex(transcript.as_bytes()),
        bounded_preview: transcript[..preview_bytes].to_owned(),
    };

    let inspection_session =
        MeasurementSession::begin().expect("complex typed inspection measurement must start");
    let mut successful_views = false;
    let mut rejected_foreign_directory = false;
    let inspection_allocations = allocation_counter::measure(|| {
        successful_views = target
            .fixture
            .inspect_fixed_image_views(descriptor.image_count)
            .is_some();
        rejected_foreign_directory = target.fixture.rejects_foreign_directory(&foreign.fixture);
    });
    let inspection_measurement = inspection_session
        .finish()
        .expect("complex typed inspection measurement must finish");
    let physical_after_typed_inspection = account.snapshot();

    let evidence = evidence_report(
        target.report.outcome_evidence_block_count(),
        target.report.ledger_evidence(),
    );
    let target_release_allocations = measure_release(target);
    assert_allocator_lifecycle_balanced(
        "scaled complex target",
        &[&construction_allocations, &target_release_allocations],
    );
    let physical_after_target_release = account.snapshot();
    let foreign_release_allocations = measure_release(foreign);
    let physical_after_foreign_release = account.snapshot();

    drop(account);
    let factory_after_account_release = factory.snapshot();

    ScaleScenarioReport {
        scenario_id: descriptor.scenario_id,
        image_count: descriptor.image_count,
        canonical_block_count,
        foreign_canonical_block_count,
        target_generation_reservations: target_nonce.load(Ordering::Relaxed),
        foreign_generation_reservations: foreign_nonce.load(Ordering::Relaxed),
        typed_inspection_succeeded: successful_views,
        foreign_directory_rejected: rejected_foreign_directory,
        semantic_transcript,
        evidence,
        construction: phase_report(
            descriptor.construction_scenario_id,
            construction_measurement,
            construction_allocations,
        ),
        typed_inspection: phase_report(
            descriptor.inspection_scenario_id,
            inspection_measurement,
            inspection_allocations,
        ),
        target_release: release_report(
            descriptor.target_release_scenario_id,
            target_release_allocations,
        ),
        foreign_release: release_report(
            descriptor.foreign_release_scenario_id,
            foreign_release_allocations,
        ),
        physical_account: ScalePhysicalReport {
            foreign_baseline: physical_foreign_baseline.into(),
            target_retained: physical_target_retained.into(),
            after_typed_inspection: physical_after_typed_inspection.into(),
            after_target_release: physical_after_target_release.into(),
            after_foreign_release: physical_after_foreign_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}

pub(super) fn collect_inherited_hot_leaf_scenario() -> InheritedHotLeafReport {
    const FRESH_CONSTRUCTION_ID: &str = "inherited-hot-leaf-fresh-construction-v1";
    const FRESH_VIEW_ID: &str = "inherited-hot-leaf-fresh-view-v1";
    const DERIVED_CONSTRUCTION_ID: &str = "inherited-hot-leaf-derived-construction-v1";
    const DERIVED_VIEW_ID: &str = "inherited-hot-leaf-derived-view-v1";
    const AFTER_PARENT_VIEW_ID: &str = "inherited-hot-leaf-after-parent-release-view-v1";

    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let physical_baseline = account.snapshot();

    let fresh_session = MeasurementSession::begin().expect("fresh share measurement must start");
    let mut fresh_terminal = None;
    let fresh_allocations = allocation_counter::measure(|| {
        let prepared = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("fresh share operation must start")
            .prepare_fresh_two_node_with_generation_source(authored_fixture(), || {
                SceneGenerationId::reserve_from(&nonce)
            })
            .expect("fresh share parent must prepare");
        fresh_terminal = Some(prepared.handoff());
    });
    let fresh_measurement = fresh_session
        .finish()
        .expect("fresh share measurement must finish");
    let (parent, fresh_success_report) =
        match fresh_terminal.expect("fresh share closure must set a terminal") {
            SceneConstructionTerminal::Success(parts) => (parts.scene, parts.report),
            SceneConstructionTerminal::RichFailure(_) => {
                panic!("fresh share parent must validate")
            }
            SceneConstructionTerminal::MinimalFallback(_) => {
                panic!("fresh share parent must not fall back")
            }
        };
    let physical_fresh_handoff = account.snapshot();
    let fresh_evidence = evidence_report(
        fresh_success_report.outcome_evidence_block_count(),
        fresh_success_report.ledger_evidence(),
    );
    let fresh_report_release_allocations = measure_release(fresh_success_report);
    let physical_fresh_scene_only = account.snapshot();

    let fresh_view_session =
        MeasurementSession::begin().expect("fresh hot-leaf view measurement must start");
    let mut fresh_matrix_bits = None;
    let fresh_view_allocations = allocation_counter::measure(|| {
        fresh_matrix_bits = parent.authored_matrix_bits(1);
    });
    let fresh_view_measurement = fresh_view_session
        .finish()
        .expect("fresh hot-leaf view measurement must finish");
    let fresh_matrix_bits = fresh_matrix_bits.expect("fresh hot leaf must be readable");

    let derived_session =
        MeasurementSession::begin().expect("derived share measurement must start");
    let mut derived_terminal = None;
    let derived_allocations = allocation_counter::measure(|| {
        let prepared = SceneConstructionOperation::derived_from_exact_input(&parent, test_limits())
            .expect("derived share operation must start")
            .prepare_derived_two_node_with_generation_source(|| {
                SceneGenerationId::reserve_from(&nonce)
            })
            .expect("derived share scene must prepare");
        derived_terminal = Some(prepared.handoff());
    });
    let derived_measurement = derived_session
        .finish()
        .expect("derived share measurement must finish");
    let (derived, derived_success_report) =
        match derived_terminal.expect("derived share closure must set a terminal") {
            SceneConstructionTerminal::Success(parts) => (parts.scene, parts.report),
            SceneConstructionTerminal::RichFailure(_) => {
                panic!("derived share scene must validate")
            }
            SceneConstructionTerminal::MinimalFallback(_) => {
                panic!("derived share scene must not fall back")
            }
        };
    let physical_derived_handoff = account.snapshot();
    let derived_evidence = evidence_report(
        derived_success_report.outcome_evidence_block_count(),
        derived_success_report.ledger_evidence(),
    );
    let derived_report_release_allocations = measure_release(derived_success_report);
    let physical_parent_and_derived = account.snapshot();

    let derived_view_session =
        MeasurementSession::begin().expect("derived hot-leaf view measurement must start");
    let mut derived_matrix_bits = None;
    let derived_view_allocations = allocation_counter::measure(|| {
        derived_matrix_bits = derived.authored_matrix_bits(1);
    });
    let derived_view_measurement = derived_view_session
        .finish()
        .expect("derived hot-leaf view measurement must finish");
    let derived_matrix_bits = derived_matrix_bits.expect("derived hot leaf must be readable");

    let fresh_canonical_block_count = parent.canonical_block_count();
    let derived_canonical_block_count = derived.canonical_block_count();
    let parent_release_allocations = measure_release(parent);
    let physical_after_parent_release = account.snapshot();

    let after_parent_view_session =
        MeasurementSession::begin().expect("post-parent hot-leaf view measurement must start");
    let mut after_parent_matrix_bits = None;
    let after_parent_view_allocations = allocation_counter::measure(|| {
        after_parent_matrix_bits = derived.authored_matrix_bits(1);
    });
    let after_parent_view_measurement = after_parent_view_session
        .finish()
        .expect("post-parent hot-leaf view measurement must finish");
    let after_parent_matrix_bits =
        after_parent_matrix_bits.expect("derived hot leaf must outlive its parent");

    let derived_release_allocations = measure_release(derived);
    assert_allocator_lifecycle_balanced(
        "fresh and derived shared canonical stores",
        &[
            &fresh_allocations,
            &derived_allocations,
            &fresh_report_release_allocations,
            &derived_report_release_allocations,
            &parent_release_allocations,
            &derived_release_allocations,
        ],
    );
    let physical_after_derived_release = account.snapshot();
    drop(account);
    let factory_after_account_release = factory.snapshot();

    InheritedHotLeafReport {
        scenario_id: "fresh-derived-exact-input-inherited-hot-leaf-v1",
        generation_reservations: nonce.load(Ordering::Relaxed),
        fresh: ShareGenerationReport {
            construction: phase_report(FRESH_CONSTRUCTION_ID, fresh_measurement, fresh_allocations),
            evidence: fresh_evidence,
            canonical_block_count: fresh_canonical_block_count,
        },
        derived: ShareGenerationReport {
            construction: phase_report(
                DERIVED_CONSTRUCTION_ID,
                derived_measurement,
                derived_allocations,
            ),
            evidence: derived_evidence,
            canonical_block_count: derived_canonical_block_count,
        },
        fresh_hot_leaf: HotLeafObservationReport {
            phase: phase_report(
                FRESH_VIEW_ID,
                fresh_view_measurement,
                fresh_view_allocations,
            ),
            matrix_bits: fresh_matrix_bits,
        },
        derived_hot_leaf: HotLeafObservationReport {
            phase: phase_report(
                DERIVED_VIEW_ID,
                derived_view_measurement,
                derived_view_allocations,
            ),
            matrix_bits: derived_matrix_bits,
        },
        derived_after_parent_release_hot_leaf: HotLeafObservationReport {
            phase: phase_report(
                AFTER_PARENT_VIEW_ID,
                after_parent_view_measurement,
                after_parent_view_allocations,
            ),
            matrix_bits: after_parent_matrix_bits,
        },
        fresh_report_release: release_report(
            "inherited-hot-leaf-fresh-report-release-v1",
            fresh_report_release_allocations,
        ),
        derived_report_release: release_report(
            "inherited-hot-leaf-derived-report-release-v1",
            derived_report_release_allocations,
        ),
        parent_release: release_report(
            "inherited-hot-leaf-parent-release-v1",
            parent_release_allocations,
        ),
        derived_release: release_report(
            "inherited-hot-leaf-derived-release-v1",
            derived_release_allocations,
        ),
        physical_account: SharePhysicalReport {
            baseline: physical_baseline.into(),
            fresh_handoff: physical_fresh_handoff.into(),
            fresh_scene_only: physical_fresh_scene_only.into(),
            derived_handoff: physical_derived_handoff.into(),
            parent_and_derived_scenes: physical_parent_and_derived.into(),
            after_parent_release: physical_after_parent_release.into(),
            after_derived_release: physical_after_derived_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}

pub(super) fn collect_validation_faults() -> Vec<ValidationFaultReport> {
    VALIDATION_FAULT_DESCRIPTORS
        .into_iter()
        .map(collect_validation_fault)
        .collect()
}

pub(super) fn collect_minimal_fallback() -> MinimalFallbackReport {
    const SCENARIO_ID: &str = "canonical-node-minimal-fallback-v1";
    const CONSTRUCTION_ID: &str = "canonical-node-minimal-fallback-construction-v1";
    const HANDOFF_ID: &str = "canonical-node-minimal-fallback-handoff-v1";
    const RELEASE_ID: &str = "canonical-node-minimal-fallback-release-v1";
    const FALLBACK_OUTPUT_COUNT: usize = 1;

    let factory = test_factory(1);
    let account = test_account(&factory);
    let physical_baseline = account.snapshot();

    let construction_session =
        MeasurementSession::begin().expect("minimal fallback measurement must start");
    let mut prepared = None;
    let construction_allocations = allocation_counter::measure(|| {
        prepared = Some(build_minimal_fallback_terminal_for_probe(&account));
    });
    let construction_measurement = construction_session
        .finish()
        .expect("minimal fallback measurement must finish");
    let physical_retained = account.snapshot();

    let mut terminal = None;
    let handoff_allocations = allocation_counter::measure(|| {
        terminal = Some(
            prepared
                .take()
                .expect("minimal fallback construction must produce a prepared terminal")
                .handoff(),
        );
    });
    let physical_after_handoff = account.snapshot();
    assert_eq!(physical_after_handoff, physical_retained);
    assert_zero_allocation_handoff("minimal fallback", &handoff_allocations);

    let fallback = match terminal.expect("minimal fallback handoff must set a terminal") {
        NodeSceneConstructionTerminal::MinimalFallback(fallback) => fallback,
        NodeSceneConstructionTerminal::Success(_) => {
            panic!("a clean validator panic must not publish canonical data")
        }
        NodeSceneConstructionTerminal::RichFailure(_) => {
            panic!("a clean validator panic must terminate as minimal fallback")
        }
    };
    assert!(fallback.account().is_same_account(&account));
    let fallback_code = fallback.fallback_code();
    let ordinary_dimension = fallback
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("minimal fallback evidence must contain the ordinary dimension");
    assert_eq!(
        ordinary_dimension.published_outcome_evidence(),
        u64::try_from(FALLBACK_OUTPUT_COUNT)
            .expect("fallback output count must fit u64")
            .checked_add(1)
            .expect("the ledger snapshot must fit the publication count"),
        "ledger publication evidence must cover the fallback output and frozen ledger snapshot",
    );
    let evidence = evidence_report(0, fallback.ledger_evidence());

    let release_allocations = measure_release(fallback);
    assert_allocator_lifecycle_balanced(
        "minimal fallback",
        &[
            &construction_allocations,
            &handoff_allocations,
            &release_allocations,
        ],
    );
    let physical_after_release = account.snapshot();
    assert_eq!(physical_after_release.live_retained_bytes(), 0);
    assert_eq!(physical_after_release.live_retained_allocations(), 0);
    drop(account);
    let factory_after_account_release = factory.snapshot();
    assert_eq!(
        factory_after_account_release.live_account_control_bytes(),
        0
    );
    assert_eq!(factory_after_account_release.live_accounts(), 0);

    MinimalFallbackReport {
        scenario_id: SCENARIO_ID,
        fallback_code,
        fallback_output_count: FALLBACK_OUTPUT_COUNT,
        evidence,
        construction: phase_report(
            CONSTRUCTION_ID,
            construction_measurement,
            construction_allocations,
        ),
        handoff: handoff_report(HANDOFF_ID, handoff_allocations),
        release: release_report(RELEASE_ID, release_allocations),
        physical_account: FaultPhysicalReport {
            baseline: physical_baseline.into(),
            retained: physical_retained.into(),
            after_handoff: physical_after_handoff.into(),
            after_release: physical_after_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}

pub(super) fn collect_pending_scene_activation() -> PendingSceneActivationReport {
    const ENTRY_POINT: &str = "pending-scene-activation";
    const SCENARIO_ID: &str = "canonical-node-pending-scene-activation-v1";
    const CONSTRUCTION_ID: &str = "canonical-node-pending-scene-activation-construction-v1";
    const CANDIDATE_RELEASE_ID: &str =
        "canonical-node-pending-scene-activation-candidate-release-v1";
    let factory = test_factory(1);
    let account = test_account(&factory);
    let physical_baseline = account.snapshot();
    assert_eq!(physical_baseline.live_retained_bytes(), 0);
    assert_eq!(physical_baseline.live_retained_allocations(), 0);
    assert!(!physical_baseline.poisoned());

    let construction_session =
        MeasurementSession::begin().expect("pending activation measurement must start");
    let mut transition = None;
    let construction_allocations = allocation_counter::measure(|| {
        transition = Some(
            SceneConstructionOperation::with_account_for_activation(&account, test_limits())
                .expect("pending activation operation must start")
                .activate_fresh_two_node(authored_fixture())
                .expect("pending activation must remain contained"),
        );
    });
    let construction_measurement = construction_session
        .finish()
        .expect("pending activation measurement must finish");
    let candidate = match transition.expect("pending activation closure must set a transition") {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(prepared) => match prepared.handoff() {
            SceneConstructionTerminal::Success(_) => {
                panic!("pending activation must not publish before joint commit")
            }
            SceneConstructionTerminal::RichFailure(_) => {
                panic!("valid pending activation unexpectedly reached rich failure")
            }
            SceneConstructionTerminal::MinimalFallback(fallback) => {
                panic!(
                    "valid pending activation unexpectedly reached minimal fallback: code={:#010x}, cause={:?}",
                    fallback.fallback_code(),
                    fallback.ledger_evidence().header().terminal_cause(),
                )
            }
        },
    };

    let destination_counts = candidate.destination_counts();
    let canonical_block_count = destination_counts.canonical_blocks();
    let outcome_evidence_block_count = destination_counts.outcome_evidence_blocks();
    let new_block_count = usize::try_from(construction_measurement.new_block_begins())
        .expect("pending activation new-block count must fit usize");
    assert_eq!(
        new_block_count,
        canonical_block_count
            .checked_add(outcome_evidence_block_count)
            .expect("validated destination counts must fit usize"),
        "physical new-block count must equal the validated destination closure",
    );
    let directory_identity_preserved = candidate.directory() == candidate.refs().directory;

    assert!(canonical_block_count > 0);
    assert!(outcome_evidence_block_count > 0);
    assert_eq!(
        construction_measurement.new_block_begins(),
        u64::try_from(new_block_count).expect("new block count must fit u64"),
    );
    assert_eq!(
        construction_measurement.new_block_seals(),
        u64::try_from(new_block_count).expect("new block count must fit u64"),
    );
    assert_eq!(
        construction_measurement.new_slot_publications(),
        u64::try_from(new_block_count).expect("new block count must fit u64"),
    );
    assert_eq!(construction_measurement.inherited_block_begins(), 0);
    assert_eq!(construction_measurement.inherited_block_seals(), 0);
    assert_eq!(construction_measurement.inherited_slot_publications(), 0);
    assert_eq!(
        construction_measurement.payload_backing_allocations(),
        construction_measurement.new_block_begins(),
    );
    assert!(construction_measurement.semantic_cell_writes() > 0);
    assert!(construction_measurement.reference_writes() > 0);
    let derived_producer_events = construction_measurement
        .semantic_cell_writes()
        .checked_add(construction_measurement.reference_writes())
        .and_then(|total| total.checked_add(construction_measurement.block_begins()))
        .and_then(|total| total.checked_add(construction_measurement.block_seals()))
        .and_then(|total| total.checked_add(construction_measurement.slot_publications()))
        .expect("pending activation producer event total must fit u64");
    assert_eq!(
        construction_measurement.producer_events(),
        derived_producer_events,
    );
    assert_eq!(construction_measurement.element_decodes(), 0);
    assert_eq!(construction_measurement.completed_lookups(), 0);
    assert_eq!(construction_measurement.rejected_lookups(), 0);
    assert_eq!(construction_measurement.lookup_events(), 0);
    assert_eq!(construction_measurement.max_lookup_events(), 0);
    assert_eq!(construction_measurement.member_probes(), 0);
    assert_eq!(construction_measurement.max_member_probes_per_lookup(), 0);
    assert!(directory_identity_preserved);

    let physical_candidate_retained = account.snapshot();
    assert!(physical_candidate_retained.live_retained_bytes() > 0);
    assert!(physical_candidate_retained.live_retained_allocations() > 0);
    assert!(!physical_candidate_retained.poisoned());

    let candidate_release_allocations = measure_release(candidate);
    assert_allocator_lifecycle_balanced(
        "pending scene activation",
        &[&construction_allocations, &candidate_release_allocations],
    );
    let physical_after_candidate_release = account.snapshot();
    assert_eq!(physical_after_candidate_release.live_retained_bytes(), 0);
    assert_eq!(
        physical_after_candidate_release.live_retained_allocations(),
        0,
    );
    assert!(!physical_after_candidate_release.poisoned());

    drop(account);
    let factory_after_account_release = factory.snapshot();
    assert_eq!(
        factory_after_account_release.live_account_control_bytes(),
        0,
    );
    assert_eq!(factory_after_account_release.live_accounts(), 0);

    PendingSceneActivationReport {
        entry_point: ENTRY_POINT,
        scenario_id: SCENARIO_ID,
        canonical_block_count,
        outcome_evidence_block_count,
        new_block_count,
        directory_identity_preserved,
        construction: phase_report(
            CONSTRUCTION_ID,
            construction_measurement,
            construction_allocations,
        ),
        candidate_release: release_report(CANDIDATE_RELEASE_ID, candidate_release_allocations),
        physical_account: PendingSceneActivationPhysicalReport {
            baseline: physical_baseline.into(),
            candidate_retained: physical_candidate_retained.into(),
            after_candidate_release: physical_after_candidate_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}

pub(super) fn collect_protected_terminal_activation() -> ProtectedTerminalActivationReport {
    const SCENARIO_ID: &str = "canonical-node-protected-terminal-activation-v1";
    const CONSTRUCTION_ID: &str = "canonical-node-protected-terminal-construction-v1";
    const HANDOFF_ID: &str = "canonical-node-protected-terminal-handoff-v1";
    const RELEASE_ID: &str = "canonical-node-protected-terminal-release-v1";

    let factory = test_factory(1);
    let account = test_account(&factory);
    let physical_baseline = account.snapshot();

    let construction_session =
        MeasurementSession::begin().expect("protected-terminal measurement must start");
    let mut prepared = None;
    let construction_allocations = allocation_counter::measure(|| {
        prepared = Some(build_protected_terminal_activation_for_probe(&account));
    });
    let construction_measurement = construction_session
        .finish()
        .expect("protected-terminal measurement must finish");
    let physical_retained = account.snapshot();

    let mut terminal = None;
    let handoff_allocations = allocation_counter::measure(|| {
        terminal = Some(
            prepared
                .take()
                .expect("protected-terminal construction must produce a prepared terminal")
                .handoff(),
        );
    });
    let physical_after_handoff = account.snapshot();
    assert_eq!(physical_after_handoff, physical_retained);
    assert_zero_allocation_handoff("protected-terminal rich failure", &handoff_allocations);

    let failure = match terminal.expect("protected-terminal handoff must set a terminal") {
        NodeSceneConstructionTerminal::RichFailure(failure) => failure,
        NodeSceneConstructionTerminal::Success(_) => {
            panic!("ordinary work exhaustion must not publish canonical data")
        }
        NodeSceneConstructionTerminal::MinimalFallback(_) => {
            panic!("ordinary work exhaustion must preserve rich evidence")
        }
    };
    let terminal_work = failure
        .ledger_evidence()
        .dimension_by_semantic(FAILURE_TERMINAL_WORK_DIMENSION)
        .expect("protected-terminal evidence must contain its dedicated dimension");
    assert!(terminal_work.attempted() > 0);
    assert_eq!(terminal_work.attempted(), terminal_work.consumed());
    let retained_evidence_blocks = failure.outcome_evidence_block_count();
    let evidence = evidence_report(retained_evidence_blocks, failure.ledger_evidence());

    let release_allocations = measure_release(failure);
    assert_allocator_lifecycle_balanced(
        "protected-terminal rich failure",
        &[
            &construction_allocations,
            &handoff_allocations,
            &release_allocations,
        ],
    );
    let physical_after_release = account.snapshot();
    assert_eq!(physical_after_release.live_retained_bytes(), 0);
    assert_eq!(physical_after_release.live_retained_allocations(), 0);
    drop(account);
    let factory_after_account_release = factory.snapshot();
    assert_eq!(factory_after_account_release.live_accounts(), 0);

    ProtectedTerminalActivationReport {
        scenario_id: SCENARIO_ID,
        evidence,
        construction: phase_report(
            CONSTRUCTION_ID,
            construction_measurement,
            construction_allocations,
        ),
        handoff: handoff_report(HANDOFF_ID, handoff_allocations),
        release: release_report(RELEASE_ID, release_allocations),
        physical_account: FaultPhysicalReport {
            baseline: physical_baseline.into(),
            retained: physical_retained.into(),
            after_handoff: physical_after_handoff.into(),
            after_release: physical_after_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}

fn collect_validation_fault(descriptor: ValidationFaultDescriptor) -> ValidationFaultReport {
    const FAULT_IMAGE_COUNT: usize = 14;

    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let physical_baseline = account.snapshot();

    let construction_session =
        MeasurementSession::begin().expect("validation fault measurement must start");
    let mut terminal = None;
    let allocations = allocation_counter::measure(|| {
        let prepared = SceneConstructionOperation::with_account_for_scaled_complex_fixture(
            &account,
            test_limits(),
            FAULT_IMAGE_COUNT,
        )
        .expect("validation fault operation must start")
        .prepare(Some(descriptor.fault), || {
            SceneGenerationId::reserve_from(&nonce)
        })
        .expect("validation fault must prepare terminal evidence");
        terminal = Some(prepared.handoff());
    });
    let measurement = construction_session
        .finish()
        .expect("validation fault measurement must finish");
    let physical_retained = account.snapshot();
    let failure = match terminal.expect("validation fault closure must set a terminal") {
        ComplexFixtureTerminal::RichFailure(failure) => failure,
        ComplexFixtureTerminal::Success(_) => {
            panic!("validation fault must not publish canonical data")
        }
        ComplexFixtureTerminal::MinimalFallback(_) => {
            panic!("validation fault must preserve rich evidence")
        }
    };
    let physical_after_handoff = account.snapshot();
    let evidence_count = failure.outcome_evidence_block_count();
    let evidence = evidence_report(evidence_count, failure.ledger_evidence());
    let release_allocations = measure_release(failure);
    assert_allocator_lifecycle_balanced(
        "validation fault rich failure",
        &[&allocations, &release_allocations],
    );
    let physical_after_release = account.snapshot();
    drop(account);
    let factory_after_account_release = factory.snapshot();
    let mutation_transcript = descriptor.fault.mutation_transcript();

    ValidationFaultReport {
        scenario_id: descriptor.scenario_id,
        field_class: descriptor.field_class,
        mutation_schema_id: "baozi.complex-fixture.mutation-transcript.v1",
        mutation_transcript_bytes: mutation_transcript.len(),
        mutation_transcript_sha256: sha256_hex(mutation_transcript.as_bytes()),
        image_count: FAULT_IMAGE_COUNT,
        generation_reservations: nonce.load(Ordering::Relaxed),
        evidence,
        construction: phase_report(descriptor.scenario_id, measurement, allocations),
        release: release_report(descriptor.release_scenario_id, release_allocations),
        physical_account: FaultPhysicalReport {
            baseline: physical_baseline.into(),
            retained: physical_retained.into(),
            after_handoff: physical_after_handoff.into(),
            after_release: physical_after_release.into(),
        },
        factory_after_account_release: factory_after_account_release.into(),
    }
}
