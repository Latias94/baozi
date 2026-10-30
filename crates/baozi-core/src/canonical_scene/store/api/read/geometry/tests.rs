use super::*;

fn attribute(data: AttributeData<'_>, count: u32) -> AttributeRef<'_> {
    let lanes = match data {
        AttributeData::DenseF32 { lanes, .. }
        | AttributeData::DenseF64 { lanes, .. }
        | AttributeData::DenseU32 { lanes, .. }
        | AttributeData::DenseI32 { lanes, .. }
        | AttributeData::SparseF32 { lanes, .. }
        | AttributeData::SparseU32 { lanes, .. }
        | AttributeData::SparseI32 { lanes, .. } => lanes,
    };
    AttributeRef {
        semantic: AttributeSemanticRef::Custom("test:element"),
        domain: AttributeDomain::Vertex,
        lanes,
        count,
        data,
    }
}

#[test]
fn dense_elements_borrow_all_four_scalar_types_without_changing_bits() {
    let floats = [-0.0, f32::from_bits(1), 0.5, 1.0, -2.0, f32::MAX];
    let doubles = [
        -0.0,
        f64::from_bits(1),
        f64::from_bits(0x3ff0_0000_0000_0001),
        f64::MAX,
    ];
    let unsigned = [0, u32::MAX];
    let signed = [i32::MIN, -1, 0, i32::MAX];
    let f32s = attribute(
        AttributeData::DenseF32 {
            lanes: 3,
            values: &floats,
        },
        2,
    );
    let f64s = attribute(
        AttributeData::DenseF64 {
            lanes: 2,
            values: &doubles,
        },
        2,
    );
    let u32s = attribute(
        AttributeData::DenseU32 {
            lanes: 1,
            values: &unsigned,
        },
        2,
    );
    let i32s = attribute(
        AttributeData::DenseI32 {
            lanes: 2,
            values: &signed,
        },
        2,
    );

    let Some(AttributeElement::F32(first)) = f32s.element(0) else {
        panic!("F32 element")
    };
    let Some(AttributeElement::F32(last)) = f32s.element(1) else {
        panic!("F32 element")
    };
    assert!(std::ptr::eq(first, &floats[..3]));
    assert!(std::ptr::eq(last, &floats[3..]));
    assert_eq!(
        [first[0].to_bits(), first[1].to_bits()],
        [(-0.0_f32).to_bits(), 1]
    );

    let Some(AttributeElement::F64(first)) = f64s.element(0) else {
        panic!("F64 element")
    };
    let Some(AttributeElement::F64(last)) = f64s.element(1) else {
        panic!("F64 element")
    };
    assert!(std::ptr::eq(first, &doubles[..2]));
    assert!(std::ptr::eq(last, &doubles[2..]));
    assert_eq!(
        [first[0].to_bits(), first[1].to_bits()],
        [(-0.0_f64).to_bits(), 1]
    );
    assert_eq!(last[0].to_bits(), 0x3ff0_0000_0000_0001);
    assert!(
        matches!(u32s.element(1), Some(AttributeElement::U32(value)) if std::ptr::eq(value, &unsigned[1..]))
    );
    assert!(
        matches!(i32s.element(0), Some(AttributeElement::I32(value)) if std::ptr::eq(value, &signed[..2]))
    );
    assert!(
        matches!(i32s.element(1), Some(AttributeElement::I32(value)) if std::ptr::eq(value, &signed[2..]))
    );
    for value in [f32s, f64s, u32s, i32s] {
        assert_eq!(value.element(value.len()), None);
        assert_eq!(value.element(usize::MAX), None);
    }
}

#[test]
fn sparse_elements_borrow_explicit_or_default_sources_for_all_three_scalar_types() {
    let indices = [0, 2, 5];
    let floats = [0.0, -0.0, 0.25, -0.5, f32::from_bits(1), 1.0];
    let float_default = [0.0, -0.0];
    let unsigned = [0, 9, u32::MAX];
    let unsigned_default = [0];
    let signed = [0, i32::MIN, i32::MAX];
    let signed_default = [0];
    let f32s = attribute(
        AttributeData::SparseF32 {
            lanes: 2,
            logical_count: 7,
            indices: &indices,
            values: &floats,
            default: &float_default,
        },
        7,
    );
    let u32s = attribute(
        AttributeData::SparseU32 {
            lanes: 1,
            logical_count: 7,
            indices: &indices,
            values: &unsigned,
            default: &unsigned_default,
        },
        7,
    );
    let i32s = attribute(
        AttributeData::SparseI32 {
            lanes: 1,
            logical_count: 7,
            indices: &indices,
            values: &signed,
            default: &signed_default,
        },
        7,
    );

    for (index, explicit) in [(0, 0), (2, 1), (5, 2)] {
        assert!(
            matches!(f32s.element(index), Some(AttributeElement::F32(value)) if std::ptr::eq(value, &floats[explicit * 2..explicit * 2 + 2]))
        );
        assert!(
            matches!(u32s.element(index), Some(AttributeElement::U32(value)) if std::ptr::eq(value, &unsigned[explicit..explicit + 1]))
        );
        assert!(
            matches!(i32s.element(index), Some(AttributeElement::I32(value)) if std::ptr::eq(value, &signed[explicit..explicit + 1]))
        );
    }
    for index in [1, 3, 4, 6] {
        assert!(
            matches!(f32s.element(index), Some(AttributeElement::F32(value)) if std::ptr::eq(value, &float_default[..]))
        );
        assert!(
            matches!(u32s.element(index), Some(AttributeElement::U32(value)) if std::ptr::eq(value, &unsigned_default[..]))
        );
        assert!(
            matches!(i32s.element(index), Some(AttributeElement::I32(value)) if std::ptr::eq(value, &signed_default[..]))
        );
    }
    // Equal explicit/default zeros remain distinguishable through the original data slices.
    let Some(AttributeElement::F32(explicit)) = f32s.element(0) else {
        panic!("explicit zero")
    };
    let Some(AttributeElement::F32(default)) = f32s.element(1) else {
        panic!("default zero")
    };
    assert_eq!(explicit, default);
    assert!(!std::ptr::eq(explicit, default));
    assert_eq!(default[1].to_bits(), (-0.0_f32).to_bits());
    let AttributeData::SparseF32 {
        indices: retained_indices,
        values,
        default,
        ..
    } = f32s.data()
    else {
        panic!("sparse representation")
    };
    assert!(std::ptr::eq(retained_indices, &indices[..]));
    assert!(std::ptr::eq(values, &floats[..]));
    assert!(std::ptr::eq(default, &float_default[..]));
    for value in [f32s, u32s, i32s] {
        assert_eq!(value.element(7), None);
        assert_eq!(value.element(usize::MAX), None);
    }
}

#[test]
fn maximum_lane_elements_use_the_complete_tuple_in_dense_and_sparse_data() {
    let values: [i32; 48] = std::array::from_fn(|index| index as i32 - 24);
    let default = [i32::MIN; 16];
    let indices = [0, 1, 3];
    let dense = attribute(
        AttributeData::DenseI32 {
            lanes: 16,
            values: &values,
        },
        3,
    );
    let sparse = attribute(
        AttributeData::SparseI32 {
            lanes: 16,
            logical_count: 4,
            indices: &indices,
            values: &values,
            default: &default,
        },
        4,
    );
    assert!(
        matches!(dense.element(2), Some(AttributeElement::I32(value)) if std::ptr::eq(value, &values[32..48]))
    );
    assert!(
        matches!(sparse.element(3), Some(AttributeElement::I32(value)) if std::ptr::eq(value, &values[32..48]))
    );
    assert!(
        matches!(sparse.element(2), Some(AttributeElement::I32(value)) if std::ptr::eq(value, &default[..]))
    );
    assert_eq!(dense.element(3), None);
    assert_eq!(sparse.element(4), None);
    assert_eq!(dense.element(usize::MAX), None);
    assert_eq!(sparse.element(usize::MAX), None);
}

#[test]
fn empty_attributes_and_default_only_sparse_data_observe_the_logical_range() {
    let float_default = [-0.0; 3];
    let unsigned_default = [u32::MAX];
    let signed_default = [i32::MIN];
    let empty = [
        AttributeData::DenseF32 {
            lanes: 3,
            values: &[],
        },
        AttributeData::DenseF64 {
            lanes: 1,
            values: &[],
        },
        AttributeData::DenseU32 {
            lanes: 1,
            values: &[],
        },
        AttributeData::DenseI32 {
            lanes: 1,
            values: &[],
        },
        AttributeData::SparseF32 {
            lanes: 3,
            logical_count: 0,
            indices: &[],
            values: &[],
            default: &float_default,
        },
        AttributeData::SparseU32 {
            lanes: 1,
            logical_count: 0,
            indices: &[],
            values: &[],
            default: &unsigned_default,
        },
        AttributeData::SparseI32 {
            lanes: 1,
            logical_count: 0,
            indices: &[],
            values: &[],
            default: &signed_default,
        },
    ];
    for data in empty {
        let value = attribute(data, 0);
        assert_eq!(value.element(0), None);
        assert_eq!(value.element(usize::MAX), None);
    }
    let defaults = attribute(
        AttributeData::SparseF32 {
            lanes: 3,
            logical_count: u32::MAX,
            indices: &[],
            values: &[],
            default: &float_default,
        },
        u32::MAX,
    );
    let last = u32::MAX as usize - 1;
    assert!(
        matches!(defaults.element(last), Some(AttributeElement::F32(value)) if std::ptr::eq(value, &float_default[..]))
    );
    assert_eq!(defaults.element(u32::MAX as usize), None);
    assert_eq!(defaults.element(usize::MAX), None);
}

#[test]
fn element_borrows_outlive_temporary_attribute_views() {
    fn borrow(values: &[f64]) -> &[f64] {
        let Some(AttributeElement::F64(value)) =
            attribute(AttributeData::DenseF64 { lanes: 2, values }, 2).element(1)
        else {
            panic!("borrowed F64 element")
        };
        value
    }
    let values = [1.0, -0.0, f64::from_bits(1), f64::MAX];
    let element = borrow(&values);
    assert!(std::ptr::eq(element, &values[2..]));
}

#[test]
fn dense_and_sparse_element_reads_allocate_nothing() {
    let floats = [-0.0, 0.5, 1.0, -1.0];
    let doubles = [-0.0, f64::from_bits(0x3ff0_0000_0000_0001)];
    let unsigned = [u32::MAX, 0];
    let signed = [i32::MIN, i32::MAX];
    let indices = [1];
    let attributes = [
        attribute(
            AttributeData::DenseF32 {
                lanes: 2,
                values: &floats,
            },
            2,
        ),
        attribute(
            AttributeData::DenseF64 {
                lanes: 1,
                values: &doubles,
            },
            2,
        ),
        attribute(
            AttributeData::DenseU32 {
                lanes: 1,
                values: &unsigned,
            },
            2,
        ),
        attribute(
            AttributeData::DenseI32 {
                lanes: 1,
                values: &signed,
            },
            2,
        ),
        attribute(
            AttributeData::SparseF32 {
                lanes: 2,
                logical_count: 3,
                indices: &indices,
                values: &floats[..2],
                default: &floats[2..],
            },
            3,
        ),
        attribute(
            AttributeData::SparseU32 {
                lanes: 1,
                logical_count: 3,
                indices: &indices,
                values: &unsigned[..1],
                default: &unsigned[1..],
            },
            3,
        ),
        attribute(
            AttributeData::SparseI32 {
                lanes: 1,
                logical_count: 3,
                indices: &indices,
                values: &signed[..1],
                default: &signed[1..],
            },
            3,
        ),
    ];
    let allocations = allocation_counter::measure(|| {
        for value in attributes {
            for index in 0..value.len() {
                assert!(std::hint::black_box(value.element(index)).is_some());
            }
            assert!(std::hint::black_box(value.element(value.len())).is_none());
            assert!(std::hint::black_box(value.element(usize::MAX)).is_none());
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}
