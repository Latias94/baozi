use baozi_import::{
    CapabilityStatus, FormatCapability, FormatInfo, FormatMaturity, FormatSidecarPolicy,
};

const SUPPORT_MATRIX: &str = include_str!("../../../docs/formats/support-matrix.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportMatrixColumn {
    Geometry,
    Hierarchy,
    Materials,
    Textures,
    CamerasLights,
    Animation,
    Skinning,
    MorphTargets,
    Metadata,
    CompressionContainers,
    CoordinatesUnits,
    ResourceLimits,
    Diagnostics,
}

impl SupportMatrixColumn {
    fn index(self) -> usize {
        match self {
            Self::Geometry => 3,
            Self::Hierarchy => 4,
            Self::Materials => 5,
            Self::Textures => 6,
            Self::CamerasLights => 7,
            Self::Animation => 8,
            Self::Skinning => 9,
            Self::MorphTargets => 10,
            Self::Metadata => 11,
            Self::CompressionContainers => 12,
            Self::CoordinatesUnits => 13,
            Self::ResourceLimits => 14,
            Self::Diagnostics => 16,
        }
    }
}

const MATRIX_CAPABILITY_COLUMNS: &[(SupportMatrixColumn, FormatCapability)] = &[
    (SupportMatrixColumn::Geometry, FormatCapability::Geometry),
    (SupportMatrixColumn::Hierarchy, FormatCapability::Hierarchy),
    (SupportMatrixColumn::Materials, FormatCapability::Materials),
    (SupportMatrixColumn::Textures, FormatCapability::Textures),
    (
        SupportMatrixColumn::CamerasLights,
        FormatCapability::CamerasLights,
    ),
    (SupportMatrixColumn::Animation, FormatCapability::Animation),
    (SupportMatrixColumn::Skinning, FormatCapability::Skinning),
    (
        SupportMatrixColumn::MorphTargets,
        FormatCapability::MorphTargets,
    ),
    (SupportMatrixColumn::Metadata, FormatCapability::Metadata),
    (
        SupportMatrixColumn::CompressionContainers,
        FormatCapability::CompressionContainers,
    ),
    (
        SupportMatrixColumn::CoordinatesUnits,
        FormatCapability::CoordinatesUnits,
    ),
    (
        SupportMatrixColumn::ResourceLimits,
        FormatCapability::ResourceLimits,
    ),
    (
        SupportMatrixColumn::Diagnostics,
        FormatCapability::Diagnostics,
    ),
];

pub fn assert_support_matrix_row(crate_name: &str, info: &FormatInfo) {
    let row = support_matrix_row(crate_name);
    let columns = parse_markdown_row(row);
    let expected_crate = format!("`{crate_name}`");

    assert_eq!(
        columns.len(),
        18,
        "support matrix column count drifted for {crate_name}"
    );
    assert_eq!(columns[1], expected_crate);
    assert_eq!(columns[2], maturity_label(info.maturity()));
    assert_eq!(
        columns[15],
        sidecar_label(info.sidecar_policy()),
        "support matrix Sidecars/archives column drifted for {crate_name}"
    );

    for (column, capability) in MATRIX_CAPABILITY_COLUMNS {
        let expected = capability_label(capability_status(info, *capability));
        assert_eq!(
            columns[column.index()],
            expected,
            "support matrix {column:?} column drifted for {crate_name}"
        );
    }
}

fn support_matrix_row(crate_name: &str) -> &'static str {
    let needle = format!("| `{crate_name}` |");
    SUPPORT_MATRIX
        .lines()
        .find(|line| line.contains(&needle))
        .unwrap_or_else(|| panic!("support matrix row for {crate_name} was not found"))
}

fn parse_markdown_row(row: &str) -> Vec<String> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .map(str::to_owned)
        .collect()
}

fn capability_status(info: &FormatInfo, capability: FormatCapability) -> CapabilityStatus {
    info.capability_status(capability).unwrap_or_else(|| {
        panic!(
            "FormatInfo for {} does not declare {capability:?}",
            info.id()
        )
    })
}

fn maturity_label(maturity: FormatMaturity) -> &'static str {
    match maturity {
        FormatMaturity::Experimental => "Experimental",
        FormatMaturity::Beta => "Beta",
        FormatMaturity::Stable => "Stable",
        FormatMaturity::Deprecated => "Deprecated",
    }
}

fn capability_label(status: CapabilityStatus) -> &'static str {
    match status {
        CapabilityStatus::Supported => "Supported",
        CapabilityStatus::Partial => "Partial",
        CapabilityStatus::ParsedLossy => "ParsedLossy",
        CapabilityStatus::IgnoredWithDiagnostic => "IgnoredWithDiagnostic",
        CapabilityStatus::Unsupported => "Unsupported",
        CapabilityStatus::Unknown => "Unknown",
    }
}

fn sidecar_label(policy: FormatSidecarPolicy) -> &'static str {
    match policy {
        FormatSidecarPolicy::None => "Unsupported",
        FormatSidecarPolicy::Optional | FormatSidecarPolicy::ExternalBuffers => "Partial",
        FormatSidecarPolicy::Required | FormatSidecarPolicy::ArchiveEntries => "Supported",
        FormatSidecarPolicy::Unknown => "Unknown",
    }
}
