use super::*;
use crate::appearance::{
    ClearcoatFactors, EmissionFactors, SheenFactors, ThicknessFactors, TransmissionFactors,
};
use crate::{AttributeData, AttributeSemanticRef, ValidatedScene};

#[test]
fn material_factor_presence_and_strength_have_independent_complete_snapshot_framing() {
    let factory = factory();
    let account = account(&factory);
    let empty = MaterialFactors::empty();
    let cases = [
        (empty, [0, 0, 0, 0, 0], 0.0_f32, None),
        (
            empty.with_emission(EmissionFactors::try_new(Vec3::ZERO, Some(0.0)).unwrap()),
            [1, 0, 0, 0, 0],
            0.0,
            None,
        ),
        (
            empty.with_emission(EmissionFactors::try_new(Vec3::ZERO, Some(-0.0)).unwrap()),
            [1, 0, 0, 0, 0],
            -0.0,
            None,
        ),
        (
            empty.with_emission(EmissionFactors::try_new(Vec3::ZERO, Some(1.0)).unwrap()),
            [1, 0, 0, 0, 0],
            1.0,
            None,
        ),
        (
            empty.with_clearcoat(Some(ClearcoatFactors::default())),
            [0, 1, 0, 0, 0],
            0.0,
            None,
        ),
        (
            empty.with_transmission(Some(TransmissionFactors::default())),
            [0, 0, 1, 0, 0],
            0.0,
            None,
        ),
        (
            empty.with_thickness(Some(ThicknessFactors::default())),
            [0, 0, 0, 1, 0],
            0.0,
            None,
        ),
        (
            empty.with_sheen(Some(SheenFactors::default())),
            [0, 0, 0, 0, 1],
            0.0,
            None,
        ),
        (
            empty.with_thickness(Some(ThicknessFactors::try_new(-0.0).unwrap())),
            [0, 0, 0, 1, 0],
            0.0,
            Some(34),
        ),
    ];
    let mut digests = Vec::new();
    for (factors, presence, strength, negative_zero_slot) in cases {
        let materials = [MaterialAuthoring::new(
            AuthoringRange::new(0, 0),
            MaterialSemantics::new(ShadingModel::Unlit, AlphaMode::Opaque, false, factors),
        )];
        let parts = build_scene_with_appearance_and_geometry(
            &account,
            &[],
            &[],
            AppearanceAuthoringBatch::new(&[], &[], &[], &[], &materials),
            GeometryAuthoringBatch::empty(),
        );
        let (full, transcript) = capture_node_scene_with_test_transcript(
            &parts.scene,
            snapshot_limits(16_384, 64),
            None,
        )
        .unwrap();
        assert_eq!(full.schema_revision, 8);
        assert!(transcript.complete);

        // Independent material-row encoding fixes existing tags, new presence tags, and width.
        let mut expected = vec![17, 13];
        expected.extend_from_slice(&0_u64.to_le_bytes());
        expected.extend_from_slice(&[142, 0, 143, 0, 144]);
        expected.extend_from_slice(&0_u32.to_le_bytes());
        expected.extend_from_slice(&[145, 0, 146, 0, 147, 0, 148, 0]);
        for (tag, present) in [229, 230, 231, 232, 233].into_iter().zip(presence) {
            expected.extend_from_slice(&[tag, present]);
        }
        for tag in [72, 11, 149] {
            expected.push(tag);
            expected.extend_from_slice(&0_u64.to_le_bytes());
        }
        expected.extend_from_slice(&[150, 11]);
        expected.extend_from_slice(&40_u64.to_le_bytes());
        for slot in 0..40 {
            let value = if slot == 39 {
                strength
            } else if negative_zero_slot == Some(slot) {
                -0.0
            } else {
                0.0
            };
            expected.push(165);
            expected.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        expected.extend_from_slice(&[166, 12, 0]);
        assert!(
            transcript
                .bytes
                .windows(expected.len())
                .any(|row| row == expected)
        );
        assert!(full.preview.text.contains("factors(present,total=40)="));
        for (name, present) in [
            "emissive_strength",
            "clearcoat",
            "transmission",
            "thickness",
            "sheen",
        ]
        .into_iter()
        .zip(presence)
        {
            let state = if present == 1 { "present" } else { "absent" };
            assert!(full.preview.text.contains(&format!("{name}={state}")));
        }
        let zero = capture_node_scene(&parts.scene, snapshot_limits(0, 0), None).unwrap();
        let bounded = capture_node_scene(&parts.scene, snapshot_limits(64, 1), None).unwrap();
        assert_eq!(full.digest, zero.digest);
        assert_eq!(full.digest, bounded.digest);
        assert!(zero.preview.text.is_empty());
        assert!(!digests.contains(&full.digest));
        digests.push(full.digest);
        let scene = ValidatedScene::from_node_scene(parts.scene);
        let actual = scene.materials().next().unwrap().factors();
        assert_eq!(
            actual.emission().strength().map(f32::to_bits),
            factors.emission().strength().map(f32::to_bits)
        );
        assert_eq!(actual.clearcoat(), factors.clearcoat());
        assert_eq!(actual.transmission(), factors.transmission());
        assert_eq!(
            actual.thickness().map(|value| value.thickness().to_bits()),
            factors.thickness().map(|value| value.thickness().to_bits())
        );
        assert_eq!(actual.sheen(), factors.sheen());
        drop(scene);
        drop(parts.report);
    }
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

fn f64_builtin_scene(
    account: &PhysicalStorageAccount,
    attribute: GeometryAttributeAuthoring<'_>,
) -> NodeSceneSuccessParts {
    let positions = [0.0_f32; 12];
    let is_position = *attribute.semantic() == AttributeSemantic::Position;
    let attribute = if *attribute.semantic() == AttributeSemantic::Tangent {
        attribute.with_tangent_conventions(TangentConventions {
            basis: TangentBasisConvention::TriangleUvDerivative,
            encoding: TangentHandednessEncoding::SignInW,
        })
    } else {
        attribute
    };
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        attribute,
    ];
    let attributes = if is_position {
        &attributes[1..]
    } else {
        &attributes[..]
    };
    let indices = [0, 1, 2, 0, 2, 3];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Present(&indices),
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, attributes.len() as u32),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    build_scene_with_geometry(
        account,
        &[],
        &[],
        GeometryAuthoringBatch::new(&meshes, &primitives, attributes, &[]),
    )
}

fn assert_dense_f64_builtin_bits(semantic: AttributeSemantic, domain: AttributeDomain, lanes: u8) {
    let factory = factory();
    let account = account(&factory);
    let element_count = if domain == AttributeDomain::Vertex {
        4
    } else {
        6
    };
    let scalar_count = element_count * usize::from(lanes);
    let samples = [
        16_777_217.0_f64,
        16_777_216.0,
        -0.0,
        f64::from_bits(1),
        f64::MAX,
        f64::from_bits(0x3ff0_0000_0000_0001),
    ];
    let expected: [f64; 24] = std::array::from_fn(|index| samples[index % samples.len()]);
    let expected = &expected[..scalar_count];
    let mut authored = [0.0_f64; 24];
    authored[..scalar_count].copy_from_slice(expected);
    let parts = f64_builtin_scene(
        &account,
        GeometryAttributeAuthoring::f64(semantic.clone(), domain, lanes, &authored[..scalar_count]),
    );
    authored.fill(0.0);

    let full = capture_node_scene(&parts.scene, snapshot_limits(32_768, 128), None)
        .expect("dense F64 built-in snapshot");
    let bounded = capture_node_scene(&parts.scene, snapshot_limits(0, 0), None)
        .expect("dense F64 built-in snapshot without preview");
    assert_eq!(full.digest, bounded.digest);
    assert_eq!(full.totals.f64_scalar_count, scalar_count as u64);
    for value in expected {
        assert!(
            full.preview
                .text
                .contains(&format!("0x{:016x}", value.to_bits()))
        );
    }

    let scene = ValidatedScene::from_node_scene(parts.scene);
    let allocations = allocation_counter::measure(|| {
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(primitive.vertex_count(), 4);
        assert_eq!(primitive.corner_count(), 6);
        let attribute = match semantic {
            AttributeSemantic::Position => primitive.positions().unwrap(),
            AttributeSemantic::Normal => primitive.normals(domain).unwrap(),
            AttributeSemantic::Tangent => primitive
                .attribute(AttributeSemanticRef::Tangent, domain)
                .unwrap(),
            AttributeSemantic::Texcoord(set) => primitive.texcoords(set, domain).unwrap(),
            _ => panic!("precision contract requires a floating-point geometry built-in"),
        };
        assert_eq!(attribute.domain(), domain);
        assert_eq!(attribute.lanes(), lanes);
        assert_eq!(attribute.len(), element_count);
        let AttributeData::DenseF64 {
            lanes: stored_lanes,
            values,
        } = attribute.data()
        else {
            panic!("the authored built-in must retain F64 storage");
        };
        assert_eq!(stored_lanes, lanes);
        assert_eq!(values.len(), expected.len());
        for (actual, expected) in values.iter().zip(expected) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
        for index in 0..element_count {
            let Some(crate::AttributeElement::F64(tuple)) = attribute.element(index) else {
                panic!("the logical built-in element must borrow F64 values");
            };
            let start = index * usize::from(lanes);
            assert!(std::ptr::eq(
                tuple,
                &values[start..start + usize::from(lanes)]
            ));
        }
        assert_eq!(attribute.element(element_count), None);
        assert_eq!(attribute.element(usize::MAX), None);
        if semantic == AttributeSemantic::Tangent {
            assert_eq!(
                attribute.tangent_conventions(),
                Some(crate::TangentConventionsRef {
                    basis: crate::TangentBasisRef::TriangleUvDerivative,
                    encoding: crate::TangentEncodingRef::SignInW,
                })
            );
        } else {
            assert_eq!(attribute.tangent_conventions(), None);
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);

    authored[..scalar_count].copy_from_slice(expected);
    authored[0] = 16_777_216.0;
    assert_eq!(expected[0] as f32, authored[0] as f32);
    let rounded = f64_builtin_scene(
        &account,
        GeometryAttributeAuthoring::f64(semantic.clone(), domain, lanes, &authored[..scalar_count]),
    );
    assert_ne!(
        full.digest,
        capture_node_scene(&rounded.scene, snapshot_limits(0, 0), None)
            .unwrap()
            .digest
    );
    drop(rounded);

    authored[..scalar_count].copy_from_slice(expected);
    authored[2] = 0.0;
    let positive_zero = f64_builtin_scene(
        &account,
        GeometryAttributeAuthoring::f64(semantic, domain, lanes, &authored[..scalar_count]),
    );
    assert_ne!(
        full.digest,
        capture_node_scene(&positive_zero.scene, snapshot_limits(0, 0), None)
            .unwrap()
            .digest
    );
    drop(positive_zero);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn precision_contract_dense_f64_position_retains_bits_in_public_reads_and_snapshots() {
    assert_dense_f64_builtin_bits(AttributeSemantic::Position, AttributeDomain::Vertex, 3);
}

#[test]
fn precision_contract_dense_f64_normal_retains_bits_in_vertex_and_corner_domains() {
    for domain in [AttributeDomain::Vertex, AttributeDomain::Corner] {
        assert_dense_f64_builtin_bits(AttributeSemantic::Normal, domain, 3);
    }
}

#[test]
fn precision_contract_dense_f64_tangent_retains_bits_and_attribute_conventions() {
    for domain in [AttributeDomain::Vertex, AttributeDomain::Corner] {
        assert_dense_f64_builtin_bits(AttributeSemantic::Tangent, domain, 4);
    }
}

#[test]
fn precision_contract_dense_f64_texcoord_retains_one_to_three_lanes_in_both_domains() {
    for domain in [AttributeDomain::Vertex, AttributeDomain::Corner] {
        for lanes in 1..=3 {
            assert_dense_f64_builtin_bits(AttributeSemantic::Texcoord(2), domain, lanes);
        }
    }
}

#[test]
fn precision_contract_sparse_f64_builtins_preserve_bits_defaults_and_public_borrows() {
    let factory = factory();
    let account = account(&factory);
    for (semantic, domain, lanes) in [
        (AttributeSemantic::Position, AttributeDomain::Vertex, 3),
        (AttributeSemantic::Normal, AttributeDomain::Vertex, 3),
        (AttributeSemantic::Normal, AttributeDomain::Corner, 3),
        (AttributeSemantic::Tangent, AttributeDomain::Vertex, 4),
        (AttributeSemantic::Tangent, AttributeDomain::Corner, 4),
        (AttributeSemantic::Texcoord(1), AttributeDomain::Vertex, 1),
        (AttributeSemantic::Texcoord(1), AttributeDomain::Vertex, 2),
        (AttributeSemantic::Texcoord(1), AttributeDomain::Vertex, 3),
        (AttributeSemantic::Texcoord(1), AttributeDomain::Corner, 1),
        (AttributeSemantic::Texcoord(1), AttributeDomain::Corner, 2),
        (AttributeSemantic::Texcoord(1), AttributeDomain::Corner, 3),
        (
            AttributeSemantic::Custom(CustomAttributeSemantic::new("test:f64").unwrap()),
            AttributeDomain::Face,
            2,
        ),
    ] {
        let count = match domain {
            AttributeDomain::Vertex => 4,
            AttributeDomain::Corner => 6,
            AttributeDomain::Face => 2,
            AttributeDomain::Instance => unreachable!(),
        };
        let lanes_usize = usize::from(lanes);
        let indices = [0, count - 1];
        let default = [-0.0_f64, f64::from_bits(1), 16_777_217.0, f64::MAX];
        let second = [16_777_217.0, f64::MAX, f64::from_bits(1), -0.0];
        let mut values = [0.0_f64; 8];
        values[..lanes_usize].copy_from_slice(&default[..lanes_usize]);
        values[lanes_usize..2 * lanes_usize].copy_from_slice(&second[..lanes_usize]);
        let expected = values;
        let mut authored_default = default;
        let parts = f64_builtin_scene(
            &account,
            GeometryAttributeAuthoring::sparse_f64(
                semantic.clone(),
                domain,
                lanes,
                count,
                &indices,
                &values[..2 * lanes_usize],
                &authored_default[..lanes_usize],
            ),
        );
        values.fill(0.0);
        authored_default.fill(0.0);
        let snapshot =
            capture_node_scene(&parts.scene, snapshot_limits(32_768, 128), None).unwrap();
        assert_eq!(snapshot.totals.f64_scalar_count, u64::from(lanes) * 3);
        assert!(snapshot.preview.text.contains("scalar=f64"));
        assert!(snapshot.preview.text.contains("payload=sparse"));
        assert!(snapshot.preview.text.contains("0x8000000000000000"));
        assert!(snapshot.preview.text.contains("0x4170000010000000"));
        assert_eq!(
            snapshot.digest,
            capture_node_scene(&parts.scene, snapshot_limits(0, 0), None)
                .unwrap()
                .digest
        );
        let exact_work =
            NodeSceneSnapshotLimits::try_new(32_768, 128, snapshot.totals.work_units).unwrap();
        assert_eq!(
            capture_node_scene(&parts.scene, exact_work, None)
                .unwrap()
                .digest,
            snapshot.digest
        );
        assert!(matches!(
            capture_node_scene(
                &parts.scene,
                NodeSceneSnapshotLimits::try_new(32_768, 128, snapshot.totals.work_units - 1)
                    .unwrap(),
                None,
            ),
            Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
        ));
        let scene = ValidatedScene::from_node_scene(parts.scene);
        let allocations = allocation_counter::measure(|| {
            let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
            let attribute = primitive.attributes().last().unwrap();
            assert_eq!(attribute.domain(), domain);
            assert_eq!(attribute.len(), count as usize);
            let AttributeData::SparseF64 {
                lanes: stored_lanes,
                logical_count,
                indices: stored_indices,
                values,
                default: stored_default,
            } = attribute.data()
            else {
                panic!("the authored sparse F64 representation must remain sparse");
            };
            assert_eq!(stored_lanes, lanes);
            assert_eq!(logical_count, count);
            assert_eq!(stored_indices, indices);
            for (actual, expected) in values.iter().zip(&expected[..2 * lanes_usize]) {
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
            for (actual, expected) in stored_default.iter().zip(&default[..lanes_usize]) {
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
            for index in 0..count as usize {
                let Some(crate::AttributeElement::F64(tuple)) = attribute.element(index) else {
                    panic!("sparse F64 logical tuple");
                };
                let expected = if index == 0 {
                    &values[..lanes_usize]
                } else if index == count as usize - 1 {
                    &values[lanes_usize..]
                } else {
                    stored_default
                };
                assert!(std::ptr::eq(tuple, expected));
            }
            assert!(!std::ptr::eq(&values[..lanes_usize], stored_default));
            assert_eq!(attribute.element(count as usize), None);
            assert_eq!(attribute.element(usize::MAX), None);
            if semantic == AttributeSemantic::Tangent {
                assert_eq!(
                    attribute.tangent_conventions(),
                    Some(crate::TangentConventionsRef {
                        basis: crate::TangentBasisRef::TriangleUvDerivative,
                        encoding: crate::TangentEncodingRef::SignInW,
                    })
                );
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        drop(scene);
        drop(parts.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}

#[test]
fn precision_contract_sparse_f64_snapshot_commits_defaults_and_explicit_presence() {
    let factory = factory();
    let account = account(&factory);
    let explicit = [-0.0_f64, f64::from_bits(1), 16_777_217.0];
    let mut digests = Vec::new();
    for (indices, values, default) in [
        (&[0][..], &explicit[..], &explicit[..]),
        (&[][..], &[][..], &explicit[..]),
        (
            &[0][..],
            &explicit[..],
            &[0.0, f64::from_bits(1), 16_777_217.0][..],
        ),
        (
            &[0][..],
            &explicit[..],
            &[-0.0, f64::from_bits(1), 16_777_216.0][..],
        ),
        (
            &[0][..],
            &[-0.0, f64::from_bits(1), 16_777_216.0][..],
            &explicit[..],
        ),
    ] {
        let parts = f64_builtin_scene(
            &account,
            GeometryAttributeAuthoring::sparse_f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                4,
                indices,
                values,
                default,
            ),
        );
        let (snapshot, transcript) = capture_node_scene_with_test_transcript(
            &parts.scene,
            snapshot_limits(32_768, 128),
            None,
        )
        .unwrap();
        assert!(transcript.complete);
        assert!(
            transcript
                .bytes
                .windows(9)
                .any(|bytes| bytes == [14, 0, 0, 0, 0, 0, 0, 0, 128])
        );
        assert!(!digests.contains(&snapshot.digest));
        digests.push(snapshot.digest);
    }
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn borrowed_readers_preserve_dense_sparse_and_presence_without_allocation() {
    let factory = factory();
    let account = account(&factory);
    let parts = all_attribute_payload_kinds_scene(&account);
    let scene = ValidatedScene::from_node_scene(parts.scene);

    let allocations = allocation_counter::measure(|| {
        for _ in 0..2 {
            let mesh = scene.meshes().next().unwrap();
            assert_eq!(mesh.primitives().len(), 3);
            let first = mesh.primitives().next().unwrap();
            let second = mesh.primitives().nth(1).unwrap();
            let third = mesh.primitives().next_back().unwrap();
            assert_eq!(third.indices(), None);
            assert_eq!(second.indices(), Some(&[0, 1, 2][..]));
            assert_eq!(second.polygon_face_sizes(), Some(&[3][..]));
            assert_eq!(
                first
                    .material_assignment()
                    .effective()
                    .unwrap()
                    .shading_model(),
                ShadingModel::Unlit
            );
            assert_eq!(first.attributes().len(), 2);
            assert_eq!(
                first.positions().unwrap().data(),
                AttributeData::DenseF32 {
                    lanes: 3,
                    values: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                }
            );
            assert_eq!(
                first
                    .attribute(AttributeSemanticRef::SmoothingGroup, AttributeDomain::Face)
                    .unwrap()
                    .data(),
                AttributeData::SparseU32 {
                    lanes: 1,
                    logical_count: 1,
                    indices: &[0],
                    values: &[7],
                    default: &[0],
                }
            );
            assert_eq!(
                second
                    .attribute(AttributeSemanticRef::Color(0), AttributeDomain::Face)
                    .unwrap()
                    .data(),
                AttributeData::SparseF32 {
                    lanes: 3,
                    logical_count: 1,
                    indices: &[0],
                    values: &[0.25, 0.5, 0.75],
                    default: &[0.0, 0.0, 0.0],
                }
            );
            assert_eq!(
                third
                    .attribute(AttributeSemanticRef::SmoothingGroup, AttributeDomain::Face)
                    .unwrap()
                    .data(),
                AttributeData::DenseU32 {
                    lanes: 1,
                    values: &[3],
                }
            );
            assert_eq!(
                third
                    .attribute(
                        AttributeSemanticRef::Custom("ply:dense_signed"),
                        AttributeDomain::Vertex
                    )
                    .unwrap()
                    .data(),
                AttributeData::DenseI32 {
                    lanes: 1,
                    values: &[i32::MIN, -1, i32::MAX],
                }
            );
            let sparse = third
                .attribute(
                    AttributeSemanticRef::Custom("ply:sparse_signed"),
                    AttributeDomain::Vertex,
                )
                .unwrap();
            assert_eq!(sparse.len(), 3);
            assert_eq!(
                sparse.data(),
                AttributeData::SparseI32 {
                    lanes: 1,
                    logical_count: 3,
                    indices: &[0],
                    values: &[i32::MIN],
                    default: &[i32::MAX],
                }
            );
            let instance = scene.mesh_instances().next().unwrap();
            assert_eq!(instance.mesh().id(), mesh.id());
            let root = scene.roots().next().unwrap();
            assert_eq!(root.id(), instance.node().id());
            assert_eq!(root.children().len(), 0);
            assert_eq!(root.authored_transform(), AuthoredTransform::identity_trs());
            assert_eq!(scene.node(root.id()).unwrap().id(), root.id());
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn material_assignment_public_reads_and_snapshot_preserve_source_presence() {
    let factory = factory();
    let account = account(&factory);
    let build = |assignment| {
        let attributes = [GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &[0.0, 0.0, 0.0],
            Presence::Absent,
        )];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            assignment,
            AuthoringRange::new(0, 1),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let materials = [MaterialAuthoring::new(
            AuthoringRange::new(0, 0),
            MaterialSemantics::new(
                ShadingModel::Unlit,
                AlphaMode::Opaque,
                false,
                MaterialFactors::empty(),
            ),
        )];
        build_scene_with_appearance_and_geometry(
            &account,
            &[],
            &[],
            AppearanceAuthoringBatch::new(&[], &[], &[], &[], &materials),
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
        )
    };
    let mut digests = Vec::new();
    for assignment in [
        crate::MaterialAssignment::Unspecified,
        crate::MaterialAssignment::Authored(MaterialLocalId::new(0)),
        crate::MaterialAssignment::ImplicitDefault(MaterialLocalId::new(0)),
    ] {
        let parts = build(assignment);
        let snapshot = capture_node_scene(&parts.scene, snapshot_limits(16_384, 64), None).unwrap();
        assert_eq!(snapshot.schema_revision, 8);
        assert!(!digests.contains(&snapshot.digest));
        digests.push(snapshot.digest);
        let scene = ValidatedScene::from_node_scene(parts.scene);
        let allocations = allocation_counter::measure(|| {
            let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
            let effective = primitive.effective_material();
            assert_eq!(effective.is_some(), assignment.effective().is_some());
            assert_eq!(
                primitive.authored_material().is_some(),
                assignment.authored().is_some()
            );
            match primitive.material_assignment() {
                crate::MaterialAssignment::Unspecified => assert!(effective.is_none()),
                crate::MaterialAssignment::Authored(material) => {
                    assert!(matches!(assignment, crate::MaterialAssignment::Authored(_)));
                    assert_eq!(material.id(), effective.unwrap().id());
                }
                crate::MaterialAssignment::ImplicitDefault(material) => {
                    assert!(matches!(
                        assignment,
                        crate::MaterialAssignment::ImplicitDefault(_)
                    ));
                    assert_eq!(material.id(), effective.unwrap().id());
                    assert!(primitive.authored_material().is_none());
                }
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        drop(scene);
        drop(parts.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }
}

#[test]
fn attribute_lookup_matches_semantic_and_domain_without_allocation() {
    let factory = factory();
    let account = account(&factory);
    let positions = [
        [0.0_f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let vertex_normals = [[0.0_f32, 0.0, 1.0]; 4];
    let corner_normals = [[0.0_f32, 1.0, 0.0]; 6];
    let vertex_texcoords = [[0.0_f32, 0.0]; 4];
    let corner_texcoords = [[1.0_f32, 1.0]; 6];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            positions.as_flattened(),
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            vertex_normals.as_flattened(),
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            3,
            corner_normals.as_flattened(),
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Vertex,
            2,
            vertex_texcoords.as_flattened(),
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Corner,
            2,
            corner_texcoords.as_flattened(),
            Presence::Absent,
        ),
    ];
    let indices = [0, 1, 2, 0, 2, 3];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Present(&indices),
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 5),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let parts = build_scene_with_geometry(
        &account,
        &[],
        &[],
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
    );
    let scene = ValidatedScene::from_node_scene(parts.scene);

    let allocations = allocation_counter::measure(|| {
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(primitive.vertex_count(), 4);
        assert_eq!(primitive.corner_count(), 6);
        assert_eq!(primitive.attributes().len(), 5);
        assert_eq!(
            primitive.positions().unwrap().domain(),
            AttributeDomain::Vertex
        );
        assert_eq!(
            primitive.positions().unwrap().data(),
            AttributeData::DenseF32 {
                lanes: 3,
                values: positions.as_flattened(),
            }
        );
        for (domain, normal_values, texcoord_values) in [
            (
                AttributeDomain::Vertex,
                vertex_normals.as_flattened(),
                vertex_texcoords.as_flattened(),
            ),
            (
                AttributeDomain::Corner,
                corner_normals.as_flattened(),
                corner_texcoords.as_flattened(),
            ),
        ] {
            let normals = primitive.normals(domain).unwrap();
            assert_eq!(normals.domain(), domain);
            assert_eq!(normals.len(), normal_values.len() / 3);
            assert_eq!(
                normals.data(),
                AttributeData::DenseF32 {
                    lanes: 3,
                    values: normal_values,
                }
            );
            assert_eq!(
                primitive
                    .attribute(AttributeSemanticRef::Normal, domain)
                    .unwrap()
                    .data(),
                normals.data()
            );
            let texcoords = primitive.texcoords(0, domain).unwrap();
            assert_eq!(texcoords.domain(), domain);
            assert_eq!(texcoords.len(), texcoord_values.len() / 2);
            assert_eq!(
                texcoords.data(),
                AttributeData::DenseF32 {
                    lanes: 2,
                    values: texcoord_values,
                }
            );
            assert!(primitive.texcoords(1, domain).is_none());
        }
        assert!(
            primitive
                .attribute(AttributeSemanticRef::Position, AttributeDomain::Corner)
                .is_none()
        );
        assert!(
            primitive
                .attribute(AttributeSemanticRef::Normal, AttributeDomain::Face)
                .is_none()
        );
        assert!(primitive.normals(AttributeDomain::Face).is_none());
        assert!(primitive.texcoords(0, AttributeDomain::Face).is_none());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn index_absence_and_authored_emptiness_are_distinct_in_public_views() {
    let factory = factory();
    let account = account(&factory);
    for present in [false, true] {
        let parts = point_index_presence_scene(&account, present);
        let scene = ValidatedScene::from_node_scene(parts.scene);
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(primitive.indices(), present.then_some(&[][..]));
        assert_eq!(primitive.vertex_count(), 1);
        drop(scene);
        drop(parts.report);
    }
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn spatial_readers_follow_node_attachments_and_preserve_unknowns() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        matrix(3.0),
        Presence::Present(CameraLocalId::new(0)),
        Presence::Present(LightLocalId::new(0)),
    )];
    let projection =
        CameraProjection::perspective(LocalAxes::STANDARD, 1.0, None, 0.1, None).unwrap();
    let cameras = [projection];
    let lights = [LightDefinition::new(
        LightKind::directional(),
        LocalAxes::STANDARD,
        white_emission(2.0),
    )];
    let positions = [0.0_f32, 0.0, 0.0];
    let tangents = [1.0_f32, 0.0, 0.0, 1.0];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            4,
            &tangents,
            Presence::Absent,
        ),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 2),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let parts = build_scene_with_spatial_appearance_and_geometry(
        &account,
        &roots,
        &nodes,
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &cameras, &lights),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
    );
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let node = scene.roots().next().unwrap();
    assert_eq!(
        node.authored_transform().kind(),
        crate::AuthoredTransformKind::Matrix
    );
    assert_eq!(
        node.authored_transform().authored_matrix(),
        matrix(3.0).authored_matrix()
    );
    let camera = node.camera().unwrap();
    let CameraProjection::Perspective(projection) = camera.projection() else {
        panic!("perspective projection expected");
    };
    assert_eq!(projection.axes(), LocalAxes::STANDARD);
    assert_eq!(
        projection.vertical_fov_radians().to_bits(),
        1.0_f32.to_bits()
    );
    assert_eq!(projection.aspect_ratio(), None);
    assert_eq!(projection.near().to_bits(), 0.1_f32.to_bits());
    assert_eq!(projection.far(), None);
    assert_eq!(scene.camera(camera.id()).unwrap().id(), camera.id());
    let light = node.light().unwrap();
    assert_eq!(
        light.definition().kind().category(),
        crate::LightKindCategory::Directional
    );
    assert_eq!(light.definition().axes(), LocalAxes::STANDARD);
    assert_eq!(light.definition().intensity().to_bits(), 2.0_f32.to_bits());
    assert_eq!(scene.light(light.id()).unwrap().id(), light.id());
    assert_eq!(scene.space().handedness, Handedness::Unknown);
    assert_eq!(scene.space().unit_scale_to_meters, None);
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.positions().unwrap().tangent_conventions(), None);
    let conventions = primitive
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
        .unwrap()
        .tangent_conventions()
        .unwrap();
    assert_eq!(conventions.basis, crate::TangentBasisRef::Unknown);
    assert_eq!(conventions.encoding, crate::TangentEncodingRef::Unknown);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn spatial_and_tangent_readers_borrow_retained_values_after_authoring_storage_is_dropped() {
    let factory = factory();
    let account = account(&factory);
    let reflected_axes =
        LocalAxes::try_new(Axis::NegativeX, Axis::PositiveY, Axis::NegativeZ).unwrap();
    let rotated_axes =
        LocalAxes::try_new(Axis::PositiveZ, Axis::PositiveX, Axis::NegativeY).unwrap();
    let projections = [
        CameraProjection::perspective(LocalAxes::STANDARD, 0.75, None, 0.125, None).unwrap(),
        CameraProjection::orthographic(reflected_axes, 2.5, 1.25, 0.25, 1024.0).unwrap(),
    ];
    let light_kinds = [
        LightKind::directional(),
        LightKind::point(None).unwrap(),
        LightKind::spot(Some(32.0), 0.125, 0.75).unwrap(),
    ];
    let light_definitions = [
        LightDefinition::new(light_kinds[0], rotated_axes, white_emission(2.0)),
        LightDefinition::new(light_kinds[1], reflected_axes, white_emission(2.0)),
        LightDefinition::new(light_kinds[2], LocalAxes::STANDARD, white_emission(2.0)),
    ];
    let parts = {
        let roots = [
            NodeLocalId::new(0),
            NodeLocalId::new(1),
            NodeLocalId::new(2),
        ];
        let nodes = std::array::from_fn::<_, 3, _>(|index| {
            NodeAuthoringNode::new(
                Presence::Absent,
                trs(index as f32),
                Presence::Present(CameraLocalId::new((index % 2) as u32)),
                Presence::Present(LightLocalId::new(index as u32)),
            )
        });
        let space = spatial_scene_space();
        let cameras = projections;
        let lights = light_definitions;
        let positions = [0.0_f32, 0.0, 0.0];
        let tangents = [1.0_f32, 0.0, 0.0, 1.0];
        let attributes = [
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Tangent,
                AttributeDomain::Vertex,
                4,
                &tangents,
                Presence::Absent,
            )
            .with_tangent_conventions(namespaced_tangent_conventions()),
        ];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 2),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        build_scene_with_spatial_appearance_and_geometry(
            &account,
            &roots,
            &nodes,
            SpatialAuthoringBatch::new(&space, &cameras, &lights),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
        )
    };
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let allocations = allocation_counter::measure(|| {
        let space = scene.space();
        assert_eq!(space.handedness, Handedness::Left);
        assert_eq!(space.up_axis, Axis::NegativeY);
        assert_eq!(space.front_axis, Axis::PositiveZ);
        assert_eq!(
            space.unit_scale_to_meters.map(f64::to_bits),
            Some(0.01_f64.to_bits())
        );
        assert_eq!(space.front_face_winding, FrontFaceWinding::Clockwise);
        assert_eq!(space.uv_origin, UvOrigin::LowerLeft);
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(primitive.positions().unwrap().tangent_conventions(), None);
        let tangent = primitive
            .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
            .unwrap();
        assert_eq!(
            tangent.data(),
            AttributeData::DenseF32 {
                lanes: 4,
                values: &[1.0, 0.0, 0.0, 1.0],
            }
        );
        let conventions = tangent.tangent_conventions().unwrap();
        let crate::TangentBasisRef::Namespaced { name, version } = conventions.basis else {
            panic!("the retained tangent-basis identity must remain borrowed");
        };
        assert_eq!((name, version.get()), ("example:tangent-basis", 2));
        let crate::TangentEncodingRef::Namespaced { name, version } = conventions.encoding else {
            panic!("the retained tangent encoding must remain borrowed");
        };
        assert_eq!((name, version.get()), ("example:tangent-sign", 3));

        assert_eq!(scene.cameras().len(), projections.len());
        for (camera, expected) in scene.cameras().zip(projections) {
            assert_eq!(camera.projection(), expected);
            assert_eq!(scene.camera(camera.id()).unwrap().projection(), expected);
        }
        assert_eq!(scene.lights().len(), light_definitions.len());
        for (light, expected) in scene.lights().zip(light_definitions) {
            let definition = light.definition();
            assert_eq!(definition, expected);
            assert_eq!(definition.axes(), expected.axes());
            assert_eq!(definition.tint(), Vec3::new(1.0, 1.0, 1.0));
            assert_eq!(definition.intensity().to_bits(), 2.0_f32.to_bits());
            assert_eq!(scene.light(light.id()).unwrap().definition(), expected);
        }
        for (index, node) in scene.nodes().enumerate() {
            assert_eq!(node.camera().unwrap().projection(), projections[index % 2]);
            assert_eq!(node.light().unwrap().definition(), light_definitions[index]);
        }
        let first = scene.nodes().next().unwrap().camera().unwrap();
        let repeated = scene.nodes().nth(2).unwrap().camera().unwrap();
        assert_eq!(first.id(), repeated.id());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
