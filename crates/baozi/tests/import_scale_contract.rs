#![cfg(feature = "format-obj")]

mod common;

use baozi::{ImportDimension, ImportLedgerSnapshot, ImportReport, Importer};
use baozi_test_support::{ObjScale, ObjScaleFixture};
use common::{scene_snapshot_with_limits, snapshot_line};

fn assert_counter(report: &ImportReport, dimension: ImportDimension, expected: u64) {
    let evidence = report
        .ledger()
        .dimension(dimension)
        .expect("scale contract dimension should exist");
    assert_eq!(evidence.attempted(), expected, "{dimension:?} attempted");
    assert_eq!(evidence.consumed(), expected, "{dimension:?} consumed");
}

fn assert_clean_terminal_ledger(ledger: &ImportLedgerSnapshot) {
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(ledger.active_reservations(), 0);

    for dimension in [
        ImportDimension::PrimaryBytes,
        ImportDimension::Records,
        ImportDimension::Tokens,
        ImportDimension::Faces,
        ImportDimension::CanonicalOutputBytes,
    ] {
        let evidence = ledger
            .dimension(dimension)
            .expect("reviewed scale dimension must exist");
        assert!(
            evidence.consumed() <= evidence.attempted(),
            "{dimension:?} consumed work must not exceed attempted work"
        );
        assert_eq!(
            evidence.current_live_bytes(),
            0,
            "{dimension:?} retained live bytes"
        );
    }
}

#[test]
fn obj_scale_work_is_affine_and_declared_live_memory_stays_inside_reviewed_envelope() {
    let importer = Importer::new().expect("default format registry should freeze");

    for scale in ObjScale::CORRECTNESS {
        let fixture =
            ObjScaleFixture::try_new(scale).expect("bounded scale fixture should allocate");
        let report = importer
            .read_bytes("scale.obj", fixture.bytes())
            .expect("deterministic OBJ fixture should import");
        let snapshot = scene_snapshot_with_limits(&report, 16 * 1024, 8, 4_000_000);

        assert_eq!(report.format().id().as_str(), "org.baozi.obj");
        assert!(report.diagnostics().is_empty());
        assert_eq!(snapshot.logical_mesh_count(), 1);
        assert_eq!(snapshot.primitive_count(), 1);
        assert_eq!(snapshot.attribute_count(), 1);
        assert_eq!(
            snapshot.primitive_index_count(),
            (fixture.face_count() * 3) as u64
        );
        let primitive = snapshot_line(&snapshot, "primitive[0]");
        assert!(primitive.contains("topology=triangle_list vertices=3"));
        assert!(primitive.contains(&format!("faces={}", fixture.face_count())));
        assert!(snapshot_line(&snapshot, "attribute[0]").contains(concat!(
            "semantic=position semantic_set=0 domain=vertex scalar=f32 lanes=3 ",
            "normalization=none payload=dense elements=3"
        )));

        assert_counter(
            &report,
            ImportDimension::PrimaryBytes,
            fixture.bytes().len() as u64,
        );
        assert_counter(
            &report,
            ImportDimension::Records,
            fixture.expected_record_count(),
        );
        assert_counter(
            &report,
            ImportDimension::Tokens,
            fixture.expected_token_count(),
        );
        assert_counter(&report, ImportDimension::Faces, fixture.face_count() as u64);
        assert_clean_terminal_ledger(report.ledger());
        assert!(report.ledger().peak_live_bytes() > 0);
        assert!(
            report.ledger().peak_live_bytes() <= fixture.reviewed_peak_live_envelope(),
            "{} observed {} declared peak bytes above reviewed affine envelope {}",
            scale.label(),
            report.ledger().peak_live_bytes(),
            fixture.reviewed_peak_live_envelope()
        );
    }
}
