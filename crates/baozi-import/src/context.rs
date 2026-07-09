use crate::format::FormatInfo;
use baozi_core::{Diagnostic, Scene};
use baozi_io::{AssetIo, AssetPath, ResourceLimits};

#[derive(Debug, Clone, Default)]
pub struct ReadHint {
    pub source: Option<AssetPath>,
    pub extension: Option<String>,
}

impl ReadHint {
    pub fn from_source(source: AssetPath) -> Self {
        let extension = source.extension().map(str::to_owned);
        Self {
            source: Some(source),
            extension,
        }
    }

    pub fn display_hint(&self) -> String {
        self.extension
            .as_deref()
            .or_else(|| self.source.as_ref().map(AssetPath::as_str))
            .filter(|hint| !hint.is_empty())
            .unwrap_or("unknown")
            .to_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalReferencePolicy {
    Deny,
    WithinScope,
    AllowListedRoots,
    CustomResolver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoOptions {
    pub external_references: ExternalReferencePolicy,
}

impl IoOptions {
    pub const fn memory() -> Self {
        Self {
            external_references: ExternalReferencePolicy::Deny,
        }
    }

    pub const fn filesystem() -> Self {
        Self {
            external_references: ExternalReferencePolicy::WithinScope,
        }
    }
}

impl Default for IoOptions {
    fn default() -> Self {
        Self::memory()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionOptions {
    pub max_probe_bytes: u64,
}

impl Default for DetectionOptions {
    fn default() -> Self {
        Self {
            max_probe_bytes: 4096,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticOptions {
    pub strict: bool,
}

impl Default for DiagnosticOptions {
    fn default() -> Self {
        Self { strict: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportOptions {
    pub limits: ResourceLimits,
    pub io: IoOptions,
    pub detection: DetectionOptions,
    pub diagnostics: DiagnosticOptions,
}

impl ImportOptions {
    pub fn memory() -> Self {
        Self::default()
    }

    pub fn filesystem() -> Self {
        Self {
            io: IoOptions::filesystem(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    pub scene: Scene,
    pub diagnostics: Vec<Diagnostic>,
    pub format: FormatInfo,
}

pub struct ImportContext<'a> {
    pub io: &'a dyn AssetIo,
    pub source: AssetPath,
    pub options: ImportOptions,
    pub diagnostics: Vec<Diagnostic>,
}

impl<'a> ImportContext<'a> {
    pub fn new(io: &'a dyn AssetIo, source: AssetPath) -> Self {
        Self::with_options(io, source, ImportOptions::default())
    }

    pub fn with_options(io: &'a dyn AssetIo, source: AssetPath, options: ImportOptions) -> Self {
        Self {
            io,
            source,
            options,
            diagnostics: Vec::new(),
        }
    }

    pub fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
        if self.diagnostics.len() < self.options.limits.max_diagnostics {
            self.diagnostics.push(diagnostic);
        }
    }

    pub fn into_scene_report(self, scene: Scene) -> (Scene, Vec<Diagnostic>) {
        (scene, self.diagnostics)
    }

    pub fn into_report(self, scene: Scene, format: FormatInfo) -> ImportReport {
        ImportReport {
            scene,
            diagnostics: self.diagnostics,
            format,
        }
    }
}
