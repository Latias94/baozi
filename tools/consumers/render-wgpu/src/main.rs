use baozi::{CameraProjection, LocalAxes, TargetSpace};
use baozi_asset_consumer::{
    EvaluationLimits, EvaluationRequest, FramePolicy, Options, SampleTime, evaluate, process_file,
};
use baozi_render_wgpu::{
    Control,
    deformation::evaluated_frames,
    device::{DeviceLimits, DevicePlan},
    renderer::{AdapterSelector, Backend, RenderSettings, Renderer, adapter_report, adapters},
    spatial::{CameraInput, IDENTITY},
};
use std::{collections::BTreeMap, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args
        .next()
        .ok_or("expected adapters or render; see README.md")?;
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        if !key.starts_with("--") {
            return Err(format!("unexpected argument: {key}").into());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?;
        if options.insert(key, value).is_some() {
            return Err("duplicate option".into());
        }
    }
    let get = |key: &str| {
        options
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("required option {key}"))
    };
    let backend = match get("--backend")? {
        "dx12" => Backend::Dx12,
        "vulkan" => Backend::Vulkan,
        _ => return Err("backend must be dx12 or vulkan".into()),
    };
    if command == "adapters" {
        if options.len() != 1 {
            return Err("adapters accepts only --backend".into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(
                &adapters(backend)
                    .iter()
                    .map(adapter_report)
                    .collect::<Vec<_>>()
            )?
        );
        return Ok(());
    }
    if command != "render" {
        return Err("expected adapters or render".into());
    }
    let allowed = [
        "--backend",
        "--adapter",
        "--vendor",
        "--device",
        "--root",
        "--asset",
        "--output",
        "--camera-node",
        "--eye",
        "--clip",
        "--time",
        "--width",
        "--height",
        "--source-revision",
    ];
    if options.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err("unknown option; see README.md".into());
    }
    let selector = AdapterSelector {
        backend,
        name: get("--adapter")?.into(),
        vendor: parse_id(get("--vendor")?)?,
        device: parse_id(get("--device")?)?,
    };
    let root = PathBuf::from(get("--root")?);
    let relative = get("--asset")?;
    let output = PathBuf::from(get("--output")?);
    let width = options
        .get("--width")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(512);
    let height = options
        .get("--height")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(512);
    let clip = options
        .get("--clip")
        .map(|v| v.parse::<usize>())
        .transpose()?;
    let time = options
        .get("--time")
        .map(|v| v.parse::<f64>())
        .transpose()?
        .unwrap_or(0.0);
    let camera = match (options.get("--camera-node"), options.get("--eye")) {
        (Some(node), None) => CameraInput::Node(node.parse()?),
        (None, Some(eye)) => {
            let coordinates = eye
                .split(',')
                .map(str::parse::<f64>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if coordinates.len() != 3 || coordinates.iter().any(|v| !v.is_finite()) {
                return Err("--eye requires finite x,y,z".into());
            }
            let mut world = IDENTITY;
            world[3][..3].copy_from_slice(&coordinates);
            CameraInput::Explicit {
                projection: CameraProjection::perspective(
                    LocalAxes::STANDARD,
                    std::f32::consts::FRAC_PI_2,
                    None,
                    0.01,
                    None,
                )?,
                world,
            }
        }
        _ => {
            return Err(
                "choose exactly one of --camera-node or --eye (eye looks along -Z, +Y up)".into(),
            );
        }
    };
    // process_file destroys Scene/importer/IO before this function creates any GPU resource.
    let loaded = process_file(
        &root,
        relative,
        Options {
            target: TargetSpace::STANDARD,
            ..Default::default()
        },
    )?;
    let asset = loaded.asset;
    drop(loaded.import_evidence);
    let evaluation = evaluate(
        &asset,
        clip,
        SampleTime::Declared(time),
        EvaluationRequest {
            frames: FramePolicy::FrozenWeightNormalsOnly,
            limits: EvaluationLimits::default(),
            cancellation: None,
        },
    )?;
    let frames = evaluated_frames(&evaluation)?;
    let plan = DevicePlan::prepare(
        &asset,
        &evaluation,
        &frames,
        DeviceLimits::default(),
        Control::default(),
    )?;
    let renderer = Renderer::new(&selector)?;
    let uploaded = renderer.upload(
        &plan,
        camera,
        RenderSettings {
            width,
            height,
            ..Default::default()
        },
        &[],
        Control::default(),
    )?;
    drop(plan);
    drop(frames);
    drop(evaluation);
    drop(asset);
    let mut frame = renderer.render(&uploaded, Control::default())?;
    frame.report["source_revision"] = options
        .get("--source-revision")
        .map(|v| serde_json::json!(v))
        .unwrap_or(serde_json::Value::Null);
    frame.report["asset_path"] = serde_json::json!(relative);
    frame.report["clip"] = serde_json::json!(clip);
    frame.report["declared_time"] = serde_json::json!(time);
    frame.report["evaluation_frame_policy"] = serde_json::json!("FrozenWeightNormalsOnly");
    frame.report["target"] = serde_json::json!(std::env::consts::ARCH);
    frame.report["os"] = serde_json::json!(std::env::consts::OS);
    frame.save(&output)?;
    println!("{}", serde_json::to_string_pretty(&frame.report)?);
    Ok(())
}

fn parse_id(value: &str) -> Result<u32, Box<dyn std::error::Error>> {
    Ok(if let Some(hex) = value.strip_prefix("0x") {
        u32::from_str_radix(hex, 16)?
    } else {
        value.parse()?
    })
}
