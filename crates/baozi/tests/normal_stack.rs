#![cfg(feature = "format-obj")]

use baozi::{
    CancellationToken, ControlCause, Error, ImportCause, ImportOperationControl, Importer,
    SceneSnapshotLimits, ScopeAbortCause,
};
use std::process::Command;

const CHILD_ENV: &str = "BAOZI_NORMAL_STACK_TEST";
const TRIANGLE: &[u8] = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n";

#[test]
fn import_success_and_failure_fit_a_one_mib_stack() {
    if std::env::var_os(CHILD_ENV).is_some() {
        // A stack overflow aborts the process, so exercise the caller's stack in a child.
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let importer = Importer::new().expect("default formats register");
                let report = importer
                    .read_bytes("triangle.obj", TRIANGLE)
                    .expect("a triangle imports on the caller's normal stack");
                let snapshot = report
                    .scene()
                    .capture_snapshot(
                        SceneSnapshotLimits::try_new(16 * 1024, 64, 1_000_000)
                            .expect("valid snapshot limits"),
                    )
                    .expect("the imported Scene is observable");
                assert_eq!(snapshot.logical_mesh_count(), 1);
                assert_eq!(snapshot.primitive_index_count(), 3);
                assert!(report.scene().meshes().next().unwrap()
                    .primitives().next().unwrap().texcoords(0).is_some());
                assert!(importer.read_bytes("broken.obj", b"f 1 2 3\n").is_err());
                let request = importer.prepare_default_request().expect("default request compiles");
                let cancellation = CancellationToken::new();
                cancellation.cancel();
                let error = importer
                    .read_bytes_with_request(
                        "cancelled.obj",
                        TRIANGLE,
                        &request,
                        ImportOperationControl::default().with_cancellation(cancellation),
                    )
                    .expect_err("cancelled import returns a failure on the caller's stack");
                let Error::Import(failure) = error else {
                    panic!("cancelled import must preserve its terminal failure");
                };
                assert!(matches!(
                    failure.cause(),
                    ImportCause::Scope(abort)
                        if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled { .. }))
                ));
                assert_eq!(failure.ledger().current_live_bytes(), 0);
                drop(report);
            })
            .expect("the caller thread starts")
            .join()
            .expect("the caller thread completes");
        return;
    }

    let output = Command::new(std::env::current_exe().expect("test executable path"))
        .arg("--exact")
        .arg("import_success_and_failure_fit_a_one_mib_stack")
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .output()
        .expect("the normal-stack consumer starts");
    assert!(
        output.status.success(),
        "normal-stack consumer failed: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
