use super::*;
use crate::canonical_scene::store::typed_storage::{
    StoredLogicalMeshIndex, StoredNodeIndex, TextRange,
};
use crate::scene_space::{Axis, FrontFaceWinding, Handedness, SceneSpace};
use crate::space_conversion::TargetSpace;

#[derive(Default)]
struct Control {
    completed: u64,
    reject_before: Option<u64>,
    omit_before: Option<u64>,
}

impl Control {
    fn step<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
        if self.reject_before == Some(self.completed) {
            return Err(self.completed);
        }
        let result = action();
        self.completed += 1;
        Ok(result)
    }
}
impl TypedStorageBuildControl for Control {
    type Error = u64;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
        self.step(action)
    }
    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
        self.step(action)
    }
}
impl TypedStorageValidationControl for Control {
    type Error = u64;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, u64> {
        if self.omit_before == Some(self.completed) {
            return Ok(None);
        }
        self.step(action).map(Some)
    }
}

fn values<T: Copy>(pool: &mut Vec<T>, values: &[T]) -> TableRange<T> {
    let start = pool.len();
    pool.extend_from_slice(values);
    TableRange::new(start, values.len())
}
fn empty() -> GeometryStorage {
    GeometryStorage {
        logical_meshes: Vec::new(),
        primitives: Vec::new(),
        attributes: Vec::new(),
        mesh_instances: Vec::new(),
        names: Vec::new(),
        f32_values: Vec::new(),
        f64_values: Vec::new(),
        u32_values: Vec::new(),
        i32_values: Vec::new(),
    }
}
fn attribute(
    storage: &mut GeometryStorage,
    semantic: StoredAttributeSemantic,
    domain: AttributeDomain,
    lanes: u8,
    count: u32,
    payload: StoredAttributePayload,
) {
    storage.attributes.push(StoredAttribute {
        semantic,
        domain,
        lanes,
        element_count: count,
        payload,
    });
}
fn finish_fixture(
    storage: &mut GeometryStorage,
    topology: TopologyKind,
    shape: [u32; 3],
    boundaries: Option<&[u32]>,
    indices: Option<&[u32]>,
) {
    let [vertices, corners, faces] = shape;
    let polygon_boundaries = boundaries.map(|data| values(&mut storage.u32_values, data));
    let indices = indices.map(|data| values(&mut storage.u32_values, data));
    storage.primitives.push(StoredPrimitive {
        topology,
        vertex_count: vertices,
        corner_count: corners,
        face_count: faces,
        polygon_boundaries,
        indices,
        material: None,
        attributes: TableRange::new(0, storage.attributes.len()),
    });
    let name = TextRange(values(&mut storage.names, b"shared"));
    storage.logical_meshes.push(StoredLogicalMesh {
        primitives: TableRange::new(0, 1),
        name: Some(name),
    });
    storage.mesh_instances.extend([
        StoredMeshInstance {
            owner_node: StoredNodeIndex(1),
            logical_mesh: StoredLogicalMeshIndex(0),
        },
        StoredMeshInstance {
            owner_node: StoredNodeIndex(2),
            logical_mesh: StoredLogicalMeshIndex(0),
        },
    ]);
}
fn conversion(hand: Handedness, units: f64, winding: FrontFaceWinding) -> SpaceConversion {
    SpaceConversion::try_new(
        &SceneSpace::STANDARD_TARGET,
        TargetSpace::try_new(hand, Axis::PositiveY, Axis::NegativeZ, units, winding).unwrap(),
        None,
    )
    .unwrap()
}
fn reflected() -> SpaceConversion {
    conversion(Handedness::Left, 0.5, FrontFaceWinding::CounterClockwise)
}
fn winding_only() -> SpaceConversion {
    conversion(Handedness::Right, 1.0, FrontFaceWinding::Clockwise)
}

fn triangles(indexed: bool) -> GeometryStorage {
    let mut storage = empty();
    let vertices = if indexed { 4 } else { 6 };
    let positions = [
        [1.0, 2.0, 3.0],
        [4.0, 5.0, 6.0],
        [7.0, 8.0, 9.0],
        [10.0, 11.0, 12.0],
        [13.0, 14.0, 15.0],
        [16.0, 17.0, 18.0],
    ];
    let positions = values(
        &mut storage.f32_values,
        positions[..vertices as usize].as_flattened(),
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        vertices,
        StoredAttributePayload::DenseF32(positions),
    );
    let normals = values(
        &mut storage.f32_values,
        vec![[2.0, 3.0, 5.0]; vertices as usize].as_flattened(),
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Normal,
        AttributeDomain::Vertex,
        3,
        vertices,
        StoredAttributePayload::DenseF32(normals),
    );
    let payload = StoredAttributePayload::SparseF32 {
        indices: values(&mut storage.u32_values, &[1, 2, 4]),
        values: values(
            &mut storage.f32_values,
            &[2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 2.0, 3.0, 5.0],
        ),
        default: values(&mut storage.f32_values, &[2.0, 3.0, 5.0]),
    };
    attribute(
        &mut storage,
        StoredAttributeSemantic::Normal,
        AttributeDomain::Corner,
        3,
        6,
        payload,
    );
    let tangents = values(
        &mut storage.f32_values,
        vec![[2.0, 3.0, 5.0, 1.0]; vertices as usize].as_flattened(),
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Tangent,
        AttributeDomain::Vertex,
        4,
        vertices,
        StoredAttributePayload::DenseF32(tangents),
    );
    let payload = StoredAttributePayload::SparseF32 {
        indices: values(&mut storage.u32_values, &[1, 2, 5]),
        values: values(
            &mut storage.f32_values,
            &[
                2.0, 3.0, 5.0, -0.0, 7.0, 11.0, 13.0, -1.0, 17.0, 19.0, 23.0, 1.0,
            ],
        ),
        default: values(&mut storage.f32_values, &[2.0, 3.0, 5.0, -0.0]),
    };
    attribute(
        &mut storage,
        StoredAttributeSemantic::Tangent,
        AttributeDomain::Corner,
        4,
        6,
        payload,
    );
    let uv = values(
        &mut storage.f32_values,
        &[
            0.0, 10.0, 1.0, 11.0, 2.0, 12.0, 3.0, 13.0, 4.0, 14.0, 5.0, 15.0,
        ],
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Texcoord(0),
        AttributeDomain::Corner,
        2,
        6,
        StoredAttributePayload::DenseF32(uv),
    );
    let color = values(&mut storage.f32_values, &[0.0, -0.0, 1.0, 0.5, 1.0, -0.0]);
    attribute(
        &mut storage,
        StoredAttributeSemantic::Color(0),
        AttributeDomain::Face,
        3,
        2,
        StoredAttributePayload::DenseF32(color),
    );
    let smoothing = values(&mut storage.u32_values, &[u32::MAX, 0]);
    attribute(
        &mut storage,
        StoredAttributeSemantic::SmoothingGroup,
        AttributeDomain::Face,
        1,
        2,
        StoredAttributePayload::DenseU32(smoothing),
    );
    finish_fixture(
        &mut storage,
        TopologyKind::TriangleList,
        [vertices, 6, 2],
        None,
        indexed.then_some(&[0, 1, 2, 2, 1, 3][..]),
    );
    storage
}

fn opaque() -> GeometryStorage {
    let mut storage = empty();
    let positions = values(&mut storage.f32_values, &[0.0; 18]);
    attribute(
        &mut storage,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        6,
        StoredAttributePayload::DenseF32(positions),
    );
    for kind in 0..7 {
        let name = [b't', b'e', b's', b't', b':', b'a' + kind];
        let semantic =
            StoredAttributeSemantic::Custom(TextRange(values(&mut storage.names, &name)));
        let payload = match kind {
            0 => StoredAttributePayload::DenseF32(values(
                &mut storage.f32_values,
                &[-0.0, f32::from_bits(1), f32::MAX, -1.0, 2.0, 3.0],
            )),
            1 => StoredAttributePayload::DenseF64(values(
                &mut storage.f64_values,
                &[f64::from_bits(1), -0.0, f64::MAX, -f64::MAX, 3.0, 4.0],
            )),
            2 => StoredAttributePayload::DenseU32(values(
                &mut storage.u32_values,
                &[0, u32::MAX, 1, 2, 3, 4],
            )),
            3 => StoredAttributePayload::DenseI32(values(
                &mut storage.i32_values,
                &[i32::MIN, -1, 0, 1, i32::MAX, 2],
            )),
            4 => StoredAttributePayload::SparseF32 {
                indices: values(&mut storage.u32_values, &[1, 2, 5]),
                values: values(&mut storage.f32_values, &[7.0, -0.0, 11.0, 13.0, 7.0, -0.0]),
                default: values(&mut storage.f32_values, &[7.0, -0.0]),
            },
            5 => StoredAttributePayload::SparseU32 {
                indices: values(&mut storage.u32_values, &[1, 2, 5]),
                values: values(&mut storage.u32_values, &[0, u32::MAX, 7, 11, 0, u32::MAX]),
                default: values(&mut storage.u32_values, &[0, u32::MAX]),
            },
            _ => StoredAttributePayload::SparseI32 {
                indices: values(&mut storage.u32_values, &[2, 4, 5]),
                values: values(&mut storage.i32_values, &[i32::MIN, -7, i32::MAX]),
                default: values(&mut storage.i32_values, &[-7]),
            },
        };
        attribute(
            &mut storage,
            semantic,
            AttributeDomain::Corner,
            if matches!(kind, 4 | 5) { 2 } else { 1 },
            6,
            payload,
        );
    }
    finish_fixture(
        &mut storage,
        TopologyKind::TriangleList,
        [6, 6, 2],
        None,
        None,
    );
    storage
}

fn topology_fixture(topology: TopologyKind, empty_primitive: bool) -> GeometryStorage {
    let mut storage = empty();
    let corners = if empty_primitive { 0 } else { 4 };
    let positions = values(&mut storage.f32_values, &vec![1.0; corners as usize * 3]);
    attribute(
        &mut storage,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        corners,
        StoredAttributePayload::DenseF32(positions),
    );
    let uv = values(&mut storage.f32_values, &vec![2.0; corners as usize * 2]);
    attribute(
        &mut storage,
        StoredAttributeSemantic::Texcoord(0),
        AttributeDomain::Corner,
        2,
        corners,
        StoredAttributePayload::DenseF32(uv),
    );
    let faces = if empty_primitive {
        0
    } else {
        match topology {
            TopologyKind::Points | TopologyKind::LineLoop => 4,
            TopologyKind::LineList | TopologyKind::TriangleStrip | TopologyKind::TriangleFan => 2,
            TopologyKind::LineStrip => 3,
            TopologyKind::Polygons => 1,
            _ => unreachable!(),
        }
    };
    let ids = [0, 1, 2, 3];
    finish_fixture(
        &mut storage,
        topology,
        [corners, corners, faces],
        (topology == TopologyKind::Polygons).then_some(if empty_primitive { &[] } else { &[4] }),
        Some(&ids[..corners as usize]),
    );
    storage
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}
fn dense_f32(storage: &GeometryStorage, index: usize) -> &[f32] {
    let StoredAttributePayload::DenseF32(range) = storage.attributes[index].payload else {
        panic!("expected DenseF32")
    };
    range.get(&storage.f32_values).unwrap()
}
fn assert_valid(storage: &GeometryStorage) -> u64 {
    let mut control = Control::default();
    assert_eq!(
        storage.validate(
            &mut control,
            [
                storage.logical_meshes.len(),
                storage.primitives.len(),
                storage.attributes.len(),
                storage.mesh_instances.len()
            ],
            3,
            0
        ),
        Ok(true)
    );
    control.completed
}

#[test]
fn indexed_and_nonindexed_reflection_preserve_vertex_identity_and_pair_all_corner_payloads() {
    for indexed in [true, false] {
        let source = triangles(indexed);
        assert_valid(&source);
        let plan =
            GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
        let mut built = Control::default();
        let output = plan.materialize(&mut built).unwrap();
        assert_eq!(built.completed, plan.work());
        assert_eq!(assert_valid(&output), plan.validation_work());
        let mut compared = Control::default();
        assert_eq!(
            plan.validate_correspondence(&output, &mut compared),
            Ok(true)
        );
        assert_eq!(compared.completed, plan.correspondence_work());
        assert_eq!(
            output.primitives[0].vertex_count,
            if indexed { 4 } else { 6 }
        );
        assert_eq!(output.primitives[0].corner_count, 6);
        assert_eq!(output.primitives[0].face_count, 2);
        assert_eq!(
            output.primitives[0].attributes,
            source.primitives[0].attributes
        );
        assert_eq!(
            output.logical_meshes[0].primitives,
            source.logical_meshes[0].primitives
        );
        assert_eq!(output.names, source.names);
        for (source, output) in source.mesh_instances.iter().zip(&output.mesh_instances) {
            assert_eq!(source.owner_node, output.owner_node);
            assert_eq!(source.logical_mesh, output.logical_mesh);
        }
        let ids = output.primitives[0]
            .indices
            .unwrap()
            .get(&output.u32_values)
            .unwrap();
        assert_eq!(
            ids,
            if indexed {
                &[0, 2, 1, 2, 3, 1]
            } else {
                &[0, 2, 1, 3, 5, 4]
            }
        );
        assert_eq!(
            &dense_f32(&output, 0)[..12],
            &[
                -2.0, 4.0, 6.0, -8.0, 10.0, 12.0, -14.0, 16.0, 18.0, -20.0, 22.0, 24.0
            ]
        );
        assert_eq!(&dense_f32(&output, 1)[..3], &[-2.0, 3.0, 5.0]);
        assert_eq!(&dense_f32(&output, 3)[..4], &[-2.0, 3.0, 5.0, -1.0]);
        assert_eq!(
            dense_f32(&output, 5),
            &[
                0.0, 10.0, 2.0, 12.0, 1.0, 11.0, 3.0, 13.0, 5.0, 15.0, 4.0, 14.0
            ]
        );
        assert_eq!(bits(dense_f32(&output, 6)), bits(dense_f32(&source, 6)));
        let StoredAttributePayload::SparseF32 {
            indices,
            values,
            default,
        } = output.attributes[2].payload
        else {
            panic!()
        };
        assert_eq!(indices.get(&output.u32_values).unwrap(), &[1, 2, 5]);
        assert_eq!(
            values.get(&output.f32_values).unwrap(),
            &[-7.0, 11.0, 13.0, -2.0, 3.0, 5.0, -2.0, 3.0, 5.0]
        );
        assert_eq!(default.get(&output.f32_values).unwrap(), &[-2.0, 3.0, 5.0]);
        let StoredAttributePayload::SparseF32 {
            indices,
            values,
            default,
        } = output.attributes[4].payload
        else {
            panic!()
        };
        assert_eq!(indices.get(&output.u32_values).unwrap(), &[1, 2, 4]);
        assert_eq!(
            bits(values.get(&output.f32_values).unwrap()),
            bits(&[
                -7.0, 11.0, 13.0, 1.0, -2.0, 3.0, 5.0, 0.0, -17.0, 19.0, 23.0, -1.0
            ])
        );
        assert_eq!(
            bits(default.get(&output.f32_values).unwrap()),
            bits(&[-2.0, 3.0, 5.0, 0.0])
        );
        let StoredAttributePayload::DenseU32(range) = output.attributes[7].payload else {
            panic!()
        };
        assert_eq!(range.get(&output.u32_values).unwrap(), &[u32::MAX, 0]);
    }
}

#[test]
fn mixed_primitive_packing_places_synthesized_indices_before_later_primitive_streams() {
    let mut source = empty();
    let position = values(&mut source.f32_values, &[1.0; 9]);
    attribute(
        &mut source,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        3,
        StoredAttributePayload::DenseF32(position),
    );
    let semantic = StoredAttributeSemantic::Custom(TextRange(values(&mut source.names, b"test:a")));
    let payload = StoredAttributePayload::SparseU32 {
        indices: values(&mut source.u32_values, &[1, 2]),
        values: values(&mut source.u32_values, &[10, 20]),
        default: values(&mut source.u32_values, &[30]),
    };
    attribute(
        &mut source,
        semantic,
        AttributeDomain::Corner,
        1,
        3,
        payload,
    );
    let position = values(&mut source.f32_values, &[2.0; 9]);
    attribute(
        &mut source,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        3,
        StoredAttributePayload::DenseF32(position),
    );
    let semantic = StoredAttributeSemantic::Custom(TextRange(values(&mut source.names, b"test:b")));
    let payload = StoredAttributePayload::DenseU32(values(&mut source.u32_values, &[7, 11, 13]));
    attribute(
        &mut source,
        semantic,
        AttributeDomain::Corner,
        1,
        3,
        payload,
    );
    let later_indices = values(&mut source.u32_values, &[2, 1, 0]);
    let empty_indices = values(&mut source.u32_values, &[]);
    source.primitives.extend([
        StoredPrimitive {
            topology: TopologyKind::TriangleList,
            vertex_count: 3,
            corner_count: 3,
            face_count: 1,
            polygon_boundaries: None,
            indices: None,
            material: None,
            attributes: TableRange::new(0, 2),
        },
        StoredPrimitive {
            topology: TopologyKind::TriangleList,
            vertex_count: 3,
            corner_count: 3,
            face_count: 1,
            polygon_boundaries: None,
            indices: Some(later_indices),
            material: None,
            attributes: TableRange::new(2, 2),
        },
        StoredPrimitive {
            topology: TopologyKind::Points,
            vertex_count: 0,
            corner_count: 0,
            face_count: 0,
            polygon_boundaries: None,
            indices: Some(empty_indices),
            material: None,
            attributes: TableRange::new(4, 0),
        },
    ]);
    let name = TextRange(values(&mut source.names, b"mixed"));
    source.logical_meshes.push(StoredLogicalMesh {
        primitives: TableRange::new(0, 3),
        name: Some(name),
    });
    assert_valid(&source);
    let plan =
        GeometrySpacePlan::try_new(&source, winding_only(), &mut Control::default()).unwrap();
    let mut built = Control::default();
    let output = plan.materialize(&mut built).unwrap();
    assert_eq!(
        output.u32_values,
        [1, 2, 20, 10, 30, 7, 13, 11, 0, 2, 1, 2, 0, 1]
    );
    assert_eq!(output.primitives[0].indices, Some(TableRange::new(8, 3)));
    assert_eq!(output.primitives[1].indices, Some(TableRange::new(11, 3)));
    assert_eq!(output.primitives[2].indices, Some(TableRange::new(14, 0)));
    for (source, output) in source.primitives.iter().zip(&output.primitives) {
        assert_eq!(source.attributes, output.attributes);
        assert_eq!(source.vertex_count, output.vertex_count);
        assert_eq!(source.face_count, output.face_count);
    }
    assert_eq!(built.completed, plan.work());
    assert_eq!(assert_valid(&output), plan.validation_work());
    let mut compared = Control::default();
    assert_eq!(
        plan.validate_correspondence(&output, &mut compared),
        Ok(true)
    );
    assert_eq!(compared.completed, plan.correspondence_work());
}

#[test]
fn reflection_with_opposite_target_winding_changes_tangent_w_without_corner_reordering() {
    let source = triangles(false);
    let conversion = conversion(Handedness::Left, 1.0, FrontFaceWinding::Clockwise);
    assert!(!conversion.reverse_winding());
    let plan = GeometrySpacePlan::try_new(&source, conversion, &mut Control::default()).unwrap();
    let output = plan.materialize(&mut Control::default()).unwrap();
    assert!(output.primitives[0].indices.is_none());
    assert_eq!(bits(dense_f32(&output, 5)), bits(dense_f32(&source, 5)));
    assert_eq!(&dense_f32(&output, 3)[..4], &[-2.0, 3.0, 5.0, -1.0]);
    assert_valid(&output);
}

#[test]
fn winding_only_converts_all_seven_opaque_shapes_and_retains_explicit_defaults_and_scalar_bits() {
    let source = opaque();
    assert_valid(&source);
    let plan =
        GeometrySpacePlan::try_new(&source, winding_only(), &mut Control::default()).unwrap();
    let mut built = Control::default();
    let output = plan.materialize(&mut built).unwrap();
    assert_eq!(built.completed, plan.work());
    assert_eq!(assert_valid(&output), plan.validation_work());
    assert_eq!(
        plan.validate_correspondence(&output, &mut Control::default()),
        Ok(true)
    );
    assert_eq!(output.names, source.names);
    assert_eq!(
        bits(dense_f32(&output, 1)),
        bits(&[-0.0, f32::MAX, f32::from_bits(1), -1.0, 3.0, 2.0])
    );
    let StoredAttributePayload::DenseF64(range) = output.attributes[2].payload else {
        panic!()
    };
    assert_eq!(
        range
            .get(&output.f64_values)
            .unwrap()
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        [f64::from_bits(1), f64::MAX, -0.0, -f64::MAX, 4.0, 3.0].map(f64::to_bits)
    );
    let StoredAttributePayload::DenseU32(range) = output.attributes[3].payload else {
        panic!()
    };
    assert_eq!(
        range.get(&output.u32_values).unwrap(),
        &[0, 1, u32::MAX, 2, 4, 3]
    );
    let StoredAttributePayload::DenseI32(range) = output.attributes[4].payload else {
        panic!()
    };
    assert_eq!(
        range.get(&output.i32_values).unwrap(),
        &[i32::MIN, 0, -1, 1, 2, i32::MAX]
    );
    let StoredAttributePayload::SparseF32 {
        indices,
        values,
        default,
    } = output.attributes[5].payload
    else {
        panic!()
    };
    assert_eq!(indices.get(&output.u32_values).unwrap(), &[1, 2, 4]);
    assert_eq!(
        bits(values.get(&output.f32_values).unwrap()),
        bits(&[11.0, 13.0, 7.0, -0.0, 7.0, -0.0])
    );
    assert_eq!(
        bits(default.get(&output.f32_values).unwrap()),
        bits(&[7.0, -0.0])
    );
    let StoredAttributePayload::SparseU32 {
        indices,
        values,
        default,
    } = output.attributes[6].payload
    else {
        panic!()
    };
    assert_eq!(indices.get(&output.u32_values).unwrap(), &[1, 2, 4]);
    assert_eq!(
        values.get(&output.u32_values).unwrap(),
        &[7, 11, 0, u32::MAX, 0, u32::MAX]
    );
    assert_eq!(default.get(&output.u32_values).unwrap(), &[0, u32::MAX]);
    let StoredAttributePayload::SparseI32 {
        indices,
        values,
        default,
    } = output.attributes[7].payload
    else {
        panic!()
    };
    assert_eq!(indices.get(&output.u32_values).unwrap(), &[1, 4, 5]);
    assert_eq!(
        values.get(&output.i32_values).unwrap(),
        &[i32::MIN, i32::MAX, -7]
    );
    assert_eq!(default.get(&output.i32_values).unwrap(), &[-7]);
    let last_attribute_end = match output.attributes[6].payload {
        StoredAttributePayload::SparseU32 { default, .. } => default.start() + default.len(),
        _ => unreachable!(),
    };
    let last_sparse_ids = match output.attributes[7].payload {
        StoredAttributePayload::SparseI32 { indices, .. } => indices,
        _ => unreachable!(),
    };
    assert_eq!(last_sparse_ids.start(), last_attribute_end);
    assert_eq!(
        output.primitives[0].indices.unwrap().start(),
        last_sparse_ids.start() + last_sparse_ids.len()
    );
}

#[test]
fn no_flip_preserves_strip_fan_polygon_and_linear_topologies_with_their_payloads() {
    for topology in [
        TopologyKind::TriangleStrip,
        TopologyKind::TriangleFan,
        TopologyKind::Polygons,
        TopologyKind::Points,
        TopologyKind::LineList,
        TopologyKind::LineStrip,
        TopologyKind::LineLoop,
    ] {
        let source = topology_fixture(topology, false);
        assert_valid(&source);
        // Reflection and winding changes cancel, while points and directions still change.
        let conversion = conversion(Handedness::Left, 0.5, FrontFaceWinding::Clockwise);
        let plan =
            GeometrySpacePlan::try_new(&source, conversion, &mut Control::default()).unwrap();
        let output = plan.materialize(&mut Control::default()).unwrap();
        assert_eq!(output.primitives[0].topology, topology);
        assert_eq!(
            output.primitives[0].polygon_boundaries,
            source.primitives[0].polygon_boundaries
        );
        assert_eq!(output.primitives[0].indices, source.primitives[0].indices);
        assert_eq!(output.u32_values, source.u32_values);
        assert_eq!(bits(dense_f32(&output, 1)), bits(dense_f32(&source, 1)));
        assert_eq!(assert_valid(&output), plan.validation_work());
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(true)
        );
    }
}

#[test]
fn reverse_winding_rejects_nonempty_unsupported_topologies_but_accepts_empty_and_linear_rows() {
    for topology in [
        TopologyKind::TriangleStrip,
        TopologyKind::TriangleFan,
        TopologyKind::Polygons,
    ] {
        let source = topology_fixture(topology, false);
        assert!(
            matches!(GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()),
            Err(GeometrySpacePlanError::UnsupportedTopology { primitive: 0, topology: actual }) if actual == topology)
        );
        let source = topology_fixture(topology, true);
        assert_valid(&source);
        let plan =
            GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
        let output = plan.materialize(&mut Control::default()).unwrap();
        assert_valid(&output);
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(true)
        );
    }
    for topology in [
        TopologyKind::Points,
        TopologyKind::LineList,
        TopologyKind::LineStrip,
        TopologyKind::LineLoop,
    ] {
        let source = topology_fixture(topology, false);
        let plan =
            GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
        let output = plan.materialize(&mut Control::default()).unwrap();
        assert_eq!(output.u32_values, source.u32_values);
        assert_eq!(bits(dense_f32(&output, 1)), bits(dense_f32(&source, 1)));
        assert_valid(&output);
    }
}

#[test]
fn planning_rejects_custom_spatial_semantics_and_actual_dense_or_sparse_numeric_errors() {
    let source = opaque();
    assert!(matches!(
        GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()),
        Err(GeometrySpacePlanError::CustomAttribute { attribute: 1 })
    ));
    let mut source = triangles(true);
    source.f32_values[0] = f32::MAX;
    assert!(matches!(
        GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()),
        Err(GeometrySpacePlanError::Numeric {
            attribute: 0,
            source: NumericConversionError::NonFiniteOutput
        })
    ));
    source.f32_values[0] = f32::from_bits(1);
    let half = conversion(Handedness::Right, 2.0, FrontFaceWinding::CounterClockwise);
    assert!(matches!(
        GeometrySpacePlan::try_new(&source, half, &mut Control::default()),
        Err(GeometrySpacePlanError::Numeric {
            attribute: 0,
            source: NumericConversionError::UnderflowToZero
        })
    ));
    let mut source = empty();
    let payload = StoredAttributePayload::SparseF32 {
        indices: values(&mut source.u32_values, &[]),
        values: values(&mut source.f32_values, &[]),
        default: values(&mut source.f32_values, &[f32::MAX, 0.0, 0.0]),
    };
    attribute(
        &mut source,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        3,
        payload,
    );
    finish_fixture(
        &mut source,
        TopologyKind::TriangleList,
        [3, 3, 1],
        None,
        None,
    );
    assert_valid(&source);
    assert!(matches!(
        GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()),
        Err(GeometrySpacePlanError::Numeric {
            attribute: 0,
            source: NumericConversionError::NonFiniteOutput
        })
    ));
}

#[test]
fn planning_and_central_comparison_allocate_nothing_and_finish_at_exact_work() {
    let source = triangles(false);
    let mut planned = None;
    let mut preflight = Control::default();
    let allocations = allocation_counter::measure(|| {
        planned = Some(GeometrySpacePlan::try_new(&source, reflected(), &mut preflight).unwrap());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    let plan = planned.unwrap();
    assert_eq!(plan.name_bytes(), source.names.len());
    assert_eq!(
        plan.capacities()[7].desired_capacity(),
        source.u32_values.len() + 6
    );
    let mut built = Control::default();
    let output = plan.materialize(&mut built).unwrap();
    assert_eq!(built.completed, plan.work());
    let mut compared = Control::default();
    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            plan.validate_correspondence(&output, &mut compared),
            Ok(true)
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(compared.completed, plan.correspondence_work());
    for stop in [0, preflight.completed / 2, preflight.completed - 1] {
        let mut control = Control {
            reject_before: Some(stop),
            ..Control::default()
        };
        assert!(
            matches!(GeometrySpacePlan::try_new(&source, reflected(), &mut control),
            Err(GeometrySpacePlanError::Control(actual)) if actual == stop)
        );
        assert_eq!(control.completed, stop);
    }
    for stop in [0, plan.work() / 2, plan.work() - 1] {
        let mut control = Control {
            reject_before: Some(stop),
            ..Control::default()
        };
        assert!(
            matches!(plan.materialize(&mut control), Err(TypedStorageBuildError::Control(actual)) if actual == stop)
        );
        assert_eq!(control.completed, stop);
        let mut control = Control {
            reject_before: Some(stop),
            ..Control::default()
        };
        assert_eq!(
            plan.validate_correspondence(&output, &mut control),
            Err(stop)
        );
        let mut control = Control {
            omit_before: Some(stop),
            ..Control::default()
        };
        assert_eq!(
            plan.validate_correspondence(&output, &mut control),
            Ok(false)
        );
        assert_eq!(control.completed, stop);
    }
}

#[test]
fn central_correspondence_rejects_changed_bits_ranges_owner_fields_and_foreign_source_results() {
    let source = triangles(true);
    let plan = GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    for fault in 0..10 {
        let mut output = plan.materialize(&mut Control::default()).unwrap();
        match fault {
            0 => output.f32_values[0] = -output.f32_values[0],
            1 => output.primitives[0].vertex_count += 1,
            2 => output.primitives[0].face_count += 1,
            3 => output.primitives[0].attributes = TableRange::new(1, source.attributes.len() - 1),
            4 => output.mesh_instances[0].owner_node = StoredNodeIndex(2),
            5 => output.logical_meshes[0].primitives = TableRange::new(0, 0),
            6 => output.names[0] ^= 1,
            7 => {
                let StoredAttributePayload::DenseF32(range) = output.attributes[0].payload else {
                    panic!()
                };
                output.attributes[0].payload = StoredAttributePayload::DenseF32(TableRange::new(
                    range.start() + 1,
                    range.len(),
                ));
            }
            8 => {
                let StoredAttributePayload::SparseF32 { default, .. } =
                    output.attributes[4].payload
                else {
                    panic!()
                };
                let w = default.start() + 3;
                output.f32_values[w] = -output.f32_values[w];
            }
            _ => output.u32_values[0] += 1,
        }
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(false),
            "fault {fault}"
        );
    }
    let mut foreign = triangles(true);
    foreign.f32_values[0] = 23.0;
    let other = GeometrySpacePlan::try_new(&foreign, reflected(), &mut Control::default()).unwrap();
    let foreign_output = other.materialize(&mut Control::default()).unwrap();
    assert_valid(&foreign_output);
    assert_eq!(
        plan.validate_correspondence(&foreign_output, &mut Control::default()),
        Ok(false)
    );
}

#[test]
fn sparse_vertex_positions_scale_explicit_values_and_default_without_remapping_vertex_ids() {
    let mut source = empty();
    let payload = StoredAttributePayload::SparseF32 {
        indices: values(&mut source.u32_values, &[1]),
        values: values(&mut source.f32_values, &[5.0, 7.0, 11.0]),
        default: values(&mut source.f32_values, &[2.0, 3.0, 5.0]),
    };
    attribute(
        &mut source,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        3,
        payload,
    );
    finish_fixture(
        &mut source,
        TopologyKind::TriangleList,
        [3, 3, 1],
        None,
        None,
    );
    assert_valid(&source);
    let plan = GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    let mut built = Control::default();
    let output = plan.materialize(&mut built).unwrap();
    let StoredAttributePayload::SparseF32 {
        indices,
        values,
        default,
    } = output.attributes[0].payload
    else {
        panic!()
    };
    assert_eq!(indices.get(&output.u32_values).unwrap(), &[1]);
    assert_eq!(
        values.get(&output.f32_values).unwrap(),
        &[-10.0, 14.0, 22.0]
    );
    assert_eq!(default.get(&output.f32_values).unwrap(), &[-4.0, 6.0, 10.0]);
    assert_eq!(
        output.primitives[0]
            .indices
            .unwrap()
            .get(&output.u32_values)
            .unwrap(),
        &[0, 2, 1]
    );
    assert_eq!(built.completed, plan.work());
    assert_eq!(assert_valid(&output), plan.validation_work());
    let mut compared = Control::default();
    assert_eq!(
        plan.validate_correspondence(&output, &mut compared),
        Ok(true)
    );
    assert_eq!(compared.completed, plan.correspondence_work());
}

#[test]
fn empty_geometry_and_zero_explicit_sparse_payloads_keep_presence_and_exact_work() {
    let source = empty();
    let plan = GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    let mut built = Control::default();
    let output = plan.materialize(&mut built).unwrap();
    assert_eq!(plan.work(), 0);
    assert_eq!(plan.validation_work(), 0);
    assert_eq!(built.completed, 0);
    assert_eq!(
        plan.validate_correspondence(&output, &mut Control::default()),
        Ok(true)
    );
    let mut source = empty();
    let payload = StoredAttributePayload::SparseF32 {
        indices: values(&mut source.u32_values, &[]),
        values: values(&mut source.f32_values, &[]),
        default: values(&mut source.f32_values, &[2.0, 3.0, 5.0]),
    };
    attribute(
        &mut source,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        3,
        payload,
    );
    finish_fixture(
        &mut source,
        TopologyKind::TriangleList,
        [3, 3, 1],
        None,
        None,
    );
    let plan = GeometrySpacePlan::try_new(&source, reflected(), &mut Control::default()).unwrap();
    let output = plan.materialize(&mut Control::default()).unwrap();
    let StoredAttributePayload::SparseF32 {
        indices,
        values,
        default,
    } = output.attributes[0].payload
    else {
        panic!()
    };
    assert!(indices.is_empty());
    assert!(values.is_empty());
    assert_eq!(default.get(&output.f32_values).unwrap(), &[-4.0, 6.0, 10.0]);
    assert_eq!(assert_valid(&output), plan.validation_work());
    assert_eq!(
        plan.validate_correspondence(&output, &mut Control::default()),
        Ok(true)
    );
}
