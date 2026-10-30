use baozi_operation::{CanonicalRecordView, StoredScalar};

use crate::canonical_scene::store::typed_storage::{NodeStorage, StoredNodeIndex};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::entity::AuthoredTransform;

use super::SceneStorage;

pub(in crate::canonical_scene::store) mod animation;
pub(in crate::canonical_scene::store) mod appearance;
pub(in crate::canonical_scene::store) mod deformation;
pub(in crate::canonical_scene::store) mod geometry;
mod profile;
pub(in crate::canonical_scene::store) mod spatial;

pub(in crate::canonical_scene::store) use animation::{
    NodeSceneAnimationCursor, NodeSceneAnimationExtrapolation, NodeSceneAnimationInterpolation,
    NodeSceneAnimationTarget, NodeSceneAnimationTimeDomain,
};
pub(in crate::canonical_scene::store) use appearance::NodeSceneAppearanceCursor;
pub(in crate::canonical_scene::store) use deformation::{
    NodeSceneDeformationCursor, NodeSceneDeformationF32Cursor, NodeSceneDeformationMorphPayload,
    NodeSceneDeformationVec3Cursor,
};
pub(in crate::canonical_scene::store) use geometry::{
    NodeSceneGeometryAttributePayload, NodeSceneGeometryAttributeSemantic, NodeSceneGeometryCursor,
    NodeSceneGeometryF32Cursor, NodeSceneGeometryI32Cursor, NodeSceneGeometryNormalization,
    NodeSceneGeometryPayloadKind, NodeSceneGeometryScalarKind, NodeSceneGeometryU32Cursor,
};
pub(in crate::canonical_scene::store) use profile::{
    NodeSceneProfileCursor, NodeSceneProfileDigest, NodeSceneProfileEvidence,
    NodeSceneProfileMetadataArray, NodeSceneProfileMetadataArrayKind,
    NodeSceneProfileMetadataEntry, NodeSceneProfileMetadataValue, NodeSceneProfileOperation,
    NodeSceneProfileOperationKind, NodeSceneProfileOwner, NodeSceneProfileProvenance,
    NodeSceneProfileProvenanceIndices, NodeSceneProfileProvenanceKind, NodeSceneProfileResource,
    NodeSceneProfileResourceStability, NodeSceneProfileSource, NodeSceneProfileSourceLocator,
    NodeSceneProfileStep,
};
pub(in crate::canonical_scene::store) use spatial::{
    NodeSceneSpatialCamera, NodeSceneSpatialLight, NodeSceneSpatialPlacementCursor,
    NodeSceneSpatialPlacementsCursor, NodeSceneSpatialSceneSpace, NodeSceneSpatialTangentBasis,
    NodeSceneSpatialTangentEncoding,
};

pub(in crate::canonical_scene::store) type NodeSceneSemanticResult<T> =
    Result<T, NodeSceneSemanticError>;

#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) enum NodeSceneSemanticReadFault {
    ChildReferenceRow(usize),
    AuthoredTransformNode(usize),
}

/// A physical typed-view failure below an already validated semantic proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticError {
    reason: &'static str,
}

impl NodeSceneSemanticError {
    const fn new(reason: &'static str) -> Self {
        Self { reason }
    }

    pub(in crate::canonical_scene::store) const fn reason(&self) -> &'static str {
        self.reason
    }
}

/// One-time typed projections governed by the central validation proof.
///
/// This layer closes the constant-time physical table shape against the proof.
/// Traversal decodes only proof-bound fields and fails closed whenever a typed
/// scalar, cross-table reference, transform slice, or payload leaf cannot be
/// read mechanically.
#[must_use = "the semantic view must be traversed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticView<'scene> {
    nodes: &'scene NodeStorage,
    spatial: spatial::NodeSceneSpatialTables<'scene>,
    appearance: appearance::NodeSceneAppearanceTables<'scene>,
    geometry: geometry::NodeSceneGeometryTables<'scene>,
    deformation: deformation::NodeSceneDeformationTables<'scene>,
    animation: animation::NodeSceneAnimationTables<'scene>,
    profile: profile::NodeSceneProfileTables<'scene>,
    proof: NodeSceneValidationProof,
    #[cfg(test)]
    read_fault: Option<NodeSceneSemanticReadFault>,
}

/// One proof-qualified authored transform resolved by source-node index.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticAuthoredTransform {
    authored_transform: AuthoredTransform,
}

impl NodeSceneSemanticAuthoredTransform {
    pub(in crate::canonical_scene::store) const fn authored_transform(self) -> AuthoredTransform {
        self.authored_transform
    }
}

impl SceneStorage {
    /// Resolves fixed typed tables once; payload leaves remain row-demand driven.
    pub(in crate::canonical_scene::store) fn semantic_view(
        &self,
    ) -> NodeSceneSemanticResult<NodeSceneSemanticView<'_>> {
        NodeSceneSemanticView::try_new(self)
    }

    /// Reads one source-node transform without projecting unrelated Scene domains.
    pub(in crate::canonical_scene::store) fn semantic_authored_transform_at(
        &self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<NodeSceneSemanticAuthoredTransform> {
        if source_index >= self.proof.source_node_count() {
            return Err(invariant(
                "source node index exceeds the validation proof while reading its transform",
            ));
        }
        let row = self
            .read_nodes()
            .source_node(source_index)
            .ok_or_else(|| invariant("validated typed node row is absent"))?;
        #[cfg(test)]
        if matches!(self.semantic_read_fault, Some(NodeSceneSemanticReadFault::AuthoredTransformNode(index)) if index == source_index)
        {
            return Err(invariant("injected authored-transform read failure"));
        }
        let authored_transform = row
            .transform
            .authored()
            .ok_or_else(|| invariant("validated authored transform is invalid"))?;
        Ok(NodeSceneSemanticAuthoredTransform { authored_transform })
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_semantic_layout_fault_for_test(
        &mut self,
        source_node_count: usize,
        source_root_count: usize,
    ) -> bool {
        self.proof
            .install_layout_fault_for_test(source_node_count, source_root_count)
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_foreign_semantic_proof_for_test(
        &mut self,
        donor: &Self,
    ) {
        self.proof = donor.proof;
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_child_reference_read_fault_for_test(
        &mut self,
        row: usize,
    ) {
        self.semantic_read_fault = Some(NodeSceneSemanticReadFault::ChildReferenceRow(row));
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_authored_transform_read_fault_for_test(
        &mut self,
        source_node_index: usize,
    ) {
        self.semantic_read_fault = Some(NodeSceneSemanticReadFault::AuthoredTransformNode(
            source_node_index,
        ));
    }
}

impl<'scene> NodeSceneSemanticView<'scene> {
    fn try_new(scene: &'scene SceneStorage) -> NodeSceneSemanticResult<Self> {
        let proof = scene.proof;
        let nodes = scene.read_nodes();
        if proof.directory().into_store() != scene.canonical.root_ref()
            || nodes.nodes.len()
                != proof
                    .source_node_count()
                    .checked_add(1)
                    .ok_or_else(|| invariant("typed node count overflows"))?
            || nodes.roots().map(|roots| roots.len()) != Some(proof.source_root_count())
            || nodes.children.len() != proof.source_node_count()
        {
            return Err(invariant(
                "typed node storage disagrees with central validation proof",
            ));
        }
        let spatial = spatial::NodeSceneSpatialTables::try_new(&scene.canonical, proof)?;
        let appearance = appearance::NodeSceneAppearanceTables::try_new(&scene.canonical, proof)?;
        let geometry = geometry::NodeSceneGeometryTables::try_new(scene.read_geometry(), proof)?;
        let deformation =
            deformation::NodeSceneDeformationTables::try_new(&scene.canonical, proof)?;
        let animation = animation::NodeSceneAnimationTables::try_new(&scene.canonical, proof)?;
        let profile = profile::NodeSceneProfileTables::try_new(&scene.tables.profile)?;
        Ok(Self {
            nodes,
            spatial,
            appearance,
            geometry,
            deformation,
            animation,
            profile,
            proof,
            #[cfg(test)]
            read_fault: scene.semantic_read_fault,
        })
    }

    pub(in crate::canonical_scene::store) const fn node_count(&self) -> usize {
        self.proof.source_node_count()
    }

    pub(in crate::canonical_scene::store) const fn root_count(&self) -> usize {
        self.proof.source_root_count()
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> usize {
        self.proof.source_edge_count()
    }

    pub(in crate::canonical_scene::store) fn material_count(&self) -> usize {
        self.proof.material_count()
    }

    pub(in crate::canonical_scene::store) fn camera_count(&self) -> usize {
        self.proof.camera_count()
    }

    pub(in crate::canonical_scene::store) fn light_count(&self) -> usize {
        self.proof.light_count()
    }

    pub(in crate::canonical_scene::store) fn scene_space(
        &self,
    ) -> NodeSceneSemanticResult<NodeSceneSpatialSceneSpace<'scene>> {
        self.spatial.scene_space()
    }

    pub(in crate::canonical_scene::store) fn camera_at(
        &self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialCamera>> {
        self.spatial.camera_at(source_index)
    }

    pub(in crate::canonical_scene::store) fn light_at(
        &self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialLight>> {
        self.spatial.light_at(source_index)
    }

    /// Scans all spatial attachments once in source-node order.
    pub(in crate::canonical_scene::store) fn spatial_placements(
        &self,
    ) -> NodeSceneSpatialPlacementsCursor<'scene> {
        NodeSceneSpatialPlacementsCursor::new(self.nodes, self.proof)
    }

    /// Scans at most every source node for placements of one camera.
    pub(in crate::canonical_scene::store) fn camera_placements(
        &self,
        camera: usize,
    ) -> NodeSceneSemanticResult<NodeSceneSpatialPlacementCursor<'scene>> {
        NodeSceneSpatialPlacementCursor::camera(self.nodes, camera, self.proof)
    }

    /// Scans at most every source node for placements of one light.
    pub(in crate::canonical_scene::store) fn light_placements(
        &self,
        light: usize,
    ) -> NodeSceneSemanticResult<NodeSceneSpatialPlacementCursor<'scene>> {
        NodeSceneSpatialPlacementCursor::light(self.nodes, light, self.proof)
    }

    pub(in crate::canonical_scene::store) fn image_count(&self) -> usize {
        self.proof.image_count()
    }

    pub(in crate::canonical_scene::store) fn sampler_count(&self) -> usize {
        self.proof.sampler_count()
    }

    pub(in crate::canonical_scene::store) fn texture_count(&self) -> usize {
        self.proof.texture_count()
    }

    pub(in crate::canonical_scene::store) fn material_binding_count(&self) -> usize {
        self.proof.material_binding_count()
    }

    pub(in crate::canonical_scene::store) fn logical_mesh_count(&self) -> usize {
        self.proof.logical_mesh_count()
    }

    pub(in crate::canonical_scene::store) fn primitive_count(&self) -> usize {
        self.proof.primitive_count()
    }

    pub(in crate::canonical_scene::store) fn attribute_count(&self) -> usize {
        self.proof.attribute_count()
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_count(&self) -> usize {
        self.proof.mesh_instance_count()
    }

    pub(in crate::canonical_scene::store) fn skin_count(&self) -> usize {
        self.proof.skin_count()
    }

    pub(in crate::canonical_scene::store) fn logical_morph_count(&self) -> usize {
        self.proof.logical_morph_count()
    }

    pub(in crate::canonical_scene::store) fn morph_definition_count(&self) -> usize {
        self.proof.morph_definition_count()
    }

    pub(in crate::canonical_scene::store) fn primitive_deformation_count(&self) -> usize {
        self.proof.primitive_deformation_count()
    }

    pub(in crate::canonical_scene::store) fn morph_target_count(&self) -> usize {
        self.proof.morph_target_count()
    }

    pub(in crate::canonical_scene::store) fn morph_delta_count(&self) -> usize {
        self.proof.morph_delta_count()
    }

    pub(in crate::canonical_scene::store) fn animation_clip_count(&self) -> usize {
        self.proof.animation_clip_count()
    }

    pub(in crate::canonical_scene::store) fn animation_channel_count(&self) -> usize {
        self.proof.animation_channel_count()
    }

    pub(in crate::canonical_scene::store) fn into_cursor(
        self,
    ) -> NodeSceneSemanticResult<NodeSceneSemanticCursor<'scene>> {
        let roots = NodeSceneChildCursor::try_new(
            &self.nodes.children,
            0,
            self.proof.source_root_count(),
            self.proof.source_root_count(),
            #[cfg(test)]
            self.read_fault,
        )?;
        Ok(NodeSceneSemanticCursor {
            nodes: self.nodes,
            appearance: Some(self.appearance.into_cursor(self.proof)),
            geometry: Some(self.geometry.into_cursor(self.proof)),
            deformation: Some(self.deformation.into_cursor(self.proof)),
            animation: Some(self.animation.into_cursor(self.proof)),
            profile: Some(self.profile.into_cursor()),
            proof: self.proof,
            roots: Some(roots),
            next_node_index: 0,
            phase: NodeSceneCursorPhase::Nodes,
            #[cfg(test)]
            read_fault: self.read_fault,
        })
    }
}

/// Linear semantic traversal bounded by the immutable validation proof.
#[must_use = "the semantic cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticCursor<'scene> {
    nodes: &'scene NodeStorage,
    appearance: Option<NodeSceneAppearanceCursor<'scene>>,
    geometry: Option<NodeSceneGeometryCursor<'scene>>,
    deformation: Option<NodeSceneDeformationCursor<'scene>>,
    animation: Option<NodeSceneAnimationCursor<'scene>>,
    profile: Option<NodeSceneProfileCursor<'scene>>,
    proof: NodeSceneValidationProof,
    roots: Option<NodeSceneChildCursor<'scene>>,
    next_node_index: usize,
    phase: NodeSceneCursorPhase,
    #[cfg(test)]
    read_fault: Option<NodeSceneSemanticReadFault>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NodeSceneCursorPhase {
    Nodes,
    Appearance,
    Geometry,
    Deformation,
    Animation,
    Profile,
}

impl<'scene> NodeSceneSemanticCursor<'scene> {
    pub(in crate::canonical_scene::store) fn take_roots(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneChildCursor<'scene>> {
        self.roots
            .take()
            .ok_or_else(|| invariant("synthetic-root cursor was already taken"))
    }

    /// Returns `None` only at the proof-qualified source-node count.
    pub(in crate::canonical_scene::store) fn next_node(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSemanticNode<'scene>>> {
        if self.roots.is_some() {
            return Err(invariant(
                "synthetic-root cursor must be taken before source nodes",
            ));
        }
        if self.next_node_index == self.proof.source_node_count() {
            return Ok(None);
        }
        if self.next_node_index > self.proof.source_node_count() {
            return Err(invariant(
                "node cursor advanced beyond its proof-qualified count",
            ));
        }

        let source_index = self.next_node_index;
        let row = source_index
            .checked_add(1)
            .ok_or_else(|| invariant("canonical node row index overflows"))?;
        let node = self
            .nodes
            .nodes
            .get(row)
            .ok_or_else(|| invariant("validated typed node row is unavailable"))?;
        #[cfg(test)]
        if matches!(self.read_fault, Some(NodeSceneSemanticReadFault::AuthoredTransformNode(index)) if index == source_index)
        {
            return Err(invariant("injected authored-transform read failure"));
        }
        let authored_transform = node
            .transform
            .authored()
            .ok_or_else(|| invariant("validated authored transform is unreadable"))?;
        let name = node
            .name
            .map(|range| {
                range
                    .get(&self.nodes.names)
                    .ok_or_else(|| invariant("validated node name is unreadable"))
            })
            .transpose()?;
        let child_end = node
            .children
            .start()
            .checked_add(node.children.len())
            .ok_or_else(|| invariant("validated child range overflows"))?;
        let children = NodeSceneChildCursor::try_new(
            &self.nodes.children,
            node.children.start(),
            child_end,
            node.children.len(),
            #[cfg(test)]
            self.read_fault,
        )?;
        self.next_node_index += 1;
        Ok(Some(NodeSceneSemanticNode {
            source_index,
            name,
            authored_transform,
            camera: node.camera.map(|index| index as usize),
            light: node.light.map(|index| index as usize),
            children,
        }))
    }

    /// Advances into appearance after the node graph has been exhausted.
    pub(in crate::canonical_scene::store) fn appearance(
        &mut self,
    ) -> NodeSceneSemanticResult<&mut NodeSceneAppearanceCursor<'scene>> {
        if self.phase > NodeSceneCursorPhase::Appearance {
            return Err(invariant(
                "semantic cursor cannot return to the appearance domain",
            ));
        }
        self.ensure_node_phase_finished()?;
        self.phase = NodeSceneCursorPhase::Appearance;
        self.appearance
            .as_mut()
            .ok_or_else(|| invariant("appearance cursor was already consumed"))
    }

    /// Advances into geometry after appearance has been exhausted.
    pub(in crate::canonical_scene::store) fn geometry(
        &mut self,
    ) -> NodeSceneSemanticResult<&mut NodeSceneGeometryCursor<'scene>> {
        if self.phase > NodeSceneCursorPhase::Geometry {
            return Err(invariant(
                "semantic cursor cannot return to the geometry domain",
            ));
        }
        self.ensure_node_phase_finished()?;
        self.close_appearance_before_geometry()?;
        self.geometry
            .as_mut()
            .ok_or_else(|| invariant("geometry cursor was already consumed"))
    }

    /// Advances into deformation after geometry has been exhausted.
    pub(in crate::canonical_scene::store) fn deformation(
        &mut self,
    ) -> NodeSceneSemanticResult<&mut NodeSceneDeformationCursor<'scene>> {
        if self.phase > NodeSceneCursorPhase::Deformation {
            return Err(invariant(
                "semantic cursor cannot return to the deformation domain",
            ));
        }
        self.ensure_node_phase_finished()?;
        self.close_appearance_before_geometry()?;
        self.close_geometry_before_deformation()?;
        self.deformation
            .as_mut()
            .ok_or_else(|| invariant("deformation cursor was already consumed"))
    }

    /// Advances into animation after deformation has been exhausted.
    pub(in crate::canonical_scene::store) fn animation(
        &mut self,
    ) -> NodeSceneSemanticResult<&mut NodeSceneAnimationCursor<'scene>> {
        self.ensure_node_phase_finished()?;
        self.close_appearance_before_geometry()?;
        self.close_geometry_before_deformation()?;
        self.close_deformation_before_animation()?;
        self.animation
            .as_mut()
            .ok_or_else(|| invariant("animation cursor was already consumed"))
    }

    /// Advances into retained lineage and metadata after animation has been exhausted.
    pub(in crate::canonical_scene::store) fn profile(
        &mut self,
    ) -> NodeSceneSemanticResult<&mut NodeSceneProfileCursor<'scene>> {
        self.ensure_node_phase_finished()?;
        self.close_appearance_before_geometry()?;
        self.close_geometry_before_deformation()?;
        self.close_deformation_before_animation()?;
        self.close_animation_before_profile()?;
        self.profile
            .as_mut()
            .ok_or_else(|| invariant("profile cursor was already consumed"))
    }

    /// Closes every semantic phase reached by this proof-qualified traversal.
    pub(in crate::canonical_scene::store) fn finish(mut self) -> NodeSceneSemanticResult<()> {
        self.ensure_node_phase_finished()?;
        self.close_appearance_before_geometry()?;
        self.close_geometry_before_deformation()?;
        self.close_deformation_before_animation()?;
        self.close_animation_before_profile()?;
        self.profile
            .take()
            .ok_or_else(|| invariant("profile cursor was already consumed"))?
            .finish()?;
        Ok(())
    }

    fn ensure_node_phase_finished(&self) -> NodeSceneSemanticResult<()> {
        if self.roots.is_some() {
            return Err(invariant("synthetic-root cursor was not taken"));
        }
        if self.next_node_index != self.proof.source_node_count() {
            return Err(invariant(
                "node cursor ended before its proof-qualified count",
            ));
        }
        Ok(())
    }

    fn close_appearance_before_geometry(&mut self) -> NodeSceneSemanticResult<()> {
        if self.phase < NodeSceneCursorPhase::Geometry {
            self.appearance
                .take()
                .ok_or_else(|| invariant("appearance cursor was already consumed"))?
                .finish()?;
            self.phase = NodeSceneCursorPhase::Geometry;
        } else if self.appearance.is_some() {
            return Err(invariant(
                "appearance cursor remains live after entering geometry",
            ));
        }
        Ok(())
    }

    fn close_geometry_before_deformation(&mut self) -> NodeSceneSemanticResult<()> {
        if self.phase < NodeSceneCursorPhase::Deformation {
            self.geometry
                .take()
                .ok_or_else(|| invariant("geometry cursor was already consumed"))?
                .finish()?;
            self.phase = NodeSceneCursorPhase::Deformation;
        } else if self.geometry.is_some() {
            return Err(invariant(
                "geometry cursor remains live after entering deformation",
            ));
        }
        Ok(())
    }

    fn close_deformation_before_animation(&mut self) -> NodeSceneSemanticResult<()> {
        if self.phase < NodeSceneCursorPhase::Animation {
            self.deformation
                .take()
                .ok_or_else(|| invariant("deformation cursor was already consumed"))?
                .finish()?;
            self.phase = NodeSceneCursorPhase::Animation;
        } else if self.deformation.is_some() {
            return Err(invariant(
                "deformation cursor remains live after entering animation",
            ));
        }
        Ok(())
    }

    fn close_animation_before_profile(&mut self) -> NodeSceneSemanticResult<()> {
        if self.phase < NodeSceneCursorPhase::Profile {
            self.animation
                .take()
                .ok_or_else(|| invariant("animation cursor was already consumed"))?
                .finish()?;
            self.phase = NodeSceneCursorPhase::Profile;
        } else if self.animation.is_some() {
            return Err(invariant(
                "animation cursor remains live after entering profile",
            ));
        }
        Ok(())
    }
}

/// One decoded source node coupled to its mechanically readable child range.
#[must_use = "the decoded node owns its exact child cursor"]
pub(in crate::canonical_scene::store) struct NodeSceneSemanticNode<'scene> {
    source_index: usize,
    name: Option<&'scene str>,
    authored_transform: AuthoredTransform,
    camera: Option<usize>,
    light: Option<usize>,
    children: NodeSceneChildCursor<'scene>,
}

impl<'scene> NodeSceneSemanticNode<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn name(&self) -> Option<&'scene str> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn authored_transform(&self) -> AuthoredTransform {
        self.authored_transform
    }

    pub(in crate::canonical_scene::store) const fn camera(&self) -> Option<usize> {
        self.camera
    }

    pub(in crate::canonical_scene::store) const fn light(&self) -> Option<usize> {
        self.light
    }

    pub(in crate::canonical_scene::store) const fn child_count(&self) -> usize {
        self.children.declared_count()
    }

    pub(in crate::canonical_scene::store) fn into_children(self) -> NodeSceneChildCursor<'scene> {
        self.children
    }
}

/// Strict mechanical traversal of one proof-qualified reference range.
#[must_use = "the child cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneChildCursor<'scene> {
    references: &'scene [StoredNodeIndex],
    next_row: usize,
    end_row: usize,
    declared_count: usize,
    #[cfg(test)]
    read_fault: Option<NodeSceneSemanticReadFault>,
}

impl<'scene> NodeSceneChildCursor<'scene> {
    fn try_new(
        references: &'scene [StoredNodeIndex],
        start: usize,
        end: usize,
        declared_count: usize,
        #[cfg(test)] read_fault: Option<NodeSceneSemanticReadFault>,
    ) -> NodeSceneSemanticResult<Self> {
        if end.checked_sub(start) != Some(declared_count) {
            return Err(invariant("validated child-reference range is inconsistent"));
        }
        if end > references.len() {
            return Err(invariant("validated child range exceeds typed storage"));
        }
        Ok(Self {
            references,
            next_row: start,
            end_row: end,
            declared_count,
            #[cfg(test)]
            read_fault,
        })
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.declared_count
    }

    /// Returns `None` only at the exact end of the mechanically declared range.
    pub(in crate::canonical_scene::store) fn next(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<usize>> {
        if self.next_row == self.end_row {
            return Ok(None);
        }
        if self.next_row > self.end_row {
            return Err(invariant("child cursor advanced beyond its declared range"));
        }
        let row = self.next_row;
        #[cfg(test)]
        if matches!(
            self.read_fault,
            Some(NodeSceneSemanticReadFault::ChildReferenceRow(fault_row)) if fault_row == row
        ) {
            return Err(invariant("injected child-reference read failure"));
        }
        let child = (self
            .references
            .get(row)
            .ok_or_else(|| invariant("validated child reference is unavailable"))?
            .0 as usize)
            .checked_sub(1)
            .ok_or_else(|| invariant("synthetic root cannot be a child"))?;
        self.next_row = row
            .checked_add(1)
            .ok_or_else(|| invariant("child cursor row index overflows"))?;
        Ok(Some(child))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next_row != self.end_row {
            return Err(invariant("child cursor ended before its declared range"));
        }
        Ok(())
    }
}

fn read_u32_as_usize(
    records: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<usize> {
    let value = read_scalar::<u32>(records, row, field, reason)?;
    usize::try_from(value).map_err(|_| invariant(reason))
}

fn read_scalar<T: StoredScalar>(
    records: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<T> {
    records
        .scalar::<T>(row, field)
        .ok_or_else(|| invariant(reason))
}

const fn invariant(reason: &'static str) -> NodeSceneSemanticError {
    NodeSceneSemanticError::new(reason)
}

impl<'scene> NodeSceneSemanticView<'scene> {
    pub(in crate::canonical_scene::store) fn node_at(
        self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<NodeSceneSemanticNode<'scene>> {
        if source_index >= self.proof.source_node_count() {
            return Err(invariant("node row is outside the validated Scene"));
        }
        let mut cursor = self.into_cursor()?;
        cursor.roots = None;
        cursor.next_node_index = source_index;
        cursor
            .next_node()?
            .ok_or_else(|| invariant("node row is absent"))
    }
    pub(in crate::canonical_scene::store) fn roots(
        self,
    ) -> NodeSceneSemanticResult<NodeSceneChildCursor<'scene>> {
        self.into_cursor()?.take_roots()
    }
}
