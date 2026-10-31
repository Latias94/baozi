use std::{path::PathBuf, process::ExitCode};

use baozi_asset_consumer::{Options, Result, process_file};
use serde_json::json;

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = args.next().map(PathBuf::from).ok_or("usage: baozi-asset-consumer ROOT RELATIVE_ASSET [--assume-standard-space] [--max-work-units N]")?;
    let source = args
        .next()
        .ok_or("missing relative asset path")?
        .into_string()
        .map_err(|_| "asset path must be UTF-8")?;
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--assume-standard-space") => {
                options.source_assumptions = Some(baozi::TargetSpace::STANDARD)
            }
            Some("--max-work-units") => {
                options.max_work_units = args
                    .next()
                    .ok_or("missing work limit")?
                    .to_str()
                    .ok_or("work limit must be UTF-8")?
                    .parse()?
            }
            _ => return Err(format!("unknown argument: {}", arg.to_string_lossy()).into()),
        }
    }
    #[cfg(feature = "allocation-metrics")]
    let (output, allocations) = {
        let mut result = None;
        let measured = allocation_counter::measure(|| {
            result = Some(process_file(&root, &source, options));
        });
        (
            result.expect("measurement executed the consumer")?,
            json!({
                "requested_peak_bytes": measured.bytes_max,
                "requested_total_bytes": measured.bytes_total,
                "allocation_requests": measured.count_total,
                "requested_retained_bytes_after_scene_drop": measured.bytes_current,
                "scope": "calling-thread allocator requests during root open, import, edits, and buffer extraction; output plus detached import evidence retained"
            }),
        )
    };
    #[cfg(not(feature = "allocation-metrics"))]
    let (output, allocations) = (
        process_file(&root, &source, options)?,
        serde_json::Value::Null,
    );
    let asset = &output.asset;
    let metrics = &output.metrics;
    println!(
        "{}",
        json!({
            "source": output.import_evidence.source().as_str(),
            "format": output.import_evidence.format().id().as_str(),
            "nodes": asset.nodes.len(), "meshes": asset.mesh_count, "primitives": asset.draws.len(),
            "instances": asset.instances.len(), "skins": asset.skins.len(), "animations": asset.animations.len(),
            "materials": asset.materials.len(), "images": asset.images.len(), "draw_vertices": asset.vertex_count(),
            "mesh_local_bounds": asset.bounds(), "loss_records": output.import_evidence.loss_evidence().len(),
            "import_micros": metrics.import_micros, "postprocess_micros": metrics.postprocess_micros,
            "buffers_micros": metrics.buffers_micros, "import_work": metrics.import_work,
            "postprocess_work": metrics.postprocess_work, "import_peak_live_bytes": metrics.import_peak_live_bytes,
            "max_postprocess_peak_live_bytes": metrics.max_postprocess_peak_live_bytes,
            "opened_assets": metrics.opened_assets, "changed_operations": metrics.changed_operations,
        "allocations": allocations,
            "target": "left-handed Y-up centimeters; counter-clockwise fronts",
            "memory_note": "library counters are operation-local; application buffers, decoder allowances and process RSS are separate"
        })
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("asset processing failed: {error}");
            let mut source = error.source();
            while let Some(cause) = source {
                eprintln!("caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}
