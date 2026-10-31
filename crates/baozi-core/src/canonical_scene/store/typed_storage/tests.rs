use super::build::{NodeStoragePlan, TypedStorageBuildControl, TypedStorageBuildError};
use super::validation::TypedStorageValidationControl;
use super::*;

#[derive(Default)]
struct Control {
    completed: usize,
    reject_before: Option<usize>,
}

impl TypedStorageValidationControl for Control {
    type Error = ();

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, ()> {
        if self.reject_before == Some(self.completed) {
            return Err(());
        }
        let value = action();
        self.completed += 1;
        Ok(Some(value))
    }
}

impl TypedStorageBuildControl for &mut Control {
    type Error = ();

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, ()> {
        TypedStorageValidationControl::run_unit(*self, action)?.ok_or(())
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, ()> {
        TypedStorageBuildControl::run_unit(self, action)
    }
}

fn geometry() -> GeometryStorage {
    GeometryStorage {
        logical_meshes: vec![StoredLogicalMesh {
            primitives: TableRange::new(0, 1),
            name: None,
        }],
        primitives: vec![StoredPrimitive {
            topology: TopologyKind::TriangleList,
            vertex_count: 3,
            corner_count: 3,
            face_count: 1,
            polygon_boundaries: None,
            indices: Some(TableRange::new(3, 3)),
            material: None,
            attributes: TableRange::new(0, 4),
        }],
        attributes: vec![
            StoredAttribute {
                semantic: StoredAttributeSemantic::Position,
                domain: AttributeDomain::Vertex,
                lanes: 3,
                element_count: 3,
                payload: StoredAttributePayload::DenseF32(TableRange::new(0, 9)),
            },
            StoredAttribute {
                semantic: StoredAttributeSemantic::Normal,
                domain: AttributeDomain::Corner,
                lanes: 3,
                element_count: 3,
                payload: StoredAttributePayload::SparseF32 {
                    indices: TableRange::new(0, 2),
                    values: TableRange::new(9, 6),
                    default: TableRange::new(15, 3),
                },
            },
            StoredAttribute {
                semantic: StoredAttributeSemantic::SmoothingGroup,
                domain: AttributeDomain::Face,
                lanes: 1,
                element_count: 1,
                payload: StoredAttributePayload::DenseU32(TableRange::new(2, 1)),
            },
            StoredAttribute {
                semantic: StoredAttributeSemantic::Custom(TextRange(TableRange::new(0, 8))),
                domain: AttributeDomain::Vertex,
                lanes: 1,
                element_count: 3,
                payload: StoredAttributePayload::DenseF64(TableRange::new(0, 3)),
            },
        ],
        mesh_instances: vec![StoredMeshInstance {
            owner_node: StoredNodeIndex(1),
            logical_mesh: StoredLogicalMeshIndex(0),
        }],
        names: b"ply:mass".to_vec(),
        f32_values: vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0,
            1.0,
        ],
        f64_values: vec![1.0e200, 2.0e200, 3.0e200],
        u32_values: vec![0, 2, 7, 0, 1, 2],
        i32_values: vec![],
    }
}

#[test]
fn validation_borrows_vertex_corner_face_sparse_and_f64_tables_without_allocating() {
    let storage = geometry();
    let before = storage.f32_values.as_ptr();
    let mut control = Control::default();
    let allocations = allocation_counter::measure(|| {
        assert_eq!(storage.validate(&mut control, [1, 1, 4, 1], 2, 0), Ok(true));
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(control.completed, 35);
    assert_eq!(storage.f32_values.as_ptr(), before);
    assert_eq!(storage.f64_values, [1.0e200, 2.0e200, 3.0e200]);
}

#[test]
fn every_geometry_validation_action_is_interruptible_before_its_effect() {
    let storage = geometry();
    for completed in 0..35 {
        let mut control = Control {
            completed: 0,
            reject_before: Some(completed),
        };
        assert_eq!(storage.validate(&mut control, [1, 1, 4, 1], 2, 0), Err(()));
        assert_eq!(control.completed, completed);
    }
}

#[test]
fn typed_ranges_reject_foreign_domain_targets_and_uncovered_payloads() {
    let mut storage = geometry();
    storage.mesh_instances[0].owner_node = StoredNodeIndex(0);
    assert_eq!(
        storage.validate(&mut Control::default(), [1, 1, 4, 1], 2, 0),
        Ok(false)
    );
    storage.mesh_instances[0].owner_node = StoredNodeIndex(1);
    storage.u32_values[1] = 0;
    assert_eq!(
        storage.validate(&mut Control::default(), [1, 1, 4, 1], 2, 0),
        Ok(false)
    );
    storage.u32_values[1] = 2;
    storage.f32_values.push(17.0);
    assert_eq!(
        storage.validate(&mut Control::default(), [1, 1, 4, 1], 2, 0),
        Ok(false)
    );
}

#[test]
fn typed_geometry_rejects_invalid_ranges_cardinality_schema_and_nonfinite_values() {
    let corruptions: [fn(&mut GeometryStorage); 8] = [
        |storage| storage.primitives[0].attributes = TableRange::new(usize::MAX, 2),
        |storage| storage.attributes[1].element_count = 2,
        |storage| storage.attributes[0].domain = AttributeDomain::Corner,
        |storage| {
            storage.attributes[3].payload = StoredAttributePayload::DenseF64(TableRange::new(0, 2))
        },
        |storage| storage.f64_values[0] = f64::INFINITY,
        |storage| storage.f32_values[15] = f32::NAN,
        |storage| storage.u32_values[5] = 3,
        |storage| storage.names[7] = b'\n',
    ];
    for corrupt in corruptions {
        let mut storage = geometry();
        corrupt(&mut storage);
        assert_eq!(
            storage.validate(&mut Control::default(), [1, 1, 4, 1], 2, 0),
            Ok(false),
        );
    }
}

#[test]
fn ordered_typed_children_and_authored_transform_variants_remain_explicit() {
    let mut storage = NodeStorage {
        transform_scalar_count: 26,
        nodes: vec![
            StoredNode {
                children: TableRange::new(0, 2),
                transform: StoredTransform::Synthetic,
                name: None,
                camera: None,
                light: None,
            },
            StoredNode {
                children: TableRange::new(2, 0),
                transform: StoredTransform::Matrix(Mat4::IDENTITY),
                name: Some(TextRange(TableRange::new(0, 0))),
                camera: None,
                light: None,
            },
            StoredNode {
                children: TableRange::new(2, 0),
                transform: StoredTransform::from_authored(AuthoredTransform::identity_trs())
                    .unwrap(),
                name: None,
                camera: None,
                light: None,
            },
        ],
        children: vec![StoredNodeIndex(1), StoredNodeIndex(2)],
        names: vec![],
    };
    assert_eq!(
        storage.validate(&mut Control::default(), 3, 2, 0, &mut [], &mut []),
        Ok(true)
    );
    assert_eq!(
        storage
            .source_node(0)
            .unwrap()
            .name
            .unwrap()
            .get(&storage.names),
        Some("")
    );
    assert!(storage.source_node(1).unwrap().name.is_none());
    storage.children.swap(0, 1);
    assert_eq!(
        storage.validate(&mut Control::default(), 3, 2, 0, &mut [], &mut []),
        Ok(false)
    );
}

struct BuildControl;
impl TypedStorageBuildControl for BuildControl {
    type Error = ();
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, ()> {
        Ok(action())
    }
    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, ()> {
        Ok(action())
    }
}

#[test]
fn matrix_trs_replacement_reuses_one_node_layout_and_retains_name_and_child_ranges() {
    let source = NodeStorage {
        transform_scalar_count: 16,
        nodes: vec![
            StoredNode {
                children: TableRange::new(0, 1),
                transform: StoredTransform::Synthetic,
                name: None,
                camera: None,
                light: None,
            },
            StoredNode {
                children: TableRange::new(1, 0),
                transform: StoredTransform::Matrix(Mat4::IDENTITY),
                name: Some(TextRange(TableRange::new(0, 4))),
                camera: None,
                light: None,
            },
        ],
        children: vec![StoredNodeIndex(1)],
        names: b"name".to_vec(),
    };
    let replacement = AuthoredTransform::identity_trs();
    let plan = NodeStoragePlan::for_storage(&source).unwrap();
    let rewritten = plan.rewrite(&source, 1, replacement, BuildControl).unwrap();
    assert_eq!(rewritten.transform_scalar_count, 10);
    assert!(
        rewritten.nodes[1]
            .transform
            .authored()
            .unwrap()
            .has_same_authored_bits(replacement)
    );
    assert!(matches!(
        source.nodes[1].transform,
        StoredTransform::Matrix(_)
    ));
    assert_eq!(
        rewritten.nodes[1].name.unwrap().get(&rewritten.names),
        Some("name")
    );
    assert_eq!(rewritten.roots(), source.roots());
    assert_eq!(
        rewritten.validate(&mut Control::default(), 2, 1, 4, &mut [], &mut []),
        Ok(true)
    );

    // Interrupt every row, child, and UTF-8 copy action. Partially built vectors must
    // be released even when the central control refuses the next action.
    for reject_before in 0..usize::try_from(plan.work().unwrap()).unwrap() {
        let mut control = Control {
            completed: 0,
            reject_before: Some(reject_before),
        };
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(
                plan.rewrite(&source, 1, replacement, &mut control),
                Err(TypedStorageBuildError::Control(())),
            ));
        });
        assert_eq!(control.completed, reject_before);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
    }
}
