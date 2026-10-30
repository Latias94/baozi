use super::*;

use crate::canonical_scene::store::production::ProfileOwnerCounts;

#[derive(Default)]
struct Control {
    completed: usize,
    reject_before: Option<usize>,
    stop_before: Option<usize>,
}

impl TypedStorageValidationControl for Control {
    type Error = usize;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        if self.reject_before == Some(self.completed) {
            return Err(self.completed);
        }
        if self.stop_before == Some(self.completed) {
            return Ok(None);
        }
        let result = action();
        self.completed += 1;
        Ok(Some(result))
    }
}

fn text(pool: &mut String, value: &str) -> TextRange {
    let range = TextRange(TableRange::new(pool.len(), value.len()));
    pool.push_str(value);
    range
}

fn domain(lineage: &LineageSegment) -> ProfileSegmentAddressDomain {
    ProfileSegmentAddressDomain::new(0, lineage.nonce, 0, lineage.provenance.len() as u32)
}

fn lineage() -> LineageSegment {
    let mut pool = String::new();
    let resource = StoredResource {
        identity: text(&mut pool, "models/scene.glb"),
        stability: StoredResourceStability::Stable {
            owner: text(&mut pool, "test:loader"),
            schema: text(&mut pool, "test:resource"),
            version: NonZeroU32::new(1).unwrap(),
        },
    };
    let sources = vec![
        StoredSource {
            resource: 0,
            format: text(&mut pool, "org.baozi.gltf"),
            locator: StoredSourceLocator::Indexed {
                kind: SourceObjectKind::Document,
                index: u64::MAX,
            },
            display_name: Some(text(&mut pool, "Document")),
            content_digest: Some(ContentDigest::sha256([9; DIGEST_LEN])),
        },
        StoredSource {
            resource: 0,
            format: text(&mut pool, "org.baozi.gltf"),
            locator: StoredSourceLocator::Namespaced {
                schema: text(&mut pool, "test:locator"),
                version: NonZeroU32::new(2).unwrap(),
                value: text(&mut pool, "object 2"),
            },
            display_name: None,
            content_digest: None,
        },
    ];
    let kind = StoredOperationKind::Namespaced(text(&mut pool, "test:normalize"));
    let evidence = vec![
        StoredOperationEvidence {
            code: text(&mut pool, "test:a"),
            occurrences: NonZeroU64::new(u64::MAX).unwrap(),
            digest: Some(EvidenceDigest::blake3([7; DIGEST_LEN])),
        },
        StoredOperationEvidence {
            code: text(&mut pool, "test:z"),
            occurrences: NonZeroU64::new(1).unwrap(),
            digest: None,
        },
    ];
    let operations = vec![
        StoredOperation {
            kind,
            options: OptionsDigest::sha256([1; DIGEST_LEN]),
            evidence: TableRange::new(0, 2),
        },
        StoredOperation {
            kind: StoredOperationKind::Convert,
            options: OptionsDigest::blake3([2; DIGEST_LEN]),
            evidence: TableRange::new(2, 0),
        },
        StoredOperation {
            kind: StoredOperationKind::Generated,
            options: OptionsDigest::sha256([3; DIGEST_LEN]),
            evidence: TableRange::new(2, 0),
        },
    ];
    let mut lineage = LineageSegment {
        nonce: ProfileSegmentNonce::from_stored(41).unwrap(),
        resources: vec![resource],
        sources,
        provenance: vec![
            StoredProvenance {
                kind: StoredProvenanceKind::Source(0),
                depth: 1,
                parents: TableRange::new(0, 0),
            },
            StoredProvenance {
                kind: StoredProvenanceKind::Source(1),
                depth: 1,
                parents: TableRange::new(0, 0),
            },
            StoredProvenance {
                kind: StoredProvenanceKind::Operation(0),
                depth: 2,
                parents: TableRange::new(0, 2),
            },
            StoredProvenance {
                kind: StoredProvenanceKind::Operation(1),
                depth: 3,
                parents: TableRange::new(2, 1),
            },
            StoredProvenance {
                kind: StoredProvenanceKind::Operation(2),
                depth: 1,
                parents: TableRange::new(3, 0),
            },
        ],
        operations,
        evidence,
        parents: vec![],
        text: pool,
        wide_value_count: 3,
        digest_byte_count: 5 * DIGEST_LEN,
    };
    let domain = domain(&lineage);
    lineage.parents = vec![
        domain.reference(0).unwrap(),
        domain.reference(1).unwrap(),
        domain.reference(2).unwrap(),
    ];
    lineage
}

fn validate_test_lineage(
    lineage: &LineageSegment,
    control: &mut Control,
    maximum_depth: u32,
    depths: &mut [u32],
) -> Result<Option<u32>, usize> {
    validate_lineage(
        control,
        lineage,
        &[],
        0,
        LineageCounts::default(),
        maximum_depth,
        depths,
    )
}

#[test]
fn lineage_validation_borrows_pools_and_overwrites_depths_in_linear_work() {
    let lineage = lineage();
    let mut control = Control::default();
    let mut depths = [u32::MAX; 5];
    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            validate_test_lineage(&lineage, &mut control, 3, &mut depths),
            Ok(Some(3)),
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(depths, [1, 1, 2, 3, 1]);
    assert_eq!(
        control.completed,
        lineage.text.len() + 1 + 2 + 2 * 5 + 3 + 2 + 3 + 1,
    );
    assert_eq!(
        validate_test_lineage(&lineage, &mut Control::default(), 2, &mut depths),
        Ok(None),
    );
}

#[test]
fn every_lineage_action_observes_control_before_reading_or_updating_depths() {
    let lineage = lineage();
    let mut completed = Control::default();
    assert_eq!(
        validate_test_lineage(&lineage, &mut completed, 3, &mut [0; 5]),
        Ok(Some(3)),
    );
    for position in 0..completed.completed {
        let mut reject = Control {
            reject_before: Some(position),
            ..Control::default()
        };
        let mut stop = Control {
            stop_before: Some(position),
            ..Control::default()
        };
        assert_eq!(
            validate_test_lineage(&lineage, &mut reject, 3, &mut [0; 5]),
            Err(position),
        );
        assert_eq!(
            validate_test_lineage(&lineage, &mut stop, 3, &mut [0; 5]),
            Ok(None),
        );
        assert_eq!(reject.completed, position);
        assert_eq!(stop.completed, position);
    }
}

#[test]
fn lineage_rejects_graph_aliases_foreign_nonces_bad_grammar_and_false_inline_totals() {
    let corruptions: [fn(&mut LineageSegment); 22] = [
        |lineage| lineage.sources[0].resource = 1,
        |lineage| lineage.provenance[0].kind = StoredProvenanceKind::Source(2),
        |lineage| lineage.provenance[0].depth = 0,
        |lineage| lineage.provenance[0].parents = TableRange::new(0, 1),
        |lineage| lineage.provenance[2].kind = StoredProvenanceKind::Operation(1),
        |lineage| lineage.provenance[3].depth = 2,
        |lineage| lineage.provenance[3].parents = TableRange::new(1, 1),
        |lineage| lineage.parents[1] = lineage.parents[0],
        |lineage| lineage.parents.swap(0, 1),
        |lineage| lineage.parents[0] = domain(lineage).reference(2).unwrap(),
        |lineage| {
            let foreign = ProfileSegmentAddressDomain::new(
                0,
                ProfileSegmentNonce::from_stored(42).unwrap(),
                0,
                5,
            );
            lineage.parents[0] = foreign.reference(0).unwrap();
        },
        |lineage| {
            let future = ProfileSegmentAddressDomain::new(1, lineage.nonce, 0, 5);
            lineage.parents[0] = future.reference(0).unwrap();
        },
        |lineage| lineage.operations[1].evidence = TableRange::new(0, 0),
        |lineage| lineage.evidence[1].code = lineage.evidence[0].code,
        |lineage| {
            let range = lineage.evidence[1]
                .code
                .0
                .checked(lineage.text.len())
                .unwrap();
            lineage.text.replace_range(range, "test:a");
        },
        |lineage| {
            let range = lineage.sources[0].format.0;
            lineage
                .text
                .replace_range(range.start()..range.start() + 1, "G");
        },
        |lineage| {
            let start = lineage.sources[0].format.0.start();
            lineage.text.replace_range(start + 3..start + 4, ":");
        },
        |lineage| {
            lineage.text.replace_range(0..1, "/");
        },
        |lineage| {
            let StoredSourceLocator::Namespaced { value, .. } = lineage.sources[1].locator else {
                panic!("locator fixture");
            };
            lineage
                .text
                .replace_range(value.0.start()..value.0.start() + 1, "/");
        },
        |lineage| {
            let range = lineage.sources[0].display_name.unwrap().0;
            lineage
                .text
                .replace_range(range.start()..range.start() + 1, "\n");
        },
        |lineage| lineage.wide_value_count += 1,
        |lineage| lineage.digest_byte_count -= DIGEST_LEN,
    ];
    for (index, corrupt) in corruptions.into_iter().enumerate() {
        let mut lineage = lineage();
        corrupt(&mut lineage);
        assert_eq!(
            validate_test_lineage(&lineage, &mut Control::default(), 3, &mut [0; 5]),
            Ok(None),
            "corruption {index}",
        );
    }
}

fn owner_layout() -> ProfileOwnerLayout {
    ProfileOwnerLayout::try_new(ProfileOwnerCounts {
        nodes: 1,
        cameras: 0,
        lights: 0,
        images: 0,
        samplers: 0,
        textures: 0,
        texture_bindings: 0,
        materials: 0,
        logical_meshes: 0,
        primitives: 0,
        attributes: 0,
        mesh_instances: 0,
        skins: 0,
        morph_definitions: 0,
        morph_deltas: 0,
        animation_clips: 0,
        animation_channels: 0,
    })
    .unwrap()
}

fn metadata() -> (Vec<StoredMetadataEntry>, MetadataPayloads) {
    let mut pool = String::new();
    let mut entry = |key, value| StoredMetadataEntry {
        owner: CanonicalProfileOwner::Scene,
        key: text(&mut pool, key),
        value,
    };
    let mut entries = vec![
        entry("test:a", StoredMetadataValue::Bool(TableRange::new(0, 1))),
        entry(
            "test:b",
            StoredMetadataValue::I64Array(TableRange::new(0, 2)),
        ),
        entry("test:c", StoredMetadataValue::U64(TableRange::new(0, 1))),
        entry("test:d", StoredMetadataValue::F64(TableRange::new(0, 1))),
    ];
    let key = text(&mut pool, "test:e");
    let value = StoredMetadataValue::String(text(&mut pool, "hello \u{4e16}\u{754c}"));
    entries.push(StoredMetadataEntry {
        owner: CanonicalProfileOwner::Scene,
        key,
        value,
    });
    let key = text(&mut pool, "test:f");
    let value = StoredMetadataValue::Bytes {
        media_type: text(&mut pool, "application/octet-stream"),
        payload: TableRange::new(0, 3),
    };
    entries.push(StoredMetadataEntry {
        owner: CanonicalProfileOwner::Scene,
        key,
        value,
    });
    for (key, value) in [
        ("test:g", StoredMetadataValue::Vec2(TableRange::new(0, 2))),
        (
            "test:h",
            StoredMetadataValue::Vec3Array(TableRange::new(2, 6)),
        ),
        (
            "test:i",
            StoredMetadataValue::Mat4Array(TableRange::new(8, 16)),
        ),
    ] {
        entries.push(StoredMetadataEntry {
            owner: CanonicalProfileOwner::Scene,
            key: text(&mut pool, key),
            value,
        });
    }
    entries.push(StoredMetadataEntry {
        owner: CanonicalProfileOwner::Node(1),
        key: text(&mut pool, "test:a"),
        value: StoredMetadataValue::BoolArray(TableRange::new(1, 0)),
    });
    (
        entries,
        MetadataPayloads {
            text: pool,
            bytes: vec![0, 255, 19],
            bools: vec![1],
            i64s: vec![i64::MIN, i64::MAX],
            u64s: vec![u64::MAX],
            f64s: vec![-1.0e200],
            f32s: vec![0.5; 24],
        },
    )
}

#[test]
fn metadata_visits_actual_payloads_once_with_no_scratch_allocation() {
    let (entries, pools) = metadata();
    let owners = owner_layout();
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            validate_metadata(&mut control, &entries, &pools, owners),
            Ok(true),
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(
        control.completed,
        entries.len() + pools.text.len() + 3 + 1 + 2 + 1 + 1 + 24 + 1,
    );
    for position in 0..control.completed {
        let mut reject = Control {
            reject_before: Some(position),
            ..Control::default()
        };
        assert_eq!(
            validate_metadata(&mut reject, &entries, &pools, owners),
            Err(position),
        );
        assert_eq!(reject.completed, position);
    }
}

#[test]
fn metadata_rejects_foreign_owners_aliases_gaps_bad_shapes_and_nonfinite_values() {
    type Corruption = fn(&mut [StoredMetadataEntry], &mut MetadataPayloads);
    let corruptions: [Corruption; 15] = [
        |entries, _| entries[0].owner = CanonicalProfileOwner::Camera(0),
        |entries, _| entries[9].owner = CanonicalProfileOwner::Node(0),
        |entries, _| entries[0].value = StoredMetadataValue::Bool(TableRange::new(0, 0)),
        |entries, _| entries[1].value = StoredMetadataValue::I64Array(TableRange::new(1, 1)),
        |entries, _| entries[6].value = StoredMetadataValue::Vec2(TableRange::new(0, 1)),
        |entries, _| entries[7].value = StoredMetadataValue::Vec3Array(TableRange::new(2, 5)),
        |entries, _| entries[8].value = StoredMetadataValue::Mat4Array(TableRange::new(8, 15)),
        |_, pools| pools.bools[0] = 2,
        |_, pools| pools.f32s[17] = f32::NAN,
        |_, pools| pools.f64s[0] = f64::INFINITY,
        |_, pools| pools.bytes.push(0),
        |entries, _| entries[1].key = entries[0].key,
        |entries, pools| {
            let range = entries[1].key.0.checked(pools.text.len()).unwrap();
            pools.text.replace_range(range, "test:a");
        },
        |entries, _| entries[1].owner = CanonicalProfileOwner::Node(1),
        |entries, pools| {
            let StoredMetadataValue::Bytes { media_type, .. } = entries[5].value else {
                panic!("bytes fixture");
            };
            let start = media_type.0.start();
            pools.text.replace_range(start..start + 1, "A");
        },
    ];
    let owners = owner_layout();
    for (index, corrupt) in corruptions.into_iter().enumerate() {
        let (mut entries, mut pools) = metadata();
        corrupt(&mut entries, &mut pools);
        assert_eq!(
            validate_metadata(&mut Control::default(), &entries, &pools, owners),
            Ok(false),
            "corruption {index}",
        );
    }
}

#[test]
fn metadata_string_ranges_must_end_on_character_boundaries() {
    let (mut entries, pools) = metadata();
    let StoredMetadataValue::String(range) = entries[4].value else {
        panic!("string fixture");
    };
    entries[4].value = StoredMetadataValue::String(TextRange(TableRange::new(
        range.0.start(),
        range.0.len() - 1,
    )));
    assert_eq!(
        validate_metadata(&mut Control::default(), &entries, &pools, owner_layout()),
        Ok(false),
    );
}
