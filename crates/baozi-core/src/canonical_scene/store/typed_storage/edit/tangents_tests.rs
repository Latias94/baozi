use super::*;
use crate::canonical_scene::store::typed_storage::{
    StoredAttributePayload, StoredLogicalMeshIndex, StoredNodeIndex, TextRange,
};
use std::num::NonZeroU32;

const CONVENTIONS: TangentConventions = TangentConventions {
    basis: TangentBasisConvention::TriangleUvDerivative,
    encoding: TangentHandednessEncoding::SignInW,
};
const TANGENTS: [[f32; 4]; 6] = [
    [1.0, -0.0, 0.0, 1.0],
    [1.0, 0.0, -0.0, 1.0],
    [1.0, 0.0, 0.0, 1.0],
    [-0.0, 1.0, 0.0, -1.0],
    [0.0, 1.0, -0.0, -1.0],
    [0.0, 1.0, 0.0, -1.0],
];

#[derive(Default)]
struct Control {
    completed: u64,
    strings: u64,
    reject_before: Option<u64>,
    omit_before: Option<u64>,
}

impl TypedStorageBuildControl for Control {
    type Error = u64;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        if self.reject_before == Some(self.completed) {
            return Err(self.completed);
        }
        let result = action();
        self.completed += 1;
        Ok(result)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        let result = TypedStorageBuildControl::run_unit(self, action)?;
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
        TypedStorageBuildControl::run_unit(self, action).map(Some)
    }
}

fn values<T: Copy>(pool: &mut Vec<T>, values: &[T]) -> TableRange<T> {
    let start = pool.len();
    pool.extend_from_slice(values);
    TableRange::new(start, values.len())
}

fn name(source: &mut GeometryStorage, text: &str) -> TextRange {
    TextRange(values(&mut source.names, text.as_bytes()))
}

fn attribute(
    source: &mut GeometryStorage,
    semantic: StoredAttributeSemantic,
    domain: AttributeDomain,
    lanes: u8,
    count: u32,
    payload: StoredAttributePayload,
) {
    source.attributes.push(StoredAttribute {
        semantic,
        tangent_conventions: (semantic == StoredAttributeSemantic::Tangent)
            .then_some(StoredTangentConventions::UNKNOWN),
        domain,
        lanes,
        element_count: count,
        payload,
    });
}

fn external_conventions(source: &mut GeometryStorage, basis: &str) -> StoredTangentConventions {
    StoredTangentConventions {
        basis: StoredTangentBasis::Namespaced {
            name: name(source, basis),
            version: NonZeroU32::new(2).unwrap(),
        },
        encoding: StoredTangentEncoding::Namespaced {
            name: name(source, "test:encoding"),
            version: NonZeroU32::new(3).unwrap(),
        },
    }
}

/// The selected primitive is between two unchanged primitives, with two Normal domains and
/// two UV domains. Replacing its external tangent must compact only that descriptor's names.
fn geometry(tangent: bool) -> GeometryStorage {
    let mut source = GeometryStorage {
        logical_meshes: Vec::new(),
        primitives: Vec::new(),
        attributes: Vec::new(),
        mesh_instances: vec![StoredMeshInstance {
            owner_node: StoredNodeIndex(1),
            logical_mesh: StoredLogicalMeshIndex(0),
        }],
        names: Vec::new(),
        f32_values: Vec::new(),
        f64_values: Vec::new(),
        u32_values: Vec::new(),
        i32_values: Vec::new(),
    };
    let positions = values(&mut source.f32_values, &[0.0; 9]);
    attribute(
        &mut source,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        3,
        StoredAttributePayload::DenseF32(positions),
    );
    let other_tangents = values(&mut source.f32_values, &[1.0, 0.0, 0.0, 1.0].repeat(3));
    let conventions = external_conventions(&mut source, "test:retained_basis");
    attribute(
        &mut source,
        StoredAttributeSemantic::Tangent,
        AttributeDomain::Vertex,
        4,
        3,
        StoredAttributePayload::DenseF32(other_tangents),
    );
    source.attributes.last_mut().unwrap().tangent_conventions = Some(conventions);
    let target_start = source.attributes.len();
    let positions = values(
        &mut source.f32_values,
        &[0.0, -0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0],
    );
    attribute(
        &mut source,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        4,
        StoredAttributePayload::DenseF32(positions),
    );
    let normal = StoredAttributePayload::SparseF32 {
        indices: values(&mut source.u32_values, &[2]),
        values: values(&mut source.f32_values, &[0.0, -0.0, 1.0]),
        default: values(&mut source.f32_values, &[-0.0, 0.0, 1.0]),
    };
    attribute(
        &mut source,
        StoredAttributeSemantic::Normal,
        AttributeDomain::Vertex,
        3,
        4,
        normal,
    );
    let corner_normals = values(&mut source.f32_values, &[0.0, 0.0, 1.0].repeat(6));
    attribute(
        &mut source,
        StoredAttributeSemantic::Normal,
        AttributeDomain::Corner,
        3,
        6,
        StoredAttributePayload::DenseF32(corner_normals),
    );
    if tangent {
        let old = StoredAttributePayload::SparseF32 {
            indices: values(&mut source.u32_values, &[1, 3]),
            values: values(&mut source.f32_values, &[1.0, 0.0, 0.0, -1.0].repeat(2)),
            default: values(&mut source.f32_values, &[1.0, -0.0, 0.0, 1.0]),
        };
        let conventions = external_conventions(&mut source, "test:removed_basis");
        attribute(
            &mut source,
            StoredAttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            4,
            4,
            old,
        );
        source.attributes.last_mut().unwrap().tangent_conventions = Some(conventions);
    }
    let uv = values(
        &mut source.f32_values,
        &[0.0, -0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
    );
    attribute(
        &mut source,
        StoredAttributeSemantic::Texcoord(7),
        AttributeDomain::Vertex,
        2,
        4,
        StoredAttributePayload::DenseF32(uv),
    );
    let uv = StoredAttributePayload::SparseF32 {
        indices: values(&mut source.u32_values, &[1, 2, 4, 5]),
        values: values(
            &mut source.f32_values,
            &[1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0],
        ),
        default: values(&mut source.f32_values, &[-0.0, 0.0]),
    };
    attribute(
        &mut source,
        StoredAttributeSemantic::Texcoord(7),
        AttributeDomain::Corner,
        2,
        6,
        uv,
    );
    let custom_name = name(&mut source, "test:a_sparse");
    let custom = StoredAttributePayload::SparseI32 {
        indices: values(&mut source.u32_values, &[0, 5]),
        values: values(&mut source.i32_values, &[0, i32::MIN]),
        default: values(&mut source.i32_values, &[0]),
    };
    attribute(
        &mut source,
        StoredAttributeSemantic::Custom(custom_name),
        AttributeDomain::Corner,
        1,
        6,
        custom,
    );
    let custom_name = name(&mut source, "test:b_precise");
    let precise = values(
        &mut source.f64_values,
        &[-0.0, f64::from_bits(0x3ff0_0000_0000_0001)],
    );
    attribute(
        &mut source,
        StoredAttributeSemantic::Custom(custom_name),
        AttributeDomain::Face,
        1,
        2,
        StoredAttributePayload::DenseF64(precise),
    );
    let target_end = source.attributes.len();
    let positions = values(&mut source.f32_values, &[2.0; 9]);
    attribute(
        &mut source,
        StoredAttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        3,
        StoredAttributePayload::DenseF32(positions),
    );
    let indices = values(&mut source.u32_values, &[0, 1, 2, 0, 2, 3]);
    source.primitives = vec![
        StoredPrimitive {
            topology: TopologyKind::TriangleList,
            vertex_count: 3,
            corner_count: 3,
            face_count: 1,
            polygon_boundaries: None,
            indices: None,
            material: None,
            attributes: TableRange::new(0, target_start),
        },
        StoredPrimitive {
            topology: TopologyKind::TriangleList,
            vertex_count: 4,
            corner_count: 6,
            face_count: 2,
            polygon_boundaries: None,
            indices: Some(indices),
            material: None,
            attributes: TableRange::new(target_start, target_end - target_start),
        },
        StoredPrimitive {
            topology: TopologyKind::TriangleList,
            vertex_count: 3,
            corner_count: 3,
            face_count: 1,
            polygon_boundaries: None,
            indices: None,
            material: None,
            attributes: TableRange::new(target_end, source.attributes.len() - target_end),
        },
    ];
    let mesh_name = name(&mut source, "mesh α");
    source.logical_meshes.push(StoredLogicalMesh {
        primitives: TableRange::new(0, 3),
        name: Some(mesh_name),
    });
    source
}

fn plan(source: &GeometryStorage, had_tangent: bool) -> GeometryTangentsPlan<'_> {
    GeometryTangentsPlan::try_new(
        source,
        1,
        VectorValues::F32(&TANGENTS),
        had_tangent,
        AttributeDomain::Vertex,
        7,
        AttributeDomain::Corner,
        &CONVENTIONS,
        &mut Control::default(),
    )
    .unwrap()
}

#[test]
fn add_and_replace_preserve_sparse_payloads_and_exact_name_owner_layouts() {
    for had_tangent in [false, true] {
        let source = geometry(had_tangent);
        assert_eq!(
            source.validate(
                &mut Control::default(),
                [1, 3, source.attributes.len(), 1],
                2,
                0
            ),
            Ok(true)
        );
        let mut planned = None;
        let allocation = allocation_counter::measure(|| {
            planned = Some(plan(&source, had_tangent));
        });
        assert_eq!(allocation.count_total, 0);
        let plan = planned.unwrap();
        let mut build = Control::default();
        let output = plan.materialize(&mut build).unwrap();
        assert_eq!(build.completed, plan.work());
        assert_eq!(build.strings, plan.name_bytes() as u64);
        assert_eq!(output.validation_work(), Some(plan.validation_work()));
        let mut validation = Control::default();
        assert_eq!(
            output.validate(&mut validation, [1, 3, output.attributes.len(), 1], 2, 0),
            Ok(true)
        );
        assert_eq!(validation.completed, plan.validation_work());
        let mut correspondence = Control::default();
        assert_eq!(
            plan.validate_correspondence(&output, &mut correspondence),
            Ok(true)
        );
        assert_eq!(correspondence.completed, plan.correspondence_work());
        let mapping = plan.attribute_mapping();
        assert_eq!(mapping.tangent_attribute(), 5);
        assert_eq!(mapping.source_position(), 2);
        assert_eq!(mapping.source_normal(), 3);
        assert_eq!(mapping.source_uv(), 6 + u32::from(had_tangent));
        assert_eq!(mapping.source_tangent(), had_tangent.then_some(5));
        assert!(same_range(
            source.primitives[0].attributes,
            output.primitives[0].attributes
        ));
        assert_eq!(
            source.primitives[1].attributes.start(),
            output.primitives[1].attributes.start()
        );
        assert_eq!(
            output.primitives[2].attributes.start(),
            source.primitives[2].attributes.start() + usize::from(!had_tangent)
        );
        for source_row in 0..source.attributes.len() as u32 {
            let destination = mapping.output_attribute(source_row).unwrap();
            assert_eq!(mapping.source_attribute(destination), Some(source_row));
        }
        assert_eq!(
            mapping.output_attribute(source.attributes.len() as u32),
            None
        );
        assert_eq!(
            mapping.source_attribute(output.attributes.len() as u32),
            None
        );
        let row = &output.attributes[5];
        assert_eq!(row.domain, AttributeDomain::Corner);
        assert_eq!(row.tangent_conventions, Some(GENERATED_CONVENTIONS));
        let StoredAttributePayload::DenseF32(range) = row.payload else {
            panic!("dense generated tangents")
        };
        assert_eq!(
            range
                .get(&output.f32_values)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            TANGENTS
                .as_flattened()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
        let names = std::str::from_utf8(&output.names).unwrap();
        assert!(names.contains("test:retained_basis"));
        assert!(!names.contains("test:removed_basis"));
        let removed = if had_tangent {
            "test:removed_basis".len() + "test:encoding".len()
        } else {
            0
        };
        assert_eq!(plan.name_bytes(), source.names.len() - removed);
        assert_eq!(source.f64_values[0].to_bits(), (-0.0_f64).to_bits());
    }
}

#[test]
fn generated_f64_tangents_use_the_f64_pool_and_preserve_candidate_bits() {
    let tangents = [
        [1.0, f64::from_bits(1), -0.0, 1.0],
        [f64::from_bits(0x3fef_ffff_ffff_ffff), -0.0, 0.0, -1.0],
        [0.0, -0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0, -1.0],
        [-0.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, -1.0, -1.0],
    ];
    for had_tangent in [false, true] {
        let source = geometry(had_tangent);
        let plan = GeometryTangentsPlan::try_new(
            &source,
            1,
            VectorValues::F64(&tangents),
            had_tangent,
            AttributeDomain::Vertex,
            7,
            AttributeDomain::Corner,
            &CONVENTIONS,
            &mut Control::default(),
        )
        .unwrap();
        let expected_f32 = source.f32_values.len() - usize::from(had_tangent) * 12;
        assert_eq!(plan.capacities()[5].desired_capacity(), expected_f32);
        assert_eq!(
            plan.capacities()[6].desired_capacity(),
            source.f64_values.len() + 24
        );
        let mut build = Control::default();
        let mut output = plan.materialize(&mut build).unwrap();
        assert_eq!(build.completed, plan.work());
        assert_eq!(output.f32_values.len(), expected_f32);
        let mut validation = Control::default();
        assert_eq!(
            output.validate(&mut validation, [1, 3, output.attributes.len(), 1], 2, 0),
            Ok(true)
        );
        assert_eq!(validation.completed, plan.validation_work());
        let row = output.attributes[plan.attribute_mapping().tangent_attribute() as usize];
        assert_eq!(row.tangent_conventions, Some(GENERATED_CONVENTIONS));
        let StoredAttributePayload::DenseF64(range) = row.payload else {
            panic!("F64 generated tangents must retain their selected width");
        };
        assert_eq!(
            range
                .get(&output.f64_values)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            tangents
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
    }
}

#[test]
fn f64_tangent_validation_checks_unit_tolerance_handedness_and_finite_candidates() {
    let source = geometry(true);
    for invalid in [
        [0.0; 4],
        [1.0 + 5.0 * f64::EPSILON, 0.0, 0.0, 1.0],
        [1.0 + f64::from(f32::EPSILON) / 8.0, 0.0, 0.0, 1.0],
        [f64::NAN, 0.0, 0.0, 1.0],
        [1.0, f64::INFINITY, 0.0, 1.0],
        [1.0, 0.0, 0.0, -0.0],
        [1.0, 0.0, 0.0, f64::NAN],
        [1.0, 0.0, 0.0, f64::INFINITY],
    ] {
        let mut tangents = [[1.0, 0.0, 0.0, 1.0]; 6];
        tangents[5] = invalid;
        assert!(matches!(
            GeometryTangentsPlan::try_new(
                &source,
                1,
                VectorValues::F64(&tangents),
                true,
                AttributeDomain::Vertex,
                7,
                AttributeDomain::Corner,
                &CONVENTIONS,
                &mut Control::default(),
            ),
            Err(TypedStorageBuildError::Invariant)
        ));
    }
    let tangents = [[1.0 + 3.0 * f64::EPSILON, 0.0, 0.0, 1.0]; 6];
    assert!(
        GeometryTangentsPlan::try_new(
            &source,
            1,
            VectorValues::F64(&tangents),
            true,
            AttributeDomain::Vertex,
            7,
            AttributeDomain::Corner,
            &CONVENTIONS,
            &mut Control::default(),
        )
        .is_ok()
    );
}

#[test]
fn sparse_f64_survives_tangent_addition_and_replacement_with_exact_work() {
    let explicit = [
        -0.0,
        f64::from_bits(1),
        f64::from_bits(1.0_f64.to_bits() + 1),
        1.0e250,
    ];
    for had_tangent in [false, true] {
        let mut source = geometry(had_tangent);
        let source_row = source
            .attributes
            .iter()
            .position(|row| matches!(row.payload, StoredAttributePayload::SparseI32 { .. }))
            .unwrap();
        let StoredAttributePayload::SparseI32 { indices, .. } =
            source.attributes[source_row].payload
        else {
            unreachable!();
        };
        source.i32_values.clear();
        let dense_values = std::mem::take(&mut source.f64_values);
        source.attributes[source_row].lanes = 2;
        source.attributes[source_row].payload = StoredAttributePayload::SparseF64 {
            indices,
            values: values(&mut source.f64_values, &explicit),
            default: values(&mut source.f64_values, &explicit[..2]),
        };
        let prefix = source.f64_values.len();
        for row in &mut source.attributes {
            if let StoredAttributePayload::DenseF64(range) = &mut row.payload {
                *range = TableRange::new(range.start() + prefix, range.len());
            }
        }
        source.f64_values.extend_from_slice(&dense_values);
        assert_eq!(
            source.validate(
                &mut Control::default(),
                [1, 3, source.attributes.len(), 1],
                2,
                0
            ),
            Ok(true)
        );
        let plan = plan(&source, had_tangent);
        assert_eq!(
            plan.capacities()[6].desired_capacity(),
            source.f64_values.len()
        );
        let mapping = plan.attribute_mapping();
        let output_row = mapping.output_attribute(source_row as u32).unwrap() as usize;
        assert_eq!(
            mapping.source_attribute(output_row as u32),
            Some(source_row as u32)
        );
        let mut build = Control::default();
        let mut output = plan.materialize(&mut build).unwrap();
        assert_eq!(build.completed, plan.work());
        let mut validation = Control::default();
        assert_eq!(
            output.validate(&mut validation, [1, 3, output.attributes.len(), 1], 2, 0),
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
        assert_eq!(indices.get(&output.u32_values), Some(&[0, 5][..]));
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
        for scalar in [values.start(), default.start()] {
            let mut corrupted = plan.materialize(&mut Control::default()).unwrap();
            corrupted.f64_values[scalar] = 0.0;
            assert_eq!(
                plan.validate_correspondence(&corrupted, &mut Control::default()),
                Ok(false)
            );
        }

        // Correspondence compares payloads independently of their compacted scalar offsets.
        let split = values.len();
        let length = output.f64_values.len();
        output.f64_values.rotate_left(split);
        let relocate = |range: TableRange<f64>| {
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
                StoredAttributePayload::DenseF64(range) => {
                    StoredAttributePayload::DenseF64(relocate(range))
                }
                StoredAttributePayload::SparseF64 {
                    indices,
                    values,
                    default,
                } => StoredAttributePayload::SparseF64 {
                    indices,
                    values: relocate(values),
                    default: relocate(default),
                },
                other => other,
            };
        }
        let mut correspondence = Control::default();
        assert_eq!(
            plan.validate_correspondence(&output, &mut correspondence),
            Ok(true)
        );
        assert_eq!(correspondence.completed, plan.correspondence_work());
    }
}

#[test]
fn source_selection_replacement_policy_and_generated_declarations_are_checked() {
    let source = geometry(true);
    let rejected = |had, normal, set, uv, conventions: &TangentConventions| {
        matches!(
            GeometryTangentsPlan::try_new(
                &source,
                1,
                VectorValues::F32(&TANGENTS),
                had,
                normal,
                set,
                uv,
                conventions,
                &mut Control::default()
            ),
            Err(TypedStorageBuildError::Invariant)
        )
    };
    assert!(rejected(
        false,
        AttributeDomain::Vertex,
        7,
        AttributeDomain::Corner,
        &CONVENTIONS
    ));
    assert!(rejected(
        true,
        AttributeDomain::Face,
        7,
        AttributeDomain::Corner,
        &CONVENTIONS
    ));
    assert!(rejected(
        true,
        AttributeDomain::Vertex,
        8,
        AttributeDomain::Corner,
        &CONVENTIONS
    ));
    assert!(rejected(
        true,
        AttributeDomain::Vertex,
        7,
        AttributeDomain::Face,
        &CONVENTIONS
    ));
    for basis in [
        TangentBasisConvention::Unknown,
        TangentBasisConvention::MikkTspace,
    ] {
        let conventions = TangentConventions {
            basis,
            encoding: TangentHandednessEncoding::SignInW,
        };
        assert!(rejected(
            true,
            AttributeDomain::Vertex,
            7,
            AttributeDomain::Corner,
            &conventions
        ));
    }
    let conventions = TangentConventions {
        basis: TangentBasisConvention::TriangleUvDerivative,
        encoding: TangentHandednessEncoding::Unknown,
    };
    assert!(rejected(
        true,
        AttributeDomain::Vertex,
        7,
        AttributeDomain::Corner,
        &conventions
    ));
    let mut duplicate = geometry(true);
    let row = duplicate.attributes[5];
    duplicate.attributes.insert(
        6,
        StoredAttribute {
            domain: AttributeDomain::Corner,
            ..row
        },
    );
    duplicate.primitives[1].attributes =
        TableRange::new(2, duplicate.primitives[1].attributes.len() + 1);
    assert!(matches!(
        GeometryTangentsPlan::try_new(
            &duplicate,
            1,
            VectorValues::F32(&TANGENTS),
            true,
            AttributeDomain::Vertex,
            7,
            AttributeDomain::Corner,
            &CONVENTIONS,
            &mut Control::default()
        ),
        Err(TypedStorageBuildError::Invariant)
    ));
    let without = geometry(false);
    assert!(matches!(
        GeometryTangentsPlan::try_new(
            &without,
            1,
            VectorValues::F32(&TANGENTS),
            true,
            AttributeDomain::Vertex,
            7,
            AttributeDomain::Corner,
            &CONVENTIONS,
            &mut Control::default()
        ),
        Err(TypedStorageBuildError::Invariant)
    ));
}

#[test]
fn invalid_values_counts_and_candidate_corruption_cannot_be_published() {
    let source = geometry(true);
    for value in [
        [0.0; 4],
        [2.0, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, -0.0],
        [1.0, 0.0, 0.0, f32::NAN],
        [f32::INFINITY, 0.0, 0.0, 1.0],
    ] {
        let mut tangents = TANGENTS;
        tangents[5] = value;
        assert!(matches!(
            GeometryTangentsPlan::try_new(
                &source,
                1,
                VectorValues::F32(&tangents),
                true,
                AttributeDomain::Vertex,
                7,
                AttributeDomain::Corner,
                &CONVENTIONS,
                &mut Control::default()
            ),
            Err(TypedStorageBuildError::Invariant)
        ));
    }
    assert!(matches!(
        GeometryTangentsPlan::try_new(
            &source,
            1,
            VectorValues::F32(&TANGENTS[..5]),
            true,
            AttributeDomain::Vertex,
            7,
            AttributeDomain::Corner,
            &CONVENTIONS,
            &mut Control::default()
        ),
        Err(TypedStorageBuildError::Invariant)
    ));
    let plan = plan(&source, true);
    for corruption in 0..6 {
        let mut output = plan.materialize(&mut Control::default()).unwrap();
        match corruption {
            0 => output.attributes[5].tangent_conventions = Some(StoredTangentConventions::UNKNOWN),
            1 => output.f32_values[0] = -0.0,
            2 => output.f64_values[0] = 0.0,
            3 => output.names[0] = b'X',
            4 => output.primitives[1].indices = None,
            5 => {
                let StoredAttributePayload::DenseF32(range) = output.attributes[5].payload else {
                    panic!("dense generated tangents")
                };
                output.f32_values[range.start() + 1] = 0.0;
            }
            _ => unreachable!(),
        }
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(false)
        );
    }
}

#[test]
fn planning_materialization_and_correspondence_have_interruptible_exact_work() {
    let source = geometry(true);
    let mut planning = Control::default();
    let plan = GeometryTangentsPlan::try_new(
        &source,
        1,
        VectorValues::F32(&TANGENTS),
        true,
        AttributeDomain::Vertex,
        7,
        AttributeDomain::Corner,
        &CONVENTIONS,
        &mut planning,
    )
    .unwrap();
    for reject_before in 0..planning.completed {
        let mut control = Control {
            reject_before: Some(reject_before),
            ..Control::default()
        };
        assert!(
            matches!(GeometryTangentsPlan::try_new(&source, 1, VectorValues::F32(&TANGENTS), true, AttributeDomain::Vertex, 7,
            AttributeDomain::Corner, &CONVENTIONS, &mut control), Err(TypedStorageBuildError::Control(at)) if at == reject_before)
        );
    }
    for reject_before in 0..plan.work() {
        let mut control = Control {
            reject_before: Some(reject_before),
            ..Control::default()
        };
        assert!(
            matches!(plan.materialize(&mut control), Err(TypedStorageBuildError::Control(at)) if at == reject_before)
        );
    }
    let output = plan.materialize(&mut Control::default()).unwrap();
    for omit_before in 0..plan.correspondence_work() {
        let mut control = Control {
            omit_before: Some(omit_before),
            ..Control::default()
        };
        assert_eq!(
            plan.validate_correspondence(&output, &mut control),
            Ok(false)
        );
    }
    assert_eq!(source.attributes[5].domain, AttributeDomain::Vertex);
    assert_eq!(source.primitives[1].corner_count, 6);
}
