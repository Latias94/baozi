//! Complete source influence sets, streamed into the existing Vertex-major CSR rows.

use super::*;
use gltf::mesh::util::{
    ReadWeights,
    weights::{Cast as _, F32},
};
use std::cell::RefCell;

#[derive(Debug, Clone, Copy)]
pub(super) struct InfluenceContract {
    set_count: u32,
    vertex_count: usize,
    pub(super) offset_count: usize,
    pub(super) cell_count: usize,
    pub(super) work: u64,
}

pub(super) struct InfluenceRows<'scope> {
    pub(super) offsets: MeteredVec<'scope, u32>,
    pub(super) values: MeteredVec<'scope, InfluenceAuthoring>,
    pub(super) non_unit_rows: usize,
}

pub(super) fn preflight(
    ctx: &ParseScope<'_>,
    primitive: &json::mesh::Primitive,
    vertex_count: usize,
    owner: AccessorOwner,
) -> Result<InfluenceContract> {
    // Semantic derives Ord: each indexed semantic group is visited in ascending set order.
    // Count actual keys instead of reserving storage indexed by an untrusted suffix.
    let mut joints = 0u32;
    let mut weights = 0u32;
    for semantic in primitive.attributes.keys() {
        ctx.charge_work(1)?;
        let (set, expected, label) = match semantic {
            Checked::Valid(json::mesh::Semantic::Joints(set)) => (*set, &mut joints, "JOINTS"),
            Checked::Valid(json::mesh::Semantic::Weights(set)) => (*set, &mut weights, "WEIGHTS"),
            _ => continue,
        };
        if set != *expected {
            return Err(gltf_parse_error(
                ctx,
                format!("{owner} has non-contiguous {label} sets"),
            ));
        }
        *expected = expected
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-influence-set-count"))?;
    }
    if joints != weights {
        return Err(gltf_parse_error(
            ctx,
            format!(
                "{owner} JOINTS and WEIGHTS attributes must be provided together for every set"
            ),
        ));
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
    // Two accessor lookups per set, bounded construction, decoding/writing every lane,
    // and one stored offset per row. The row scratch heap sort takes at most 2R comparisons
    // per level; the remaining 8R covers collection, adjacent checks and lane conversion.
    // Ordinary work scopes cannot nest during decoding.
    let lookup_levels = u64::from(usize::BITS - primitive.attributes.len().leading_zeros());
    let row_levels = u64::from(u32::BITS - row_width.leading_zeros());
    let work = u64::from(joints)
        .checked_mul(2 * lookup_levels + 16)
        .and_then(|work| work.checked_add(to_u64(cell_count).checked_mul(2 * row_levels + 8)?))
        .and_then(|work| work.checked_add(to_u64(offset_count)))
        .ok_or_else(|| ctx.allocation_failure("gltf-influence-work"))?;
    Ok(InfluenceContract {
        set_count: joints,
        vertex_count,
        offset_count,
        cell_count,
        work,
    })
}

pub(super) fn build_rows<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    primitive: &gltf::Primitive<'_>,
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

    let reader = primitive.reader(|buffer| {
        buffers
            .get(buffer.index())
            .and_then(Option::as_ref)
            .map(BufferData::as_slice)
    });
    let mut sets = ctx.metered_vec(
        ImportStorageClass::FormatNormalization,
        "gltf-influence-set-readers",
    );
    sets.try_reserve(contract.set_count as usize)?;
    let mut quantized = true;
    for set in 0..contract.set_count {
        poll_parser_loop(ctx, set as usize)?;
        let joints = reader.read_joints(set).ok_or_else(|| {
            gltf_parse_error(ctx, format!("glTF JOINTS_{set} accessor could not be read"))
        })?;
        let weights = reader.read_weights(set).ok_or_else(|| {
            gltf_parse_error(
                ctx,
                format!("glTF WEIGHTS_{set} accessor could not be read"),
            )
        })?;
        quantized &= !matches!(&weights, ReadWeights::F32(_));
        // MeteredVec restricts mutable slices to Copy elements. Each inline cell holds only
        // borrowed iterator state; interior mutation cannot replace or grow the metered table.
        sets.try_push_reserved(RefCell::new((joints.into_u16(), weights)))?;
    }

    let mut nonzero_joints = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        (contract.set_count as usize) * 4,
        "gltf-influence-unique-joints",
    )?;
    let mut non_unit_rows = 0;
    offsets.try_push_reserved(0)?;
    for vertex in 0..contract.vertex_count {
        poll_parser_loop(ctx, vertex)?;
        nonzero_joints.truncate(0);
        let mut integer_sum = 0u64;
        let mut float_sum = 0f64;
        for (set, readers) in sets.as_slice().iter().enumerate() {
            poll_parser_loop(ctx, influences.len())?;
            let (joints, weights, raw_sum) = {
                let mut readers = readers.borrow_mut();
                let joints = readers.0.next().ok_or_else(|| {
                    gltf_parse_error(ctx, "glTF joint stream is shorter than its admitted count")
                })?;
                let (weights, raw_sum) = next_weights(&mut readers.1).ok_or_else(|| {
                    gltf_parse_error(ctx, "glTF weight stream is shorter than its admitted count")
                })?;
                (joints, weights, raw_sum)
            };
            // The row width is in the u32 domain; summing u16 lanes cannot overflow u64.
            integer_sum += raw_sum;
            // Preflight checked the complete width, endpoint and output capacity.
            let source_base = (set as u32) * 4;
            for lane in 0..4 {
                let weight = weights[lane];
                if !weight.is_finite() || weight < 0.0 {
                    return Err(gltf_parse_error(
                        ctx,
                        "glTF influence weights must be finite and non-negative",
                    ));
                }
                float_sum += f64::from(weight);
                if weight > 0.0 {
                    nonzero_joints.try_push_reserved(joints[lane])?;
                }
                influences.try_push_reserved(InfluenceAuthoring::new(
                    u32::from(joints[lane]),
                    weight,
                    source_base + lane as u32,
                ))?;
            }
        }
        if quantized && integer_sum != 65_535 {
            return Err(gltf_parse_error(
                ctx,
                "glTF quantized influence weights must sum to one across all sets",
            ));
        }
        if !quantized && (float_sum - 1.0).abs() > 2e-7 * nonzero_joints.len() as f64 {
            non_unit_rows += 1;
        }
        let mut until_poll = 0;
        nonzero_joints.try_sort_unstable_by(|left, right| -> Result<_> {
            if until_poll == 0 {
                ctx.poll()?;
                until_poll = PARSER_POLL_INTERVAL;
            }
            until_poll -= 1;
            Ok(left.cmp(right))
        })?;
        for (index, pair) in nonzero_joints.as_slice().windows(2).enumerate() {
            poll_parser_loop(ctx, index)?;
            if pair[0] == pair[1] {
                return Err(gltf_parse_error(
                    ctx,
                    "glTF nonzero influence joint indices must be unique per vertex",
                ));
            }
        }
        offsets.try_push_reserved(authoring_u32(
            ctx,
            influences.len(),
            "gltf-influence-offset",
        )?)?;
    }
    for (set, readers) in sets.as_slice().iter().enumerate() {
        poll_parser_loop(ctx, set)?;
        let mut readers = readers.borrow_mut();
        if readers.0.next().is_some() || next_weights(&mut readers.1).is_some() {
            return Err(gltf_parse_error(
                ctx,
                "glTF influence stream exceeds its admitted count",
            ));
        }
    }
    if influences.len() != contract.cell_count || offsets.len() != contract.offset_count {
        return Err(GltfFailure::internal("glTF influence output differs from preflight").into());
    }
    Ok(InfluenceRows {
        offsets,
        values: influences,
        non_unit_rows,
    })
}

/// Keep raw integer sums separate from the existing backend's F32 dequantization.
fn next_weights(weights: &mut ReadWeights<'_>) -> Option<([f32; 4], u64)> {
    match weights {
        ReadWeights::U8(values) => values.next().map(|values| {
            let sum = values.iter().map(|&value| u64::from(value) * 257).sum();
            (F32::cast_u8(values), sum)
        }),
        ReadWeights::U16(values) => values.next().map(|values| {
            let sum = values.iter().map(|&value| u64::from(value)).sum();
            (F32::cast_u16(values), sum)
        }),
        ReadWeights::F32(values) => values.next().map(|values| (values, 0)),
    }
}
