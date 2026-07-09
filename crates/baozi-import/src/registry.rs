use crate::{DetectionOptions, FormatImporter, FormatInfo};
use baozi_core::{BaoziError, Result};
use baozi_io::ReadSeek;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};

use crate::{ReadConfidence, ReadHint};

#[derive(Default)]
pub struct ImporterRegistry {
    importers: Vec<Box<dyn FormatImporter>>,
}

#[derive(Clone)]
pub struct SelectedImporter<'a> {
    pub importer: &'a dyn FormatImporter,
    pub info: FormatInfo,
    pub confidence: ReadConfidence,
}

impl fmt::Debug for SelectedImporter<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectedImporter")
            .field("info", &self.info)
            .field("confidence", &self.confidence)
            .finish_non_exhaustive()
    }
}

struct DetectionCandidate {
    importer_index: usize,
    info: FormatInfo,
    confidence: ReadConfidence,
    extension_match: bool,
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
            .any(|existing| existing.info().id() == info.id())
        {
            return Err(BaoziError::duplicate_format_id(info.id()));
        }
        self.importers.push(Box::new(importer));
        Ok(())
    }

    pub fn formats(&self) -> impl Iterator<Item = FormatInfo> + '_ {
        self.importers.iter().map(|importer| importer.info())
    }

    pub fn by_extension(&self, extension: &str) -> Vec<&dyn FormatImporter> {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        self.importers
            .iter()
            .filter_map(|importer| {
                let info = importer.info();
                info.extensions()
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&extension))
                    .then_some(importer.as_ref())
            })
            .collect()
    }

    pub fn detect(
        &self,
        input: &mut dyn ReadSeek,
        hint: &ReadHint,
    ) -> Result<SelectedImporter<'_>> {
        self.detect_with_options(input, hint, &DetectionOptions::default())
    }

    pub fn detect_with_options(
        &self,
        input: &mut dyn ReadSeek,
        hint: &ReadHint,
        options: &DetectionOptions,
    ) -> Result<SelectedImporter<'_>> {
        let mut candidates = Vec::new();
        for (importer_index, importer) in self.importers.iter().enumerate() {
            let info = importer.info();
            let result = {
                let mut probe = BoundedReadSeek::new(input, options.max_probe_bytes);
                importer.can_read(&mut probe, hint)
            };
            rewind(input, hint)?;
            let confidence = result?;
            if confidence == ReadConfidence::No {
                continue;
            }
            candidates.push(DetectionCandidate {
                importer_index,
                extension_match: extension_matches(&info, hint),
                info,
                confidence,
            });
        }

        let content_best = candidates
            .iter()
            .filter(|candidate| candidate.confidence >= ReadConfidence::Likely)
            .map(|candidate| candidate.confidence)
            .max();

        if let Some(best_confidence) = content_best {
            let selected = select_best(
                candidates
                    .iter()
                    .filter(|candidate| candidate.confidence == best_confidence),
                hint,
            )?;
            return Ok(self.selected_importer(selected));
        }

        let selected = select_best(
            candidates
                .iter()
                .filter(|candidate| candidate.extension_match),
            hint,
        )?;
        Ok(self.selected_importer(selected))
    }

    pub fn unsupported(&self, hint: impl Into<String>) -> BaoziError {
        BaoziError::unsupported_format(hint)
    }

    fn selected_importer<'a>(&'a self, candidate: &DetectionCandidate) -> SelectedImporter<'a> {
        SelectedImporter {
            importer: self.importers[candidate.importer_index].as_ref(),
            info: candidate.info.clone(),
            confidence: candidate.confidence,
        }
    }
}

struct BoundedReadSeek<'a> {
    inner: &'a mut dyn ReadSeek,
    remaining: u64,
}

impl<'a> BoundedReadSeek<'a> {
    fn new(inner: &'a mut dyn ReadSeek, max_bytes: u64) -> Self {
        Self {
            inner,
            remaining: max_bytes,
        }
    }
}

impl Read for BoundedReadSeek<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 && !buffer.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "format probe byte limit exceeded",
            ));
        }

        let allowed = buffer
            .len()
            .min(self.remaining.min(usize::MAX as u64) as usize);
        let read = self.inner.read(&mut buffer[..allowed])?;
        self.remaining = self.remaining.saturating_sub(read as u64);
        Ok(read)
    }
}

impl Seek for BoundedReadSeek<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

fn rewind(input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<()> {
    input
        .seek(SeekFrom::Start(0))
        .map(|_| ())
        .map_err(|error| BaoziError::io(hint.display_hint(), error.to_string()))
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
) -> Result<&'a DetectionCandidate> {
    let matches: Vec<_> = candidates.collect();
    match matches.as_slice() {
        [] => Err(BaoziError::unsupported_format(hint.display_hint())),
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

            Err(BaoziError::ambiguous_format(
                hint.display_hint(),
                many.iter().map(|candidate| candidate.info.id()),
            ))
        }
    }
}

pub fn ensure_supported(candidates: &[&dyn FormatImporter], hint: &str) -> Result<()> {
    if candidates.is_empty() {
        Err(BaoziError::unsupported_format(hint))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CapabilityStatus, DetectionOptions, FormatCapability, FormatInfo, FormatMaturity,
        ImportContext, ReadConfidence,
    };
    use baozi_core::{Result, Scene, SceneBuilder};
    use baozi_io::{AssetPath, ReadSeek};
    use std::io::{Cursor, Seek};

    struct DummyImporter;
    struct ReadsOneByteImporter;
    struct GreedyImporter;

    impl FormatImporter for DummyImporter {
        fn info(&self) -> FormatInfo {
            FormatInfo::new("dummy", "Dummy", &["dum"])
                .with_maturity(FormatMaturity::Experimental)
                .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Unknown)])
                .with_notes("test importer")
        }

        fn can_read(
            &self,
            _input: &mut dyn ReadSeek,
            _hint: &crate::ReadHint,
        ) -> Result<ReadConfidence> {
            Ok(ReadConfidence::Certain)
        }

        fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
            SceneBuilder::new().finish()
        }
    }

    impl FormatImporter for ReadsOneByteImporter {
        fn info(&self) -> FormatInfo {
            FormatInfo::new("reads-one-byte", "Reads One Byte", &["one"])
                .with_maturity(FormatMaturity::Experimental)
                .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Unknown)])
                .with_notes("test importer")
        }

        fn can_read(
            &self,
            input: &mut dyn ReadSeek,
            _hint: &crate::ReadHint,
        ) -> Result<ReadConfidence> {
            let mut byte = [0];
            input
                .read_exact(&mut byte)
                .map_err(|error| baozi_core::BaoziError::io("probe", error.to_string()))?;
            Ok(ReadConfidence::Certain)
        }

        fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
            SceneBuilder::new().finish()
        }
    }

    impl FormatImporter for GreedyImporter {
        fn info(&self) -> FormatInfo {
            FormatInfo::new("greedy", "Greedy", &["greedy"])
                .with_maturity(FormatMaturity::Experimental)
                .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Unknown)])
                .with_notes("test importer")
        }

        fn can_read(
            &self,
            input: &mut dyn ReadSeek,
            _hint: &crate::ReadHint,
        ) -> Result<ReadConfidence> {
            let mut bytes = Vec::new();
            input
                .read_to_end(&mut bytes)
                .map_err(|error| baozi_core::BaoziError::io("probe", error.to_string()))?;
            Ok(ReadConfidence::Certain)
        }

        fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
            SceneBuilder::new().finish()
        }
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
    fn detect_rewinds_input_after_probe() {
        let mut registry = ImporterRegistry::new();
        registry.register(ReadsOneByteImporter).unwrap();
        let source = AssetPath::new("mesh.one").unwrap();
        let hint = ReadHint::from_source(source);
        let mut input = Cursor::new(vec![42, 43, 44]);

        registry.detect(&mut input, &hint).unwrap();

        assert_eq!(input.stream_position().unwrap(), 0);
    }

    #[test]
    fn detect_enforces_probe_byte_limit() {
        let mut registry = ImporterRegistry::new();
        registry.register(GreedyImporter).unwrap();
        let source = AssetPath::new("mesh.greedy").unwrap();
        let hint = ReadHint::from_source(source);
        let mut input = Cursor::new(vec![1, 2, 3]);
        let options = DetectionOptions { max_probe_bytes: 1 };

        let error = match registry.detect_with_options(&mut input, &hint, &options) {
            Ok(_) => panic!("greedy probe unexpectedly passed byte limit"),
            Err(error) => error,
        };

        assert!(matches!(error, baozi_core::BaoziError::Io { .. }));
        assert_eq!(input.stream_position().unwrap(), 0);
    }
}
