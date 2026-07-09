use baozi_import::{CapabilityStatus, FormatCapability, FormatInfo, FormatMaturity};

const SUPPORT_MATRIX: &str = include_str!("../../../docs/formats/support-matrix.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportMatrixColumn {
    Geometry,
    Materials,
    Textures,
    Animation,
    Diagnostics,
}

impl SupportMatrixColumn {
    fn index(self) -> usize {
        match self {
            Self::Geometry => 3,
            Self::Materials => 4,
            Self::Textures => 5,
            Self::Animation => 6,
            Self::Diagnostics => 8,
        }
    }
}

pub fn assert_support_matrix_row(
    crate_name: &str,
    info: &FormatInfo,
    expectations: &[(SupportMatrixColumn, FormatCapability)],
) {
    let row = support_matrix_row(crate_name);
    let columns = parse_markdown_row(row);
    let expected_crate = format!("`{crate_name}`");

    assert_eq!(columns[1], expected_crate);
    assert_eq!(columns[2], maturity_label(info.maturity()));

    for (column, capability) in expectations {
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
