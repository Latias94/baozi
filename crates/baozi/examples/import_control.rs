#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{
    AssetPath, CancellationToken, ImportOperationControl, ImportProgressPhase, ImportProgressUnit,
    ImportReport, Importer, ProgressDecision, ProgressEvent, Result,
};
use std::time::{Duration, Instant};

fn import_controlled(bytes: &[u8], cancellation: CancellationToken) -> Result<ImportReport> {
    let plan = Importer::new()?.default_plan()?;
    let control = ImportOperationControl::default()
        .with_cancellation(cancellation)
        .with_deadline(Instant::now() + Duration::from_secs(30))
        .with_progress_observer(
            |event: ProgressEvent<ImportProgressPhase, ImportProgressUnit>| {
                match event.total() {
                    Some(total) => eprintln!(
                        "{:?} {:?}: {} / {total}",
                        event.phase(),
                        event.unit(),
                        event.completed()
                    ),
                    None => eprintln!(
                        "{:?} {:?}: {} (total unknown)",
                        event.phase(),
                        event.unit(),
                        event.completed()
                    ),
                }
                // A UI may also return ProgressDecision::Cancel from this observer.
                ProgressDecision::Continue
            },
        );
    plan.read_bytes(AssetPath::new("controlled.obj")?, bytes, control)
        .map_err(baozi::Error::from)
}

fn main() -> Result<()> {
    // A host can keep this clone on its UI thread and call cancellation.cancel().
    let cancellation = CancellationToken::new();
    let report = import_controlled(
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        cancellation.clone(),
    )?;
    println!("Imported {} mesh", report.scene().mesh_count());
    // Cancellation and deadlines are cooperative. A blocking backend call must return
    // before its surrounding checkpoints can observe a request.
    Ok(())
}
