use baozi_import::{
    CapabilityStatus, DeclarationEvidenceAuthority, DependencyRequirement, FormatCapability,
    FormatDescriptor,
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
            Self::Geometry => 4,
            Self::Hierarchy => 5,
            Self::Materials => 6,
            Self::Textures => 7,
            Self::CamerasLights => 8,
            Self::Animation => 9,
            Self::Skinning => 10,
            Self::MorphTargets => 11,
            Self::Metadata => 12,
            Self::CompressionContainers => 13,
            Self::CoordinatesUnits => 14,
            Self::ResourceLimits => 15,
            Self::Diagnostics => 17,
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

pub fn assert_support_matrix_row(crate_name: &str, descriptor: &FormatDescriptor) {
    let row = support_matrix_row(crate_name);
    let columns = parse_markdown_row(row);
    let expected_crate = format!("`{crate_name}`");

    assert_eq!(
        columns.len(),
        19,
        "support matrix column count drifted for {crate_name}"
    );
    assert_eq!(columns[1], expected_crate);
    assert_eq!(
        columns[2],
        declaration_evidence_label(descriptor.declaration_evidence_authority())
    );
    assert_eq!(columns[3], "None", "unexpected Baozi maturity claim");
    assert_eq!(
        columns[16],
        sidecar_label(descriptor),
        "support matrix Sidecars/archives column drifted for {crate_name}"
    );

    for (column, capability) in MATRIX_CAPABILITY_COLUMNS {
        let expected = capability_label(capability_status(descriptor, *capability));
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

fn capability_status(
    descriptor: &FormatDescriptor,
    capability: FormatCapability,
) -> CapabilityStatus {
    descriptor.capability_status(capability).unwrap_or_else(|| {
        panic!(
            "FormatDescriptor for {} does not declare {capability:?}",
            descriptor.id()
        )
    })
}

fn declaration_evidence_label(authority: DeclarationEvidenceAuthority) -> &'static str {
    match authority {
        DeclarationEvidenceAuthority::Unverified => "Unverified",
        _ => "Unknown",
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
        _ => "Unknown",
    }
}

fn sidecar_label(descriptor: &FormatDescriptor) -> &'static str {
    let mut roles = descriptor
        .dependency_roles()
        .map(|(_, requirement)| requirement);
    let Some(first) = roles.next() else {
        return "Unsupported";
    };
    if first == DependencyRequirement::Optional
        || roles.any(|requirement| requirement == DependencyRequirement::Optional)
    {
        "Partial"
    } else {
        "Supported"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_published_row_is_unverified_without_baozi_maturity() {
        for crate_name in [
            "baozi-format-stl",
            "baozi-format-obj",
            "baozi-format-ply",
            "baozi-format-gltf",
        ] {
            let columns = parse_markdown_row(support_matrix_row(crate_name));
            assert_eq!(
                columns[2], "Unverified",
                "unexpected authority for {crate_name}"
            );
            assert_eq!(columns[3], "None", "unexpected maturity for {crate_name}");
        }
    }
}
