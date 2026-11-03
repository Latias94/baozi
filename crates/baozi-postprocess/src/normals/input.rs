#![allow(
    clippy::result_large_err,
    reason = "runtime and kernel failures stay inline when allocation or cancellation rejects work"
)]

use baozi_core::__scene_integration::integration::SceneVectorValues;
use baozi_core::AttributeRef;
use baozi_operation::{StorePlanError, StoreRuntimeError};

use super::kernel;
use super::options::NormalMode;
use crate::operation::runtime::{ActiveRuntime, RuntimeKeys, ScratchVec};

#[derive(Debug)]
pub(crate) enum InputError {
    Kernel(kernel::Error<StoreRuntimeError>),
    Runtime(StoreRuntimeError),
}

impl From<StoreRuntimeError> for InputError {
    fn from(cause: StoreRuntimeError) -> Self {
        Self::Runtime(cause)
    }
}

/// Source payloads remain borrowed until generation finishes; no position projection is retained.
pub(crate) struct NormalInput<'a> {
    corner_count: u32,
    positions: AttributeRef<'a>,
    indices: Option<&'a [u32]>,
    smoothing_groups: Option<&'a [u32]>,
    mode: NormalMode,
    wide_output: bool,
}

/// Only the generated Corner values and source facts survive into the Scene transaction.
pub(crate) struct GeneratedNormals {
    normals: NormalValues,
}

enum NormalValues {
    F32(ScratchVec<[f32; 3]>),
    F64(ScratchVec<[f64; 3]>),
}

impl GeneratedNormals {
    pub(crate) fn values(&self) -> SceneVectorValues<'_, 3> {
        match &self.normals {
            NormalValues::F32(values) => SceneVectorValues::F32(values.values()),
            NormalValues::F64(values) => SceneVectorValues::F64(values.values()),
        }
    }

    pub(crate) fn orient(
        &mut self,
        reverse: bool,
        operation: &mut ActiveRuntime<'_>,
    ) -> Result<(), InputError> {
        if reverse {
            match &mut self.normals {
                NormalValues::F32(values) => {
                    for value in values.values_mut() {
                        run_copy_action(operation, kernel::Action::OrientElement, || {
                            *value = value.map(|v| if v == 0.0 { v } else { -v })
                        })?;
                    }
                }
                NormalValues::F64(values) => {
                    for value in values.values_mut() {
                        run_copy_action(operation, kernel::Action::OrientElement, || {
                            *value = value.map(|v| if v == 0.0 { v } else { -v })
                        })?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn generate(
    source: NormalInput<'_>,
    operation: &mut ActiveRuntime<'_>,
    keys: RuntimeKeys,
) -> Result<GeneratedNormals, InputError> {
    let corners = source.corner_count;
    let faces = corners / 3;
    let input = kernel::Input {
        positions: source.positions,
        vertex_ids: source
            .indices
            .map_or(kernel::VertexIds::NonIndexed, kernel::VertexIds::Indexed),
        smoothing_groups: source.smoothing_groups,
    };
    let mode = match source.mode {
        NormalMode::Flat => kernel::Mode::Flat,
        NormalMode::Smooth => kernel::Mode::Smooth,
    };
    let mut normals = if source.wide_output {
        let mut values = ScratchVec::new(operation, keys, corners as usize)?;
        for start in (0..corners).step_by(32) {
            let count = (corners - start).min(32);
            run_copy_action(operation, kernel::Action::ScratchChunk, || {
                for _ in 0..count {
                    values.values_mut().push([0.0_f64; 3]);
                }
            })?;
        }
        NormalValues::F64(values)
    } else {
        let mut values = ScratchVec::new(operation, keys, corners as usize)?;
        for start in (0..corners).step_by(32) {
            let count = (corners - start).min(32);
            run_copy_action(operation, kernel::Action::ScratchChunk, || {
                for _ in 0..count {
                    values.values_mut().push([0.0_f32; 3]);
                }
            })?;
        }
        NormalValues::F32(values)
    };
    let cursor_count = input.vertex_cursor_count(mode);
    let order_count = (corners as usize)
        .checked_add(cursor_count)
        .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let mut order = if source.mode == NormalMode::Smooth {
        Some(ScratchVec::new(operation, keys, order_count)?)
    } else {
        None
    };
    let mut crosses = if source.mode == NormalMode::Smooth {
        Some(ScratchVec::new(operation, keys, faces as usize)?)
    } else {
        None
    };
    if let Some(order) = &mut order {
        for start in (0..order_count).step_by(32) {
            let count = (order_count - start).min(32);
            run_copy_action(operation, kernel::Action::ScratchChunk, || {
                for _ in 0..count {
                    order.values_mut().push(0_usize);
                }
            })?;
        }
    }
    if let Some(crosses) = &mut crosses {
        for start in (0..faces).step_by(32) {
            let count = (faces - start).min(32);
            run_copy_action(operation, kernel::Action::ScratchChunk, || {
                for _ in 0..count {
                    crosses.values_mut().push(crate::precision::Estimate3 {
                        value: [0.0; 3],
                        abs_error: [0.0; 3],
                    });
                }
            })?;
        }
    }

    let (corner_order, vertex_cursors) = match order.as_mut() {
        Some(order) => {
            let (corner_order, cursors) = order.values_mut().split_at_mut(corners as usize);
            (corner_order, (cursor_count != 0).then_some(cursors))
        }
        None => (&mut [][..], None),
    };
    kernel::generate_into(
        input,
        mode,
        match &mut normals {
            NormalValues::F32(values) => kernel::Output::F32(values.values_mut()),
            NormalValues::F64(values) => kernel::Output::F64(values.values_mut()),
        },
        kernel::Workspace {
            corner_order,
            vertex_cursors,
            face_crosses: crosses
                .as_mut()
                .map(|crosses| crosses.values_mut().as_mut_slice()),
        },
        |event| observe(operation, event),
    )
    .map_err(InputError::Kernel)?;
    drop(crosses);
    drop(order);
    Ok(GeneratedNormals { normals })
}

impl<'a> NormalInput<'a> {
    pub(crate) fn projected(
        positions: AttributeRef<'a>,
        indices: Option<&'a [u32]>,
        smoothing_groups: Option<&'a [u32]>,
        corner_count: u32,
        mode: NormalMode,
        wide_output: bool,
    ) -> Self {
        Self {
            corner_count,
            positions,
            indices,
            smoothing_groups,
            mode,
            wide_output,
        }
    }
}

fn observe(
    operation: &mut ActiveRuntime<'_>,
    event: kernel::WorkEvent,
) -> Result<(), StoreRuntimeError> {
    match event {
        kernel::WorkEvent::Begin(kind) => operation.begin_action(kind.control_cost()),
        kernel::WorkEvent::Complete => operation.complete_action(),
        kernel::WorkEvent::Checkpoint => operation.checkpoint(),
    }
}

fn run_copy_action<R>(
    operation: &mut ActiveRuntime<'_>,
    kind: kernel::Action,
    action: impl FnOnce() -> R,
) -> Result<R, InputError> {
    kernel::run_action(
        &mut |event| observe(operation, event),
        kind,
        || Ok(action()),
    )
    .map_err(InputError::Kernel)
}
