#![cfg(feature = "format-obj")]

use std::fmt::Write;

use baozi::{
    AttributeData, AttributeDomain, AttributeSemanticRef, ImportDimension, ImportTerminalCause,
    Importer, TopologyKind,
};

fn unique_triangles_obj(triangle_count: usize) -> String {
    // Five decimal digits cover every coordinate and positive index in the bounded cases.
    // Three 12-byte vertex records and one 20-byte face record make byte growth exact, too.
    let vertex_count = 3 * triangle_count;
    assert!(vertex_count <= 99_999);
    let mut document = String::with_capacity(56 * triangle_count);
    for vertex in 0..vertex_count {
        let y = usize::from(vertex % 3 == 1);
        let z = usize::from(vertex % 3 == 2);
        writeln!(document, "v {vertex:05} {y} {z}").expect("write OBJ position");
    }
    for first in (1..=vertex_count).step_by(3) {
        writeln!(document, "f {first:05} {:05} {:05}", first + 1, first + 2)
            .expect("write OBJ face");
    }
    assert_eq!(document.len(), 56 * triangle_count);
    document
}

fn position_sort_work_bound(vertex_count: usize) -> u64 {
    let count = vertex_count as u64;
    // mesh_builder::materialize_admitted_geometry sorts one position reference per corner.
    // MeteredVec::try_sort_unstable_by uses heap sort: heap construction needs at most
    // 2*m comparisons (sum of subtree heights < m), and extraction at most
    // 2*m*floor(log2(m)). mtl::ComparisonWork rounds the one sort up to 64 work units.
    let comparisons = 2 * count * (u64::from(vertex_count.ilog2()) + 1);
    comparisons.div_ceil(64) * 64
}

#[test]
fn obj_unique_vertices_preserve_typed_geometry_and_bound_metered_work_growth() {
    let importer = Importer::new().expect("default format registry should freeze");
    let mut previous_work: Option<u64> = None;

    for triangle_count in [256_usize, 1_024, 4_096] {
        let vertex_count = 3 * triangle_count;
        let document = unique_triangles_obj(triangle_count);
        let report = importer
            .read_bytes("unique-vertices.obj", document.as_bytes())
            .expect("bounded unique-vertex OBJ should import");

        assert_eq!(report.format().id().as_str(), "org.baozi.obj");
        assert!(report.diagnostics().is_empty());
        assert!(report.loss_evidence().is_empty());
        let scene = report.scene();
        assert_eq!(scene.mesh_count(), 1);
        assert_eq!(scene.primitive_count(), 1);
        assert_eq!(scene.attribute_count(), 1);
        assert_eq!(scene.mesh_instance_count(), 1);
        let mesh = scene.meshes().next().expect("one logical mesh");
        assert_eq!(mesh.primitive_count(), 1);
        let primitive = mesh.primitives().next().expect("one indexed primitive");
        assert_eq!(primitive.topology(), TopologyKind::TriangleList);
        assert_eq!(primitive.vertex_count() as usize, vertex_count);
        assert_eq!(primitive.corner_count() as usize, vertex_count);
        assert_eq!(primitive.face_count() as usize, triangle_count);
        assert_eq!(primitive.polygon_face_sizes(), None);
        let indices = primitive.indices().expect("explicit position indices");
        assert_eq!(indices.len(), vertex_count);
        let positions = primitive.positions().expect("vertex positions");
        assert_eq!(positions.semantic(), AttributeSemanticRef::Position);
        assert_eq!(positions.domain(), AttributeDomain::Vertex);
        assert_eq!(positions.len(), vertex_count);
        let AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
            panic!("expected borrowed dense F32 positions");
        };
        assert_eq!(values.len(), 3 * vertex_count);
        let (position_arrays, remainder) = values.as_chunks::<3>();
        assert!(remainder.is_empty());
        for (vertex, (&index, position)) in indices.iter().zip(position_arrays).enumerate() {
            // Every index is used exactly once. The strictly increasing x coordinate makes
            // all 3*N positions distinct; each triple also forms a nondegenerate triangle.
            assert_eq!(
                index as usize, vertex,
                "{triangle_count} triangles, index {vertex}"
            );
            assert_eq!(
                *position,
                [
                    vertex as f32,
                    f32::from(u8::from(vertex % 3 == 1)),
                    f32::from(u8::from(vertex % 3 == 2)),
                ],
                "{triangle_count} triangles, position {vertex}"
            );
        }

        let ledger = report.ledger();
        assert_eq!(ledger.terminal_cause(), ImportTerminalCause::Success);
        assert_eq!(ledger.current_live_bytes(), 0);
        assert_eq!(ledger.current_evidence_bytes(), 0);
        assert_eq!(ledger.active_reservations(), 0);
        assert!(ledger.peak_live_bytes() > 0);
        for (dimension, expected) in [
            (ImportDimension::PrimaryBytes, document.len() as u64),
            // Both parsing source positions and materializing canonical positions are metered.
            (ImportDimension::Vertices, 2 * vertex_count as u64),
            (ImportDimension::Faces, triangle_count as u64),
            (ImportDimension::TopologyCells, 2 * vertex_count as u64),
        ] {
            let evidence = ledger.dimension(dimension).expect("metered OBJ dimension");
            assert_eq!(evidence.attempted(), expected, "{dimension:?} attempted");
            assert_eq!(evidence.consumed(), expected, "{dimension:?} consumed");
            assert_eq!(evidence.current_live_bytes(), 0, "{dimension:?} live bytes");
        }
        let evidence = ledger
            .dimension(ImportDimension::WorkUnits)
            .expect("real import pipeline work evidence");
        let work = evidence.consumed();
        assert_eq!(evidence.attempted(), work);
        assert_eq!(evidence.current_live_bytes(), 0);
        // Source hashing charges every byte. In addition, the OBJ normalizer has six complete
        // corner passes: reference copy, first-occurrence initialization, grouping, index
        // initialization, position materialization, and remapping. Core work adds to this.
        assert!(work >= document.len() as u64 + 6 * vertex_count as u64);
        eprintln!(
            "Unique vertices: triangles={triangle_count}, vertices={vertex_count}, \
             input_bytes={}, consumed_work={work}, peak_declared_bytes={}",
            document.len(),
            ledger.peak_live_bytes()
        );

        if let Some(previous) = previous_work {
            // Bytes, faces, vertices, and indices grow fourfold; the single primitive and
            // profile stay fixed. Their scans, construction, and validation are affine.
            // Position-reference heap sort is the sole superlinear pass for this input.
            // Allow its entire proven upper bound at the larger size, without assuming a
            // minimum comparison count at the smaller size or using a measured tolerance.
            let sort_allowance = position_sort_work_bound(vertex_count);
            let maximum = previous
                .checked_mul(4)
                .and_then(|scaled| scaled.checked_add(sort_allowance))
                .expect("bounded pipeline work envelope should fit u64");
            assert!(work > previous, "unique-vertex pipeline work must increase");
            assert!(
                work <= maximum,
                "{triangle_count} triangles: consumed {work} work units exceeds \
                 fourfold previous work {previous} plus heap-sort allowance {sort_allowance} \
                 (maximum {maximum})"
            );
        }
        previous_work = Some(work);
    }
}
