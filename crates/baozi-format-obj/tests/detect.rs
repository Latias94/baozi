use baozi_format_obj::ObjImporter;
use baozi_import::{
    DetectionFailure, DetectionInconclusive, DetectionOptions, DetectionOutcome, ImporterRegistry,
    ReadConfidence, ReadHint,
};
use baozi_io::AssetPath;
use std::io::Cursor;

fn detect(
    source: &str,
    bytes: &[u8],
    max_probe_bytes: u64,
) -> std::result::Result<ReadConfidence, DetectionFailure> {
    let mut registry = ImporterRegistry::new();
    registry.register(ObjImporter).expect("unique OBJ importer");
    let hint = ReadHint::from_source(AssetPath::new(source).expect("valid test path"));
    let mut input = Cursor::new(bytes.to_vec());
    registry
        .detect_with_options(&mut input, &hint, &DetectionOptions { max_probe_bytes })
        .map(|selected| selected.confidence)
}

#[test]
fn obj_extension_is_candidate() {
    let confidence = detect("mesh.obj", b"not enough content", 4096)
        .expect("the extension is weak positive evidence");

    assert_eq!(confidence, ReadConfidence::Maybe);
}

#[test]
fn obj_like_content_is_likely() {
    let confidence = detect("mesh.txt", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n", 4096)
        .expect("OBJ content is detected");

    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn complete_vertex_prefix_is_strong_obj_evidence_before_the_first_face() {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let face_offset = bytes
        .windows(2)
        .position(|window| window == b"f ")
        .expect("fixture contains a face");

    let confidence = detect("mesh.bin", bytes, face_offset as u64)
        .expect("three complete position records establish OBJ syntax");
    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn weak_obj_statements_do_not_outweigh_content_from_other_formats() {
    let bytes = b"o object\ng group\ns 1\nmtllib material.mtl\nusemtl material\n";
    let error = detect("mesh.bin", bytes, 4096)
        .expect_err("OBJ metadata statements are not geometry evidence");

    assert!(matches!(error, DetectionFailure::NoMatch { .. }));
}

#[test]
fn malformed_geometry_statements_are_not_obj_evidence() {
    let bytes = b"v x y z\nf one two three\n";
    let error = detect("mesh.bin", bytes, 4096)
        .expect_err("geometry keywords without valid arguments are insufficient");

    assert!(matches!(error, DetectionFailure::NoMatch { .. }));
}

#[test]
fn non_finite_positions_are_not_obj_evidence() {
    let bytes = b"v NaN 0 0\nv NaN 1 0\nv NaN 0 1\n";
    let error = detect("mesh.bin", bytes, 4096)
        .expect_err("non-finite coordinates cannot establish valid OBJ geometry");

    assert!(matches!(error, DetectionFailure::NoMatch { .. }));
}

#[test]
fn unterminated_probe_tail_is_not_counted_as_a_complete_face() {
    let bytes = b"v 0 0 0\nf 1 2 3\n";
    let error = detect("mesh.bin", bytes, (bytes.len() - 1) as u64)
        .expect_err("the final face may still be extended inside the source");
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

    let confidence = detect("mesh.bin", &bytes[..bytes.len() - 1], 4096)
        .expect("an unterminated final line is complete when it reaches end of source");
    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn completed_obj_evidence_survives_a_probe_cut_inside_utf8() {
    let mut bytes = b"v 0 0 0\nf 1 2 3\n# ".to_vec();
    bytes.extend_from_slice("mesh name".as_bytes());
    bytes.extend_from_slice("包".as_bytes());

    let confidence = detect("mesh.bin", &bytes, (bytes.len() - 1) as u64)
        .expect("completed geometry lines precede the truncated UTF-8 comment");
    assert_eq!(confidence, ReadConfidence::Likely);
}

#[test]
fn random_text_is_not_obj_without_extension() {
    let error = detect("notes.txt", b"this is not a model\n", 4096)
        .expect_err("random text must not match OBJ");

    assert!(matches!(error, DetectionFailure::NoMatch { .. }));
}

#[test]
fn exhausted_obj_probe_is_inconclusive_without_poisoning_the_registry() {
    let error = detect("mesh.txt", b"v 0 0 0\nf 1 1 1\n", 1)
        .expect_err("one byte cannot establish or reject OBJ content");

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
}
