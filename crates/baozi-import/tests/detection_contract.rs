use baozi_core::{Result, Scene, SceneBuilder};
use baozi_import::{
    DetectionEvidence, DetectionFailure, DetectionInconclusive, DetectionOptions, DetectionOutcome,
    FormatImporter, FormatInfo, ImportContext, ImporterRegistry, Probe, ReadConfidence, ReadHint,
};
use baozi_io::AssetPath;
use std::io::Cursor;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy)]
enum Behavior {
    Match(ReadConfidence),
    Prefix(&'static [u8]),
    Fault(&'static str),
    Panic,
    NoMatch,
}

struct TestImporter {
    id: &'static str,
    behavior: Behavior,
}

struct UnstableInfoImporter {
    info_calls: Arc<AtomicUsize>,
}

impl FormatImporter for UnstableInfoImporter {
    fn info(&self) -> FormatInfo {
        if self.info_calls.fetch_add(1, Ordering::SeqCst) != 0 {
            panic!("format metadata must only be read during registration");
        }
        FormatInfo::new("stable", "Stable", &["stable"])
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
        SceneBuilder::new().finish()
    }
}

impl TestImporter {
    const fn new(id: &'static str, behavior: Behavior) -> Self {
        Self { id, behavior }
    }
}

impl FormatImporter for TestImporter {
    fn info(&self) -> FormatInfo {
        FormatInfo::new(self.id, self.id, &[])
    }

    fn can_read(&self, probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        match self.behavior {
            Behavior::Match(confidence) => DetectionOutcome::Match(confidence),
            Behavior::Prefix(prefix) if probe.bytes().starts_with(prefix) => {
                DetectionOutcome::Match(ReadConfidence::Certain)
            }
            Behavior::Prefix(prefix)
                if !probe.is_complete() && prefix.starts_with(probe.bytes()) =>
            {
                DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
            }
            Behavior::Prefix(_) | Behavior::NoMatch => DetectionOutcome::NoMatch,
            Behavior::Fault(message) => DetectionOutcome::fault(message),
            Behavior::Panic => panic!("broken detector"),
        }
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
        SceneBuilder::new().finish()
    }
}

fn hint() -> ReadHint {
    ReadHint::from_source(AssetPath::new("mesh.bin").expect("valid test path"))
}

fn detect(
    importers: impl IntoIterator<Item = TestImporter>,
    bytes: &[u8],
    max_probe_bytes: u64,
) -> std::result::Result<String, DetectionFailure> {
    detect_with_evidence(importers, bytes, max_probe_bytes).map(|(format_id, _)| format_id)
}

fn detect_with_evidence(
    importers: impl IntoIterator<Item = TestImporter>,
    bytes: &[u8],
    max_probe_bytes: u64,
) -> std::result::Result<(String, Vec<DetectionEvidence>), DetectionFailure> {
    let mut registry = ImporterRegistry::new();
    for importer in importers {
        registry.register(importer).expect("unique test importer");
    }
    let mut input = Cursor::new(bytes.to_vec());
    registry
        .detect_with_options(&mut input, &hint(), &DetectionOptions { max_probe_bytes })
        .map(|selected| (selected.info.id().to_owned(), selected.evidence().to_vec()))
}

fn for_each_permutation<T>(values: &mut [T], fixed: usize, visitor: &mut impl FnMut(&[T])) {
    if fixed == values.len() {
        visitor(values);
        return;
    }

    for swap_index in fixed..values.len() {
        values.swap(fixed, swap_index);
        for_each_permutation(values, fixed + 1, visitor);
        values.swap(fixed, swap_index);
    }
}

fn assert_evidence(evidence: &[DetectionEvidence], expected: &[(&'static str, DetectionOutcome)]) {
    assert_eq!(
        evidence
            .iter()
            .map(|item| (item.format_id(), item.outcome().clone()))
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn probe_budget_boundaries_are_typed_and_exact() {
    for budget in [0, 4] {
        let error = detect(
            [TestImporter::new("prefix", Behavior::Prefix(b"baozi"))],
            b"baozi payload",
            budget,
        )
        .expect_err("an incomplete prefix must not be rejected as a no-match");

        let DetectionFailure::NoMatch { evidence, .. } = error else {
            panic!("expected structured no-match evidence");
        };
        assert!(matches!(
            evidence.as_slice(),
            [evidence]
                if evidence.outcome()
                    == &DetectionOutcome::Inconclusive(
                        DetectionInconclusive::ProbeExhausted
                    )
        ));
    }

    for budget in [5, 6, u64::MAX] {
        assert_eq!(
            detect(
                [TestImporter::new("prefix", Behavior::Prefix(b"baozi"),)],
                b"baozi payload",
                budget,
            )
            .expect("exact or larger budgets reveal the signature"),
            "prefix"
        );
    }
}

#[test]
fn strongest_match_wins_despite_faults_and_exhausted_probes_in_every_order() {
    let mut importer_specs = [
        ("fault", Behavior::Fault("broken detector")),
        (
            "greedy",
            Behavior::Prefix(b"this signature is beyond the budget"),
        ),
        ("panic", Behavior::Panic),
        ("valid", Behavior::Match(ReadConfidence::Certain)),
    ];
    let mut permutations = 0;
    for_each_permutation(&mut importer_specs, 0, &mut |specs| {
        permutations += 1;
        let importers = specs
            .iter()
            .copied()
            .map(|(id, behavior)| TestImporter::new(id, behavior));
        let (format_id, evidence) =
            detect_with_evidence(importers, b"this is a large primary asset", 4)
                .expect("local detector failures must not poison a valid match");

        assert_eq!(format_id, "valid");
        assert_evidence(
            &evidence,
            &[
                ("fault", DetectionOutcome::fault("broken detector")),
                (
                    "greedy",
                    DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted),
                ),
                ("panic", DetectionOutcome::fault("detector panicked")),
                ("valid", DetectionOutcome::Match(ReadConfidence::Certain)),
            ],
        );
    });
    assert_eq!(permutations, 24);
}

#[test]
fn selected_importer_preserves_complete_sorted_evidence() {
    let (format_id, evidence) = detect_with_evidence(
        [
            TestImporter::new("z-valid", Behavior::Match(ReadConfidence::Certain)),
            TestImporter::new("a-fault", Behavior::Fault("broken detector")),
            TestImporter::new("m-greedy", Behavior::Prefix(b"long signature")),
        ],
        b"long payload",
        2,
    )
    .expect("the valid match wins");

    assert_eq!(format_id, "z-valid");
    assert_eq!(
        evidence
            .iter()
            .map(DetectionEvidence::format_id)
            .collect::<Vec<_>>(),
        ["a-fault", "m-greedy", "z-valid"]
    );
    assert!(matches!(evidence[0].outcome(), DetectionOutcome::Fault(_)));
    assert!(matches!(
        evidence[1].outcome(),
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    ));
    assert_eq!(
        evidence[2].outcome(),
        &DetectionOutcome::Match(ReadConfidence::Certain)
    );
}

#[test]
fn no_candidate_returns_sorted_fault_and_inconclusive_evidence() {
    let error = detect(
        [
            TestImporter::new("z-fault", Behavior::Fault("broken detector")),
            TestImporter::new("a-greedy", Behavior::Prefix(b"long signature")),
            TestImporter::new("m-no", Behavior::NoMatch),
        ],
        b"long payload",
        2,
    )
    .expect_err("no detector produced a match");

    let DetectionFailure::NoMatch { evidence, .. } = error else {
        panic!("expected structured no-match evidence");
    };
    assert_eq!(
        evidence
            .iter()
            .map(|item| item.format_id())
            .collect::<Vec<_>>(),
        ["a-greedy", "m-no", "z-fault"]
    );
    assert!(matches!(
        evidence[0].outcome(),
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    ));
    assert_eq!(evidence[1].outcome(), &DetectionOutcome::NoMatch);
    assert!(matches!(evidence[2].outcome(), DetectionOutcome::Fault(_)));
}

#[test]
fn ambiguous_candidates_are_sorted_independent_of_registration_order() {
    let mut importer_specs = [
        ("zeta", Behavior::Match(ReadConfidence::Likely)),
        ("alpha", Behavior::Match(ReadConfidence::Likely)),
        ("a-fault", Behavior::Fault("broken detector")),
        ("m-greedy", Behavior::Prefix(b"long signature")),
        ("p-panic", Behavior::Panic),
    ];
    let mut permutations = 0;
    for_each_permutation(&mut importer_specs, 0, &mut |specs| {
        permutations += 1;
        let error = detect(
            specs
                .iter()
                .copied()
                .map(|(id, behavior)| TestImporter::new(id, behavior)),
            b"long payload",
            2,
        )
        .expect_err("equal content matches are ambiguous");

        let DetectionFailure::Ambiguous {
            candidates,
            evidence,
            ..
        } = error
        else {
            panic!("expected a structured ambiguity");
        };
        assert_eq!(candidates, ["alpha", "zeta"]);
        assert_evidence(
            &evidence,
            &[
                ("a-fault", DetectionOutcome::fault("broken detector")),
                ("alpha", DetectionOutcome::Match(ReadConfidence::Likely)),
                (
                    "m-greedy",
                    DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted),
                ),
                ("p-panic", DetectionOutcome::fault("detector panicked")),
                ("zeta", DetectionOutcome::Match(ReadConfidence::Likely)),
            ],
        );
    });
    assert_eq!(permutations, 120);
}

#[test]
fn registration_freezes_importer_metadata() {
    let info_calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ImporterRegistry::new();
    registry
        .register(UnstableInfoImporter {
            info_calls: Arc::clone(&info_calls),
        })
        .expect("registration reads metadata once");

    assert_eq!(
        registry.formats().next().expect("registered format").id(),
        "stable"
    );
    assert_eq!(registry.by_extension("stable").len(), 1);
    let mut input = Cursor::new(b"payload".to_vec());
    let selected = registry
        .detect(&mut input, &hint())
        .expect("detection uses frozen metadata");

    assert_eq!(selected.info.id(), "stable");
    assert_eq!(info_calls.load(Ordering::SeqCst), 1);
}
