use std::sync::atomic::{AtomicU64, Ordering};

use allocation_counter::AllocationInfo;
use baozi_operation::__u4c::MeasurementSession;

use crate::identity::SceneGenerationId;

use super::super::super::construction::node_scene::{
    OBJ_SOURCE_VECTOR_SCHEMA_ID, ObjCapabilityCanonicalShapeObservation,
    ObjCapabilitySourceShapeObservation, build_obj_capability_scene_for_probe,
    capture_obj_capability_snapshots_for_probe, collect_obj_capability_vectors_for_probe,
    inspect_obj_capability_semantics_for_probe,
};
use super::super::super::construction::{
    COMPLEX_SEMANTIC_TRANSCRIPT_SCHEMA_ID, ComplexFixtureFault, ComplexFixtureTerminal,
    SceneConstructionOperation, SceneConstructionTerminal,
};
use super::super::{
    authored_fixture, test_account, test_account_with_allocation_limit, test_factory, test_limits,
};
use super::report::{
    FaultPhysicalReport, FixedVectorReport, HotLeafObservationReport, InheritedHotLeafReport,
    ObjBoundedPreviewReport, ObjCanonicalShapeReport, ObjCapabilityReport, ObjMutationReport,
    ObjPhysicalReport, ObjSnapshotTotalsReport, ObjSourceShapeReport, ScalePhysicalReport,
    ScaleScenarioReport, SemanticTranscriptReport, ShareGenerationReport, SharePhysicalReport,
    ValidationFaultReport, evidence_report, hex_encode, phase_report, release_report, sha256_hex,
};

const COMPLEX_PREVIEW_LIMIT_BYTES: usize = 192;

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
        canonical_shape = Some(inspect_obj_capability_semantics_for_probe(&parts));
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
    let snapshot_totals = ObjSnapshotTotalsReport {
        node_count: full_snapshot.totals.node_count,
        root_count: full_snapshot.totals.root_count,
        edge_count: full_snapshot.totals.edge_count,
        material_count: full_snapshot.totals.material_count,
        logical_mesh_count: full_snapshot.totals.logical_mesh_count,
        primitive_count: full_snapshot.totals.primitive_count,
        attribute_count: full_snapshot.totals.attribute_count,
        mesh_instance_count: full_snapshot.totals.mesh_instance_count,
        polygon_boundary_count: full_snapshot.totals.polygon_boundary_count,
        primitive_index_count: full_snapshot.totals.primitive_index_count,
        sparse_index_count: full_snapshot.totals.sparse_index_count,
        canonical_scalar_count: full_snapshot.totals.canonical_scalar_count,
        f32_scalar_count: full_snapshot.totals.f32_scalar_count,
        u32_scalar_count: full_snapshot.totals.u32_scalar_count,
        transcript_bytes: full_snapshot.totals.transcript_bytes,
        work_units: full_snapshot.totals.work_units,
    };
    let bounded_preview = ObjBoundedPreviewReport {
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
        .map(|mutation| ObjMutationReport {
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
        physical_account: ObjPhysicalReport {
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
    let evidence_count = failure
        .outcome_evidence_block_count()
        .expect("rich failure must expose evidence count");
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
