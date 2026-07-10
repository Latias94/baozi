mod common;

use baozi_format_gltf::GltfImporter;
use baozi_import::{
    DetectionFailure, DetectionInconclusive, DetectionOptions, DetectionOutcome, ImporterRegistry,
    ReadConfidence, ReadHint,
};
use baozi_io::AssetPath;
use common::triangle_gltf;
use std::io::Cursor;

fn detect(
    source: &str,
    bytes: Vec<u8>,
    max_probe_bytes: u64,
) -> std::result::Result<ReadConfidence, DetectionFailure> {
    let mut registry = ImporterRegistry::new();
    registry
        .register(GltfImporter)
        .expect("unique glTF importer");
    let hint = ReadHint::from_source(AssetPath::new(source).expect("valid test path"));
    let mut input = Cursor::new(bytes);
    registry
        .detect_with_options(&mut input, &hint, &DetectionOptions { max_probe_bytes })
        .map(|selected| selected.confidence)
}

#[test]
fn detects_gltf_json() {
    let confidence = detect("scene.gltf", triangle_gltf(), 4096).expect("glTF JSON is detected");

    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn detects_glb_magic() {
    let confidence =
        detect("scene.glb", b"glTF\x02\0\0\0".to_vec(), 4096).expect("GLB magic is detected");

    assert_eq!(confidence, ReadConfidence::Certain);
}

#[test]
fn glb_magic_requires_all_four_bytes() {
    let bytes = b"glTF\x02\0\0\0".to_vec();
    let error = detect("scene.mesh", bytes.clone(), 3)
        .expect_err("three bytes cannot establish the GLB magic");
    let DetectionFailure::NoMatch { evidence, .. } = error else {
        panic!("expected structured detection evidence");
    };
    assert!(matches!(
        evidence.as_slice(),
        [evidence]
            if evidence.outcome()
                == &DetectionOutcome::Inconclusive(
                    DetectionInconclusive::ProbeExhausted
                )
    ));

    let confidence = detect("scene.mesh", bytes, 4).expect("four bytes establish the GLB magic");
    assert_eq!(confidence, ReadConfidence::Certain);
}

#[test]
fn completed_json_signature_survives_a_probe_cut_inside_utf8() {
    let mut bytes = br#"{"asset":{"version":"2.0"},"name":""#.to_vec();
    bytes.extend_from_slice("包".as_bytes());
    bytes.extend_from_slice(br#""}"#);

    let confidence = detect("scene.mesh", bytes.clone(), (bytes.len() - 3) as u64)
        .expect("the completed glTF signature precedes the truncated UTF-8 value");
    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn registry_probe_limit_bounds_content_detection() {
    let error =
        detect("scene.mesh", triangle_gltf(), 1).expect_err("one byte cannot establish glTF JSON");
    let DetectionFailure::NoMatch { evidence, .. } = error else {
        panic!("expected structured detection evidence");
    };
    assert!(matches!(
        evidence.as_slice(),
        [evidence]
            if evidence.outcome()
                == &DetectionOutcome::Inconclusive(
                    DetectionInconclusive::ProbeExhausted
                )
    ));

    let confidence = detect("scene.mesh", triangle_gltf(), 4096)
        .expect("the default probe exposes glTF metadata");
    assert_eq!(confidence, ReadConfidence::Likely);
}
