//! Test fixture adapter. All reconstruction and correspondence use the production selection.

use super::selected::{
    GeometrySelectionPlan, LocalMapping, SceneGeometryPlanScratch, SceneGeometryReplacement,
    SceneTriangleOrigins,
};
use super::*;
use crate::identity::{LogicalMeshId, PrimitiveId, SceneGenerationId};

#[derive(Clone, Copy)]
pub(super) struct TestSelection<'a> {
    pub source: &'a GeometryStorage,
    pub target: usize,
    id: PrimitiveId,
    pub origins: Option<TriangulationOrigins<'a>>,
    pub normals: Option<VectorValues<'a, 3>>,
    pub tangents: Option<VectorValues<'a, 4>>,
    pub remove_tangents: bool,
    pub normal_domain: crate::attribute::AttributeDomain,
    pub uv_set: u32,
    pub uv_domain: crate::attribute::AttributeDomain,
    pub capacities: [CapacityPlan; 9],
    pub work: u64,
    pub validation_work: u64,
    pub mapping: LocalMapping,
}
struct Silent;
impl TypedStorageBuildControl for Silent {
    type Error = std::convert::Infallible;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        Ok(action())
    }
    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        Ok(action())
    }
}
impl<'a> TestSelection<'a> {
    pub fn description(source: &'a GeometryStorage, target: usize) -> Self {
        let mesh = source
            .logical_meshes
            .iter()
            .position(|m| {
                m.primitives
                    .checked(source.primitives.len())
                    .is_some_and(|r| r.contains(&target))
            })
            .expect("fixture primitive owner");
        let id = PrimitiveId::new(
            LogicalMeshId::new(
                SceneGenerationId::allocate().expect("fixture generation"),
                mesh as u32,
            ),
            (target - source.logical_meshes[mesh].primitives.start()) as u32,
        );
        let empty = CapacityPlan::for_type::<u8>(0, 0, 0).unwrap();
        Self {
            source,
            target,
            id,
            origins: None,
            normals: None,
            tangents: None,
            remove_tangents: false,
            normal_domain: crate::attribute::AttributeDomain::Corner,
            uv_set: 0,
            uv_domain: crate::attribute::AttributeDomain::Vertex,
            capacities: [empty; 9],
            work: 0,
            validation_work: 0,
            mapping: LocalMapping {
                row: target,
                source_start: 0,
                source_count: 0,
                output_start: 0,
                output_count: 0,
                position: None,
                normal: None,
                tangent: None,
                uv: None,
                smoothing: None,
                output_normal: None,
                output_tangent: None,
                corner_offset: 0,
                face_offset: 0,
            },
        }
    }
    fn edit(self) -> SceneGeometryReplacement<'a> {
        let mut edit = SceneGeometryReplacement::new(self.id);
        edit.origins = self.origins.map(|o| SceneTriangleOrigins {
            triangles: o.triangles,
            face_offsets: o.face_offsets,
        });
        edit.normals = self.normals;
        edit.tangents = self.tangents;
        edit.remove_tangents = self.remove_tangents;
        edit.normal_domain = self.normal_domain;
        edit.uv_set = self.uv_set;
        edit.uv_domain = self.uv_domain;
        edit
    }
    pub fn seal<C: TypedStorageBuildControl>(
        mut self,
        control: &mut C,
    ) -> EditResult<Self, C::Error> {
        let edit = [self.edit()];
        let mut mappings = [LocalMapping::EMPTY];
        let plan = GeometrySelectionPlan::try_new(
            self.source,
            &edit,
            SceneGeometryPlanScratch::single_for_test(&mut mappings),
            control,
        )?;
        self.capacities = plan.capacities();
        self.work = plan.work();
        self.validation_work = plan.validation_work();
        self.mapping = plan.mapping(0);
        Ok(self)
    }
    fn with_plan<R>(self, action: impl FnOnce(GeometrySelectionPlan<'_>) -> R) -> R {
        let edit = [self.edit()];
        let mut mappings = [LocalMapping::EMPTY];
        let plan = GeometrySelectionPlan::try_new(
            self.source,
            &edit,
            SceneGeometryPlanScratch::single_for_test(&mut mappings),
            &mut Silent,
        )
        .expect("immutable sealed fixture selection");
        action(plan)
    }
    pub fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> EditResult<GeometryStorage, C::Error> {
        self.with_plan(|p| p.materialize(control))
    }
    pub fn validate_correspondence<C: super::super::validation::TypedStorageValidationControl>(
        self,
        actual: &GeometryStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        self.with_plan(|p| p.validate_correspondence(actual, control))
    }
}
