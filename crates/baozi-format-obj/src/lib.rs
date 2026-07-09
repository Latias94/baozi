#![forbid(unsafe_code)]

mod detect;
mod mesh_builder;
mod mtl;
mod obj;
mod parser;

use baozi_core::{Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatImporter, FormatInfo, FormatMaturity, ImportContext,
    ImporterRegistry, ReadConfidence, ReadHint,
};
use baozi_io::ReadSeek;

pub struct ObjImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo {
        id: "obj",
        display_name: "Wavefront OBJ",
        extensions: &["obj"],
        maturity: FormatMaturity::Experimental,
        capabilities: &[
            (FormatCapability::Geometry, CapabilityStatus::Supported),
            (FormatCapability::Hierarchy, CapabilityStatus::Partial),
            (FormatCapability::Materials, CapabilityStatus::Partial),
            (FormatCapability::Textures, CapabilityStatus::Partial),
            (
                FormatCapability::CamerasLights,
                CapabilityStatus::Unsupported,
            ),
            (FormatCapability::Animation, CapabilityStatus::Unsupported),
            (FormatCapability::Skinning, CapabilityStatus::Unsupported),
            (
                FormatCapability::MorphTargets,
                CapabilityStatus::Unsupported,
            ),
            (FormatCapability::Metadata, CapabilityStatus::Partial),
            (
                FormatCapability::CompressionContainers,
                CapabilityStatus::Unsupported,
            ),
            (
                FormatCapability::CoordinatesUnits,
                CapabilityStatus::ParsedLossy,
            ),
            (FormatCapability::Diagnostics, CapabilityStatus::Supported),
            (
                FormatCapability::ResourceLimits,
                CapabilityStatus::Supported,
            ),
        ],
        notes: "experimental OBJ/MTL importer for static face meshes and external texture URI references",
    }
}

pub fn register(registry: &mut ImporterRegistry) {
    registry.register(ObjImporter);
}

impl FormatImporter for ObjImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn can_read(&self, input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
        detect::can_read(input, hint)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        parser::read_obj(ctx)
    }
}
