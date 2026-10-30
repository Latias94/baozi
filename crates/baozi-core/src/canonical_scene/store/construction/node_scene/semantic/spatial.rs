use std::num::NonZeroU32;

use crate::canonical_scene::store::typed_storage::spatial::{SpatialStorage, schema_name_bytes};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::entity::{CameraProjection, LightKind};
use crate::extension::ExtensionSchemaId;
use crate::math::Vec3;
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, TangentBasisConvention, TangentHandednessEncoding, UvOrigin,
};

use super::{NodeSceneSemanticResult, invariant};

pub(super) struct NodeSceneSpatialTables<'scene> {
    storage: &'scene SpatialStorage,
}

impl<'scene> NodeSceneSpatialTables<'scene> {
    pub(super) fn try_new(
        storage: &'scene SpatialStorage,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        if storage.cameras.len() != proof.camera_count()
            || storage.lights.len() != proof.light_count()
            || schema_name_bytes(&storage.space) != proof.spatial_name_bytes()
        {
            return Err(invariant(
                "typed spatial storage disagrees with central validation proof",
            ));
        }
        Ok(Self { storage })
    }

    pub(super) fn scene_space(
        &self,
    ) -> NodeSceneSemanticResult<NodeSceneSpatialSceneSpace<'scene>> {
        let space = &self.storage.space;
        let schema = |schema: &'scene ExtensionSchemaId| NodeSceneSpatialSchemaRef {
            name: schema.name().as_str(),
            version: schema.version(),
        };
        Ok(NodeSceneSpatialSceneSpace {
            handedness: space.handedness,
            up_axis: space.up_axis,
            front_axis: space.front_axis,
            unit_scale_to_meters: space.unit_scale_to_meters,
            front_face_winding: space.front_face_winding,
            uv_origin: space.uv_origin,
            tangent_basis: match &space.tangent_basis {
                TangentBasisConvention::Unknown => NodeSceneSpatialTangentBasis::Unknown,
                TangentBasisConvention::MikkTspace => NodeSceneSpatialTangentBasis::MikkTspace,
                TangentBasisConvention::Namespaced { schema_id } => {
                    NodeSceneSpatialTangentBasis::Namespaced(schema(schema_id))
                }
            },
            tangent_encoding: match &space.tangent_handedness_encoding {
                TangentHandednessEncoding::Unknown => NodeSceneSpatialTangentEncoding::Unknown,
                TangentHandednessEncoding::SignInW => NodeSceneSpatialTangentEncoding::SignInW,
                TangentHandednessEncoding::Namespaced { schema_id } => {
                    NodeSceneSpatialTangentEncoding::Namespaced(schema(schema_id))
                }
            },
        })
    }

    pub(super) fn camera_at(
        &self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialCamera>> {
        Ok(self
            .storage
            .cameras
            .get(source_index)
            .copied()
            .map(|projection| NodeSceneSpatialCamera {
                source_index,
                projection,
            }))
    }

    pub(super) fn light_at(
        &self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialLight>> {
        Ok(self
            .storage
            .lights
            .get(source_index)
            .map(|light| NodeSceneSpatialLight {
                source_index,
                kind: light.kind,
                tint: light.tint,
                intensity: light.intensity,
            }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSpatialSchemaRef<'scene> {
    name: &'scene str,
    version: NonZeroU32,
}

impl<'scene> NodeSceneSpatialSchemaRef<'scene> {
    pub(in crate::canonical_scene::store) const fn name(self) -> &'scene str {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn version(self) -> NonZeroU32 {
        self.version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneSpatialTangentBasis<'scene> {
    Unknown,
    MikkTspace,
    Namespaced(NodeSceneSpatialSchemaRef<'scene>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneSpatialTangentEncoding<'scene> {
    Unknown,
    SignInW,
    Namespaced(NodeSceneSpatialSchemaRef<'scene>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneSpatialSceneSpace<'scene> {
    handedness: Handedness,
    up_axis: Axis,
    front_axis: Axis,
    unit_scale_to_meters: Option<f64>,
    front_face_winding: FrontFaceWinding,
    uv_origin: UvOrigin,
    tangent_basis: NodeSceneSpatialTangentBasis<'scene>,
    tangent_encoding: NodeSceneSpatialTangentEncoding<'scene>,
}

impl<'scene> NodeSceneSpatialSceneSpace<'scene> {
    pub(in crate::canonical_scene::store) const fn handedness(self) -> Handedness {
        self.handedness
    }
    pub(in crate::canonical_scene::store) const fn up_axis(self) -> Axis {
        self.up_axis
    }
    pub(in crate::canonical_scene::store) const fn front_axis(self) -> Axis {
        self.front_axis
    }
    pub(in crate::canonical_scene::store) const fn unit_scale_to_meters(self) -> Option<f64> {
        self.unit_scale_to_meters
    }
    pub(in crate::canonical_scene::store) const fn front_face_winding(self) -> FrontFaceWinding {
        self.front_face_winding
    }
    pub(in crate::canonical_scene::store) const fn uv_origin(self) -> UvOrigin {
        self.uv_origin
    }
    pub(in crate::canonical_scene::store) const fn tangent_basis(
        self,
    ) -> NodeSceneSpatialTangentBasis<'scene> {
        self.tangent_basis
    }
    pub(in crate::canonical_scene::store) const fn tangent_encoding(
        self,
    ) -> NodeSceneSpatialTangentEncoding<'scene> {
        self.tangent_encoding
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneSpatialCamera {
    source_index: usize,
    projection: CameraProjection,
}

impl NodeSceneSpatialCamera {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }
    pub(in crate::canonical_scene::store) const fn projection(self) -> CameraProjection {
        self.projection
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneSpatialLight {
    source_index: usize,
    kind: LightKind,
    tint: Vec3,
    intensity: f32,
}

impl NodeSceneSpatialLight {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }
    pub(in crate::canonical_scene::store) const fn kind(self) -> LightKind {
        self.kind
    }
    pub(in crate::canonical_scene::store) const fn tint(self) -> Vec3 {
        self.tint
    }
    pub(in crate::canonical_scene::store) const fn intensity(self) -> f32 {
        self.intensity
    }
}

#[derive(Clone, Copy)]
enum SpatialPlacementTarget {
    Camera,
    Light,
}

/// One proof-qualified node and its canonical spatial attachments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSpatialPlacement {
    source_node: usize,
    camera: Option<usize>,
    light: Option<usize>,
}

impl NodeSceneSpatialPlacement {
    pub(in crate::canonical_scene::store) const fn source_node(self) -> usize {
        self.source_node
    }

    pub(in crate::canonical_scene::store) const fn camera(self) -> Option<usize> {
        self.camera
    }

    pub(in crate::canonical_scene::store) const fn light(self) -> Option<usize> {
        self.light
    }
}

/// A single linear scan over every proof-qualified node with a spatial attachment.
#[must_use = "the spatial placements cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneSpatialPlacementsCursor<'scene> {
    nodes: &'scene crate::canonical_scene::store::typed_storage::NodeStorage,
    next_source_node: usize,
    source_node_count: usize,
}

impl<'scene> NodeSceneSpatialPlacementsCursor<'scene> {
    pub(super) const fn new(
        nodes: &'scene crate::canonical_scene::store::typed_storage::NodeStorage,
        proof: NodeSceneValidationProof,
    ) -> Self {
        Self {
            nodes,
            next_source_node: 0,
            source_node_count: proof.source_node_count(),
        }
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialPlacement>> {
        while self.next_source_node < self.source_node_count {
            let source_node = self.next_source_node;
            self.next_source_node = source_node
                .checked_add(1)
                .ok_or_else(|| invariant("spatial placements cursor index overflows"))?;
            let node = self
                .nodes
                .source_node(source_node)
                .ok_or_else(|| invariant("validated spatial placement node is absent"))?;
            let camera = node.camera.map(|index| index as usize);
            let light = node.light.map(|index| index as usize);
            if camera.is_some() || light.is_some() {
                return Ok(Some(NodeSceneSpatialPlacement {
                    source_node,
                    camera,
                    light,
                }));
            }
        }
        Ok(None)
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next_source_node != self.source_node_count {
            return Err(invariant(
                "spatial placements cursor ended before all source nodes were scanned",
            ));
        }
        Ok(())
    }
}

/// A derived reverse-placement scan over proof-qualified node attachments.
///
/// One target query scans at most every source node. Callers enumerating all targets should use the
/// bulk spatial placements cursor so the complete traversal remains linear in the node count.
#[must_use = "the spatial placement cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneSpatialPlacementCursor<'scene> {
    placements: NodeSceneSpatialPlacementsCursor<'scene>,
    target: SpatialPlacementTarget,
    target_index: usize,
}

impl<'scene> NodeSceneSpatialPlacementCursor<'scene> {
    pub(super) fn camera(
        nodes: &'scene crate::canonical_scene::store::typed_storage::NodeStorage,
        target_index: usize,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        if target_index >= proof.camera_count() {
            return Err(invariant(
                "camera placement target exceeds validation proof",
            ));
        }
        Ok(Self {
            placements: NodeSceneSpatialPlacementsCursor::new(nodes, proof),
            target: SpatialPlacementTarget::Camera,
            target_index,
        })
    }

    pub(super) fn light(
        nodes: &'scene crate::canonical_scene::store::typed_storage::NodeStorage,
        target_index: usize,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        if target_index >= proof.light_count() {
            return Err(invariant("light placement target exceeds validation proof"));
        }
        Ok(Self {
            placements: NodeSceneSpatialPlacementsCursor::new(nodes, proof),
            target: SpatialPlacementTarget::Light,
            target_index,
        })
    }

    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<usize>> {
        while let Some(placement) = self.placements.next()? {
            let target = match self.target {
                SpatialPlacementTarget::Camera => placement.camera(),
                SpatialPlacementTarget::Light => placement.light(),
            };
            if target == Some(self.target_index) {
                return Ok(Some(placement.source_node()));
            }
        }
        Ok(None)
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        self.placements.finish()
    }
}
