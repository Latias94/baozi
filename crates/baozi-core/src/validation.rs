use crate::material::{Material, MaterialProperty, TextureSource};
use crate::scene::SceneData;
use crate::scene_storage::scene_data_footprint_with;
use crate::{
    Animation, AnimationInterpolation, AnimationProperty, AnimationValues, Camera,
    CameraProjection, Color, Light, Mesh, MetadataKeyViolation, MetadataMap, MetadataValue,
    MetadataValueViolation, PrimitiveTopology, SCENE_VALIDATION_POLL_INTERVAL,
    SceneAnimationValueKind, SceneCameraParameter, SceneCanonicalCounts, SceneFootprint,
    SceneLightParameter, SceneMeshChannel, SceneMetadataOwner, SceneReferenceKind,
    SceneReservationKind, SceneStringOwner, SceneStringViolation, SceneValidationCode,
    SceneValidationControl, SceneValidationData, SceneValidationError, SceneValidationFailure,
    SceneValidationIssue, SceneValidationLimit, SceneValidationLimits, SceneValidationPath,
    SceneValidationStats, SceneValidationWork, TextureId, Vec2, Vec3, Vec4, VertexAttribute,
    VertexAttributeData, VertexAttributeSemantic,
};
use baozi_operation::{CapacityPlan, CapacityPlanError};
use std::error::Error;

type ValidationResult<T, C> =
    Result<T, SceneValidationFailure<<C as SceneValidationControl>::Error>>;

pub(crate) fn validate_scene_data<C: SceneValidationControl>(
    scene: &SceneData,
    limits: SceneValidationLimits,
    control: &mut C,
) -> ValidationResult<SceneValidationStats, C> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.validate").entered();

    let mut context = ValidationContext::new(limits, control);
    context.poll_now()?;
    let measurement = measure_scene(scene, &mut context)?;
    context.set_measurement(measurement)?;

    validate_root(scene, &mut context)?;
    validate_materials(scene, &mut context)?;
    validate_textures(scene, &mut context)?;
    validate_nodes(scene, &mut context)?;
    validate_skins(scene, &mut context)?;
    validate_meshes(scene, &mut context)?;
    validate_cameras(scene, &mut context)?;
    validate_lights(scene, &mut context)?;
    validate_animations(scene, &mut context)?;
    validate_space(scene, &mut context)?;
    validate_metadata_map(&scene.metadata, SceneMetadataOwner::Scene, &mut context)?;
    context.poll_now()?;
    Ok(context.stats())
}

struct ValidationContext<'a, C: SceneValidationControl> {
    control: &'a mut C,
    limits: SceneValidationLimits,
    counts: SceneCanonicalCounts,
    footprint: SceneFootprint,
    work_units: u64,
    maximum_depth: u64,
    scratch_peak_bytes: u64,
    work_since_poll: u64,
}

impl<'a, C: SceneValidationControl> ValidationContext<'a, C> {
    fn new(limits: SceneValidationLimits, control: &'a mut C) -> Self {
        Self {
            control,
            limits,
            counts: SceneCanonicalCounts::default(),
            footprint: SceneFootprint::default(),
            work_units: 0,
            maximum_depth: 0,
            scratch_peak_bytes: 0,
            work_since_poll: 0,
        }
    }

    fn work(&mut self, kind: SceneValidationWork, amount: u64) -> ValidationResult<(), C> {
        self.work_units = checked_add(self.work_units, amount, "validation work")?;
        enforce_limit(
            SceneValidationLimit::Work,
            self.work_units,
            self.limits.max_work(),
        )?;
        self.control
            .charge_work(kind, amount)
            .map_err(SceneValidationFailure::Control)?;

        let mut remaining = amount;
        while remaining > 0 {
            let until_poll = SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll;
            let chunk = remaining.min(until_poll);
            self.work_since_poll += chunk;
            remaining -= chunk;
            if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
                self.poll_now()?;
            }
        }
        Ok(())
    }

    fn poll_now(&mut self) -> ValidationResult<(), C> {
        self.control
            .poll()
            .map_err(SceneValidationFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn work_until_poll(&self) -> usize {
        usize::try_from(SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll)
            .expect("scene validation poll interval must fit usize")
    }

    fn record_depth(&mut self, depth: u64) -> ValidationResult<(), C> {
        enforce_limit(SceneValidationLimit::Depth, depth, self.limits.max_depth())?;
        self.maximum_depth = self.maximum_depth.max(depth);
        Ok(())
    }

    fn reserve_scratch(&mut self, bytes: u64) -> ValidationResult<C::Reservation, C> {
        enforce_limit(
            SceneValidationLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(bytes);
        self.control
            .reserve(SceneReservationKind::ValidationScratch, bytes)
            .map_err(SceneValidationFailure::Control)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> ValidationResult<(), C> {
        enforce_limit(
            SceneValidationLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(bytes);
        self.control
            .resize_reservation(reservation, SceneReservationKind::ValidationScratch, bytes)
            .map_err(SceneValidationFailure::Control)
    }

    fn set_measurement(&mut self, measurement: SceneMeasurement) -> ValidationResult<(), C> {
        enforce_limit(
            SceneValidationLimit::CanonicalBytes,
            measurement.footprint.accounted_payload_bytes(),
            self.limits.max_canonical_bytes(),
        )?;
        self.counts = measurement.counts;
        self.footprint = measurement.footprint;
        Ok(())
    }

    fn stats(&self) -> SceneValidationStats {
        SceneValidationStats::new(
            self.counts,
            self.footprint,
            self.work_units,
            self.maximum_depth,
            self.scratch_peak_bytes,
        )
    }
}

struct SceneMeasurement {
    counts: SceneCanonicalCounts,
    footprint: SceneFootprint,
}

fn measure_scene<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<SceneMeasurement, C> {
    let mut counts = SceneCanonicalCounts::default();
    add_limited(
        &mut counts.nodes,
        u64_len(scene.nodes.len(), "node count")?,
        SceneValidationLimit::Nodes,
        context.limits.max_nodes(),
    )?;

    let top_level_entities = [
        scene.nodes.len(),
        scene.meshes.len(),
        scene.materials.len(),
        scene.textures.len(),
        scene.animations.len(),
        scene.cameras.len(),
        scene.lights.len(),
        scene.skins.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, len| {
        checked_add(total, u64_len(len, "entity count")?, "entity count")
    })?;
    add_limited(
        &mut counts.entities,
        top_level_entities,
        SceneValidationLimit::Entities,
        context.limits.max_entities(),
    )?;

    for node in &scene.nodes {
        context.work(SceneValidationWork::Entity, 1)?;
        let edges = u64_len(node.children.len(), "node edge count")?;
        add_limited(
            &mut counts.edges,
            edges,
            SceneValidationLimit::Edges,
            context.limits.max_edges(),
        )?;
        add_references(
            &mut counts,
            edges
                + u64::from(node.parent.is_some())
                + u64_len(node.mesh_bindings.len(), "mesh binding references")?
                + u64_len(
                    node.mesh_bindings
                        .iter()
                        .filter(|binding| binding.skin.is_some())
                        .count(),
                    "skin binding references",
                )?
                + u64::from(node.camera.is_some())
                + u64::from(node.light.is_some()),
            context,
        )?;
        measure_metadata_counts(&node.metadata, &mut counts, context)?;
    }

    for mesh in &scene.meshes {
        context.work(SceneValidationWork::Entity, 1)?;
        add_references(&mut counts, u64::from(mesh.material.is_some()), context)?;
        add_entities(
            &mut counts,
            u64_len(
                mesh.morph_targets.len() + mesh.custom_attributes.len(),
                "mesh nested entities",
            )?,
            context,
        )?;
        add_payload_cells(&mut counts, mesh_payload_cells(mesh)?, context)?;
        measure_metadata_counts(&mesh.metadata, &mut counts, context)?;
        for target in &mesh.morph_targets {
            context.work(SceneValidationWork::Entity, 1)?;
            measure_metadata_counts(&target.metadata, &mut counts, context)?;
        }
        for attribute in &mesh.custom_attributes {
            context.work(SceneValidationWork::Entity, 1)?;
            measure_metadata_counts(&attribute.metadata, &mut counts, context)?;
        }
    }

    for material in &scene.materials {
        context.work(SceneValidationWork::Entity, 1)?;
        add_entities(
            &mut counts,
            u64_len(material.texture_slots.len(), "texture slot entities")?,
            context,
        )?;
        let property_texture_references = material
            .properties
            .values()
            .filter(|value| matches!(value, MaterialProperty::Texture(_)))
            .count();
        add_references(
            &mut counts,
            checked_add(
                u64_len(material.texture_slots.len(), "texture slot references")?,
                u64_len(property_texture_references, "property texture references")?,
                "material references",
            )?,
            context,
        )?;
        measure_metadata_counts(&material.metadata, &mut counts, context)?;
        measure_material_properties(material, &mut counts, context)?;
    }

    for texture in &scene.textures {
        context.work(SceneValidationWork::Entity, 1)?;
        if let TextureSource::Embedded { bytes, .. } = &texture.source {
            let bytes = u64_len(bytes.len(), "embedded texture bytes")?;
            counts.embedded_bytes =
                checked_add(counts.embedded_bytes, bytes, "embedded texture bytes")?;
            add_payload_cells(&mut counts, bytes, context)?;
        }
        measure_metadata_counts(&texture.metadata, &mut counts, context)?;
    }

    for animation in &scene.animations {
        context.work(SceneValidationWork::Entity, 1)?;
        add_entities(
            &mut counts,
            u64_len(animation.channels.len(), "animation channel entities")?,
            context,
        )?;
        add_references(
            &mut counts,
            u64_len(animation.channels.len(), "animation target references")?,
            context,
        )?;
        measure_metadata_counts(&animation.metadata, &mut counts, context)?;
        for channel in &animation.channels {
            context.work(SceneValidationWork::Entity, 1)?;
            let values = animation_value_cells(&channel.values)?;
            let payload = checked_add(
                u64_len(channel.times_seconds.len(), "animation time cells")?,
                values,
                "animation payload cells",
            )?;
            add_payload_cells(&mut counts, payload, context)?;
            measure_metadata_counts(&channel.metadata, &mut counts, context)?;
        }
    }

    for camera in &scene.cameras {
        context.work(SceneValidationWork::Entity, 1)?;
        measure_metadata_counts(&camera.metadata, &mut counts, context)?;
    }
    for light in &scene.lights {
        context.work(SceneValidationWork::Entity, 1)?;
        measure_metadata_counts(&light.metadata, &mut counts, context)?;
    }
    for skin in &scene.skins {
        context.work(SceneValidationWork::Entity, 1)?;
        add_references(
            &mut counts,
            checked_add(
                u64_len(skin.joints.len(), "skin joint references")?,
                u64::from(skin.skeleton_root.is_some()),
                "skin references",
            )?,
            context,
        )?;
        add_payload_cells(
            &mut counts,
            u64_len(
                skin.inverse_bind_matrices.len(),
                "inverse bind matrix cells",
            )?,
            context,
        )?;
        measure_metadata_counts(&skin.metadata, &mut counts, context)?;
    }
    measure_metadata_counts(&scene.metadata, &mut counts, context)?;

    let footprint = scene_data_footprint_with(scene, &mut |amount| {
        context.work(SceneValidationWork::Traversal, amount)
    })?;
    context.poll_now()?;
    Ok(SceneMeasurement { counts, footprint })
}

fn measure_metadata_counts<C: SceneValidationControl>(
    metadata: &MetadataMap,
    counts: &mut SceneCanonicalCounts,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    add_limited(
        &mut counts.metadata_entries,
        u64_len(metadata.len(), "metadata entry count")?,
        SceneValidationLimit::MetadataEntries,
        context.limits.max_metadata_entries(),
    )?;
    for (key, value) in metadata {
        context.work(SceneValidationWork::MetadataEntry, 1)?;
        let value_bytes = match value {
            MetadataValue::String(value) => value.len(),
            MetadataValue::Bool(_) | MetadataValue::I64(_) | MetadataValue::F64(_) => 0,
        };
        let bytes =
            key.len()
                .checked_add(value_bytes)
                .ok_or(SceneValidationError::AccountingOverflow {
                    resource: "metadata bytes",
                })?;
        add_limited(
            &mut counts.metadata_bytes,
            u64_len(bytes, "metadata bytes")?,
            SceneValidationLimit::MetadataBytes,
            context.limits.max_metadata_bytes(),
        )?;
    }
    Ok(())
}

fn measure_material_properties<C: SceneValidationControl>(
    material: &Material,
    counts: &mut SceneCanonicalCounts,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    add_limited(
        &mut counts.metadata_entries,
        u64_len(material.properties.len(), "material property count")?,
        SceneValidationLimit::MetadataEntries,
        context.limits.max_metadata_entries(),
    )?;
    for (key, value) in &material.properties {
        context.work(SceneValidationWork::MetadataEntry, 1)?;
        let value_bytes = match value {
            MaterialProperty::String(value) => value.len(),
            _ => 0,
        };
        let bytes =
            key.len()
                .checked_add(value_bytes)
                .ok_or(SceneValidationError::AccountingOverflow {
                    resource: "material property bytes",
                })?;
        add_limited(
            &mut counts.metadata_bytes,
            u64_len(bytes, "material property bytes")?,
            SceneValidationLimit::MetadataBytes,
            context.limits.max_metadata_bytes(),
        )?;
    }
    Ok(())
}

fn mesh_payload_cells(mesh: &Mesh) -> Result<u64, SceneValidationError> {
    let direct = [
        mesh.positions.len(),
        mesh.normals.len(),
        mesh.tangents.len(),
        mesh.indices.len(),
        mesh.face_vertex_counts.len(),
        mesh.joint_indices.len(),
        mesh.joint_weights.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, len| {
        checked_add(
            total,
            u64_len(len, "mesh payload cells")?,
            "mesh payload cells",
        )
    })?;
    let texcoords = mesh.texcoords.iter().try_fold(0_u64, |total, values| {
        checked_add(
            total,
            u64_len(values.len(), "texture coordinate cells")?,
            "texture coordinate cells",
        )
    })?;
    let colors = mesh.colors.iter().try_fold(0_u64, |total, values| {
        checked_add(total, u64_len(values.len(), "color cells")?, "color cells")
    })?;
    let morphs = mesh.morph_targets.iter().try_fold(0_u64, |total, target| {
        let target_cells = [
            target.positions.len(),
            target.normals.len(),
            target.tangents.len(),
        ]
        .into_iter()
        .try_fold(0_u64, |target_total, len| {
            checked_add(
                target_total,
                u64_len(len, "morph payload cells")?,
                "morph payload cells",
            )
        })?;
        checked_add(total, target_cells, "morph payload cells")
    })?;
    let attributes = mesh
        .custom_attributes
        .iter()
        .try_fold(0_u64, |total, attribute| {
            checked_add(
                total,
                u64_len(attribute.data.len(), "attribute payload cells")?,
                "attribute payload cells",
            )
        })?;
    [direct, texcoords, colors, morphs, attributes]
        .into_iter()
        .try_fold(0_u64, |total, value| {
            checked_add(total, value, "mesh payload cells")
        })
}

fn animation_value_cells(values: &AnimationValues) -> Result<u64, SceneValidationError> {
    let len = match values {
        AnimationValues::Translations(values) => values.len(),
        AnimationValues::Rotations(values) => values.len(),
        AnimationValues::Scales(values) => values.len(),
        AnimationValues::MorphWeights { values, .. } => values.len(),
    };
    u64_len(len, "animation value cells")
}

fn add_entities<C: SceneValidationControl>(
    counts: &mut SceneCanonicalCounts,
    amount: u64,
    context: &ValidationContext<'_, C>,
) -> Result<(), SceneValidationError> {
    add_limited(
        &mut counts.entities,
        amount,
        SceneValidationLimit::Entities,
        context.limits.max_entities(),
    )
}

fn add_references<C: SceneValidationControl>(
    counts: &mut SceneCanonicalCounts,
    amount: u64,
    context: &ValidationContext<'_, C>,
) -> Result<(), SceneValidationError> {
    add_limited(
        &mut counts.references,
        amount,
        SceneValidationLimit::References,
        context.limits.max_references(),
    )
}

fn add_payload_cells<C: SceneValidationControl>(
    counts: &mut SceneCanonicalCounts,
    amount: u64,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    add_limited(
        &mut counts.payload_cells,
        amount,
        SceneValidationLimit::PayloadCells,
        context.limits.max_payload_cells(),
    )?;
    context.work(SceneValidationWork::PayloadCell, amount)
}

fn validate_root<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    context.work(SceneValidationWork::Reference, 1)?;
    let root = scene.root.index();
    let Some(root_node) = scene.nodes.get(root) else {
        return invalid(
            SceneValidationIssue::new(SceneValidationCode::OutOfRange, SceneValidationPath::Root)
                .with_data(SceneValidationData::Index {
                    value: root,
                    upper_bound: scene.nodes.len(),
                }),
        );
    };
    if root_node.parent.is_some() {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::ParentMismatch,
            SceneValidationPath::Root,
        ));
    }
    Ok(())
}

fn validate_nodes<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (index, node) in scene.nodes.iter().enumerate() {
        context.work(SceneValidationWork::GraphNode, 1)?;
        validate_optional_string(
            node.name.as_deref(),
            SceneStringOwner::NodeName(index),
            context,
        )?;
        validate_metadata_map(&node.metadata, SceneMetadataOwner::Node(index), context)?;
        if !node.transform.is_finite() {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::NonFinite,
                SceneValidationPath::Node { node: index },
            ));
        }
        if let Some(parent) = node.parent {
            context.work(SceneValidationWork::Reference, 1)?;
            if parent.index() >= scene.nodes.len() {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::OutOfRange,
                        SceneValidationPath::NodeReference {
                            node: index,
                            reference: SceneReferenceKind::Parent,
                        },
                    )
                    .with_data(SceneValidationData::Index {
                        value: parent.index(),
                        upper_bound: scene.nodes.len(),
                    }),
                );
            }
        }
        for child in &node.children {
            context.work(SceneValidationWork::Reference, 1)?;
            let Some(child_node) = scene.nodes.get(child.index()) else {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::OutOfRange,
                        SceneValidationPath::NodeReference {
                            node: index,
                            reference: SceneReferenceKind::Child,
                        },
                    )
                    .with_data(SceneValidationData::Index {
                        value: child.index(),
                        upper_bound: scene.nodes.len(),
                    }),
                );
            };
            if child_node.parent != Some(crate::NodeId::new(index as u32)) {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::ParentMismatch,
                    SceneValidationPath::NodeChild {
                        node: index,
                        child: child.index(),
                    },
                ));
            }
        }
        for binding in &node.mesh_bindings {
            context.work(SceneValidationWork::Reference, 1)?;
            if binding.mesh.index() >= scene.meshes.len() {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::OutOfRange,
                        SceneValidationPath::NodeReference {
                            node: index,
                            reference: SceneReferenceKind::Mesh,
                        },
                    )
                    .with_data(SceneValidationData::Index {
                        value: binding.mesh.index(),
                        upper_bound: scene.meshes.len(),
                    }),
                );
            }
            if let Some(skin) = binding.skin {
                context.work(SceneValidationWork::Reference, 1)?;
                if skin.index() >= scene.skins.len() {
                    return invalid(
                        SceneValidationIssue::new(
                            SceneValidationCode::OutOfRange,
                            SceneValidationPath::NodeReference {
                                node: index,
                                reference: SceneReferenceKind::Skin,
                            },
                        )
                        .with_data(SceneValidationData::Index {
                            value: skin.index(),
                            upper_bound: scene.skins.len(),
                        }),
                    );
                }
            }
        }
        if let Some(camera) = node.camera {
            context.work(SceneValidationWork::Reference, 1)?;
            if camera.index() >= scene.cameras.len() {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::OutOfRange,
                        SceneValidationPath::NodeReference {
                            node: index,
                            reference: SceneReferenceKind::Camera,
                        },
                    )
                    .with_data(SceneValidationData::Index {
                        value: camera.index(),
                        upper_bound: scene.cameras.len(),
                    }),
                );
            }
        }
        if let Some(light) = node.light {
            context.work(SceneValidationWork::Reference, 1)?;
            if light.index() >= scene.lights.len() {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::OutOfRange,
                        SceneValidationPath::NodeReference {
                            node: index,
                            reference: SceneReferenceKind::Light,
                        },
                    )
                    .with_data(SceneValidationData::Index {
                        value: light.index(),
                        upper_bound: scene.lights.len(),
                    }),
                );
            }
        }
    }

    let state_plan =
        empty_vec_capacity_plan::<VisitState>(scene.nodes.len(), "node visitation state")?;
    let stack_plan =
        empty_vec_capacity_plan::<TraversalFrame>(scene.nodes.len(), "node traversal stack")?;
    let scratch_envelope = checked_add(
        capacity_envelope_bytes(state_plan, "node visitation state")?,
        capacity_envelope_bytes(stack_plan, "node traversal stack")?,
        "node validation scratch",
    )?;
    let mut scratch = context.reserve_scratch(scratch_envelope)?;

    let (mut state, state_bytes) =
        apply_empty_vec_capacity_plan(state_plan, "node visitation state")?;
    while state.len() < scene.nodes.len() {
        let amount = (scene.nodes.len() - state.len()).min(context.work_until_poll());
        context.work(
            SceneValidationWork::Traversal,
            u64_len(amount, "node visitation initialization")?,
        )?;
        state.extend(std::iter::repeat_n(VisitState::Unvisited, amount));
    }
    let (mut stack, stack_bytes) =
        apply_empty_vec_capacity_plan(stack_plan, "node traversal stack")?;
    let actual_scratch = checked_add(state_bytes, stack_bytes, "node validation scratch capacity")?;
    context.resize_scratch(&mut scratch, actual_scratch)?;
    let _scratch = scratch;

    let root = scene.root.index();
    state[root] = VisitState::Visiting;
    context.record_depth(1)?;
    stack.push(TraversalFrame {
        node: root,
        next_child: 0,
        depth: 1,
    });
    while let Some(frame) = stack.last_mut() {
        context.work(SceneValidationWork::Traversal, 1)?;
        let children = &scene.nodes[frame.node].children;
        let Some(child) = children.get(frame.next_child) else {
            state[frame.node] = VisitState::Visited;
            stack.pop();
            continue;
        };
        frame.next_child += 1;
        context.work(SceneValidationWork::GraphEdge, 1)?;
        let child_index = child.index();
        match state[child_index] {
            VisitState::Visiting => {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::Cycle,
                    SceneValidationPath::NodeHierarchy { node: child_index },
                ));
            }
            VisitState::Visited => {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::DuplicateReachability,
                    SceneValidationPath::NodeHierarchy { node: child_index },
                ));
            }
            VisitState::Unvisited => {
                let depth =
                    frame
                        .depth
                        .checked_add(1)
                        .ok_or(SceneValidationError::AccountingOverflow {
                            resource: "node hierarchy depth",
                        })?;
                context.record_depth(depth)?;
                state[child_index] = VisitState::Visiting;
                stack.push(TraversalFrame {
                    node: child_index,
                    next_child: 0,
                    depth,
                });
            }
        }
    }
    for (index, state) in state.iter().enumerate() {
        context.work(SceneValidationWork::GraphNode, 1)?;
        if *state == VisitState::Unvisited {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::Unreachable,
                SceneValidationPath::NodeHierarchy { node: index },
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Unvisited,
    Visiting,
    Visited,
}

struct TraversalFrame {
    node: usize,
    next_child: usize,
    depth: u64,
}

fn validate_materials<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (index, material) in scene.materials.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_material(index, material, scene.textures.len(), context)?;
    }
    Ok(())
}

fn validate_material<C: SceneValidationControl>(
    index: usize,
    material: &Material,
    texture_count: usize,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    validate_optional_string(
        material.name.as_deref(),
        SceneStringOwner::MaterialName(index),
        context,
    )?;
    validate_metadata_map(
        &material.metadata,
        SceneMetadataOwner::Material(index),
        context,
    )?;
    if !color_is_finite(material.base_color)
        || !color_is_finite(material.emissive)
        || !material.metallic.is_finite()
        || !material.roughness.is_finite()
        || !material.alpha_cutoff.is_finite()
    {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::Material { material: index },
        ));
    }

    for (slot_index, slot) in material.texture_slots.iter().enumerate() {
        context.work(SceneValidationWork::Reference, 1)?;
        validate_texture_id(
            SceneValidationPath::MaterialTextureSlot {
                material: index,
                slot: slot_index,
            },
            slot.texture,
            texture_count,
        )?;
        validate_optional_string(
            slot.source_key.as_deref(),
            SceneStringOwner::MaterialTextureSourceKey {
                material: index,
                slot: slot_index,
            },
            context,
        )?;
        if !slot.scale.is_finite()
            || !slot.transform.offset.is_finite()
            || !slot.transform.rotation_radians.is_finite()
            || !slot.transform.scale.is_finite()
        {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::NonFinite,
                SceneValidationPath::MaterialTextureSlot {
                    material: index,
                    slot: slot_index,
                },
            ));
        }
    }

    for (entry, (key, property)) in material.properties.iter().enumerate() {
        context.work(SceneValidationWork::MetadataEntry, 1)?;
        validate_namespaced_key(key, SceneMetadataOwner::Material(index), entry, context)?;
        validate_material_property(index, entry, property, texture_count, context)?;
    }
    Ok(())
}

fn validate_material_property<C: SceneValidationControl>(
    material_index: usize,
    entry: usize,
    property: &MaterialProperty,
    texture_count: usize,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    match property {
        MaterialProperty::Bool(_) | MaterialProperty::I64(_) => Ok(()),
        MaterialProperty::String(value) => validate_canonical_string(
            value,
            SceneStringOwner::MaterialPropertyString {
                material: material_index,
                entry,
            },
            context,
        ),
        MaterialProperty::F64(value) if value.is_finite() => Ok(()),
        MaterialProperty::F64(_) => invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::MaterialProperty {
                material: material_index,
                property: entry,
            },
        )),
        MaterialProperty::Color(value) if color_is_finite(*value) => Ok(()),
        MaterialProperty::Vec2(value) if value.is_finite() => Ok(()),
        MaterialProperty::Vec3(value) if value.is_finite() => Ok(()),
        MaterialProperty::Vec4(value) if value.is_finite() => Ok(()),
        MaterialProperty::Color(_)
        | MaterialProperty::Vec2(_)
        | MaterialProperty::Vec3(_)
        | MaterialProperty::Vec4(_) => invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::MaterialProperty {
                material: material_index,
                property: entry,
            },
        )),
        MaterialProperty::Texture(texture) => {
            context.work(SceneValidationWork::Reference, 1)?;
            validate_texture_id(
                SceneValidationPath::MaterialProperty {
                    material: material_index,
                    property: entry,
                },
                *texture,
                texture_count,
            )
        }
    }
}

fn validate_texture_id<E: Error + Send + Sync + 'static>(
    path: SceneValidationPath,
    texture: TextureId,
    texture_count: usize,
) -> Result<(), SceneValidationFailure<E>> {
    if texture.index() >= texture_count {
        invalid(
            SceneValidationIssue::new(SceneValidationCode::OutOfRange, path).with_data(
                SceneValidationData::Index {
                    value: texture.index(),
                    upper_bound: texture_count,
                },
            ),
        )
    } else {
        Ok(())
    }
}

fn validate_textures<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (index, texture) in scene.textures.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_optional_string(
            texture.name.as_deref(),
            SceneStringOwner::TextureName(index),
            context,
        )?;
        match &texture.source {
            TextureSource::External { uri } => {
                if uri.is_empty() {
                    return invalid(SceneValidationIssue::new(
                        SceneValidationCode::Empty,
                        SceneValidationPath::Texture { texture: index },
                    ));
                }
                validate_canonical_string(
                    uri,
                    SceneStringOwner::TextureExternalUri(index),
                    context,
                )?;
            }
            TextureSource::Embedded { bytes, mime_type } => {
                context.work(
                    SceneValidationWork::PayloadCell,
                    u64_len(bytes.len(), "embedded texture bytes")?,
                )?;
                validate_optional_string(
                    mime_type.as_deref(),
                    SceneStringOwner::TextureMimeType(index),
                    context,
                )?;
            }
        }
        validate_metadata_map(
            &texture.metadata,
            SceneMetadataOwner::Texture(index),
            context,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct MeshBindingUsage {
    minimum_joint_count: Option<usize>,
    has_skinned_binding: bool,
    has_unskinned_binding: bool,
}

struct ReservedMeshBindingUsage<R> {
    values: Vec<MeshBindingUsage>,
    _reservation: R,
}

impl<R> ReservedMeshBindingUsage<R> {
    fn get(&self, index: usize) -> &MeshBindingUsage {
        &self.values[index]
    }
}

fn mesh_binding_usage<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<ReservedMeshBindingUsage<C::Reservation>, C> {
    let usage_plan = empty_vec_capacity_plan::<MeshBindingUsage>(
        scene.meshes.len(),
        "mesh binding validation state",
    )?;
    let scratch_envelope = capacity_envelope_bytes(usage_plan, "mesh binding validation state")?;
    let mut reservation = context.reserve_scratch(scratch_envelope)?;
    let (mut usage, actual_scratch) =
        apply_empty_vec_capacity_plan(usage_plan, "mesh binding validation state")?;
    context.resize_scratch(&mut reservation, actual_scratch)?;
    while usage.len() < scene.meshes.len() {
        let amount = (scene.meshes.len() - usage.len()).min(context.work_until_poll());
        context.work(
            SceneValidationWork::Traversal,
            u64_len(amount, "mesh binding state initialization")?,
        )?;
        usage.extend(std::iter::repeat_n(MeshBindingUsage::default(), amount));
    }
    for node in &scene.nodes {
        for binding in &node.mesh_bindings {
            context.work(SceneValidationWork::Reference, 1)?;
            let mesh_usage = &mut usage[binding.mesh.index()];
            if let Some(skin_id) = binding.skin {
                let skin = &scene.skins[skin_id.index()];
                mesh_usage.has_skinned_binding = true;
                mesh_usage.minimum_joint_count = Some(
                    mesh_usage
                        .minimum_joint_count
                        .map_or(skin.joints.len(), |count| count.min(skin.joints.len())),
                );
            } else {
                mesh_usage.has_unskinned_binding = true;
            }
        }
    }
    Ok(ReservedMeshBindingUsage {
        values: usage,
        _reservation: reservation,
    })
}

fn validate_meshes<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    let binding_usage = mesh_binding_usage(scene, context)?;
    for (index, mesh) in scene.meshes.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_mesh(
            index,
            mesh,
            scene.materials.len(),
            binding_usage.get(index),
            context,
        )?;
    }
    Ok(())
}

fn validate_mesh<C: SceneValidationControl>(
    index: usize,
    mesh: &Mesh,
    material_count: usize,
    binding_usage: &MeshBindingUsage,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    validate_optional_string(
        mesh.name.as_deref(),
        SceneStringOwner::MeshName(index),
        context,
    )?;
    validate_metadata_map(&mesh.metadata, SceneMetadataOwner::Mesh(index), context)?;
    let vertex_count = mesh.positions.len();
    if vertex_count == 0 {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::Empty,
            SceneValidationPath::MeshChannel {
                mesh: index,
                channel: SceneMeshChannel::Positions,
            },
        ));
    }
    if let Some(material) = mesh.material {
        context.work(SceneValidationWork::Reference, 1)?;
        if material.index() >= material_count {
            return invalid(
                SceneValidationIssue::new(
                    SceneValidationCode::OutOfRange,
                    SceneValidationPath::MeshReference {
                        mesh: index,
                        reference: SceneReferenceKind::Material,
                    },
                )
                .with_data(SceneValidationData::Index {
                    value: material.index(),
                    upper_bound: material_count,
                }),
            );
        }
    }

    validate_vec3_channel(
        index,
        SceneMeshChannel::Positions,
        &mesh.positions,
        Some(vertex_count),
        context,
    )?;
    validate_vec3_channel(
        index,
        SceneMeshChannel::Normals,
        &mesh.normals,
        optional_len(vertex_count, &mesh.normals),
        context,
    )?;
    validate_vec4_channel(
        index,
        SceneMeshChannel::Tangents,
        &mesh.tangents,
        optional_len(vertex_count, &mesh.tangents),
        context,
    )?;
    for (channel, texcoords) in mesh.texcoords.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_vec2_channel(
            index,
            SceneMeshChannel::TextureCoordinates(channel),
            texcoords,
            Some(vertex_count),
            context,
        )?;
    }
    for (channel, colors) in mesh.colors.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_color_channel(
            index,
            SceneMeshChannel::Colors(channel),
            colors,
            Some(vertex_count),
            context,
        )?;
    }
    validate_joint_channels(index, vertex_count, mesh, binding_usage, context)?;

    for (target_index, target) in mesh.morph_targets.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_optional_string(
            target.name.as_deref(),
            SceneStringOwner::MorphTargetName {
                mesh: index,
                target: target_index,
            },
            context,
        )?;
        validate_metadata_map(
            &target.metadata,
            SceneMetadataOwner::MorphTarget {
                mesh: index,
                target: target_index,
            },
            context,
        )?;
        if target.positions.is_empty() && target.normals.is_empty() && target.tangents.is_empty() {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::Empty,
                SceneValidationPath::MeshMorphTarget {
                    mesh: index,
                    target: target_index,
                },
            ));
        }
        validate_vec3_channel(
            index,
            SceneMeshChannel::MorphPositions(target_index),
            &target.positions,
            optional_len(vertex_count, &target.positions),
            context,
        )?;
        validate_vec3_channel(
            index,
            SceneMeshChannel::MorphNormals(target_index),
            &target.normals,
            optional_len(vertex_count, &target.normals),
            context,
        )?;
        validate_vec4_channel(
            index,
            SceneMeshChannel::MorphTangents(target_index),
            &target.tangents,
            optional_len(vertex_count, &target.tangents),
            context,
        )?;
    }
    for (attribute_index, attribute) in mesh.custom_attributes.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_vertex_attribute(index, attribute_index, vertex_count, attribute, context)?;
    }
    for index_value in &mesh.indices {
        context.work(SceneValidationWork::PayloadCell, 1)?;
        if *index_value as usize >= vertex_count {
            return invalid(
                SceneValidationIssue::new(
                    SceneValidationCode::OutOfRange,
                    SceneValidationPath::MeshElement {
                        mesh: index,
                        element: *index_value as usize,
                    },
                )
                .with_data(SceneValidationData::Index {
                    value: *index_value as usize,
                    upper_bound: vertex_count,
                }),
            );
        }
    }
    validate_topology(
        index,
        mesh.topology,
        mesh.element_count(),
        &mesh.face_vertex_counts,
        context,
    )?;
    if let Some(bounds) = mesh.bounds {
        if !bounds.min.is_finite() || !bounds.max.is_finite() {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::NonFinite,
                SceneValidationPath::MeshBounds { mesh: index },
            ));
        }
        if bounds.min.x > bounds.max.x || bounds.min.y > bounds.max.y || bounds.min.z > bounds.max.z
        {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::OrderViolation,
                SceneValidationPath::MeshBounds { mesh: index },
            ));
        }
    }
    Ok(())
}

fn validate_joint_channels<C: SceneValidationControl>(
    mesh_index: usize,
    vertex_count: usize,
    mesh: &Mesh,
    binding_usage: &MeshBindingUsage,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    if mesh.joint_indices.is_empty() && mesh.joint_weights.is_empty() {
        return Ok(());
    }
    if mesh.joint_indices.len() != vertex_count || mesh.joint_weights.len() != vertex_count {
        return invalid(
            SceneValidationIssue::new(
                SceneValidationCode::LengthMismatch,
                SceneValidationPath::MeshJointChannels { mesh: mesh_index },
            )
            .with_data(SceneValidationData::JointChannelCounts {
                indices: mesh.joint_indices.len(),
                weights: mesh.joint_weights.len(),
                positions: vertex_count,
            }),
        );
    }
    if !binding_usage.has_skinned_binding {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::RequiresSkin,
            SceneValidationPath::MeshJointChannels { mesh: mesh_index },
        ));
    }
    if binding_usage.has_unskinned_binding {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::ConflictingBinding,
            SceneValidationPath::MeshJointChannels { mesh: mesh_index },
        ));
    }
    let minimum_joint_count = binding_usage.minimum_joint_count.unwrap_or(0);
    for (vertex, joints) in mesh.joint_indices.iter().enumerate() {
        for joint in joints {
            context.work(SceneValidationWork::PayloadCell, 1)?;
            if *joint as usize >= minimum_joint_count {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::OutOfRange,
                        SceneValidationPath::MeshJoint {
                            mesh: mesh_index,
                            vertex,
                            joint: *joint as usize,
                        },
                    )
                    .with_data(SceneValidationData::Index {
                        value: *joint as usize,
                        upper_bound: minimum_joint_count,
                    }),
                );
            }
        }
    }
    for (vertex, weights) in mesh.joint_weights.iter().enumerate() {
        if contains_invalid(weights, context, |weight| !weight.is_finite())? {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::NonFinite,
                SceneValidationPath::MeshJointWeights {
                    mesh: mesh_index,
                    vertex,
                },
            ));
        }
    }
    Ok(())
}

fn validate_vertex_attribute<C: SceneValidationControl>(
    mesh_index: usize,
    attribute_index: usize,
    vertex_count: usize,
    attribute: &VertexAttribute,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    validate_namespaced_key(
        &attribute.name,
        SceneMetadataOwner::VertexAttribute {
            mesh: mesh_index,
            attribute: attribute_index,
        },
        0,
        context,
    )?;
    if let VertexAttributeSemantic::Custom(value) = &attribute.semantic {
        validate_canonical_string(
            value,
            SceneStringOwner::VertexAttributeSemantic {
                mesh: mesh_index,
                attribute: attribute_index,
            },
            context,
        )?;
    }
    validate_metadata_map(
        &attribute.metadata,
        SceneMetadataOwner::VertexAttribute {
            mesh: mesh_index,
            attribute: attribute_index,
        },
        context,
    )?;
    validate_channel_len(
        mesh_index,
        SceneMeshChannel::CustomAttribute(attribute_index),
        attribute.data.len(),
        Some(vertex_count),
    )?;
    let non_finite = match &attribute.data {
        VertexAttributeData::F32(values) => {
            contains_invalid(values, context, |value| !value.is_finite())?
        }
        VertexAttributeData::Vec2(values) => {
            contains_invalid(values, context, |value| !value.is_finite())?
        }
        VertexAttributeData::Vec3(values) => {
            contains_invalid(values, context, |value| !value.is_finite())?
        }
        VertexAttributeData::Vec4(values) => {
            contains_invalid(values, context, |value| !value.is_finite())?
        }
        VertexAttributeData::U16x4(values) => {
            context.work(
                SceneValidationWork::PayloadCell,
                checked_mul(
                    u64_len(values.len(), "u16x4 attribute cells")?,
                    4,
                    "u16x4 attribute cells",
                )?,
            )?;
            false
        }
        VertexAttributeData::U32(values) => {
            context.work(
                SceneValidationWork::PayloadCell,
                u64_len(values.len(), "u32 attribute cells")?,
            )?;
            false
        }
        VertexAttributeData::I32(values) => {
            context.work(
                SceneValidationWork::PayloadCell,
                u64_len(values.len(), "i32 attribute cells")?,
            )?;
            false
        }
    };
    if non_finite {
        invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::MeshChannel {
                mesh: mesh_index,
                channel: SceneMeshChannel::CustomAttribute(attribute_index),
            },
        ))
    } else {
        Ok(())
    }
}

fn optional_len<T>(vertex_count: usize, values: &[T]) -> Option<usize> {
    (!values.is_empty()).then_some(vertex_count)
}

fn validate_vec2_channel<C: SceneValidationControl>(
    mesh_index: usize,
    channel: SceneMeshChannel,
    values: &[Vec2],
    expected_len: Option<usize>,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    validate_channel_len(mesh_index, channel, values.len(), expected_len)?;
    if contains_invalid(values, context, |value| !value.is_finite())? {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::MeshChannel {
                mesh: mesh_index,
                channel,
            },
        ));
    }
    Ok(())
}

fn validate_vec3_channel<C: SceneValidationControl>(
    mesh_index: usize,
    channel: SceneMeshChannel,
    values: &[Vec3],
    expected_len: Option<usize>,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    validate_channel_len(mesh_index, channel, values.len(), expected_len)?;
    if contains_invalid(values, context, |value| !value.is_finite())? {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::MeshChannel {
                mesh: mesh_index,
                channel,
            },
        ));
    }
    Ok(())
}

fn validate_vec4_channel<C: SceneValidationControl>(
    mesh_index: usize,
    channel: SceneMeshChannel,
    values: &[Vec4],
    expected_len: Option<usize>,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    validate_channel_len(mesh_index, channel, values.len(), expected_len)?;
    if contains_invalid(values, context, |value| !value.is_finite())? {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::MeshChannel {
                mesh: mesh_index,
                channel,
            },
        ));
    }
    Ok(())
}

fn validate_color_channel<C: SceneValidationControl>(
    mesh_index: usize,
    channel: SceneMeshChannel,
    values: &[Color],
    expected_len: Option<usize>,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    validate_channel_len(mesh_index, channel, values.len(), expected_len)?;
    if contains_invalid(values, context, |value| !color_is_finite(*value))? {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::MeshChannel {
                mesh: mesh_index,
                channel,
            },
        ));
    }
    Ok(())
}

fn validate_channel_len<E: Error + Send + Sync + 'static>(
    mesh_index: usize,
    channel: SceneMeshChannel,
    actual_len: usize,
    expected_len: Option<usize>,
) -> Result<(), SceneValidationFailure<E>> {
    if let Some(expected_len) = expected_len
        && actual_len != expected_len
    {
        return invalid(
            SceneValidationIssue::new(
                SceneValidationCode::LengthMismatch,
                SceneValidationPath::MeshChannel {
                    mesh: mesh_index,
                    channel,
                },
            )
            .with_data(SceneValidationData::Counts {
                actual: actual_len,
                expected: expected_len,
            }),
        );
    }
    Ok(())
}

fn validate_topology<C: SceneValidationControl>(
    mesh_index: usize,
    topology: PrimitiveTopology,
    element_count: usize,
    face_vertex_counts: &[u32],
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    match topology {
        PrimitiveTopology::Points | PrimitiveTopology::Lines | PrimitiveTopology::Triangles
            if !face_vertex_counts.is_empty() =>
        {
            invalid(SceneValidationIssue::new(
                SceneValidationCode::UnexpectedData,
                SceneValidationPath::Mesh { mesh: mesh_index },
            ))
        }
        PrimitiveTopology::Points => Ok(()),
        PrimitiveTopology::Lines if element_count.is_multiple_of(2) => Ok(()),
        PrimitiveTopology::Triangles if element_count.is_multiple_of(3) => Ok(()),
        PrimitiveTopology::Lines => invalid(
            SceneValidationIssue::new(
                SceneValidationCode::NotDivisible,
                SceneValidationPath::Mesh { mesh: mesh_index },
            )
            .with_data(SceneValidationData::Divisibility {
                count: element_count,
                divisor: 2,
            }),
        ),
        PrimitiveTopology::Triangles => invalid(
            SceneValidationIssue::new(
                SceneValidationCode::NotDivisible,
                SceneValidationPath::Mesh { mesh: mesh_index },
            )
            .with_data(SceneValidationData::Divisibility {
                count: element_count,
                divisor: 3,
            }),
        ),
        PrimitiveTopology::Polygons => {
            validate_polygon_faces(mesh_index, element_count, face_vertex_counts, context)
        }
    }
}

fn validate_polygon_faces<C: SceneValidationControl>(
    mesh_index: usize,
    element_count: usize,
    face_vertex_counts: &[u32],
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    if face_vertex_counts.is_empty() {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::MissingRequiredData,
            SceneValidationPath::Mesh { mesh: mesh_index },
        ));
    }
    let mut total = 0usize;
    for (face_index, count) in face_vertex_counts.iter().copied().enumerate() {
        context.work(SceneValidationWork::PayloadCell, 1)?;
        if count < 3 {
            return invalid(
                SceneValidationIssue::new(
                    SceneValidationCode::MinimumNotMet,
                    SceneValidationPath::MeshPolygonFace {
                        mesh: mesh_index,
                        face: face_index,
                    },
                )
                .with_data(SceneValidationData::Minimum {
                    actual: count as usize,
                    minimum: 3,
                }),
            );
        }
        total =
            total
                .checked_add(count as usize)
                .ok_or(SceneValidationError::AccountingOverflow {
                    resource: "polygon face counts",
                })?;
    }
    if total != element_count {
        return invalid(
            SceneValidationIssue::new(
                SceneValidationCode::LengthMismatch,
                SceneValidationPath::Mesh { mesh: mesh_index },
            )
            .with_data(SceneValidationData::Counts {
                actual: total,
                expected: element_count,
            }),
        );
    }
    Ok(())
}

fn validate_skins<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (index, skin) in scene.skins.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_optional_string(
            skin.name.as_deref(),
            SceneStringOwner::SkinName(index),
            context,
        )?;
        validate_metadata_map(&skin.metadata, SceneMetadataOwner::Skin(index), context)?;
        if let Some(root) = skin.skeleton_root {
            context.work(SceneValidationWork::Reference, 1)?;
            if root.index() >= scene.nodes.len() {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::OutOfRange,
                        SceneValidationPath::SkinReference {
                            skin: index,
                            reference: SceneReferenceKind::SkeletonRoot,
                        },
                    )
                    .with_data(SceneValidationData::Index {
                        value: root.index(),
                        upper_bound: scene.nodes.len(),
                    }),
                );
            }
        }
        for joint in &skin.joints {
            context.work(SceneValidationWork::Reference, 1)?;
            if joint.index() >= scene.nodes.len() {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::OutOfRange,
                        SceneValidationPath::SkinReference {
                            skin: index,
                            reference: SceneReferenceKind::Joint,
                        },
                    )
                    .with_data(SceneValidationData::Index {
                        value: joint.index(),
                        upper_bound: scene.nodes.len(),
                    }),
                );
            }
        }
        if !skin.inverse_bind_matrices.is_empty()
            && skin.inverse_bind_matrices.len() != skin.joints.len()
        {
            return invalid(
                SceneValidationIssue::new(
                    SceneValidationCode::LengthMismatch,
                    SceneValidationPath::Skin { skin: index },
                )
                .with_data(SceneValidationData::Counts {
                    actual: skin.inverse_bind_matrices.len(),
                    expected: skin.joints.len(),
                }),
            );
        }
        if contains_invalid(&skin.inverse_bind_matrices, context, |matrix| {
            !matrix.is_finite()
        })? {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::NonFinite,
                SceneValidationPath::Skin { skin: index },
            ));
        }
    }
    Ok(())
}

fn validate_cameras<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (index, camera) in scene.cameras.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_optional_string(
            camera.name.as_deref(),
            SceneStringOwner::CameraName(index),
            context,
        )?;
        validate_metadata_map(&camera.metadata, SceneMetadataOwner::Camera(index), context)?;
        validate_camera(index, camera)?;
    }
    Ok(())
}

fn validate_camera<E: Error + Send + Sync + 'static>(
    index: usize,
    camera: &Camera,
) -> Result<(), SceneValidationFailure<E>> {
    match camera.projection {
        CameraProjection::Perspective {
            yfov_radians,
            aspect_ratio,
            znear,
            zfar,
        } => {
            if !yfov_radians.is_finite() || yfov_radians <= 0.0 {
                return invalid(SceneValidationIssue::new(
                    if yfov_radians.is_finite() {
                        SceneValidationCode::NonPositive
                    } else {
                        SceneValidationCode::NonFinite
                    },
                    SceneValidationPath::Camera {
                        camera: index,
                        parameter: SceneCameraParameter::FieldOfView,
                    },
                ));
            }
            if let Some(aspect_ratio) = aspect_ratio
                && (!aspect_ratio.is_finite() || aspect_ratio <= 0.0)
            {
                return invalid(SceneValidationIssue::new(
                    if aspect_ratio.is_finite() {
                        SceneValidationCode::NonPositive
                    } else {
                        SceneValidationCode::NonFinite
                    },
                    SceneValidationPath::Camera {
                        camera: index,
                        parameter: SceneCameraParameter::AspectRatio,
                    },
                ));
            }
            validate_positive_depth(index, SceneCameraParameter::NearDepth, znear)?;
            if let Some(zfar) = zfar {
                validate_positive_depth(index, SceneCameraParameter::FarDepth, zfar)?;
                if zfar <= znear {
                    return invalid(SceneValidationIssue::new(
                        SceneValidationCode::OrderViolation,
                        SceneValidationPath::Camera {
                            camera: index,
                            parameter: SceneCameraParameter::DepthRange,
                        },
                    ));
                }
            }
        }
        CameraProjection::Orthographic {
            xmag,
            ymag,
            znear,
            zfar,
        } => {
            if !xmag.is_finite() || xmag <= 0.0 || !ymag.is_finite() || ymag <= 0.0 {
                return invalid(SceneValidationIssue::new(
                    if xmag.is_finite() && ymag.is_finite() {
                        SceneValidationCode::NonPositive
                    } else {
                        SceneValidationCode::NonFinite
                    },
                    SceneValidationPath::Camera {
                        camera: index,
                        parameter: SceneCameraParameter::Magnification,
                    },
                ));
            }
            validate_positive_depth(index, SceneCameraParameter::NearDepth, znear)?;
            validate_positive_depth(index, SceneCameraParameter::FarDepth, zfar)?;
            if zfar <= znear {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::OrderViolation,
                    SceneValidationPath::Camera {
                        camera: index,
                        parameter: SceneCameraParameter::DepthRange,
                    },
                ));
            }
        }
        CameraProjection::Unknown => {}
    }
    Ok(())
}

fn validate_positive_depth<E: Error + Send + Sync + 'static>(
    camera_index: usize,
    parameter: SceneCameraParameter,
    value: f32,
) -> Result<(), SceneValidationFailure<E>> {
    if !value.is_finite() || value <= 0.0 {
        invalid(SceneValidationIssue::new(
            if value.is_finite() {
                SceneValidationCode::NonPositive
            } else {
                SceneValidationCode::NonFinite
            },
            SceneValidationPath::Camera {
                camera: camera_index,
                parameter,
            },
        ))
    } else {
        Ok(())
    }
}

fn validate_lights<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (index, light) in scene.lights.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_optional_string(
            light.name.as_deref(),
            SceneStringOwner::LightName(index),
            context,
        )?;
        validate_metadata_map(&light.metadata, SceneMetadataOwner::Light(index), context)?;
        validate_light(index, light)?;
    }
    Ok(())
}

fn validate_light<E: Error + Send + Sync + 'static>(
    index: usize,
    light: &Light,
) -> Result<(), SceneValidationFailure<E>> {
    if !color_is_finite(light.color) || !light.intensity.is_finite() {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::NonFinite,
            SceneValidationPath::Light {
                light: index,
                parameter: SceneLightParameter::Value,
            },
        ));
    }
    if light.intensity < 0.0 {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::Negative,
            SceneValidationPath::Light {
                light: index,
                parameter: SceneLightParameter::Intensity,
            },
        ));
    }
    if let Some(range) = light.range
        && (!range.is_finite() || range < 0.0)
    {
        return invalid(SceneValidationIssue::new(
            if range.is_finite() {
                SceneValidationCode::Negative
            } else {
                SceneValidationCode::NonFinite
            },
            SceneValidationPath::Light {
                light: index,
                parameter: SceneLightParameter::Range,
            },
        ));
    }
    if let Some(angle) = light.inner_cone_angle
        && (!angle.is_finite() || angle < 0.0)
    {
        return invalid(SceneValidationIssue::new(
            if angle.is_finite() {
                SceneValidationCode::Negative
            } else {
                SceneValidationCode::NonFinite
            },
            SceneValidationPath::Light {
                light: index,
                parameter: SceneLightParameter::InnerConeAngle,
            },
        ));
    }
    if let Some(angle) = light.outer_cone_angle
        && (!angle.is_finite() || angle < 0.0)
    {
        return invalid(SceneValidationIssue::new(
            if angle.is_finite() {
                SceneValidationCode::Negative
            } else {
                SceneValidationCode::NonFinite
            },
            SceneValidationPath::Light {
                light: index,
                parameter: SceneLightParameter::OuterConeAngle,
            },
        ));
    }
    if let (Some(inner), Some(outer)) = (light.inner_cone_angle, light.outer_cone_angle)
        && inner > outer
    {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::OrderViolation,
            SceneValidationPath::Light {
                light: index,
                parameter: SceneLightParameter::ConeAngleOrder,
            },
        ));
    }
    Ok(())
}

fn validate_animations<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (index, animation) in scene.animations.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_optional_string(
            animation.name.as_deref(),
            SceneStringOwner::AnimationName(index),
            context,
        )?;
        validate_metadata_map(
            &animation.metadata,
            SceneMetadataOwner::Animation(index),
            context,
        )?;
        validate_animation(index, animation, scene.nodes.len(), context)?;
    }
    Ok(())
}

fn validate_animation<C: SceneValidationControl>(
    animation_index: usize,
    animation: &Animation,
    node_count: usize,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (channel_index, channel) in animation.channels.iter().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_metadata_map(
            &channel.metadata,
            SceneMetadataOwner::AnimationChannel {
                animation: animation_index,
                channel: channel_index,
            },
            context,
        )?;
        context.work(SceneValidationWork::Reference, 1)?;
        if channel.target.node.index() >= node_count {
            return invalid(
                SceneValidationIssue::new(
                    SceneValidationCode::OutOfRange,
                    SceneValidationPath::AnimationValues {
                        animation: animation_index,
                        channel: channel_index,
                        values: SceneAnimationValueKind::Target,
                    },
                )
                .with_data(SceneValidationData::Index {
                    value: channel.target.node.index(),
                    upper_bound: node_count,
                }),
            );
        }
        if channel.times_seconds.is_empty() {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::Empty,
                SceneValidationPath::AnimationValues {
                    animation: animation_index,
                    channel: channel_index,
                    values: SceneAnimationValueKind::Times,
                },
            ));
        }
        if contains_invalid(&channel.times_seconds, context, |time| !time.is_finite())? {
            return invalid(SceneValidationIssue::new(
                SceneValidationCode::NonFinite,
                SceneValidationPath::AnimationValues {
                    animation: animation_index,
                    channel: channel_index,
                    values: SceneAnimationValueKind::Times,
                },
            ));
        }
        for window in channel.times_seconds.windows(2) {
            context.work(SceneValidationWork::PayloadCell, 1)?;
            if window[0] > window[1] {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::Unsorted,
                    SceneValidationPath::AnimationValues {
                        animation: animation_index,
                        channel: channel_index,
                        values: SceneAnimationValueKind::Times,
                    },
                ));
            }
        }
        validate_animation_values(
            animation_index,
            channel_index,
            channel.target.property,
            channel.interpolation,
            channel.times_seconds.len(),
            &channel.values,
            context,
        )?;
    }
    Ok(())
}

fn validate_animation_values<C: SceneValidationControl>(
    animation_index: usize,
    channel_index: usize,
    property: AnimationProperty,
    interpolation: AnimationInterpolation,
    keyframes: usize,
    values: &AnimationValues,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    if !animation_values_match_property(property, values) {
        return invalid(SceneValidationIssue::new(
            SceneValidationCode::KindMismatch,
            SceneValidationPath::AnimationChannel {
                animation: animation_index,
                channel: channel_index,
            },
        ));
    }
    let sample_count = match values {
        AnimationValues::Translations(values) => {
            if contains_invalid(values, context, |value| !value.is_finite())? {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::NonFinite,
                    SceneValidationPath::AnimationValues {
                        animation: animation_index,
                        channel: channel_index,
                        values: SceneAnimationValueKind::Translations,
                    },
                ));
            }
            values.len()
        }
        AnimationValues::Rotations(values) => {
            if contains_invalid(values, context, |value| !value.is_finite())? {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::NonFinite,
                    SceneValidationPath::AnimationValues {
                        animation: animation_index,
                        channel: channel_index,
                        values: SceneAnimationValueKind::Rotations,
                    },
                ));
            }
            values.len()
        }
        AnimationValues::Scales(values) => {
            if contains_invalid(values, context, |value| !value.is_finite())? {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::NonFinite,
                    SceneValidationPath::AnimationValues {
                        animation: animation_index,
                        channel: channel_index,
                        values: SceneAnimationValueKind::Scales,
                    },
                ));
            }
            values.len()
        }
        AnimationValues::MorphWeights {
            values,
            weights_per_keyframe,
        } => {
            if *weights_per_keyframe == 0 {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::MinimumNotMet,
                        SceneValidationPath::AnimationValues {
                            animation: animation_index,
                            channel: channel_index,
                            values: SceneAnimationValueKind::MorphWeights,
                        },
                    )
                    .with_data(SceneValidationData::Minimum {
                        actual: 0,
                        minimum: 1,
                    }),
                );
            }
            if contains_invalid(values, context, |value| !value.is_finite())? {
                return invalid(SceneValidationIssue::new(
                    SceneValidationCode::NonFinite,
                    SceneValidationPath::AnimationValues {
                        animation: animation_index,
                        channel: channel_index,
                        values: SceneAnimationValueKind::MorphWeights,
                    },
                ));
            }
            if values.len() % weights_per_keyframe != 0 {
                return invalid(
                    SceneValidationIssue::new(
                        SceneValidationCode::InvalidSampleCount,
                        SceneValidationPath::AnimationValues {
                            animation: animation_index,
                            channel: channel_index,
                            values: SceneAnimationValueKind::MorphWeights,
                        },
                    )
                    .with_data(SceneValidationData::Divisibility {
                        count: values.len(),
                        divisor: *weights_per_keyframe,
                    }),
                );
            }
            values.len() / weights_per_keyframe
        }
    };
    let expected = match interpolation {
        AnimationInterpolation::CubicSpline => {
            keyframes
                .checked_mul(3)
                .ok_or(SceneValidationError::AccountingOverflow {
                    resource: "cubic animation sample count",
                })?
        }
        AnimationInterpolation::Step
        | AnimationInterpolation::Linear
        | AnimationInterpolation::Unknown => keyframes,
    };
    if sample_count != expected {
        return invalid(
            SceneValidationIssue::new(
                SceneValidationCode::LengthMismatch,
                SceneValidationPath::AnimationChannel {
                    animation: animation_index,
                    channel: channel_index,
                },
            )
            .with_data(SceneValidationData::Counts {
                actual: sample_count,
                expected,
            }),
        );
    }
    Ok(())
}

fn animation_values_match_property(property: AnimationProperty, values: &AnimationValues) -> bool {
    matches!(
        (property, values),
        (
            AnimationProperty::Translation,
            AnimationValues::Translations(_)
        ) | (AnimationProperty::Rotation, AnimationValues::Rotations(_))
            | (AnimationProperty::Scale, AnimationValues::Scales(_))
            | (
                AnimationProperty::MorphWeights,
                AnimationValues::MorphWeights { .. }
            )
    )
}

fn validate_space<C: SceneValidationControl>(
    scene: &SceneData,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    context.work(SceneValidationWork::Entity, 1)?;
    if let Some(scale) = scene.space.unit_scale_to_meters
        && (!scale.is_finite() || scale <= 0.0)
    {
        return invalid(SceneValidationIssue::new(
            if scale.is_finite() {
                SceneValidationCode::NonPositive
            } else {
                SceneValidationCode::NonFinite
            },
            SceneValidationPath::SceneSpaceUnitScale,
        ));
    }
    Ok(())
}

fn validate_metadata_map<C: SceneValidationControl>(
    metadata: &MetadataMap,
    owner: SceneMetadataOwner,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    for (entry, (key, value)) in metadata.iter().enumerate() {
        context.work(SceneValidationWork::MetadataEntry, 1)?;
        validate_namespaced_key(key, owner, entry, context)?;
        match value {
            MetadataValue::F64(value) if !value.is_finite() => {
                return Err(SceneValidationError::InvalidMetadataValue {
                    owner,
                    entry,
                    violation: MetadataValueViolation::NonFiniteF64,
                }
                .into());
            }
            MetadataValue::String(value) => {
                if value.len() > context.limits.max_metadata_string_bytes() {
                    return Err(SceneValidationError::InvalidMetadataValue {
                        owner,
                        entry,
                        violation: MetadataValueViolation::StringTooLong,
                    }
                    .into());
                }
                if let Some(offset) = forbidden_control_offset(value, context)? {
                    return Err(SceneValidationError::InvalidMetadataValue {
                        owner,
                        entry,
                        violation: MetadataValueViolation::ForbiddenControl { offset },
                    }
                    .into());
                }
            }
            MetadataValue::Bool(_) | MetadataValue::I64(_) | MetadataValue::F64(_) => {}
        }
    }
    Ok(())
}

fn validate_namespaced_key<C: SceneValidationControl>(
    key: &str,
    owner: SceneMetadataOwner,
    entry: usize,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    let violation = if key.is_empty() {
        Some(MetadataKeyViolation::Empty)
    } else if key.len() > context.limits.max_metadata_key_bytes() {
        Some(MetadataKeyViolation::TooLong)
    } else {
        validate_namespaced_key_bytes(key, context)?
    };
    if let Some(violation) = violation {
        Err(SceneValidationError::InvalidMetadataKey {
            owner,
            entry,
            violation,
        }
        .into())
    } else {
        Ok(())
    }
}

fn validate_namespaced_key_bytes<C: SceneValidationControl>(
    key: &str,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<Option<MetadataKeyViolation>, C> {
    let mut seen_colon = false;
    let mut component_has_value = false;
    for (offset, byte) in key.bytes().enumerate() {
        context.work(SceneValidationWork::MetadataByte, 1)?;
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => component_has_value = true,
            b':' if !seen_colon && component_has_value => {
                seen_colon = true;
                component_has_value = false;
            }
            b':' => return Ok(Some(MetadataKeyViolation::InvalidSeparator { offset })),
            b'.' | b'-' if component_has_value => component_has_value = false,
            b'_' if seen_colon && component_has_value => component_has_value = false,
            b'.' | b'-' | b'_' => {
                return Ok(Some(MetadataKeyViolation::InvalidSeparator { offset }));
            }
            _ => return Ok(Some(MetadataKeyViolation::InvalidByte { offset })),
        }
    }
    if !seen_colon {
        Ok(Some(MetadataKeyViolation::MissingNamespace))
    } else if !component_has_value {
        Ok(Some(MetadataKeyViolation::InvalidSeparator {
            offset: key.len().saturating_sub(1),
        }))
    } else {
        Ok(None)
    }
}

fn validate_optional_string<C: SceneValidationControl>(
    value: Option<&str>,
    owner: SceneStringOwner,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    if let Some(value) = value {
        validate_canonical_string(value, owner, context)?;
    }
    Ok(())
}

fn validate_canonical_string<C: SceneValidationControl>(
    value: &str,
    owner: SceneStringOwner,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<(), C> {
    if value.len() > context.limits.max_metadata_string_bytes() {
        return Err(SceneValidationError::InvalidCanonicalString {
            owner,
            violation: SceneStringViolation::TooLong,
        }
        .into());
    }
    if let Some(offset) = forbidden_control_offset(value, context)? {
        return Err(SceneValidationError::InvalidCanonicalString {
            owner,
            violation: SceneStringViolation::ForbiddenControl { offset },
        }
        .into());
    }
    Ok(())
}

fn forbidden_control_offset<C: SceneValidationControl>(
    value: &str,
    context: &mut ValidationContext<'_, C>,
) -> ValidationResult<Option<usize>, C> {
    for (offset, character) in value.char_indices() {
        context.work(
            SceneValidationWork::MetadataByte,
            u64_len(character.len_utf8(), "string validation bytes")?,
        )?;
        if character.is_control() {
            return Ok(Some(offset));
        }
    }
    Ok(None)
}

fn contains_invalid<T, C: SceneValidationControl>(
    values: &[T],
    context: &mut ValidationContext<'_, C>,
    mut invalid_value: impl FnMut(&T) -> bool,
) -> ValidationResult<bool, C> {
    for value in values {
        context.work(SceneValidationWork::PayloadCell, 1)?;
        if invalid_value(value) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn color_is_finite(color: Color) -> bool {
    color.r.is_finite() && color.g.is_finite() && color.b.is_finite() && color.a.is_finite()
}

pub(crate) fn empty_vec_capacity_plan<T>(
    desired_capacity: usize,
    resource: &'static str,
) -> Result<CapacityPlan, SceneValidationError> {
    CapacityPlan::for_type::<T>(0, 0, desired_capacity)
        .map_err(|cause| capacity_plan_error(resource, cause))
}

fn capacity_envelope_bytes(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, SceneValidationError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| capacity_plan_error(resource, cause))
}

pub(crate) fn apply_empty_vec_capacity_plan<T>(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<(Vec<T>, u64), SceneValidationError> {
    plan.verify_preflight(0, 0)
        .map_err(|cause| capacity_plan_error(resource, cause))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| SceneValidationError::AllocationFailed { resource })?;
    let actual = plan
        .finish(values.len(), values.capacity())
        .map_err(|cause| capacity_plan_error(resource, cause))?;
    Ok((values, actual))
}

pub(crate) fn capacity_plan_error(
    resource: &'static str,
    cause: CapacityPlanError,
) -> SceneValidationError {
    SceneValidationError::CapacityPlan { resource, cause }
}

fn invalid<T, E: Error + Send + Sync + 'static>(
    issue: SceneValidationIssue,
) -> Result<T, SceneValidationFailure<E>> {
    Err(SceneValidationError::InvalidScene { issue }.into())
}

fn add_limited(
    value: &mut u64,
    amount: u64,
    limit: SceneValidationLimit,
    maximum: u64,
) -> Result<(), SceneValidationError> {
    let actual = checked_add(*value, amount, "canonical count")?;
    enforce_limit(limit, actual, maximum)?;
    *value = actual;
    Ok(())
}

pub(crate) fn enforce_limit(
    limit: SceneValidationLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), SceneValidationError> {
    if actual > maximum {
        Err(SceneValidationError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn checked_add(left: u64, right: u64, resource: &'static str) -> Result<u64, SceneValidationError> {
    left.checked_add(right)
        .ok_or(SceneValidationError::AccountingOverflow { resource })
}

fn checked_mul(left: u64, right: u64, resource: &'static str) -> Result<u64, SceneValidationError> {
    left.checked_mul(right)
        .ok_or(SceneValidationError::AccountingOverflow { resource })
}

pub(crate) fn u64_len(value: usize, resource: &'static str) -> Result<u64, SceneValidationError> {
    u64::try_from(value).map_err(|_| SceneValidationError::AccountingOverflow { resource })
}
