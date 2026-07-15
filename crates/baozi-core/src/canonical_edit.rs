use crate::canonical_scene::{
    CanonicalSceneCollection, CanonicalSceneData, CanonicalValidatedScene, SceneBounds,
};
use crate::canonical_validation::{
    CanonicalValidationError, CanonicalValidationFailure, CanonicalValidationLimit,
    validate_canonical_scene_data_with_initial_work,
};
use crate::derived::DerivedAbsenceReason;
use crate::entity::{AuthoredTransform, Node};
use crate::geometry::{
    ControlledLogicalMeshGenerationRebindError, LogicalMeshGenerationRebindError,
};
use crate::identity::{
    NodeId, NodeIndex, SceneGenerationError, SceneGenerationId, SceneLookupError, SceneStorageIndex,
};
use crate::remap::{
    AnimationClipRemapSpace, CameraRemapSpace, CornerRemapSpace, ExplicitUnchangedRemap,
    FaceRemapSpace, ImageRemapSpace, InstanceRemapSpace, LightRemapSpace, LogicalMeshRemapSpace,
    MaterialRemapSpace, MeshInstanceRemapSpace, MorphTargetRemapSpace, NodeRemapSpace,
    PrimitiveRemapSpace, RemapChange, RemapError, RemapGenerationBoundary,
    RemapGenerationBoundaryError, RemapSpace, SamplerRemapSpace, SkinRemapSpace, TextureRemapSpace,
    VertexRemapSpace,
};
use crate::shared_storage::SharedStorage;
use crate::validation_control::{
    NoopSceneValidationControl, SCENE_VALIDATION_POLL_INTERVAL, SceneReservationKind,
    SceneValidationControl, SceneValidationLimits, SceneValidationWork, try_publish_coupled,
};
use baozi_operation::CapacityPlan;
use std::convert::Infallible;
use std::error::Error;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum CanonicalEditError {
    #[error(transparent)]
    Lookup(#[from] SceneLookupError),
    #[error(transparent)]
    Generation(#[from] SceneGenerationError),
    #[error(transparent)]
    LogicalMeshRebind(#[from] LogicalMeshGenerationRebindError),
    #[error(transparent)]
    Remaps(#[from] CanonicalRemapSetError),
    #[error(transparent)]
    Validation(#[from] CanonicalValidationError),
    #[error("canonical edit allocation failed for {resource:?}")]
    AllocationFailed { resource: CanonicalSceneCollection },
    #[error("canonical edit allocation accounting overflowed for {resource:?}")]
    AllocationAccountingOverflow { resource: CanonicalSceneCollection },
    #[error("canonical edit accounting overflowed")]
    AccountingOverflow,
    #[error("canonical edit {work:?} work {actual} exceeds limit {maximum}")]
    WorkLimitExceeded {
        work: SceneValidationWork,
        actual: u64,
        maximum: u64,
    },
    #[error("canonical edit {limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: CanonicalValidationLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("canonical edit output retained {actual} bytes beyond admitted {admitted} bytes")]
    OutputReservationUnderestimated { admitted: u64, actual: u64 },
    #[error("canonical node transform edit did not invalidate scene bounds")]
    MissingSceneBoundsInvalidation,
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalEditFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Edit(#[from] CanonicalEditError),
    #[error("canonical edit control failed")]
    Control(#[source] E),
}

// Boxing the changed variant would add an unmetered fallible allocation after admission.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CanonicalNodeTransformEditStart<'a, R = ()> {
    Unchanged,
    Changed(CanonicalNodeTransformEdit<'a, R>),
}

struct CanonicalEditCandidate {
    target: NodeIndex,
    replacement: Node,
    invalidations: CanonicalInvalidations,
    #[cfg(test)]
    root_override: Option<Option<NodeIndex>>,
    #[cfg(test)]
    drop_probe: Option<CanonicalEditBundleDropProbe>,
}

#[must_use = "dropping a canonical edit releases its candidate reservation"]
pub(crate) struct CanonicalNodeTransformEdit<'a, R = ()> {
    // Storage must drop before its covering reservation during rollback.
    candidate: CanonicalEditCandidate,
    input: &'a CanonicalValidatedScene,
    reservation: R,
    limits: SceneValidationLimits,
    consumed_work: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CanonicalInvalidations {
    scene_bounds: Option<DerivedAbsenceReason>,
}

impl CanonicalInvalidations {
    pub(crate) const fn scene_bounds(self) -> Option<DerivedAbsenceReason> {
        self.scene_bounds
    }
}

pub(crate) struct CanonicalEditPublication {
    scene: CanonicalValidatedScene,
    remaps: CanonicalRemapSet,
    invalidations: CanonicalInvalidations,
    #[cfg(test)]
    drop_probe: Option<CanonicalEditBundleDropProbe>,
}

impl CanonicalEditPublication {
    pub(crate) const fn scene(&self) -> &CanonicalValidatedScene {
        &self.scene
    }

    pub(crate) const fn remaps(&self) -> &CanonicalRemapSet {
        &self.remaps
    }

    pub(crate) const fn invalidations(&self) -> CanonicalInvalidations {
        self.invalidations
    }
}

#[must_use = "dropping a canonical edit commit releases its canonical reservation"]
pub(crate) struct CanonicalEditCommit<R> {
    // Publication storage must drop before its covering reservation.
    publication: CanonicalEditPublication,
    reservation: R,
}

impl<R> CanonicalEditCommit<R> {
    pub(crate) fn try_publish<E>(
        self,
        publish_reservation: impl FnOnce(&mut R) -> Result<(), E>,
    ) -> Result<CanonicalEditPublication, E> {
        let Self {
            publication,
            reservation,
        } = self;
        try_publish_coupled(publication, reservation, publish_reservation)
    }
}

impl CanonicalEditCommit<()> {
    pub(crate) fn into_unreserved_publication(self) -> CanonicalEditPublication {
        self.publication
    }
}

impl<'a> CanonicalNodeTransformEdit<'a, ()> {
    pub(crate) fn try_begin(
        input: &'a CanonicalValidatedScene,
        node: NodeId,
        authored_transform: AuthoredTransform,
        limits: SceneValidationLimits,
    ) -> Result<CanonicalNodeTransformEditStart<'a>, CanonicalEditError> {
        let mut control = NoopSceneValidationControl;
        unwrap_uncontrolled(Self::try_begin_with(
            input,
            node,
            authored_transform,
            limits,
            &mut control,
        ))
    }

    pub(crate) fn try_begin_with<C: SceneValidationControl>(
        input: &'a CanonicalValidatedScene,
        node: NodeId,
        authored_transform: AuthoredTransform,
        limits: SceneValidationLimits,
        control: &mut C,
    ) -> Result<CanonicalNodeTransformEditStart<'a, C::Reservation>, CanonicalEditFailure<C::Error>>
    {
        let input_node = input.node(node).map_err(CanonicalEditError::from)?;
        if input_node
            .authored_transform()
            .has_same_authored_bits(authored_transform)
        {
            return Ok(CanonicalNodeTransformEditStart::Unchanged);
        }

        let target =
            NodeIndex::from_handle(input.generation(), node).map_err(CanonicalEditError::from)?;
        let mut context = CanonicalEditCloneContext::new(limits, control);
        context.poll_now()?;
        context.add_clone_work(1)?;
        let reservation = context
            .control
            .reserve(SceneReservationKind::EditCandidate, 0)
            .map_err(CanonicalEditFailure::Control)?;
        let reserved = ReservedCanonicalEditCandidate { reservation };
        let candidate = CanonicalEditCandidate {
            target,
            replacement: input_node.fork_with_authored_transform(authored_transform),
            invalidations: CanonicalInvalidations {
                scene_bounds: Some(DerivedAbsenceReason::Invalidated),
            },
            #[cfg(test)]
            root_override: None,
            #[cfg(test)]
            drop_probe: None,
        };
        context.poll_now()?;
        let consumed_work = context.total_work();
        let ReservedCanonicalEditCandidate { reservation } = reserved;
        Ok(CanonicalNodeTransformEditStart::Changed(
            CanonicalNodeTransformEdit {
                candidate,
                input,
                reservation,
                limits,
                consumed_work,
            },
        ))
    }

    pub(crate) fn commit(self) -> Result<CanonicalEditCommit<()>, CanonicalEditError> {
        let mut control = NoopSceneValidationControl;
        unwrap_uncontrolled(self.commit_with(&mut control))
    }
}

impl<'a, R> CanonicalNodeTransformEdit<'a, R> {
    #[cfg(test)]
    fn install_drop_probe(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        self.candidate.drop_probe = Some(CanonicalEditBundleDropProbe(state));
    }

    #[cfg(test)]
    fn override_root_for_test(&mut self, root: Option<NodeIndex>) {
        self.candidate.root_override = Some(root);
    }

    pub(crate) fn commit_with<C>(
        self,
        control: &mut C,
    ) -> Result<CanonicalEditCommit<R>, CanonicalEditFailure<C::Error>>
    where
        C: SceneValidationControl<Reservation = R>,
    {
        control
            .validate_reservation(&self.reservation, SceneReservationKind::EditCandidate)
            .map_err(CanonicalEditFailure::Control)?;
        enforce_unchanged_output_limits(self.input, self.limits)?;
        let Self {
            candidate,
            input,
            reservation,
            limits,
            consumed_work,
        } = self;
        let CanonicalEditCandidate {
            target,
            replacement,
            invalidations,
            #[cfg(test)]
            root_override,
            #[cfg(test)]
            drop_probe,
        } = candidate;
        let mut pending = PendingCanonicalEditCommit {
            input,
            target,
            replacement: Some(replacement),
            invalidations,
            #[cfg(test)]
            root_override,
            #[cfg(test)]
            drop_probe,
            reservation,
        };
        let (data, remaps, validation_initial_work, admitted_bytes) = {
            let mut context =
                CanonicalEditCloneContext::with_initial_work(limits, control, consumed_work);
            context.poll_now()?;
            let edit_plan =
                CanonicalEditPlan::try_new(pending.input, pending.invalidations, &mut context)?;
            let admitted_bytes = canonical_output_reservation_envelope(pending.input, edit_plan)?;
            enforce_edit_limit(
                CanonicalValidationLimit::CanonicalBytes,
                admitted_bytes,
                limits.max_canonical_bytes(),
            )?;
            context
                .control
                .resize_reservation(
                    &mut pending.reservation,
                    SceneReservationKind::EditCandidate,
                    admitted_bytes,
                )
                .map_err(CanonicalEditFailure::Control)?;
            context.poll_now()?;
            let output_generation =
                SceneGenerationId::allocate().map_err(CanonicalEditError::from)?;
            let replacement = pending
                .replacement
                .take()
                .ok_or(CanonicalEditError::AccountingOverflow)?;
            let data = try_fork_scene_for_node_transform(
                pending.input.data(),
                pending.target,
                replacement,
                output_generation,
                pending.invalidations,
                &mut context,
            )?;
            #[cfg(test)]
            let data = apply_root_override_for_test(data, pending.root_override);
            let remaps =
                CanonicalRemapSet::try_all_unchanged(pending.input.generation(), output_generation)
                    .map_err(CanonicalEditError::from)?;
            (data, remaps, context.total_work(), admitted_bytes)
        };
        let certificate = validate_canonical_scene_data_with_initial_work(
            data,
            limits,
            validation_initial_work,
            control,
        )
        .map_err(map_validation_failure)?;
        let required_bytes = certificate
            .stats()
            .footprint()
            .accounted_payload_bytes()
            .checked_add(remaps.retained_allocation_bytes())
            .ok_or(CanonicalEditError::AccountingOverflow)?;
        if required_bytes > admitted_bytes {
            return Err(CanonicalEditError::OutputReservationUnderestimated {
                admitted: admitted_bytes,
                actual: required_bytes,
            }
            .into());
        }
        control
            .transition_reservation(
                &mut pending.reservation,
                SceneReservationKind::EditCandidate,
                SceneReservationKind::CanonicalOutput,
                required_bytes,
            )
            .map_err(CanonicalEditFailure::Control)?;

        let PendingCanonicalEditCommit {
            invalidations,
            #[cfg(test)]
            drop_probe,
            reservation,
            ..
        } = pending;
        let committed = CanonicalEditCommit {
            publication: CanonicalEditPublication {
                scene: certificate.into_scene(),
                remaps,
                invalidations,
                #[cfg(test)]
                drop_probe,
            },
            reservation,
        };
        control.poll().map_err(CanonicalEditFailure::Control)?;
        Ok(committed)
    }
}

#[cfg(test)]
struct CanonicalEditBundleDropProbe(std::sync::Arc<std::sync::atomic::AtomicU8>);

#[cfg(test)]
impl Drop for CanonicalEditBundleDropProbe {
    fn drop(&mut self) {
        self.0.store(0b111, std::sync::atomic::Ordering::SeqCst);
    }
}

struct ReservedCanonicalEditCandidate<R> {
    reservation: R,
}

struct PendingCanonicalEditCommit<'a, R> {
    input: &'a CanonicalValidatedScene,
    target: NodeIndex,
    replacement: Option<Node>,
    invalidations: CanonicalInvalidations,
    #[cfg(test)]
    root_override: Option<Option<NodeIndex>>,
    #[cfg(test)]
    drop_probe: Option<CanonicalEditBundleDropProbe>,
    // Companion storage must drop before the reservation on every exit path.
    reservation: R,
}

fn map_validation_failure<E>(failure: CanonicalValidationFailure<E>) -> CanonicalEditFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    match failure {
        CanonicalValidationFailure::Validation(error) => {
            CanonicalEditFailure::Edit(CanonicalEditError::Validation(error))
        }
        CanonicalValidationFailure::Control(error) => CanonicalEditFailure::Control(error),
    }
}

fn unwrap_uncontrolled<T>(
    result: Result<T, CanonicalEditFailure<Infallible>>,
) -> Result<T, CanonicalEditError> {
    match result {
        Ok(value) => Ok(value),
        Err(CanonicalEditFailure::Edit(error)) => Err(error),
        Err(CanonicalEditFailure::Control(error)) => match error {},
    }
}

struct CanonicalEditCloneContext<'a, C: SceneValidationControl> {
    limits: SceneValidationLimits,
    control: &'a mut C,
    work: u64,
    work_since_poll: u64,
}

impl<'a, C: SceneValidationControl> CanonicalEditCloneContext<'a, C> {
    const fn new(limits: SceneValidationLimits, control: &'a mut C) -> Self {
        Self::with_initial_work(limits, control, 0)
    }

    const fn with_initial_work(
        limits: SceneValidationLimits,
        control: &'a mut C,
        initial_work: u64,
    ) -> Self {
        Self {
            limits,
            control,
            work: initial_work,
            work_since_poll: 0,
        }
    }

    const fn total_work(&self) -> u64 {
        self.work
    }

    fn add_typed_work(
        &mut self,
        kind: SceneValidationWork,
        amount: u64,
    ) -> Result<(), CanonicalEditFailure<C::Error>> {
        let next = self
            .work
            .checked_add(amount)
            .ok_or(CanonicalEditError::AccountingOverflow)?;
        if next > self.limits.max_work() {
            return Err(CanonicalEditError::WorkLimitExceeded {
                work: kind,
                actual: next,
                maximum: self.limits.max_work(),
            }
            .into());
        }
        self.control
            .charge_work(kind, amount)
            .map_err(CanonicalEditFailure::Control)?;
        self.work = next;

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

    fn add_clone_work(&mut self, amount: u64) -> Result<(), CanonicalEditFailure<C::Error>> {
        self.add_typed_work(SceneValidationWork::CloneCell, amount)
    }

    fn poll_now(&mut self) -> Result<(), CanonicalEditFailure<C::Error>> {
        self.control.poll().map_err(CanonicalEditFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }
}

fn try_fork_scene_for_node_transform<C: SceneValidationControl>(
    source: &CanonicalSceneData,
    target: NodeIndex,
    replacement: Node,
    generation: SceneGenerationId,
    invalidations: CanonicalInvalidations,
    context: &mut CanonicalEditCloneContext<'_, C>,
) -> Result<CanonicalSceneData, CanonicalEditFailure<C::Error>> {
    let scene_bounds = invalidations
        .scene_bounds()
        .map(SceneBounds::absent)
        .ok_or(CanonicalEditError::MissingSceneBoundsInvalidation)?;
    let target_index = target
        .try_to_usize()
        .map_err(|_| CanonicalEditError::AccountingOverflow)?;
    let mut nodes = try_empty_vec(source.nodes.len(), CanonicalSceneCollection::Nodes)?;
    let mut replacement = Some(replacement);
    for (index, node) in source.nodes.iter().enumerate() {
        context.add_clone_work(1)?;
        let replacement = if index == target_index {
            replacement
                .take()
                .ok_or(CanonicalEditError::AccountingOverflow)?
        } else {
            node.fork()
        };
        nodes.push(replacement);
    }

    let mut logical_meshes = try_empty_vec(
        source.logical_meshes.len(),
        CanonicalSceneCollection::LogicalMeshes,
    )?;
    for mesh in &source.logical_meshes {
        context.add_clone_work(1)?;
        let rebound = match mesh.try_rebind_validated_generation_controlled(
            source.generation,
            generation,
            || context.add_clone_work(1),
        ) {
            Ok(rebound) => rebound,
            Err(ControlledLogicalMeshGenerationRebindError::Rebind(error)) => {
                return Err(CanonicalEditError::LogicalMeshRebind(error).into());
            }
            Err(ControlledLogicalMeshGenerationRebindError::Control(error)) => {
                return Err(error);
            }
        };
        logical_meshes.push(rebound);
    }

    Ok(CanonicalSceneData {
        generation,
        root: source.root,
        scene_space: source.scene_space.clone(),
        provenance: clone_shared_value(&source.provenance, context)?,
        nodes,
        mesh_instances: try_clone_shared_vec(
            &source.mesh_instances,
            CanonicalSceneCollection::MeshInstances,
            context,
        )?,
        logical_meshes,
        materials: try_clone_shared_vec(
            &source.materials,
            CanonicalSceneCollection::Materials,
            context,
        )?,
        images: try_clone_shared_vec(&source.images, CanonicalSceneCollection::Images, context)?,
        samplers: try_clone_shared_vec(
            &source.samplers,
            CanonicalSceneCollection::Samplers,
            context,
        )?,
        textures: try_clone_shared_vec(
            &source.textures,
            CanonicalSceneCollection::Textures,
            context,
        )?,
        skins: try_clone_shared_vec(&source.skins, CanonicalSceneCollection::Skins, context)?,
        animation_clips: try_clone_shared_vec(
            &source.animation_clips,
            CanonicalSceneCollection::AnimationClips,
            context,
        )?,
        cameras: try_clone_shared_vec(&source.cameras, CanonicalSceneCollection::Cameras, context)?,
        lights: try_clone_shared_vec(&source.lights, CanonicalSceneCollection::Lights, context)?,
        extension_schemas: clone_shared_value(&source.extension_schemas, context)?,
        metadata: clone_shared_value(&source.metadata, context)?,
        provenance_arena: source.provenance_arena.clone(),
        scene_bounds,
    })
}

#[cfg(test)]
fn apply_root_override_for_test(
    mut data: CanonicalSceneData,
    root_override: Option<Option<NodeIndex>>,
) -> CanonicalSceneData {
    if let Some(root) = root_override {
        data.root = root;
    }
    data
}

fn try_clone_shared_vec<T, C: SceneValidationControl>(
    source: &[SharedStorage<T>],
    resource: CanonicalSceneCollection,
    context: &mut CanonicalEditCloneContext<'_, C>,
) -> Result<Vec<SharedStorage<T>>, CanonicalEditFailure<C::Error>> {
    let mut output = try_empty_vec(source.len(), resource)?;
    for value in source {
        context.add_clone_work(1)?;
        output.push(value.clone());
    }
    Ok(output)
}

fn clone_shared_value<T, C: SceneValidationControl>(
    source: &SharedStorage<T>,
    context: &mut CanonicalEditCloneContext<'_, C>,
) -> Result<SharedStorage<T>, CanonicalEditFailure<C::Error>> {
    context.add_clone_work(1)?;
    Ok(source.clone())
}

fn try_empty_vec<T>(
    desired_capacity: usize,
    resource: CanonicalSceneCollection,
) -> Result<Vec<T>, CanonicalEditError> {
    let plan = CapacityPlan::for_type::<T>(0, 0, desired_capacity)
        .map_err(|_| CanonicalEditError::AllocationAccountingOverflow { resource })?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| CanonicalEditError::AllocationFailed { resource })?;
    plan.finish(output.len(), output.capacity())
        .map_err(|_| CanonicalEditError::AllocationFailed { resource })?;
    Ok(output)
}

fn canonical_output_reservation_envelope(
    _input: &CanonicalValidatedScene,
    edit_plan: CanonicalEditPlan,
) -> Result<u64, CanonicalEditError> {
    // Publication may outlive the borrowed input. Until SharedStorage has one
    // cross-scene accounting owner, charge the complete output footprint.
    Ok(edit_plan.output_envelope_bytes)
}

#[derive(Debug, Clone, Copy)]
struct CanonicalEditPlan {
    output_envelope_bytes: u64,
}

impl CanonicalEditPlan {
    fn try_new<C: SceneValidationControl>(
        input: &CanonicalValidatedScene,
        invalidations: CanonicalInvalidations,
        context: &mut CanonicalEditCloneContext<'_, C>,
    ) -> Result<Self, CanonicalEditFailure<C::Error>> {
        let scene = input.data();
        let mut plan = Self {
            output_envelope_bytes: input
                .validation_stats()
                .footprint()
                .accounted_payload_bytes(),
        };

        macro_rules! replace_collection_capacity {
            ($field:ident, $resource:ident) => {
                plan.replace_collection_capacity(
                    &scene.$field,
                    CanonicalSceneCollection::$resource,
                )?;
            };
        }
        replace_collection_capacity!(nodes, Nodes);
        replace_collection_capacity!(mesh_instances, MeshInstances);
        replace_collection_capacity!(logical_meshes, LogicalMeshes);
        replace_collection_capacity!(materials, Materials);
        replace_collection_capacity!(images, Images);
        replace_collection_capacity!(samplers, Samplers);
        replace_collection_capacity!(textures, Textures);
        replace_collection_capacity!(skins, Skins);
        replace_collection_capacity!(animation_clips, AnimationClips);
        replace_collection_capacity!(cameras, Cameras);
        replace_collection_capacity!(lights, Lights);

        for mesh in &scene.logical_meshes {
            context.add_clone_work(1)?;
            let capacity = mesh
                .generation_rebind_capacity()
                .map_err(CanonicalEditError::from)?;
            plan.replace_bytes(capacity.source_bytes(), capacity.output_envelope_bytes())?;
        }
        if invalidations.scene_bounds().is_some() {
            let source_bytes = u64::try_from(
                scene
                    .scene_bounds
                    .retained_producer_allocation_bytes()
                    .map_err(|_| CanonicalEditError::AccountingOverflow)?,
            )
            .map_err(|_| CanonicalEditError::AccountingOverflow)?;
            plan.replace_bytes(source_bytes, 0)?;
        }
        Ok(plan)
    }

    fn replace_collection_capacity<T>(
        &mut self,
        source: &Vec<T>,
        resource: CanonicalSceneCollection,
    ) -> Result<(), CanonicalEditError> {
        let source_bytes = collection_capacity_bytes::<T>(source.capacity(), resource)?;
        let output_plan = CapacityPlan::for_type::<T>(0, 0, source.len())
            .map_err(|_| CanonicalEditError::AllocationAccountingOverflow { resource })?;
        let output_bytes = output_plan
            .envelope_delta_bytes()
            .map_err(|_| CanonicalEditError::AllocationAccountingOverflow { resource })?;
        self.replace_bytes(source_bytes, output_bytes)
    }

    fn replace_bytes(
        &mut self,
        source_bytes: u64,
        output_bytes: u64,
    ) -> Result<(), CanonicalEditError> {
        self.output_envelope_bytes = self
            .output_envelope_bytes
            .checked_sub(source_bytes)
            .and_then(|bytes| bytes.checked_add(output_bytes))
            .ok_or(CanonicalEditError::AccountingOverflow)?;
        Ok(())
    }
}

fn collection_capacity_bytes<T>(
    capacity: usize,
    resource: CanonicalSceneCollection,
) -> Result<u64, CanonicalEditError> {
    capacity
        .checked_mul(std::mem::size_of::<T>())
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(CanonicalEditError::AllocationAccountingOverflow { resource })
}

fn enforce_unchanged_output_limits(
    input: &CanonicalValidatedScene,
    limits: SceneValidationLimits,
) -> Result<(), CanonicalEditError> {
    let stats = input.validation_stats();
    let counts = stats.canonical_counts();
    for (limit, actual, maximum) in [
        (
            CanonicalValidationLimit::Nodes,
            counts.nodes,
            limits.max_nodes(),
        ),
        (
            CanonicalValidationLimit::Edges,
            counts.edges,
            limits.max_edges(),
        ),
        (
            CanonicalValidationLimit::Depth,
            stats.maximum_depth(),
            limits.max_depth(),
        ),
        (
            CanonicalValidationLimit::Entities,
            counts.entities,
            limits.max_entities(),
        ),
        (
            CanonicalValidationLimit::References,
            counts.references,
            limits.max_references(),
        ),
        (
            CanonicalValidationLimit::PayloadCells,
            counts.payload_cells,
            limits.max_payload_cells(),
        ),
        (
            CanonicalValidationLimit::MetadataEntries,
            counts.metadata_entries,
            limits.max_metadata_entries(),
        ),
        (
            CanonicalValidationLimit::MetadataBytes,
            counts.metadata_bytes,
            limits.max_metadata_bytes(),
        ),
    ] {
        enforce_edit_limit(limit, actual, maximum)?;
    }
    Ok(())
}

fn enforce_edit_limit(
    limit: CanonicalValidationLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), CanonicalEditError> {
    if actual > maximum {
        return Err(CanonicalEditError::LimitExceeded {
            limit,
            actual,
            maximum,
        });
    }
    Ok(())
}

pub(crate) struct CanonicalSubjectRemap<S: RemapSpace> {
    unchanged: ExplicitUnchangedRemap<S>,
}

impl<S: RemapSpace> CanonicalSubjectRemap<S> {
    pub(crate) fn input_generation(&self) -> SceneGenerationId {
        self.unchanged.input_generation()
    }

    pub(crate) fn output_generation(&self) -> SceneGenerationId {
        self.unchanged.output_generation()
    }

    pub(crate) fn change(&self) -> RemapChange {
        self.unchanged.change()
    }

    pub(crate) fn try_rebind_unchanged(
        &self,
        endpoint: S::Endpoint,
    ) -> Result<Option<S::Endpoint>, RemapError<S::Endpoint>> {
        self.unchanged.try_rebind(endpoint).map(Some)
    }
}

pub(crate) struct CanonicalRemapSet {
    nodes: CanonicalSubjectRemap<NodeRemapSpace>,
    mesh_instances: CanonicalSubjectRemap<MeshInstanceRemapSpace>,
    logical_meshes: CanonicalSubjectRemap<LogicalMeshRemapSpace>,
    primitives: CanonicalSubjectRemap<PrimitiveRemapSpace>,
    morph_targets: CanonicalSubjectRemap<MorphTargetRemapSpace>,
    materials: CanonicalSubjectRemap<MaterialRemapSpace>,
    images: CanonicalSubjectRemap<ImageRemapSpace>,
    samplers: CanonicalSubjectRemap<SamplerRemapSpace>,
    textures: CanonicalSubjectRemap<TextureRemapSpace>,
    skins: CanonicalSubjectRemap<SkinRemapSpace>,
    animation_clips: CanonicalSubjectRemap<AnimationClipRemapSpace>,
    cameras: CanonicalSubjectRemap<CameraRemapSpace>,
    lights: CanonicalSubjectRemap<LightRemapSpace>,
    vertices: CanonicalSubjectRemap<VertexRemapSpace>,
    corners: CanonicalSubjectRemap<CornerRemapSpace>,
    faces: CanonicalSubjectRemap<FaceRemapSpace>,
    instances: CanonicalSubjectRemap<InstanceRemapSpace>,
}

impl CanonicalRemapSet {
    fn try_all_unchanged(
        from_generation: SceneGenerationId,
        to_generation: SceneGenerationId,
    ) -> Result<Self, CanonicalRemapSetError> {
        let boundary = RemapGenerationBoundary::try_new(from_generation, to_generation)
            .map_err(CanonicalRemapSetError::Generation)?;
        macro_rules! unchanged {
            ($space:ty) => {
                CanonicalSubjectRemap {
                    unchanged: ExplicitUnchangedRemap::<$space>::from_boundary(boundary),
                }
            };
        }
        Ok(Self {
            nodes: unchanged!(NodeRemapSpace),
            mesh_instances: unchanged!(MeshInstanceRemapSpace),
            logical_meshes: unchanged!(LogicalMeshRemapSpace),
            primitives: unchanged!(PrimitiveRemapSpace),
            morph_targets: unchanged!(MorphTargetRemapSpace),
            materials: unchanged!(MaterialRemapSpace),
            images: unchanged!(ImageRemapSpace),
            samplers: unchanged!(SamplerRemapSpace),
            textures: unchanged!(TextureRemapSpace),
            skins: unchanged!(SkinRemapSpace),
            animation_clips: unchanged!(AnimationClipRemapSpace),
            cameras: unchanged!(CameraRemapSpace),
            lights: unchanged!(LightRemapSpace),
            vertices: unchanged!(VertexRemapSpace),
            corners: unchanged!(CornerRemapSpace),
            faces: unchanged!(FaceRemapSpace),
            instances: unchanged!(InstanceRemapSpace),
        })
    }

    pub(crate) const fn nodes(&self) -> &CanonicalSubjectRemap<NodeRemapSpace> {
        &self.nodes
    }

    pub(crate) const fn mesh_instances(&self) -> &CanonicalSubjectRemap<MeshInstanceRemapSpace> {
        &self.mesh_instances
    }

    pub(crate) const fn logical_meshes(&self) -> &CanonicalSubjectRemap<LogicalMeshRemapSpace> {
        &self.logical_meshes
    }

    pub(crate) const fn primitives(&self) -> &CanonicalSubjectRemap<PrimitiveRemapSpace> {
        &self.primitives
    }

    pub(crate) const fn morph_targets(&self) -> &CanonicalSubjectRemap<MorphTargetRemapSpace> {
        &self.morph_targets
    }

    pub(crate) const fn materials(&self) -> &CanonicalSubjectRemap<MaterialRemapSpace> {
        &self.materials
    }

    pub(crate) const fn images(&self) -> &CanonicalSubjectRemap<ImageRemapSpace> {
        &self.images
    }

    pub(crate) const fn samplers(&self) -> &CanonicalSubjectRemap<SamplerRemapSpace> {
        &self.samplers
    }

    pub(crate) const fn textures(&self) -> &CanonicalSubjectRemap<TextureRemapSpace> {
        &self.textures
    }

    pub(crate) const fn skins(&self) -> &CanonicalSubjectRemap<SkinRemapSpace> {
        &self.skins
    }

    pub(crate) const fn animation_clips(&self) -> &CanonicalSubjectRemap<AnimationClipRemapSpace> {
        &self.animation_clips
    }

    pub(crate) const fn cameras(&self) -> &CanonicalSubjectRemap<CameraRemapSpace> {
        &self.cameras
    }

    pub(crate) const fn lights(&self) -> &CanonicalSubjectRemap<LightRemapSpace> {
        &self.lights
    }

    pub(crate) const fn vertices(&self) -> &CanonicalSubjectRemap<VertexRemapSpace> {
        &self.vertices
    }

    pub(crate) const fn corners(&self) -> &CanonicalSubjectRemap<CornerRemapSpace> {
        &self.corners
    }

    pub(crate) const fn faces(&self) -> &CanonicalSubjectRemap<FaceRemapSpace> {
        &self.faces
    }

    pub(crate) const fn instances(&self) -> &CanonicalSubjectRemap<InstanceRemapSpace> {
        &self.instances
    }

    const fn retained_allocation_bytes(&self) -> u64 {
        0
    }
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalRemapSetError {
    #[error("canonical remap generation boundary failed")]
    Generation(#[source] RemapGenerationBoundaryError),
}

#[cfg(test)]
mod tests;
