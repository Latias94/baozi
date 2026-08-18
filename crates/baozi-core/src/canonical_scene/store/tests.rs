use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use baozi_operation::{
    CancellationToken, PhysicalStorageAccount, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits, RecordBlock, ScalarSequence,
    StorePreparationFailureClass, StoreTerminalCause,
};

use crate::identity::SceneGenerationId;
use crate::math::{Mat4, Quat, Vec3};

use super::construction::activation::{
    SceneActivationTransition, exact_scene_activation_preflight_ordinary_work,
    exact_scene_activation_protected_work_limit, exact_scene_activation_work_limit,
};
use super::construction::{
    FAILURE_TERMINAL_WORK_DIMENSION, NodeSemanticSnapshot, OptionalCollectionSnapshot,
    PreparedSceneConstruction, ReferenceSemanticSnapshot, STORE_DIMENSION, SceneConstructionLimits,
    SceneConstructionOperation, SceneConstructionTerminal, SceneSemanticSnapshot,
    TestPostReservationFault, TestSceneFault, TracerSceneSuccessParts, TwoNodeAuthoredMatrices,
    ValidatedTracerScene,
};
use super::schema::{
    AuthoredMatrixSchema, CanonicalSchema, DirectorySchema, NodeTableSchema, ReferenceTableSchema,
};
use super::validation::authored_matrix_is_valid;

fn test_limits() -> SceneConstructionLimits {
    SceneConstructionLimits {
        operation_live_bytes: u64::MAX,
        dimension_live_bytes: u64::MAX,
        attempted_work: u64::MAX,
        consumed_work: u64::MAX,
        committed_output: u64::MAX,
        published_evidence: u64::MAX,
    }
}

fn test_factory(max_live_accounts: u64) -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, max_live_accounts)
            .expect("test factory limits"),
    )
    .expect("test physical account factory")
}

fn test_account(factory: &PhysicalStorageAccountFactory) -> PhysicalStorageAccount {
    test_account_with_allocation_limit(factory, 256)
}

fn test_account_with_allocation_limit(
    factory: &PhysicalStorageAccountFactory,
    max_live_allocations: u64,
) -> PhysicalStorageAccount {
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, max_live_allocations)
                .expect("test account limits"),
        )
        .expect("test physical account")
}

fn authored_fixture() -> TwoNodeAuthoredMatrices {
    let root = Mat4::IDENTITY;
    let child = Mat4::from_trs(
        Vec3::new(-0.0, 2.25, -7.5),
        Quat::IDENTITY,
        Vec3::new(1.5, 2.0, 0.25),
    );
    TwoNodeAuthoredMatrices::new(root, child)
}

fn matrix_bits(matrix: Mat4) -> [u32; 16] {
    std::array::from_fn(|index| matrix.cols[index / 4][index % 4].to_bits())
}

fn prepared_fresh(
    account: &PhysicalStorageAccount,
    nonce: &AtomicU64,
) -> PreparedSceneConstruction {
    SceneConstructionOperation::with_account(account, test_limits())
        .expect("explicit-account construction operation")
        .prepare_fresh_two_node_with_generation_source(authored_fixture(), || {
            SceneGenerationId::reserve_from(nonce)
        })
        .expect("fresh two-node Scene preparation")
}

fn success_scene(terminal: SceneConstructionTerminal) -> ValidatedTracerScene {
    match terminal {
        SceneConstructionTerminal::Success(parts) => parts.scene,
        SceneConstructionTerminal::RichFailure(_) => panic!("expected exact Scene success"),
        SceneConstructionTerminal::MinimalFallback(_) => panic!("expected exact Scene success"),
    }
}

fn success_parts(terminal: SceneConstructionTerminal) -> TracerSceneSuccessParts {
    match terminal {
        SceneConstructionTerminal::Success(parts) => parts,
        SceneConstructionTerminal::RichFailure(_) => panic!("expected exact Scene success"),
        SceneConstructionTerminal::MinimalFallback(_) => panic!("expected exact Scene success"),
    }
}

fn derived_scene(parent: &ValidatedTracerScene, nonce: &AtomicU64) -> ValidatedTracerScene {
    let prepared = SceneConstructionOperation::derived_from_exact_input(parent, test_limits())
        .expect("derived construction operation")
        .prepare_derived_two_node_with_generation_source(|| SceneGenerationId::reserve_from(nonce))
        .expect("derived two-node Scene preparation");
    success_scene(prepared.handoff())
}

fn assert_rich_without_generation(prepared: PreparedSceneConstruction, nonce: &AtomicU64) {
    assert_eq!(nonce.load(Ordering::Relaxed), 0);
    match prepared.handoff() {
        SceneConstructionTerminal::RichFailure(failure) => drop(failure),
        SceneConstructionTerminal::Success(_) => {
            panic!("a rejected candidate must not create a Validated tracer Scene")
        }
        SceneConstructionTerminal::MinimalFallback(_) => {
            panic!("a semantic or lineage rejection must preserve rich evidence")
        }
    }
    assert_eq!(nonce.load(Ordering::Relaxed), 0);
}

#[test]
fn canonical_store_matrix_validation_requires_exact_finite_affine_bits() {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    assert!(authored_matrix_is_valid(&identity));
    assert!(!authored_matrix_is_valid(&identity[..15]));

    let mut non_finite = identity;
    non_finite[0] = f32::NAN;
    assert!(!authored_matrix_is_valid(&non_finite));

    let mut projective = identity;
    projective[3] = 1.0;
    assert!(!authored_matrix_is_valid(&projective));
}

#[test]
fn sealed_activation_request_reaches_a_core_owned_validated_candidate() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let mut limits = test_limits();
    let exact_work = exact_scene_activation_work_limit();
    limits.attempted_work = exact_work;
    limits.consumed_work = exact_work;
    let transition = SceneConstructionOperation::with_account_for_activation(&account, limits)
        .expect("explicit-account construction operation")
        .activate_fresh_two_node(authored_fixture())
        .expect("sealed core activation");
    let candidate = match transition {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(prepared) => match prepared.handoff() {
            SceneConstructionTerminal::Success(_) => {
                panic!("activation must not publish before joint commit")
            }
            SceneConstructionTerminal::RichFailure(failure) => {
                let _ = failure;
                panic!("valid core input unexpectedly reached rich failure")
            }
            SceneConstructionTerminal::MinimalFallback(fallback) => {
                panic!(
                    "valid core input unexpectedly reached minimal fallback: code={:#010x}, cause={:?}",
                    fallback.fallback_code(),
                    fallback.ledger_evidence().header().terminal_cause()
                )
            }
        },
    };

    let destination_counts = candidate.destination_counts();
    assert_eq!(
        destination_counts.canonical_blocks(),
        usize::try_from(super::schema::CANONICAL_BLOCK_COUNT).unwrap()
    );
    assert_eq!(destination_counts.outcome_evidence_blocks(), 1);
    assert_eq!(candidate.directory(), candidate.refs().directory);
    assert!(account.snapshot().live_retained_allocations() > 0);

    drop(candidate);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sealed_activation_joint_commit_hands_off_one_exact_generation() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let expected_generation = Cell::new(None);
    let expected = authored_fixture();
    let transition =
        SceneConstructionOperation::with_account_for_activation(&account, test_limits())
            .expect("explicit-account activation operation")
            .activate_fresh_two_node(expected)
            .expect("sealed core activation");
    let candidate = match transition {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(_) => {
            panic!("valid activation must reach joint commit")
        }
    };

    let prepared = candidate
        .prepare_with_generation_source(|| {
            let generation = SceneGenerationId::reserve_from(&nonce)?;
            expected_generation.set(Some(generation.id()));
            Ok(generation)
        })
        .expect("scoped joint commit must prepare exact success");
    assert_eq!(nonce.load(Ordering::Relaxed), 1);
    assert!(matches!(&prepared, PreparedSceneConstruction::Success(_)));
    let before_handoff = account.snapshot();
    let parts = success_parts(prepared.handoff());
    assert_eq!(account.snapshot(), before_handoff);
    assert_eq!(
        parts.scene.generation(),
        expected_generation
            .get()
            .expect("the successful reservation records its exact generation")
    );
    assert_eq!(parts.scene.authored_matrix(0), Some(expected.root()));
    assert_eq!(parts.scene.authored_matrix(1), Some(expected.child()));
    assert_eq!(nonce.load(Ordering::Relaxed), 1);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sealed_activation_destination_rejection_skips_generation_reservation() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let mut limits = test_limits();
    limits.committed_output = 0;
    let transition = SceneConstructionOperation::with_account_for_activation(&account, limits)
        .expect("zero-destination activation operation")
        .activate_fresh_two_node(authored_fixture())
        .expect("destination rejection remains contained");
    let candidate = match transition {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(_) => {
            panic!("destination qualification belongs to the freeze-permit boundary")
        }
    };

    let prepared = candidate
        .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&nonce))
        .expect("destination rejection must preserve a prepared terminal");
    assert_eq!(nonce.load(Ordering::Relaxed), 0);
    match prepared.handoff() {
        SceneConstructionTerminal::RichFailure(failure) => {
            let terminal = match failure.ledger_evidence().header().terminal_cause() {
                StoreTerminalCause::Failure(cause) => cause,
                StoreTerminalCause::Success => {
                    panic!("destination rejection must not report a success terminal")
                }
                _ => panic!("destination rejection reported an unknown terminal cause"),
            };
            assert_eq!(terminal.class(), StorePreparationFailureClass::Ledger);
            assert_eq!(terminal.code(), 1);
            drop(failure);
        }
        SceneConstructionTerminal::Success(_) => panic!("rejected destinations must not publish"),
        SceneConstructionTerminal::MinimalFallback(_) => {
            panic!("qualified destination rejection must retain rich evidence")
        }
    }
    assert_eq!(nonce.load(Ordering::Relaxed), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sealed_activation_frozen_interruption_skips_generation_reservation() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let cancellation = CancellationToken::new();
    let transition = SceneConstructionOperation::with_account_and_cancellation_for_activation(
        &account,
        test_limits(),
        cancellation.clone(),
    )
    .expect("cancellable activation operation")
    .activate_fresh_two_node(authored_fixture())
    .expect("sealed core activation");
    let candidate = match transition {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(_) => panic!("valid input must reach the candidate"),
    };
    cancellation.cancel();

    let prepared = candidate
        .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&nonce))
        .expect("frozen control interruption must preserve a terminal");
    assert_eq!(nonce.load(Ordering::Relaxed), 0);
    match prepared.handoff() {
        SceneConstructionTerminal::RichFailure(failure) => {
            let terminal = match failure.ledger_evidence().header().terminal_cause() {
                StoreTerminalCause::Failure(cause) => cause,
                StoreTerminalCause::Success => {
                    panic!("interrupted activation must not report a success terminal")
                }
                _ => panic!("interrupted activation reported an unknown terminal cause"),
            };
            assert_eq!(terminal.class(), StorePreparationFailureClass::Control);
            assert_eq!(terminal.code(), 1);
            drop(failure);
        }
        SceneConstructionTerminal::Success(_) => panic!("interrupted activation must not publish"),
        SceneConstructionTerminal::MinimalFallback(_) => {
            panic!("qualified control interruption must retain rich evidence")
        }
    }
    assert_eq!(nonce.load(Ordering::Relaxed), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn dropping_prepared_activation_success_burns_one_invisible_gap() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let transition =
        SceneConstructionOperation::with_account_for_activation(&account, test_limits())
            .expect("activation operation")
            .activate_fresh_two_node(authored_fixture())
            .expect("sealed core activation");
    let candidate = match transition {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(_) => panic!("valid input must reach the candidate"),
    };
    let prepared = candidate
        .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&nonce))
        .expect("joint commit must prepare success");
    assert!(matches!(&prepared, PreparedSceneConstruction::Success(_)));
    assert_eq!(nonce.load(Ordering::Relaxed), 1);
    let before_drop = account.snapshot();
    drop(prepared);
    let after_drop = account.snapshot();
    assert!(!after_drop.poisoned());
    assert_eq!(after_drop.live_retained_bytes(), 0);
    assert_eq!(after_drop.live_retained_allocations(), 0);
    assert_eq!(
        after_drop.total_admitted_retained_bytes(),
        before_drop.total_admitted_retained_bytes()
    );
    assert_eq!(
        after_drop.total_admitted_retained_allocations(),
        before_drop.total_admitted_retained_allocations()
    );
    assert_eq!(
        after_drop.poison_event_count(),
        before_drop.poison_event_count()
    );

    let next = SceneConstructionOperation::with_account_for_activation(&account, test_limits())
        .expect("next activation operation")
        .activate_fresh_two_node(authored_fixture())
        .expect("next sealed activation");
    let candidate = match next {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(_) => panic!("next input must reach the candidate"),
    };
    let prepared = candidate
        .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&nonce))
        .expect("next joint commit must succeed");
    assert_eq!(nonce.load(Ordering::Relaxed), 2);
    drop(success_scene(prepared.handoff()));
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sealed_activation_generation_exhaustion_never_publishes_or_consumes_nonce() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(u64::MAX);
    let transition =
        SceneConstructionOperation::with_account_for_activation(&account, test_limits())
            .expect("explicit-account activation operation")
            .activate_fresh_two_node(authored_fixture())
            .expect("sealed core activation");
    let candidate = match transition {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(_) => {
            panic!("valid activation must reach joint commit")
        }
    };

    let prepared = candidate
        .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&nonce))
        .expect("generation exhaustion must preserve a prepared terminal");
    assert_eq!(nonce.load(Ordering::Relaxed), u64::MAX);
    match prepared.handoff() {
        SceneConstructionTerminal::RichFailure(failure) => {
            let terminal = match failure.ledger_evidence().header().terminal_cause() {
                StoreTerminalCause::Failure(cause) => cause,
                StoreTerminalCause::Success => {
                    panic!("generation exhaustion must not report a success terminal")
                }
                _ => panic!("generation exhaustion reported an unknown terminal cause"),
            };
            assert_eq!(terminal.class(), StorePreparationFailureClass::Requested);
            assert_eq!(terminal.code(), 0x5343_0003);
            drop(failure);
        }
        SceneConstructionTerminal::Success(_) => panic!("generation exhaustion must not publish"),
        SceneConstructionTerminal::MinimalFallback(_) => {
            panic!("qualified generation exhaustion must retain rich evidence")
        }
    }
    assert_eq!(nonce.load(Ordering::Relaxed), u64::MAX);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn generation_source_panic_after_reservation_burns_the_gap_and_fails_closed() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let transition =
        SceneConstructionOperation::with_account_for_activation(&account, test_limits())
            .expect("explicit-account activation operation")
            .activate_fresh_two_node(authored_fixture())
            .expect("sealed core activation");
    let candidate = match transition {
        SceneActivationTransition::Candidate(candidate) => candidate,
        SceneActivationTransition::Terminal(_) => {
            panic!("valid activation must reach the generation boundary")
        }
    };
    let before = account.snapshot();

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = candidate.prepare_with_generation_source(|| {
            let _generation = SceneGenerationId::reserve_from(&nonce)
                .expect("the injected generation reservation must succeed");
            panic!("injected generation source panic after reservation")
        });
    }));

    assert!(panic.is_err());
    assert_eq!(nonce.load(Ordering::Relaxed), 1);
    let after = account.snapshot();
    assert!(after.poisoned());
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert_eq!(
        after.total_admitted_retained_bytes(),
        before.total_admitted_retained_bytes()
    );
    assert_eq!(
        after.total_admitted_retained_allocations(),
        before.total_admitted_retained_allocations()
    );
}

#[test]
fn sealed_activation_one_short_work_budget_interrupts_before_candidate() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let mut limits = test_limits();
    let exact_work = exact_scene_activation_work_limit();
    let one_short = exact_work
        .checked_sub(1)
        .expect("the exact activation work limit is non-zero");
    limits.attempted_work = one_short;
    limits.consumed_work = one_short;

    let transition = SceneConstructionOperation::with_account_for_activation(&account, limits)
        .expect("the one-short budget still covers protected terminal work")
        .activate_fresh_two_node(authored_fixture())
        .expect("the late ledger interruption remains contained");
    let prepared = match transition {
        SceneActivationTransition::Candidate(_) => {
            panic!("one-short ordinary work must not produce a validated candidate")
        }
        SceneActivationTransition::Terminal(prepared) => prepared,
    };
    let failure = match prepared.handoff() {
        SceneConstructionTerminal::RichFailure(failure) => failure,
        SceneConstructionTerminal::Success(_) => {
            panic!("one-short ordinary work must not publish")
        }
        SceneConstructionTerminal::MinimalFallback(_) => {
            panic!("a ledger interruption after qualified evidence must remain rich")
        }
    };
    let terminal = match failure.ledger_evidence().header().terminal_cause() {
        StoreTerminalCause::Failure(cause) => cause,
        StoreTerminalCause::Success => panic!("the interrupted operation cannot report success"),
        _ => panic!("the interrupted operation reported an unknown terminal cause"),
    };

    assert_eq!(terminal.class(), StorePreparationFailureClass::Ledger);
    assert_eq!(failure.outcome_evidence_block_count(), 1);
    assert!(failure.account().is_same_account(&account));
    let ordinary = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("the activation has its ordinary work dimension");
    let protected = failure
        .ledger_evidence()
        .dimension_by_semantic(FAILURE_TERMINAL_WORK_DIMENSION)
        .expect("the activation has its protected failure dimension");
    let rich_validation_work =
        u64::try_from(super::schema::EVIDENCE_FIELD_COUNT).expect("the evidence width fits u64");
    let full_ordinary_work = exact_work
        .checked_sub(exact_scene_activation_protected_work_limit())
        .expect("protected work is contained in the total activation limit");
    assert_eq!(ordinary.attempted(), full_ordinary_work);
    assert_eq!(
        ordinary.consumed() + super::validation::scene_graph_validation_work(),
        ordinary.attempted()
    );
    assert_eq!(protected.attempted(), rich_validation_work);
    assert_eq!(protected.attempted(), protected.consumed());

    drop(failure);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sealed_activation_rejects_invalid_authored_semantics_before_activation() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let mut invalid_root = Mat4::IDENTITY;
    invalid_root.cols[0][3] = 1.0;

    let transition =
        SceneConstructionOperation::with_account_for_activation(&account, test_limits())
            .expect("explicit-account construction operation")
            .activate_fresh_two_node(TwoNodeAuthoredMatrices::new(invalid_root, Mat4::IDENTITY))
            .expect("sealed core activation");
    let prepared = match transition {
        SceneActivationTransition::Candidate(_) => {
            panic!("a non-affine authored matrix must fail preflight")
        }
        SceneActivationTransition::Terminal(prepared) => prepared,
    };
    let failure = match prepared.handoff() {
        SceneConstructionTerminal::RichFailure(failure) => failure,
        SceneConstructionTerminal::Success(_) => {
            panic!("a non-affine authored matrix must not publish")
        }
        SceneConstructionTerminal::MinimalFallback(_) => {
            panic!("preflight semantic rejection must preserve qualified rich evidence")
        }
    };

    assert_eq!(failure.outcome_evidence_block_count(), 1);
    assert!(failure.account().is_same_account(&account));
    assert!(account.snapshot().live_retained_allocations() > 0);
    let ordinary = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("the activation has its ordinary work dimension");
    let protected = failure
        .ledger_evidence()
        .dimension_by_semantic(FAILURE_TERMINAL_WORK_DIMENSION)
        .expect("the activation has its protected failure dimension");
    assert_eq!(
        ordinary.attempted(),
        exact_scene_activation_preflight_ordinary_work()
    );
    assert_eq!(ordinary.attempted(), ordinary.consumed());
    assert_eq!(ordinary.committed_canonical_output(), 0);
    assert_eq!(
        protected.attempted(),
        exact_scene_activation_protected_work_limit()
    );
    assert_eq!(protected.attempted(), protected.consumed());

    drop(failure);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn canonical_store_explicit_account_handoff_preserves_snapshot_and_authored_bits() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let expected = authored_fixture();
    let prepared = prepared_fresh(&account, &nonce);
    assert!(matches!(prepared, PreparedSceneConstruction::Success(_)));
    assert_eq!(nonce.load(std::sync::atomic::Ordering::Relaxed), 1);

    let before_handoff = account.snapshot();
    let parts = success_parts(prepared.handoff());
    assert_eq!(account.snapshot(), before_handoff);
    let scene = parts.scene;
    let report = parts.report;
    assert_eq!(scene.canonical_block_count(), 4);
    assert_eq!(report.outcome_evidence_block_count(), 1);
    assert_eq!(scene.authored_matrix(0), Some(expected.root()));
    assert_eq!(scene.authored_matrix(1), Some(expected.child()));
    assert_eq!(
        scene.authored_matrix_bits(0),
        Some(matrix_bits(expected.root()))
    );
    assert_eq!(
        scene.authored_matrix_bits(1),
        Some(matrix_bits(expected.child()))
    );
    let expected_snapshot = SceneSemanticSnapshot {
        schema_revision: 1,
        root_node: 0,
        node_count: 2,
        reference_count: 1,
        root_children: OptionalCollectionSnapshot {
            presence: 1,
            count: 1,
        },
        child_children: OptionalCollectionSnapshot {
            presence: 1,
            count: 0,
        },
        metadata: OptionalCollectionSnapshot {
            presence: 0,
            count: 0,
        },
        matrix_scalar_count: 32,
        nodes: [
            NodeSemanticSnapshot {
                node: 0,
                parent: OptionalCollectionSnapshot {
                    presence: 0,
                    count: 0,
                },
                parent_reference_offset: 0,
                matrix_offset: 0,
                matrix_length: 16,
            },
            NodeSemanticSnapshot {
                node: 1,
                parent: OptionalCollectionSnapshot {
                    presence: 1,
                    count: 1,
                },
                parent_reference_offset: 0,
                matrix_offset: 16,
                matrix_length: 16,
            },
        ],
        parent_reference: ReferenceSemanticSnapshot {
            reference: 0,
            parent: 0,
            child: 1,
        },
        authored_matrix_bits: [
            [
                1.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                1.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                1.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                1.0_f32.to_bits(),
            ],
            [
                1.5_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                2.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.0_f32.to_bits(),
                0.25_f32.to_bits(),
                0.0_f32.to_bits(),
                (-0.0_f32).to_bits(),
                2.25_f32.to_bits(),
                (-7.5_f32).to_bits(),
                1.0_f32.to_bits(),
            ],
        ],
    };
    assert_eq!(scene.semantic_snapshot(), Some(expected_snapshot));
    assert!(matches!(
        report.ledger_evidence().header().terminal_cause(),
        StoreTerminalCause::Success
    ));

    drop(report);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_private_account_uses_factory_limits_and_releases_account() {
    let factory = test_factory(1);
    let account_limits =
        PhysicalStorageAccountLimits::new(u64::MAX, 256).expect("private account limits");
    let nonce = AtomicU64::new(40);
    let prepared =
        SceneConstructionOperation::with_private_account(&factory, account_limits, test_limits())
            .expect("private-account construction operation")
            .prepare_fresh_two_node_with_generation_source(authored_fixture(), || {
                SceneGenerationId::reserve_from(&nonce)
            })
            .expect("private-account Scene preparation");
    assert_eq!(factory.snapshot().live_accounts(), 1);

    let scene = success_scene(prepared.handoff());
    assert_eq!(scene.account().limits(), account_limits);
    assert_eq!(scene.authored_matrix(1), Some(authored_fixture().child()));
    drop(scene);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_parent_and_siblings_use_fresh_slots_and_share_leaf_lifetime() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let parent = success_scene(prepared_fresh(&account, &nonce).handoff());
    let first_child = derived_scene(&parent, &nonce);
    let second_child = derived_scene(&parent, &nonce);

    let parent_refs = parent.block_refs();
    let first_refs = first_child.block_refs();
    let second_refs = second_child.block_refs();
    assert_ne!(parent_refs.directory, first_refs.directory);
    assert_ne!(parent_refs.nodes, first_refs.nodes);
    assert_ne!(parent_refs.references, first_refs.references);
    assert_ne!(parent_refs.matrices, first_refs.matrices);
    assert_ne!(first_refs.directory, second_refs.directory);
    assert_ne!(first_refs.nodes, second_refs.nodes);
    assert_ne!(first_refs.references, second_refs.references);
    assert_ne!(first_refs.matrices, second_refs.matrices);
    assert_eq!(nonce.load(std::sync::atomic::Ordering::Relaxed), 3);

    let expected_child = parent.authored_matrix_bits(1);
    drop(parent);
    assert_eq!(first_child.authored_matrix_bits(1), expected_child);
    assert_eq!(second_child.authored_matrix_bits(1), expected_child);
    drop(first_child);
    assert_eq!(second_child.authored_matrix_bits(1), expected_child);
    assert!(account.snapshot().live_retained_allocations() > 0);
    drop(second_child);

    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_generation_exhaustion_downgrades_without_consuming_nonce() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(u64::MAX);
    let prepared = prepared_fresh(&account, &nonce);
    assert!(matches!(
        prepared,
        PreparedSceneConstruction::RichFailure(_)
    ));
    assert_eq!(nonce.load(Ordering::Relaxed), u64::MAX);

    let before_handoff = account.snapshot();
    let failure = match prepared.handoff() {
        SceneConstructionTerminal::RichFailure(failure) => failure,
        SceneConstructionTerminal::Success(_) => panic!("exhaustion must not publish a Scene"),
        SceneConstructionTerminal::MinimalFallback(_) => {
            panic!("ordinary generation exhaustion must retain rich evidence")
        }
    };
    assert_eq!(account.snapshot(), before_handoff);
    assert_eq!(failure.outcome_evidence_block_count(), 1);
    assert!(matches!(
        failure.ledger_evidence().header().terminal_cause(),
        StoreTerminalCause::Failure(_)
    ));
    let dimension = failure
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("the tracer has its ordinary ledger dimension");
    assert_eq!(dimension.committed_canonical_output(), 0);
    assert_eq!(dimension.published_outcome_evidence(), 2);

    drop(failure);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_dropped_prepared_success_burns_one_local_gap_then_handoff_succeeds() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let discarded = prepared_fresh(&account, &nonce);
    assert!(matches!(discarded, PreparedSceneConstruction::Success(_)));
    assert_eq!(nonce.load(Ordering::Relaxed), 1);
    drop(discarded);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);

    let published = prepared_fresh(&account, &nonce);
    assert_eq!(nonce.load(Ordering::Relaxed), 2);
    let scene = success_scene(published.handoff());
    assert_eq!(scene.authored_matrix(0), Some(Mat4::IDENTITY));
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_post_reservation_failures_each_burn_one_invisible_gap() {
    #[derive(Clone, Copy)]
    enum Scenario {
        PreparationFailure,
        Cancellation,
        CaughtUnwind,
        PreparedDrop,
        RichPreparationFailure,
    }

    for scenario in [
        Scenario::PreparationFailure,
        Scenario::Cancellation,
        Scenario::CaughtUnwind,
        Scenario::PreparedDrop,
        Scenario::RichPreparationFailure,
    ] {
        let factory = test_factory(1);
        let account = test_account(&factory);
        let nonce = AtomicU64::new(0);
        let result = match scenario {
            Scenario::PreparationFailure => {
                let mut limits = test_limits();
                limits.committed_output = 3;
                SceneConstructionOperation::with_account(&account, limits)
                    .expect("preparation-failure operation")
                    .prepare_fresh_two_node_with_generation_source(authored_fixture(), || {
                        SceneGenerationId::reserve_from(&nonce)
                    })
            }
            Scenario::Cancellation => {
                let cancellation = CancellationToken::new();
                SceneConstructionOperation::with_account_and_cancellation_for_test(
                    &account,
                    test_limits(),
                    cancellation.clone(),
                )
                .expect("cancellable operation")
                .prepare_fresh_two_node_with_generation_source(authored_fixture(), || {
                    SceneGenerationId::reserve_from(&nonce).inspect(|_| cancellation.cancel())
                })
            }
            Scenario::CaughtUnwind => {
                SceneConstructionOperation::with_account_and_panicking_progress_observer_for_test(
                    &account,
                    test_limits(),
                )
                .expect("panic-containing operation")
                .prepare_fresh_two_node_with_generation_source(authored_fixture(), || {
                    SceneGenerationId::reserve_from(&nonce)
                })
            }
            Scenario::PreparedDrop => Ok(prepared_fresh(&account, &nonce)),
            Scenario::RichPreparationFailure => {
                let mut limits = test_limits();
                limits.published_evidence = 2;
                SceneConstructionOperation::with_account(&account, limits)
                    .expect("rich-preparation-failure operation")
                    .prepare_fresh_two_node_with_post_reservation_fault_for_test(
                        authored_fixture(),
                        TestPostReservationFault::RichPreparationLedgerFailure,
                        || SceneGenerationId::reserve_from(&nonce),
                    )
            }
        };

        assert_eq!(nonce.load(Ordering::Relaxed), 1);
        let prepared = result.expect("post-reservation containment must produce a terminal");
        match scenario {
            Scenario::PreparedDrop => {
                assert!(matches!(&prepared, PreparedSceneConstruction::Success(_)));
                drop(prepared);
            }
            Scenario::PreparationFailure => {
                assert!(matches!(
                    &prepared,
                    PreparedSceneConstruction::RichFailure(_)
                ));
                assert!(matches!(
                    prepared.handoff(),
                    SceneConstructionTerminal::RichFailure(_)
                ));
            }
            Scenario::RichPreparationFailure => {
                assert!(matches!(
                    &prepared,
                    PreparedSceneConstruction::MinimalFallback(_)
                ));
                assert!(matches!(
                    prepared.handoff(),
                    SceneConstructionTerminal::MinimalFallback(_)
                ));
            }
            Scenario::Cancellation | Scenario::CaughtUnwind => match prepared.handoff() {
                SceneConstructionTerminal::RichFailure(failure) => drop(failure),
                SceneConstructionTerminal::MinimalFallback(fallback) => drop(fallback),
                SceneConstructionTerminal::Success(_) => {
                    panic!("post-reservation control failure must not publish a Scene")
                }
            },
        }
        assert_eq!(account.snapshot().live_retained_bytes(), 0);

        let next = prepared_fresh(&account, &nonce);
        assert_eq!(nonce.load(Ordering::Relaxed), 2);
        let scene = success_scene(next.handoff());
        drop(scene);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }
}

#[test]
fn canonical_store_rejects_unauthorized_lineage_before_reserve_and_remains_usable() {
    let same_factory = test_factory(1);
    let same_account = test_account(&same_factory);
    let foreign_factory = test_factory(1);
    let foreign_account = test_account(&foreign_factory);
    let setup_nonce = AtomicU64::new(0);
    let parent = success_scene(prepared_fresh(&same_account, &setup_nonce).handoff());
    let independent = success_scene(prepared_fresh(&same_account, &setup_nonce).handoff());
    let foreign = success_scene(prepared_fresh(&foreign_account, &setup_nonce).handoff());
    let rejection_nonce = AtomicU64::new(0);

    for (source_scene, source_matrix) in [
        (&independent, independent.block_refs().matrices),
        (&foreign, foreign.block_refs().matrices),
        (&parent, independent.block_refs().matrices),
    ] {
        let prepared = SceneConstructionOperation::derived_from_exact_input(&parent, test_limits())
            .expect("derived operation for lineage rejection")
            .prepare_derived_two_node_with_fault_for_test(source_scene, source_matrix, None, || {
                SceneGenerationId::reserve_from(&rejection_nonce)
            })
            .expect("lineage rejection should prepare rich evidence");
        assert_rich_without_generation(prepared, &rejection_nonce);
    }

    let injected = SceneConstructionOperation::derived_from_exact_input(&parent, test_limits())
        .expect("derived operation for source-slot injection")
        .prepare_derived_two_node_with_fault_for_test(
            &parent,
            parent.block_refs().matrices,
            Some(TestSceneFault::SourceSlotInDirectory),
            || SceneGenerationId::reserve_from(&rejection_nonce),
        )
        .expect("source-slot injection should prepare rich evidence");
    assert_rich_without_generation(injected, &rejection_nonce);

    let legal = derived_scene(&parent, &rejection_nonce);
    assert_eq!(rejection_nonce.load(Ordering::Relaxed), 1);
    assert_eq!(
        legal.authored_matrix_bits(1),
        parent.authored_matrix_bits(1)
    );

    drop(legal);
    drop(parent);
    drop(independent);
    drop(foreign);
    assert_eq!(same_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(foreign_account.snapshot().live_retained_bytes(), 0);
    drop(same_account);
    drop(foreign_account);
    assert_eq!(same_factory.snapshot().live_accounts(), 0);
    assert_eq!(foreign_factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_semantic_fault_matrix_rejects_without_validated_scene() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    for fault in [
        TestSceneFault::WrongRoot,
        TestSceneFault::WrongParent,
        TestSceneFault::WrongCardinality,
        TestSceneFault::InvalidMatrix,
        TestSceneFault::WrongReference,
        TestSceneFault::OrphanCanonicalBlock,
    ] {
        let prepared = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("fault-matrix construction operation")
            .prepare_fresh_two_node_with_fault_for_test(authored_fixture(), fault, || {
                SceneGenerationId::reserve_from(&nonce)
            })
            .expect("semantic rejection should prepare rich evidence");
        assert_rich_without_generation(prepared, &nonce);
    }

    let valid = prepared_fresh(&account, &nonce);
    let scene = success_scene(valid.handoff());
    assert_eq!(nonce.load(Ordering::Relaxed), 1);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_keeps_share_surface_leaf_only_and_core_private() {
    fn assert_record_schema<Schema>()
    where
        Schema: CanonicalSchema<StoreKind = RecordBlock>,
    {
    }

    fn assert_shareable_leaf_schema<Schema>()
    where
        Schema: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
    {
    }

    // The directory is also the Scene header; all structural schemas remain record-only.
    assert_record_schema::<DirectorySchema>();
    assert_record_schema::<NodeTableSchema>();
    assert_record_schema::<ReferenceTableSchema>();
    assert_shareable_leaf_schema::<AuthoredMatrixSchema>();

    let operation_source = format!(
        "{}\n{}\n{}",
        include_str!("../../../../baozi-operation/src/concrete_store.rs"),
        include_str!("../../../../baozi-operation/src/concrete_store/vocabulary.rs"),
        include_str!("../../../../baozi-operation/src/concrete_store/terminal_output.rs")
    );
    let core_source = include_str!("construction.rs");
    let schema_source = include_str!("schema.rs");

    assert!(operation_source.contains("SlotKind: ShareableStoreKind"));
    assert!(
        operation_source
            .contains("impl<T: StoredScalar> ShareableStoreKind for ScalarSequence<T> {}")
    );
    assert!(!operation_source.contains("ShareableStoreKind for RecordBlock"));
    assert_eq!(core_source.matches(".try_share_canonical(").count(), 1);
    assert!(!core_source.contains("pub fn try_share"));
    assert!(!core_source.contains("&mut Block"));
    assert!(!core_source.contains("Box<dyn"));
    assert!(!core_source.contains("HList"));

    let projection = schema_source
        .split("pub(super) struct CanonicalBlockRef")
        .nth(1)
        .and_then(|tail| tail.split('}').next())
        .expect("private canonical projection declaration");
    assert!(projection.contains("CanonicalStoreRef"));
    for forbidden in ["Vec<", "Arc<", "Box<", "Mutex<"] {
        assert!(!projection.contains(forbidden));
    }
}

#[test]
fn canonical_store_complex_fixture_closes_typed_cross_table_graph() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);
    let prepared = SceneConstructionOperation::with_account(&account, test_limits())
        .expect("complex fixture construction operation")
        .prepare_complex_fixture_for_test(None, || SceneGenerationId::reserve_from(&nonce))
        .expect("complex fixture preparation");
    let before_handoff = account.snapshot();
    let parts = match prepared.handoff() {
        super::construction::ComplexFixtureTerminal::Success(parts) => parts,
        super::construction::ComplexFixtureTerminal::RichFailure(failure) => {
            panic!(
                "the complete complex fixture must validate; it was rejected with {:#?} evidence blocks",
                failure.outcome_evidence_block_count()
            )
        }
        super::construction::ComplexFixtureTerminal::MinimalFallback(_) => {
            panic!("the complete complex fixture must not fall back")
        }
    };
    assert_eq!(nonce.load(Ordering::Relaxed), 1);
    assert_eq!(account.snapshot(), before_handoff);
    assert_eq!(parts.fixture.canonical_block_count(), 26);
    assert!(parts.fixture.root_is_retained());
    assert_eq!(parts.report.outcome_evidence_block_count(), 1);
    let ledger = parts.report.ledger_evidence();
    assert_eq!(ledger.header().dimension_count(), 2);
    let ordinary = ledger
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("the complex fixture has its ordinary ledger dimension");
    let failure_terminal = ledger
        .dimension_by_semantic(FAILURE_TERMINAL_WORK_DIMENSION)
        .expect("the complex fixture has its protected failure-terminal dimension");
    assert_eq!(ordinary.committed_canonical_output(), 26);
    // The frozen ledger records the retained candidate evidence plus its terminal snapshot.
    assert_eq!(ordinary.published_outcome_evidence(), 2);
    assert_eq!(failure_terminal.attempted(), 0);
    assert_eq!(failure_terminal.consumed(), 0);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_complex_fixture_rejects_representative_validation_faults() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);

    for fault in [
        super::construction::ComplexFixtureFault::CrossTableIndex,
        super::construction::ComplexFixtureFault::ReferenceTarget,
        super::construction::ComplexFixtureFault::SequenceValue,
        super::construction::ComplexFixtureFault::Utf8Value,
        super::construction::ComplexFixtureFault::BytesValue,
        super::construction::ComplexFixtureFault::OrphanCanonicalBlock,
    ] {
        let prepared = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("complex fixture fault operation")
            .prepare_complex_fixture_for_test(Some(fault), || {
                SceneGenerationId::reserve_from(&nonce)
            })
            .expect("rejected complex fixture should retain rich evidence");
        assert!(matches!(
            prepared,
            super::construction::PreparedComplexFixture::RichFailure(_)
        ));
        assert_eq!(nonce.load(Ordering::Relaxed), 0);
        match prepared.handoff() {
            super::construction::ComplexFixtureTerminal::RichFailure(failure) => {
                assert_eq!(failure.outcome_evidence_block_count(), 1);
                drop(failure);
            }
            super::construction::ComplexFixtureTerminal::Success(_) => {
                panic!("a rejected complex fixture must not publish canonical data")
            }
            super::construction::ComplexFixtureTerminal::MinimalFallback(_) => {
                panic!("semantic rejection must preserve rich evidence")
            }
        }
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn canonical_store_complex_fixture_scales_reachable_image_leaves() {
    let factory = test_factory(1);
    let account = test_account(&factory);
    let nonce = AtomicU64::new(0);

    for (image_count, expected_blocks) in [(1, 26), (14, 52), (40, 104)] {
        let prepared = SceneConstructionOperation::with_account_for_scaled_complex_fixture(
            &account,
            test_limits(),
            image_count,
        )
        .expect("scaled complex fixture operation")
        .prepare(None, || SceneGenerationId::reserve_from(&nonce))
        .expect("scaled complex fixture should prepare");
        let parts = match prepared.handoff() {
            super::construction::ComplexFixtureTerminal::Success(parts) => parts,
            super::construction::ComplexFixtureTerminal::RichFailure(_) => {
                panic!("scaled complex fixture must validate")
            }
            super::construction::ComplexFixtureTerminal::MinimalFallback(_) => {
                panic!("scaled complex fixture must not fall back")
            }
        };
        assert_eq!(parts.fixture.canonical_block_count(), expected_blocks);
        parts
            .fixture
            .inspect_fixed_image_views(image_count)
            .expect("scaled complex image views must remain inspectable");
        let transcript = parts
            .fixture
            .semantic_transcript()
            .expect("scaled complex fixture must have a semantic transcript");
        assert!(transcript.contains(&format!("u32:directory.image_count={image_count}\n")));
        drop(parts);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    assert_eq!(nonce.load(Ordering::Relaxed), 3);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}
