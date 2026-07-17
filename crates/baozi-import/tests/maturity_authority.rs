use baozi_core::FormatId;
use baozi_import::{DeclarationEvidenceAuthority, FormatDescriptor, FormatEncoding};

const DESCRIPTOR_SOURCE: &str = include_str!("../src/descriptor.rs");
const REGISTRY_SURFACE: &str = include_str!("../src/registry.rs");
const IMPORT_SURFACE: &str = include_str!("../src/lib.rs");
const FACADE_SURFACE: &str = include_str!("../../baozi/src/lib.rs");

#[test]
fn descriptors_default_to_unverified_declaration_evidence() {
    let descriptor = FormatDescriptor::builder(
        FormatId::new("org.example.external").expect("test format ID is valid"),
        "External test format",
    )
    .encoding(FormatEncoding::Binary)
    .build()
    .expect("minimal binary descriptor is valid");

    assert_eq!(
        descriptor.declaration_evidence_authority(),
        DeclarationEvidenceAuthority::Unverified
    );
}

#[test]
fn descriptor_and_public_surfaces_cannot_issue_baozi_maturity() {
    for forbidden in [
        "pub enum FormatMaturity",
        "maturity: FormatMaturity",
        "fn maturity(",
        "BaoziVerified",
    ] {
        assert!(
            !DESCRIPTOR_SOURCE.contains(forbidden),
            "descriptor maturity authority leaked through `{forbidden}`"
        );
    }

    for (surface_name, surface) in [
        ("registry", REGISTRY_SURFACE),
        ("import facade", IMPORT_SURFACE),
        ("workspace facade", FACADE_SURFACE),
    ] {
        assert!(
            !surface.contains("FormatMaturity"),
            "{surface_name} reintroduced descriptor-owned maturity"
        );
        assert!(
            !surface.contains("BaoziVerified"),
            "{surface_name} leaked managed Baozi evidence authority"
        );
    }
}
