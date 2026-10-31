mod common;

use baozi_core::{AttributeData, AttributeDomain, AttributeSemanticRef};
use baozi_postprocess::{
    ExistingNormals, NormalMode, NormalOptions, PostProcessCancellationToken, PostProcessControl,
    PostProcessOperation, PostProcessRunDisposition, PostProcessRunError, generate_normals,
};
use common::import_obj;

const TWO_FACES: &[u8] = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\n\
    vt 0 0\nvt 1 0\nvt 0 1\nvt 1 1\ns 1\nf 1/1 2/2 3/3\nf 1/4 4/3 2/2\n";

fn assert_vector(actual: &[f32], expected: [f32; 3]) {
    assert_eq!(actual.len(), 3);
    for (actual, expected) in actual.iter().zip(expected) {
        assert!((actual - expected).abs() < 2.0e-6, "{actual} != {expected}");
    }
}

#[test]
fn smooth_normals_cross_uv_seams_and_keep_authored_data() {
    let (scene, import_evidence) = import_obj(TWO_FACES).into_parts();
    let source = scene.meshes().next().unwrap().primitives().next().unwrap();
    let target = source.id();
    let indices = source.indices().unwrap().to_vec();
    let AttributeData::DenseF32 {
        values: texcoords, ..
    } = source.texcoords(0, AttributeDomain::Corner).unwrap().data()
    else {
        panic!("UV fixture");
    };
    let texcoords = texcoords.to_vec();
    let result = generate_normals(
        scene,
        target,
        NormalOptions::default(),
        PostProcessControl::default(),
    )
    .expect("smooth normals should publish");
    let (scene, report) = result.into_parts();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_eq!(
        report.operation(),
        PostProcessOperation::GenerateNormals {
            target,
            options: NormalOptions::default()
        }
    );
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
    assert!(!import_evidence.matches_scene(&scene));
    let evidence = report.normal_generation().unwrap();
    let output = scene.primitive(evidence.output_target()).unwrap();
    assert_eq!(output.indices().unwrap(), indices);
    assert_eq!(
        output.normals(AttributeDomain::Corner).unwrap().domain(),
        AttributeDomain::Corner
    );
    let AttributeData::DenseF32 { values, lanes: 3 } =
        output.normals(AttributeDomain::Corner).unwrap().data()
    else {
        panic!("normal values");
    };
    let shared = [
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    for (corner, normal) in values.as_chunks::<3>().0.iter().enumerate() {
        assert_vector(
            normal,
            if matches!(corner, 0 | 1 | 3 | 5) {
                shared
            } else if corner == 2 {
                [0.0, 0.0, 1.0]
            } else {
                [0.0, 1.0, 0.0]
            },
        );
    }
    let AttributeData::DenseF32 { values, .. } =
        output.texcoords(0, AttributeDomain::Corner).unwrap().data()
    else {
        panic!("UV values");
    };
    assert_eq!(values, texcoords);
    assert!(
        output
            .attributes()
            .map(|attribute| attribute.semantic())
            .eq([
                AttributeSemanticRef::Position,
                AttributeSemanticRef::Normal,
                AttributeSemanticRef::Texcoord(0),
                AttributeSemanticRef::SmoothingGroup,
            ])
    );
    assert!(!evidence.replaced_authored_normals());
    assert!(!evidence.removed_tangents());
    let output_target = evidence.output_target();
    drop(scene);
    drop(import_evidence);
    assert_eq!(
        report.normal_generation().unwrap().output_target(),
        output_target
    );
}

#[test]
fn flat_normals_and_zero_smoothing_groups_keep_hard_edges() {
    for (source, mode) in [
        (TWO_FACES.to_vec(), NormalMode::Flat),
        (
            String::from_utf8(TWO_FACES.to_vec())
                .unwrap()
                .replace("s 1", "s off")
                .into_bytes(),
            NormalMode::Smooth,
        ),
    ] {
        let scene = import_obj(&source).into_parts().0;
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let options = NormalOptions {
            mode,
            ..NormalOptions::default()
        };
        let (scene, report) =
            generate_normals(scene, target, options, PostProcessControl::default())
                .unwrap()
                .into_parts();
        let primitive = scene
            .primitive(report.normal_generation().unwrap().output_target())
            .unwrap();
        let AttributeData::DenseF32 { values, .. } =
            primitive.normals(AttributeDomain::Corner).unwrap().data()
        else {
            panic!("flat normal values");
        };
        for (corner, normal) in values.as_chunks::<3>().0.iter().enumerate() {
            assert_vector(
                normal,
                if corner < 3 {
                    [0.0, 0.0, 1.0]
                } else {
                    [0.0, 1.0, 0.0]
                },
            );
        }
    }
}

#[test]
fn authored_normals_require_explicit_replacement() {
    let fixture = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 1 0 0\nf 1//1 2//1 3//1\n";
    let scene = import_obj(fixture).into_parts().0;
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let generation = scene.generation();
    let (scene, report) = generate_normals(
        scene,
        target,
        NormalOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_eq!(scene.generation(), generation);
    assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
    assert!(report.normal_generation().is_none());
    let options = NormalOptions {
        existing: ExistingNormals::Replace,
        ..NormalOptions::default()
    };
    let (scene, report) = generate_normals(scene, target, options, PostProcessControl::default())
        .unwrap()
        .into_parts();
    assert!(
        report
            .normal_generation()
            .unwrap()
            .replaced_authored_normals()
    );
    let primitive = scene
        .primitive(report.normal_generation().unwrap().output_target())
        .unwrap();
    let AttributeData::DenseF32 { values, .. } =
        primitive.normals(AttributeDomain::Corner).unwrap().data()
    else {
        panic!("replacement normals");
    };
    for normal in values.as_chunks::<3>().0 {
        assert_vector(normal, [0.0, 0.0, 1.0]);
    }
}

#[test]
fn normal_generation_and_detached_evidence_cross_threads_on_a_one_mib_stack() {
    let scene = import_obj(TWO_FACES).into_parts().0;
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let success = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            generate_normals(
                scene,
                target,
                NormalOptions::default(),
                PostProcessControl::default(),
            )
            .expect("normal generation must fit an ordinary caller stack")
        })
        .unwrap()
        .join()
        .expect("normal generation thread");
    let (scene, report) = success.into_parts();
    let output_generation = scene.generation();
    drop(scene);
    std::thread::spawn(move || {
        let normals = report.normal_generation().unwrap();
        assert_eq!(normals.input_target(), target);
        assert_eq!(normals.output_target().generation(), output_generation);
        assert_eq!(normals.mode(), NormalMode::Smooth);
        assert_eq!(report.resources().current_live_bytes(), 0);
    })
    .join()
    .expect("detached normal evidence thread");
}

#[test]
fn degenerate_and_cancelled_generation_return_the_original_scene() {
    for cancelled in [false, true] {
        let (scene, evidence) = import_obj(b"v 0 0 0\nv 1 0 0\nv 2 0 0\nf 1 2 3\n").into_parts();
        let generation = scene.generation();
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let cancellation = PostProcessCancellationToken::new();
        if cancelled {
            cancellation.cancel();
        }
        let failure = generate_normals(
            scene,
            target,
            NormalOptions::default(),
            PostProcessControl::default().with_cancellation(cancellation),
        )
        .unwrap_err();
        assert_eq!(failure.input().generation(), generation);
        assert!(evidence.matches_scene(failure.input()));
        assert!(
            failure
                .input()
                .primitive(target)
                .unwrap()
                .normals(AttributeDomain::Corner)
                .is_none()
        );
        let PostProcessRunError::Normals { failure } = failure.error() else {
            panic!("normal error");
        };
        assert_eq!(failure.resources().unwrap().current_live_bytes(), 0);
        assert_eq!(failure.resources().unwrap().committed_canonical_bytes(), 0);
    }
}
