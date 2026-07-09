use crate::{FormatImporter, FormatInfo};
use baozi_core::{BaoziError, Result};

#[derive(Default)]
pub struct ImporterRegistry {
    importers: Vec<Box<dyn FormatImporter>>,
}

impl ImporterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<I>(&mut self, importer: I)
    where
        I: FormatImporter,
    {
        self.importers.push(Box::new(importer));
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
                info.extensions
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&extension))
                    .then_some(importer.as_ref())
            })
            .collect()
    }

    pub fn unsupported(&self, hint: impl Into<String>) -> BaoziError {
        BaoziError::unsupported_format(hint)
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
        CapabilityStatus, FormatCapability, FormatInfo, FormatMaturity, ImportContext,
        ReadConfidence,
    };
    use baozi_core::{Result, Scene, SceneBuilder};
    use baozi_io::ReadSeek;

    struct DummyImporter;

    impl FormatImporter for DummyImporter {
        fn info(&self) -> FormatInfo {
            FormatInfo {
                id: "dummy",
                display_name: "Dummy",
                extensions: &["dum"],
                maturity: FormatMaturity::Experimental,
                capabilities: &[(FormatCapability::Geometry, CapabilityStatus::Unknown)],
                notes: "test importer",
            }
        }

        fn can_read(
            &self,
            _input: &mut dyn ReadSeek,
            _hint: &crate::ReadHint,
        ) -> Result<ReadConfidence> {
            Ok(ReadConfidence::Certain)
        }

        fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
            Ok(SceneBuilder::new().finish())
        }
    }

    #[test]
    fn finds_importer_by_extension() {
        let mut registry = ImporterRegistry::new();
        registry.register(DummyImporter);
        assert_eq!(registry.by_extension("dum").len(), 1);
        assert!(registry.by_extension("obj").is_empty());
    }
}
