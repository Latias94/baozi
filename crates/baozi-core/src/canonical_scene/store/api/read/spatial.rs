use std::fmt;
use std::num::NonZeroU32;

use super::super::super::construction::node_scene::semantic::{
    NodeSceneSpatialTangentBasis, NodeSceneSpatialTangentEncoding,
};
use super::{ValidatedScene, checked_index, expect_validated};
use crate::entity::{CameraProjection, LightKind};
use crate::identity::{CameraId, LightId, SceneEntityKind, SceneLookupError};
use crate::math::Vec3;
use crate::scene_space::{Axis, FrontFaceWinding, Handedness, UvOrigin};

/// The authored tangent basis, including allocation-free borrowed extension identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TangentBasisRef<'s> {
    Unknown,
    MikkTspace,
    Namespaced { name: &'s str, version: NonZeroU32 },
}

/// The authored tangent handedness encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TangentEncodingRef<'s> {
    Unknown,
    SignInW,
    Namespaced { name: &'s str, version: NonZeroU32 },
}

/// Source space declarations. Unknown conventions remain unknown until an explicit conversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneSpaceRef<'s> {
    pub handedness: Handedness,
    pub up_axis: Axis,
    pub front_axis: Axis,
    pub unit_scale_to_meters: Option<f64>,
    pub front_face_winding: FrontFaceWinding,
    pub uv_origin: UvOrigin,
    pub tangent_basis: TangentBasisRef<'s>,
    pub tangent_handedness_encoding: TangentEncodingRef<'s>,
}

/// One camera definition. Node placements reference the definition independently.
#[derive(Clone, Copy)]
pub struct CameraRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> CameraRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }
    pub fn id(self) -> CameraId {
        CameraId::new(self.scene.generation(), self.index as u32)
    }
    pub fn projection(self) -> CameraProjection {
        expect_validated(
            expect_validated(self.scene.as_node_scene().semantic_view()).camera_at(self.index),
        )
        .expect("validated camera row is absent")
        .projection()
    }
}

/// One light definition with authored intensity and linear tint.
#[derive(Clone, Copy)]
pub struct LightRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> LightRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }
    pub fn id(self) -> LightId {
        LightId::new(self.scene.generation(), self.index as u32)
    }
    fn read(
        self,
    ) -> super::super::super::construction::node_scene::semantic::spatial::NodeSceneSpatialLight
    {
        expect_validated(
            expect_validated(self.scene.as_node_scene().semantic_view()).light_at(self.index),
        )
        .expect("validated light row is absent")
    }
    pub fn kind(self) -> LightKind {
        self.read().kind()
    }
    pub fn tint(self) -> Vec3 {
        self.read().tint()
    }
    pub fn intensity(self) -> f32 {
        self.read().intensity()
    }
}

impl fmt::Debug for CameraRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CameraRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for LightRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LightRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

impl ValidatedScene {
    pub fn space(&self) -> SceneSpaceRef<'_> {
        let space =
            expect_validated(expect_validated(self.as_node_scene().semantic_view()).scene_space());
        SceneSpaceRef {
            handedness: space.handedness(),
            up_axis: space.up_axis(),
            front_axis: space.front_axis(),
            unit_scale_to_meters: space.unit_scale_to_meters(),
            front_face_winding: space.front_face_winding(),
            uv_origin: space.uv_origin(),
            tangent_basis: match space.tangent_basis() {
                NodeSceneSpatialTangentBasis::Unknown => TangentBasisRef::Unknown,
                NodeSceneSpatialTangentBasis::MikkTspace => TangentBasisRef::MikkTspace,
                NodeSceneSpatialTangentBasis::Namespaced(schema) => TangentBasisRef::Namespaced {
                    name: schema.name(),
                    version: schema.version(),
                },
            },
            tangent_handedness_encoding: match space.tangent_encoding() {
                NodeSceneSpatialTangentEncoding::Unknown => TangentEncodingRef::Unknown,
                NodeSceneSpatialTangentEncoding::SignInW => TangentEncodingRef::SignInW,
                NodeSceneSpatialTangentEncoding::Namespaced(schema) => {
                    TangentEncodingRef::Namespaced {
                        name: schema.name(),
                        version: schema.version(),
                    }
                }
            },
        }
    }

    pub fn camera_count(&self) -> usize {
        self.as_node_scene().read_proof().camera_count()
    }
    pub fn light_count(&self) -> usize {
        self.as_node_scene().read_proof().light_count()
    }
    pub fn cameras(
        &self,
    ) -> impl ExactSizeIterator<Item = CameraRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.camera_count()).map(|index| CameraRef::at(self, index))
    }
    pub fn lights(
        &self,
    ) -> impl ExactSizeIterator<Item = LightRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.light_count()).map(|index| LightRef::at(self, index))
    }
    pub fn camera(&self, id: CameraId) -> Result<CameraRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::Camera,
            id.generation(),
            id.index(),
            self.camera_count(),
        )?;
        Ok(CameraRef::at(self, index))
    }
    pub fn light(&self, id: LightId) -> Result<LightRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::Light,
            id.generation(),
            id.index(),
            self.light_count(),
        )?;
        Ok(LightRef::at(self, index))
    }
}
