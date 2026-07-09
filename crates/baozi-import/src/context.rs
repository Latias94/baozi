use baozi_core::{Diagnostic, Scene};
use baozi_io::{AssetIo, AssetPath, ResourceLimits};

#[derive(Debug, Clone, Default)]
pub struct ReadHint {
    pub source: Option<AssetPath>,
    pub extension: Option<String>,
}

pub struct ImportContext<'a> {
    pub io: &'a dyn AssetIo,
    pub source: AssetPath,
    pub limits: ResourceLimits,
    pub diagnostics: Vec<Diagnostic>,
}

impl<'a> ImportContext<'a> {
    pub fn new(io: &'a dyn AssetIo, source: AssetPath) -> Self {
        Self {
            io,
            source,
            limits: ResourceLimits::default(),
            diagnostics: Vec::new(),
        }
    }

    pub fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
        if self.diagnostics.len() < self.limits.max_diagnostics {
            self.diagnostics.push(diagnostic);
        }
    }

    pub fn into_scene_report(self, scene: Scene) -> (Scene, Vec<Diagnostic>) {
        (scene, self.diagnostics)
    }
}
