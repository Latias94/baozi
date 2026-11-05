use super::*;
use crate::canonical_scene::store::typed_storage::StoredAttributeSemantic;
use crate::canonical_scene::store::typed_storage::finite::FiniteScalarPool;
use crate::identity::{LogicalMeshId, SceneGenerationId};

#[derive(Default)]
struct Control {
    completed: u64,
    stop: Option<u64>,
}
impl TypedStorageBuildControl for Control {
    type Error = u64;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        if self.stop == Some(self.completed) {
            return Err(self.completed);
        }
        let result = action();
        self.completed += 1;
        Ok(result)
    }
    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        TypedStorageBuildControl::run_unit(self, action)
    }
}
impl TypedStorageValidationControl for Control {
    type Error = u64;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        TypedStorageBuildControl::run_unit(self, action).map(Some)
    }
}
fn source(count: usize, topology: TopologyKind) -> GeometryStorage {
    let mut geometry = GeometryStorage {
        integer_attribute_scalar_count: 0,
        logical_meshes: vec![StoredLogicalMesh {
            primitives: TableRange::new(0, count),
            name: None,
        }],
        primitives: Vec::new(),
        attributes: Vec::new(),
        mesh_instances: Vec::new(),
        names: Vec::new(),
        f32_values: FiniteScalarPool::fixture(&[]),
        f64_values: FiniteScalarPool::fixture(&[]),
        u32_values: Vec::new(),
        i32_values: Vec::new(),
    };
    let corners = if topology == TopologyKind::TriangleList {
        3
    } else {
        4
    };
    let mut f64_values = Vec::new();
    for _ in 0..count {
        let start = f64_values.len();
        f64_values.extend_from_slice(
            &[0.0, -0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0][..corners * 3],
        );
        geometry.attributes.push(StoredAttribute {
            semantic: StoredAttributeSemantic::Position,
            tangent_conventions: None,
            domain: AttributeDomain::Vertex,
            lanes: 3,
            element_count: corners as u32,
            payload: Payload::DenseF64(TableRange::new(start, corners * 3)),
        });
        geometry.primitives.push(StoredPrimitive {
            topology,
            vertex_count: corners as u32,
            corner_count: corners as u32,
            face_count: if topology == TopologyKind::TriangleList {
                1
            } else {
                2
            },
            polygon_boundaries: None,
            indices: None,
            material: crate::MaterialAssignment::Unspecified,
            attributes: TableRange::new(geometry.attributes.len() - 1, 1),
        });
    }
    geometry.f64_values = FiniteScalarPool::fixture(&f64_values);
    geometry
}
fn requests<'a>(count: usize) -> Vec<SceneGeometryReplacement<'a>> {
    let generation = SceneGenerationId::allocate().unwrap();
    (0..count)
        .map(|i| {
            SceneGeometryReplacement::new(PrimitiveId::new(
                LogicalMeshId::new(generation, 0),
                i as u32,
            ))
        })
        .collect()
}

#[test]
fn selected_reconstruction_is_once_linear_and_bit_preserving_at_32_128_512() {
    let mut previous = None;
    for count in [32, 128, 512] {
        let source = source(count, TopologyKind::TriangleList);
        let mut edits = requests(count);
        for edit in &mut edits {
            edit.normals = Some(VectorValues::F64(&[[0.0, 0.0, 1.0]; 3]));
        }
        let mut mappings = Vec::with_capacity(edits.len());
        let scratch = SceneGeometryPlanScratch::for_test(&mut mappings);
        let mut plan = None;
        let allocation = allocation_counter::measure(|| {
            plan = Some(
                GeometrySelectionPlan::try_new(&source, &edits, scratch, &mut Control::default())
                    .unwrap(),
            );
        });
        assert_eq!(allocation.count_total, 0);
        let plan = plan.unwrap();
        let mut build = Control::default();
        let output = plan.materialize(&mut build).unwrap();
        assert_eq!(build.completed, plan.work());
        let mut validation = Control::default();
        assert_eq!(
            output.validate(&mut validation, [1, count, count * 2, 0], 1, 0),
            Ok(true)
        );
        assert_eq!(validation.completed, plan.validation_work());
        let mut correspondence = Control::default();
        assert_eq!(
            plan.validate_correspondence(&output, &mut correspondence),
            Ok(true)
        );
        assert_eq!(correspondence.completed, plan.correspondence_work());
        if let Some(work) = previous {
            assert!(plan.work() <= work * 4);
            assert!(plan.work() >= work * 4 - 16);
        }
        previous = Some(plan.work());
        for i in 0..count {
            let Payload::DenseF64(range) = output.attributes[2 * i].payload else {
                panic!()
            };
            assert_eq!(
                range
                    .get(&output.f64_values)
                    .unwrap()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                source.f64_values[i * 9..i * 9 + 9]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn lowering_checks_exact_strip_parity_and_fan_origin_rotation() {
    for (topology, expected) in [
        (TopologyKind::TriangleStrip, [[0, 1, 2], [1, 3, 2]]),
        (TopologyKind::TriangleFan, [[1, 2, 0], [2, 3, 0]]),
    ] {
        let source = source(1, topology);
        let mut edits = requests(1);
        edits[0].origins = Some(SceneTriangleOrigins {
            triangles: &expected,
            face_offsets: &[0, 1, 2],
        });
        let mut mappings = Vec::with_capacity(edits.len());
        let plan = GeometrySelectionPlan::try_new(
            &source,
            &edits,
            SceneGeometryPlanScratch::for_test(&mut mappings),
            &mut Control::default(),
        )
        .unwrap();
        let output = plan.materialize(&mut Control::default()).unwrap();
        assert_eq!(
            output.primitives[0]
                .indices
                .unwrap()
                .get(&output.u32_values),
            Some(expected.as_flattened())
        );
        assert_eq!(
            output.validate(&mut Control::default(), [1, 1, 1, 0], 1, 0),
            Ok(true)
        );
        let rotated = expected.map(|[a, b, c]| [b, c, a]);
        let mut invalid = requests(1);
        invalid[0].origins = Some(SceneTriangleOrigins {
            triangles: &rotated,
            face_offsets: &[0, 1, 2],
        });
        let mut invalid_mappings = Vec::with_capacity(invalid.len());
        assert!(
            GeometrySelectionPlan::try_new(
                &source,
                &invalid,
                SceneGeometryPlanScratch::for_test(&mut invalid_mappings),
                &mut Control::default()
            )
            .is_err()
        );
    }
}

#[test]
fn every_materialization_action_is_cancellable_and_source_remains_unchanged() {
    let source = source(3, TopologyKind::TriangleList);
    let original: Vec<_> = source.f64_values.iter().map(|v| v.to_bits()).collect();
    let mut edits = requests(3);
    for edit in &mut edits {
        edit.normals = Some(VectorValues::F64(&[[0.0, 0.0, 1.0]; 3]));
    }
    let mut mappings = Vec::with_capacity(edits.len());
    let plan = GeometrySelectionPlan::try_new(
        &source,
        &edits,
        SceneGeometryPlanScratch::for_test(&mut mappings),
        &mut Control::default(),
    )
    .unwrap();
    for stop in 0..plan.work() {
        assert!(
            matches!(plan.materialize(&mut Control { completed: 0, stop: Some(stop) }), Err(TypedStorageBuildError::Control(actual)) if actual == stop)
        );
    }
    assert_eq!(
        source
            .f64_values
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        original
    );
    assert_eq!(source.attributes.len(), 3);
}

#[test]
fn tangent_dependency_mapping_uses_the_selected_authored_normal_domain() {
    let mut source = source(1, TopologyKind::TriangleList);
    let mut f64_values = source.f64_values.to_vec();
    for domain in [AttributeDomain::Vertex, AttributeDomain::Corner] {
        let start = f64_values.len();
        f64_values.extend_from_slice(&[0.0, 0.0, 1.0].repeat(3));
        source.attributes.push(StoredAttribute {
            semantic: Semantic::Normal,
            tangent_conventions: None,
            domain,
            lanes: 3,
            element_count: 3,
            payload: Payload::DenseF64(TableRange::new(start, 9)),
        });
    }
    let start = f64_values.len();
    f64_values.extend_from_slice(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    source.f64_values = FiniteScalarPool::fixture(&f64_values);
    source.attributes.push(StoredAttribute {
        semantic: Semantic::Texcoord(0),
        tangent_conventions: None,
        domain: AttributeDomain::Vertex,
        lanes: 2,
        element_count: 3,
        payload: Payload::DenseF64(TableRange::new(start, 6)),
    });
    source.primitives[0].attributes = TableRange::new(0, 4);
    for (domain, normal) in [(AttributeDomain::Vertex, 1), (AttributeDomain::Corner, 2)] {
        let mut edits = requests(1);
        edits[0].normal_domain = domain;
        edits[0].tangents = Some(VectorValues::F64(&[[1.0, 0.0, 0.0, 1.0]; 3]));
        let mut mappings = Vec::with_capacity(edits.len());
        let plan = GeometrySelectionPlan::try_new(
            &source,
            &edits,
            SceneGeometryPlanScratch::for_test(&mut mappings),
            &mut Control::default(),
        )
        .unwrap();
        assert_eq!(plan.mapping(0).normal, Some(normal));
        assert_eq!(plan.output_attribute(normal), Some(normal));
        let output = plan.materialize(&mut Control::default()).unwrap();
        assert_eq!(
            output.validate(&mut Control::default(), [1, 1, 5, 0], 1, 0),
            Ok(true)
        );
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(true)
        );
    }
}

#[test]
fn correspondence_follows_relocated_generated_and_source_payloads_and_rejects_changed_bits() {
    let source = source(1, TopologyKind::TriangleList);
    let mut edits = requests(1);
    edits[0].normals = Some(VectorValues::F64(&[[0.0, 0.0, 1.0]; 3]));
    let mut mappings = Vec::with_capacity(edits.len());
    let plan = GeometrySelectionPlan::try_new(
        &source,
        &edits,
        SceneGeometryPlanScratch::for_test(&mut mappings),
        &mut Control::default(),
    )
    .unwrap();
    let mut output = plan.materialize(&mut Control::default()).unwrap();
    let mut f64_values = output.f64_values.to_vec();
    f64_values.rotate_left(9);
    output.f64_values = FiniteScalarPool::fixture(&f64_values);
    for a in &mut output.attributes {
        let Payload::DenseF64(range) = a.payload else {
            panic!("dense fixture")
        };
        a.payload = Payload::DenseF64(TableRange::new((range.start() + 9) % 18, range.len()));
    }
    let mut control = Control::default();
    assert_eq!(
        plan.validate_correspondence(&output, &mut control),
        Ok(true)
    );
    assert_eq!(control.completed, plan.correspondence_work());
    for scalar in [0, 9] {
        let original = output.f64_values[scalar];
        f64_values[scalar] = f64::from_bits(original.to_bits() ^ 1);
        output.f64_values = FiniteScalarPool::fixture(&f64_values);
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(false)
        );
        f64_values[scalar] = original;
        output.f64_values = FiniteScalarPool::fixture(&f64_values);
    }
}
