//! Complete source influence sets, streamed into the existing Vertex-major CSR rows.

use super::*;
use crate::document::read::Weights as ReadWeights;
use std::cell::RefCell;

#[derive(Debug, Clone, Copy)]
pub(super) struct InfluenceContract {
    set_count: u32,
    vertex_count: usize,
    pub(super) offset_count: usize,
    pub(super) cell_count: usize,
    setup_work: u64,
}

pub(super) struct InfluenceRows<'scope> {
    pub(super) offsets: MeteredVec<'scope, u32>,
    pub(super) values: MeteredVec<'scope, InfluenceAuthoring>,
    pub(super) non_unit_rows: usize,
}

struct InfluenceSet<'bytes, J> {
    joints: J,
    weights: ReadWeights<'bytes>,
    joint_bounds: accessor::MeshAccessorBounds<4>,
    weight_bounds: accessor::MeshAccessorBounds<4>,
}

struct InfluenceWeights {
    decoded: [f32; 4],
    stored: [f32; 4],
    unit_sum: u64,
}

pub(super) fn preflight(
    ctx: &mut ParseExecution<'_, '_>,
    primitive: &json::mesh::Primitive<'_>,
    vertex_count: usize,
    owner: AccessorOwner,
) -> Result<InfluenceContract> {
    // Semantic derives Ord: each indexed semantic group is visited in ascending set order.
    // Count actual keys instead of reserving storage indexed by an untrusted suffix.
    let mut joints = 0u32;
    let mut weights = 0u32;
    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for semantic in primitive.attributes.keys() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        let (set, expected, label) = match semantic {
            json::mesh::Semantic::Joints(set) => (*set, &mut joints, "JOINTS"),
            json::mesh::Semantic::Weights(set) => (*set, &mut weights, "WEIGHTS"),
            _ => continue,
        };
        if set != *expected {
            return Err(gltf_parse_error(format_args!(
                "{owner} has non-contiguous {label} sets"
            )));
        }
        *expected = expected
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-influence-set-count"))?;
    }
    if joints != weights {
        return Err(gltf_parse_error(format_args!(
            "{owner} JOINTS and WEIGHTS attributes must be provided together for every set"
        )));
    }
    let row_width = joints
        .checked_mul(4)
        .ok_or_else(|| ctx.allocation_failure("gltf-influence-row-width"))?;
    let cell_count = vertex_count
        .checked_mul(row_width as usize)
        .ok_or_else(|| ctx.allocation_failure("gltf-influence-cell-count"))?;
    authoring_u32(ctx, cell_count, "gltf-influence-endpoint")?;
    let offset_count = if joints == 0 {
        0
    } else {
        vertex_count
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-influence-offset-count"))?
    };
    let lookup_levels = u64::from(usize::BITS - primitive.attributes.len().leading_zeros());
    let setup_work = u64::from(joints)
        .checked_mul(2 * lookup_levels + 48 + 18)
        .ok_or_else(|| ctx.allocation_failure("gltf-influence-work"))?;
    Ok(InfluenceContract {
        set_count: joints,
        vertex_count,
        offset_count,
        cell_count,
        setup_work,
    })
}

pub(super) fn build_rows<'scope>(
    ctx: &mut ParseExecution<'_, 'scope>,
    buffers: &[Option<BufferData<'_>>],
    primitive: &crate::document::Primitive<'_>,
    contract: InfluenceContract,
) -> Result<InfluenceRows<'scope>> {
    let class = ImportStorageClass::SceneConstruction;
    let mut offsets = try_vec_capacity(
        ctx,
        class,
        contract.offset_count,
        "gltf-authoring-influence-offsets",
    )?;
    let mut influences =
        try_vec_capacity(ctx, class, contract.cell_count, "gltf-authoring-influences")?;
    if contract.set_count == 0 {
        return Ok(InfluenceRows {
            offsets,
            values: influences,
            non_unit_rows: 0,
        });
    }

    let mut sets = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-influence-set-readers",
    );
    sets.try_reserve(ctx, contract.set_count as usize)?;
    let mut quantized = true;
    ctx.with_work_group(
        [
            (
                ImportWorkClass::AttributeStreams,
                2 * u64::from(contract.set_count),
            ),
            (ImportWorkClass::WorkUnits, contract.setup_work),
        ],
        |ctx, work| -> Result<()> {
            let per_set = contract.setup_work / u64::from(contract.set_count);
            for set in 0..contract.set_count {
                poll_parser_loop(ctx, set as usize)?;
                let joint_source = primitive
                    .get(&Semantic::Joints(set))
                    .ok_or_else(|| gltf_parse_error("glTF joint accessor disappeared"))?;
                let weight_source = primitive
                    .get(&Semantic::Weights(set))
                    .ok_or_else(|| gltf_parse_error("glTF weight accessor disappeared"))?;
                let joints = accessor::bind(ctx, joint_source, buffers)?
                    .joints()
                    .ok_or_else(|| gltf_parse_error("glTF joint accessor encoding is invalid"))?;
                let weights = accessor::bind(ctx, weight_source, buffers)?
                    .weights()
                    .ok_or_else(|| gltf_parse_error("glTF weight accessor encoding is invalid"))?;
                let joint_bounds = accessor::MeshAccessorBounds::new(
                    joint_source.row(),
                    joint_source.index(),
                    false,
                )?;
                let weight_bounds = accessor::MeshAccessorBounds::new(
                    weight_source.row(),
                    weight_source.index(),
                    false,
                )?;
                quantized &= !matches!(&weights, ReadWeights::F32(_));
                // Inline cells retain only borrowed iterator state, never growing the table.
                sets.try_push_reserved(RefCell::new(InfluenceSet {
                    joints,
                    weights,
                    joint_bounds,
                    weight_bounds,
                }))?;
                work.complete([2, per_set])?;
            }
            Ok(())
        },
    )?;

    let mut nonzero_joints = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        (contract.set_count as usize) * 4,
        "gltf-influence-unique-joints",
    )?;
    ctx.with_work_group(
        [(ImportWorkClass::WorkUnits, 1)],
        |_ctx, work| -> Result<()> {
            offsets.try_push_reserved(0)?;
            work.complete([1])?;
            Ok(())
        },
    )?;
    let row_width = u64::from(contract.set_count) * 4;
    let sample_work = 8 * accessor::MESH_SAMPLE_WORK_PER_LANE;
    let row_work = u64::from(contract.set_count) * sample_work + row_width;
    let mut non_unit_rows = 0;
    for _vertex in 0..contract.vertex_count {
        nonzero_joints.truncate(0);
        let mut integer_sum = 0u64;
        let mut float_sum = 0f64;
        ctx.with_work_group(
            [
                (
                    ImportWorkClass::AttributeCells,
                    2 * u64::from(contract.set_count),
                ),
                (ImportWorkClass::InfluenceCells, row_width),
                (ImportWorkClass::WorkUnits, row_work),
            ],
            |ctx, work| -> Result<()> {
                // A vertex can contain arbitrarily many sets. Count each fixed decode/lane.
                let mut actions = 0usize;
                for (set, readers) in sets.as_slice().iter().enumerate() {
                    poll_parser_loop(ctx, actions)?;
                    let (joints, weights, raw_sum) = {
                        let mut readers = readers.borrow_mut();
                        let joints = readers.joints.next().ok_or_else(|| {
                            gltf_parse_error("glTF joint stream is shorter than its admitted count")
                        })?;
                        let weights = next_weights(&mut readers.weights).ok_or_else(|| {
                            gltf_parse_error(
                                "glTF weight stream is shorter than its admitted count",
                            )
                        })?;
                        if weights
                            .decoded
                            .iter()
                            .any(|weight| !weight.is_finite() || *weight < 0.0)
                        {
                            return Err(gltf_parse_error(
                                "glTF influence weights must be finite and non-negative",
                            ));
                        }
                        readers.joint_bounds.observe_stored(joints.map(f32::from))?;
                        readers.weight_bounds.observe_stored(weights.stored)?;
                        (joints, weights.decoded, weights.unit_sum)
                    };
                    work.complete([2, 0, sample_work])?;
                    actions += 1;
                    integer_sum += raw_sum;
                    let source_base = (set as u32) * 4;
                    for lane in 0..4 {
                        poll_parser_loop(ctx, actions)?;
                        let weight = weights[lane];
                        float_sum += f64::from(weight);
                        if weight > 0.0 {
                            nonzero_joints.try_push_reserved(joints[lane])?;
                        }
                        influences.try_push_reserved(InfluenceAuthoring::new(
                            u32::from(joints[lane]),
                            weight,
                            source_base + lane as u32,
                        ))?;
                        work.complete([0, 1, 1])?;
                        actions += 1;
                    }
                }
                Ok(())
            },
        )?;
        if quantized && integer_sum != 65_535 {
            return Err(gltf_parse_error(
                "glTF quantized influence weights must sum to one across all sets",
            ));
        }
        if !quantized && (float_sum - 1.0).abs() > 2e-7 * nonzero_joints.len() as f64 {
            non_unit_rows += 1;
        }
        ctx.with_work_group(
            [(ImportWorkClass::WorkUnits, 0)],
            |ctx, work| -> Result<()> {
                let mut actions = 0usize;
                nonzero_joints.try_sort_unstable_by(|left, right| -> Result<_> {
                    poll_parser_loop(ctx, actions)?;
                    work.replenish(1)?;
                    let ordering = left.cmp(right);
                    work.complete([1])?;
                    actions += 1;
                    Ok(ordering)
                })?;
                for pair in nonzero_joints.as_slice().windows(2) {
                    poll_parser_loop(ctx, actions)?;
                    work.replenish(1)?;
                    let duplicate = pair[0] == pair[1];
                    work.complete([1])?;
                    actions += 1;
                    if duplicate {
                        return Err(gltf_parse_error(
                            "glTF nonzero influence joint indices must be unique per vertex",
                        ));
                    }
                }
                poll_parser_loop(ctx, actions)?;
                work.replenish(1)?;
                offsets.try_push_reserved(authoring_u32(
                    ctx,
                    influences.len(),
                    "gltf-influence-offset",
                )?)?;
                work.complete([1])?;
                Ok(())
            },
        )?;
    }
    ctx.with_work_group(
        [(ImportWorkClass::WorkUnits, u64::from(contract.set_count))],
        |ctx, work| -> Result<()> {
            for (set, readers) in sets.as_slice().iter().enumerate() {
                poll_parser_loop(ctx, set)?;
                let mut readers = readers.borrow_mut();
                if readers.joints.next().is_some() || next_weights(&mut readers.weights).is_some() {
                    return Err(gltf_parse_error(
                        "glTF influence stream exceeds its admitted count",
                    ));
                }
                readers.joint_bounds.finish()?;
                readers.weight_bounds.finish()?;
                work.complete([1])?;
            }
            Ok(())
        },
    )?;
    if influences.len() != contract.cell_count || offsets.len() != contract.offset_count {
        return Err(GltfFailure::internal("glTF influence output differs from preflight").into());
    }
    Ok(InfluenceRows {
        offsets,
        values: influences,
        non_unit_rows,
    })
}

/// Keep raw integer sums separate from the shared, single-rounding accessor conversion.
fn next_weights(weights: &mut ReadWeights<'_>) -> Option<InfluenceWeights> {
    match weights {
        ReadWeights::U8(values) => values.next().map(|values| {
            let sum = values.iter().map(|&value| u64::from(value) * 257).sum();
            // Raw carries component bits in U32; this variant's checked source width is U8.
            InfluenceWeights {
                decoded: values.map(|value| crate::document::read::normalized_u8(value as u8)),
                stored: values.map(|value| value as f32),
                unit_sum: sum,
            }
        }),
        ReadWeights::U16(values) => values.next().map(|values| {
            let sum = values.iter().map(|&value| u64::from(value)).sum();
            InfluenceWeights {
                decoded: values.map(|value| crate::document::read::normalized_u16(value as u16)),
                stored: values.map(|value| value as f32),
                unit_sum: sum,
            }
        }),
        ReadWeights::F32(values) => values.next().map(|values| InfluenceWeights {
            decoded: values,
            stored: values,
            unit_sum: 0,
        }),
    }
}
