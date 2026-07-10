mod common;

use baozi_format_gltf::detect_for_development;
use baozi_import::{DetectionInconclusive, DetectionOutcome, ReadConfidence, ReadHint};
use baozi_io::AssetPath;
use common::triangle_gltf;

fn detect(source: &str, bytes: Vec<u8>, max_probe_bytes: u64) -> DetectionOutcome {
    let hint = ReadHint::from_source(AssetPath::new(source).expect("valid test path"));
    let visible_len = usize::try_from(max_probe_bytes)
        .unwrap_or(usize::MAX)
        .min(bytes.len());
    detect_for_development(&bytes[..visible_len], bytes.len() as u64, &hint)
}

#[test]
fn detects_gltf_json() {
    assert_eq!(
        detect("scene.gltf", triangle_gltf(), 4096),
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
}

#[test]
fn detects_glb_magic() {
    assert_eq!(
        detect("scene.glb", b"glTF\x02\0\0\0".to_vec(), 4096),
        DetectionOutcome::Match(ReadConfidence::Certain)
    );
}

#[test]
fn glb_magic_requires_all_four_bytes() {
    let bytes = b"glTF\x02\0\0\0".to_vec();
    assert_eq!(
        detect("scene.mesh", bytes.clone(), 3),
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    );
    assert_eq!(
        detect("scene.mesh", bytes, 4),
        DetectionOutcome::Match(ReadConfidence::Certain)
    );
}

#[test]
fn completed_json_signature_survives_a_probe_cut_inside_utf8() {
    let mut bytes = br#"{"asset":{"version":"2.0"},"name":""#.to_vec();
    bytes.extend_from_slice("包".as_bytes());
    bytes.extend_from_slice(br#""}"#);

    let visible_len = (bytes.len() - 3) as u64;
    assert_eq!(
        detect("scene.mesh", bytes, visible_len),
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
}

#[test]
fn development_probe_limit_bounds_content_detection() {
    assert_eq!(
        detect("scene.mesh", triangle_gltf(), 1),
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    );
    assert_eq!(
        detect("scene.mesh", triangle_gltf(), 4096),
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
}
