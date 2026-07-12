mod common;

use baozi_format_stl::StlImporter;
use baozi_import::{
    DetectionOptions, DetectionOutcome, ImportCause, ImportConfigOverlay, ImportOperationControl,
    ImportRequest, ImportRequestLayer, OptionCompileLimits, ReadConfidence, RegistryBuildLimits,
    RegistryBuilder, import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use common::{BinaryFacet, TestImportOutcome, ascii_triangle, binary_stl};
use std::sync::Arc;

fn detect(source: &str, bytes: Vec<u8>, max_probe_bytes: u64) -> TestImportOutcome<ReadConfidence> {
    let mut builder = RegistryBuilder::new();
    builder.register(StlImporter).expect("valid STL importer");
    let snapshot = builder
        .freeze(RegistryBuildLimits::default())
        .expect("valid registry");
    let request = snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(
                    ImportConfigOverlay::default().with_detection_options(
                        DetectionOptions::new(max_probe_bytes)
                            .expect("test probe is within cancellation bound"),
                    ),
                ),
            ),
            OptionCompileLimits::default(),
        )
        .expect("request compiles");
    let path = AssetPath::new(source).expect("valid path");
    let io = MemoryAssetIo::new();
    io.try_insert(path.clone(), Arc::<[u8]>::from(bytes))
        .expect("test asset insertion must succeed");
    TestImportOutcome::new(
        import_asset(&request, &io, path, ImportOperationControl::default()).map(|report| {
            let evidence = report
                .detection_evidence()
                .first()
                .expect("STL detector evidence");
            match evidence.outcome() {
                DetectionOutcome::Match(confidence) => *confidence,
                outcome => panic!("expected matching detector outcome, got {outcome:?}"),
            }
        }),
    )
}

#[test]
fn binary_exact_size_is_certain() {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    assert_eq!(
        detect("triangle.stl", bytes, 4096).expect("binary STL is detected"),
        ReadConfidence::Certain
    );
}

#[test]
fn binary_header_starting_with_solid_is_still_binary() {
    let bytes = binary_stl(b"solid binary header", &[BinaryFacet::unit_triangle()]);
    assert_eq!(
        detect("solid-header.stl", bytes, 4096).expect("size evidence wins"),
        ReadConfidence::Certain
    );
}

#[test]
fn ascii_solid_after_ascii_whitespace_is_likely() {
    let bytes = format!("\n\t{}", ascii_triangle("triangle"));
    assert_eq!(
        detect("triangle.stl", bytes.into_bytes(), 4096).expect("ASCII STL is detected"),
        ReadConfidence::Likely
    );
}

#[test]
fn incomplete_ascii_keyword_remains_inconclusive() {
    let bytes = ascii_triangle("triangle").into_bytes();
    let error =
        detect("triangle.mesh", bytes.clone(), 5).expect_err("partial keyword must not match");
    let ImportCause::Detection(_) = error.cause() else {
        panic!("expected detection failure")
    };

    assert_eq!(
        detect("triangle.mesh", bytes, 6).expect("keyword boundary is sufficient"),
        ReadConfidence::Likely
    );
}

#[test]
fn random_bytes_are_not_stl() {
    let failure = detect("triangle.mesh", b"baozi is not an stl".to_vec(), 4096)
        .expect_err("random bytes must not match STL");
    assert!(matches!(failure.cause(), ImportCause::UnsupportedFormat));
}

#[test]
fn short_binary_probe_is_inconclusive() {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let error = detect("triangle.stl", bytes, 83).expect_err("facet count is outside probe");
    assert!(matches!(error.cause(), ImportCause::Detection(_)));
}
