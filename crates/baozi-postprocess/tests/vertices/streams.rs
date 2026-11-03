use super::*;
use baozi_core::{TangentBasisRef, TangentConventionsRef, TangentEncodingRef};
use baozi_import::authoring::{
    TangentBasisConvention, TangentConventions, TangentHandednessEncoding,
};
use baozi_postprocess::ListAttribute;

const WIDE_DEFAULT: [f64; 16] = [
    -0.0,
    f64::from_bits(1),
    f64::from_bits(0x3ff0_0000_0000_0001),
    16_777_217.0,
    1e100,
    -1e100,
    2.0,
    -2.0,
    3.0,
    -3.0,
    4.0,
    -4.0,
    5.0,
    -5.0,
    6.0,
    -6.0,
];
const WIDE_EXPLICIT: [f64; 32] = [
    -0.0,
    f64::from_bits(1),
    f64::from_bits(0x3ff0_0000_0000_0001),
    16_777_217.0,
    1e100,
    -1e100,
    2.0,
    -2.0,
    3.0,
    -3.0,
    4.0,
    -4.0,
    5.0,
    -5.0,
    6.0,
    -6.0,
    0.0,
    -f64::from_bits(1),
    1.0,
    16_777_219.0,
    -1e100,
    1e100,
    -2.0,
    2.0,
    -3.0,
    3.0,
    -4.0,
    4.0,
    -5.0,
    5.0,
    -6.0,
    6.0,
];
const UNSIGNED: [u32; 5] = [0, u32::MAX, 0x8000_0001, 7, 19];
const TANGENTS: [f32; 20] = [
    1.0, -0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, -1.0, 1.0, -0.0, 0.0, 1.0, 0.0, 1.0,
    0.0, -1.0,
];

pub(super) fn submit(context: &mut ParseContext<'_>) -> ParseResult<()> {
    let mut attributes = Vec::new();
    for _ in 0..2 {
        attributes.extend([
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &RICH_POSITIONS,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Tangent,
                AttributeDomain::Vertex,
                4,
                &TANGENTS,
                Presence::Absent,
            )
            .with_tangent_conventions(TangentConventions {
                basis: TangentBasisConvention::MikkTspace,
                encoding: TangentHandednessEncoding::SignInW,
            }),
            GeometryAttributeAuthoring::u32(
                custom("test:unsigned"),
                AttributeDomain::Vertex,
                1,
                &UNSIGNED,
            ),
            GeometryAttributeAuthoring::sparse_f64(
                custom("test:wide"),
                AttributeDomain::Vertex,
                16,
                5,
                &[1, 3],
                &WIDE_EXPLICIT,
                &WIDE_DEFAULT,
            ),
            GeometryAttributeAuthoring::sparse_f64(
                custom("test:wide"),
                AttributeDomain::Corner,
                16,
                6,
                &[1, 4],
                &WIDE_EXPLICIT,
                &WIDE_DEFAULT,
            ),
        ]);
    }
    let primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&RICH_INDICES),
            baozi_core::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 5),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&RICH_INDICES),
            baozi_core::MaterialAssignment::Unspecified,
            AuthoringRange::new(5, 5),
        ),
    ];
    let meshes = [
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1)),
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(1, 1)),
    ];
    context.submit(
        StaticSceneBuilder::new(Presence::Present(&[]), &[]).with_geometry(
            &meshes,
            &primitives,
            &attributes,
            &[],
        ),
    )
}

fn assert_same_element(actual: AttributeElement<'_>, expected: AttributeElement<'_>) {
    match (actual, expected) {
        (AttributeElement::F32(actual), AttributeElement::F32(expected)) => {
            assert_f32(actual, expected);
            assert!(std::ptr::eq(actual, expected));
        }
        (AttributeElement::F64(actual), AttributeElement::F64(expected)) => {
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(expected) {
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
            assert!(std::ptr::eq(actual, expected));
        }
        (AttributeElement::U32(actual), AttributeElement::U32(expected)) => {
            assert_eq!(actual, expected);
            assert!(std::ptr::eq(actual, expected));
        }
        (AttributeElement::I32(actual), AttributeElement::I32(expected)) => {
            assert_eq!(actual, expected);
            assert!(std::ptr::eq(actual, expected));
        }
        _ => panic!("a bound stream must retain its source scalar kind"),
    }
}

#[test]
fn bound_streams_keep_literal_coordinates_seams_domains_and_descriptor_order() {
    let report = import_fixture(Fixture::Rich);
    let primitive = first_primitive(&report);
    let layout = list_vertices(primitive).unwrap();
    let expected_keys = [
        (AttributeSemanticRef::Position, AttributeDomain::Vertex),
        (AttributeSemanticRef::Normal, AttributeDomain::Vertex),
        (AttributeSemanticRef::Normal, AttributeDomain::Corner),
        (AttributeSemanticRef::Texcoord(0), AttributeDomain::Vertex),
        (AttributeSemanticRef::Texcoord(0), AttributeDomain::Corner),
        (AttributeSemanticRef::Color(1), AttributeDomain::Face),
        (AttributeSemanticRef::SmoothingGroup, AttributeDomain::Face),
        (
            AttributeSemanticRef::Custom("test:face-signed"),
            AttributeDomain::Face,
        ),
        (
            AttributeSemanticRef::Custom("test:implicit"),
            AttributeDomain::Vertex,
        ),
        (
            AttributeSemanticRef::Custom("test:signed"),
            AttributeDomain::Vertex,
        ),
        (
            AttributeSemanticRef::Custom("test:unsigned"),
            AttributeDomain::Corner,
        ),
        (
            AttributeSemanticRef::Custom("test:wide"),
            AttributeDomain::Face,
        ),
    ];
    assert_eq!(layout.attributes().len(), expected_keys.len());
    for (stream, (semantic, domain)) in layout.attributes().zip(expected_keys) {
        let source = primitive.attribute(semantic, domain).unwrap();
        assert_eq!(stream.attribute().semantic(), semantic);
        assert_eq!(stream.attribute().domain(), domain);
        assert_eq!(stream.attribute().metadata_owner(), source.metadata_owner());
        assert_eq!(stream.attribute().data(), source.data());
        assert_eq!(stream.layout().primitive().id(), primitive.id());
        assert_eq!(stream.len(), 6);
        assert!(!stream.is_empty());
    }
    let position = layout
        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
        .unwrap();
    let normal_vertex = layout
        .attribute(AttributeSemanticRef::Normal, AttributeDomain::Vertex)
        .unwrap();
    let normal_corner = layout
        .attribute(AttributeSemanticRef::Normal, AttributeDomain::Corner)
        .unwrap();
    let uv_vertex = layout
        .attribute(AttributeSemanticRef::Texcoord(0), AttributeDomain::Vertex)
        .unwrap();
    let uv_corner = layout
        .attribute(AttributeSemanticRef::Texcoord(0), AttributeDomain::Corner)
        .unwrap();
    let colors = layout
        .attribute(AttributeSemanticRef::Color(1), AttributeDomain::Face)
        .unwrap();
    let expected_positions = [
        [0.0, -0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let expected_uvs = [
        [0.0, -0.0],
        [0.5, -0.0],
        [0.0, -0.0],
        [0.0, -0.0],
        [0.75, 1.0],
        [0.0, -0.0],
    ];
    for draw in 0..6 {
        assert_f32(
            f32_values(position.element(draw).unwrap()),
            &expected_positions[draw],
        );
        assert_f32(
            f32_values(normal_vertex.element(draw).unwrap()),
            &[1.0, 0.0, 0.0],
        );
        assert_f32(
            f32_values(normal_corner.element(draw).unwrap()),
            if draw < 3 {
                &[0.0, 0.0, 1.0]
            } else {
                &[0.0, 1.0, 0.0]
            },
        );
        assert_f32(
            f32_values(uv_corner.element(draw).unwrap()),
            &expected_uvs[draw],
        );
        assert_f32(
            f32_values(colors.element(draw).unwrap()),
            if draw < 3 {
                &[-0.0, 0.25, 0.5, 1.0]
            } else {
                &[1.0, 0.5, 0.25, -0.0]
            },
        );
    }
    assert_f32(f32_values(uv_vertex.element(1).unwrap()), &[1.0, 0.0]);
    assert_f32(f32_values(uv_vertex.element(5).unwrap()), &[1.0, 0.0]);
    assert_eq!(position.attribute().len(), 5);
    assert_ne!(
        position.layout().vertex(0).unwrap().source_vertex(),
        position.layout().vertex(3).unwrap().source_vertex()
    );
    assert!(
        position
            .layout()
            .vertices()
            .all(|v| v.source_vertex().index != 4)
    );
    assert!(
        layout
            .attribute(AttributeSemanticRef::Normal, AttributeDomain::Face)
            .is_none()
    );
    assert!(
        layout
            .attribute(AttributeSemanticRef::Position, AttributeDomain::Instance)
            .is_none()
    );
    assert!(
        layout
            .attribute(
                AttributeSemanticRef::Custom("test:missing"),
                AttributeDomain::Corner
            )
            .is_none()
    );
}

#[test]
fn all_dense_and_sparse_scalar_kinds_keep_core_bits_and_original_slices() {
    let mut kinds = 0_u8;
    for fixture in [Fixture::Rich, Fixture::Streams] {
        let report = import_fixture(fixture);
        let primitive = first_primitive(&report);
        let layout = list_vertices(primitive).unwrap();
        for stream in layout.attributes() {
            let attribute = stream.attribute();
            kinds |= match attribute.data() {
                AttributeData::DenseF32 { .. } => 1,
                AttributeData::DenseF64 { .. } => 2,
                AttributeData::DenseU32 { .. } => 4,
                AttributeData::DenseI32 { .. } => 8,
                AttributeData::SparseF32 { .. } => 16,
                AttributeData::SparseF64 { .. } => 32,
                AttributeData::SparseU32 { .. } => 64,
                AttributeData::SparseI32 { .. } => 128,
            };
            for draw in [5, 0, 4, 1, 3, 2] {
                let source = match attribute.domain() {
                    AttributeDomain::Vertex => [0, 1, 2, 3, 2, 1][draw],
                    AttributeDomain::Corner => draw,
                    AttributeDomain::Face => [0, 0, 0, 1, 1, 1][draw],
                    AttributeDomain::Instance => unreachable!(),
                };
                let expected = attribute.element(source).unwrap();
                assert_same_element(stream.element(draw).unwrap(), expected);
                let tuple = layout
                    .vertex(draw)
                    .unwrap()
                    .attribute(attribute.semantic(), attribute.domain())
                    .unwrap();
                assert_same_element(tuple.element(), expected);
            }
            assert!(stream.element(6).is_none());
            assert!(stream.element(usize::MAX).is_none());
        }
    }
    assert_eq!(kinds, u8::MAX);
}

#[test]
fn sparse_f64_maximum_lanes_keep_equal_explicit_and_default_storage_distinct() {
    let report = import_fixture(Fixture::Streams);
    let layout = list_vertices(first_primitive(&report)).unwrap();
    let stream = layout
        .attribute(
            AttributeSemanticRef::Custom("test:wide"),
            AttributeDomain::Corner,
        )
        .unwrap();
    let AttributeData::SparseF64 {
        lanes,
        indices,
        values,
        default,
        ..
    } = stream.attribute().data()
    else {
        panic!("sparse F64 fixture")
    };
    assert_eq!(lanes, 16);
    assert_eq!(indices, [1, 4]);
    for draw in [0, 1, 5, 4, 2, 3, 1] {
        let AttributeElement::F64(actual) = stream.element(draw).unwrap() else {
            panic!("F64")
        };
        let expected = if draw == 4 {
            &WIDE_EXPLICIT[16..]
        } else {
            &WIDE_DEFAULT
        };
        assert_eq!(actual.len(), 16);
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
        assert_eq!(
            actual.as_ptr(),
            match draw {
                1 => values.as_ptr(),
                4 => values[16..].as_ptr(),
                _ => default.as_ptr(),
            }
        );
    }
    let AttributeElement::F64(implicit) = stream.element(0).unwrap() else {
        panic!("F64")
    };
    let AttributeElement::F64(explicit) = stream.element(1).unwrap() else {
        panic!("F64")
    };
    assert_eq!(implicit, explicit);
    assert_ne!(implicit.as_ptr(), explicit.as_ptr());
    let vertex_wide = layout
        .attribute(
            AttributeSemanticRef::Custom("test:wide"),
            AttributeDomain::Vertex,
        )
        .unwrap();
    let AttributeData::SparseF64 {
        values, default, ..
    } = vertex_wide.attribute().data()
    else {
        panic!("sparse F64")
    };
    for draw in [3, 5, 0, 4, 1, 2, 3] {
        let AttributeElement::F64(actual) = vertex_wide.element(draw).unwrap() else {
            panic!("F64")
        };
        let expected = if draw == 3 {
            &WIDE_EXPLICIT[16..]
        } else {
            &WIDE_DEFAULT
        };
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
        assert_eq!(
            actual.as_ptr(),
            match draw {
                1 | 5 => values.as_ptr(),
                3 => values[16..].as_ptr(),
                _ => default.as_ptr(),
            }
        );
    }
    let unsigned = layout
        .attribute(
            AttributeSemanticRef::Custom("test:unsigned"),
            AttributeDomain::Vertex,
        )
        .unwrap();
    for (draw, expected) in [0, u32::MAX, 0x8000_0001, 7, 0x8000_0001, u32::MAX]
        .into_iter()
        .enumerate()
    {
        assert!(
            matches!(unsigned.element(draw), Some(AttributeElement::U32(actual)) if actual == [expected])
        );
    }
}

#[test]
fn bound_iterators_are_exact_independent_double_ended_and_own_temporary_views() {
    let report = import_fixture(Fixture::Rich);
    let primitive = first_primitive(&report);
    let stream = list_vertices(primitive)
        .unwrap()
        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
        .unwrap();
    let mut elements = stream.elements();
    assert_eq!(elements.len(), 6);
    assert_eq!(elements.size_hint(), (6, Some(6)));
    assert_f32(f32_values(elements.next().unwrap()), &[0.0, -0.0, 0.0]);
    assert_f32(f32_values(elements.next_back().unwrap()), &[1.0, 0.0, 0.0]);
    let mut clone = elements.clone();
    assert_f32(f32_values(elements.nth(1).unwrap()), &[0.0, 1.0, 0.0]);
    assert_eq!(elements.len(), 2);
    assert_f32(f32_values(clone.nth_back(1).unwrap()), &[0.0, -0.0, 0.0]);
    assert_eq!(clone.len(), 2);
    assert_f32(f32_values(clone.next().unwrap()), &[1.0, 0.0, 0.0]);
    assert_f32(f32_values(elements.next_back().unwrap()), &[0.0, 1.0, 0.0]);
    assert!(elements.nth(usize::MAX).is_none());
    assert_eq!(elements.len(), 0);
    assert!(elements.next().is_none());
    assert!(elements.next_back().is_none());
    let mut fields = list_vertices(primitive).unwrap().attributes();
    assert_eq!(fields.len(), 12);
    assert_eq!(
        fields.next().unwrap().attribute().semantic(),
        AttributeSemanticRef::Position
    );
    let mut cloned_fields = fields.clone();
    assert_eq!(
        fields.next_back().unwrap().attribute().semantic(),
        AttributeSemanticRef::Custom("test:wide")
    );
    assert_eq!(
        cloned_fields.next().unwrap().attribute().domain(),
        AttributeDomain::Vertex
    );
    assert_eq!(fields.len(), 10);
    assert_eq!(cloned_fields.len(), 10);
}

fn temporary_stream<'scene>(primitive: PrimitiveRef<'scene>) -> ListAttribute<'scene> {
    list_vertices(primitive)
        .unwrap()
        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
        .unwrap()
}

fn temporary_element<'scene>(primitive: PrimitiveRef<'scene>) -> AttributeElement<'scene> {
    temporary_stream(primitive).element(1).unwrap()
}

fn temporary_elements<'scene>(
    primitive: PrimitiveRef<'scene>,
) -> impl ExactSizeIterator<Item = AttributeElement<'scene>> + DoubleEndedIterator + Clone + 'scene
{
    temporary_stream(primitive).elements()
}

fn temporary_descriptor_element<'scene>(
    primitive: PrimitiveRef<'scene>,
) -> AttributeElement<'scene> {
    let attribute = primitive.positions().unwrap();
    baozi_core::AttributeRef::element(&attribute, 1).unwrap()
}

#[test]
fn core_borrowed_reader_keeps_scene_lifetime_after_temporary_descriptor_drop() {
    let report = import_fixture(Fixture::Rich);
    let primitive = first_primitive(&report);
    let element = temporary_descriptor_element(primitive);
    let attribute = primitive.positions().unwrap();
    assert_eq!(baozi_core::AttributeRef::len(&attribute), 5);
    assert_eq!(
        baozi_core::AttributeRef::domain(&attribute),
        AttributeDomain::Vertex
    );
    let AttributeData::DenseF32 { values, .. } = attribute.data() else {
        panic!("F32")
    };
    assert_f32(f32_values(element), &[1.0, 0.0, 0.0]);
    assert!(std::ptr::eq(f32_values(element), &values[3..6]));
}

#[test]
fn elements_and_iterators_borrow_scene_after_temporary_streams_drop() {
    let report = import_fixture(Fixture::Rich);
    let primitive = first_primitive(&report);
    let element = temporary_element(primitive);
    let AttributeData::DenseF32 { values, .. } = primitive.positions().unwrap().data() else {
        panic!("F32")
    };
    assert_f32(f32_values(element), &[1.0, 0.0, 0.0]);
    assert!(std::ptr::eq(f32_values(element), &values[3..6]));
    let mut elements = temporary_elements(primitive);
    assert_f32(f32_values(elements.nth(3).unwrap()), &[0.0, -0.0, 0.0]);
    assert_f32(f32_values(elements.next_back().unwrap()), &[1.0, 0.0, 0.0]);
}

#[test]
fn streams_keep_primitive_generation_and_tangent_declarations_without_rebinding() {
    let report = import_fixture(Fixture::Streams);
    let foreign = import_fixture(Fixture::Streams);
    let first = first_primitive(&report);
    let second = report
        .scene()
        .meshes()
        .nth(1)
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let other = first_primitive(&foreign);
    let a = list_vertices(first)
        .unwrap()
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
        .unwrap();
    let b = list_vertices(second)
        .unwrap()
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
        .unwrap();
    let c = list_vertices(other)
        .unwrap()
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
        .unwrap();
    for (stream, primitive) in [(a, first), (b, second), (c, other)] {
        assert_eq!(stream.layout().primitive().id(), primitive.id());
        assert_eq!(
            stream.attribute().metadata_owner(),
            primitive
                .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
                .unwrap()
                .metadata_owner()
        );
        assert_eq!(
            stream.attribute().tangent_conventions(),
            Some(TangentConventionsRef {
                basis: TangentBasisRef::MikkTspace,
                encoding: TangentEncodingRef::SignInW,
            })
        );
        assert_f32(
            f32_values(stream.element(0).unwrap()),
            &[1.0, -0.0, 0.0, -1.0],
        );
    }
    assert_ne!(a.layout().primitive().id(), b.layout().primitive().id());
    assert_ne!(
        a.layout().primitive().id().generation(),
        c.layout().primitive().id().generation()
    );
    assert_ne!(
        a.attribute().metadata_owner(),
        b.attribute().metadata_owner()
    );
    assert_ne!(
        a.attribute().metadata_owner(),
        c.attribute().metadata_owner()
    );
}

#[test]
fn binding_random_sampling_enumeration_and_early_drop_allocate_nothing_and_keep_scene() {
    for fixture in [Fixture::Rich, Fixture::Streams] {
        let report = import_fixture(fixture);
        let scene = report.scene();
        let limits = SceneSnapshotLimits::try_new(32 * 1024, 128, 1_000_000).unwrap();
        let before = scene.capture_snapshot(limits).unwrap();
        let generation = scene.generation();
        let allocations = allocation_counter::measure(|| {
            for primitive in scene.meshes().flat_map(|mesh| mesh.primitives()) {
                let layout = list_vertices(primitive).unwrap();
                let position = layout
                    .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
                    .unwrap();
                for draw in [5, 1, 0, 4, 3, 2] {
                    std::hint::black_box(position.element(draw).unwrap());
                }
                assert!(position.element(layout.len()).is_none());
                assert!(position.element(usize::MAX).is_none());
                assert!(
                    layout
                        .attribute(AttributeSemanticRef::Texcoord(77), AttributeDomain::Corner)
                        .is_none()
                );
                for stream in layout.attributes() {
                    for element in stream.elements().rev() {
                        std::hint::black_box(element);
                    }
                    let mut partial = stream.elements();
                    std::hint::black_box(partial.next());
                }
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
        assert_eq!(scene.generation(), generation);
        assert_eq!(scene.capture_snapshot(limits).unwrap(), before);
        assert!(report.evidence().matches_scene(scene));
    }
}
