use baozi_format_obj::{FORMAT_ID, ObjImporter};
use baozi_import::{
    DetectionInconclusive, DetectionOptions, DetectionOutcome, ImportConfigOverlay,
    ImportOperationControl, ImportRequest, ImportRequestLayer, OptionCompileLimits, ReadConfidence,
    RegistryBuildLimits, RegistryBuilder, import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::sync::Arc;

fn detect(source: &str, bytes: &[u8], max_probe_bytes: u64) -> DetectionOutcome {
    let source = AssetPath::new(source).expect("valid test path");
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::<[u8]>::from(bytes))
        .expect("test asset insertion succeeds");
    let mut registry = RegistryBuilder::new();
    registry.register(ObjImporter).expect("unique OBJ importer");
    let snapshot = registry
        .freeze(RegistryBuildLimits::default())
        .expect("valid registry");
    let request = snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(ImportConfigOverlay::default().with_detection_options(
                    DetectionOptions::new(max_probe_bytes).expect("bounded probe size"),
                )),
            ),
            OptionCompileLimits::default(),
        )
        .expect("valid request");
    let evidence = match import_asset(&request, &io, source, ImportOperationControl::default()) {
        Ok(report) => report.detection_evidence().to_vec(),
        Err(error) => error
            .into_operation_failure()
            .unwrap_or_else(|source| panic!("OBJ detection session could not initialize: {source}"))
            .detection_evidence()
            .to_vec(),
    };
    evidence
        .into_iter()
        .find(|evidence| evidence.format().as_str() == FORMAT_ID)
        .expect("OBJ detector evidence")
        .outcome()
        .clone()
}

#[test]
fn obj_extension_is_candidate() {
    assert_eq!(
        detect("mesh.obj", b"not enough content", 4096),
        DetectionOutcome::Match(ReadConfidence::Maybe)
    );
}

#[test]
fn obj_like_content_is_likely() {
    assert_eq!(
        detect("mesh.txt", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n", 4096),
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
}

#[test]
fn complete_vertex_prefix_is_strong_obj_evidence_before_the_first_face() {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let face_offset = bytes
        .windows(2)
        .position(|window| window == b"f ")
        .expect("fixture contains a face");

    assert_eq!(
        detect("mesh.bin", bytes, face_offset as u64),
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
}

#[test]
fn weak_obj_statements_do_not_outweigh_content_from_other_formats() {
    let bytes = b"o object\ng group\ns 1\nmtllib material.mtl\nusemtl material\n";

    assert_eq!(detect("mesh.bin", bytes, 4096), DetectionOutcome::NoMatch);
}

#[test]
fn malformed_geometry_statements_are_not_obj_evidence() {
    let bytes = b"v x y z\nf one two three\n";

    assert_eq!(detect("mesh.bin", bytes, 4096), DetectionOutcome::NoMatch);
}

#[test]
fn non_finite_positions_are_not_obj_evidence() {
    let bytes = b"v NaN 0 0\nv NaN 1 0\nv NaN 0 1\n";

    assert_eq!(detect("mesh.bin", bytes, 4096), DetectionOutcome::NoMatch);
}

#[test]
fn unterminated_probe_tail_is_not_counted_as_a_complete_face() {
    let bytes = b"v 0 0 0\nf 1 2 3\n";

    assert_eq!(
        detect("mesh.bin", bytes, (bytes.len() - 1) as u64),
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    );
    assert_eq!(
        detect("mesh.bin", &bytes[..bytes.len() - 1], 4096),
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
}

#[test]
fn completed_obj_evidence_survives_a_probe_cut_inside_utf8() {
    let mut bytes = b"v 0 0 0\nf 1 2 3\n# ".to_vec();
    bytes.extend_from_slice("mesh name".as_bytes());
    bytes.extend_from_slice("包".as_bytes());

    assert_eq!(
        detect("mesh.bin", &bytes, (bytes.len() - 1) as u64),
        DetectionOutcome::Match(ReadConfidence::Likely)
    );
}

#[test]
fn random_text_is_not_obj_without_extension() {
    assert_eq!(
        detect("notes.txt", b"this is not a model\n", 4096),
        DetectionOutcome::NoMatch
    );
}

#[test]
fn exhausted_obj_probe_is_inconclusive_without_poisoning_the_registry() {
    assert_eq!(
        detect("mesh.txt", b"v 0 0 0\nf 1 1 1\n", 1),
        DetectionOutcome::Inconclusive(DetectionInconclusive::ProbeExhausted)
    );
}
