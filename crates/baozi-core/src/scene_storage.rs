use crate::material::{
    Material, MaterialProperty, MaterialPropertyMap, Texture, TextureSlot, TextureSource,
};
use crate::scene::{
    Animation, AnimationChannel, AnimationValues, Camera, Light, Mesh, MetadataMap, MetadataValue,
    MorphTarget, Node, SceneBuilder, SceneData, SceneDraft, Skin, VertexAttribute,
    VertexAttributeData, VertexAttributeSemantic,
};
use crate::validation::{
    apply_empty_vec_capacity_plan, capacity_plan_error, empty_vec_capacity_plan, enforce_limit,
    u64_len,
};
use crate::{
    OrderedMap, OrderedMapError, SceneFootprint, SceneReservationKind, SceneValidationControl,
    SceneValidationError, SceneValidationFailure, SceneValidationLimit, SceneValidationLimits,
    SceneValidationWork,
};
use baozi_operation::CapacityPlan;
use std::convert::Infallible;
use std::error::Error;
use std::mem::size_of;

/// Deterministic accounting for payload storage visible through stable Rust APIs.
///
/// This metric includes inline values, reported `Vec`/`String`/`OrderedMap`
/// capacities, and shared byte payloads. Allocator-private metadata remains an
/// external RSS concern rather than a logical retained-capacity claim.
pub trait SceneStorageFootprint {
    fn storage_footprint(&self) -> Result<SceneFootprint, SceneValidationError>;
}

macro_rules! impl_storage_footprint {
    ($type:ty, $measure:ident) => {
        impl SceneStorageFootprint for $type {
            fn storage_footprint(&self) -> Result<SceneFootprint, SceneValidationError> {
                let mut meter = FootprintMeter::default();
                meter.add_inline::<Self>()?;
                let mut work = |_| Ok::<(), SceneValidationFailure<Infallible>>(());
                unwrap_uncontrolled($measure(self, &mut meter, &mut work))?;
                Ok(SceneFootprint::new(meter.bytes))
            }
        }
    };
}

impl_storage_footprint!(Node, measure_node_dynamic);
impl_storage_footprint!(Mesh, measure_mesh_dynamic);
impl_storage_footprint!(Material, measure_material_dynamic);
impl_storage_footprint!(Texture, measure_texture_dynamic);
impl_storage_footprint!(Animation, measure_animation_dynamic);
impl_storage_footprint!(Camera, measure_camera_dynamic);
impl_storage_footprint!(Light, measure_light_dynamic);
impl_storage_footprint!(Skin, measure_skin_dynamic);
impl_storage_footprint!(MetadataMap, measure_metadata_map_dynamic);

impl SceneStorageFootprint for SceneBuilder {
    fn storage_footprint(&self) -> Result<SceneFootprint, SceneValidationError> {
        scene_data_footprint(&self.data)
    }
}

impl SceneStorageFootprint for SceneDraft {
    fn storage_footprint(&self) -> Result<SceneFootprint, SceneValidationError> {
        scene_data_footprint(&self.data)
    }
}

pub(crate) fn scene_data_footprint(
    scene: &SceneData,
) -> Result<SceneFootprint, SceneValidationError> {
    let mut work = |_| Ok::<(), SceneValidationFailure<Infallible>>(());
    unwrap_uncontrolled(scene_data_footprint_with(scene, &mut work))
}

pub(crate) fn scene_data_footprint_with<E: Error + Send + Sync + 'static>(
    scene: &SceneData,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<SceneFootprint, SceneValidationFailure<E>> {
    scene_data_footprint_with_meter(scene, FootprintMeter::default(), work)
}

fn scene_data_clone_envelope_with<E: Error + Send + Sync + 'static>(
    scene: &SceneData,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<SceneFootprint, SceneValidationFailure<E>> {
    scene_data_footprint_with_meter(scene, FootprintMeter::clone_envelope(), work)
}

fn scene_data_footprint_with_meter<E: Error + Send + Sync + 'static>(
    scene: &SceneData,
    mut meter: FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<SceneFootprint, SceneValidationFailure<E>> {
    meter.add_inline::<SceneData>()?;
    meter.add_vec(&scene.nodes)?;
    meter.add_vec(&scene.meshes)?;
    meter.add_vec(&scene.materials)?;
    meter.add_vec(&scene.textures)?;
    meter.add_vec(&scene.animations)?;
    meter.add_vec(&scene.cameras)?;
    meter.add_vec(&scene.lights)?;
    meter.add_vec(&scene.skins)?;

    for node in &scene.nodes {
        work(1)?;
        measure_node_dynamic(node, &mut meter, work)?;
    }
    for mesh in &scene.meshes {
        work(1)?;
        measure_mesh_dynamic(mesh, &mut meter, work)?;
    }
    for material in &scene.materials {
        work(1)?;
        measure_material_dynamic(material, &mut meter, work)?;
    }
    for texture in &scene.textures {
        work(1)?;
        measure_texture_dynamic(texture, &mut meter, work)?;
    }
    for animation in &scene.animations {
        work(1)?;
        measure_animation_dynamic(animation, &mut meter, work)?;
    }
    for camera in &scene.cameras {
        work(1)?;
        measure_camera_dynamic(camera, &mut meter, work)?;
    }
    for light in &scene.lights {
        work(1)?;
        measure_light_dynamic(light, &mut meter, work)?;
    }
    for skin in &scene.skins {
        work(1)?;
        measure_skin_dynamic(skin, &mut meter, work)?;
    }
    measure_metadata_map_dynamic(&scene.metadata, &mut meter, work)?;
    Ok(SceneFootprint::new(meter.bytes))
}

pub(crate) fn unwrap_uncontrolled<T>(
    result: Result<T, SceneValidationFailure<Infallible>>,
) -> Result<T, SceneValidationError> {
    match result {
        Ok(value) => Ok(value),
        Err(SceneValidationFailure::Scene(error)) => Err(error),
        Err(SceneValidationFailure::Control(error)) => match error {},
    }
}

struct FootprintMeter {
    bytes: u64,
    mode: CapacityMeasure,
}

#[derive(Debug, Clone, Copy, Default)]
enum CapacityMeasure {
    #[default]
    Retained,
    CloneEnvelope,
}

impl Default for FootprintMeter {
    fn default() -> Self {
        Self {
            bytes: 0,
            mode: CapacityMeasure::Retained,
        }
    }
}

impl FootprintMeter {
    const fn clone_envelope() -> Self {
        Self {
            bytes: 0,
            mode: CapacityMeasure::CloneEnvelope,
        }
    }

    fn add_inline<T>(&mut self) -> Result<(), SceneValidationError> {
        self.add_usize(size_of::<T>(), "scene inline storage")
    }

    fn add_vec<T>(&mut self, values: &Vec<T>) -> Result<(), SceneValidationError> {
        match self.mode {
            CapacityMeasure::Retained => {
                let bytes = values.capacity().checked_mul(size_of::<T>()).ok_or(
                    SceneValidationError::AccountingOverflow {
                        resource: "scene vector capacity",
                    },
                )?;
                self.add_usize(bytes, "scene vector capacity")
            }
            CapacityMeasure::CloneEnvelope => {
                self.add_clone_envelope::<T>(values.len(), "scene vector clone envelope")
            }
        }
    }

    fn add_string(&mut self, value: &String) -> Result<(), SceneValidationError> {
        match self.mode {
            CapacityMeasure::Retained => self.add_usize(value.capacity(), "scene string capacity"),
            CapacityMeasure::CloneEnvelope => {
                self.add_clone_envelope::<u8>(value.len(), "scene string clone envelope")
            }
        }
    }

    fn add_map<K, V>(&mut self, map: &OrderedMap<K, V>) -> Result<(), SceneValidationError> {
        let bytes = match self.mode {
            CapacityMeasure::Retained => map.capacity_bytes().map_err(map_accounting_error)?,
            CapacityMeasure::CloneEnvelope => CapacityPlan::for_type::<(K, V)>(0, 0, map.len())
                .and_then(CapacityPlan::envelope_delta_bytes)
                .map_err(|cause| capacity_plan_error("scene map clone envelope", cause))?,
        };
        self.bytes =
            self.bytes
                .checked_add(bytes)
                .ok_or(SceneValidationError::AccountingOverflow {
                    resource: "scene map storage",
                })?;
        Ok(())
    }

    fn add_clone_envelope<T>(
        &mut self,
        desired_capacity: usize,
        resource: &'static str,
    ) -> Result<(), SceneValidationError> {
        let bytes = CapacityPlan::for_type::<T>(0, 0, desired_capacity)
            .and_then(CapacityPlan::envelope_delta_bytes)
            .map_err(|cause| capacity_plan_error(resource, cause))?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(SceneValidationError::AccountingOverflow { resource })?;
        Ok(())
    }

    fn add_arc_bytes(&mut self, len: usize) -> Result<(), SceneValidationError> {
        let header =
            size_of::<usize>()
                .checked_mul(2)
                .ok_or(SceneValidationError::AccountingOverflow {
                    resource: "shared payload header",
                })?;
        let bytes = len
            .checked_add(header)
            .ok_or(SceneValidationError::AccountingOverflow {
                resource: "shared payload storage",
            })?;
        self.add_usize(bytes, "shared payload storage")
    }

    fn add_usize(
        &mut self,
        amount: usize,
        resource: &'static str,
    ) -> Result<(), SceneValidationError> {
        let amount = u64::try_from(amount)
            .map_err(|_| SceneValidationError::AccountingOverflow { resource })?;
        self.bytes = self
            .bytes
            .checked_add(amount)
            .ok_or(SceneValidationError::AccountingOverflow { resource })?;
        Ok(())
    }
}

fn measure_optional_string(
    value: &Option<String>,
    meter: &mut FootprintMeter,
) -> Result<(), SceneValidationError> {
    if let Some(value) = value {
        meter.add_string(value)?;
    }
    Ok(())
}

fn measure_node_dynamic<E: Error + Send + Sync + 'static>(
    node: &Node,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&node.name, meter)?;
    meter.add_vec(&node.children)?;
    meter.add_vec(&node.mesh_bindings)?;
    measure_metadata_map_dynamic(&node.metadata, meter, work)
}

fn measure_mesh_dynamic<E: Error + Send + Sync + 'static>(
    mesh: &Mesh,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&mesh.name, meter)?;
    meter.add_vec(&mesh.positions)?;
    meter.add_vec(&mesh.normals)?;
    meter.add_vec(&mesh.tangents)?;
    meter.add_vec(&mesh.texcoords)?;
    for channel in &mesh.texcoords {
        work(1)?;
        meter.add_vec(channel)?;
    }
    meter.add_vec(&mesh.colors)?;
    for channel in &mesh.colors {
        work(1)?;
        meter.add_vec(channel)?;
    }
    meter.add_vec(&mesh.indices)?;
    meter.add_vec(&mesh.face_vertex_counts)?;
    meter.add_vec(&mesh.joint_indices)?;
    meter.add_vec(&mesh.joint_weights)?;
    meter.add_vec(&mesh.morph_targets)?;
    for target in &mesh.morph_targets {
        work(1)?;
        measure_morph_target_dynamic(target, meter, work)?;
    }
    meter.add_vec(&mesh.custom_attributes)?;
    for attribute in &mesh.custom_attributes {
        work(1)?;
        measure_vertex_attribute_dynamic(attribute, meter, work)?;
    }
    measure_metadata_map_dynamic(&mesh.metadata, meter, work)
}

fn measure_morph_target_dynamic<E: Error + Send + Sync + 'static>(
    target: &MorphTarget,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&target.name, meter)?;
    meter.add_vec(&target.positions)?;
    meter.add_vec(&target.normals)?;
    meter.add_vec(&target.tangents)?;
    measure_metadata_map_dynamic(&target.metadata, meter, work)
}

fn measure_vertex_attribute_dynamic<E: Error + Send + Sync + 'static>(
    attribute: &VertexAttribute,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    meter.add_string(&attribute.name)?;
    if let VertexAttributeSemantic::Custom(value) = &attribute.semantic {
        meter.add_string(value)?;
    }
    match &attribute.data {
        VertexAttributeData::F32(values) => meter.add_vec(values)?,
        VertexAttributeData::Vec2(values) => meter.add_vec(values)?,
        VertexAttributeData::Vec3(values) => meter.add_vec(values)?,
        VertexAttributeData::Vec4(values) => meter.add_vec(values)?,
        VertexAttributeData::U16x4(values) => meter.add_vec(values)?,
        VertexAttributeData::U32(values) => meter.add_vec(values)?,
        VertexAttributeData::I32(values) => meter.add_vec(values)?,
    }
    measure_metadata_map_dynamic(&attribute.metadata, meter, work)
}

fn measure_material_dynamic<E: Error + Send + Sync + 'static>(
    material: &Material,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&material.name, meter)?;
    meter.add_vec(&material.texture_slots)?;
    for slot in &material.texture_slots {
        work(1)?;
        measure_optional_string(&slot.source_key, meter)?;
    }
    meter.add_map(&material.properties)?;
    for (key, value) in &material.properties {
        work(1)?;
        meter.add_string(key)?;
        if let MaterialProperty::String(value) = value {
            meter.add_string(value)?;
        }
    }
    measure_metadata_map_dynamic(&material.metadata, meter, work)
}

fn measure_texture_dynamic<E: Error + Send + Sync + 'static>(
    texture: &Texture,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&texture.name, meter)?;
    match &texture.source {
        TextureSource::External { uri } => meter.add_string(uri)?,
        TextureSource::Embedded { bytes, mime_type } => {
            meter.add_arc_bytes(bytes.len())?;
            measure_optional_string(mime_type, meter)?;
        }
    }
    measure_metadata_map_dynamic(&texture.metadata, meter, work)
}

fn measure_animation_dynamic<E: Error + Send + Sync + 'static>(
    animation: &Animation,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&animation.name, meter)?;
    meter.add_vec(&animation.channels)?;
    for channel in &animation.channels {
        work(1)?;
        meter.add_vec(&channel.times_seconds)?;
        match &channel.values {
            AnimationValues::Translations(values) => meter.add_vec(values)?,
            AnimationValues::Rotations(values) => meter.add_vec(values)?,
            AnimationValues::Scales(values) => meter.add_vec(values)?,
            AnimationValues::MorphWeights { values, .. } => meter.add_vec(values)?,
        }
        measure_metadata_map_dynamic(&channel.metadata, meter, work)?;
    }
    measure_metadata_map_dynamic(&animation.metadata, meter, work)
}

fn measure_camera_dynamic<E: Error + Send + Sync + 'static>(
    camera: &Camera,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&camera.name, meter)?;
    measure_metadata_map_dynamic(&camera.metadata, meter, work)
}

fn measure_light_dynamic<E: Error + Send + Sync + 'static>(
    light: &Light,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&light.name, meter)?;
    measure_metadata_map_dynamic(&light.metadata, meter, work)
}

fn measure_skin_dynamic<E: Error + Send + Sync + 'static>(
    skin: &Skin,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    measure_optional_string(&skin.name, meter)?;
    meter.add_vec(&skin.joints)?;
    meter.add_vec(&skin.inverse_bind_matrices)?;
    measure_metadata_map_dynamic(&skin.metadata, meter, work)
}

fn measure_metadata_map_dynamic<E: Error + Send + Sync + 'static>(
    metadata: &MetadataMap,
    meter: &mut FootprintMeter,
    work: &mut impl FnMut(u64) -> Result<(), SceneValidationFailure<E>>,
) -> Result<(), SceneValidationFailure<E>> {
    meter.add_map(metadata)?;
    for (key, value) in metadata {
        work(1)?;
        meter.add_string(key)?;
        if let MetadataValue::String(value) = value {
            meter.add_string(value)?;
        }
    }
    Ok(())
}

pub(crate) fn clone_scene_data_with<C: SceneValidationControl>(
    scene: &SceneData,
    footprint: SceneFootprint,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<(SceneData, C::Reservation), SceneValidationFailure<C::Error>> {
    enforce_limit(
        SceneValidationLimit::CanonicalBytes,
        footprint.accounted_payload_bytes(),
        limits.max_canonical_bytes(),
    )?;
    let mut context = CloneContext::new(limits, control);
    context.poll_now()?;
    let envelope = scene_data_clone_envelope_with(scene, &mut |amount| context.work(amount))?;
    enforce_limit(
        SceneValidationLimit::CanonicalBytes,
        envelope.accounted_payload_bytes(),
        limits.max_canonical_bytes(),
    )?;
    let mut reservation = context
        .control
        .reserve(
            SceneReservationKind::EditCandidate,
            envelope.accounted_payload_bytes(),
        )
        .map_err(SceneValidationFailure::Control)?;

    let cloned = try_clone_scene_data(scene, &mut context)?;
    let actual = scene_data_footprint_with(&cloned, &mut |amount| context.work(amount))?;
    if actual.accounted_payload_bytes() > envelope.accounted_payload_bytes() {
        return Err(SceneValidationError::CapacityReservationUnderestimated {
            resource: "scene clone envelope",
            admitted: envelope.accounted_payload_bytes(),
            actual: actual.accounted_payload_bytes(),
        }
        .into());
    }
    context
        .control
        .resize_reservation(
            &mut reservation,
            SceneReservationKind::EditCandidate,
            actual.accounted_payload_bytes(),
        )
        .map_err(SceneValidationFailure::Control)?;
    context.poll_now()?;
    Ok((cloned, reservation))
}

struct CloneContext<'a, C: SceneValidationControl> {
    control: &'a mut C,
    limits: SceneValidationLimits,
    work_units: u64,
    work_since_poll: u64,
}

impl<'a, C: SceneValidationControl> CloneContext<'a, C> {
    fn new(limits: SceneValidationLimits, control: &'a mut C) -> Self {
        Self {
            control,
            limits,
            work_units: 0,
            work_since_poll: 0,
        }
    }

    fn work(&mut self, amount: u64) -> Result<(), SceneValidationFailure<C::Error>> {
        self.work_units = self.work_units.checked_add(amount).ok_or(
            SceneValidationError::AccountingOverflow {
                resource: "scene clone work",
            },
        )?;
        enforce_limit(
            SceneValidationLimit::Work,
            self.work_units,
            self.limits.max_work(),
        )?;
        self.control
            .charge_work(SceneValidationWork::CloneCell, amount)
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

    fn poll_now(&mut self) -> Result<(), SceneValidationFailure<C::Error>> {
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
}

use crate::SCENE_VALIDATION_POLL_INTERVAL;

fn try_clone_scene_data<C: SceneValidationControl>(
    source: &SceneData,
    context: &mut CloneContext<'_, C>,
) -> Result<SceneData, SceneValidationFailure<C::Error>> {
    Ok(SceneData {
        root: source.root,
        nodes: try_clone_vec_with(&source.nodes, context, try_clone_node)?,
        meshes: try_clone_vec_with(&source.meshes, context, try_clone_mesh)?,
        materials: try_clone_vec_with(&source.materials, context, try_clone_material)?,
        textures: try_clone_vec_with(&source.textures, context, try_clone_texture)?,
        animations: try_clone_vec_with(&source.animations, context, try_clone_animation)?,
        cameras: try_clone_vec_with(&source.cameras, context, try_clone_camera)?,
        lights: try_clone_vec_with(&source.lights, context, try_clone_light)?,
        skins: try_clone_vec_with(&source.skins, context, try_clone_skin)?,
        metadata: try_clone_metadata(&source.metadata, context)?,
        space: source.space,
    })
}

fn try_clone_vec_copy<T: Copy, C: SceneValidationControl>(
    source: &[T],
    context: &mut CloneContext<'_, C>,
    resource: &'static str,
) -> Result<Vec<T>, SceneValidationFailure<C::Error>> {
    let mut output = reserve_clone_vec::<T>(source.len(), resource)?;
    let mut offset = 0;
    while offset < source.len() {
        let chunk_len = (source.len() - offset).min(context.work_until_poll());
        let end =
            offset
                .checked_add(chunk_len)
                .ok_or(SceneValidationError::AccountingOverflow {
                    resource: "scene clone chunk",
                })?;
        context.work(u64_len(chunk_len, "scene clone cells")?)?;
        output.extend_from_slice(&source[offset..end]);
        offset = end;
    }
    Ok(output)
}

fn try_clone_vec_with<T, U, C: SceneValidationControl>(
    source: &[T],
    context: &mut CloneContext<'_, C>,
    mut clone_item: impl FnMut(
        &T,
        &mut CloneContext<'_, C>,
    ) -> Result<U, SceneValidationFailure<C::Error>>,
) -> Result<Vec<U>, SceneValidationFailure<C::Error>> {
    let mut output = reserve_clone_vec::<U>(source.len(), "scene clone collection")?;
    for item in source {
        context.work(1)?;
        output.push(clone_item(item, context)?);
    }
    Ok(output)
}

fn try_clone_string<C: SceneValidationControl>(
    source: &str,
    context: &mut CloneContext<'_, C>,
) -> Result<String, SceneValidationFailure<C::Error>> {
    let plan = CapacityPlan::for_type::<u8>(0, 0, source.len())
        .map_err(|cause| capacity_plan_error("scene clone string", cause))?;
    plan.verify_preflight(0, 0)
        .map_err(|cause| capacity_plan_error("scene clone string", cause))?;
    let mut output = String::new();
    output
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| SceneValidationError::AllocationFailed {
            resource: "scene clone string",
        })?;
    plan.finish(output.len(), output.capacity())
        .map_err(|cause| capacity_plan_error("scene clone string", cause))?;
    for character in source.chars() {
        context.work(u64_len(character.len_utf8(), "scene clone string")?)?;
        output.push(character);
    }
    Ok(output)
}

fn try_clone_optional_string<C: SceneValidationControl>(
    source: &Option<String>,
    context: &mut CloneContext<'_, C>,
) -> Result<Option<String>, SceneValidationFailure<C::Error>> {
    source
        .as_deref()
        .map(|value| try_clone_string(value, context))
        .transpose()
}

fn try_clone_node<C: SceneValidationControl>(
    source: &Node,
    context: &mut CloneContext<'_, C>,
) -> Result<Node, SceneValidationFailure<C::Error>> {
    Ok(Node {
        name: try_clone_optional_string(&source.name, context)?,
        transform: source.transform,
        parent: source.parent,
        children: try_clone_vec_copy(&source.children, context, "node children")?,
        mesh_bindings: try_clone_vec_copy(&source.mesh_bindings, context, "node mesh bindings")?,
        camera: source.camera,
        light: source.light,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_mesh<C: SceneValidationControl>(
    source: &Mesh,
    context: &mut CloneContext<'_, C>,
) -> Result<Mesh, SceneValidationFailure<C::Error>> {
    Ok(Mesh {
        name: try_clone_optional_string(&source.name, context)?,
        topology: source.topology,
        positions: try_clone_vec_copy(&source.positions, context, "mesh positions")?,
        normals: try_clone_vec_copy(&source.normals, context, "mesh normals")?,
        tangents: try_clone_vec_copy(&source.tangents, context, "mesh tangents")?,
        texcoords: try_clone_vec_with(&source.texcoords, context, |values, context| {
            try_clone_vec_copy(values, context, "mesh texture coordinates")
        })?,
        colors: try_clone_vec_with(&source.colors, context, |values, context| {
            try_clone_vec_copy(values, context, "mesh colors")
        })?,
        indices: try_clone_vec_copy(&source.indices, context, "mesh indices")?,
        face_vertex_counts: try_clone_vec_copy(
            &source.face_vertex_counts,
            context,
            "mesh face counts",
        )?,
        material: source.material,
        joint_indices: try_clone_vec_copy(&source.joint_indices, context, "mesh joint indices")?,
        joint_weights: try_clone_vec_copy(&source.joint_weights, context, "mesh joint weights")?,
        morph_targets: try_clone_vec_with(&source.morph_targets, context, try_clone_morph_target)?,
        custom_attributes: try_clone_vec_with(
            &source.custom_attributes,
            context,
            try_clone_vertex_attribute,
        )?,
        bounds: source.bounds,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_morph_target<C: SceneValidationControl>(
    source: &MorphTarget,
    context: &mut CloneContext<'_, C>,
) -> Result<MorphTarget, SceneValidationFailure<C::Error>> {
    Ok(MorphTarget {
        name: try_clone_optional_string(&source.name, context)?,
        positions: try_clone_vec_copy(&source.positions, context, "morph positions")?,
        normals: try_clone_vec_copy(&source.normals, context, "morph normals")?,
        tangents: try_clone_vec_copy(&source.tangents, context, "morph tangents")?,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_vertex_attribute<C: SceneValidationControl>(
    source: &VertexAttribute,
    context: &mut CloneContext<'_, C>,
) -> Result<VertexAttribute, SceneValidationFailure<C::Error>> {
    let semantic = match &source.semantic {
        VertexAttributeSemantic::Position => VertexAttributeSemantic::Position,
        VertexAttributeSemantic::Normal => VertexAttributeSemantic::Normal,
        VertexAttributeSemantic::Tangent => VertexAttributeSemantic::Tangent,
        VertexAttributeSemantic::Texcoord(value) => VertexAttributeSemantic::Texcoord(*value),
        VertexAttributeSemantic::Color(value) => VertexAttributeSemantic::Color(*value),
        VertexAttributeSemantic::Joints(value) => VertexAttributeSemantic::Joints(*value),
        VertexAttributeSemantic::Weights(value) => VertexAttributeSemantic::Weights(*value),
        VertexAttributeSemantic::Custom(value) => {
            VertexAttributeSemantic::Custom(try_clone_string(value, context)?)
        }
    };
    let data = match &source.data {
        VertexAttributeData::F32(values) => {
            VertexAttributeData::F32(try_clone_vec_copy(values, context, "attribute f32")?)
        }
        VertexAttributeData::Vec2(values) => {
            VertexAttributeData::Vec2(try_clone_vec_copy(values, context, "attribute vec2")?)
        }
        VertexAttributeData::Vec3(values) => {
            VertexAttributeData::Vec3(try_clone_vec_copy(values, context, "attribute vec3")?)
        }
        VertexAttributeData::Vec4(values) => {
            VertexAttributeData::Vec4(try_clone_vec_copy(values, context, "attribute vec4")?)
        }
        VertexAttributeData::U16x4(values) => {
            VertexAttributeData::U16x4(try_clone_vec_copy(values, context, "attribute u16x4")?)
        }
        VertexAttributeData::U32(values) => {
            VertexAttributeData::U32(try_clone_vec_copy(values, context, "attribute u32")?)
        }
        VertexAttributeData::I32(values) => {
            VertexAttributeData::I32(try_clone_vec_copy(values, context, "attribute i32")?)
        }
    };
    Ok(VertexAttribute {
        name: try_clone_string(&source.name, context)?,
        semantic,
        data,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_material<C: SceneValidationControl>(
    source: &Material,
    context: &mut CloneContext<'_, C>,
) -> Result<Material, SceneValidationFailure<C::Error>> {
    Ok(Material {
        name: try_clone_optional_string(&source.name, context)?,
        shading_model: source.shading_model,
        base_color: source.base_color,
        metallic: source.metallic,
        roughness: source.roughness,
        emissive: source.emissive,
        alpha_mode: source.alpha_mode,
        alpha_cutoff: source.alpha_cutoff,
        double_sided: source.double_sided,
        texture_slots: try_clone_vec_with(&source.texture_slots, context, try_clone_texture_slot)?,
        properties: try_clone_material_properties(&source.properties, context)?,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_texture_slot<C: SceneValidationControl>(
    source: &TextureSlot,
    context: &mut CloneContext<'_, C>,
) -> Result<TextureSlot, SceneValidationFailure<C::Error>> {
    Ok(TextureSlot {
        texture: source.texture,
        role: source.role,
        color_space: source.color_space,
        uv_set: source.uv_set,
        scale: source.scale,
        transform: source.transform,
        source_key: try_clone_optional_string(&source.source_key, context)?,
    })
}

fn try_clone_material_properties<C: SceneValidationControl>(
    source: &MaterialPropertyMap,
    context: &mut CloneContext<'_, C>,
) -> Result<MaterialPropertyMap, SceneValidationFailure<C::Error>> {
    let mut output = reserve_map_clone(source, "material property map")?;
    for (key, value) in source {
        context.work(1)?;
        output
            .insert_reserved(
                try_clone_string(key, context)?,
                try_clone_material_property(value, context)?,
            )
            .map_err(map_clone_error)?;
    }
    Ok(output)
}

fn try_clone_material_property<C: SceneValidationControl>(
    source: &MaterialProperty,
    context: &mut CloneContext<'_, C>,
) -> Result<MaterialProperty, SceneValidationFailure<C::Error>> {
    Ok(match source {
        MaterialProperty::Bool(value) => MaterialProperty::Bool(*value),
        MaterialProperty::I64(value) => MaterialProperty::I64(*value),
        MaterialProperty::F64(value) => MaterialProperty::F64(*value),
        MaterialProperty::String(value) => {
            MaterialProperty::String(try_clone_string(value, context)?)
        }
        MaterialProperty::Color(value) => MaterialProperty::Color(*value),
        MaterialProperty::Vec2(value) => MaterialProperty::Vec2(*value),
        MaterialProperty::Vec3(value) => MaterialProperty::Vec3(*value),
        MaterialProperty::Vec4(value) => MaterialProperty::Vec4(*value),
        MaterialProperty::Texture(value) => MaterialProperty::Texture(*value),
    })
}

fn try_clone_texture<C: SceneValidationControl>(
    source: &Texture,
    context: &mut CloneContext<'_, C>,
) -> Result<Texture, SceneValidationFailure<C::Error>> {
    let source_value = match &source.source {
        TextureSource::External { uri } => TextureSource::External {
            uri: try_clone_string(uri, context)?,
        },
        TextureSource::Embedded { bytes, mime_type } => TextureSource::Embedded {
            bytes: bytes.clone(),
            mime_type: try_clone_optional_string(mime_type, context)?,
        },
    };
    Ok(Texture {
        name: try_clone_optional_string(&source.name, context)?,
        source: source_value,
        sampler: source.sampler,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_animation<C: SceneValidationControl>(
    source: &Animation,
    context: &mut CloneContext<'_, C>,
) -> Result<Animation, SceneValidationFailure<C::Error>> {
    Ok(Animation {
        name: try_clone_optional_string(&source.name, context)?,
        channels: try_clone_vec_with(&source.channels, context, try_clone_animation_channel)?,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_animation_channel<C: SceneValidationControl>(
    source: &AnimationChannel,
    context: &mut CloneContext<'_, C>,
) -> Result<AnimationChannel, SceneValidationFailure<C::Error>> {
    let values = match &source.values {
        AnimationValues::Translations(values) => AnimationValues::Translations(try_clone_vec_copy(
            values,
            context,
            "animation translations",
        )?),
        AnimationValues::Rotations(values) => {
            AnimationValues::Rotations(try_clone_vec_copy(values, context, "animation rotations")?)
        }
        AnimationValues::Scales(values) => {
            AnimationValues::Scales(try_clone_vec_copy(values, context, "animation scales")?)
        }
        AnimationValues::MorphWeights {
            values,
            weights_per_keyframe,
        } => AnimationValues::MorphWeights {
            values: try_clone_vec_copy(values, context, "animation morph weights")?,
            weights_per_keyframe: *weights_per_keyframe,
        },
    };
    Ok(AnimationChannel {
        target: source.target,
        interpolation: source.interpolation,
        times_seconds: try_clone_vec_copy(&source.times_seconds, context, "animation times")?,
        values,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_camera<C: SceneValidationControl>(
    source: &Camera,
    context: &mut CloneContext<'_, C>,
) -> Result<Camera, SceneValidationFailure<C::Error>> {
    Ok(Camera {
        name: try_clone_optional_string(&source.name, context)?,
        projection: source.projection.clone(),
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_light<C: SceneValidationControl>(
    source: &Light,
    context: &mut CloneContext<'_, C>,
) -> Result<Light, SceneValidationFailure<C::Error>> {
    Ok(Light {
        name: try_clone_optional_string(&source.name, context)?,
        kind: source.kind,
        color: source.color,
        intensity: source.intensity,
        range: source.range,
        inner_cone_angle: source.inner_cone_angle,
        outer_cone_angle: source.outer_cone_angle,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_skin<C: SceneValidationControl>(
    source: &Skin,
    context: &mut CloneContext<'_, C>,
) -> Result<Skin, SceneValidationFailure<C::Error>> {
    Ok(Skin {
        name: try_clone_optional_string(&source.name, context)?,
        joints: try_clone_vec_copy(&source.joints, context, "skin joints")?,
        inverse_bind_matrices: try_clone_vec_copy(
            &source.inverse_bind_matrices,
            context,
            "skin inverse bind matrices",
        )?,
        skeleton_root: source.skeleton_root,
        metadata: try_clone_metadata(&source.metadata, context)?,
    })
}

fn try_clone_metadata<C: SceneValidationControl>(
    source: &MetadataMap,
    context: &mut CloneContext<'_, C>,
) -> Result<MetadataMap, SceneValidationFailure<C::Error>> {
    let mut output = reserve_map_clone(source, "metadata map")?;
    for (key, value) in source {
        context.work(1)?;
        let value = match value {
            MetadataValue::Bool(value) => MetadataValue::Bool(*value),
            MetadataValue::I64(value) => MetadataValue::I64(*value),
            MetadataValue::F64(value) => MetadataValue::F64(*value),
            MetadataValue::String(value) => {
                MetadataValue::String(try_clone_string(value, context)?)
            }
        };
        output
            .insert_reserved(try_clone_string(key, context)?, value)
            .map_err(map_clone_error)?;
    }
    Ok(output)
}

fn reserve_map_clone<K, V>(
    source: &OrderedMap<K, V>,
    resource: &'static str,
) -> Result<OrderedMap<K, V>, SceneValidationError> {
    let mut output = OrderedMap::new();
    let plan = output
        .capacity_plan(source.len())
        .map_err(|error| map_allocation_error(error, resource))?;
    output
        .apply_capacity_plan(plan)
        .map_err(|error| map_allocation_error(error, resource))?;
    Ok(output)
}

fn reserve_clone_vec<T>(
    desired_capacity: usize,
    resource: &'static str,
) -> Result<Vec<T>, SceneValidationError> {
    let plan = empty_vec_capacity_plan::<T>(desired_capacity, resource)?;
    let (output, _) = apply_empty_vec_capacity_plan(plan, resource)?;
    Ok(output)
}

fn map_accounting_error(_error: OrderedMapError) -> SceneValidationError {
    SceneValidationError::AccountingOverflow {
        resource: "scene map capacity",
    }
}

fn map_allocation_error(error: OrderedMapError, resource: &'static str) -> SceneValidationError {
    match error {
        OrderedMapError::CapacityOverflow => SceneValidationError::AccountingOverflow { resource },
        OrderedMapError::AllocationFailed | OrderedMapError::CapacityNotReserved => {
            SceneValidationError::AllocationFailed { resource }
        }
        OrderedMapError::CapacityPlan(cause) => capacity_plan_error(resource, cause),
    }
}

fn map_clone_error(error: OrderedMapError) -> SceneValidationError {
    map_allocation_error(error, "scene map clone")
}
