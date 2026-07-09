use baozi::{ImportOptions, Importer, PostProcessPipeline, PostProcessStep};
use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn bench_obj_triangle(c: &mut Criterion) {
    let importer = Importer::new();
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
    let importer = Importer::new();
    let pipeline = PostProcessPipeline::new([
        PostProcessStep::Triangulate,
        PostProcessStep::GenerateBoundingBoxes,
    ]);
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
                .read_bytes_with_postprocess(
                    "quad.obj",
                    black_box(bytes),
                    ImportOptions::memory(),
                    &pipeline,
                )
                .unwrap()
        });
    });
}

criterion_group!(benches, bench_obj_triangle, bench_obj_quad_postprocess);
criterion_main!(benches);
