mod common;
use baozi::*;
use baozi_asset_consumer::{
    Influences, MorphStream, TangentBasis, TangentConventions, TangentEncoding, UvStream,
};
use baozi_render_wgpu::{Control, Error, deformation::evaluated_frames, device::*};

#[test]
fn u16_full_domain_and_whole_triangle_splits() {
    for vertices in [65_535, 65_536, 65_537] {
        let asset = common::asset(vertices);
        let evaluated = common::evaluated(&asset);
        let plan = DevicePlan::prepare(
            &asset,
            &evaluated,
            &[],
            DeviceLimits::default(),
            Control::default(),
        )
        .unwrap();
        assert_eq!(plan.chunks().len(), if vertices > 65_536 { 2 } else { 1 });
        let expanded: Vec<_> = plan
            .chunks()
            .iter()
            .flat_map(|c| c.indices.iter().map(|&i| c.vertex_sources[i as usize]))
            .collect();
        assert_eq!(expanded, asset.draws[0].indices);
        if vertices == 65_536 {
            let bytes = plan.chunks()[0].index_bytes(IndexWidth::U16).unwrap();
            assert!(bytes.chunks_exact(2).any(|b| b == [255, 255]));
        }
    }
}

#[test]
fn complete_rows_morphs_inactive_uvs_and_no_narrow_weld() {
    let mut asset = common::asset(7);
    asset.materials[0].shading_model = ShadingModel::PbrMetallicRoughness;
    asset.materials[0].alpha_mode = AlphaMode::Blend;
    asset.draws[0].vertices[1].position = asset.draws[0].vertices[0].position;
    asset.draws[0].vertices[1].position[0] += 1e-9;
    asset.draws[0].indices = vec![0, 1, 2, 2, 3, 4, 4, 5, 0];
    for (i, vertex) in asset.draws[0].vertices.iter_mut().enumerate() {
        vertex.normal = Some([0.0, 0.0, 1.0]);
        vertex.tangent = Some([1.0, 0.0, 0.0, if i % 2 == 0 { 1.0 } else { -1.0 }]);
        vertex.color = Some([i as f64 / 8.0, 0.25, 1.5, 0.5]);
    }
    asset.draws[0].tangent_conventions = Some(TangentConventions {
        domain: AttributeDomain::Vertex,
        basis: TangentBasis::Unknown,
        encoding: TangentEncoding::SignInW,
    });
    asset.draws[0].uv_streams = [0, 1, 3, 7]
        .into_iter()
        .map(|set| UvStream {
            set,
            domain: AttributeDomain::Vertex,
            values: (0..7).map(|i| [(i + set) as f64 / 8.0, -0.0]).collect(),
        })
        .collect();
    asset.draws[0].morphs = [
        MorphDeltaSemantic::Position,
        MorphDeltaSemantic::Normal,
        MorphDeltaSemantic::Tangent,
    ]
    .into_iter()
    .enumerate()
    .map(|(stream, semantic)| MorphStream {
        target: 0,
        semantic,
        values: (0..7)
            .map(|i| [i as f32, -0.0, stream as f32 + 0.25])
            .collect(),
    })
    .collect();
    asset.mesh_morph_weights[0] = vec![0.0];
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 6, 8, 11, 12, 16, 21, 23],
        joints: vec![
            3, 1, 3, 0, 2, 1, 4, 4, 2, 0, 2, 5, 1, 6, 1, 0, 7, 3, 7, 2, 0, 8, 8,
        ],
        weights: vec![
            0.5, -0.0, 0.25, 0.0, 1.5, 0.75, 0.125, -0.0, 0.75, 0.0, 0.25, 2.0, -0.0, 0.375, 0.625,
            0.0, 0.25, -0.0, 0.5, 0.125, 0.0, 0.5, 0.5,
        ],
    });
    let texture = common::image(&mut asset, &[255; 4], 1, 1);
    asset.materials[0].bindings = vec![
        common::binding(
            TextureRole::BaseColor,
            texture,
            1,
            TextureChannelSwizzle::RGBA,
            ColorInterpretation::SrgbColor,
        ),
        common::binding(
            TextureRole::MetallicRoughness,
            texture,
            3,
            TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green]).unwrap(),
            ColorInterpretation::Data,
        ),
    ];
    let evaluated = common::evaluated(&asset);
    let frames = evaluated_frames(&evaluated).unwrap();
    let limits = DeviceLimits {
        max_vertices_per_chunk: 3,
        max_indices_per_chunk: 3,
        ..Default::default()
    };
    let plan =
        DevicePlan::prepare(&asset, &evaluated, &frames, limits, Control::default()).unwrap();
    assert_eq!(plan.chunks().len(), 3);
    assert_eq!(
        plan.chunks()
            .iter()
            .flat_map(|c| c.indices.iter().map(|&i| c.vertex_sources[i as usize]))
            .collect::<Vec<_>>(),
        asset.draws[0].indices
    );
    assert_eq!(
        &plan.chunks()[0].vertices[..4],
        &plan.chunks()[0].vertices[64..68]
    );
    assert_eq!(&plan.chunks()[0].vertex_sources[..2], &[0, 1]);
    for (chunk, c) in plan.chunks().iter().enumerate() {
        assert_eq!(c.elements, chunk..chunk + 1);
        assert_eq!(c.material, asset.draws[0].material);
        assert_eq!(c.uv_sets, vec![0, 1, 3, 7]);
        assert_eq!(c.active_uv_ordinals, [1, 2, u32::MAX, u32::MAX]);
        assert_eq!(
            c.field_flags,
            SOURCE_NORMAL | SOURCE_TANGENT | SOURCE_COLOR | WORLD_TANGENT
        );
        assert_eq!(
            c.palette_source_joints,
            match chunk {
                0 => vec![3, 1, 0, 2, 4],
                1 => vec![2, 0, 5, 1, 6],
                2 => vec![1, 6, 0, 7, 3, 2],
                _ => unreachable!(),
            }
        );
        let rows = c.influences.as_ref().unwrap();
        assert_eq!(
            c.vertex_sources,
            asset.draws[0].indices[chunk * 3..chunk * 3 + 3]
        );
        let source_rows = asset.draws[0].influences.as_ref().unwrap();
        let mut offset = 0;
        assert_eq!(rows.offsets[0], offset);
        for (local, &source) in c.vertex_sources.iter().enumerate() {
            let vertex = &asset.draws[0].vertices[source as usize];
            let position = evaluated.draws[0].world_positions[source as usize];
            let tangent = vertex.tangent.unwrap();
            let color = vertex.color.unwrap();
            let expected = [
                position[0],
                position[1],
                position[2],
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                tangent[0],
                tangent[1],
                tangent[2],
                tangent[3],
                color[0],
                color[1],
                color[2],
                color[3],
            ];
            for (lane, value) in expected.into_iter().enumerate() {
                let offset = local * 64 + lane * 4;
                assert_eq!(
                    f32::from_le_bytes(c.vertices[offset..offset + 4].try_into().unwrap())
                        .to_bits(),
                    (value as f32).to_bits()
                );
            }
            for (set, stream) in asset.draws[0].uv_streams.iter().enumerate() {
                for (lane, value) in stream.values[source as usize].into_iter().enumerate() {
                    let offset = (local * 4 + set) * 8 + lane * 4;
                    assert_eq!(
                        f32::from_le_bytes(c.uv_bytes[offset..offset + 4].try_into().unwrap())
                            .to_bits(),
                        (value as f32).to_bits()
                    );
                }
            }
            let (joints, weights) = plan.source_influence_row(chunk, local).unwrap();
            let source_range =
                source_rows.offsets[source as usize]..source_rows.offsets[source as usize + 1];
            let expected_joints = &source_rows.joints[source_range.clone()];
            let expected_weights = &source_rows.weights[source_range];
            let range = rows.offsets[local]..rows.offsets[local + 1];
            assert_eq!(joints, expected_joints);
            assert_eq!(
                expected_joints,
                rows.joints[range.clone()]
                    .iter()
                    .map(|&i| c.palette_source_joints[i as usize])
                    .collect::<Vec<_>>()
            );
            let expected_bits = expected_weights
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>();
            assert_eq!(
                weights.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                expected_bits
            );
            assert_eq!(
                rows.weights[range]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected_bits
            );
            offset += expected_joints.len();
            assert_eq!(rows.offsets[local + 1], offset);
        }
        for stream in 0..3 {
            let (morph, map) = plan.source_morph(chunk, stream).unwrap();
            assert_eq!(
                morph.semantic,
                [
                    MorphDeltaSemantic::Position,
                    MorphDeltaSemantic::Normal,
                    MorphDeltaSemantic::Tangent
                ][stream]
            );
            assert_eq!(map, c.vertex_sources.as_slice());
            assert!(std::ptr::eq(morph, &asset.draws[0].morphs[stream]));
            for &source in map {
                assert_eq!(
                    morph.values[source as usize].map(f32::to_bits),
                    [source as f32, -0.0, stream as f32 + 0.25].map(f32::to_bits)
                );
            }
        }
        assert_eq!(plan.source_draw(chunk).unwrap().vertices.len(), 7); // Unused vertex 6 remains authoritative.
    }
}

#[test]
fn repeated_triangle_splits_charge_source_scans_before_work() {
    let mut asset = common::asset(100_000);
    asset.draws[0].indices = [0, 1, 2].repeat(1000);
    let evaluated = common::evaluated(&asset);
    let limits = DeviceLimits {
        max_indices_per_chunk: 3,
        max_expansion: 1000,
        max_work: 2_000_000,
        ..Default::default()
    };
    assert!(
        matches!(DevicePlan::prepare(&asset,&evaluated,&[],limits,Control::default()),
        Err(Error::Limit { dimension:"planning work",requested,limit:2_000_000 }) if requested>2_000_000 && requested<=2_000_016)
    );
    let plan = DevicePlan::prepare(
        &asset,
        &evaluated,
        &[],
        DeviceLimits {
            max_work: 6_000_000,
            ..limits
        },
        Control::default(),
    )
    .unwrap();
    assert_eq!(plan.chunks().len(), 1000);
    assert!(plan.work() < 6_000_000);
    for chunk in plan.chunks() {
        assert_eq!(chunk.vertex_sources, [0, 1, 2]);
        assert_eq!(chunk.indices, [0, 1, 2]);
    }
}

#[test]
fn distinct_triangle_splits_have_equivalent_tuples_and_linear_capacity_with_loose_index_caps() {
    let mut asset = common::asset(3000);
    for (source, vertex) in asset.draws[0].vertices.iter_mut().enumerate() {
        vertex.color = Some([source as f64 / 4096.0, 0.25, 0.5, 1.0]);
    }
    let evaluated = common::evaluated(&asset);
    let limits = DeviceLimits {
        max_vertices_per_chunk: 3,
        max_plan_bytes: 2 * 1024 * 1024,
        ..Default::default()
    };
    let loose = DevicePlan::prepare(&asset, &evaluated, &[], limits, Control::default()).unwrap();
    let tight = DevicePlan::prepare(
        &asset,
        &evaluated,
        &[],
        DeviceLimits {
            max_indices_per_chunk: 3,
            ..limits
        },
        Control::default(),
    )
    .unwrap();
    for plan in [&loose, &tight] {
        assert_eq!(plan.chunks().len(), 1000);
        assert!(plan.plan_bytes() <= limits.max_plan_bytes);
        assert_eq!(
            plan.chunks()
                .iter()
                .flat_map(|c| c.indices.iter().map(|&i| c.vertex_sources[i as usize]))
                .collect::<Vec<_>>(),
            asset.draws[0].indices
        );
        let index_slots: usize = plan.chunks().iter().map(|c| c.indices.capacity()).sum();
        let source_slots: usize = plan
            .chunks()
            .iter()
            .map(|c| c.vertex_sources.capacity())
            .sum();
        assert!(index_slots <= 2 * asset.draws[0].indices.len());
        assert!(source_slots <= 2 * asset.draws[0].vertices.len());
    }
    assert_eq!(loose.plan_bytes(), tight.plan_bytes());
    for (element, (a, b)) in loose.chunks().iter().zip(tight.chunks()).enumerate() {
        assert_eq!(a.elements, element..element + 1);
        assert_eq!(
            a.vertex_sources,
            asset.draws[0].indices[element * 3..element * 3 + 3]
        );
        assert_eq!(a.indices, [0, 1, 2]);
        assert_eq!(
            (
                a.instance,
                a.draw,
                a.material,
                a.compiled_material,
                a.field_flags,
                a.orientation
            ),
            (
                b.instance,
                b.draw,
                b.material,
                b.compiled_material,
                b.field_flags,
                b.orientation
            )
        );
        assert_eq!(a.elements, b.elements);
        assert_eq!(a.vertex_sources, b.vertex_sources);
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.uv_bytes, b.uv_bytes);
        assert_eq!(a.uv_sets, b.uv_sets);
        assert_eq!(a.active_uv_ordinals, b.active_uv_ordinals);
        assert!(a.influences.is_none() && b.influences.is_none());
        assert!(a.palette_source_joints.is_empty() && b.palette_source_joints.is_empty());
        for (local, &source) in a.vertex_sources.iter().enumerate() {
            let offset = local * 64 + 48;
            assert_eq!(
                f32::from_le_bytes(a.vertices[offset..offset + 4].try_into().unwrap()),
                source as f32 / 4096.0
            );
        }
    }
}

#[test]
fn palette_orientation_and_byte_splits_keep_vertex_and_index_capacities_linear() {
    let mut asset = common::asset(180);
    asset.draws[0].influences = Some(Influences {
        offsets: (0..=180).collect(),
        joints: (0..180).map(|v| (v / 3) % 2).collect(),
        weights: vec![1.0; 180],
    });
    let evaluated = common::evaluated(&asset);
    let frames = evaluated_frames(&evaluated).unwrap();
    let alternating: Vec<i8> = (0..180)
        .map(|v| if (v / 3) % 2 == 0 { 1 } else { -1 })
        .collect();
    let orientation_frames = [baozi_render_wgpu::deformation::DrawFrames {
        orientations: &alternating,
        ..frames[0]
    }];
    for (limits, supplied) in [
        (
            DeviceLimits {
                max_chunk_bytes: 3 * 64 + 3 * 2,
                ..Default::default()
            },
            frames.as_slice(),
        ),
        (
            DeviceLimits {
                palette: PalettePolicy::Bounded {
                    max_joints: 1,
                    allow_cpu_fallback: false,
                },
                ..Default::default()
            },
            frames.as_slice(),
        ),
        (DeviceLimits::default(), orientation_frames.as_slice()),
    ] {
        let plan =
            DevicePlan::prepare(&asset, &evaluated, supplied, limits, Control::default()).unwrap();
        assert_eq!(plan.chunks().len(), 60);
        for (element, chunk) in plan.chunks().iter().enumerate() {
            assert_eq!(chunk.elements, element..element + 1);
            assert_eq!(
                chunk.vertex_sources,
                asset.draws[0].indices[element * 3..element * 3 + 3]
            );
            assert_eq!(chunk.indices, [0, 1, 2]);
            assert!(chunk.vertex_sources.capacity() <= 2 * chunk.vertex_sources.len());
            assert!(chunk.indices.capacity() <= 2 * chunk.indices.len());
        }
    }
}

#[test]
fn large_palette_rejects_at_growth_after_material_and_chunk_headers_are_admitted() {
    let mut asset = common::asset(3);
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 4096, 4097, 4098],
        joints: (0..4096).chain([0, 0]).collect(),
        weights: vec![0.0; 4098],
    });
    let evaluated = common::evaluated(&asset);
    let material = baozi_render_wgpu::material::CompiledMaterial::compile(&asset, Some(0)).unwrap();
    // At the first triangle's palette discovery, geometry and row buffers are still empty.
    let resident = std::mem::size_of_val(&material)
        + material.active_uv_sets.capacity() * 4
        + std::mem::size_of::<Chunk>()
        + 3 * std::mem::size_of::<usize>();
    let expected_request = (resident + (1024 + 2048) * 4) as u64;
    let limits = DeviceLimits {
        max_plan_bytes: expected_request - 1,
        ..Default::default()
    };
    // A stricter count cap proves that this budget reaches the 1025th distinct joint.
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluated,
            &[],
            DeviceLimits {
                max_cpu_joints: 1024,
                ..limits
            },
            Control::default()
        ),
        Err(Error::Limit {
            dimension: "palette scratch",
            requested: 1025,
            limit: 1024
        })
    ));
    assert!(
        matches!(DevicePlan::prepare(&asset,&evaluated,&[],limits,Control::default()),
        Err(Error::Limit { dimension:"plan bytes",requested,limit }) if requested==expected_request && limit==expected_request-1)
    );
}

#[test]
fn plan_capacity_counts_headers_material_uv_storage_and_live_draw_scratch() {
    let mut asset = common::asset(1000);
    asset.draws[0].indices = vec![0, 1, 2];
    let evaluated = common::evaluated(&asset);
    let plan = DevicePlan::prepare(
        &asset,
        &evaluated,
        &[],
        DeviceLimits::default(),
        Control::default(),
    )
    .unwrap();
    let chunk = &plan.chunks()[0];
    let material = &plan.materials()[0];
    let bytes = std::mem::size_of::<Chunk>()
        + std::mem::size_of_val(material)
        + material.active_uv_sets.capacity() * 4
        + chunk.vertex_sources.capacity() * 4
        + chunk.indices.capacity() * 4
        + chunk.vertices.capacity()
        + chunk.uv_bytes.capacity();
    assert_eq!(plan.plan_bytes(), bytes as u64);
    // The final resident plan alone cannot admit the simultaneously live 1000-entry lookup.
    let lookup_request = (std::mem::size_of_val(material)
        + material.active_uv_sets.capacity() * 4
        + asset.draws[0].vertices.len() * std::mem::size_of::<usize>())
        as u64;
    assert!(
        matches!(DevicePlan::prepare(&asset,&evaluated,&[],DeviceLimits { max_plan_bytes:plan.plan_bytes(),..Default::default() },Control::default()),
        Err(Error::Limit { dimension:"plan bytes",requested,limit }) if requested==lookup_request && limit==plan.plan_bytes())
    );
}

#[test]
fn named_failures_are_atomic_and_palette_fallback_is_explicit() {
    let mut asset = common::asset(3);
    asset.draws[0].influences = Some(Influences {
        offsets: vec![0, 2, 4, 6],
        joints: vec![0, 1, 1, 2, 2, 3],
        weights: vec![0.5; 6],
    });
    let evaluated = common::evaluated(&asset);
    let limits = DeviceLimits {
        palette: PalettePolicy::Bounded {
            max_joints: 3,
            allow_cpu_fallback: false,
        },
        ..Default::default()
    };
    assert!(matches!(
        DevicePlan::prepare(&asset, &evaluated, &[], limits, Control::default()),
        Err(Error::IndivisibleElementTooLarge {
            dimension: "joint palette",
            ..
        })
    ));
    let plan = DevicePlan::prepare(
        &asset,
        &evaluated,
        &[],
        DeviceLimits {
            palette: PalettePolicy::Bounded {
                max_joints: 3,
                allow_cpu_fallback: true,
            },
            ..limits
        },
        Control::default(),
    )
    .unwrap();
    assert_eq!(plan.chunks()[0].palette_policy, PalettePolicy::CpuFullRows);
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluated,
            &[],
            limits,
            Control {
                cancellation: Some(&cancelled)
            }
        ),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        DevicePlan::prepare(
            &asset,
            &evaluated,
            &[],
            DeviceLimits {
                max_chunk_bytes: 1,
                ..Default::default()
            },
            Control::default()
        ),
        Err(Error::IndivisibleElementTooLarge {
            dimension: "chunk bytes",
            ..
        })
    ));
}

#[test]
fn scalar_precision_and_signed_zero() {
    let at = ScalarLocation {
        instance: 0,
        draw: 0,
        vertex: 0,
        stream: "test",
        lane: 0,
    };
    assert_eq!(
        narrow(-0.0, Tolerance::POSITION, at).unwrap().to_bits(),
        (-0.0f32).to_bits()
    );
    for (value, expected) in [
        (f64::NAN, ScalarError::NonFinite),
        (f64::MAX, ScalarError::Overflow),
        (f64::MIN_POSITIVE, ScalarError::Underflow),
        (1e-40, ScalarError::SubnormalUnsupported),
    ] {
        assert!(
            matches!(narrow(value,Tolerance::POSITION,at),Err(Error::Scalar {kind,..}) if kind==expected)
        );
    }
    assert!(matches!(
        narrow(
            1.0 + 1e-9,
            Tolerance {
                absolute: 0.0,
                relative: 0.0
            },
            at
        ),
        Err(Error::Scalar {
            kind: ScalarError::PrecisionExceeded,
            ..
        })
    ));
}
