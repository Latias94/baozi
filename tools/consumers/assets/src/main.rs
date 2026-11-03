use std::{path::PathBuf, process::ExitCode};

use baozi_asset_consumer::{Options, Result, process_file};
use serde_json::json;

fn compare(args: &[std::ffi::OsString]) -> Result<()> {
    use baozi_asset_consumer::comparison::{self, Fields, IoMode, Stage};
    let mut options = comparison::Options::default();
    let result = (|| {
        let root = args.first().map(PathBuf::from).ok_or("missing root")?;
        let source = args
            .get(1)
            .and_then(|arg| arg.to_str())
            .ok_or("missing UTF-8 asset path")?;
        let mut remaining = args[2..].iter();
        while let Some(flag) = remaining.next() {
            let value = remaining
                .next()
                .and_then(|arg| arg.to_str())
                .ok_or("missing comparison option value")?;
            match flag.to_str() {
                Some("--comparison") => options.stage = Stage::parse(value)?,
                Some("--io-mode") => {
                    options.io_mode = match value {
                        "file" => IoMode::File,
                        "memory" => IoMode::Memory,
                        _ => return Err("unknown comparison IO mode".into()),
                    }
                }
                Some("--fields") => {
                    options.fields = match value {
                        "position" => Fields::Position,
                        "position-normal" => Fields::PositionNormal,
                        "position-normal-uv0" => Fields::PositionNormalUv0,
                        _ => return Err("unknown comparison fields".into()),
                    }
                }
                Some("--normals") => {
                    options.generate_missing_normals = match value {
                        "missing" => true,
                        "preserve" => false,
                        _ => return Err("unknown comparison normal policy".into()),
                    }
                }
                Some("--max-work-units") => options.max_work_units = value.parse()?,
                _ => return Err("unknown comparison option".into()),
            }
        }
        comparison::measure_file(&root, source, options)
    })();
    match result {
        Ok(measurement) => {
            println!("{}", measurement.summary);
            std::hint::black_box(&measurement.output);
            Ok(())
        }
        Err(error) => {
            println!("{}", comparison::failure(error.as_ref(), options));
            Err(error)
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|arg| arg == "--comparison") {
        return compare(&args);
    }
    let mut args = std::env::args_os().skip(1);
    let root = args.next().map(PathBuf::from).ok_or("usage: baozi-asset-consumer ROOT RELATIVE_ASSET [--assume-standard-space] [--max-work-units N] [--postprocess-actions N] [--postprocess-bytes N]")?;
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
            Some("--postprocess-actions") | Some("--postprocess-bytes") => {
                let value: u64 = args
                    .next()
                    .ok_or("missing postprocess limit")?
                    .to_str()
                    .ok_or("postprocess limit must be UTF-8")?
                    .parse()?;
                if arg == "--postprocess-actions" {
                    options.postprocess_budgets.max_work_actions = value;
                } else {
                    options.postprocess_budgets.storage_bytes = value;
                }
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
            "source_json_metadata_entries": asset.source_json_metadata.len(),
            "source_json_metadata_payload_bytes": asset.source_json_metadata.iter().map(|m| m.payload.len()).sum::<usize>(),
            "mesh_local_bounds": asset.bounds(), "loss_records": output.import_evidence.loss_evidence().len(),
            "import_micros": metrics.import_micros, "postprocess_micros": metrics.postprocess_micros,
            "source_trace_micros": metrics.source_trace_micros,
            "traced_primitive_owners": metrics.traced_primitive_owners,
            "traced_provenance_nodes": metrics.traced_provenance_nodes,
            "traced_source_objects": metrics.traced_source_objects,
            "buffers_micros": metrics.buffers_micros, "import_work": metrics.import_work,
            "postprocess_work_actions": metrics.postprocess_work_actions,
            "postprocess_work_action_contract": metrics.postprocess_work_action_contract,
            "import_peak_live_bytes": metrics.import_peak_live_bytes,
            "max_postprocess_peak_live_bytes": metrics.max_postprocess_peak_live_bytes,
            "opened_assets": metrics.opened_assets, "changed_operations": metrics.changed_operations,
            "selected_geometry_reconstructions": metrics.selected_geometry_reconstructions,
        "allocations": allocations,
            "target": "left-handed Y-up centimeters; counter-clockwise fronts",
            "memory_note": "library counters are operation-local; application buffers and process RSS are separate"
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
