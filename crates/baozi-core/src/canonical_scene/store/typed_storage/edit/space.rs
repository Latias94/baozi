//! One complete spatial recipe bound to the exact immutable Scene tables.
//!
//! All domain plans use the same conversion and source root. Preflight retains only
//! borrowed plans and fixed costs; the existing Scene transaction owns admission,
//! materialization, central validation, and publication.

use baozi_operation::StorePlanError;

use crate::space_conversion::SpaceConversion;

use super::super::SceneTables;
use super::super::build::TypedStorageBuildControl;
use super::super::validation::TypedStorageValidationControl;
use super::space_animation::{AnimationSpacePlan, AnimationSpacePlanError};
use super::space_deformation::{DeformationSpacePlan, DeformationSpacePlanError};
use super::space_geometry::{GeometrySpacePlan, GeometrySpacePlanError};
use super::space_nodes::{NodeSpacePlan, NodeSpacePlanError};
use super::space_spatial::{SpatialSpacePlan, SpatialSpacePlanError};

#[derive(Debug)]
pub(in crate::canonical_scene::store) enum SceneSpacePlanError<E> {
    Nodes(NodeSpacePlanError<E>),
    Geometry(GeometrySpacePlanError<E>),
    Deformation(DeformationSpacePlanError<E>),
    Animation(AnimationSpacePlanError<E>),
    Spatial(SpatialSpacePlanError<E>),
    Plan(StorePlanError),
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct SceneSpacePlan<'a> {
    source: &'a SceneTables,
    conversion: SpaceConversion,
    nodes: NodeSpacePlan<'a>,
    geometry: GeometrySpacePlan<'a>,
    deformation: DeformationSpacePlan<'a>,
    animation: AnimationSpacePlan<'a>,
    spatial: SpatialSpacePlan<'a>,
    correspondence: u64,
    name_bytes: usize,
}

impl<'a> SceneSpacePlan<'a> {
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a SceneTables,
        conversion: SpaceConversion,
        control: &mut C,
    ) -> Result<Self, SceneSpacePlanError<C::Error>> {
        let nodes = NodeSpacePlan::try_new(source.nodes.as_ref(), conversion, control)
            .map_err(SceneSpacePlanError::Nodes)?;
        let geometry = GeometrySpacePlan::try_new(source.geometry.as_ref(), conversion, control)
            .map_err(SceneSpacePlanError::Geometry)?;
        let deformation =
            DeformationSpacePlan::try_new(source.deformation.as_ref(), conversion, control)
                .map_err(SceneSpacePlanError::Deformation)?;
        let animation = AnimationSpacePlan::try_new(source.animation.as_ref(), conversion, control)
            .map_err(SceneSpacePlanError::Animation)?;
        let spatial = SpatialSpacePlan::try_new(source.spatial.as_ref(), conversion, control)
            .map_err(SceneSpacePlanError::Spatial)?;
        let overflow = || SceneSpacePlanError::Plan(StorePlanError::SizeOverflow);
        let correspondence = nodes
            .correspondence_work()
            .checked_add(geometry.correspondence_work())
            .and_then(|work| work.checked_add(deformation.correspondence_work()))
            .and_then(|work| work.checked_add(animation.correspondence_work()))
            .and_then(|work| work.checked_add(spatial.correspondence_work()))
            .ok_or_else(overflow)?;
        let name_bytes = spatial
            .name_bytes()
            .checked_add(if conversion.changes_coordinates() {
                nodes.name_bytes()
            } else {
                0
            })
            .and_then(|bytes| {
                bytes.checked_add(if conversion.is_identity() {
                    0
                } else {
                    geometry.name_bytes()
                })
            })
            .and_then(|bytes| {
                bytes.checked_add(if deformation.changes_storage() {
                    deformation.name_bytes()
                } else {
                    0
                })
            })
            .and_then(|bytes| {
                bytes.checked_add(if animation.changes_storage() {
                    animation.name_bytes()
                } else {
                    0
                })
            })
            .ok_or_else(overflow)?;
        Ok(Self {
            source,
            conversion,
            nodes,
            geometry,
            deformation,
            animation,
            spatial,
            correspondence,
            name_bytes,
        })
    }

    pub(in crate::canonical_scene::store) fn matches_source(self, source: &SceneTables) -> bool {
        std::ptr::eq(self.source, source)
    }

    pub(in crate::canonical_scene::store) const fn conversion(self) -> SpaceConversion {
        self.conversion
    }

    pub(in crate::canonical_scene::store) const fn nodes(self) -> NodeSpacePlan<'a> {
        self.nodes
    }

    pub(in crate::canonical_scene::store) const fn geometry(self) -> GeometrySpacePlan<'a> {
        self.geometry
    }

    pub(in crate::canonical_scene::store) const fn deformation(self) -> DeformationSpacePlan<'a> {
        self.deformation
    }

    pub(in crate::canonical_scene::store) const fn animation(self) -> AnimationSpacePlan<'a> {
        self.animation
    }

    pub(in crate::canonical_scene::store) const fn spatial(self) -> SpatialSpacePlan<'a> {
        self.spatial
    }

    pub(in crate::canonical_scene::store) const fn correspondence_work(self) -> u64 {
        self.correspondence
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(self) -> usize {
        self.name_bytes
    }

    /// Includes unchanged shared domains: valid storage alone cannot prove a conversion.
    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        self,
        actual: &SceneTables,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        Ok(self
            .nodes
            .validate_correspondence(actual.nodes.as_ref(), control)?
            && self
                .geometry
                .validate_correspondence(actual.geometry.as_ref(), control)?
            && self
                .deformation
                .validate_correspondence(actual.deformation.as_ref(), control)?
            && self
                .animation
                .validate_correspondence(actual.animation.as_ref(), control)?
            && self
                .spatial
                .validate_correspondence(actual.spatial.as_ref(), control)?)
    }
}
