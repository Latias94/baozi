use baozi::{Importer, PostProcessPipeline, PostProcessStep};
use baozi_test_support::{ObjScale, ObjScaleFixture};
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

fn bench_obj_triangle(c: &mut Criterion) {
    let importer = Importer::new().expect("default format registry should freeze");
    let bytes = b"o triangle
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";

    c.bench_function("obj_triangle_import", |b| {
        b.iter(|| {
            importer
                .read_bytes("triangle.obj", black_box(bytes))
                .unwrap()
        });
    });
}

fn bench_obj_quad_postprocess(c: &mut Criterion) {
    let importer = Importer::new().expect("default format registry should freeze");
    let pipeline = PostProcessPipeline::new([
        PostProcessStep::Triangulate,
        PostProcessStep::GenerateNormals,
        PostProcessStep::GenerateBoundingBoxes,
    ])
    .expect("the static benchmark post-process plan must be valid");
    let bytes = b"o quad
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
f 1 2 3 4
";

    c.bench_function("obj_quad_import_postprocess", |b| {
        b.iter(|| {
            importer
                .read_bytes_with_postprocess("quad.obj", black_box(bytes), &pipeline)
                .unwrap()
        });
    });
}

fn bench_obj_face_scale(c: &mut Criterion) {
    let importer = Importer::new().expect("default format registry should freeze");
    let mut group = c.benchmark_group("obj_face_import_scale");

    for scale in ObjScale::PROFILE {
        let fixture =
            ObjScaleFixture::try_new(scale).expect("bounded scale fixture should allocate");
        group.throughput(Throughput::Bytes(fixture.bytes().len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(scale.label()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    importer
                        .read_bytes("scale.obj", black_box(fixture.bytes()))
                        .unwrap()
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_obj_triangle,
    bench_obj_quad_postprocess,
    bench_obj_face_scale
);
criterion_main!(benches);
