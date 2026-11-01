use super::*;
use crate::canonical_scene::store::typed_storage::{
    StoredLogicalMeshIndex, StoredNodeIndex, TextRange,
};

const NORMALS: [[f32; 3]; 6] = [
    [-0.0, 0.0, 1.0],
    [0.0, -0.0, 1.0],
    [0.0, 0.0, 1.0],
    [0.0, 1.0, 0.0],
    [0.0, 1.0, -0.0],
    [-0.0, 1.0, 0.0],
];

#[derive(Default)]
struct Control {
    completed: u64,
    strings: u64,
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

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.step(action)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        let result = self.step(action)?;
        self.strings += 1;
        Ok(result)
    }
}

impl TypedStorageValidationControl for Control {
    type Error = u64;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
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

fn name(storage: &mut GeometryStorage, text: &str) -> TextRange {
    TextRange(values(&mut storage.names, text.as_bytes()))
}

fn attribute(
    storage: &mut GeometryStorage,
    semantic: StoredAttributeSemantic,
    domain: AttributeDomain,
    lanes: u8,
    element_count: u32,
    payload: StoredAttributePayload,
) -> u32 {
    let index = storage.attributes.len() as u32;
    storage.attributes.push(StoredAttribute {
        tangent_conventions: if semantic == StoredAttributeSemantic::Tangent {
            Some(crate::canonical_scene::store::typed_storage::StoredTangentConventions::UNKNOWN)
        } else {
            None
        },
        semantic,
        domain,
        lanes,
        element_count,
        payload,
    });
    index
}

/// The middle indexed primitive is the target. All seven payload shapes occur; optional Normal
/// and Tangent are sparse, so deletion must compact both their F32 and U32 scalar ranges.
fn geometry(normal: bool, tangent: bool) -> GeometryStorage {
    let mut storage = GeometryStorage {
        logical_meshes: Vec::new(),
        primitives: Vec::new(),
        attributes: Vec::new(),
        mesh_instances: vec![
            StoredMeshInstance {
                owner_node: StoredNodeIndex(1),
                logical_mesh: StoredLogicalMeshIndex(0),
            },
            StoredMeshInstance {
                owner_node: StoredNodeIndex(2),
                logical_mesh: StoredLogicalMeshIndex(0),
            },
        ],
        names: Vec::new(),
        f32_values: Vec::new(),
        f64_values: Vec::new(),
        u32_values: Vec::new(),
        i32_values: Vec::new(),
    };
    let left_position = values(
        &mut storage.f32_values,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        3,
        StoredAttributePayload::DenseF32(left_position),
    );
    let target_start = storage.attributes.len();
    let position = values(
        &mut storage.f32_values,
        &[0.0, -0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0],
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        4,
        StoredAttributePayload::DenseF32(position),
    );
    if normal {
        let payload = StoredAttributePayload::SparseF32 {
            indices: values(&mut storage.u32_values, &[2]),
            values: values(&mut storage.f32_values, &[0.0, -0.0, 1.0]),
            default: values(&mut storage.f32_values, &[-0.0, 0.0, 1.0]),
        };
        attribute(
            &mut storage,
            StoredAttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            4,
            payload,
        );
    }
    if tangent {
        let payload = StoredAttributePayload::SparseF32 {
            indices: values(&mut storage.u32_values, &[1, 5]),
            values: values(
                &mut storage.f32_values,
                &[1.0, -0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0],
            ),
            default: values(&mut storage.f32_values, &[1.0, 0.0, -0.0, 1.0]),
        };
        attribute(
            &mut storage,
            StoredAttributeSemantic::Tangent,
            AttributeDomain::Corner,
            4,
            6,
            payload,
        );
    }
    let uv = values(
        &mut storage.f32_values,
        &[0.0, -0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0],
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Texcoord(0),
        AttributeDomain::Corner,
        2,
        6,
        StoredAttributePayload::DenseF32(uv),
    );
    let color = StoredAttributePayload::SparseF32 {
        indices: values(&mut storage.u32_values, &[1]),
        values: values(&mut storage.f32_values, &[-0.0, 0.5, 1.0]),
        default: values(&mut storage.f32_values, &[-0.0, 0.5, 1.0]),
    };
    attribute(
        &mut storage,
        StoredAttributeSemantic::Color(0),
        AttributeDomain::Face,
        3,
        2,
        color,
    );
    let groups = StoredAttributePayload::SparseU32 {
        indices: values(&mut storage.u32_values, &[1]),
        values: values(&mut storage.u32_values, &[9]),
        default: values(&mut storage.u32_values, &[0]),
    };
    attribute(
        &mut storage,
        StoredAttributeSemantic::SmoothingGroup,
        AttributeDomain::Face,
        1,
        2,
        groups,
    );
    let wide = name(&mut storage, "edit:a_dense_f64");
    let wide_values = values(
        &mut storage.f64_values,
        &[
            1.0e250,
            -0.0,
            -1.0e250,
            0.0,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            1.0,
            -1.0,
            2.0,
            -2.0,
            3.0,
            -3.0,
        ],
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Custom(wide),
        AttributeDomain::Corner,
        2,
        6,
        StoredAttributePayload::DenseF64(wide_values),
    );
    let sparse = name(&mut storage, "edit:b_sparse_i32");
    let sparse_values = StoredAttributePayload::SparseI32 {
        indices: values(&mut storage.u32_values, &[0, 4]),
        values: values(&mut storage.i32_values, &[0, -7, 99, -12]),
        default: values(&mut storage.i32_values, &[0, -7]),
    };
    attribute(
        &mut storage,
        StoredAttributeSemantic::Custom(sparse),
        AttributeDomain::Corner,
        2,
        6,
        sparse_values,
    );
    let unsigned = name(&mut storage, "edit:c_dense_u32");
    let unsigned_values = values(&mut storage.u32_values, &[70, 71]);
    attribute(
        &mut storage,
        StoredAttributeSemantic::Custom(unsigned),
        AttributeDomain::Face,
        1,
        2,
        StoredAttributePayload::DenseU32(unsigned_values),
    );
    let signed = name(&mut storage, "edit:d_dense_i32");
    let signed_values = values(&mut storage.i32_values, &[-5, 0, 7, i32::MIN]);
    attribute(
        &mut storage,
        StoredAttributeSemantic::Custom(signed),
        AttributeDomain::Vertex,
        1,
        4,
        StoredAttributePayload::DenseI32(signed_values),
    );
    let target_end = storage.attributes.len();
    let right_position = values(
        &mut storage.f32_values,
        &[2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 3.0, 1.0, 0.0, 2.0, 1.0, 0.0],
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        4,
        StoredAttributePayload::DenseF32(right_position),
    );
    let right_normal = values(
        &mut storage.f32_values,
        &[
            0.0, 0.0, -1.0, 0.0, -0.0, -1.0, 0.0, 0.0, -1.0, -0.0, 0.0, -1.0,
        ],
    );
    attribute(
        &mut storage,
        StoredAttributeSemantic::Normal,
        AttributeDomain::Corner,
        3,
        4,
        StoredAttributePayload::DenseF32(right_normal),
    );
    let indices = values(&mut storage.u32_values, &[0, 1, 2, 0, 2, 3]);
    let boundaries = values(&mut storage.u32_values, &[4]);
    let right_indices = values(&mut storage.u32_values, &[3, 2, 1, 0]);
    storage.primitives = vec![
        StoredPrimitive {
            topology: TopologyKind::TriangleList,
            vertex_count: 3,
            corner_count: 3,
            face_count: 1,
            polygon_boundaries: None,
            indices: None,
            material: crate::MaterialAssignment::Authored(0),
            attributes: TableRange::new(0, target_start),
        },
        StoredPrimitive {
            topology: TopologyKind::TriangleList,
            vertex_count: 4,
            corner_count: 6,
            face_count: 2,
            polygon_boundaries: None,
            indices: Some(indices),
            material: crate::MaterialAssignment::Authored(1),
            attributes: TableRange::new(target_start, target_end - target_start),
        },
        StoredPrimitive {
            topology: TopologyKind::Polygons,
            vertex_count: 4,
            corner_count: 4,
            face_count: 1,
            polygon_boundaries: Some(boundaries),
            indices: Some(right_indices),
            material: crate::MaterialAssignment::Unspecified,
            attributes: TableRange::new(target_end, storage.attributes.len() - target_end),
        },
    ];
    let mesh_name = name(&mut storage, "shared mesh");
    storage.logical_meshes.push(StoredLogicalMesh {
        primitives: TableRange::new(0, 3),
        name: Some(mesh_name),
    });
    storage
}

fn plan(source: &GeometryStorage) -> GeometryNormalsPlan<'_> {
    GeometryNormalsPlan::try_new(
        source,
        1,
        VectorValues::F32(&NORMALS),
        NormalWritePolicy::Replace,
        TangentPolicy::Remove,
        &mut Control::default(),
    )
    .expect("valid replacement")
}

#[test]
fn removing_tangents_removes_only_their_schema_text_and_rebases_surviving_name_ranges() {
    use crate::canonical_scene::store::typed_storage::{
        StoredTangentBasis, StoredTangentConventions, StoredTangentEncoding,
    };
    let mut source = geometry(true, true);
    let original_names = source.names.clone();
    let basis = "test:removed-basis";
    let encoding = "test:removed-sign";
    let mut names = [basis.as_bytes(), encoding.as_bytes()].concat();
    let removed_bytes = names.len();
    names.extend_from_slice(&source.names);
    source.names = names;
    for attribute in &mut source.attributes {
        if let StoredAttributeSemantic::Custom(name) = &mut attribute.semantic {
            name.0 = TableRange::new(name.0.start() + removed_bytes, name.0.len());
        }
    }
    for mesh in &mut source.logical_meshes {
        if let Some(name) = &mut mesh.name {
            name.0 = TableRange::new(name.0.start() + removed_bytes, name.0.len());
        }
    }
    let tangent = source
        .attributes
        .iter_mut()
        .find(|attribute| attribute.semantic == StoredAttributeSemantic::Tangent)
        .unwrap();
    tangent.tangent_conventions = Some(StoredTangentConventions {
        basis: StoredTangentBasis::Namespaced {
            name: TextRange(TableRange::new(0, basis.len())),
            version: std::num::NonZeroU32::new(2).unwrap(),
        },
        encoding: StoredTangentEncoding::Namespaced {
            name: TextRange(TableRange::new(basis.len(), encoding.len())),
            version: std::num::NonZeroU32::new(3).unwrap(),
        },
    });
    assert_eq!(
        source.validate(
            &mut Control::default(),
            [1, 3, source.attributes.len(), 2],
            3,
            2
        ),
        Ok(true)
    );
    let plan = plan(&source);
    assert_eq!(plan.name_bytes(), original_names.len());
    let mut build = Control::default();
    let output = plan.materialize(&mut build).unwrap();
    assert_eq!(build.completed, plan.work());
    assert_eq!(build.strings, original_names.len() as u64);
    assert_eq!(output.names, original_names);
    assert!(
        output
            .attributes
            .iter()
            .all(|attribute| attribute.tangent_conventions.is_none())
    );
    assert_eq!(
        output.validate(
            &mut Control::default(),
            [1, 3, output.attributes.len(), 2],
            3,
            2
        ),
        Ok(true)
    );
    let mut correspondence = Control::default();
    assert_eq!(
        plan.validate_correspondence(&output, &mut correspondence),
        Ok(true)
    );
    assert_eq!(correspondence.completed, plan.correspondence_work());
}

#[test]
fn removing_target_tangent_preserves_other_primitive_tangent_conventions_and_names() {
    use crate::canonical_scene::store::typed_storage::{
        StoredTangentBasis, StoredTangentConventions, StoredTangentEncoding,
    };
    use std::num::NonZeroU32;

    let mut source = geometry(true, true);
    let original_names = std::mem::take(&mut source.names);
    let custom_name_bytes = source.logical_meshes[0].name.unwrap().0.start();
    let removed_basis = "test:removed-basis";
    let retained_basis = "test:retained-basis";
    let shared_encoding = "test:shared-sign";
    let removed = StoredTangentConventions {
        basis: StoredTangentBasis::Namespaced {
            name: name(&mut source, removed_basis),
            version: NonZeroU32::new(2).unwrap(),
        },
        encoding: StoredTangentEncoding::Namespaced {
            name: name(&mut source, shared_encoding),
            version: NonZeroU32::new(3).unwrap(),
        },
    };
    let removed_name_bytes = source.names.len();
    values(&mut source.names, &original_names[..custom_name_bytes]);
    let retained = StoredTangentConventions {
        basis: StoredTangentBasis::Namespaced {
            name: name(&mut source, retained_basis),
            version: NonZeroU32::new(5).unwrap(),
        },
        encoding: StoredTangentEncoding::Namespaced {
            name: name(&mut source, shared_encoding),
            version: NonZeroU32::new(7).unwrap(),
        },
    };
    let retained_name_bytes = retained_basis.len() + shared_encoding.len();
    values(&mut source.names, &original_names[custom_name_bytes..]);
    for attribute in &mut source.attributes {
        if let StoredAttributeSemantic::Custom(name) = &mut attribute.semantic {
            name.0 = TableRange::new(name.0.start() + removed_name_bytes, name.0.len());
        }
    }
    for mesh in &mut source.logical_meshes {
        if let Some(name) = &mut mesh.name {
            name.0 = TableRange::new(
                name.0.start() + removed_name_bytes + retained_name_bytes,
                name.0.len(),
            );
        }
    }
    let removed_attribute = source.primitives[1]
        .attributes
        .checked(source.attributes.len())
        .unwrap()
        .find(|&index| source.attributes[index].semantic == StoredAttributeSemantic::Tangent)
        .unwrap();
    source.attributes[removed_attribute].tangent_conventions = Some(removed);
    let retained_values = [
        1.0, -0.0, 0.0, -1.0, 0.0, 1.0, -0.0, 1.0, -0.0, 0.0, 1.0, -1.0, 1.0, 0.0, 0.0, 1.0,
    ];
    let payload = values(&mut source.f32_values, &retained_values);
    let retained_attribute = attribute(
        &mut source,
        StoredAttributeSemantic::Tangent,
        AttributeDomain::Corner,
        4,
        4,
        StoredAttributePayload::DenseF32(payload),
    );
    source.attributes[retained_attribute as usize].tangent_conventions = Some(retained);
    let right_attributes = source.primitives[2].attributes;
    source.primitives[2].attributes =
        TableRange::new(right_attributes.start(), right_attributes.len() + 1);
    assert_eq!(
        source.validate(
            &mut Control::default(),
            [1, 3, source.attributes.len(), 2],
            3,
            2
        ),
        Ok(true)
    );

    let mut expected_names = original_names[..custom_name_bytes].to_vec();
    expected_names.extend_from_slice(retained_basis.as_bytes());
    expected_names.extend_from_slice(shared_encoding.as_bytes());
    expected_names.extend_from_slice(&original_names[custom_name_bytes..]);
    let plan = plan(&source);
    assert_eq!(plan.output_attribute(removed_attribute as u32), None);
    let output_attribute = plan.output_attribute(retained_attribute).unwrap();
    assert_eq!(output_attribute, retained_attribute - 1);
    assert_eq!(
        plan.source_attribute(output_attribute),
        Some(retained_attribute)
    );
    assert_eq!(plan.name_bytes(), source.names.len() - removed_name_bytes);
    let mut build = Control::default();
    let output = plan.materialize(&mut build).unwrap();
    assert_eq!(build.completed, plan.work());
    assert_eq!(build.strings, expected_names.len() as u64);
    assert_eq!(output.names, expected_names);
    assert!(
        output.primitives[1]
            .attributes
            .get(&output.attributes)
            .unwrap()
            .iter()
            .all(
                |attribute| attribute.semantic != StoredAttributeSemantic::Tangent
                    && attribute.tangent_conventions.is_none()
            )
    );
    assert_eq!(
        output.primitives[2].attributes.len(),
        right_attributes.len() + 1
    );
    assert!(
        output.primitives[2]
            .attributes
            .checked(output.attributes.len())
            .unwrap()
            .contains(&(output_attribute as usize))
    );
    let row = output.attributes[output_attribute as usize];
    assert_eq!(row.semantic, StoredAttributeSemantic::Tangent);
    assert_eq!(row.domain, AttributeDomain::Corner);
    assert_eq!(row.lanes, 4);
    assert_eq!(row.element_count, 4);
    let Some(StoredTangentConventions {
        basis:
            StoredTangentBasis::Namespaced {
                name: basis,
                version: basis_version,
            },
        encoding:
            StoredTangentEncoding::Namespaced {
                name: encoding,
                version: encoding_version,
            },
    }) = row.tangent_conventions
    else {
        panic!("the other primitive retains both schema declarations");
    };
    assert_eq!(basis.get(&output.names), Some(retained_basis));
    assert_eq!(basis_version.get(), 5);
    assert_eq!(basis.0.start(), custom_name_bytes);
    assert_eq!(encoding.get(&output.names), Some(shared_encoding));
    assert_eq!(encoding_version.get(), 7);
    assert_eq!(encoding.0.start(), custom_name_bytes + retained_basis.len());
    let StoredAttributePayload::DenseF32(payload) = row.payload else {
        panic!("the other primitive retains its tangent payload encoding");
    };
    assert_eq!(
        payload
            .get(&output.f32_values)
            .unwrap()
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        retained_values.map(f32::to_bits)
    );
    let mut validation = Control::default();
    assert_eq!(
        output.validate(&mut validation, [1, 3, output.attributes.len(), 2], 3, 2),
        Ok(true)
    );
    assert_eq!(validation.completed, plan.validation_work());
    let mut correspondence = Control::default();
    assert_eq!(
        plan.validate_correspondence(&output, &mut correspondence),
        Ok(true)
    );
    assert_eq!(correspondence.completed, plan.correspondence_work());
}

#[test]
fn replace_and_add_keep_canonical_order_preserve_payloads_and_compact_removed_ranges() {
    for normal in [false, true] {
        for tangent in [false, true] {
            let source = geometry(normal, tangent);
            let rows = [1, 3, source.attributes.len(), 2];
            assert_eq!(
                source.validate(&mut Control::default(), rows, 3, 2),
                Ok(true)
            );
            let write = if normal {
                NormalWritePolicy::Replace
            } else {
                NormalWritePolicy::Add
            };
            let plan = GeometryNormalsPlan::try_new(
                &source,
                1,
                VectorValues::F32(&NORMALS),
                write,
                TangentPolicy::Remove,
                &mut Control::default(),
            )
            .unwrap();
            let mut build = Control::default();
            let output = plan.materialize(&mut build).unwrap();
            assert_eq!(build.completed, plan.work());
            assert_eq!(build.strings, plan.name_bytes() as u64);
            assert_eq!(plan.target_row(), 1);
            assert_eq!(plan.output_face_count(), 2);
            assert_eq!(plan.output_corner_count(), 6);
            assert_eq!(
                plan.source_attribute_count(),
                source.attributes.len() as u32
            );
            assert_eq!(
                plan.output_attribute_count(),
                output.attributes.len() as u32
            );
            assert_eq!(output.validation_work(), Some(plan.validation_work()));
            let mut validation = Control::default();
            assert_eq!(
                output.validate(&mut validation, [1, 3, output.attributes.len(), 2], 3, 2),
                Ok(true)
            );
            assert_eq!(validation.completed, plan.validation_work());
            let mut correspondence = Control::default();
            assert_eq!(
                plan.validate_correspondence(&output, &mut correspondence),
                Ok(true)
            );
            assert_eq!(correspondence.completed, plan.correspondence_work());
            assert_eq!(output.names, source.names);
            assert!(same_range(
                source.primitives[0].attributes,
                output.primitives[0].attributes
            ));
            assert_eq!(
                output.primitives[1].attributes.start(),
                source.primitives[1].attributes.start()
            );
            assert_eq!(
                output.primitives[2].attributes.start(),
                source.primitives[2].attributes.start() + usize::from(!normal)
                    - usize::from(tangent)
            );
            assert_eq!(output.primitives[2].topology, TopologyKind::Polygons);
            let target = output.primitives[1]
                .attributes
                .get(&output.attributes)
                .unwrap();
            assert_eq!(
                target
                    .iter()
                    .filter(|row| row.semantic == StoredAttributeSemantic::Normal)
                    .count(),
                1
            );
            assert!(
                !target
                    .iter()
                    .any(|row| row.semantic == StoredAttributeSemantic::Tangent)
            );
            assert_eq!(target[0].semantic, StoredAttributeSemantic::Position);
            assert_eq!(target[1].semantic, StoredAttributeSemantic::Normal);
            assert_eq!(target[2].semantic, StoredAttributeSemantic::Texcoord(0));
            assert_eq!(plan.normal_attribute(), 2);
            assert_eq!(
                plan.source_attribute(2),
                if normal { Some(2) } else { None }
            );
            if tangent {
                let removed = if normal { 3 } else { 2 };
                assert_eq!(plan.output_attribute(removed), None);
            }
            let StoredAttributePayload::DenseF32(range) =
                output.attributes[plan.normal_attribute() as usize].payload
            else {
                panic!("the generated normal must be dense F32");
            };
            let actual: Vec<_> = range
                .get(&output.f32_values)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect();
            let expected: Vec<_> = NORMALS.as_flattened().iter().map(|v| v.to_bits()).collect();
            assert_eq!(actual, expected);
            let expected_f32 =
                source.f32_values.len() - usize::from(normal) * 6 - usize::from(tangent) * 12
                    + NORMALS.len() * 3;
            let expected_u32 =
                source.u32_values.len() - usize::from(normal) - usize::from(tangent) * 2;
            assert_eq!(output.f32_values.len(), expected_f32);
            assert_eq!(output.u32_values.len(), expected_u32);
            assert_eq!(output.i32_values, source.i32_values);
            assert_eq!(
                output
                    .f64_values
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                source
                    .f64_values
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            );
            for (index, row) in source.attributes.iter().enumerate() {
                if plan.source_normal() == Some(index as u32)
                    || plan.source_tangent() == Some(index as u32)
                {
                    continue;
                }
                let actual =
                    &output.attributes[plan.output_attribute(index as u32).unwrap() as usize];
                assert_eq!(
                    payload_signature(row.payload, &source),
                    payload_signature(actual.payload, &output)
                );
            }
        }
    }
}

#[test]
fn generated_f64_normals_use_the_f64_pool_and_preserve_candidate_bits() {
    let normals = [
        [1.0, f64::from_bits(1), -0.0],
        [f64::from_bits(0x3fef_ffff_ffff_ffff), -0.0, 0.0],
        [0.0, -0.0, 1.0],
        [0.0, 1.0, 0.0],
        [-0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ];
    for (had_normal, had_tangent) in [(false, false), (true, true)] {
        let source = geometry(had_normal, had_tangent);
        let candidate = VectorValues::F64(&normals);
        assert!(candidate.is_f64());
        assert!(!candidate.is_empty());
        let plan = GeometryNormalsPlan::try_new(
            &source,
            1,
            candidate,
            if had_normal {
                NormalWritePolicy::Replace
            } else {
                NormalWritePolicy::Add
            },
            TangentPolicy::Remove,
            &mut Control::default(),
        )
        .unwrap();
        let expected_f32 =
            source.f32_values.len() - usize::from(had_normal) * 6 - usize::from(had_tangent) * 12;
        assert_eq!(plan.capacities()[5].desired_capacity(), expected_f32);
        assert_eq!(
            plan.capacities()[6].desired_capacity(),
            source.f64_values.len() + 18
        );
        let mut build = Control::default();
        let mut output = plan.materialize(&mut build).unwrap();
        assert_eq!(build.completed, plan.work());
        assert_eq!(output.f32_values.len(), expected_f32);
        let mut validation = Control::default();
        assert_eq!(
            output.validate(&mut validation, [1, 3, output.attributes.len(), 2], 3, 2),
            Ok(true)
        );
        assert_eq!(validation.completed, plan.validation_work());
        let row = output.attributes[plan.normal_attribute() as usize];
        let StoredAttributePayload::DenseF64(range) = row.payload else {
            panic!("F64 generated normals must retain their selected width");
        };
        assert_eq!(
            range
                .get(&output.f64_values)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            normals
                .as_flattened()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
        let mut correspondence = Control::default();
        assert_eq!(
            plan.validate_correspondence(&output, &mut correspondence),
            Ok(true)
        );
        assert_eq!(correspondence.completed, plan.correspondence_work());
        output.f64_values[range.start() + 2] = 0.0;
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(false)
        );
        assert_eq!(source.primitives[1].corner_count, 6);
        assert_eq!(
            source.attributes.len() as u32,
            plan.source_attribute_count()
        );
    }
}

#[test]
fn f64_normal_validation_uses_f64_unit_tolerance_and_rejects_nonfinite_candidates() {
    let source = geometry(true, false);
    for invalid in [
        [0.0; 3],
        [1.0 + 5.0 * f64::EPSILON, 0.0, 0.0],
        [1.0 + f64::from(f32::EPSILON) / 8.0, 0.0, 0.0],
        [f64::NAN, 0.0, 1.0],
        [f64::INFINITY, 0.0, 0.0],
        [0.0, f64::NEG_INFINITY, 1.0],
    ] {
        let mut normals = [[0.0, 0.0, 1.0]; 6];
        normals[5] = invalid;
        assert!(matches!(
            GeometryNormalsPlan::try_new(
                &source,
                1,
                VectorValues::F64(&normals),
                NormalWritePolicy::Replace,
                TangentPolicy::Reject,
                &mut Control::default(),
            ),
            Err(TypedStorageBuildError::Invariant)
        ));
    }
    for normal in [
        [1.0 / 3.0_f64.sqrt(); 3],
        [1.0 + 3.0 * f64::EPSILON, 0.0, 0.0],
    ] {
        let normals = [normal; 6];
        assert!(
            GeometryNormalsPlan::try_new(
                &source,
                1,
                VectorValues::F64(&normals),
                NormalWritePolicy::Replace,
                TangentPolicy::Reject,
                &mut Control::default(),
            )
            .is_ok()
        );
    }
}

#[test]
fn sparse_f64_survives_normal_addition_and_replacement_with_exact_work() {
    let explicit = [
        -0.0,
        f64::from_bits(1),
        f64::from_bits(1.0_f64.to_bits() + 1),
        1.0e250,
    ];
    for (normal, tangent) in [(false, false), (true, true)] {
        let mut source = geometry(normal, tangent);
        let source_row = source
            .attributes
            .iter()
            .position(|row| matches!(row.payload, StoredAttributePayload::SparseI32 { .. }))
            .unwrap();
        let StoredAttributePayload::SparseI32 {
            indices,
            values: old_values,
            default: old_default,
        } = source.attributes[source_row].payload
        else {
            unreachable!();
        };
        let removed = old_values.len() + old_default.len();
        source.i32_values = source.i32_values[removed..].to_vec();
        for row in &mut source.attributes {
            if let StoredAttributePayload::DenseI32(range) = &mut row.payload {
                *range = TableRange::new(range.start() - removed, range.len());
            }
        }
        source.attributes[source_row].payload = StoredAttributePayload::SparseF64 {
            indices,
            values: values(&mut source.f64_values, &explicit),
            default: values(&mut source.f64_values, &explicit[..2]),
        };
        assert_eq!(
            source.validate(
                &mut Control::default(),
                [1, 3, source.attributes.len(), 2],
                3,
                2
            ),
            Ok(true)
        );
        let plan = GeometryNormalsPlan::try_new(
            &source,
            1,
            VectorValues::F32(&NORMALS),
            if normal {
                NormalWritePolicy::Replace
            } else {
                NormalWritePolicy::Add
            },
            TangentPolicy::Remove,
            &mut Control::default(),
        )
        .unwrap();
        assert_eq!(
            plan.capacities()[6].desired_capacity(),
            source.f64_values.len()
        );
        let output_row = plan.output_attribute(source_row as u32).unwrap() as usize;
        assert_eq!(
            plan.source_attribute(output_row as u32),
            Some(source_row as u32)
        );
        let mut build = Control::default();
        let output = plan.materialize(&mut build).unwrap();
        assert_eq!(build.completed, plan.work());
        let mut validation = Control::default();
        assert_eq!(
            output.validate(&mut validation, [1, 3, output.attributes.len(), 2], 3, 2),
            Ok(true)
        );
        assert_eq!(validation.completed, plan.validation_work());
        let mut correspondence = Control::default();
        assert_eq!(
            plan.validate_correspondence(&output, &mut correspondence),
            Ok(true)
        );
        assert_eq!(correspondence.completed, plan.correspondence_work());
        let StoredAttributePayload::SparseF64 {
            indices,
            values,
            default,
        } = output.attributes[output_row].payload
        else {
            panic!("retained custom data must stay sparse F64");
        };
        assert_eq!(indices.get(&output.u32_values), Some(&[0, 4][..]));
        assert_eq!(values.start() + values.len(), default.start());
        assert_eq!(
            values
                .get(&output.f64_values)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            explicit.map(f64::to_bits)
        );
        assert_eq!(
            default
                .get(&output.f64_values)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            explicit[..2]
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
        for corruption in 0..3 {
            let mut output = plan.materialize(&mut Control::default()).unwrap();
            match corruption {
                0 => output.f64_values[values.start()] = 0.0,
                1 => output.f64_values[default.start()] = 0.0,
                2 => output.u32_values[indices.start()] = 1,
                _ => unreachable!(),
            }
            assert_eq!(
                plan.validate_correspondence(&output, &mut Control::default()),
                Ok(false)
            );
        }
    }
}

#[test]
fn add_at_end_of_position_only_range_preserves_later_primitives_and_their_tangents() {
    let source = geometry(true, true);
    let plan = GeometryNormalsPlan::try_new(
        &source,
        0,
        VectorValues::F32(&NORMALS[..3]),
        NormalWritePolicy::Add,
        TangentPolicy::Reject,
        &mut Control::default(),
    )
    .unwrap();
    assert_eq!(plan.source_normal(), None);
    assert_eq!(plan.source_tangent(), None);
    assert_eq!(plan.source_position(), 0);
    assert_eq!(plan.source_smoothing_group(), None);
    assert_eq!(plan.normal_attribute(), 1);
    assert_eq!(plan.source_attribute(1), None);
    assert_eq!(plan.output_attribute(1), Some(2));
    let mut build = Control::default();
    let output = plan.materialize(&mut build).unwrap();
    assert_eq!(build.completed, plan.work());
    assert!(output.primitives[0].indices.is_none());
    assert_eq!(output.primitives[1].attributes.start(), 2);
    assert_eq!(
        output.attributes[4].semantic,
        StoredAttributeSemantic::Tangent
    );
    let mut validation = Control::default();
    assert_eq!(
        output.validate(&mut validation, [1, 3, output.attributes.len(), 2], 3, 2),
        Ok(true)
    );
    assert_eq!(validation.completed, plan.validation_work());
    let mut correspondence = Control::default();
    assert_eq!(
        plan.validate_correspondence(&output, &mut correspondence),
        Ok(true)
    );
    assert_eq!(correspondence.completed, plan.correspondence_work());
}

#[test]
fn mapping_is_owned_copy_data_and_does_not_keep_the_source_or_normals_alive() {
    let mapping = {
        let source = geometry(false, true);
        let normals = NORMALS;
        let plan = GeometryNormalsPlan::try_new(
            &source,
            1,
            VectorValues::F32(&normals),
            NormalWritePolicy::Add,
            TangentPolicy::Remove,
            &mut Control::default(),
        )
        .unwrap();
        assert_eq!(plan.source_position(), 1);
        assert_eq!(plan.source_smoothing_group(), Some(5));
        let mapping = plan.attribute_mapping();
        assert_eq!(mapping.source_normal(), plan.source_normal());
        assert_eq!(mapping.source_tangent(), plan.source_tangent());
        assert_eq!(mapping.normal_attribute(), plan.normal_attribute());
        mapping
    };
    let copy = mapping;
    assert_eq!(copy, mapping);
    assert_eq!(mapping.source_position(), 1);
    assert_eq!(mapping.source_smoothing_group(), Some(5));
    assert_eq!(mapping.normal_attribute(), 2);
    assert_eq!(mapping.source_attribute(mapping.normal_attribute()), None);
    assert_eq!(
        mapping.output_attribute(mapping.source_tangent().unwrap()),
        None
    );
    assert_eq!(
        mapping.source_attribute_count(),
        mapping.output_attribute_count()
    );
}

#[test]
fn mapping_matches_sequence_edits_in_both_directions_at_every_small_boundary() {
    for source_count in 1..=8_u32 {
        for boundary in 0..=source_count {
            for normal in std::iter::once(None).chain((0..source_count).map(Some)) {
                for tangent in std::iter::once(None).chain((0..source_count).map(Some)) {
                    if normal.is_some() && normal == tangent {
                        continue;
                    }
                    let mut expected = Vec::new();
                    for source in 0..source_count {
                        if normal.is_none() && source == boundary {
                            expected.push(None);
                        }
                        if tangent != Some(source) {
                            expected.push(Some(source));
                        }
                    }
                    if normal.is_none() && boundary == source_count {
                        expected.push(None);
                    }
                    let normal_attribute = expected
                        .iter()
                        .position(|source| *source == normal)
                        .unwrap() as u32;
                    let mapping = NormalAttributeMapping {
                        source_attributes: source_count,
                        output_attributes: expected.len() as u32,
                        insertion_boundary: boundary,
                        normal_attribute,
                        source_normal: normal,
                        source_tangent: tangent,
                        source_position: 0,
                        source_smoothing_group: None,
                    };
                    for (output, source) in expected.iter().enumerate() {
                        assert_eq!(mapping.source_attribute(output as u32), *source);
                    }
                    for source in 0..source_count {
                        assert_eq!(
                            mapping.output_attribute(source),
                            expected
                                .iter()
                                .position(|row| *row == Some(source))
                                .map(|index| index as u32)
                        );
                    }
                    assert_eq!(mapping.output_attribute(source_count), None);
                    assert_eq!(mapping.output_attribute(u32::MAX), None);
                    assert_eq!(mapping.source_attribute(expected.len() as u32), None);
                    assert_eq!(mapping.source_attribute(u32::MAX), None);
                }
            }
        }
    }
}

#[test]
fn planning_allocates_nothing_and_rejects_wrong_policy_cardinality_topology_and_normals() {
    let source = geometry(true, true);
    let mut planned = None;
    let allocations = allocation_counter::measure(|| planned = Some(plan(&source)));
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert!(planned.is_some());
    for (write, tangents) in [
        (NormalWritePolicy::Add, TangentPolicy::Remove),
        (NormalWritePolicy::Replace, TangentPolicy::Reject),
    ] {
        assert!(matches!(
            GeometryNormalsPlan::try_new(
                &source,
                1,
                VectorValues::F32(&NORMALS),
                write,
                tangents,
                &mut Control::default()
            ),
            Err(TypedStorageBuildError::Invariant)
        ));
    }
    let no_normal = geometry(false, false);
    assert!(matches!(
        GeometryNormalsPlan::try_new(
            &no_normal,
            1,
            VectorValues::F32(&NORMALS),
            NormalWritePolicy::Replace,
            TangentPolicy::Remove,
            &mut Control::default()
        ),
        Err(TypedStorageBuildError::Invariant)
    ));
    let mut empty = geometry(false, false);
    empty.primitives[1].vertex_count = 0;
    empty.primitives[1].corner_count = 0;
    empty.primitives[1].face_count = 0;
    empty.primitives[1].indices = None;
    assert!(matches!(
        GeometryNormalsPlan::try_new(
            &empty,
            1,
            VectorValues::F32(&[]),
            NormalWritePolicy::Add,
            TangentPolicy::Remove,
            &mut Control::default()
        ),
        Err(TypedStorageBuildError::Invariant)
    ));
    for target in [2, source.primitives.len(), usize::MAX] {
        assert!(matches!(
            GeometryNormalsPlan::try_new(
                &source,
                target,
                VectorValues::F32(&NORMALS),
                NormalWritePolicy::Replace,
                TangentPolicy::Remove,
                &mut Control::default()
            ),
            Err(TypedStorageBuildError::Invariant)
        ));
    }
    for normals in [&NORMALS[..0], &NORMALS[..5], &[[0.0, 0.0, 1.0]; 7][..]] {
        assert!(matches!(
            GeometryNormalsPlan::try_new(
                &source,
                1,
                VectorValues::F32(normals),
                NormalWritePolicy::Replace,
                TangentPolicy::Remove,
                &mut Control::default()
            ),
            Err(TypedStorageBuildError::Invariant)
        ));
    }
    for bad in [
        [0.0; 3],
        [0.0, 0.0, 0.999],
        [0.0, 0.0, 1.001],
        [1.0; 3],
        [f32::NAN, 0.0, 1.0],
        [f32::INFINITY, 0.0, 0.0],
        [0.0, f32::NEG_INFINITY, 0.0],
    ] {
        let mut normals = NORMALS;
        normals[3] = bad;
        assert!(matches!(
            GeometryNormalsPlan::try_new(
                &source,
                1,
                VectorValues::F32(&normals),
                NormalWritePolicy::Replace,
                TangentPolicy::Remove,
                &mut Control::default()
            ),
            Err(TypedStorageBuildError::Invariant)
        ));
    }
    let rounded = [[1.0_f32 / 3.0_f32.sqrt(); 3]; 6];
    assert!(
        GeometryNormalsPlan::try_new(
            &source,
            1,
            VectorValues::F32(&rounded),
            NormalWritePolicy::Replace,
            TangentPolicy::Remove,
            &mut Control::default()
        )
        .is_ok()
    );
}

#[test]
fn every_planning_build_and_correspondence_action_is_interruptible() {
    for normal in [false, true] {
        let source = geometry(normal, true);
        let write = if normal {
            NormalWritePolicy::Replace
        } else {
            NormalWritePolicy::Add
        };
        let before = source
            .f32_values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        let mut planning = Control::default();
        let plan = GeometryNormalsPlan::try_new(
            &source,
            1,
            VectorValues::F32(&NORMALS),
            write,
            TangentPolicy::Remove,
            &mut planning,
        )
        .unwrap();
        for completed in 0..planning.completed {
            let mut control = Control {
                reject_before: Some(completed),
                ..Control::default()
            };
            assert!(
                matches!(GeometryNormalsPlan::try_new(&source, 1, VectorValues::F32(&NORMALS),
                write, TangentPolicy::Remove, &mut control),
                Err(TypedStorageBuildError::Control(at)) if at == completed)
            );
            assert_eq!(control.completed, completed);
        }
        for completed in 0..plan.work() {
            let mut control = Control {
                reject_before: Some(completed),
                ..Control::default()
            };
            assert!(
                matches!(plan.materialize(&mut control), Err(TypedStorageBuildError::Control(at)) if at == completed)
            );
            assert_eq!(control.completed, completed);
        }
        let output = plan.materialize(&mut Control::default()).unwrap();
        for completed in 0..plan.correspondence_work() {
            let mut control = Control {
                reject_before: Some(completed),
                ..Control::default()
            };
            assert_eq!(
                plan.validate_correspondence(&output, &mut control),
                Err(completed)
            );
            assert_eq!(control.completed, completed);
            let mut control = Control {
                omit_before: Some(completed),
                ..Control::default()
            };
            assert_eq!(
                plan.validate_correspondence(&output, &mut control),
                Ok(false)
            );
            assert_eq!(control.completed, completed);
        }
        assert_eq!(
            source
                .f32_values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            before
        );
        let tangent = plan.source_tangent().unwrap() as usize;
        assert_eq!(
            source.attributes[tangent].semantic,
            StoredAttributeSemantic::Tangent
        );
        assert_eq!(
            source.attributes[1].semantic,
            StoredAttributeSemantic::Position
        );
    }
}

#[test]
fn correspondence_compares_bits_and_sparse_explicitness_and_rejects_descriptor_corruption() {
    let source = geometry(true, true);
    let plan = plan(&source);
    let mutations: [fn(&mut GeometryStorage); 8] = [
        |output| output.attributes[2].domain = AttributeDomain::Vertex,
        |output| output.attributes[2].lanes = 2,
        |output| output.attributes[2].element_count -= 1,
        |output| {
            let StoredAttributePayload::DenseF32(range) = output.attributes[2].payload else {
                unreachable!()
            };
            output.f32_values[range.start()] = 0.0;
        },
        |output| output.f64_values[1] = 0.0,
        |output| {
            let StoredAttributePayload::SparseF32 { indices, .. } = output.attributes[4].payload
            else {
                unreachable!()
            };
            output.u32_values[indices.start()] = 0;
        },
        |output| output.primitives[1].material = crate::MaterialAssignment::Authored(0),
        |output| output.u32_values.push(0),
    ];
    for mutate in mutations {
        let mut output = plan.materialize(&mut Control::default()).unwrap();
        mutate(&mut output);
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(false)
        );
    }
}

#[test]
fn correspondence_is_independent_of_private_pool_offsets() {
    let source = geometry(true, true);
    let plan = plan(&source);
    let mut output = plan.materialize(&mut Control::default()).unwrap();
    let first_name = match output.attributes.iter().find_map(|row| {
        if let StoredAttributeSemantic::Custom(name) = row.semantic {
            Some(name)
        } else {
            None
        }
    }) {
        Some(name) => name.0.len(),
        None => panic!("fixture must own custom names"),
    };
    let names = output.names.len();
    output.names.rotate_left(first_name);
    let move_name = |name: TextRange| {
        let start = if name.0.start() < first_name {
            names - first_name + name.0.start()
        } else {
            name.0.start() - first_name
        };
        TextRange(TableRange::new(start, name.0.len()))
    };
    for row in &mut output.attributes {
        if let StoredAttributeSemantic::Custom(name) = row.semantic {
            row.semantic = StoredAttributeSemantic::Custom(move_name(name));
        }
    }
    for mesh in &mut output.logical_meshes {
        mesh.name = mesh.name.map(move_name);
    }
    // Move a complete Position payload, keeping every referenced scalar's bit value unchanged.
    let split = 9;
    let length = output.f32_values.len();
    output.f32_values.rotate_left(split);
    let relocate = |range: TableRange<f32>| {
        TableRange::new(
            if range.start() < split {
                length - split + range.start()
            } else {
                range.start() - split
            },
            range.len(),
        )
    };
    for row in &mut output.attributes {
        row.payload = match row.payload {
            StoredAttributePayload::DenseF32(range) => {
                StoredAttributePayload::DenseF32(relocate(range))
            }
            StoredAttributePayload::SparseF32 {
                indices,
                values,
                default,
            } => StoredAttributePayload::SparseF32 {
                indices,
                values: relocate(values),
                default: relocate(default),
            },
            other => other,
        };
    }
    let mut control = Control::default();
    assert_eq!(
        plan.validate_correspondence(&output, &mut control),
        Ok(true)
    );
    assert_eq!(control.completed, plan.correspondence_work());
}

fn payload_signature(
    payload: StoredAttributePayload,
    storage: &GeometryStorage,
) -> (u8, Vec<u32>, Vec<u8>, Vec<u8>) {
    macro_rules! dense {
        ($tag:expr, $range:expr, $pool:ident) => {{
            (
                $tag,
                Vec::new(),
                $range
                    .get(&storage.$pool)
                    .unwrap()
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
                Vec::new(),
            )
        }};
    }
    macro_rules! sparse {
        ($tag:expr, $indices:expr, $values:expr, $default:expr, $pool:ident) => {{
            (
                $tag,
                $indices.get(&storage.u32_values).unwrap().to_vec(),
                $values
                    .get(&storage.$pool)
                    .unwrap()
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
                $default
                    .get(&storage.$pool)
                    .unwrap()
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            )
        }};
    }
    match payload {
        StoredAttributePayload::DenseF32(r) => dense!(0, r, f32_values),
        StoredAttributePayload::DenseF64(r) => dense!(1, r, f64_values),
        StoredAttributePayload::DenseU32(r) => dense!(2, r, u32_values),
        StoredAttributePayload::DenseI32(r) => dense!(3, r, i32_values),
        StoredAttributePayload::SparseF32 {
            indices,
            values,
            default,
        } => sparse!(4, indices, values, default, f32_values),
        StoredAttributePayload::SparseF64 {
            indices,
            values,
            default,
        } => sparse!(5, indices, values, default, f64_values),
        StoredAttributePayload::SparseU32 {
            indices,
            values,
            default,
        } => sparse!(6, indices, values, default, u32_values),
        StoredAttributePayload::SparseI32 {
            indices,
            values,
            default,
        } => sparse!(7, indices, values, default, i32_values),
    }
}
