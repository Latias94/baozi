use super::*;
use baozi_import::authoring::NodeLocalId;

#[test]
fn final_control_denial_prevents_directory_publication_after_all_mutations() {
    let observation = run(
        |ctx, physical, token| {
            let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
            COMPLETION_PROBE.with(|slot| {
                slot.borrow_mut()
                    .as_mut()
                    .unwrap()
                    .cancel_before_directory_publication = Some(token.clone());
            });
            let result = selected.finish(ctx, &ordinary, &suspended);
            assert!(result.is_err());
            assert!(!selected.finished);
            assert!(selected.directory.is_none());
            physical.store(1, AtomicOrdering::Relaxed);
            result
        },
        None,
        None,
        br#"{"nodes":[{},{}]}"#,
    );
    assert_cancelled(&observation.failure);
    assert_eq!(observation.physical, 1);
    assert_eq!(
        observation.completed[ComparisonSite::DirectoryInit as usize],
        2
    );
    assert_eq!(
        observation.completed[ComparisonSite::DirectoryRegister as usize],
        2
    );
    let (attempted, consumed) = counts(&observation.failure);
    assert_eq!(attempted, consumed);
}

#[test]
fn owner_keys_follow_the_nineteen_canonical_classes() {
    let owners = [
        ProfileOwner::Scene,
        ProfileOwner::SyntheticRoot,
        ProfileOwner::Node(NodeLocalId::new(7)),
        ProfileOwner::Camera(7),
        ProfileOwner::Light(7),
        ProfileOwner::Image(7),
        ProfileOwner::Sampler(7),
        ProfileOwner::Texture(7),
        ProfileOwner::TextureBinding(7),
        ProfileOwner::Material(7),
        ProfileOwner::LogicalMesh(7),
        ProfileOwner::Primitive(7),
        ProfileOwner::Attribute(7),
        ProfileOwner::MeshInstance(7),
        ProfileOwner::Skin(7),
        ProfileOwner::MorphDefinition(7),
        ProfileOwner::MorphDelta(7),
        ProfileOwner::AnimationClip(7),
        ProfileOwner::AnimationChannel(7),
    ];
    assert_eq!(owners.len(), OWNER_CLASSES);
    for (class, owner) in owners.into_iter().enumerate() {
        assert_eq!(owner_key(owner), (class, if class < 2 { 0 } else { 7 }));
    }
}

#[test]
fn directory_shape_bounds_empty_classes_offsets_and_checked_prefixes() {
    let empty = OwnerDirectoryShape::from_maxima([0; OWNER_CLASSES], false, 0).unwrap();
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.offsets, [0; 20]);
    assert_eq!(empty.slot(ProfileOwner::Scene), None);
    assert_eq!(empty.slot(ProfileOwner::AnimationChannel(0)), None);

    let mut maxima = [0; OWNER_CLASSES];
    maxima[0] = 1;
    maxima[2] = 2;
    maxima[6] = 1;
    maxima[18] = 2;
    let shape = OwnerDirectoryShape::from_maxima(maxima, false, 6).unwrap();
    assert_eq!(shape.len(), 6);
    assert_eq!(&shape.offsets[..4], &[0, 1, 1, 3]);
    assert_eq!(&shape.offsets[4..7], &[3; 3]);
    assert_eq!(&shape.offsets[7..19], &[4; 12]);
    assert_eq!(shape.offsets[19], 6);
    for (owner, expected) in [
        (ProfileOwner::Scene, Some(0)),
        (ProfileOwner::SyntheticRoot, None),
        (ProfileOwner::Node(NodeLocalId::new(0)), Some(1)),
        (ProfileOwner::Node(NodeLocalId::new(1)), Some(2)),
        (ProfileOwner::Node(NodeLocalId::new(2)), None),
        (ProfileOwner::Camera(0), None),
        (ProfileOwner::Sampler(0), Some(3)),
        (ProfileOwner::Sampler(1), None),
        (ProfileOwner::AnimationChannel(0), Some(4)),
        (ProfileOwner::AnimationChannel(1), Some(5)),
        (ProfileOwner::AnimationChannel(2), None),
        (ProfileOwner::AnimationChannel(u32::MAX), None),
    ] {
        assert_eq!(shape.slot(owner), expected, "{owner:?}");
    }

    assert_eq!(
        OwnerDirectoryShape::from_maxima([0; OWNER_CLASSES], true, usize::MAX)
            .err()
            .unwrap()
            .to_string(),
        "glTF source owner directory exceeds usize",
    );
    let mut overflow = [0; OWNER_CLASSES];
    overflow[0] = usize::MAX;
    overflow[1] = 1;
    assert_eq!(
        OwnerDirectoryShape::from_maxima(overflow, false, usize::MAX)
            .err()
            .unwrap()
            .to_string(),
        "glTF source owner directory exceeds usize",
    );
    assert_eq!(
        OwnerDirectoryShape::from_maxima(maxima, false, 5)
            .err()
            .unwrap()
            .to_string(),
        "glTF source owner directory is not compact",
    );
}

#[test]
fn empty_directory_publishes_without_backing_storage() {
    let observation = run(
        |ctx, physical, token| {
            let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
            selected.finish(ctx, &ordinary, &suspended)?;
            assert!(selected.finished);
            let directory = selected.directory.as_ref().unwrap();
            assert_eq!(directory.shape.len(), 0);
            assert!(directory.ranges.is_empty());
            assert_eq!(directory.ranges.capacity(), 0);
            assert!(relation(&selected).is_empty());
            physical.store(1, AtomicOrdering::Relaxed);
            token.cancel();
            ctx.poll()?;
            Ok(())
        },
        None,
        None,
        b"{}",
    );
    assert_cancelled(&observation.failure);
    assert_eq!(observation.physical, 1);
    assert_eq!(observation.directory_covered, 0);
    assert_eq!(
        observation.completed[ComparisonSite::DirectoryInit as usize],
        0
    );
    assert_eq!(
        observation.completed[ComparisonSite::DirectoryRegister as usize],
        0
    );
    assert_eq!(observation.comparisons(), 0);
}

#[test]
fn sparse_and_high_owner_keys_fail_before_directory_allocation() {
    for local in [8_u32, u32::MAX] {
        let primary = serde_json::to_vec(&serde_json::json!({
            "nodes": [{}, {}], "owner_index": local,
        }))
        .unwrap();
        let setup = run(
            |ctx, _, _| {
                let _completed = completed_nodes(ctx)?;
                let input: serde_json::Value = serde_json::from_slice(ctx.primary_bytes()).unwrap();
                let local = input["owner_index"].as_u64().unwrap();
                Err(
                    GltfFailure::internal(if local == u64::from(u32::MAX) && usize::BITS == 32 {
                        "glTF source owner directory exceeds usize"
                    } else {
                        "glTF source owner directory is not compact"
                    })
                    .into(),
                )
            },
            None,
            None,
            &primary,
        );
        assert!(matches!(setup.failure.cause(), ImportCause::Format(_)));
        let mut limits = ResourceLimits::default();
        limits.max_live_bytes = setup.failure.ledger().peak_live_bytes();
        let observation = run_with_assets(
            |ctx, physical, _| {
                let input: serde_json::Value = serde_json::from_slice(ctx.primary_bytes()).unwrap();
                let local = input["owner_index"].as_u64().unwrap() as u32;
                let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                selected.owners.as_mut_slice()[0].owner =
                    ProfileOwner::Node(NodeLocalId::new(local));
                let error = selected
                    .finish(ctx, &ordinary, &suspended)
                    .expect_err("a sparse private key cannot size an allocation");
                assert_eq!(
                    error.to_string(),
                    if local == u32::MAX && usize::BITS == 32 {
                        "glTF source owner directory exceeds usize"
                    } else {
                        "glTF source owner directory is not compact"
                    }
                );
                assert!(!selected.finished);
                assert!(selected.directory.is_none());
                physical.store(1, AtomicOrdering::Relaxed);
                Err(error)
            },
            limits,
            None,
            &primary,
            &[],
        );
        assert!(
            matches!(observation.failure.cause(), ImportCause::Format(_)),
            "{:?}",
            observation.failure.cause()
        );
        assert_eq!(observation.physical, 1);
        assert_eq!(observation.directory_covered, 2);
        assert_eq!(
            observation.completed[ComparisonSite::DirectoryInit as usize],
            0
        );
        assert_eq!(
            observation.completed[ComparisonSite::DirectoryRegister as usize],
            0
        );
        assert_eq!(
            observation.failure.ledger().peak_live_bytes(),
            setup.failure.ledger().peak_live_bytes()
        );
    }
}

#[test]
fn original_range_and_covered_total_errors_precede_an_earlier_high_owner_key() {
    for case in [
        "empty",
        "gap",
        "oversized",
        "overflow",
        "overlap",
        "uncovered",
    ] {
        let primary = serde_json::to_vec(&serde_json::json!({
            "nodes": [{}, {}], "case": case,
        }))
        .unwrap();
        let observation = run(
            |ctx, physical, _| {
                let input: serde_json::Value = serde_json::from_slice(ctx.primary_bytes()).unwrap();
                let case = input["case"].as_str().unwrap();
                let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                selected.owners.as_mut_slice()[0].owner =
                    ProfileOwner::Node(NodeLocalId::new(u32::MAX));
                match case {
                    "empty" => selected.owners.as_mut_slice()[1].length = 0,
                    "gap" => selected.owners.as_mut_slice()[1].start = 2,
                    "oversized" => selected.owners.as_mut_slice()[1].length = 2,
                    "overflow" => selected.owners.as_mut_slice()[1].start = usize::MAX,
                    "overlap" => selected.owners.as_mut_slice()[1].start = 0,
                    "uncovered" => selected
                        .ordinals
                        .try_push(&mut ctx.services()?, SelectedEntryOrdinal(0))?,
                    _ => unreachable!(),
                }
                let error = selected
                    .finish(ctx, &ordinary, &suspended)
                    .expect_err("coverage faults keep their original priority");
                assert_eq!(
                    error.to_string(),
                    if case == "uncovered" {
                        "glTF source owner ranges are incomplete"
                    } else {
                        "glTF source owner range is invalid"
                    }
                );
                assert!(!selected.finished);
                assert!(selected.directory.is_none());
                physical.store(1, AtomicOrdering::Relaxed);
                Err(error)
            },
            None,
            None,
            &primary,
        );
        assert!(matches!(
            observation.failure.cause(),
            ImportCause::Format(_)
        ));
        assert_eq!(observation.physical, 1);
        assert_eq!(
            observation.directory_covered,
            if case == "uncovered" { 2 } else { 1 }
        );
        assert_eq!(
            observation.completed[ComparisonSite::DirectoryInit as usize],
            0
        );
        assert_eq!(
            observation.completed[ComparisonSite::DirectoryRegister as usize],
            0
        );
    }
}

#[test]
fn denied_directory_steps_keep_the_exact_actual_prefix_unpublished() {
    for length in [2, 130] {
        let primary = serde_json::to_vec(&serde_json::json!({
            "nodes": vec![serde_json::json!({}); length],
        }))
        .unwrap();
        let setup = run(
            |ctx, _, token| {
                let _completed = completed_nodes(ctx)?;
                token.cancel();
                ctx.poll()?;
                Ok(())
            },
            None,
            None,
            &primary,
        );
        assert_cancelled(&setup.failure);
        for site in [
            ComparisonSite::DirectoryInit,
            ComparisonSite::DirectoryRegister,
        ] {
            for allowed in [0, 1, 63, 64, 65]
                .into_iter()
                .filter(|&allowed| allowed < length)
            {
                let prior_init = if site == ComparisonSite::DirectoryRegister {
                    length
                } else {
                    0
                };
                let limit = counts(&setup.failure).1
                    + length as u64
                    + 2
                    + prior_init as u64
                    + allowed as u64;
                let observation = run(
                    |ctx, physical, token| {
                        let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                        let original = selected
                            .owners
                            .as_slice()
                            .iter()
                            .map(|range| (range.owner, range.start, range.length))
                            .collect::<Vec<_>>();
                        let error = selected
                            .finish(ctx, &ordinary, &suspended)
                            .expect_err("the current physical step must be denied before mutation");
                        assert!(!selected.finished);
                        assert!(selected.directory.is_none());
                        assert_eq!(
                            selected
                                .owners
                                .as_slice()
                                .iter()
                                .map(|range| (range.owner, range.start, range.length))
                                .collect::<Vec<_>>(),
                            original
                        );
                        physical.store(1, AtomicOrdering::Relaxed);
                        token.cancel();
                        assert_eq!(ctx.poll().unwrap_err().to_string(), error.to_string());
                        Err(error)
                    },
                    Some(limit),
                    None,
                    &primary,
                );
                assert_ledger_denial(&observation.failure);
                assert_eq!(observation.physical, 1);
                assert_eq!(observation.directory_covered, length);
                assert_eq!(observation.completed[site as usize], allowed);
                assert_eq!(
                    observation.completed[ComparisonSite::DirectoryInit as usize],
                    prior_init + if prior_init == 0 { allowed } else { 0 }
                );
                assert_eq!(
                    observation.completed[ComparisonSite::DirectoryRegister as usize],
                    if prior_init == 0 { 0 } else { allowed }
                );
                assert_eq!(counts(&observation.failure), (limit + 1, limit));
                assert_eq!(observation.comparisons(), 0);
            }
        }
    }
}

#[test]
fn construction_unwind_never_publishes_a_searchable_directory() {
    for site in [
        ComparisonSite::DirectoryInit,
        ComparisonSite::DirectoryRegister,
    ] {
        let primary = serde_json::to_vec(&serde_json::json!({
            "nodes": [{}, {}], "registration": site == ComparisonSite::DirectoryRegister,
        }))
        .unwrap();
        let observation = run(
            |ctx, physical, _| {
                let input: serde_json::Value = serde_json::from_slice(ctx.primary_bytes()).unwrap();
                let site = if input["registration"].as_bool().unwrap() {
                    ComparisonSite::DirectoryRegister
                } else {
                    ComparisonSite::DirectoryInit
                };
                let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                COMPLETION_PROBE
                    .with(|slot| slot.borrow_mut().as_mut().unwrap().panic_on = Some((site, 1)));
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    selected.finish(ctx, &ordinary, &suspended)
                }))
                .expect_err("the actual production-loop completion hook must unwind");
                assert!(!selected.finished);
                assert!(selected.directory.is_none());
                assert_eq!(
                    selected.owners.as_slice()[0].owner,
                    ProfileOwner::Node(NodeLocalId::new(1))
                );
                assert_eq!(
                    selected.owners.as_slice()[1].owner,
                    ProfileOwner::Node(NodeLocalId::new(0))
                );
                physical.store(1, AtomicOrdering::Relaxed);
                std::panic::resume_unwind(panic)
            },
            None,
            None,
            &primary,
        );
        assert!(matches!(
            observation.failure.cause(),
            ImportCause::TrustedExtension {
                code: "importer-panicked",
                ..
            }
        ));
        assert_eq!(observation.physical, 1);
        assert_eq!(observation.completed[site as usize], 1);
        assert_eq!(
            observation.completed[ComparisonSite::DirectoryInit as usize],
            if site == ComparisonSite::DirectoryInit {
                1
            } else {
                2
            }
        );
        assert_eq!(
            observation.completed[ComparisonSite::DirectoryRegister as usize],
            if site == ComparisonSite::DirectoryInit {
                0
            } else {
                1
            }
        );
        let (attempted, consumed) = counts(&observation.failure);
        assert_eq!(attempted, consumed);
    }
}

#[test]
fn directory_capacity_is_retained_and_denial_precedes_initialization() {
    let setup = run(
        |ctx, _, token| {
            let (mut selected, mut ordinary) = wide_reserved_document_run(ctx)?;
            selected.complete_owner(ctx, &mut ordinary, ProfileOwner::Scene)?;
            token.cancel();
            ctx.poll()?;
            Ok(())
        },
        None,
        None,
        b"{}",
    );
    assert_cancelled(&setup.failure);
    let completed = run(
        |ctx, physical, token| {
            let (mut selected, mut ordinary) = wide_reserved_document_run(ctx)?;
            selected.complete_owner(ctx, &mut ordinary, ProfileOwner::Scene)?;
            let suspended = OwnerSourceRun::new(ctx)?;
            selected.finish(ctx, &ordinary, &suspended)?;
            assert!(selected.finished);
            let directory = selected.directory.as_ref().unwrap();
            assert_eq!(directory.shape.len(), 1);
            assert_eq!(directory.ranges.as_slice(), &[0]);
            physical.store(
                directory.ranges.capacity() * std::mem::size_of::<usize>(),
                AtomicOrdering::Relaxed,
            );
            token.cancel();
            ctx.poll()?;
            Ok(())
        },
        None,
        None,
        b"{}",
    );
    assert_cancelled(&completed.failure);
    assert!(completed.physical >= std::mem::size_of::<usize>());
    let limit = setup.failure.ledger().peak_live_bytes();
    assert_eq!(
        completed.failure.ledger().peak_live_bytes(),
        limit + completed.physical as u64
    );
    assert_eq!(completed.directory_covered, 1);
    let expected_work = counts(&setup.failure).1
        + completed.directory_covered as u64
        + 2
        + completed.completed[ComparisonSite::DirectoryInit as usize] as u64
        + completed.completed[ComparisonSite::DirectoryRegister as usize] as u64;
    assert_eq!(counts(&completed.failure), (expected_work, expected_work));
    assert_eq!(
        completed.completed[ComparisonSite::DirectoryInit as usize],
        1
    );
    assert_eq!(
        completed.completed[ComparisonSite::DirectoryRegister as usize],
        1
    );
    let mut limits = ResourceLimits::default();
    limits.max_live_bytes = limit;
    let denied = run_with_assets(
        |ctx, physical, _| {
            let (mut selected, mut ordinary) = wide_reserved_document_run(ctx)?;
            selected.complete_owner(ctx, &mut ordinary, ProfileOwner::Scene)?;
            let suspended = OwnerSourceRun::new(ctx)?;
            let error = selected
                .finish(ctx, &ordinary, &suspended)
                .expect_err("the directory allocation needs its own retained allowance");
            assert!(!selected.finished);
            assert!(selected.directory.is_none());
            assert_eq!(selected.ordinals.as_slice(), &[SelectedEntryOrdinal(0)]);
            assert_eq!(selected.owners.len(), 1);
            physical.store(1, AtomicOrdering::Relaxed);
            Err(error)
        },
        limits,
        None,
        b"{}",
        &[],
    );
    assert_ledger_denial(&denied.failure);
    assert_eq!(denied.physical, 1);
    assert_eq!(denied.failure.ledger().peak_live_bytes(), limit);
    assert_eq!(denied.completed[ComparisonSite::DirectoryInit as usize], 0);
    assert_eq!(
        denied.completed[ComparisonSite::DirectoryRegister as usize],
        0
    );
}

#[test]
fn direct_queries_use_one_lookup_and_one_range_validation_for_every_population() {
    for length in [2, 130] {
        let primary = serde_json::to_vec(&serde_json::json!({
            "nodes": vec![serde_json::json!({}); length],
        }))
        .unwrap();
        let setup = run(
            |ctx, _, token| {
                let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                selected.finish(ctx, &ordinary, &suspended)?;
                token.cancel();
                ctx.poll()?;
                Ok(())
            },
            None,
            None,
            &primary,
        );
        assert_cancelled(&setup.failure);
        let observation = run(
            |ctx, _, token| {
                let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                selected.finish(ctx, &ordinary, &suspended)?;
                for index in [0, selected.owners.len() - 1] {
                    assert_eq!(
                        selected
                            .for_owner(ctx, ProfileOwner::Node(NodeLocalId::new(index as u32)))?,
                        &[SelectedEntryOrdinal(index + 1)]
                    );
                }
                token.cancel();
                ctx.poll()?;
                Ok(())
            },
            None,
            None,
            &primary,
        );
        assert_cancelled(&observation.failure);
        assert_eq!(observation.directory_queries, 2);
        assert_eq!(observation.directory_query_work, 2);
        let expected = counts(&setup.failure).1 + 4;
        assert_eq!(counts(&observation.failure), (expected, expected));
        for allowed in [0, 1] {
            let limit = counts(&setup.failure).1 + allowed;
            let denied = run(
                |ctx, physical, _| {
                    let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                    selected.finish(ctx, &ordinary, &suspended)?;
                    let error = selected
                        .for_owner(ctx, ProfileOwner::Node(NodeLocalId::new(0)))
                        .expect_err("both lookup and range validation require current admission");
                    assert!(selected.finished);
                    assert!(selected.directory.is_some());
                    physical.store(1, AtomicOrdering::Relaxed);
                    Err(error)
                },
                Some(limit),
                None,
                &primary,
            );
            assert_ledger_denial(&denied.failure);
            assert_eq!(denied.physical, 1);
            assert_eq!(denied.directory_queries as u64, allowed);
            assert_eq!(denied.directory_query_work, allowed);
            assert_eq!(counts(&denied.failure), (limit + 1, limit));
        }
    }
}

#[test]
fn missing_and_corrupt_directory_queries_fail_closed() {
    for (case, completed_queries) in [
        ("missing-class", 0),
        ("past-end", 0),
        ("sentinel", 0),
        ("invalid-row", 0),
        ("wrong-row", 0),
        ("foreign-owner", 0),
        ("missing-directory", 0),
        ("unfinished-with-directory", 1),
        ("empty-range", 1),
        ("overflow-range", 1),
        ("outside-range", 1),
    ] {
        let primary = serde_json::to_vec(&serde_json::json!({
            "nodes": [{}, {}], "case": case,
        }))
        .unwrap();
        let observation = run(
            |ctx, physical, _| {
                let input: serde_json::Value = serde_json::from_slice(ctx.primary_bytes()).unwrap();
                let case = input["case"].as_str().unwrap();
                let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                selected.finish(ctx, &ordinary, &suspended)?;
                let mut owner = ProfileOwner::Node(NodeLocalId::new(0));
                let directory = selected.directory.as_mut().unwrap();
                let slot = directory.shape.slot(owner).unwrap();
                let row = directory.ranges.as_slice()[slot];
                match case {
                    "missing-class" => owner = ProfileOwner::Scene,
                    "past-end" => owner = ProfileOwner::Node(NodeLocalId::new(2)),
                    "sentinel" => directory.ranges.as_mut_slice()[slot] = usize::MAX,
                    "invalid-row" => directory.ranges.as_mut_slice()[slot] = selected.owners.len(),
                    "wrong-row" => directory.ranges.as_mut_slice()[slot] = 0,
                    "foreign-owner" => {
                        selected.owners.as_mut_slice()[row].owner = ProfileOwner::Sampler(0)
                    }
                    "missing-directory" => selected.directory = None,
                    "unfinished-with-directory" => selected.finished = false,
                    "empty-range" => selected.owners.as_mut_slice()[row].length = 0,
                    "overflow-range" => selected.owners.as_mut_slice()[row].start = usize::MAX,
                    "outside-range" => selected.owners.as_mut_slice()[row].length = 3,
                    _ => unreachable!(),
                }
                let error = selected
                    .for_owner(ctx, owner)
                    .expect_err("a corrupt map cannot authorize a borrowed ordinal slice");
                assert_eq!(
                    error.to_string(),
                    match case {
                        "unfinished-with-directory" | "empty-range" =>
                            "glTF source owner directory is incomplete",
                        "overflow-range" | "outside-range" => "glTF source owner range is invalid",
                        _ => "glTF canonical owner has no source mapping",
                    }
                );
                physical.store(1, AtomicOrdering::Relaxed);
                Err(error)
            },
            None,
            None,
            &primary,
        );
        assert!(
            matches!(observation.failure.cause(), ImportCause::Format(_)),
            "{case}: {:?}",
            observation.failure.cause()
        );
        assert_eq!(observation.physical, 1, "{case}");
        assert_eq!(observation.directory_queries, completed_queries, "{case}");
        assert_eq!(
            observation.directory_query_work, completed_queries as u64,
            "{case}"
        );
    }
}
