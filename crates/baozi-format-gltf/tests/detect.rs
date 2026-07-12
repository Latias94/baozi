mod common;

use baozi_import::{DetectionInconclusive, DetectionOutcome, ReadConfidence};
use common::{TestResult, detection_outcome, triangle_gltf};

#[test]
fn detects_gltf_json() -> TestResult<()> {
    assert_eq!(
        detection_outcome("scene.gltf", triangle_gltf(), 4096)?,
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
    Ok(())
}

#[test]
fn detects_glb_magic() -> TestResult<()> {
    assert_eq!(
        detection_outcome("scene.glb", b"glTF\x02\0\0\0".to_vec(), 4096)?,
        DetectionOutcome::Match(ReadConfidence::Certain)
    );
    Ok(())
}

#[test]
fn glb_magic_requires_all_four_bytes() -> TestResult<()> {
    let bytes = b"glTF\x02\0\0\0".to_vec();
    assert_eq!(
        detection_outcome("scene.mesh", bytes.clone(), 3)?,
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    );
    assert_eq!(
        detection_outcome("scene.mesh", bytes, 4)?,
        DetectionOutcome::Match(ReadConfidence::Certain)
    );
    Ok(())
}

#[test]
fn completed_json_signature_survives_a_probe_cut_inside_utf8() -> TestResult<()> {
    let mut bytes = br#"{"asset":{"version":"2.0"},"name":""#.to_vec();
    bytes.extend_from_slice("包".as_bytes());
    bytes.extend_from_slice(br#""}"#);

    let visible_len = (bytes.len() - 3) as u64;
    assert_eq!(
        detection_outcome("scene.mesh", bytes, visible_len)?,
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
    Ok(())
}

#[test]
fn development_probe_limit_bounds_content_detection() -> TestResult<()> {
    assert_eq!(
        detection_outcome("scene.mesh", triangle_gltf(), 1)?,
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    );
    assert_eq!(
        detection_outcome("scene.mesh", triangle_gltf(), 4096)?,
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
    Ok(())
}
