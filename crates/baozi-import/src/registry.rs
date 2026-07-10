use crate::{
    DetectionOptions, DetectionOutcome, FormatImporter, FormatInfo, Probe, ReadConfidence, ReadHint,
};
use baozi_core::{BaoziError, Result};
use baozi_io::ReadSeek;
use std::fmt;
use std::io::SeekFrom;

pub type DetectionResult<T> = std::result::Result<T, DetectionFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimaryProbeOperation {
    Measure,
    Rewind,
    Allocate,
    Read,
}

impl fmt::Display for PrimaryProbeOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Measure => "measure",
            Self::Rewind => "rewind",
            Self::Allocate => "allocate",
            Self::Read => "read",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionEvidence {
    format_id: &'static str,
    outcome: DetectionOutcome,
}

impl DetectionEvidence {
    pub const fn format_id(&self) -> &'static str {
        self.format_id
    }

    pub const fn outcome(&self) -> &DetectionOutcome {
        &self.outcome
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionFailure {
    PrimaryFault {
        hint: String,
        operation: PrimaryProbeOperation,
        message: String,
    },
    NoMatch {
        hint: String,
        evidence: Vec<DetectionEvidence>,
    },
    Ambiguous {
        hint: String,
        candidates: Vec<String>,
        evidence: Vec<DetectionEvidence>,
    },
}

impl fmt::Display for DetectionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrimaryFault {
                hint,
                operation,
                message,
            } => write!(
                formatter,
                "primary format probe failed to {operation} {hint}: {message}"
            ),
            Self::NoMatch { hint, evidence } => {
                let faults = evidence
                    .iter()
                    .filter(|item| matches!(item.outcome, DetectionOutcome::Fault(_)))
                    .count();
                let inconclusive = evidence
                    .iter()
                    .filter(|item| matches!(item.outcome, DetectionOutcome::Inconclusive(_)))
                    .count();
                write!(
                    formatter,
                    "no format matched {hint} ({faults} detector faults, {inconclusive} inconclusive probes)"
                )
            }
            Self::Ambiguous {
                hint, candidates, ..
            } => {
                write!(
                    formatter,
                    "ambiguous format for {hint}: {}",
                    candidates.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for DetectionFailure {}

#[derive(Default)]
pub struct ImporterRegistry {
    importers: Vec<RegisteredImporter>,
}

struct RegisteredImporter {
    importer: Box<dyn FormatImporter>,
    info: FormatInfo,
}

#[derive(Clone)]
pub struct SelectedImporter<'a> {
    pub importer: &'a dyn FormatImporter,
    pub info: FormatInfo,
    pub confidence: ReadConfidence,
    evidence: Vec<DetectionEvidence>,
}

impl SelectedImporter<'_> {
    pub fn evidence(&self) -> &[DetectionEvidence] {
        &self.evidence
    }
}

impl<'a> SelectedImporter<'a> {
    pub fn into_parts(
        self,
    ) -> (
        &'a dyn FormatImporter,
        FormatInfo,
        ReadConfidence,
        Vec<DetectionEvidence>,
    ) {
        (self.importer, self.info, self.confidence, self.evidence)
    }
}

impl fmt::Debug for SelectedImporter<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectedImporter")
            .field("info", &self.info)
            .field("confidence", &self.confidence)
            .field("evidence", &self.evidence)
            .finish_non_exhaustive()
    }
}

struct DetectionCandidate {
    importer_index: usize,
    info: FormatInfo,
    confidence: ReadConfidence,
    extension_match: bool,
}

struct ProbeSnapshot {
    bytes: Vec<u8>,
    source_len: u64,
}

impl ImporterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<I>(&mut self, importer: I) -> Result<()>
    where
        I: FormatImporter,
    {
        let info = importer.info();
        if self
            .importers
            .iter()
            .any(|existing| existing.info.id() == info.id())
        {
            return Err(BaoziError::duplicate_format_id(info.id()));
        }
        self.importers.push(RegisteredImporter {
            importer: Box::new(importer),
            info,
        });
        Ok(())
    }

    pub fn formats(&self) -> impl Iterator<Item = FormatInfo> + '_ {
        self.importers
            .iter()
            .map(|registered| registered.info.clone())
    }

    pub fn by_extension(&self, extension: &str) -> Vec<&dyn FormatImporter> {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        self.importers
            .iter()
            .filter_map(|registered| {
                let info = &registered.info;
                info.extensions()
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&extension))
                    .then_some(registered.importer.as_ref())
            })
            .collect()
    }

    pub fn detect(
        &self,
        input: &mut dyn ReadSeek,
        hint: &ReadHint,
    ) -> DetectionResult<SelectedImporter<'_>> {
        self.detect_with_options(input, hint, &DetectionOptions::default())
    }

    pub fn detect_with_options(
        &self,
        input: &mut dyn ReadSeek,
        hint: &ReadHint,
        options: &DetectionOptions,
    ) -> DetectionResult<SelectedImporter<'_>> {
        let snapshot = snapshot_primary(input, hint, options.max_probe_bytes)?;
        let probe = Probe::new(&snapshot.bytes, snapshot.source_len);
        let mut candidates = Vec::new();
        let mut evidence = Vec::with_capacity(self.importers.len());

        for (importer_index, registered) in self.importers.iter().enumerate() {
            let info = &registered.info;
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                registered.importer.can_read(&probe, hint)
            }))
            .unwrap_or_else(|_| DetectionOutcome::fault("detector panicked"));
            if let DetectionOutcome::Match(confidence) = outcome {
                candidates.push(DetectionCandidate {
                    importer_index,
                    extension_match: extension_matches(info, hint),
                    info: info.clone(),
                    confidence,
                });
            }
            evidence.push(DetectionEvidence {
                format_id: info.id(),
                outcome,
            });
        }
        evidence.sort_unstable_by_key(DetectionEvidence::format_id);

        if let Some(best_confidence) = candidates
            .iter()
            .filter(|candidate| candidate.confidence >= ReadConfidence::Likely)
            .map(|candidate| candidate.confidence)
            .max()
        {
            let selected = select_best(
                candidates
                    .iter()
                    .filter(|candidate| candidate.confidence == best_confidence),
                hint,
                &evidence,
            )?;
            return Ok(self.selected_importer(selected, evidence));
        }

        let extension_candidates: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.extension_match)
            .collect();
        if !extension_candidates.is_empty() {
            let selected = select_best(extension_candidates.into_iter(), hint, &evidence)?;
            return Ok(self.selected_importer(selected, evidence));
        }

        Err(DetectionFailure::NoMatch {
            hint: hint.display_hint(),
            evidence,
        })
    }

    fn selected_importer<'a>(
        &'a self,
        candidate: &DetectionCandidate,
        evidence: Vec<DetectionEvidence>,
    ) -> SelectedImporter<'a> {
        SelectedImporter {
            importer: self.importers[candidate.importer_index].importer.as_ref(),
            info: candidate.info.clone(),
            confidence: candidate.confidence,
            evidence,
        }
    }
}

fn snapshot_primary(
    input: &mut dyn ReadSeek,
    hint: &ReadHint,
    max_probe_bytes: u64,
) -> DetectionResult<ProbeSnapshot> {
    let source_len = input
        .seek(SeekFrom::End(0))
        .map_err(|error| primary_fault(hint, PrimaryProbeOperation::Measure, error.to_string()))?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|error| primary_fault(hint, PrimaryProbeOperation::Rewind, error.to_string()))?;

    let snapshot_len_u64 = source_len.min(max_probe_bytes);
    let snapshot_len = usize::try_from(snapshot_len_u64).map_err(|_| {
        primary_fault(
            hint,
            PrimaryProbeOperation::Allocate,
            format!("probe size {snapshot_len_u64} does not fit in memory"),
        )
    })?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(snapshot_len)
        .map_err(|error| primary_fault(hint, PrimaryProbeOperation::Allocate, error.to_string()))?;
    bytes.resize(snapshot_len, 0);

    let read_result = input.read_exact(&mut bytes);
    let rewind_result = input.seek(SeekFrom::Start(0));
    if let Err(error) = rewind_result {
        return Err(primary_fault(
            hint,
            PrimaryProbeOperation::Rewind,
            error.to_string(),
        ));
    }
    if let Err(error) = read_result {
        return Err(primary_fault(
            hint,
            PrimaryProbeOperation::Read,
            error.to_string(),
        ));
    }

    Ok(ProbeSnapshot { bytes, source_len })
}

fn primary_fault(
    hint: &ReadHint,
    operation: PrimaryProbeOperation,
    message: String,
) -> DetectionFailure {
    DetectionFailure::PrimaryFault {
        hint: hint.display_hint(),
        operation,
        message,
    }
}

fn extension_matches(info: &FormatInfo, hint: &ReadHint) -> bool {
    hint.extension.as_deref().is_some_and(|extension| {
        let extension = extension.trim_start_matches('.');
        info.extensions()
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
    })
}

fn select_best<'a>(
    candidates: impl Iterator<Item = &'a DetectionCandidate>,
    hint: &ReadHint,
    evidence: &[DetectionEvidence],
) -> DetectionResult<&'a DetectionCandidate> {
    let matches: Vec<_> = candidates.collect();
    match matches.as_slice() {
        [] => unreachable!("select_best requires at least one candidate"),
        [candidate] => Ok(candidate),
        many => {
            let extension_matches: Vec<_> = many
                .iter()
                .copied()
                .filter(|candidate| candidate.extension_match)
                .collect();
            if let [candidate] = extension_matches.as_slice() {
                return Ok(candidate);
            }

            let mut candidates: Vec<_> = many
                .iter()
                .map(|candidate| candidate.info.id().to_owned())
                .collect();
            candidates.sort_unstable();
            Err(DetectionFailure::Ambiguous {
                hint: hint.display_hint(),
                candidates,
                evidence: evidence.to_vec(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CapabilityStatus, DetectionOutcome, FormatCapability, FormatInfo, FormatMaturity,
        ImportContext,
    };
    use baozi_core::{Result, Scene, SceneBuilder};
    use baozi_io::AssetPath;
    use std::io::{self, Cursor, Read, Seek, SeekFrom};

    struct DummyImporter;

    impl FormatImporter for DummyImporter {
        fn info(&self) -> FormatInfo {
            FormatInfo::new("dummy", "Dummy", &["dum"])
                .with_maturity(FormatMaturity::Experimental)
                .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Unknown)])
                .with_notes("test importer")
        }

        fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
            DetectionOutcome::Match(ReadConfidence::Certain)
        }

        fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
            SceneBuilder::new().finish()
        }
    }

    #[derive(Clone, Copy)]
    enum InjectedProbeFault {
        Measure,
        RewindAfterRead,
        Read,
    }

    struct FaultingProbeReader {
        inner: Cursor<Vec<u8>>,
        fault: InjectedProbeFault,
        rewind_calls: usize,
    }

    impl FaultingProbeReader {
        fn new(bytes: Vec<u8>, fault: InjectedProbeFault) -> Self {
            Self {
                inner: Cursor::new(bytes),
                fault,
                rewind_calls: 0,
            }
        }

        const fn position(&self) -> u64 {
            self.inner.position()
        }
    }

    impl Read for FaultingProbeReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if matches!(self.fault, InjectedProbeFault::Read) {
                Err(io::Error::other("read failed"))
            } else {
                self.inner.read(buffer)
            }
        }
    }

    impl Seek for FaultingProbeReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            if matches!(self.fault, InjectedProbeFault::Measure)
                && matches!(position, SeekFrom::End(_))
            {
                Err(io::Error::other("measure failed"))
            } else if matches!(position, SeekFrom::Start(0)) {
                self.rewind_calls += 1;
                if matches!(self.fault, InjectedProbeFault::RewindAfterRead)
                    && self.rewind_calls == 2
                {
                    return Err(io::Error::other("rewind failed"));
                }
                self.inner.seek(position)
            } else {
                self.inner.seek(position)
            }
        }
    }

    struct HugeLengthReader {
        position: u64,
    }

    impl Read for HugeLengthReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("allocation failure must precede reading"))
        }
    }

    impl Seek for HugeLengthReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.position = match position {
                SeekFrom::Start(position) => position,
                SeekFrom::End(0) => u64::MAX,
                _ => return Err(io::Error::other("unsupported test seek")),
            };
            Ok(self.position)
        }
    }

    fn assert_primary_fault(error: DetectionFailure, expected: PrimaryProbeOperation) {
        let DetectionFailure::PrimaryFault {
            hint,
            operation,
            message,
        } = error
        else {
            panic!("expected a structured primary probe fault");
        };
        assert_eq!(hint, "dum");
        assert_eq!(operation, expected);
        assert!(!message.is_empty());
    }

    #[test]
    fn finds_importer_by_extension() {
        let mut registry = ImporterRegistry::new();
        registry.register(DummyImporter).unwrap();
        assert_eq!(registry.by_extension("dum").len(), 1);
        assert!(registry.by_extension("obj").is_empty());
    }

    #[test]
    fn rejects_duplicate_format_ids() {
        let mut registry = ImporterRegistry::new();
        registry.register(DummyImporter).unwrap();

        let error = registry.register(DummyImporter).unwrap_err();

        assert_eq!(error.kind(), baozi_core::BaoziErrorKind::DuplicateFormatId);
    }

    #[test]
    fn detect_restores_input_to_start_after_snapshot() {
        let mut registry = ImporterRegistry::new();
        registry.register(DummyImporter).unwrap();
        let hint = ReadHint::from_source(AssetPath::new("mesh.dum").unwrap());
        let mut input = Cursor::new(vec![42, 43, 44]);
        input.set_position(2);

        registry.detect(&mut input, &hint).unwrap();

        assert_eq!(input.stream_position().unwrap(), 0);
    }

    #[test]
    fn primary_measure_fault_is_typed_and_preserves_the_reader_position() {
        let mut registry = ImporterRegistry::new();
        registry.register(DummyImporter).unwrap();
        let hint = ReadHint::from_source(AssetPath::new("mesh.dum").unwrap());
        let mut input = FaultingProbeReader::new(vec![42, 43], InjectedProbeFault::Measure);
        input.inner.set_position(1);

        let error = registry.detect(&mut input, &hint).unwrap_err();

        assert_primary_fault(error, PrimaryProbeOperation::Measure);
        assert_eq!(input.position(), 1);
    }

    #[test]
    fn primary_rewind_fault_is_typed_and_leaves_the_reader_at_the_measured_end() {
        let mut registry = ImporterRegistry::new();
        registry.register(DummyImporter).unwrap();
        let hint = ReadHint::from_source(AssetPath::new("mesh.dum").unwrap());
        let mut input =
            FaultingProbeReader::new(vec![42, 43, 44], InjectedProbeFault::RewindAfterRead);

        let error = registry.detect(&mut input, &hint).unwrap_err();

        assert_primary_fault(error, PrimaryProbeOperation::Rewind);
        assert_eq!(input.position(), 3);
    }

    #[test]
    fn primary_read_fault_is_typed_and_rewinds_the_reader() {
        let mut registry = ImporterRegistry::new();
        registry.register(DummyImporter).unwrap();
        let hint = ReadHint::from_source(AssetPath::new("mesh.dum").unwrap());
        let mut input = FaultingProbeReader::new(vec![42, 43, 44], InjectedProbeFault::Read);

        let error = registry.detect(&mut input, &hint).unwrap_err();

        assert_primary_fault(error, PrimaryProbeOperation::Read);
        assert_eq!(input.position(), 0);
    }

    #[test]
    fn primary_allocate_fault_is_typed_and_keeps_the_reader_rewound() {
        let mut registry = ImporterRegistry::new();
        registry.register(DummyImporter).unwrap();
        let hint = ReadHint::from_source(AssetPath::new("mesh.dum").unwrap());
        let mut input = HugeLengthReader { position: 17 };

        let error = registry
            .detect_with_options(
                &mut input,
                &hint,
                &DetectionOptions {
                    max_probe_bytes: u64::MAX,
                },
            )
            .unwrap_err();

        assert_primary_fault(error, PrimaryProbeOperation::Allocate);
        assert_eq!(input.position, 0);
    }
}
