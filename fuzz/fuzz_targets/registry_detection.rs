#![no_main]

use baozi_core::{FormatId, SceneDraft};
use baozi_import::{
    CapabilityStatus, DetectionEvidence, DetectionInconclusive, DetectionOptions, DetectionOutcome,
    DetectionResult, DetectionScope, FormatCapability, FormatDescriptor, FormatEncoding,
    FormatImporter, FormatSelection, ImportConfigOverlay, ImportOperationControl, ImportRequest,
    ImportRequestLayer, ImportResult, OptionCompileLimits, ParseResult, ParseScope,
    PreparedImportRequest, Probe, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    ResourceLimits, ResourceLimitsOverlay, import_bytes,
};
use baozi_io::AssetPath;
use libfuzzer_sys::fuzz_target;

const DETECTOR_COUNT: usize = 4;
const MAX_PROBE_BYTES: u64 = 64;
const MAX_PRIMARY_BYTES: usize = 4_096;
const CONTROL_BYTES: usize = 10;
const MAX_INPUT_BYTES: usize = CONTROL_BYTES + MAX_PRIMARY_BYTES;

const DETECTORS: [(&str, &str); DETECTOR_COUNT] = [
    ("org.baozi.fuzz.detector0", "d0"),
    ("org.baozi.fuzz.detector1", "d1"),
    ("org.baozi.fuzz.detector2", "shared"),
    ("org.baozi.fuzz.detector3", "shared"),
];

const SOURCES: [(&str, &str); 4] = [
    ("fuzz.bin", "bin"),
    ("fuzz.d0", "d0"),
    ("fuzz.d1", "d1"),
    ("fuzz.shared", "shared"),
];

fuzz_target!(|data: &[u8]| {
    let case = FuzzCase::decode(data);
    let forward = run_case(&case, RegistrationOrder::Forward);
    assert_expected_observation(&case, &forward);
    let reverse = run_case(&case, RegistrationOrder::Reverse);
    assert_expected_observation(&case, &reverse);

    assert_eq!(
        forward, reverse,
        "registry detection must not depend on registration order"
    );
});

#[derive(Debug, Clone, Copy)]
enum DetectorPlan {
    NoMatch,
    Match(ReadConfidence),
    ProbeByte { offset: u8, expected: u8 },
    Inconclusive,
    Fault,
}

impl DetectorPlan {
    fn decode(opcode: u8, offset: u8, expected: u8) -> Self {
        match opcode % 7 {
            0 => Self::NoMatch,
            1 => Self::Match(ReadConfidence::Maybe),
            2 => Self::Match(ReadConfidence::Likely),
            3 => Self::Match(ReadConfidence::Certain),
            4 => Self::ProbeByte { offset, expected },
            5 => Self::Inconclusive,
            _ => Self::Fault,
        }
    }

    fn detect(self, probe: &Probe<'_>) -> DetectionOutcome {
        match self {
            Self::NoMatch => DetectionOutcome::NoMatch,
            Self::Match(confidence) => DetectionOutcome::Match(confidence),
            Self::ProbeByte { offset, expected } => {
                let offset = usize::from(offset);
                match probe.bytes().get(offset) {
                    Some(actual) if *actual == expected => {
                        DetectionOutcome::Match(ReadConfidence::Certain)
                    }
                    Some(_) => DetectionOutcome::NoMatch,
                    None if u64::from(offset as u32) < probe.source_len() => {
                        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
                    }
                    None => DetectionOutcome::NoMatch,
                }
            }
            Self::Inconclusive => {
                DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
            }
            Self::Fault => DetectionOutcome::fault("synthetic detector fault"),
        }
    }

    fn expected(self, primary: &[u8], max_probe_bytes: u64) -> DetectionOutcome {
        let probe_len = (max_probe_bytes as usize).min(primary.len());
        let probe_bytes = &primary[..probe_len];
        match self {
            Self::NoMatch => DetectionOutcome::NoMatch,
            Self::Match(confidence) => DetectionOutcome::Match(confidence),
            Self::ProbeByte { offset, expected } => {
                let offset = usize::from(offset);
                match probe_bytes.get(offset) {
                    Some(actual) if *actual == expected => {
                        DetectionOutcome::Match(ReadConfidence::Certain)
                    }
                    Some(_) => DetectionOutcome::NoMatch,
                    None if offset < primary.len() => {
                        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
                    }
                    None => DetectionOutcome::NoMatch,
                }
            }
            Self::Inconclusive => {
                DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
            }
            Self::Fault => DetectionOutcome::fault("synthetic detector fault"),
        }
    }
}

struct FuzzCase<'a> {
    plans: [DetectorPlan; DETECTOR_COUNT],
    probe_bytes: u64,
    selection: Selection,
    source: &'static str,
    extension: &'static str,
    primary: &'a [u8],
}

impl<'a> FuzzCase<'a> {
    fn decode(data: &'a [u8]) -> Self {
        let bounded = &data[..data.len().min(MAX_INPUT_BYTES)];
        let byte = |index| bounded.get(index).copied().unwrap_or(0);
        let selection = match byte(1) % 3 {
            0 => Selection::Automatic,
            1 => Selection::Preferred(usize::from(byte(2)) % DETECTOR_COUNT),
            _ => Selection::Forced(usize::from(byte(2)) % DETECTOR_COUNT),
        };
        let primary_offset = bounded.len().min(CONTROL_BYTES);
        let (source, extension) = SOURCES[usize::from(byte(3)) % SOURCES.len()];

        Self {
            plans: [
                DetectorPlan::decode(byte(4), byte(5), byte(6)),
                DetectorPlan::decode(byte(7), byte(8), byte(9)),
                DetectorPlan::Inconclusive,
                DetectorPlan::Fault,
            ],
            probe_bytes: u64::from(byte(0)) % (MAX_PROBE_BYTES + 1),
            selection,
            source,
            extension,
            primary: &bounded[primary_offset..],
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Selection {
    Automatic,
    Preferred(usize),
    Forced(usize),
}

impl Selection {
    fn materialize(self) -> FormatSelection {
        match self {
            Self::Automatic => FormatSelection::Automatic,
            Self::Preferred(index) => FormatSelection::Preferred(format_id(index)),
            Self::Forced(index) => FormatSelection::Forced(format_id(index)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RegistrationOrder {
    Forward,
    Reverse,
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    succeeded: bool,
    selected: Option<FormatId>,
    detection: Vec<DetectionEvidence>,
}

#[derive(Clone)]
struct SyntheticImporter {
    descriptor: FormatDescriptor,
    plan: DetectorPlan,
}

impl SyntheticImporter {
    fn new(index: usize, plan: DetectorPlan) -> Self {
        let (id, extension) = DETECTORS[index];
        let descriptor = FormatDescriptor::builder(
            FormatId::new(id).expect("synthetic format IDs are valid"),
            "Fuzz Detector",
        )
        .extensions([extension])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .expect("synthetic descriptors are valid");
        Self { descriptor, plan }
    }
}

impl FormatImporter for SyntheticImporter {
    fn descriptor(&self) -> FormatDescriptor {
        self.descriptor.clone()
    }

    fn detect(
        &self,
        probe: &Probe<'_>,
        _hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        let permit = match self.plan {
            DetectorPlan::ProbeByte { offset, .. }
                if probe.bytes().get(usize::from(offset)).is_some() =>
            {
                Some(scope.begin_work(1)?)
            }
            _ => None,
        };
        let outcome = self.plan.detect(probe);
        if let Some(permit) = permit {
            permit.complete()?;
        }
        Ok(outcome)
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        scope.scene_builder()?.finish_draft()
    }
}

fn run_case(case: &FuzzCase<'_>, order: RegistrationOrder) -> Observation {
    let request = prepared_request(case, order);
    let first = import_observation(&request, case);
    let second = import_observation(&request, case);
    assert_eq!(
        first, second,
        "detector-owned fault evidence must not poison later sessions"
    );
    first
}

fn import_observation(request: &PreparedImportRequest, case: &FuzzCase<'_>) -> Observation {
    let source = AssetPath::new(case.source).expect("synthetic source paths are valid");
    observe(import_bytes(
        &request,
        source,
        case.primary,
        ImportOperationControl::default(),
    ))
}

fn prepared_request(case: &FuzzCase<'_>, order: RegistrationOrder) -> PreparedImportRequest {
    let mut registry = RegistryBuilder::new();
    match order {
        RegistrationOrder::Forward => {
            for index in 0..DETECTOR_COUNT {
                registry
                    .register(SyntheticImporter::new(index, case.plans[index]))
                    .expect("synthetic importer registration succeeds");
            }
        }
        RegistrationOrder::Reverse => {
            for index in (0..DETECTOR_COUNT).rev() {
                registry
                    .register(SyntheticImporter::new(index, case.plans[index]))
                    .expect("synthetic importer registration succeeds");
            }
        }
    }

    let snapshot = registry
        .freeze(RegistryBuildLimits::default())
        .expect("synthetic registry freezes");
    let detection = DetectionOptions::new(case.probe_bytes).expect("bounded probe limit is valid");
    snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(
                    ImportConfigOverlay::default()
                        .with_resource_limits(ResourceLimitsOverlay::from_limits(resource_limits()))
                        .with_detection_options(detection)
                        .with_format_selection(case.selection.materialize()),
                ),
            ),
            OptionCompileLimits::default(),
        )
        .expect("selected synthetic format is registered")
}

fn resource_limits() -> ResourceLimits {
    let mut limits = ResourceLimits::default();
    limits.max_primary_asset_bytes = MAX_PRIMARY_BYTES as u64;
    limits.max_total_asset_bytes = MAX_PRIMARY_BYTES as u64;
    limits.max_open_assets = 1;
    limits.max_dependency_references = 0;
    limits.max_live_bytes = 2 * 1024 * 1024;
    limits.max_work_units = 65_536;
    limits.max_detection_work = 65_536;
    limits.max_canonical_payload_bytes = 1024 * 1024;
    limits.max_graph_nodes = 16;
    limits.max_graph_edges = 32;
    limits.max_graph_depth = 16;
    limits.max_graph_work = 256;
    limits
}

fn observe(result: ImportResult<baozi_import::ImportReport>) -> Observation {
    let (succeeded, selected, detection) = match result {
        Ok(report) => (
            true,
            Some(report.format().id().clone()),
            clone_evidence(report.detection_evidence()),
        ),
        Err(error) => match error.operation_failure() {
            Some(failure) => (
                false,
                failure.selected_format().cloned(),
                clone_evidence(failure.detection_evidence()),
            ),
            None => (false, None, Vec::new()),
        },
    };
    Observation {
        succeeded,
        selected,
        detection,
    }
}

fn clone_evidence(evidence: &[DetectionEvidence]) -> Vec<DetectionEvidence> {
    let mut detection = Vec::new();
    detection
        .try_reserve_exact(evidence.len())
        .expect("bounded detection evidence allocation succeeds");
    detection.extend(evidence.iter().cloned());
    detection
}

fn assert_expected_observation(case: &FuzzCase<'_>, observation: &Observation) {
    let outcomes = case
        .plans
        .map(|plan| plan.expected(case.primary, case.probe_bytes));

    match case.selection {
        Selection::Forced(index) => {
            assert_eq!(observation.detection.len(), 1);
            assert_evidence(&observation.detection[0], index, &outcomes[index]);
        }
        Selection::Automatic | Selection::Preferred(_) => {
            assert_eq!(observation.detection.len(), DETECTOR_COUNT);
            for (index, evidence) in observation.detection.iter().enumerate() {
                assert_evidence(evidence, index, &outcomes[index]);
            }
        }
    }

    let expected = expected_selected(case, &outcomes);
    assert_eq!(observation.succeeded, expected.is_some());
    assert_eq!(
        observation.selected.as_ref().map(FormatId::as_str),
        expected.map(|index| DETECTORS[index].0)
    );
}

fn assert_evidence(evidence: &DetectionEvidence, index: usize, expected: &DetectionOutcome) {
    assert_eq!(evidence.format().as_str(), DETECTORS[index].0);
    assert_eq!(evidence.outcome(), expected);
}

fn expected_selected(
    case: &FuzzCase<'_>,
    outcomes: &[DetectionOutcome; DETECTOR_COUNT],
) -> Option<usize> {
    if let Selection::Forced(index) = case.selection {
        return match &outcomes[index] {
            DetectionOutcome::Match(_) | DetectionOutcome::Inconclusive(_) => Some(index),
            DetectionOutcome::NoMatch | DetectionOutcome::Fault(_) => None,
        };
    }

    let strongest = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            DetectionOutcome::Match(confidence) if *confidence >= ReadConfidence::Likely => {
                Some(*confidence)
            }
            _ => None,
        })
        .max();
    let selected_confidence = strongest.unwrap_or(ReadConfidence::Maybe);
    let mut candidates = [false; DETECTOR_COUNT];
    let mut candidate_count = 0;
    for (index, outcome) in outcomes.iter().enumerate() {
        if matches!(outcome, DetectionOutcome::Match(confidence) if *confidence == selected_confidence)
        {
            candidates[index] = true;
            candidate_count += 1;
        }
    }
    if candidate_count == 0 {
        return None;
    }
    if strongest.is_some() && candidate_count == 1 {
        return candidates.iter().position(|candidate| *candidate);
    }
    if let Selection::Preferred(index) = case.selection
        && candidates[index]
    {
        return Some(index);
    }

    let mut extension_match = None;
    let mut extension_match_count = 0;
    for (index, candidate) in candidates.iter().enumerate() {
        if *candidate && DETECTORS[index].1.eq_ignore_ascii_case(case.extension) {
            extension_match = Some(index);
            extension_match_count += 1;
        }
    }
    if extension_match_count == 1 {
        extension_match
    } else {
        None
    }
}

fn format_id(index: usize) -> FormatId {
    FormatId::new(DETECTORS[index].0).expect("synthetic format IDs are valid")
}
