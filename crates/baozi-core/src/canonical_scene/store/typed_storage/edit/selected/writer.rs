//! One streaming writer for counting, reconstruction, and semantic correspondence.

use super::*;

pub(super) enum Mode<'a> {
    Count,
    Build(&'a mut GeometryStorage),
    Compare(&'a GeometryStorage),
}

/// Only the current referenced payload span is retained. Pool relocation changes addresses,
/// while every referenced scalar is still compared at its exact source-selected bit pattern.
#[derive(Clone, Copy)]
struct ComparisonSpan {
    pool: usize,
    expected_start: usize,
    actual_start: usize,
    length: usize,
}

impl ComparisonSpan {
    fn index(self, pool: usize, expected: usize) -> Option<usize> {
        let offset = expected.checked_sub(self.expected_start)?;
        if self.pool != pool || offset >= self.length {
            return None;
        }
        self.actual_start.checked_add(offset)
    }
}

#[derive(Clone, Copy)]
pub(super) enum NameKind {
    Custom,
    TangentBasis,
    TangentEncoding,
    Mesh,
}

pub(super) struct Writer<'a, C> {
    mode: Mode<'a>,
    control: &'a mut C,
    pub(super) counts: [usize; 9],
    pub(super) work: u64,
    comparison: Option<ComparisonSpan>,
}

impl<'a, C: TypedStorageBuildControl> Writer<'a, C> {
    pub(super) fn new(mode: Mode<'a>, control: &'a mut C) -> Self {
        Self {
            mode,
            control,
            counts: [0; 9],
            work: 0,
            comparison: None,
        }
    }

    pub(super) fn action<R>(&mut self, action: impl FnOnce() -> R) -> EditResult<R, C::Error> {
        self.work = self.work.checked_add(1).ok_or_else(overflow)?;
        self.control
            .run_unit(action)
            .map_err(TypedStorageBuildError::Control)
    }

    pub(super) fn actual_payload(&mut self) -> EditResult<Option<Payload>, C::Error> {
        let index = self.counts[2];
        let mode = &self.mode;
        self.work = self.work.checked_add(1).ok_or_else(overflow)?;
        self.control
            .run_unit(|| match mode {
                Mode::Compare(output) => output
                    .attributes
                    .get(index)
                    .map(|a| Some(a.payload))
                    .ok_or(TypedStorageBuildError::Invariant),
                _ => Ok(None),
            })
            .map_err(TypedStorageBuildError::Control)?
    }

    pub(super) fn actual_primitive(&mut self) -> EditResult<Option<StoredPrimitive>, C::Error> {
        let index = self.counts[1];
        let mode = &self.mode;
        self.work = self.work.checked_add(1).ok_or_else(overflow)?;
        self.control
            .run_unit(|| match mode {
                Mode::Compare(output) => output
                    .primitives
                    .get(index)
                    .copied()
                    .map(Some)
                    .ok_or(TypedStorageBuildError::Invariant),
                _ => Ok(None),
            })
            .map_err(TypedStorageBuildError::Control)?
    }

    pub(super) fn project<T>(&mut self, pool: usize, actual: Option<TableRange<T>>) {
        self.comparison = actual.map(|range| ComparisonSpan {
            pool,
            expected_start: self.counts[pool],
            actual_start: range.start(),
            length: range.len(),
        });
    }

    pub(super) fn name(
        &mut self,
        source: &GeometryStorage,
        name: TextRange,
        kind: NameKind,
    ) -> EditResult<TextRange, C::Error> {
        let attribute = self.counts[2];
        let mesh = self.counts[0];
        let mode = &self.mode;
        self.work = self.work.checked_add(1).ok_or_else(overflow)?;
        let actual = self
            .control
            .run_unit(|| match mode {
                Mode::Compare(output) => {
                    let name = match kind {
                        NameKind::Mesh => output.logical_meshes.get(mesh).and_then(|m| m.name),
                        _ => output.attributes.get(attribute).and_then(|a| match kind {
                            NameKind::Custom => match a.semantic {
                                Semantic::Custom(name) => Some(name),
                                _ => None,
                            },
                            NameKind::TangentBasis => match a.tangent_conventions?.basis {
                                StoredTangentBasis::Namespaced { name, .. } => Some(name),
                                _ => None,
                            },
                            NameKind::TangentEncoding => match a.tangent_conventions?.encoding {
                                StoredTangentEncoding::Namespaced { name, .. } => Some(name),
                                _ => None,
                            },
                            NameKind::Mesh => None,
                        }),
                    };
                    name.map(Some).ok_or(TypedStorageBuildError::Invariant)
                }
                _ => Ok(None),
            })
            .map_err(TypedStorageBuildError::Control)??;
        self.project(4, actual.map(|name| name.0));
        let start = self.counts[4];
        for &byte in name
            .0
            .get(&source.names)
            .ok_or(TypedStorageBuildError::Invariant)?
        {
            self.byte(byte)?;
        }
        Ok(TextRange(TableRange::new(start, self.counts[4] - start)))
    }
}

macro_rules! emit {
    ($method:ident, $field:ident, $pool:expr, $ty:ty, $same:expr) => {
        impl<C: TypedStorageBuildControl> Writer<'_, C> {
            pub(super) fn $method(&mut self, value: $ty) -> EditResult<usize, C::Error> {
                let index = self.counts[$pool];
                let comparison = self.comparison;
                let mode = &mut self.mode;
                self.work = self.work.checked_add(1).ok_or_else(overflow)?;
                let valid = self
                    .control
                    .run_unit(|| match mode {
                        Mode::Count => true,
                        Mode::Build(output) => {
                            if output.$field.len() == output.$field.capacity() {
                                false
                            } else {
                                output.$field.push(value);
                                true
                            }
                        }
                        Mode::Compare(output) => {
                            let index = if $pool < 4 {
                                Some(index)
                            } else {
                                comparison.and_then(|span| span.index($pool, index))
                            };
                            index
                                .and_then(|i| output.$field.get(i))
                                .is_some_and(|actual| ($same)(*actual, value))
                        }
                    })
                    .map_err(TypedStorageBuildError::Control)?;
                if !valid {
                    return Err(TypedStorageBuildError::Invariant);
                }
                self.counts[$pool] = index.checked_add(1).ok_or_else(overflow)?;
                Ok(index)
            }
        }
    };
}

emit!(
    mesh,
    logical_meshes,
    0,
    StoredLogicalMesh,
    |a: StoredLogicalMesh, b: StoredLogicalMesh| a.primitives == b.primitives
        && same_optional_range(a.name.map(|n| n.0), b.name.map(|n| n.0))
);
emit!(
    primitive,
    primitives,
    1,
    StoredPrimitive,
    |a: StoredPrimitive, b: StoredPrimitive| a.topology == b.topology
        && a.vertex_count == b.vertex_count
        && a.corner_count == b.corner_count
        && a.face_count == b.face_count
        && same_optional_range(a.polygon_boundaries, b.polygon_boundaries)
        && same_optional_range(a.indices, b.indices)
        && a.material == b.material
        && a.attributes == b.attributes
);
emit!(
    attribute,
    attributes,
    2,
    StoredAttribute,
    |a: StoredAttribute, b: StoredAttribute| same_semantic(a.semantic, b.semantic)
        && same_conventions(a.tangent_conventions, b.tangent_conventions)
        && a.domain == b.domain
        && a.lanes == b.lanes
        && a.element_count == b.element_count
        && same_payload(a.payload, b.payload)
);
emit!(
    instance,
    mesh_instances,
    3,
    StoredMeshInstance,
    |a: StoredMeshInstance, b: StoredMeshInstance| a.owner_node == b.owner_node
        && a.logical_mesh == b.logical_mesh
);
emit!(f32, f32_values, 5, f32, |a: f32, b: f32| a.to_bits()
    == b.to_bits());
emit!(f64, f64_values, 6, f64, |a: f64, b: f64| a.to_bits()
    == b.to_bits());
emit!(u32, u32_values, 7, u32, |a, b| a == b);
emit!(i32, i32_values, 8, i32, |a, b| a == b);

impl<C: TypedStorageBuildControl> Writer<'_, C> {
    fn byte(&mut self, value: u8) -> EditResult<(), C::Error> {
        let index = self.counts[4];
        let comparison = self.comparison;
        let mode = &mut self.mode;
        self.work = self.work.checked_add(1).ok_or_else(overflow)?;
        let valid = self
            .control
            .run_string_unit(|| match mode {
                Mode::Count => true,
                Mode::Build(output) => {
                    if output.names.len() == output.names.capacity() {
                        false
                    } else {
                        output.names.push(value);
                        true
                    }
                }
                Mode::Compare(output) => {
                    comparison
                        .and_then(|span| span.index(4, index))
                        .and_then(|i| output.names.get(i))
                        == Some(&value)
                }
            })
            .map_err(TypedStorageBuildError::Control)?;
        if !valid {
            return Err(TypedStorageBuildError::Invariant);
        }
        self.counts[4] = index.checked_add(1).ok_or_else(overflow)?;
        Ok(())
    }
}

// Referenced bytes and scalars have already been compared by the streaming writer.
// Descriptors compare kind, shape and declarations, independently of pool addresses.
fn same_optional_range<T>(a: Option<TableRange<T>>, b: Option<TableRange<T>>) -> bool {
    a.map(|r| r.len()) == b.map(|r| r.len())
}

fn same_semantic(a: Semantic, b: Semantic) -> bool {
    match (a, b) {
        (Semantic::Custom(a), Semantic::Custom(b)) => a.0.len() == b.0.len(),
        _ => a == b,
    }
}

fn same_conventions(
    a: Option<StoredTangentConventions>,
    b: Option<StoredTangentConventions>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let basis = match (a.basis, b.basis) {
                (
                    StoredTangentBasis::Namespaced {
                        name: a,
                        version: av,
                    },
                    StoredTangentBasis::Namespaced {
                        name: b,
                        version: bv,
                    },
                ) => a.0.len() == b.0.len() && av == bv,
                (a, b) => a == b,
            };
            let encoding = match (a.encoding, b.encoding) {
                (
                    StoredTangentEncoding::Namespaced {
                        name: a,
                        version: av,
                    },
                    StoredTangentEncoding::Namespaced {
                        name: b,
                        version: bv,
                    },
                ) => a.0.len() == b.0.len() && av == bv,
                (a, b) => a == b,
            };
            basis && encoding
        }
        (None, None) => true,
        _ => false,
    }
}

fn same_payload(a: Payload, b: Payload) -> bool {
    match (a, b) {
        (Payload::DenseF32(a), Payload::DenseF32(b)) => a.len() == b.len(),
        (Payload::DenseF64(a), Payload::DenseF64(b)) => a.len() == b.len(),
        (Payload::DenseU32(a), Payload::DenseU32(b)) => a.len() == b.len(),
        (Payload::DenseI32(a), Payload::DenseI32(b)) => a.len() == b.len(),
        (
            Payload::SparseF32 {
                indices: a,
                values: av,
                default: ad,
            },
            Payload::SparseF32 {
                indices: b,
                values: bv,
                default: bd,
            },
        ) => a.len() == b.len() && av.len() == bv.len() && ad.len() == bd.len(),
        (
            Payload::SparseF64 {
                indices: a,
                values: av,
                default: ad,
            },
            Payload::SparseF64 {
                indices: b,
                values: bv,
                default: bd,
            },
        ) => a.len() == b.len() && av.len() == bv.len() && ad.len() == bd.len(),
        (
            Payload::SparseU32 {
                indices: a,
                values: av,
                default: ad,
            },
            Payload::SparseU32 {
                indices: b,
                values: bv,
                default: bd,
            },
        ) => a.len() == b.len() && av.len() == bv.len() && ad.len() == bd.len(),
        (
            Payload::SparseI32 {
                indices: a,
                values: av,
                default: ad,
            },
            Payload::SparseI32 {
                indices: b,
                values: bv,
                default: bd,
            },
        ) => a.len() == b.len() && av.len() == bv.len() && ad.len() == bd.len(),
        _ => false,
    }
}
