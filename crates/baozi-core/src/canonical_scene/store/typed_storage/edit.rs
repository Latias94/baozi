//! One source-bound typed geometry replacement, independent of Scene publication authority.
//!
//! The caller must admit all nine capacities before invoking `materialize`.

use baozi_operation::{CapacityPlan, CapacityPlanError};

#[cfg(test)]
use crate::attribute::AttributeDomain;
#[cfg(test)]
use crate::geometry::TopologyKind;

use super::build::{TypedStorageBuildControl, TypedStorageBuildError};
use super::{
    GeometryStorage, StoredAttribute, StoredAttributePayload, StoredLogicalMesh,
    StoredMeshInstance, StoredPrimitive, TableRange,
};

#[cfg(test)]
pub(in crate::canonical_scene::store) mod normals;
pub(in crate::canonical_scene::store) mod selected;
pub(in crate::canonical_scene::store) mod space;
pub(in crate::canonical_scene::store) mod space_animation;
pub(in crate::canonical_scene::store) mod space_appearance;
pub(in crate::canonical_scene::store) mod space_deformation;
pub(in crate::canonical_scene::store) mod space_geometry;
pub(in crate::canonical_scene::store) mod space_nodes;
pub(in crate::canonical_scene::store) mod space_spatial;
#[cfg(test)]
pub(in crate::canonical_scene::store) mod tangents;

type EditResult<T, E> = Result<T, TypedStorageBuildError<E>>;

/// Borrowed generated vectors at their selected output width.
#[derive(Debug, Clone, Copy)]
pub enum VectorValues<'a, const N: usize> {
    F32(&'a [[f32; N]]),
    F64(&'a [[f64; N]]),
}

impl<const N: usize> VectorValues<'_, N> {
    pub const fn len(self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub const fn is_f64(self) -> bool {
        matches!(self, Self::F64(_))
    }

    fn get(self, index: usize) -> Option<[f64; N]> {
        match self {
            Self::F32(values) => values.get(index).map(|value| value.map(f64::from)),
            Self::F64(values) => values.get(index).copied(),
        }
    }

    const fn unit_length_tolerance(self) -> f64 {
        if self.is_f64() {
            8.0 * f64::EPSILON
        } else {
            4.0 * f32::EPSILON as f64
        }
    }
}

#[cfg(test)]
mod test_selection;
#[cfg(test)]
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct TriangulationOrigins<'a> {
    pub triangles: &'a [[u32; 3]],
    pub face_offsets: &'a [u32],
}
#[cfg(test)]
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryTriangulationPlan<'a> {
    fixture: test_selection::TestSelection<'a>,
}
#[cfg(test)]
impl<'a> GeometryTriangulationPlan<'a> {
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a GeometryStorage,
        target: usize,
        origins: TriangulationOrigins<'a>,
        control: &mut C,
    ) -> EditResult<Self, C::Error> {
        if source
            .primitives
            .get(target)
            .is_none_or(|p| p.topology != TopologyKind::Polygons)
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        let mut fixture = test_selection::TestSelection::description(source, target);
        fixture.origins = Some(origins);
        Ok(Self {
            fixture: fixture.seal(control)?,
        })
    }
    pub(in crate::canonical_scene::store) const fn target_row(self) -> usize {
        self.fixture.target
    }
    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 9] {
        self.fixture.capacities
    }
    pub(in crate::canonical_scene::store) const fn work(self) -> u64 {
        self.fixture.work
    }
    pub(in crate::canonical_scene::store) const fn correspondence_work(self) -> u64 {
        self.fixture.work
    }
    pub(in crate::canonical_scene::store) const fn validation_work(self) -> u64 {
        self.fixture.validation_work
    }
    pub(in crate::canonical_scene::store) const fn name_bytes(self) -> usize {
        self.fixture.capacities[4].desired_capacity()
    }
    pub(in crate::canonical_scene::store) fn output_face_count(self) -> u32 {
        self.fixture.origins.unwrap().triangles.len() as u32
    }
    pub(in crate::canonical_scene::store) fn output_corner_count(self) -> u32 {
        self.output_face_count() * 3
    }
    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> EditResult<GeometryStorage, C::Error> {
        self.fixture.materialize(control)
    }
    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: super::validation::TypedStorageValidationControl,
    >(
        self,
        actual: &GeometryStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        self.fixture.validate_correspondence(actual, control)
    }
}
#[cfg(test)]
fn same_range<T>(left: TableRange<T>, right: TableRange<T>) -> bool {
    left == right
}

fn capacity<T, E>(count: usize) -> EditResult<CapacityPlan, E> {
    CapacityPlan::for_type::<T>(0, 0, count).map_err(TypedStorageBuildError::Capacity)
}

fn allocate<T, E>(plan: CapacityPlan) -> EditResult<Vec<T>, E> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| TypedStorageBuildError::AllocationFailed)?;
    plan.finish(0, values.capacity())
        .map_err(TypedStorageBuildError::Capacity)?;
    Ok(values)
}

fn overflow<E>() -> TypedStorageBuildError<E> {
    TypedStorageBuildError::Capacity(CapacityPlanError::CapacityOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_scene::store::typed_storage::finite::FiniteScalarPool;
    use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
    use crate::canonical_scene::store::typed_storage::{
        StoredAttributeSemantic, StoredLogicalMeshIndex, StoredNodeIndex, TextRange,
    };

    const TRIANGLES: [[u32; 3]; 3] = [[0, 1, 2], [0, 2, 3], [4, 5, 6]];
    const FACE_OFFSETS: [u32; 3] = [0, 2, 3];
    const CORNER_ORIGINS: [usize; 9] = [0, 1, 2, 0, 2, 3, 4, 5, 6];

    #[derive(Default)]
    struct Control {
        completed: u64,
        strings: u64,
        reject_before: Option<u64>,
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

        fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
            self.step(action)
        }

        fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
            let result = self.step(action)?;
            self.strings += 1;
            Ok(result)
        }
    }

    impl TypedStorageValidationControl for Control {
        type Error = u64;

        fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, u64> {
            self.step(action).map(Some)
        }
    }

    fn origins() -> TriangulationOrigins<'static> {
        TriangulationOrigins {
            triangles: &TRIANGLES,
            face_offsets: &FACE_OFFSETS,
        }
    }

    fn name(names: &mut Vec<u8>, text: &str) -> TextRange {
        let first = names.len();
        names.extend_from_slice(text.as_bytes());
        TextRange(TableRange::new(first, text.len()))
    }

    /// Two source faces, one unrelated implicit triangle primitive, and ordered typed pools.
    fn geometry() -> GeometryStorage {
        let mut names = Vec::new();
        let corner_f64 = name(&mut names, "edit:corner_f64");
        let corner_sparse = name(&mut names, "edit:corner_sparse");
        let face_dense = name(&mut names, "edit:face_dense");
        let mesh_name = name(&mut names, "original mesh");
        let mut f32_values = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0,
        ];
        for corner in 0..7 {
            f32_values.extend_from_slice(&[corner as f32, -(corner as f32), 1.0]);
        }
        f32_values.extend_from_slice(&[2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 2.0, 1.0, 0.0]);
        GeometryStorage {
            integer_attribute_scalar_count: 8,
            logical_meshes: vec![StoredLogicalMesh {
                primitives: TableRange::new(0, 2),
                name: Some(mesh_name),
            }],
            primitives: vec![
                StoredPrimitive {
                    topology: TopologyKind::Polygons,
                    vertex_count: 5,
                    corner_count: 7,
                    face_count: 2,
                    polygon_boundaries: Some(TableRange::new(8, 2)),
                    indices: Some(TableRange::new(10, 7)),
                    material: crate::MaterialAssignment::Authored(0),
                    attributes: TableRange::new(0, 6),
                },
                StoredPrimitive {
                    topology: TopologyKind::TriangleList,
                    vertex_count: 3,
                    corner_count: 3,
                    face_count: 1,
                    polygon_boundaries: None,
                    indices: None,
                    material: crate::MaterialAssignment::Unspecified,
                    attributes: TableRange::new(6, 1),
                },
            ],
            attributes: vec![
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Position,
                    domain: AttributeDomain::Vertex,
                    lanes: 3,
                    element_count: 5,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(0, 15)),
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Normal,
                    domain: AttributeDomain::Corner,
                    lanes: 3,
                    element_count: 7,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(15, 21)),
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::SmoothingGroup,
                    domain: AttributeDomain::Face,
                    lanes: 1,
                    element_count: 2,
                    payload: StoredAttributePayload::SparseU32 {
                        indices: TableRange::new(0, 1),
                        values: TableRange::new(1, 1),
                        default: TableRange::new(2, 1),
                    },
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Custom(corner_f64),
                    domain: AttributeDomain::Corner,
                    lanes: 2,
                    element_count: 7,
                    payload: StoredAttributePayload::DenseF64(TableRange::new(0, 14)),
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Custom(corner_sparse),
                    domain: AttributeDomain::Corner,
                    lanes: 1,
                    element_count: 7,
                    payload: StoredAttributePayload::SparseI32 {
                        indices: TableRange::new(3, 3),
                        values: TableRange::new(0, 3),
                        default: TableRange::new(3, 1),
                    },
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Custom(face_dense),
                    domain: AttributeDomain::Face,
                    lanes: 1,
                    element_count: 2,
                    payload: StoredAttributePayload::DenseU32(TableRange::new(6, 2)),
                },
                StoredAttribute {
                    tangent_conventions: None,
                    semantic: StoredAttributeSemantic::Position,
                    domain: AttributeDomain::Vertex,
                    lanes: 3,
                    element_count: 3,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(36, 9)),
                },
            ],
            mesh_instances: vec![StoredMeshInstance {
                owner_node: StoredNodeIndex(1),
                logical_mesh: StoredLogicalMeshIndex(0),
            }],
            names,
            f32_values: FiniteScalarPool::fixture(&f32_values),
            f64_values: FiniteScalarPool::fixture(&[
                1.0e200, -0.0, 2.0e200, 0.0, 3.0e200, -0.0, 4.0e200, 0.0, 5.0e200, -0.0, 6.0e200,
                0.0, 7.0e200, -0.0,
            ]),
            u32_values: vec![1, 7, 0, 0, 3, 6, 41, 42, 4, 3, 0, 1, 2, 3, 0, 3, 4],
            i32_values: vec![0, -30, 60, 0],
        }
    }

    #[test]
    fn rewrite_preserves_vertex_identity_f64_bits_and_explicit_sparse_defaults() {
        let source = geometry();
        assert_eq!(
            source.validate(&mut Control::default(), [1, 2, 7, 1], 2, 1),
            Ok(true)
        );
        let source_values = source.f64_values.as_ptr();
        let plan =
            GeometryTriangulationPlan::try_new(&source, 0, origins(), &mut Control::default())
                .expect("source-bound replacement must plan");
        assert_eq!(plan.output_face_count(), 3);
        assert_eq!(plan.output_corner_count(), 9);
        let mut build = Control::default();
        let output = plan
            .materialize(&mut build)
            .expect("typed replacement must materialize");
        assert_eq!(build.completed, plan.work());
        assert_eq!(build.strings, source.names.len() as u64);
        let mut validation = Control::default();
        assert_eq!(
            output.validate(&mut validation, [1, 2, 7, 1], 2, 1),
            Ok(true)
        );
        assert_eq!(validation.completed, plan.validation_work());
        // Surviving validation: 1 mesh + 2 * 2 primitive visits + 7 attributes
        // + 1 instance + 14 index cells; the 69 float visits are gone.
        assert_eq!(validation.completed, 27);
        assert_eq!(source.integer_attribute_scalar_count, 8);
        assert_eq!(output.integer_attribute_scalar_count, 10);
        assert_eq!(output.primitives[0].topology, TopologyKind::TriangleList);
        assert_eq!(output.primitives[0].material, source.primitives[0].material);
        assert_eq!(output.primitives[0].vertex_count, 5);
        assert_eq!(
            output.primitives[0]
                .indices
                .unwrap()
                .get(&output.u32_values),
            Some(&[0, 1, 2, 0, 2, 3, 0, 3, 4][..])
        );
        assert_eq!(output.primitives[1].topology, TopologyKind::TriangleList);
        assert!(output.primitives[1].indices.is_none());
        assert_eq!(output.names, source.names);
        assert_eq!(output.logical_meshes[0].primitives.start(), 0);
        assert_eq!(output.logical_meshes[0].primitives.len(), 2);
        assert_eq!(
            output.mesh_instances[0].owner_node,
            source.mesh_instances[0].owner_node
        );

        for (row, expected_start, expected_length) in [(0, 0, 15), (6, 36, 9)] {
            let StoredAttributePayload::DenseF32(range) = output.attributes[row].payload else {
                panic!("Vertex positions must keep their scalar kind");
            };
            assert_eq!(
                range.get(&output.f32_values),
                Some(&source.f32_values[expected_start..expected_start + expected_length])
            );
        }
        let StoredAttributePayload::DenseF32(normals) = output.attributes[1].payload else {
            panic!("authored corner normals must remain dense F32");
        };
        let normals = normals.get(&output.f32_values).unwrap();
        for (destination, &origin) in CORNER_ORIGINS.iter().enumerate() {
            assert_eq!(
                &normals[destination * 3..destination * 3 + 3],
                &source.f32_values[15 + origin * 3..18 + origin * 3]
            );
        }
        let StoredAttributePayload::DenseF64(f64s) = output.attributes[3].payload else {
            panic!("custom F64 data must retain its scalar kind");
        };
        let f64s = f64s.get(&output.f64_values).unwrap();
        for (destination, &origin) in CORNER_ORIGINS.iter().enumerate() {
            for lane in 0..2 {
                assert_eq!(
                    f64s[destination * 2 + lane].to_bits(),
                    source.f64_values[origin * 2 + lane].to_bits()
                );
            }
        }
        let StoredAttributePayload::SparseI32 {
            indices,
            values,
            default,
        } = output.attributes[4].payload
        else {
            panic!("sparse Corner data must remain sparse");
        };
        assert_eq!(indices.get(&output.u32_values), Some(&[0, 3, 5, 8][..]));
        assert_eq!(values.get(&output.i32_values), Some(&[0, 0, -30, 60][..]));
        assert_eq!(default.get(&output.i32_values), Some(&[0][..]));
        let StoredAttributePayload::SparseU32 {
            indices,
            values,
            default,
        } = output.attributes[2].payload
        else {
            panic!("sparse Face smoothing data must remain sparse");
        };
        assert_eq!(indices.get(&output.u32_values), Some(&[2][..]));
        assert_eq!(values.get(&output.u32_values), Some(&[7][..]));
        assert_eq!(default.get(&output.u32_values), Some(&[0][..]));
        let StoredAttributePayload::DenseU32(values) = output.attributes[5].payload else {
            panic!("Face values must preserve their encoding");
        };
        assert_eq!(values.get(&output.u32_values), Some(&[41, 41, 42][..]));
        assert_eq!(source.f64_values.as_ptr(), source_values);
        assert_eq!(source.primitives[0].topology, TopologyKind::Polygons);
        assert_eq!(source.primitives[0].corner_count, 7);
        assert_eq!(
            source.u32_values,
            [1, 7, 0, 0, 3, 6, 41, 42, 4, 3, 0, 1, 2, 3, 0, 3, 4]
        );
    }

    #[test]
    fn sparse_f64_copy_and_remap_preserve_bits_explicit_defaults_and_work_counts() {
        let tuples = [
            [-0.0, f64::from_bits(1)],
            [f64::from_bits(1.0_f64.to_bits() + 1), 1.0e250],
            [0.0, -f64::from_bits(1)],
        ];
        for (domain, row, count, source_indices, expected_indices, expected_tuples) in [
            (
                AttributeDomain::Corner,
                4,
                7,
                &[0, 3, 6][..],
                &[0, 3, 5, 8][..],
                &[0, 0, 1, 2][..],
            ),
            (
                AttributeDomain::Vertex,
                4,
                5,
                &[0, 3, 4][..],
                &[0, 3, 4][..],
                &[0, 1, 2][..],
            ),
            (
                AttributeDomain::Face,
                5,
                2,
                &[0, 1][..],
                &[0, 1, 2][..],
                &[0, 0, 1][..],
            ),
        ] {
            let mut source = geometry();
            source.integer_attribute_scalar_count -= source.attributes[row]
                .payload
                .integer_value_count()
                .unwrap();
            let indices = match source.attributes[row].payload {
                StoredAttributePayload::SparseI32 { indices, .. } => {
                    source.i32_values.clear();
                    indices
                }
                StoredAttributePayload::DenseU32(indices) => indices,
                _ => unreachable!(),
            };
            let index_range = indices.checked(source.u32_values.len()).unwrap();
            source.u32_values[index_range].copy_from_slice(source_indices);
            let value_start = source.f64_values.len();
            let value_count = source_indices.len() * 2;
            let mut f64_values = source.f64_values.to_vec();
            f64_values.extend_from_slice(&tuples.as_flattened()[..value_count]);
            f64_values.extend_from_slice(&tuples[0]);
            source.f64_values = FiniteScalarPool::fixture(&f64_values);
            source.attributes[row].domain = domain;
            source.attributes[row].lanes = 2;
            source.attributes[row].element_count = count;
            source.attributes[row].payload = StoredAttributePayload::SparseF64 {
                indices,
                values: TableRange::new(value_start, value_count),
                default: TableRange::new(value_start + value_count, 2),
            };
            let source_bits: Vec<_> = source.f64_values.iter().map(|v| v.to_bits()).collect();
            assert_eq!(
                source.validate(&mut Control::default(), [1, 2, 7, 1], 2, 1),
                Ok(true)
            );
            let plan =
                GeometryTriangulationPlan::try_new(&source, 0, origins(), &mut Control::default())
                    .unwrap();
            assert_eq!(
                plan.capacities()[6].desired_capacity(),
                CORNER_ORIGINS.len() * 2 + expected_tuples.len() * 2 + 2
            );
            let mut build = Control::default();
            let output = plan.materialize(&mut build).unwrap();
            assert_eq!(build.completed, plan.work());
            let mut validation = Control::default();
            assert_eq!(
                output.validate(&mut validation, [1, 2, 7, 1], 2, 1),
                Ok(true)
            );
            assert_eq!(validation.completed, plan.validation_work());
            let mut correspondence = Control::default();
            assert_eq!(
                plan.validate_correspondence(&output, &mut correspondence),
                Ok(true)
            );
            assert_eq!(correspondence.completed, plan.correspondence_work());
            assert_eq!(
                output.attributes[row].semantic,
                source.attributes[row].semantic
            );
            assert_eq!(output.attributes[row].domain, domain);
            assert_eq!(output.attributes[row].lanes, 2);
            let StoredAttributePayload::SparseF64 {
                indices,
                values,
                default,
            } = output.attributes[row].payload
            else {
                panic!("sparse F64 must retain its encoding");
            };
            assert_eq!(indices.get(&output.u32_values), Some(expected_indices));
            assert_eq!(values.start() + values.len(), default.start());
            assert_eq!(
                values
                    .get(&output.f64_values)
                    .unwrap()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected_tuples
                    .iter()
                    .flat_map(|&index| tuples[index].map(f64::to_bits))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                default
                    .get(&output.f64_values)
                    .unwrap()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                tuples[0].map(f64::to_bits)
            );
            for scalar in [values.start(), default.start()] {
                let mut corrupted = plan.materialize(&mut Control::default()).unwrap();
                let mut f64_values = corrupted.f64_values.to_vec();
                f64_values[scalar] = 0.0;
                corrupted.f64_values = FiniteScalarPool::fixture(&f64_values);
                assert_eq!(
                    plan.validate_correspondence(&corrupted, &mut Control::default()),
                    Ok(false)
                );
            }
            assert_eq!(
                source
                    .f64_values
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                source_bits
            );
        }
    }

    #[test]
    fn origin_planning_allocates_nothing_and_rejects_cross_face_or_incomplete_relations() {
        let source = geometry();
        let mut plan = None;
        let allocations = allocation_counter::measure(|| {
            plan = Some(
                GeometryTriangulationPlan::try_new(&source, 0, origins(), &mut Control::default())
                    .expect("valid origin relation must plan"),
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        assert!(plan.is_some());

        let invalid_triangles = [
            [[0, 1, 4], [0, 2, 3], [4, 5, 6]],
            [[0, 1, 1], [0, 2, 3], [4, 5, 6]],
            [[0, 1, 2], [0, 2, 3], [4, 5, 7]],
        ];
        for triangles in &invalid_triangles {
            assert!(matches!(
                GeometryTriangulationPlan::try_new(
                    &source,
                    0,
                    TriangulationOrigins {
                        triangles,
                        face_offsets: &FACE_OFFSETS
                    },
                    &mut Control::default(),
                ),
                Err(TypedStorageBuildError::Invariant)
            ));
        }
        for offsets in [[1, 2, 3], [0, 1, 3], [0, 3, 2], [0, 2, 4]] {
            assert!(matches!(
                GeometryTriangulationPlan::try_new(
                    &source,
                    0,
                    TriangulationOrigins {
                        triangles: &TRIANGLES,
                        face_offsets: &offsets
                    },
                    &mut Control::default(),
                ),
                Err(TypedStorageBuildError::Invariant)
            ));
        }
    }

    #[test]
    fn triangulation_copies_tangent_declarations_with_corner_payloads_and_schema_text() {
        use crate::canonical_scene::store::typed_storage::{
            StoredTangentBasis, StoredTangentConventions, StoredTangentEncoding,
        };
        let basis = "test:polygon-basis";
        let encoding = "test:polygon-sign";
        let declarations = Some(StoredTangentConventions {
            basis: StoredTangentBasis::Namespaced {
                name: TextRange(TableRange::new(0, basis.len())),
                version: std::num::NonZeroU32::new(2).unwrap(),
            },
            encoding: StoredTangentEncoding::Namespaced {
                name: TextRange(TableRange::new(basis.len(), encoding.len())),
                version: std::num::NonZeroU32::new(3).unwrap(),
            },
        });
        let source = GeometryStorage {
            integer_attribute_scalar_count: 0,
            logical_meshes: vec![StoredLogicalMesh {
                primitives: TableRange::new(0, 1),
                name: None,
            }],
            primitives: vec![StoredPrimitive {
                topology: TopologyKind::Polygons,
                vertex_count: 4,
                corner_count: 4,
                face_count: 1,
                polygon_boundaries: Some(TableRange::new(0, 1)),
                indices: Some(TableRange::new(1, 4)),
                material: crate::MaterialAssignment::Unspecified,
                attributes: TableRange::new(0, 2),
            }],
            attributes: vec![
                StoredAttribute {
                    semantic: StoredAttributeSemantic::Position,
                    tangent_conventions: None,
                    domain: AttributeDomain::Vertex,
                    lanes: 3,
                    element_count: 4,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(0, 12)),
                },
                StoredAttribute {
                    semantic: StoredAttributeSemantic::Tangent,
                    tangent_conventions: declarations,
                    domain: AttributeDomain::Corner,
                    lanes: 4,
                    element_count: 4,
                    payload: StoredAttributePayload::DenseF32(TableRange::new(12, 16)),
                },
            ],
            mesh_instances: vec![],
            names: [basis.as_bytes(), encoding.as_bytes()].concat(),
            f32_values: FiniteScalarPool::fixture(&[0.0; 28]),
            f64_values: FiniteScalarPool::fixture(&[]),
            u32_values: vec![4, 0, 1, 2, 3],
            i32_values: vec![],
        };
        assert_eq!(
            source.validate(&mut Control::default(), [1, 1, 2, 0], 1, 0),
            Ok(true)
        );
        let triangles = [[0, 1, 2], [0, 2, 3]];
        let offsets = [0, 2];
        let plan = GeometryTriangulationPlan::try_new(
            &source,
            0,
            TriangulationOrigins {
                triangles: &triangles,
                face_offsets: &offsets,
            },
            &mut Control::default(),
        )
        .unwrap();
        let mut output = plan.materialize(&mut Control::default()).unwrap();
        assert_eq!(output.attributes[1].element_count, 6);
        assert_eq!(output.attributes[1].tangent_conventions, declarations);
        assert_eq!(output.names, source.names);
        assert_eq!(
            output.validate(&mut Control::default(), [1, 1, 2, 0], 1, 0),
            Ok(true)
        );
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(true)
        );
        output.attributes[1]
            .tangent_conventions
            .as_mut()
            .unwrap()
            .basis = StoredTangentBasis::Unknown;
        assert_eq!(
            plan.validate_correspondence(&output, &mut Control::default()),
            Ok(false)
        );
    }

    #[test]
    fn every_planning_and_materialization_action_is_interruptible() {
        let source = geometry();
        let mut planning = Control::default();
        let plan = GeometryTriangulationPlan::try_new(&source, 0, origins(), &mut planning)
            .expect("valid relation must plan");
        for completed in 0..planning.completed {
            let mut control = Control {
                reject_before: Some(completed),
                ..Control::default()
            };
            assert!(matches!(GeometryTriangulationPlan::try_new(
                &source, 0, origins(), &mut control,
            ), Err(TypedStorageBuildError::Control(at)) if at == completed));
            assert_eq!(control.completed, completed);
        }
        for completed in 0..plan.work() {
            let mut control = Control {
                reject_before: Some(completed),
                ..Control::default()
            };
            assert!(matches!(plan.materialize(&mut control),
                Err(TypedStorageBuildError::Control(at)) if at == completed));
            assert_eq!(control.completed, completed);
        }
        assert_eq!(source.primitives[0].topology, TopologyKind::Polygons);
        assert_eq!(source.attributes[4].element_count, 7);
    }
}
